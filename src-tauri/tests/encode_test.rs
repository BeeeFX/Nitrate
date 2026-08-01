//! End-to-end checks on the encoder.
//!
//! The guarantee this app makes is "the output fits under the limit", so that's
//! what these assert — against real ffmpeg, on a real file.

use nitrate_lib::encode::{self, AudioCodec, Container, Settings, VideoCodec};
use nitrate_lib::ffmpeg;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Builds a throwaway clip so the suite doesn't depend on any fixture file.
fn make_clip(dir: &Path, name: &str, size: &str, fps: u32, secs: u32, kbps: u32) -> PathBuf {
    let bins = ffmpeg::resolve();
    let out = dir.join(name);

    let status = Command::new(&bins.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={size}:rate={fps}:duration={secs}"),
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={secs}"),
        ])
        .args(["-c:v", "libx264", "-preset", "ultrafast"])
        .args(["-b:v", &format!("{kbps}k")])
        .args(["-c:a", "aac", "-b:a", "128k", "-shortest"])
        .arg(&out)
        .status()
        .expect("ffmpeg should be available — run `npm run ffmpeg` first");

    assert!(status.success(), "failed to build the test clip");
    out
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nitrate-test-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settings(target_bytes: u64, dir: &Path) -> Settings {
    Settings {
        target_bytes,
        video_codec: VideoCodec::H264,
        container: Container::Mp4,
        audio_codec: AudioCodec::Aac,
        audio_bitrate_kbps: 128,
        hardware: false,
        max_height: None,
        max_fps: None,
        safety_margin: 0.97,
        // Keeps the suite quick; accuracy comes from two-pass, not the preset.
        preset: "veryfast".into(),
        two_pass: true,
        output_dir: Some(dir.to_string_lossy().into_owned()),
    }
}

fn encode_to(target_bytes: u64, tag: &str) -> (u64, encode::Plan) {
    let dir = temp_dir(tag);
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 8, 8000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let cancel = Arc::new(AtomicBool::new(false));

    let outcome = encode::run_job(
        &bins,
        &input,
        &settings(target_bytes, &dir),
        &info,
        &cancel,
        |_, _| {},
    )
    .expect("encode should not error")
    .unwrap_or_else(|_| panic!("encode should not be cancelled"));

    let bytes = outcome.final_bytes;
    let plan = outcome.plan;
    let _ = std::fs::remove_dir_all(&dir);
    (bytes, plan)
}

#[test]
fn lands_under_the_discord_free_limit() {
    let target = 10_000_000;
    let (bytes, _) = encode_to(target, "free");

    assert!(
        bytes <= target,
        "output was {bytes} bytes, over the {target} target"
    );
    // Landing far under means we threw away quality we didn't need to.
    assert!(
        bytes as f64 > target as f64 * 0.55,
        "output was only {bytes} bytes — far under {target}, wasting quality"
    );
}

#[test]
fn hits_a_tight_custom_target() {
    // A deliberately harsh target: the planner has to downscale to cope.
    let target = 800_000;
    let (bytes, plan) = encode_to(target, "tight");

    assert!(
        bytes <= target,
        "output was {bytes} bytes, over the {target} target"
    );
    assert!(
        plan.height < 720,
        "expected a downscale below 720p, got {}p",
        plan.height
    );
}

#[test]
fn refuses_targets_that_cannot_work() {
    let dir = temp_dir("impossible");
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 8, 4000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).unwrap();

    // 20 KB for an 8 second clip isn't achievable at any quality.
    let result = encode::plan(&info, &settings(20_000, &dir), &bins);

    assert!(result.is_err(), "should refuse an impossible target");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Encodes a deliberately tiny clip so codec-specific checks stay quick.
fn encode_small(tag: &str, mutate: impl FnOnce(&mut Settings)) -> Result<u64, String> {
    let dir = temp_dir(tag);
    let input = make_clip(&dir, "input.mp4", "320x240", 24, 3, 1200);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    let mut settings = settings(400_000, &dir);
    mutate(&mut settings);

    let cancel = Arc::new(AtomicBool::new(false));
    let result = encode::run_job(&bins, &input, &settings, &info, &cancel, |_, _| {})
        .map(|r| r.map(|o| o.final_bytes).unwrap_or(0));

    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn vp9_accepts_its_own_preset_scale() {
    // VP9 spells effort as a `cpu-used` number, not an x264-style name.
    let bytes = encode_small("vp9", |s| {
        s.video_codec = VideoCodec::Vp9;
        s.container = Container::Webm;
        s.audio_codec = AudioCodec::Opus;
        s.preset = "5".into();
        // VP9 two-pass is slow and isn't what's under test here.
        s.two_pass = false;
    })
    .expect("VP9 encode should succeed");

    assert!(bytes > 0, "VP9 produced an empty file");
}

#[test]
fn survives_a_preset_left_over_from_another_encoder() {
    // A settings file written while H.264 was selected would leave "medium"
    // behind, which VP9 would reject outright as a cpu-used value.
    let bytes = encode_small("stale-preset", |s| {
        s.video_codec = VideoCodec::Vp9;
        s.container = Container::Webm;
        s.audio_codec = AudioCodec::Opus;
        s.preset = "medium".into();
        s.two_pass = false;
    })
    .expect("a stale preset should be sanitised, not passed through");

    assert!(bytes > 0, "produced an empty file");
}

#[test]
fn probe_reads_the_essentials() {
    let dir = temp_dir("probe");
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 5, 2000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    assert_eq!(info.width, 1280);
    assert_eq!(info.height, 720);
    assert!(
        (info.duration - 5.0).abs() < 0.5,
        "duration was {}",
        info.duration
    );
    assert!((info.fps - 30.0).abs() < 0.5, "fps was {}", info.fps);
    assert!(info.has_audio(), "clip should have an audio track");

    let _ = std::fs::remove_dir_all(&dir);
}
