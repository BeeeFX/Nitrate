//! Live checks against real posts.
//!
//! Ignored by default: they need the network and depend on posts that may be
//! deleted. Run deliberately with
//! `cargo test --test media_live -- --ignored --nocapture`.

use nitrate_lib::{download, ffmpeg, media};
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

#[test]
#[ignore = "needs the network"]
fn a_photo_post_gets_past_the_probe() {
    // These were rejected at the probe step, before the media pipeline could
    // look at them — the failure users actually saw.
    //
    // The Reddit ones matter most, and are the reason this test exists in this
    // shape. `a_reddit_photo_post_comes_back_as_an_image` below calls
    // `fetch_post` directly, one layer deeper than a person can reach, so it
    // passed for three releases while every Reddit photo post in the app was
    // being turned away here with "That site isn't supported."
    for url in [
        "https://x.com/i/status/2085248162445373578",
        "https://www.instagram.com/p/DbqvFwciSjr/",
        "https://www.reddit.com/r/dijondijon/comments/1vipx0d/jane_remover_samples_talk_down/",
        // A share link, which is what the Share button hands you.
        "https://www.reddit.com/r/interesting/s/4sEQlGicku",
    ] {
        let started = std::time::Instant::now();
        let probed = download::probe_url(&yt_dlp(), url);
        let took = started.elapsed();

        // Reddit rate-limits an address that has asked too often, and running
        // this repeatedly is exactly how you get there. That's the network
        // saying no, not the code — but it is the *only* refusal allowed to
        // pass, and it says so loudly rather than going quietly green.
        if probed
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("slow down"))
        {
            println!("SKIPPED  {url} — rate-limited right now, proves nothing");
            continue;
        }

        // A refusal is the regression this test exists for: every Reddit photo
        // post used to be turned away here, immediately, with "That site isn't
        // supported." That must never come back.
        let info = probed.unwrap_or_else(|e| panic!("{url} was refused at the probe: {e}"));

        // Not passing is not the same as passing. When the probe gives up on
        // the clock it also returns Ok, with the site's name as the title —
        // indistinguishable from a real answer by looking at the result alone,
        // so asserting on the result would go green on a link that is failing.
        // A genuine answer arrives in a couple of seconds; the cap is 25.
        //
        // Reddit throttles hard enough that this is usually the network rather
        // than the code, and yt-dlp's own backoff outlasts the cap, so there's
        // nothing here to tell the two apart. It's reported rather than failed.
        if took >= std::time::Duration::from_secs(20) {
            println!("SLOW  {url} took {took:?} — probe timed out, likely throttled. Not proven.");
            continue;
        }
        println!("ok  {url} -> {:?} in {took:?}", info.title);
    }
}

#[test]
#[ignore = "needs the network"]
fn a_reddit_share_link_resolves_to_the_post() {
    // What the Share button hands you, and so what most people paste. yt-dlp
    // can't follow it — it sat there until the timeout fired — so this is
    // resolved before any tool sees it.
    //
    // One HTTP request, no extractor involved, which is why this holds up when
    // the probe test above is being throttled.
    let resolved = media::canonical_url("https://www.reddit.com/r/interesting/s/4sEQlGicku");
    println!("resolved to {resolved}");

    assert!(
        resolved.contains("/comments/1vjv949/"),
        "share link did not resolve to the post it points at: {resolved}"
    );
    assert!(
        !resolved.contains("utm_") && !resolved.contains("share_id"),
        "the share tracking came along with it: {resolved}"
    );
}

#[test]
#[ignore = "needs the network"]
fn a_reddit_gallery_gives_up_every_image() {
    let dir = std::env::temp_dir().join("nitrate-live-gallery");
    let _ = std::fs::remove_dir_all(&dir);

    let bins = ffmpeg::resolve();
    let cancel = Arc::new(AtomicBool::new(false));

    // Two images. The API refusal we read elsewhere names one image, so a post
    // like this is the case that route can't serve — and this post was being
    // rate-limited by Reddit's API for an hour while its page loaded fine.
    let items = media::fetch_post(
        &yt_dlp(),
        None,
        &bins,
        "https://www.reddit.com/r/interesting/comments/1vjv949/i_have_this_optical_illusion_since_many_years_ago/",
        &dir,
        1080,
        &cancel,
        |_| {},
    )
    .expect("fetching should not error")
    .unwrap_or_else(|_| panic!("should not be cancelled"));

    for item in &items {
        let size = std::fs::metadata(&item.path).map(|m| m.len()).unwrap_or(0);
        println!("  {:?}  {:?}  {size} bytes", item.kind, item.path.file_name().unwrap());
    }

    assert_eq!(items.len(), 2, "the post holds two images");
    assert!(items.iter().all(|i| i.kind == media::MediaKind::Photo));
    assert!(
        items
            .iter()
            .all(|i| std::fs::metadata(&i.path).unwrap().len() > 10_000),
        "real images, not error pages saved to disk"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs the network"]
fn a_reddit_photo_post_comes_back_as_an_image() {
    let dir = std::env::temp_dir().join("nitrate-live-reddit");
    let _ = std::fs::remove_dir_all(&dir);

    let bins = ffmpeg::resolve();
    let cancel = Arc::new(AtomicBool::new(false));

    // Reddit's data API is closed to us, so this works only because yt-dlp
    // names the image address in the refusal it prints. No gallery-dl here.
    let items = media::fetch_post(
        &yt_dlp(),
        None,
        &bins,
        "https://www.reddit.com/r/dijondijon/comments/1vipx0d/jane_remover_samples_talk_down/",
        &dir,
        1080,
        &cancel,
        |_| {},
    )
    .expect("fetching should not error")
    .unwrap_or_else(|_| panic!("should not be cancelled"));

    for item in &items {
        let size = std::fs::metadata(&item.path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  {:?}  {:?}  {size} bytes",
            item.kind,
            item.path.file_name().unwrap()
        );
    }

    assert!(!items.is_empty(), "nothing came back from the post");
    assert!(
        items.iter().any(|i| i.kind == media::MediaKind::Photo),
        "expected a photo"
    );
    assert!(
        items
            .iter()
            .all(|i| std::fs::metadata(&i.path).unwrap().len() > 10_000),
        "a real image, not an error page saved to disk"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs the network"]
fn an_x_photo_post_comes_back_through_gallery_dl() {
    let dir = std::env::temp_dir().join("nitrate-live-xphoto");
    let _ = std::fs::remove_dir_all(&dir);

    let gallery = dirs_bin().join("gallery-dl.exe");
    if !gallery.is_file() {
        eprintln!("skipped: gallery-dl isn't installed yet");
        return;
    }

    let bins = ffmpeg::resolve();
    let cancel = Arc::new(AtomicBool::new(false));

    // X returns nothing at all for a photo tweet — no formats, no thumbnails —
    // so this is the one path that genuinely depends on the second tool.
    let items = media::fetch_post(
        &yt_dlp(),
        Some(&gallery),
        &bins,
        "https://x.com/i/status/2085248162445373578",
        &dir,
        1080,
        &cancel,
        |_| {},
    )
    .expect("fetching should not error")
    .unwrap_or_else(|_| panic!("should not be cancelled"));

    for item in &items {
        let size = std::fs::metadata(&item.path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  {:?}  {:?}  {size} bytes",
            item.kind,
            item.path.file_name().unwrap()
        );
    }

    assert!(!items.is_empty(), "nothing came back from the tweet");
    assert!(items.iter().any(|i| i.kind == media::MediaKind::Photo));

    let _ = std::fs::remove_dir_all(&dir);
}
