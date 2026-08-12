//! Nitrate — compress any video to fit a target file size.

// Public so the integration tests can drive the encoder without a running app.
pub mod deeplink;
pub mod download;
pub mod encode;
pub mod ffmpeg;
pub mod media;
mod tray;

use base64::Engine as _;
use encode::{Edits, Settings};
use ffmpeg::{Binaries, MediaInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Where a job's video comes from. A link has to be fetched before there's
/// anything to encode; a file is ready immediately.
enum JobSource {
    File(PathBuf),
    Url { url: String, title: String },
}

struct QueuedJob {
    id: String,
    source: JobSource,
    settings: Settings,
    edits: Edits,
    cancel: Arc<AtomicBool>,
}

pub struct AppState {
    bins: Binaries,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    queue: Sender<QueuedJob>,
    /// Files named on the command line, held until the webview is ready to
    /// receive them. Launching via "Open with" beats the frontend to startup.
    pending_files: Mutex<Vec<String>>,
    /// Same, for `nitrate://` links arriving from a browser.
    pending_links: Mutex<Vec<String>>,
    /// Caps how fast links can arrive, since any page can fire the protocol.
    link_limit: Mutex<deeplink::RateLimit>,
    /// Set when something asked for the window before the UI had painted.
    show_on_ready: Arc<AtomicBool>,
    /// Set while a native dialog is open or the window is pinned, so the
    /// hide-on-blur behaviour doesn't yank the popup away mid-interaction.
    suppress_hide: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: &'a str,
    progress: f64,
    stage: &'a str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoneEvent {
    id: String,
    output: String,
    final_bytes: u64,
    original_bytes: u64,
    attempts: u32,
    notes: Vec<String>,
    width: u32,
    height: u32,
    /// True when the file was handed over as-is rather than re-encoded.
    passed_through: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailedEvent {
    id: String,
    error: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelledEvent {
    id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchedItem {
    path: String,
    kind: media::MediaKind,
}

/// A post that held more than one thing.
///
/// The link job ends here rather than compressing: each item becomes a job of
/// its own, and they're shown together under the post they came from.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchedEvent {
    id: String,
    title: String,
    items: Vec<FetchedItem>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    path: String,
    name: String,
    info: MediaInfo,
    thumbnail: Option<String>,
}

/// Probes a dropped file and extracts a poster frame for its card.
#[tauri::command]
async fn inspect_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<FileInfo, String> {
    let input = PathBuf::from(&path);
    if !input.is_file() {
        return Err("That file doesn't exist.".into());
    }

    let info = ffmpeg::probe(&state.bins, &input)?;

    // A frame from 25% in is usually more representative than frame zero,
    // which is often black or a fade-in.
    //
    // The frame comes back as a data URL rather than a file path: these are a
    // few KB each, and inlining them avoids the asset protocol entirely — no
    // scope config, no CSP origin to keep in sync, nothing to leave on disk.
    let thumbnail = app.path().app_cache_dir().ok().and_then(|cache| {
        std::fs::create_dir_all(&cache).ok()?;
        let out = cache.join(format!("thumb-{}.jpg", fast_hash(&path)));
        ffmpeg::thumbnail(&state.bins, &input, &out, info.duration * 0.25, 320).ok()?;
        let bytes = std::fs::read(&out).ok()?;
        let _ = std::fs::remove_file(&out);
        Some(format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    });

    Ok(FileInfo {
        name: input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone()),
        path,
        info,
        thumbnail,
    })
}

/// Works out what would happen, without encoding anything — drives the
/// "1080p → 720p, 1.4 Mbps" preview on each card and the editor's live readout.
#[tauri::command]
async fn preview_plan(
    state: State<'_, AppState>,
    path: String,
    settings: Settings,
    edits: Option<Edits>,
) -> Result<encode::Plan, String> {
    let input = PathBuf::from(&path);
    let info = ffmpeg::probe(&state.bins, &input)?;
    encode::plan(&info, &settings, &edits.unwrap_or_default(), &state.bins)
}

/// Reads a pasted link without downloading anything.
#[tauri::command]
async fn inspect_url(app: AppHandle, url: String) -> Result<download::UrlInfo, String> {
    if !download::looks_like_url(&url) {
        return Err("That doesn't look like a link.".into());
    }
    let data_dir = app_data_dir(&app)?;
    let bin = download::ensure(&data_dir, |_| {})?;
    download::probe_url(&bin, &url)
}

#[tauri::command]
async fn start_job(
    state: State<'_, AppState>,
    id: String,
    path: Option<String>,
    url: Option<String>,
    title: Option<String>,
    settings: Settings,
    edits: Option<Edits>,
) -> Result<(), String> {
    let source = match (path, url) {
        (Some(p), _) => JobSource::File(PathBuf::from(p)),
        (None, Some(u)) => JobSource::Url {
            title: title.unwrap_or_else(|| download::site_name(&u)),
            url: u,
        },
        _ => return Err("A job needs either a file or a link.".into()),
    };

    let cancel = Arc::new(AtomicBool::new(false));
    state
        .cancels
        .lock()
        .unwrap()
        .insert(id.clone(), Arc::clone(&cancel));

    state
        .queue
        .send(QueuedJob {
            id,
            source,
            settings,
            edits: edits.unwrap_or_default(),
            cancel,
        })
        .map_err(|_| "The encoding queue has shut down.".to_string())
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map_err(|e| format!("Couldn't find the app data folder: {e}"))
}

// ---------------------------------------------------------------------------
// Editor support
// ---------------------------------------------------------------------------

fn frame_data_url(
    bins: &Binaries,
    input: &Path,
    at: f64,
    cache: &Path,
    width: u32,
) -> Option<String> {
    std::fs::create_dir_all(cache).ok()?;
    let out = cache.join(format!("frame-{}-{}.jpg", width, (at * 1000.0) as u64));
    ffmpeg::thumbnail(bins, input, &out, at, width).ok()?;
    let bytes = std::fs::read(&out).ok()?;
    let _ = std::fs::remove_file(&out);
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// A single frame, for the editor's preview when the webview can't play the
/// file itself.
#[tauri::command]
async fn frame_at(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    time: f64,
) -> Result<String, String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("No cache folder: {e}"))?;
    // Wide enough to fill the editor's preview without being upscaled.
    frame_data_url(
        &state.bins,
        &PathBuf::from(path),
        time.max(0.0),
        &cache,
        960,
    )
    .ok_or_else(|| "Couldn't read a frame from that video.".to_string())
}

/// Lets the webview load this file directly, so the editor can play it.
///
/// The scope is extended per file rather than opened to the whole disk: only
/// videos the user has actually added become readable.
/// A copy of the video the webview is certain to play, for the editor.
///
/// Asked for only once the `<video>` element has given up on the original, so
/// nothing is re-encoded for a file that plays perfectly well. Cached by what
/// the file is rather than only where it is, so a re-downloaded file at the
/// same path doesn't keep showing the previous preview.
#[tauri::command]
async fn preview_proxy(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let input = PathBuf::from(&path);
    if !input.is_file() {
        return Err("That file doesn't exist.".into());
    }

    let out = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("No cache folder: {e}"))?
        .join("previews")
        .join(format!("{}.mp4", preview_key(&input)));

    if !out.is_file() {
        ffmpeg::preview_proxy(&state.bins, &input, &out)?;
    }

    app.asset_protocol_scope()
        .allow_file(&out)
        .map_err(|e| format!("Couldn't grant preview access: {e}"))?;
    Ok(out.to_string_lossy().into_owned())
}

/// Names a preview after the file's identity, not just its path.
fn preview_key(input: &Path) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(input) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
                since.as_secs().hash(&mut hasher);
            }
        }
    }
    format!("preview-{:016x}", hasher.finish())
}

#[tauri::command]
fn allow_preview(app: AppHandle, path: String) -> Result<String, String> {
    let file = PathBuf::from(&path);
    if !file.is_file() {
        return Err("That file doesn't exist.".into());
    }
    app.asset_protocol_scope()
        .allow_file(&file)
        .map_err(|e| format!("Couldn't grant preview access: {e}"))?;
    Ok(path)
}

/// Puts a finished video on the clipboard as a file.
///
/// The file itself, not its path as text — so pasting into Discord attaches the
/// video, and pasting into Explorer produces a copy of it. That's the CF_HDROP
/// format, the same thing Explorer's own Copy puts there, and it's why this
/// can't go through Tauri's clipboard plugin: that one handles text, HTML and
/// images, none of which is a file.
#[tauri::command]
fn copy_video_to_clipboard(path: String) -> Result<(), String> {
    let file = PathBuf::from(&path);
    if !file.is_file() {
        return Err("That file is no longer there.".into());
    }

    #[cfg(windows)]
    {
        use clipboard_win::{raw, Clipboard};

        // The clipboard is a single shared resource, and whichever app had it
        // open last may not have let go yet. A few attempts costs nothing and
        // avoids failing on a race that resolves itself in milliseconds.
        let _clipboard =
            Clipboard::new_attempts(10).map_err(|e| format!("Couldn't open the clipboard: {e}"))?;

        raw::empty().map_err(|e| format!("Couldn't clear the clipboard: {e}"))?;
        raw::set_file_list(&[file.to_string_lossy().as_ref()])
            .map_err(|e| format!("Couldn't copy the file: {e}"))?;

        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err("Copying a file to the clipboard isn't supported on this platform yet.".into())
    }
}

/// Says the title bar has been pressed, so any movement now is a drag.
///
/// The window can't tell one from the other on its own: the same event arrives
/// whether the user dragged it or the app repositioned it. See
/// `tray::note_drag_start`.
#[tauri::command]
fn window_drag_started() {
    tray::note_drag_start();
}

/// Plays a finished video in whatever the system uses for it.
///
/// Done here rather than with the plugin's `openPath` from the frontend, which
/// needs `opener:allow-open-path` — a permission with no scope, letting the
/// webview ask the OS to launch anything on disk. The check below keeps it to
/// video files that exist, in the same spirit as `allow_preview`: hand out the
/// narrowest thing that does the job.
#[tauri::command]
fn open_video(app: AppHandle, path: String) -> Result<(), String> {
    let file = PathBuf::from(&path);
    if !file.is_file() {
        return Err("That file is no longer there.".into());
    }

    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !download::VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        return Err("That isn't a video file.".into());
    }

    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(file.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Couldn't open that video: {e}"))
}

/// Evenly spaced frames for the trim timeline.
#[tauri::command]
async fn filmstrip(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    count: usize,
) -> Result<Vec<String>, String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("No cache folder: {e}"))?;
    let input = PathBuf::from(path);
    let info = ffmpeg::probe(&state.bins, &input)?;

    let count = count.clamp(4, 24);
    let mut frames = Vec::with_capacity(count);
    for i in 0..count {
        // Sample from the middle of each slice rather than its edge, so the
        // first frame isn't the usual black fade-in.
        let at = info.duration * ((i as f64 + 0.5) / count as f64);
        // Small: they're shown a few dozen pixels wide in the strip.
        match frame_data_url(&state.bins, &input, at, &cache, 160) {
            Some(url) => frames.push(url),
            None => break,
        }
    }

    if frames.is_empty() {
        Err("Couldn't read frames from that video.".into())
    } else {
        Ok(frames)
    }
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>, id: String) {
    if let Some(flag) = state.cancels.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    hardware_encoders: Vec<String>,
    ffmpeg_found: bool,
}

/// Lets the UI grey out codecs this ffmpeg build can't produce.
#[tauri::command]
async fn capabilities(state: State<'_, AppState>) -> Result<Capabilities, String> {
    let encoders = ffmpeg::available_encoders(&state.bins);
    let hardware = encoders
        .iter()
        .filter(|e| {
            e.contains("nvenc")
                || e.contains("qsv")
                || e.contains("amf")
                || e.contains("videotoolbox")
        })
        .cloned()
        .collect();

    Ok(Capabilities {
        ffmpeg_found: !encoders.is_empty(),
        hardware_encoders: hardware,
    })
}

/// Keeps the popup on screen while a native dialog is up.
#[tauri::command]
fn set_suppress_hide(state: State<'_, AppState>, suppress: bool) {
    state.suppress_hide.store(suppress, Ordering::Relaxed);
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// The popup is deliberately small, which is wrong for dragging a crop
/// rectangle. It grows while the editor is open and shrinks back afterwards.
#[tauri::command]
fn set_editor_size(app: AppHandle, expanded: bool) {
    tray::resize_popup(&app, expanded);
}

/// Hands over any files named on the command line, clearing them so a later
/// call doesn't re-add the same videos.
#[tauri::command]
fn take_pending_files(state: State<'_, AppState>) -> Vec<String> {
    std::mem::take(&mut *state.pending_files.lock().unwrap())
}

/// Called once the UI has painted.
///
/// Showing the window from `setup` races the webview: the window can be
/// revealed before there's anything in it, or the show can be swallowed
/// entirely. Waiting for the frontend to say it's ready avoids both, and means
/// the first frame the user sees is the finished interface.
#[tauri::command]
fn frontend_ready(app: AppHandle, state: State<'_, AppState>) {
    let opened_with_files = !state.pending_files.lock().unwrap().is_empty();
    if cfg!(debug_assertions) || opened_with_files || state.show_on_ready.load(Ordering::Relaxed) {
        tray::show_popup(&app, None);
    }
}

/// Picks the real file paths out of argv, ignoring the executable and any flags.
fn files_from_args<I: Iterator<Item = String>>(args: I) -> Vec<String> {
    args.skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .filter(|arg| std::path::Path::new(arg).is_file())
        .collect()
}

/// Picks out `nitrate://` links. Windows hands a protocol activation to the
/// app as an ordinary command-line argument, so this is the same path that
/// "Open with" already uses.
fn links_from_args<I: Iterator<Item = String>>(args: I) -> Vec<String> {
    args.skip(1)
        .filter(|arg| arg.starts_with(deeplink::SCHEME))
        .collect()
}

/// Hands over links that arrived while the UI wasn't ready.
#[tauri::command]
fn take_pending_links(state: State<'_, AppState>) -> Vec<String> {
    std::mem::take(&mut *state.pending_links.lock().unwrap())
}

/// Validates a batch of raw `nitrate://` strings and queues what survives.
///
/// Returns the first complaint, if any, so the UI can say why something was
/// ignored rather than silently dropping it.
fn accept_links(app: &AppHandle, raw: Vec<String>) -> Option<String> {
    if raw.is_empty() {
        return None;
    }

    let state = app.state::<AppState>();
    let mut complaint = None;
    let mut accepted = Vec::new();

    for candidate in raw {
        // Rate limiting comes first: a page firing links in a loop shouldn't
        // get to do the parsing work either.
        if !state.link_limit.lock().unwrap().allow() {
            complaint.get_or_insert(deeplink::Rejected::TooMany.message().to_string());
            break;
        }

        match deeplink::parse(&candidate) {
            Ok(link) => accepted.push(link.url),
            Err(why) => {
                complaint.get_or_insert(why.message().to_string());
            }
        }
    }

    if !accepted.is_empty() {
        state.pending_links.lock().unwrap().extend(accepted);
        let _ = app.emit("links://open", ());
        // Always surface the window. Even on the automatic setting, something
        // arriving from a web page should never happen out of sight.
        tray::show_popup(app, None);
    }

    complaint
}

fn fast_hash(s: &str) -> u64 {
    // FNV-1a — good enough to name a cache file.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Decimal megabytes, matching how upload limits are advertised.
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.2} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

// ---------------------------------------------------------------------------
// Worker pool
// ---------------------------------------------------------------------------

fn spawn_workers(app: AppHandle, rx: Receiver<QueuedJob>, count: usize) {
    let rx = Arc::new(Mutex::new(rx));

    for _ in 0..count {
        let app = app.clone();
        let rx = Arc::clone(&rx);

        std::thread::spawn(move || loop {
            // Hold the lock only long enough to claim a job.
            let job = {
                let guard = rx.lock().unwrap();
                guard.recv()
            };
            let Ok(job) = job else { break };

            process_job(&app, job);
        });
    }
}

fn process_job(app: &AppHandle, job: QueuedJob) {
    let state = app.state::<AppState>();
    let QueuedJob {
        id,
        source,
        settings,
        edits,
        cancel,
    } = job;

    let finish = |app: &AppHandle| {
        app.state::<AppState>().cancels.lock().unwrap().remove(&id);
    };
    let emit = |progress: f64, stage: &str| {
        let _ = app.emit(
            "job://progress",
            ProgressEvent {
                id: &id,
                progress,
                stage,
            },
        );
    };
    let fail = |app: &AppHandle, error: String| {
        let _ = app.emit(
            "job://failed",
            FailedEvent {
                id: id.clone(),
                error,
            },
        );
    };

    if cancel.load(Ordering::Relaxed) {
        let _ = app.emit("job://cancelled", CancelledEvent { id: id.clone() });
        finish(app);
        return;
    }

    emit(0.0, "Preparing");

    // A link has to become a local file before anything else can happen.
    let from_url = matches!(source, JobSource::Url { .. });
    let mut work_dir: Option<PathBuf> = None;

    let (input, name_hint) = match source {
        JobSource::File(path) => (path, None),
        JobSource::Url { url, title } => {
            let data_dir = match app_data_dir(app) {
                Ok(d) => d,
                Err(e) => {
                    fail(app, e);
                    finish(app);
                    return;
                }
            };

            let bin =
                match download::ensure(&data_dir, |p| emit(p * 0.05, "Fetching the downloader")) {
                    Ok(b) => b,
                    Err(e) => {
                        fail(app, e);
                        finish(app);
                        return;
                    }
                };

            let dir =
                std::env::temp_dir().join(format!("nitrate-dl-{}-{}", std::process::id(), id));
            work_dir = Some(dir.clone());

            // gallery-dl only matters for the stills yt-dlp can't reach, so a
            // failure to fetch it isn't fatal — the post still yields whatever
            // yt-dlp could see.
            let gallery = download::ensure_gallery(&data_dir).ok();

            let fetched = media::fetch_post(
                &bin,
                gallery.as_deref(),
                &state.bins,
                &url,
                &dir,
                settings.max_download_height,
                &cancel,
                |p| emit(0.05 + p * 0.40, "Downloading"),
            );

            // A post holding several things becomes several jobs. They're
            // moved out of the temp folder first, because this one is about to
            // be deleted and the frontend is about to be handed the paths.
            if let Ok(Ok(items)) = &fetched {
                if items.len() > 1 {
                    let mut placed = Vec::new();
                    for (index, item) in items.iter().enumerate() {
                        let hint = format!("{title} {}", index + 1);
                        match encode::place_untouched(&item.path, &settings, Some(&hint)) {
                            Ok(path) => placed.push(FetchedItem {
                                path: path.to_string_lossy().into_owned(),
                                kind: item.kind,
                            }),
                            Err(e) => {
                                let _ = std::fs::remove_dir_all(&dir);
                                fail(app, e);
                                finish(app);
                                return;
                            }
                        }
                    }

                    let _ = std::fs::remove_dir_all(&dir);
                    let _ = app.emit(
                        "job://fetched",
                        FetchedEvent {
                            id: id.clone(),
                            title: title.clone(),
                            items: placed,
                        },
                    );
                    finish(app);
                    return;
                }
            }

            match fetched {
                Ok(Ok(items)) => {
                    let item = items.into_iter().next().expect("one item");
                    (item.path, Some(title))
                }
                Ok(Err(_cancelled)) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    let _ = app.emit("job://cancelled", CancelledEvent { id: id.clone() });
                    finish(app);
                    return;
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    fail(app, e);
                    finish(app);
                    return;
                }
            }
        }
    };

    let cleanup = |work_dir: &Option<PathBuf>| {
        if let Some(dir) = work_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    };

    let info = match ffmpeg::probe(&state.bins, &input) {
        Ok(i) => i,
        Err(e) => {
            cleanup(&work_dir);
            fail(app, e);
            finish(app);
            return;
        }
    };

    // Two ways to skip encoding: the file already fits, or it came from a link
    // and the user asked to be left alone with it.
    let already_fits = encode::can_pass_through(&info, &settings, &edits, &input);
    let hold_download = from_url && !settings.auto_compress_downloads;

    if already_fits || hold_download {
        // A download lives in a temp folder, so it has to be moved somewhere
        // real. A file the user already had stays exactly where it is.
        let placed = if from_url {
            match encode::place_untouched(&input, &settings, name_hint.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    cleanup(&work_dir);
                    fail(app, e);
                    finish(app);
                    return;
                }
            }
        } else {
            input.clone()
        };

        cleanup(&work_dir);

        let bytes = std::fs::metadata(&placed)
            .map(|m| m.len())
            .unwrap_or(info.size_bytes);

        let note = if already_fits {
            format!(
                "Already under {} — left untouched.",
                format_size(settings.target_bytes)
            )
        } else {
            "Downloaded. Compress it from the editor when you're ready.".into()
        };

        emit(1.0, "Done");
        let _ = app.emit(
            "job://done",
            DoneEvent {
                id: id.clone(),
                output: placed.to_string_lossy().into_owned(),
                final_bytes: bytes,
                original_bytes: info.size_bytes,
                attempts: 0,
                notes: vec![note],
                width: info.width,
                height: info.height,
                passed_through: true,
            },
        );
        finish(app);
        return;
    }

    // Downloading already used the first stretch of the bar, so encoding gets
    // what's left rather than starting over from zero.
    let (base, span) = if from_url { (0.45, 0.55) } else { (0.0, 1.0) };

    // ffmpeg reports progress several times a second; the UI doesn't need
    // anything like that many repaints.
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut last_stage = String::new();

    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &settings,
        edits: &edits,
        name_hint: name_hint.as_deref(),
    };

    let result = encode::run_job(&state.bins, &task, &cancel, |progress, stage| {
        let stage_changed = stage != last_stage;
        if stage_changed || last_emit.elapsed() >= Duration::from_millis(80) {
            last_emit = Instant::now();
            last_stage = stage.to_string();
            emit(base + progress * span, stage);
        }
    });

    cleanup(&work_dir);

    match result {
        Ok(Ok(outcome)) => {
            let _ = app.emit(
                "job://done",
                DoneEvent {
                    id: id.clone(),
                    output: outcome.output.to_string_lossy().into_owned(),
                    final_bytes: outcome.final_bytes,
                    original_bytes: info.size_bytes,
                    attempts: outcome.attempts,
                    notes: outcome.plan.notes,
                    width: outcome.plan.width,
                    height: outcome.plan.height,
                    passed_through: false,
                },
            );
        }
        Ok(Err(_cancelled)) => {
            let _ = app.emit("job://cancelled", CancelledEvent { id: id.clone() });
        }
        Err(error) => {
            fail(app, error);
        }
    }

    finish(app);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (tx, rx) = channel::<QueuedJob>();

    // Starts true to match the frontend's default of a pinned window. If this
    // began false, the window could hide itself during the gap before the
    // webview finishes booting and pushes the real preference across — which
    // looks exactly like the app failing to launch.
    let suppress_hide = Arc::new(AtomicBool::new(true));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // A second launch should hand its payload to the running instance
            // and surface the window, not open a rival copy. A browser firing
            // `nitrate://` arrives here too, as an ordinary argument.
            let files = files_from_args(argv.iter().cloned());
            if !files.is_empty() {
                app.state::<AppState>()
                    .pending_files
                    .lock()
                    .unwrap()
                    .extend(files);
                let _ = app.emit("files://open", ());
            }

            if let Some(complaint) = accept_links(app, links_from_args(argv.into_iter())) {
                let _ = app.emit("links://refused", complaint);
            }

            tray::show_popup(app, None);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // Lets a finished video be dragged out of the window into Explorer or
        // Discord. The webview can't start a file drag on its own — that needs
        // the OS drag-and-drop machinery, which this reaches.
        .plugin(tauri_plugin_drag::init())
        .manage(AppState {
            bins: ffmpeg::resolve(),
            cancels: Mutex::new(HashMap::new()),
            queue: tx,
            pending_files: Mutex::new(files_from_args(std::env::args())),
            pending_links: Mutex::new(Vec::new()),
            link_limit: Mutex::new(deeplink::RateLimit::default()),
            show_on_ready: Arc::new(AtomicBool::new(false)),
            suppress_hide: Arc::clone(&suppress_hide),
        })
        .invoke_handler(tauri::generate_handler![
            inspect_file,
            preview_plan,
            start_job,
            cancel_job,
            capabilities,
            set_suppress_hide,
            hide_window,
            quit_app,
            take_pending_files,
            take_pending_links,
            frontend_ready,
            inspect_url,
            frame_at,
            filmstrip,
            set_editor_size,
            allow_preview,
            preview_proxy,
            open_video,
            copy_video_to_clipboard,
            window_drag_started,
        ])
        .setup(move |app| {
            tray::setup(app.handle(), Arc::clone(&suppress_hide))?;

            // Video encoding is already parallel internally, so a couple of
            // concurrent jobs saturates most machines. More would just make
            // every job slower without finishing the batch any sooner.
            let workers = std::thread::available_parallelism()
                .map(|n| (n.get() / 4).clamp(1, 3))
                .unwrap_or(1);
            spawn_workers(app.handle().clone(), rx, workers);

            // A link the app was launched *with*, before any window existed.
            let startup_links = links_from_args(std::env::args());
            if let Some(complaint) = accept_links(app.handle(), startup_links) {
                let _ = app.handle().emit("links://refused", complaint);
            }

            // Registering at runtime covers development, where the installer
            // hasn't written the registry entry.
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register(deeplink::SCHEME);

                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    let raw: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                    if let Some(complaint) = accept_links(&handle, raw) {
                        let _ = handle.emit("links://refused", complaint);
                    }
                });
            }

            // Sites change constantly, so a downloader left alone goes stale.
            // Off the main thread and silent about failure — there's nobody to
            // tell at startup, and a slightly old copy still works for most.
            if let Ok(data_dir) = app_data_dir(app.handle()) {
                std::thread::spawn(move || download::refresh_if_stale(&data_dir));
            }

            // Whether the window appears at startup is decided in
            // `frontend_ready`, once there's actually something to show.
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Nitrate");
}

#[cfg(all(test, windows))]
mod clipboard_tests {
    use super::copy_video_to_clipboard;

    /// Copies a file, then reads the clipboard back to see what landed there.
    ///
    /// Worth testing properly because the failure mode is silent and specific:
    /// putting the *path* on the clipboard as text also "works", right up until
    /// someone pastes into Discord and gets `C:\Users\...\clip.mp4` as a
    /// message instead of the video.
    #[test]
    fn copying_a_video_puts_the_file_on_the_clipboard() {
        let dir = std::env::temp_dir().join(format!("nitrate-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mp4");
        std::fs::write(&file, b"not really a video, but it is a file").unwrap();

        let result = copy_video_to_clipboard(file.to_string_lossy().into_owned());

        // A machine with no window station — some CI containers — can't open
        // the clipboard at all. That's the environment failing, not the code,
        // and it shouldn't turn the suite red.
        if let Err(message) = &result {
            if message.contains("Couldn't open the clipboard") {
                eprintln!("skipped: no clipboard on this machine ({message})");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }
        result.expect("copying should succeed");

        let listed: Vec<String> = clipboard_win::get_clipboard(clipboard_win::formats::FileList)
            .expect("the clipboard should now hold a file list");

        assert_eq!(listed.len(), 1, "expected exactly one file, got {listed:?}");
        assert!(
            std::path::Path::new(&listed[0]) == file,
            "clipboard holds {:?}, expected {:?}",
            listed[0],
            file
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_a_file_that_is_not_there() {
        let missing = std::env::temp_dir().join("nitrate-does-not-exist.mp4");
        let result = copy_video_to_clipboard(missing.to_string_lossy().into_owned());
        assert!(result.is_err(), "a missing file should not be copied");
    }
}
