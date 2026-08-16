//! Getting every piece of media out of a social post, not just the video.
//!
//! A post can hold several things at once, and they aren't all the same kind.
//! What comes back keeps whatever it already was: a photo stays a photo, a
//! video stays a video, and something the site itself calls a GIF comes back as
//! a real `.gif` rather than the MP4 it's stored as.
//!
//! Which tool can reach what was measured rather than assumed:
//!
//! | Site      | Photos                  | GIFs                     |
//! |-----------|-------------------------|--------------------------|
//! | Instagram | yt-dlp, no login needed | as video                 |
//! | X         | gallery-dl              | yt-dlp, `tweet_video`    |
//! | Reddit    | yt-dlp, via its refusal | as video                 |
//!
//! Instagram serves a photo post's images through the field yt-dlp reports as
//! "thumbnails", and the largest is the genuine original — 1080x1350 on the
//! post this was tested against, not a preview. X hands back nothing at all for
//! a photo tweet, which is what gallery-dl is here for.
//!
//! Reddit takes the strangest route of the three. Its data API is closed to us —
//! gallery-dl gets the web page where it expects JSON, and the public .json
//! endpoint refuses too — but yt-dlp still resolves a post and then declines to
//! download the still it found, naming the address in the refusal. That address
//! is on i.redd.it, which serves it over ordinary HTTP. The block is on reading
//! Reddit's API, not on reaching its images.

use crate::ffmpeg::{Binaries, Cancelled};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn base_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// What a piece of media is, as the site itself describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Photo,
    /// Shown as a GIF by the site, whatever it's stored as underneath.
    Gif,
    Video,
}

impl MediaKind {
    pub fn is_still(self) -> bool {
        self == MediaKind::Photo
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub path: PathBuf,
    pub kind: MediaKind,
}

// ---------------------------------------------------------------------------
// Reading what a post holds
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    formats: Vec<Format>,
    #[serde(default)]
    thumbnails: Vec<Thumbnail>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct Thumbnail {
    url: String,
}

/// One thing to go and get, once we know what kind it is.
enum Planned {
    /// A still, already resolved to a direct image URL.
    Photo(String),
    /// Something yt-dlp can download itself, at this position in the post.
    Playable { index: usize, gif: bool },
}

/// Twitter stores animated GIFs as MP4 like everything else, but files them
/// under a different path — real videos land in `ext_tw_video` or
/// `amplify_video`. It's the site's own classification rather than a guess from
/// duration or a missing audio track, both of which ordinary silent clips share.
fn looks_like_gif(entry: &Entry) -> bool {
    entry
        .formats
        .iter()
        .filter_map(|f| f.url.as_deref())
        .chain(entry.url.as_deref())
        .any(|u| u.contains("tweet_video"))
}

/// How long to let yt-dlp think about what a post holds.
///
/// `--socket-timeout` caps one socket, not the command. A rate-limited Reddit
/// sends yt-dlp into retries with backoff that ran past two minutes when this
/// was measured, which is what left a pasted link sitting on "Reading…".
const PLAN_TIMEOUT: Duration = Duration::from_secs(30);

/// What a post holds, and why we couldn't tell when we couldn't.
struct Plan {
    items: Vec<Planned>,
    /// Kept for the case where nothing came back and someone has to be told
    /// something better than "no".
    trouble: Option<String>,
}

/// Runs a metadata command under a wall-clock cap.
///
/// Both pipes are drained on their own threads: a post with several items
/// prints enough JSON to fill one, and a full pipe blocks the child forever —
/// which would turn the cap below into the very hang it exists to prevent.
fn run_capped(cmd: &mut Command, limit: Duration) -> Option<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let mut pipes = (child.stdout.take(), child.stderr.take());
    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();

    for (handle, tx) in [
        (
            pipes
                .0
                .take()
                .map(|h| Box::new(h) as Box<dyn std::io::Read + Send>),
            tx_out,
        ),
        (
            pipes
                .1
                .take()
                .map(|h| Box::new(h) as Box<dyn std::io::Read + Send>),
            tx_err,
        ),
    ] {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut h) = handle {
                let _ = h.read_to_end(&mut buf);
            }
            let _ = tx.send(buf);
        });
    }

    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let grace = Duration::from_secs(5);
                return Some(Output {
                    status,
                    stdout: rx_out.recv_timeout(grace).unwrap_or_default(),
                    stderr: rx_err.recv_timeout(grace).unwrap_or_default(),
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
}

/// Asks yt-dlp what's in a post without downloading any of it.
fn plan_from_yt_dlp(yt: &Path, url: &str) -> Plan {
    // A Reddit post of nothing but images is answered from its page, which the
    // API rate limit doesn't touch. It also names every image in a gallery,
    // and it takes about a second where the throttled API takes a minute.
    if let Some(page) = reddit_page(url) {
        if !page.images.is_empty() && !page.has_video {
            return Plan {
                items: page.images.into_iter().map(Planned::Photo).collect(),
                trouble: None,
            };
        }
    }

    crate::download::pace();

    let mut cmd = base_command(yt);
    cmd.args([
        "--dump-json",
        // Instagram photo posts have no video formats at all, and without
        // this yt-dlp treats that as fatal and prints nothing usable.
        "--ignore-no-formats-error",
        "--no-warnings",
        "--socket-timeout",
        "20",
    ])
    .arg(url);

    let Some(output) = run_capped(&mut cmd, PLAN_TIMEOUT) else {
        return Plan {
            items: Vec::new(),
            trouble: Some("That post took too long to read. Try again in a moment.".into()),
        };
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut planned = Vec::new();

    // Reddit says what it holds while refusing to hand it over.
    //
    // yt-dlp resolves the post, finds the image, and then gives up with
    // "Unsupported URL: https://www.reddit.com/media?url=https%3A%2F%2F..."
    // because downloading a still isn't its job. The address of the image is
    // right there in the complaint, and i.redd.it serves it over plain HTTP —
    // so the refusal is only about who downloads it, not about access.
    for url in reddit_images(&String::from_utf8_lossy(&output.stderr)) {
        planned.push(Planned::Photo(url));
    }

    for (index, line) in text.lines().filter(|l| l.starts_with('{')).enumerate() {
        let Ok(entry) = serde_json::from_str::<Entry>(line) else {
            continue;
        };

        if !entry.formats.is_empty() {
            planned.push(Planned::Playable {
                index,
                gif: looks_like_gif(&entry),
            });
        } else if let Some(best) = entry.thumbnails.last() {
            // yt-dlp lists thumbnails worst first, so the last is the largest —
            // and for a photo post that is the photo.
            planned.push(Planned::Photo(best.url.clone()));
        }
    }

    // Only consulted if nothing was found. A post can hit a rate limit on one
    // of its items and still hand over the rest, and that isn't worth a warning.
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    let trouble = if stderr.contains("429") || stderr.contains("too many requests") {
        Some("That site is asking us to slow down. Wait a minute and try again.".into())
    } else {
        None
    };

    Plan {
        items: planned,
        trouble,
    }
}

/// Pulls image addresses out of yt-dlp's complaints about a Reddit post.
///
/// They arrive wrapped in a `reddit.com/media?url=` redirect with the real
/// address percent-encoded inside it.
pub(crate) fn reddit_images(stderr: &str) -> Vec<String> {
    let mut found = Vec::new();

    for part in stderr.split("media?url=").skip(1) {
        let encoded = part
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(['"', '\'', ')', '.']);
        let url = percent_decode(encoded);

        // Only Reddit's own image hosts, so a stray link in an error message
        // can't turn into something we go and fetch.
        let host_ok =
            url.starts_with("https://i.redd.it/") || url.starts_with("https://preview.redd.it/");

        if host_ok && !found.contains(&url) {
            found.push(url);
        }
    }

    found
}

/// Enough percent-decoding for a URL inside a query string.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&raw[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("nitrate")
        .build()
        .map_err(|e| format!("Couldn't start the download: {e}"))
}

/// What a Reddit post shows, read from the page instead of the API.
pub struct RedditPage {
    /// The post's own title, which the API path never gets us — cards showed
    /// the bare word "Reddit".
    pub title: String,
    pub images: Vec<String>,
    /// A post holding video is left to yt-dlp, which handles it properly.
    pub has_video: bool,
}

/// Reads a Reddit post from its old front end.
///
/// Reddit rate-limits its data API hard, and yt-dlp then backs off for about a
/// minute — measured repeatedly against a post a browser was serving the whole
/// time. The old front end answers an ordinary request with plain HTML, and it
/// lists every image in a gallery, where the API refusal we otherwise read
/// names only one of them.
pub fn reddit_page(url: &str) -> Option<RedditPage> {
    let path = url.split('?').next().unwrap_or(url);
    if !path.contains("/comments/") {
        return None;
    }
    let (_, rest) = path.split_once("reddit.com/")?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("nitrate")
        .build()
        .ok()?;
    let html = client
        .get(format!("https://old.reddit.com/{rest}"))
        .send()
        .ok()?
        .text()
        .ok()?;

    // Comments carry images of their own and they are not what was asked for.
    // Everything from where they begin is ignored.
    let post = html.split("commentarea").next().unwrap_or(&html);

    let mut images = Vec::new();
    for prefix in ["https://preview.redd.it/", "https://i.redd.it/"] {
        for piece in post.split(prefix).skip(1) {
            let name: String = piece
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                .collect();

            // preview.redd.it refuses a plain request with 403 while i.redd.it
            // serves the same name at full size, so everything is taken from
            // there whichever host the page happened to mention.
            let full = format!("https://i.redd.it/{name}");
            if extension_from_url(&full).is_some_and(|ext| ext != "mp4" && ext != "webm")
                && !images.contains(&full)
            {
                images.push(full);
            }
        }
    }

    Some(RedditPage {
        title: reddit_title(post).unwrap_or_else(|| "Reddit".into()),
        has_video: post.contains("v.redd.it"),
        images,
    })
}

/// The post's title, without the " : subreddit" the page appends to it.
fn reddit_title(html: &str) -> Option<String> {
    let (_, after) = html.split_once("<title>")?;
    let (raw, _) = after.split_once("</title>")?;
    let trimmed = raw.rsplit_once(" : ").map_or(raw, |(before, _)| before);
    let text = trimmed
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(120).collect())
}

/// `reddit.com/r/<sub>/s/<id>` — the shape the Share button produces.
fn is_reddit_share_link(url: &str) -> bool {
    if !url.contains("reddit.com/") {
        return false;
    }

    let path = url.split('?').next().unwrap_or(url).trim_end_matches('/');
    let mut segments = path.rsplit('/');
    // The id, then the marker before it.
    segments.next().is_some_and(|id| !id.is_empty()) && segments.next() == Some("s")
}

/// Turns a Reddit share link into the post it points at.
///
/// The Share button hands out `reddit.com/r/sub/s/<id>`, and that is what most
/// people paste. yt-dlp doesn't resolve it — it sat on the link until the
/// timeout fired, so a shared post simply never worked. Reddit answers a plain
/// request with a 301 naming the real address, so one request settles it.
///
/// Anything unexpected returns the link untouched: this is a shortcut, not a
/// gate, and the pipeline behind it is entitled to its own opinion.
pub fn canonical_url(url: &str) -> String {
    if !is_reddit_share_link(url) {
        return url.to_string();
    }

    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("nitrate")
        .build()
    else {
        return url.to_string();
    };

    let Ok(response) = client.get(url).send() else {
        return url.to_string();
    };

    let Some(location) = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
    else {
        return url.to_string();
    };

    // Drop the share tracking that comes with it, and refuse to be redirected
    // anywhere but Reddit.
    let target = location.split('?').next().unwrap_or(location);
    if target.starts_with("https://www.reddit.com/") {
        target.to_string()
    } else {
        url.to_string()
    }
}

/// Downloads a still straight from the CDN. No tool involved — it's a plain
/// image URL by this point, and the extension it arrives with is the one it
/// keeps.
fn fetch_photo(url: &str, work_dir: &Path, index: usize) -> Result<PathBuf, String> {
    let bytes = http_client()?
        .get(url)
        .send()
        .map_err(|e| format!("Couldn't reach the image: {e}"))?
        .error_for_status()
        .map_err(|e| format!("The image request was refused: {e}"))?
        .bytes()
        .map_err(|e| format!("The image transfer failed: {e}"))?;

    let extension = extension_from_url(url).unwrap_or("jpg");
    let target = work_dir.join(format!("media-{index}.{extension}"));
    std::fs::write(&target, &bytes).map_err(|e| format!("Couldn't save the image: {e}"))?;
    Ok(target)
}

/// What a downloaded file is, from the extension it arrived with.
///
/// The site already decided this. A `.gif` is a GIF wherever it came from, and
/// the promise is that each thing comes back as whatever it already was.
fn kind_from_extension(path: &Path) -> MediaKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4") | Some("webm") | Some("mkv") => MediaKind::Video,
        Some("gif") => MediaKind::Gif,
        _ => MediaKind::Photo,
    }
}

/// The file extension a URL implies, ignoring any query string after it.
fn extension_from_url(url: &str) -> Option<&str> {
    let path = url.split('?').next()?;
    let name = path.rsplit('/').next()?;
    let ext = name.rsplit_once('.')?.1;
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "mp4" | "webm"
    )
    .then_some(ext)
}

/// Turns the MP4 a site stores a "GIF" as into an actual GIF.
///
/// Two passes in one command: `palettegen` works out the best colours for this
/// particular clip and `paletteuse` maps every frame onto them. Letting the
/// encoder pick a fixed palette instead is what makes most converted GIFs look
/// like they escaped from 1998. Bayer dithering because it's stable frame to
/// frame — error diffusion shimmers as its noise pattern shifts, and on a loop
/// that reads as the whole picture crawling.
pub fn to_gif(bins: &Binaries, input: &Path, width: u32) -> Result<PathBuf, String> {
    let target = input.with_extension("gif");

    let status = base_command(&bins.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .arg("-i")
        .arg(input)
        .args([
            "-filter_complex",
            &format!(
                "fps=15,scale={width}:-2:flags=lanczos,split[a][b];\
                 [a]palettegen=max_colors=128:stats_mode=diff[p];\
                 [b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
            ),
            "-an",
            "-loop",
            "0",
        ])
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Couldn't run ffmpeg: {e}"))?;

    if !status.success() {
        return Err("Converting that GIF failed.".into());
    }

    // The MP4 was only ever a carrier for it.
    let _ = std::fs::remove_file(input);
    Ok(target)
}

/// Everything in a post, in the order the post lists it.
///
/// `gallery` is optional: without it, sites yt-dlp can't reach for stills
/// simply come back with nothing rather than failing. `quickjs` is optional in
/// the same spirit — without it YouTube loses its last-resort fallback, and
/// every other site carries on exactly as before.
#[allow(clippy::too_many_arguments)]
pub fn fetch_post(
    yt: &Path,
    gallery: Option<&Path>,
    quickjs: Option<&Path>,
    bins: &Binaries,
    url: &str,
    work_dir: &Path,
    max_height: u32,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64),
) -> Result<Result<Vec<MediaItem>, Cancelled>, String> {
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("Couldn't create a working folder: {e}"))?;

    // Also resolved here, not only at the probe: a link can reach this without
    // having been probed, and the tools below can't follow a share link either.
    let resolved = canonical_url(url);
    let url = resolved.as_str();

    let plan = plan_from_yt_dlp(yt, url);
    let planned = &plan.items;
    let total = planned.len().max(1) as f64;
    let mut items: Vec<MediaItem> = Vec::new();

    for (done, item) in planned.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Err(Cancelled));
        }
        on_progress(done as f64 / total);

        match item {
            Planned::Photo(src) => {
                let path = fetch_photo(src, work_dir, done)?;
                // A Reddit gallery can hold GIFs alongside its photos — it's
                // the one thing besides images they're allowed to contain — and
                // calling one a photo would hand back a still of its first
                // frame. What it is comes from what arrived.
                let kind = kind_from_extension(&path);
                items.push(MediaItem { path, kind });
            }
            Planned::Playable { index, gif } => {
                let path =
                    fetch_playable(yt, quickjs, bins, url, work_dir, max_height, *index, done)?;
                if *gif {
                    let width = 480;
                    let path = to_gif(bins, &path, width)?;
                    items.push(MediaItem {
                        path,
                        kind: MediaKind::Gif,
                    });
                } else {
                    items.push(MediaItem {
                        path,
                        kind: MediaKind::Video,
                    });
                }
            }
        }
    }

    // Nothing yt-dlp could see. On X that means a photo tweet, which it reports
    // no thumbnails for at all — the one case that genuinely needs the other
    // tool.
    //
    // A post that was rate-limited is a different matter: nothing is wrong with
    // it, so the second tool would only spend another minute being turned away
    // and then report the wrong reason. Say what actually happened instead.
    if items.is_empty() {
        if let Some(reason) = plan.trouble {
            return Err(reason);
        }
        if let Some(gallery) = gallery {
            items = fetch_with_gallery(gallery, work_dir, url, cancel)?;
        }
    }

    if items.is_empty() {
        return Err("Couldn't find anything to download in that post.".into());
    }

    on_progress(1.0);
    Ok(Ok(items))
}

/// Pulls one playable item out of a post by its position in it.
#[allow(clippy::too_many_arguments)]
fn fetch_playable(
    yt: &Path,
    quickjs: Option<&Path>,
    bins: &Binaries,
    url: &str,
    work_dir: &Path,
    max_height: u32,
    index: usize,
    slot: usize,
) -> Result<PathBuf, String> {
    let format = format!(
        "bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b",
        h = max_height
    );
    let template = format!("{}/media-{slot}.%(ext)s", work_dir.display());
    let runtime = crate::download::runtime_args(quickjs);

    // Each go is a fresh extraction, which is the point: `--retries` re-requests
    // the same signed address, and a stale address returns the same 403 however
    // often it's asked for.
    let mut last = String::new();
    for extra in crate::download::attempts(url, quickjs) {
        let output = base_command(yt)
            .args(["--socket-timeout", "20", "--retries", "3"])
            .args(&runtime)
            .args(&extra)
            .args(["-f", &format])
            .args(["--merge-output-format", "mp4"])
            // One-based, and it's how yt-dlp addresses a single item of a post
            // that holds several.
            .args(["--playlist-items", &(index + 1).to_string()])
            .args(["-o", &template])
            .arg("--ffmpeg-location")
            .arg(&bins.ffmpeg)
            .arg(url)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("Couldn't start the downloader: {e}"))?;

        if output.status.success() {
            return newest_file(work_dir, &format!("media-{slot}"))
                .ok_or_else(|| "The download finished but produced no file.".to_string());
        }

        last = String::from_utf8_lossy(&output.stderr).into_owned();
        if !crate::download::worth_retrying(&last) {
            break;
        }
    }

    // Said in yt-dlp's own words rather than as a flat "couldn't be
    // downloaded", which sent people looking for a problem with the video when
    // the answer was usually sitting in stderr.
    Err(crate::download::summarise_yt_dlp_error(&last))
}

/// Falls back to gallery-dl, which reaches stills that yt-dlp doesn't.
fn fetch_with_gallery(
    gallery: &Path,
    work_dir: &Path,
    url: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<MediaItem>, String> {
    let output = base_command(gallery)
        .args(["--dest", &work_dir.to_string_lossy()])
        .args(["--no-mtime", "--quiet"])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't start the image downloader: {e}"))?;

    if cancel.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let lower = err.to_lowercase();

        // gallery-dl can't read Reddit — it gets the web page where it expects
        // JSON and dies parsing it. That isn't fatal any more: yt-dlp names the
        // image address while refusing to fetch it, so this path is only ever
        // reached for a post nothing else could see either.
        if lower.contains("jsondecodeerror") || lower.contains("expecting value") {
            return Err("Couldn't read what that post contains.".into());
        }
        if lower.contains("429") || lower.contains("too many requests") {
            return Err("That site is asking us to slow down. Wait a minute and try again.".into());
        }
        if lower.contains("login") || lower.contains("account") {
            return Err("That post needs an account to view.".into());
        }

        // Anything else keeps its own words, if it has any worth repeating.
        //
        // It usually doesn't. When Reddit answers with its web page instead of
        // data, the "error" *is* that page, and an earlier version of this put
        // the entire stylesheet on the card. Tool output is not a user-facing
        // message and has to earn its place.
        return Err(match readable_reason(&err) {
            Some(reason) => format!("Couldn't download the images from that post: {reason}"),
            None => "Couldn't download the images from that post.".into(),
        });
    }

    // gallery-dl builds its own folders under the destination, so collect
    // whatever landed rather than predicting the layout.
    let mut found = Vec::new();
    collect_files(work_dir, &mut found);
    found.sort();

    Ok(found
        .into_iter()
        .map(|path| {
            let kind = kind_from_extension(&path);
            MediaItem { path, kind }
        })
        .collect())
}

/// A line from a tool's output fit to show someone, or nothing.
///
/// Anything carrying markup, style rules or a stack frame is discarded rather
/// than trimmed: a shorter piece of a stylesheet is still a stylesheet.
fn readable_reason(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .map(str::trim)
        // The last such line is the conclusion; earlier ones tend to be the
        // retries on the way there.
        .rfind(|line| {
            !line.is_empty()
                && line.len() <= 160
                && !line.contains('{')
                && !line.contains('}')
                && !line.contains("--")
                && !line.contains("File \"")
                && !line.starts_with('<')
        })
        .map(|line| {
            line.trim_start_matches("ERROR: ")
                .trim_start_matches("error: ")
                .to_string()
        })
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn newest_file(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if !path.file_name()?.to_string_lossy().starts_with(prefix) {
            continue;
        }
        let modified = entry.metadata().ok()?.modified().ok()?;
        if best.as_ref().is_none_or(|(t, _)| modified >= *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_a_twitter_gif_by_its_path() {
        let gif = Entry {
            formats: vec![Format {
                url: Some("https://video.twimg.com/tweet_video/abc.mp4".into()),
            }],
            thumbnails: vec![],
            url: None,
        };
        let video = Entry {
            formats: vec![Format {
                url: Some("https://video.twimg.com/ext_tw_video/123/vid.mp4".into()),
            }],
            thumbnails: vec![],
            url: None,
        };

        assert!(looks_like_gif(&gif));
        assert!(
            !looks_like_gif(&video),
            "an ordinary silent clip is not a GIF"
        );
    }

    #[test]
    fn keeps_the_extension_a_url_arrives_with() {
        assert_eq!(extension_from_url("https://x/y/a.jpg"), Some("jpg"));
        assert_eq!(
            extension_from_url("https://x/y/a.png?stp=dst&_nc=1"),
            Some("png")
        );
        assert_eq!(extension_from_url("https://x/y/a.gif"), Some("gif"));
        // Not an extension we'd trust to be an image.
        assert_eq!(extension_from_url("https://x/y/a.php"), None);
        assert_eq!(extension_from_url("https://x/y/noextension"), None);
    }
}

#[cfg(test)]
mod reddit_tests {
    use super::*;

    #[test]
    fn takes_the_post_title_off_the_page() {
        let html = "<title>i have this optical illusion : interesting</title>";
        assert_eq!(
            reddit_title(html).as_deref(),
            Some("i have this optical illusion")
        );
    }

    #[test]
    fn puts_escaped_characters_back_the_way_they_read() {
        let html = "<title>Bob&#39;s &quot;best&quot; cat &amp; dog : aww</title>";
        assert_eq!(
            reddit_title(html).as_deref(),
            Some("Bob's \"best\" cat & dog")
        );
    }

    #[test]
    fn keeps_a_title_that_has_a_colon_of_its_own() {
        // Only the last " : " is the subreddit the page appends.
        let html = "<title>TIL : the deepest lake : todayilearned</title>";
        assert_eq!(
            reddit_title(html).as_deref(),
            Some("TIL : the deepest lake")
        );
    }

    #[test]
    fn spots_a_share_link_in_the_shapes_it_arrives_in() {
        for url in [
            "https://www.reddit.com/r/interesting/s/4sEQlGicku",
            "https://www.reddit.com/r/interesting/s/4sEQlGicku/",
            "https://www.reddit.com/r/interesting/s/4sEQlGicku?utm_source=share",
            "https://reddit.com/r/pics/s/AbCdEf",
        ] {
            assert!(is_reddit_share_link(url), "{url} is a share link");
        }
    }

    #[test]
    fn leaves_every_other_reddit_address_alone() {
        for url in [
            "https://www.reddit.com/r/interesting/comments/1vjv949/i_have_this/",
            "https://www.reddit.com/r/interesting/comments/1vjv949/i_have_this",
            // A subreddit that happens to be called "s" is still not a share
            // link — the marker sits one segment from the end, not at it.
            "https://www.reddit.com/r/s/",
            "https://i.redd.it/dkrhd8sdw3ih1.jpeg",
            "https://x.com/i/status/2085248162445373578",
        ] {
            assert!(!is_reddit_share_link(url), "{url} is not a share link");
        }
    }

    #[test]
    fn finds_the_image_reddit_hid_in_a_complaint() {
        // The real message, from a post that failed before this existed.
        let stderr = "ERROR: Unsupported URL: \
                      https://www.reddit.com/media?url=https%3A%2F%2Fi.redd.it%2Fdkrhd8sdw3ih1.jpeg";

        assert_eq!(
            reddit_images(stderr),
            vec!["https://i.redd.it/dkrhd8sdw3ih1.jpeg".to_string()]
        );
    }

    #[test]
    fn ignores_anything_that_is_not_reddits_own_image_host() {
        let stderr = "ERROR: Unsupported URL: \
                      https://www.reddit.com/media?url=https%3A%2F%2Fevil.example%2Fx.jpg";
        assert!(
            reddit_images(stderr).is_empty(),
            "only i.redd.it and preview.redd.it should be followed"
        );
    }
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn refuses_to_put_a_stylesheet_on_a_card() {
        // Shortened, but the same shape as what Reddit actually returned: its
        // web page, arriving where data was expected, and ending up verbatim on
        // the card as an "error".
        let stderr = ".theme-light,:root{--rem360:22.5rem;--rem320:20rem;\
                      --spacer-4xs:0.125rem;--size-2xs:0.25rem}";

        assert_eq!(
            readable_reason(stderr),
            None,
            "a stylesheet is not a reason anyone can act on"
        );
    }

    #[test]
    fn keeps_a_real_message() {
        let stderr = "[reddit][error] HTTP redirect to login page";
        assert_eq!(
            readable_reason(stderr).as_deref(),
            Some("[reddit][error] HTTP redirect to login page")
        );
    }

    #[test]
    fn drops_a_python_traceback() {
        let stderr = "Traceback (most recent call last):\n  File \"gallery_dl/job.py\", line 1\n";
        assert_eq!(
            readable_reason(stderr).as_deref(),
            Some("Traceback (most recent call last):")
        );
    }
}
