//! Nitrate — compress any video to fit a target file size.

// Public so the integration tests can drive the encoder without a running app.
pub mod encode;
pub mod ffmpeg;
mod tray;

use base64::Engine as _;
use encode::Settings;
use ffmpeg::{Binaries, MediaInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct QueuedJob {
    id: String,
    input: PathBuf,
    settings: Settings,
    cancel: Arc<AtomicBool>,
}

pub struct AppState {
    bins: Binaries,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    queue: Sender<QueuedJob>,
    /// Files named on the command line, held until the webview is ready to
    /// receive them. Launching via "Open with" beats the frontend to startup.
    pending_files: Mutex<Vec<String>>,
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
        ffmpeg::thumbnail(&state.bins, &input, &out, info.duration * 0.25).ok()?;
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
/// "1080p → 720p, 1.4 Mbps" preview on each card.
#[tauri::command]
async fn preview_plan(
    state: State<'_, AppState>,
    path: String,
    settings: Settings,
) -> Result<encode::Plan, String> {
    let input = PathBuf::from(&path);
    let info = ffmpeg::probe(&state.bins, &input)?;
    encode::plan(&info, &settings, &state.bins)
}

#[tauri::command]
async fn start_job(
    state: State<'_, AppState>,
    id: String,
    path: String,
    settings: Settings,
) -> Result<(), String> {
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
            input: PathBuf::from(path),
            settings,
            cancel,
        })
        .map_err(|_| "The encoding queue has shut down.".to_string())
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
        input,
        settings,
        cancel,
    } = job;

    let finish = |app: &AppHandle| {
        app.state::<AppState>().cancels.lock().unwrap().remove(&id);
    };

    if cancel.load(Ordering::Relaxed) {
        let _ = app.emit("job://cancelled", CancelledEvent { id: id.clone() });
        finish(app);
        return;
    }

    let _ = app.emit(
        "job://progress",
        ProgressEvent {
            id: &id,
            progress: 0.0,
            stage: "Preparing",
        },
    );

    let info = match ffmpeg::probe(&state.bins, &input) {
        Ok(i) => i,
        Err(e) => {
            let _ = app.emit(
                "job://failed",
                FailedEvent {
                    id: id.clone(),
                    error: e,
                },
            );
            finish(app);
            return;
        }
    };

    // ffmpeg reports progress several times a second; the UI doesn't need
    // anything like that many repaints.
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut last_stage = String::new();

    let result = encode::run_job(
        &state.bins,
        &input,
        &settings,
        &info,
        &cancel,
        |progress, stage| {
            let stage_changed = stage != last_stage;
            if stage_changed || last_emit.elapsed() >= Duration::from_millis(80) {
                last_emit = Instant::now();
                last_stage = stage.to_string();
                let _ = app.emit(
                    "job://progress",
                    ProgressEvent {
                        id: &id,
                        progress,
                        stage,
                    },
                );
            }
        },
    );

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
                },
            );
        }
        Ok(Err(_cancelled)) => {
            let _ = app.emit("job://cancelled", CancelledEvent { id: id.clone() });
        }
        Err(error) => {
            let _ = app.emit(
                "job://failed",
                FailedEvent {
                    id: id.clone(),
                    error,
                },
            );
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
            // A second launch should hand its files to the running instance
            // and surface the window, not open a rival copy.
            let files = files_from_args(argv.into_iter());
            if !files.is_empty() {
                app.state::<AppState>()
                    .pending_files
                    .lock()
                    .unwrap()
                    .extend(files);
                let _ = app.emit("files://open", ());
            }
            tray::show_popup(app, None);
        }))
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
        .manage(AppState {
            bins: ffmpeg::resolve(),
            cancels: Mutex::new(HashMap::new()),
            queue: tx,
            pending_files: Mutex::new(files_from_args(std::env::args())),
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
            frontend_ready,
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

            // Whether the window appears at startup is decided in
            // `frontend_ready`, once there's actually something to show.
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Nitrate");
}
