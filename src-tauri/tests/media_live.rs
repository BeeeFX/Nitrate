//! Live checks against real posts.
//!
//! Ignored by default: they need the network and depend on posts that may be
//! deleted. Run deliberately with
//! `cargo test --test media_live -- --ignored --nocapture`.

use nitrate_lib::{ffmpeg, media};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn yt_dlp() -> PathBuf {
    dirs_bin().join("yt-dlp.exe")
}

fn dirs_bin() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap())
        .join("app.nitrate.desktop")
        .join("bin")
}

#[test]
#[ignore = "needs the network"]
fn an_instagram_carousel_comes_back_as_separate_items() {
    let dir = std::env::temp_dir().join("nitrate-live-ig");
    let _ = std::fs::remove_dir_all(&dir);

    let bins = ffmpeg::resolve();
    let cancel = Arc::new(AtomicBool::new(false));

    let items = media::fetch_post(
        &yt_dlp(),
        None,
        &bins,
        "https://www.instagram.com/p/DbqvFwciSjr/",
        &dir,
        1080,
        &cancel,
        |_| {},
    )
    .expect("fetching should not error")
    .unwrap_or_else(|_| panic!("should not be cancelled"));

    println!("got {} items", items.len());
    for item in &items {
        let size = std::fs::metadata(&item.path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  {:?}  {:?}  {size} bytes",
            item.kind,
            item.path.file_name().unwrap()
        );
    }

    assert!(items.len() > 1, "expected a carousel, got {}", items.len());

    // This post is a mixed carousel — two photos and two videos. That's the
    // case worth pinning down: the interesting failure isn't "no items", it's
    // everything arriving labelled the same because the kinds got flattened.
    assert!(
        items.iter().any(|i| i.kind == media::MediaKind::Photo),
        "the photos in this post came back as something else"
    );
    assert!(
        items.iter().any(|i| i.kind == media::MediaKind::Video),
        "the videos in this post came back as something else"
    );

    for item in &items {
        let size = std::fs::metadata(&item.path).unwrap().len();
        assert!(
            size > 50_000,
            "{:?} is too small to be the real thing",
            item.path
        );

        // Each keeps the extension it belongs in, which is the whole promise.
        let ext = item.path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match item.kind {
            media::MediaKind::Photo => {
                assert!(
                    matches!(ext, "jpg" | "jpeg" | "png" | "webp"),
                    "photo saved as .{ext}"
                )
            }
            media::MediaKind::Video => assert_eq!(ext, "mp4"),
            media::MediaKind::Gif => assert_eq!(ext, "gif"),
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs the network"]
fn an_x_gif_comes_back_as_a_real_gif() {
    let dir = std::env::temp_dir().join("nitrate-live-gif");
    let _ = std::fs::remove_dir_all(&dir);

    let bins = ffmpeg::resolve();
    let cancel = Arc::new(AtomicBool::new(false));

    let items = media::fetch_post(
        &yt_dlp(),
        None,
        &bins,
        "https://x.com/BlazeBinges/status/2084677139195150496",
        &dir,
        1080,
        &cancel,
        |_| {},
    )
    .expect("fetching should not error")
    .unwrap_or_else(|_| panic!("should not be cancelled"));

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, media::MediaKind::Gif);
    assert_eq!(
        items[0].path.extension().and_then(|e| e.to_str()),
        Some("gif")
    );

    let head = std::fs::read(&items[0].path).expect("readable");
    assert_eq!(&head[0..3], b"GIF", "not actually a GIF file");
    println!("gif is {} bytes", head.len());

    let _ = std::fs::remove_dir_all(&dir);
}
