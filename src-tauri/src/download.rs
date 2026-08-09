//! Fetching videos from a pasted link, via yt-dlp.
//!
//! yt-dlp isn't bundled. Sites change constantly and a copy frozen at release
//! time goes stale within weeks, so it's fetched on first use and refreshed in
//! the background. That costs nothing in practice: you can only use it when
//! you're online anyway.

use crate::ffmpeg::Binaries;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long before we go looking for a newer yt-dlp.
const REFRESH_AFTER: Duration = Duration::from_secs(60 * 60 * 24 * 7);

/// How long to let the probe think before queueing the link anyway.
const PROBE_TIMEOUT: Duration = Duration::from_secs(25);

fn asset_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else {
        "yt-dlp_linux"
    }
}

fn local_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

pub fn binary_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(local_name())
}

fn stamp_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join("yt-dlp.stamp")
}

// ---------------------------------------------------------------------------
// gallery-dl, for the stills yt-dlp can't reach
// ---------------------------------------------------------------------------

/// Pinned rather than "latest".
///
/// gallery-dl moved off GitHub to Codeberg — the GitHub releases still exist
/// but carry no assets at all, so the obvious URL silently returns a nine-byte
/// "Not found" that looks like a downloaded program until you run it. Pinning
/// also means the checksum below stays meaningful.
const GALLERY_VERSION: &str = "1.32.9";

/// SHA-256 of the pinned Windows build, from the project's own SHA256SUMS.
///
/// Checked because this is an executable arriving over the network, and an
/// unverified one is exactly the shape of thing that gets an app quarantined.
/// yt-dlp is fetched without this today, which is worth fixing separately.
const GALLERY_SHA256_WINDOWS: &str =
    "a3f7eb5ad0fdb6176dd0044b583ced7e7d918f27221b6f729825d243daff44fe";

fn gallery_asset() -> &'static str {
    if cfg!(windows) {
        "gallery-dl.exe"
    } else {
        "gallery-dl.bin"
    }
}

pub fn gallery_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(gallery_asset())
}

/// Downloads gallery-dl if it's missing. Returns the path either way.
pub fn ensure_gallery(data_dir: &Path) -> Result<PathBuf, String> {
    let target = gallery_path(data_dir);
    if target.is_file() {
        return Ok(target);
    }

    let dir = target
        .parent()
        .ok_or("Couldn't work out where to put the image downloader.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;

    let url = format!(
        "https://codeberg.org/mikf/gallery-dl/releases/download/v{GALLERY_VERSION}/{}",
        gallery_asset()
    );

    let bytes = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("nitrate")
        .build()
        .map_err(|e| format!("Couldn't start the download: {e}"))?
        .get(&url)
        .send()
        .map_err(|e| format!("Couldn't reach Codeberg to fetch the image downloader: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Codeberg refused the request: {e}"))?
        .bytes()
        .map_err(|e| format!("The image downloader transfer failed: {e}"))?;

    if cfg!(windows) {
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != GALLERY_SHA256_WINDOWS {
            return Err(format!(
                "The image downloader didn't match its published checksum \
                 (expected {GALLERY_SHA256_WINDOWS}, got {digest}). Nothing was installed."
            ));
        }
    }

    // Written beside the target and renamed, so an interrupted fetch can't
    // leave a half-written binary that looks installed.
    let temp = target.with_extension("part");
    std::fs::write(&temp, &bytes)
        .map_err(|e| format!("Couldn't save the image downloader: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755));
    }

    std::fs::rename(&temp, &target)
        .map_err(|e| format!("Couldn't install the image downloader: {e}"))?;

    Ok(target)
}

fn base_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

// ---------------------------------------------------------------------------
// Getting hold of yt-dlp
// ---------------------------------------------------------------------------

/// Downloads yt-dlp if it's missing. Returns the path either way.
pub fn ensure(data_dir: &Path, mut on_progress: impl FnMut(f64)) -> Result<PathBuf, String> {
    let target = binary_path(data_dir);
    if target.is_file() {
        return Ok(target);
    }

    let dir = target
        .parent()
        .ok_or("Couldn't work out where to put the downloader.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;

    let url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/{}",
        asset_name()
    );

    on_progress(0.0);
    let bytes = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .user_agent("nitrate")
        .build()
        .map_err(|e| format!("Couldn't start the download: {e}"))?
        .get(&url)
        .send()
        .map_err(|e| format!("Couldn't reach GitHub to fetch the downloader: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub refused the downloader request: {e}"))?
        .bytes()
        .map_err(|e| format!("The downloader transfer failed: {e}"))?;

    // Write beside the target then rename, so an interrupted fetch can't leave
    // a half-written binary that looks installed.
    let temp = target.with_extension("part");
    std::fs::write(&temp, &bytes).map_err(|e| format!("Couldn't save the downloader: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755));
    }

    std::fs::rename(&temp, &target).map_err(|e| format!("Couldn't install the downloader: {e}"))?;
    touch_stamp(data_dir);
    on_progress(1.0);

    Ok(target)
}

fn touch_stamp(data_dir: &Path) {
    let _ = std::fs::write(
        stamp_path(data_dir),
        format!(
            "{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        ),
    );
}

fn is_stale(data_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(stamp_path(data_dir)) else {
        return true;
    };
    let Ok(then) = text.trim().parse::<u64>() else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(then) > REFRESH_AFTER.as_secs()
}

/// Asks yt-dlp to update itself, but only if it hasn't been checked recently.
///
/// Failure is deliberately silent — a stale downloader still works for most
/// sites, and this runs in the background where there's nobody to tell.
pub fn refresh_if_stale(data_dir: &Path) {
    let bin = binary_path(data_dir);
    if !bin.is_file() || !is_stale(data_dir) {
        return;
    }

    let ok = base_command(&bin)
        .arg("--update")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if ok {
        touch_stamp(data_dir);
    }
}

// ---------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------

/// Anything yt-dlp might plausibly accept. It supports well over a thousand
/// sites, so rather than keep a list, take any web link and let yt-dlp be the
/// judge — it gives a better error than we could guess at.
pub fn looks_like_url(text: &str) -> bool {
    let text = text.trim();
    if text.contains(char::is_whitespace) || text.len() > 2048 {
        return false;
    }
    (text.starts_with("http://") || text.starts_with("https://")) && text.len() > 10
}

/// A friendly name for the card while we've nothing better to show.
pub fn site_name(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_lowercase();

    let pretty = match () {
        _ if host.contains("youtu") => "YouTube",
        _ if host.contains("twitter") || host == "x.com" || host.ends_with(".x.com") => "X",
        _ if host.contains("instagram") => "Instagram",
        _ if host.contains("reddit") || host.contains("redd.it") => "Reddit",
        _ if host.contains("twitch") => "Twitch",
        _ if host.contains("tiktok") => "TikTok",
        _ => return host,
    };
    pretty.to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlInfo {
    pub title: String,
    pub duration: Option<f64>,
    pub site: String,
    pub webpage_url: String,
}

#[derive(Deserialize)]
struct DumpJson {
    title: Option<String>,
    duration: Option<f64>,
    extractor_key: Option<String>,
    webpage_url: Option<String>,
    #[serde(default)]
    is_live: bool,
}

/// Spaces out metadata requests so we don't rate-limit ourselves.
///
/// Pasting five links fired five yt-dlp processes at the same instant, and
/// Reddit answered the later ones with 429 — a limit we inflicted on ourselves,
/// which then surfaced as "couldn't download the images" on posts that were
/// perfectly fine. They now go out one at a time, spaced.
pub(crate) fn pace() {
    static GATE: Mutex<Option<Instant>> = Mutex::new(None);
    const GAP: Duration = Duration::from_millis(800);

    // Held across the sleep on purpose: that is what serialises them.
    let mut last = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = *last {
        let since = previous.elapsed();
        if since < GAP {
            std::thread::sleep(GAP - since);
        }
    }
    *last = Some(Instant::now());
}

/// Reads a link's metadata without downloading it.
pub fn probe_url(bin: &Path, url: &str) -> Result<UrlInfo, String> {
    // A share link is resolved before anything else looks at it, so everything
    // downstream — including the address stored with the job — is the real post.
    let resolved = crate::media::canonical_url(url);
    let url = resolved.as_str();

    // Same shortcut the fetch takes: a Reddit post of only images is read from
    // its page. Going to yt-dlp first meant waiting out the API's rate limit
    // before reaching a route that was never blocked — and this is also the
    // only path that knows the post's title, so the card can stop saying
    // "Reddit" and say what the post is called.
    if let Some(page) = crate::media::reddit_page(url) {
        if !page.images.is_empty() && !page.has_video {
            return Ok(UrlInfo {
                title: page.title,
                duration: None,
                site: "Reddit".to_string(),
                webpage_url: url.to_string(),
            });
        }
    }

    pace();

    let mut child = base_command(bin)
        .args([
            "--no-warnings",
            "--no-playlist",
            "--dump-single-json",
            // A post made of photos has no video formats, and without this
            // yt-dlp calls that fatal. This runs before anything is fetched, so
            // treating it as fatal here rejected every image post at the door —
            // the media pipeline that handles them never got a chance to look.
            "--ignore-no-formats-error",
            "--socket-timeout",
            "20",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run the downloader: {e}"))?;

    // Drained on a thread rather than left in the pipe. A post with several
    // items prints a lot of JSON, and a full pipe blocks the child forever —
    // which would turn the timeout below into the very hang it exists to stop.
    let piped = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut out) = piped {
            use std::io::Read as _;
            let _ = out.read_to_string(&mut text);
        }
        let _ = tx.send(text);
    });

    // A wall-clock limit on the whole thing. `--socket-timeout` caps individual
    // sockets, not the command, and on a Reddit gallery yt-dlp can sit there
    // enumerating long past the point of usefulness — the link then sat on
    // "Reading link…" indefinitely, with nothing above it willing to give up.
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                break true;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("The downloader didn't exit cleanly: {e}")),
        }
    };

    // Not an error. What the post holds is worked out when it's fetched, and
    // that path handles everything this one was still thinking about — so a
    // slow extractor delays the answer rather than refusing the link.
    if timed_out {
        return Ok(UrlInfo {
            title: site_name(url).to_string(),
            duration: None,
            site: site_name(url).to_string(),
            webpage_url: url.to_string(),
        });
    }

    let stdout = rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| String::new());
    let output = child
        .wait_with_output()
        .map_err(|e| format!("The downloader didn't exit cleanly: {e}"))?;
    let usable = stdout.trim_start().starts_with('{');

    let stderr = String::from_utf8_lossy(&output.stderr);

    // A Reddit photo post fails here by design, and the failure is the answer.
    //
    // yt-dlp resolves the post, finds the image, and then refuses to fetch a
    // still — naming the address in the refusal, which is exactly how the media
    // pipeline gets it. Reading that as "Unsupported URL" turned every Reddit
    // photo post away at the door with "That site isn't supported.", so the
    // pipeline that handles them was never reached by anyone.
    if !usable && !crate::media::reddit_images(&stderr).is_empty() {
        return Ok(UrlInfo {
            title: site_name(url).to_string(),
            duration: None,
            site: site_name(url).to_string(),
            webpage_url: url.to_string(),
        });
    }

    // Only a failure if there's nothing to read. yt-dlp still reports a
    // non-zero exit for a photo post while printing perfectly good metadata,
    // so the JSON is what decides, not the exit code.
    if !output.status.success() && !usable {
        return Err(summarise_yt_dlp_error(&stderr));
    }

    // Some extractors say nothing at all about a photo post. It's still a link
    // worth queueing — what it holds is worked out when it's fetched.
    if !usable {
        return Ok(UrlInfo {
            title: site_name(url).to_string(),
            duration: None,
            site: site_name(url).to_string(),
            webpage_url: url.to_string(),
        });
    }

    let parsed: DumpJson = serde_json::from_str(&stdout)
        .map_err(|_| "The downloader returned something unexpected for that link.".to_string())?;

    if parsed.is_live {
        return Err("That's a live stream — it has no fixed length to compress.".into());
    }

    Ok(UrlInfo {
        title: parsed
            .title
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Video".into()),
        duration: parsed.duration.filter(|d| *d > 0.0),
        site: parsed
            .extractor_key
            .unwrap_or_else(|| site_name(url))
            .to_string(),
        webpage_url: parsed.webpage_url.unwrap_or_else(|| url.to_string()),
    })
}

/// Turns yt-dlp's stderr into one line a person can act on.
fn summarise_yt_dlp_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .find(|l| l.contains("ERROR:"))
        .unwrap_or_else(|| stderr.lines().last().unwrap_or(""))
        .trim();

    let cleaned = line.replace("ERROR: ", "");
    let lower = cleaned.to_lowercase();

    // Reddit in particular refuses in bursts, and yt-dlp retries with backoff
    // for a long time before surfacing it — so this arrives after a wait that
    // looks like a hang, with a raw HTTPError that explains nothing to anyone.
    if lower.contains("429") || lower.contains("too many requests") {
        "That site is asking us to slow down. Wait a minute and try again.".into()
    } else if lower.contains("not currently live") || lower.contains("live event will begin") {
        "That channel isn't live right now.".into()
    } else if lower.contains("live stream") && lower.contains("not available") {
        "That live stream can't be downloaded — wait until it has finished.".into()
    } else if lower.starts_with("unsupported url") {
        // Anchored to the start. Matching it anywhere labelled a Reddit link
        // "not supported" because the phrase appeared somewhere inside a
        // longer failure, which sent people looking for the wrong problem.
        "That site isn't supported.".into()
    } else if lower.contains("private") || lower.contains("login") || lower.contains("cookies") {
        "That video is private or needs a sign-in.".into()
    } else if lower.contains("unavailable") || lower.contains("removed") {
        "That video isn't available any more.".into()
    } else if lower.contains("drm") {
        "That video is DRM-protected and can't be downloaded.".into()
    } else if cleaned.is_empty() {
        "The downloader couldn't fetch that link.".into()
    } else {
        cleaned.chars().take(180).collect()
    }
}

// ---------------------------------------------------------------------------
// Downloading
// ---------------------------------------------------------------------------

pub struct Cancelled;

/// Pulls the video into `work_dir` and returns the file it produced.
///
/// Capped by height because the whole point is to end up at a few megabytes —
/// fetching 4K to throw almost all of it away wastes the user's bandwidth and
/// their time.
pub fn fetch(
    bin: &Path,
    bins: &Binaries,
    url: &str,
    work_dir: &Path,
    max_height: u32,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64),
) -> Result<Result<PathBuf, Cancelled>, String> {
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("Couldn't create a working folder: {e}"))?;

    // The full path to the binary, not its folder: during development the
    // sidecars carry target-triple suffixes, so yt-dlp looking for a plain
    // "ffmpeg.exe" in that directory finds nothing. It then skips merging
    // *and still exits zero*, leaving separate video and audio files behind.
    let ffmpeg_path = bins.ffmpeg.clone();

    let format = format!(
        "bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b",
        h = max_height
    );

    let mut child = base_command(bin)
        .args(["--no-warnings", "--no-playlist", "--newline"])
        .args(["--socket-timeout", "20", "--retries", "3"])
        .args(["-f", &format])
        .args(["--merge-output-format", "mp4"])
        // A fixed output name means we don't have to parse yt-dlp's filename
        // rules to find what it produced.
        .args(["-o", &format!("{}/source.%(ext)s", work_dir.display())])
        .arg("--ffmpeg-location")
        .arg(&ffmpeg_path)
        // Machine-readable progress on stdout, rather than scraping the
        // human-facing progress bar.
        .args([
            "--progress-template",
            "download:NITRATE|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start the downloader: {e}"))?;

    let stderr_tail = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let stderr_handle = {
        let tail = Arc::clone(&stderr_tail);
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            let Some(stderr) = stderr else { return };
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut tail = tail.lock().unwrap();
                tail.push(line);
                if tail.len() > 30 {
                    tail.remove(0);
                }
            }
        })
    };

    let mut was_cancelled = false;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                was_cancelled = true;
                break;
            }
            if let Some(fraction) = parse_progress(&line) {
                on_progress(fraction);
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| format!("The downloader didn't exit cleanly: {e}"))?;
    let _ = stderr_handle.join();

    if was_cancelled || cancel.load(Ordering::Relaxed) {
        return Ok(Err(Cancelled));
    }

    if !status.success() {
        let tail = stderr_tail.lock().unwrap();
        return Err(summarise_yt_dlp_error(&tail.join("\n")));
    }

    on_progress(1.0);
    find_downloaded(work_dir).map(Ok)
}

/// `download:NITRATE|<done>|<total>|<estimate>`
fn parse_progress(line: &str) -> Option<f64> {
    let rest = line.trim().strip_prefix("NITRATE|")?;
    let mut parts = rest.split('|');
    let done: f64 = parts.next()?.trim().parse().ok()?;

    // yt-dlp prints "NA" for whichever total it doesn't know.
    let total = parts
        .next()
        .and_then(|t| t.trim().parse::<f64>().ok())
        .or_else(|| parts.next().and_then(|t| t.trim().parse::<f64>().ok()))?;

    if total <= 0.0 {
        return None;
    }
    Some((done / total).clamp(0.0, 1.0))
}

/// Containers that carry video. Anything else yt-dlp leaves behind is an
/// audio-only track from a merge that didn't happen.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "ts", "ogv",
];

fn find_downloaded(work_dir: &Path) -> Result<PathBuf, String> {
    let entries =
        std::fs::read_dir(work_dir).map_err(|e| format!("Couldn't read the download: {e}"))?;

    let mut best: Option<(u64, PathBuf)> = None;
    let mut saw_any = false;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Skip leftovers from interrupted fragments.
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".part") || name.ends_with(".ytdl") {
            continue;
        }
        saw_any = true;

        // Picking purely by size would choose an audio track over the video
        // whenever a merge failed, which then fails much later with a
        // confusing "no video stream".
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !VIDEO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(b, _)| size > *b) {
            best = Some((size, path));
        }
    }

    match best {
        Some((_, path)) => Ok(path),
        None if saw_any => {
            Err("The download couldn't be assembled into a video — the audio and video parts were never merged.".into())
        }
        None => Err("The download finished but produced no file.".into()),
    }
}
