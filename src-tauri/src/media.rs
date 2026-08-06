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
//! | Reddit    | gallery-dl              | as video                 |
//!
//! Instagram serves a photo post's images through the field yt-dlp reports as
//! "thumbnails", and the largest is the genuine original — 1080x1350 on the
//! post this was tested against, not a preview. X hands back nothing at all for
//! a photo tweet, which is what gallery-dl is here for.

use crate::ffmpeg::{Binaries, Cancelled};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// Asks yt-dlp what's in a post without downloading any of it.
fn plan_from_yt_dlp(yt: &Path, url: &str) -> Vec<Planned> {
    let Ok(output) = base_command(yt)
        .args([
            "--dump-json",
            // Instagram photo posts have no video formats at all, and without
            // this yt-dlp treats that as fatal and prints nothing usable.
            "--ignore-no-formats-error",
            "--no-warnings",
            "--socket-timeout",
            "20",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut planned = Vec::new();

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

    planned
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
/// simply come back with nothing rather than failing.
#[allow(clippy::too_many_arguments)]
pub fn fetch_post(
    yt: &Path,
    gallery: Option<&Path>,
    bins: &Binaries,
    url: &str,
    work_dir: &Path,
    max_height: u32,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(f64),
) -> Result<Result<Vec<MediaItem>, Cancelled>, String> {
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("Couldn't create a working folder: {e}"))?;

    let planned = plan_from_yt_dlp(yt, url);
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
                items.push(MediaItem {
                    path,
                    kind: MediaKind::Photo,
                });
            }
            Planned::Playable { index, gif } => {
                let path = fetch_playable(yt, bins, url, work_dir, max_height, *index, done)?;
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
    if items.is_empty() {
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

    let output = base_command(yt)
        .args(["--no-warnings", "--socket-timeout", "20", "--retries", "3"])
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

    if !output.status.success() {
        return Err("That video couldn't be downloaded.".into());
    }

    newest_file(work_dir, &format!("media-{slot}"))
        .ok_or_else(|| "The download finished but produced no file.".to_string())
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
        if err.contains("login") {
            return Err("That post needs an account to view.".into());
        }
        return Err("Couldn't download the images from that post.".into());
    }

    // gallery-dl builds its own folders under the destination, so collect
    // whatever landed rather than predicting the layout.
    let mut found = Vec::new();
    collect_files(work_dir, &mut found);
    found.sort();

    Ok(found
        .into_iter()
        .map(|path| {
            let kind = match path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .as_deref()
            {
                Some("mp4") | Some("webm") | Some("mkv") => MediaKind::Video,
                Some("gif") => MediaKind::Gif,
                _ => MediaKind::Photo,
            };
            MediaItem { path, kind }
        })
        .collect())
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
