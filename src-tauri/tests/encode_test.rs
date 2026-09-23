//! End-to-end checks on the encoder.
//!
//! The guarantee this app makes is "the output fits under the limit", so that's
//! what these assert — against real ffmpeg, on a real file.

use nitrate_lib::encode::{
    self, AudioCodec, Container, CropRect, Edits, QualityLevel, Settings, TargetMode, VideoCodec,
};
use nitrate_lib::ffmpeg;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
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
        mode: TargetMode::Size,
        quality: QualityLevel::Balanced,
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
        auto_compress_downloads: true,
        max_download_height: 1080,
    }
}

fn encode_to(target_bytes: u64, tag: &str) -> (u64, encode::Plan) {
    let dir = temp_dir(tag);
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 8, 8000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let cancel = Arc::new(AtomicBool::new(false));

    let set = settings(target_bytes, &dir);
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &Edits::default(),
        name_hint: None,
    };

    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
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
    let result = encode::plan(&info, &settings(20_000, &dir), &Edits::default(), &bins);

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
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &settings,
        edits: &Edits::default(),
        name_hint: None,
    };
    let result = encode::run_job(&bins, &task, &cancel, |_, _| {})
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

/// Runs a job with edits applied, returning the finished size and plan.
fn encode_with_edits(target_bytes: u64, tag: &str, edits: Edits) -> (u64, encode::Plan, f64) {
    let dir = temp_dir(tag);
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 12, 6000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let cancel = Arc::new(AtomicBool::new(false));

    let set = settings(target_bytes, &dir);
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };

    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("encode should not error")
        .unwrap_or_else(|_| panic!("encode should not be cancelled"));

    let out_info = ffmpeg::probe(&bins, &outcome.output).expect("output should be readable");
    let result = (outcome.final_bytes, outcome.plan, out_info.duration);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn trimming_shortens_the_output() {
    let edits = Edits {
        start: Some(3.0),
        end: Some(8.0),
        crop: None,
        force_encode: false,
    };
    let (bytes, _, duration) = encode_with_edits(10_000_000, "trim", edits);

    assert!(
        (duration - 5.0).abs() < 0.6,
        "expected a 5s clip, got {duration}s"
    );
    assert!(bytes <= 10_000_000, "output was {bytes} bytes, over target");
}

#[test]
fn trimming_buys_quality_at_the_same_target() {
    // The whole point of trimming: the same budget spread over less video.
    // A tight target that forces a downscale on the full clip should hold its
    // resolution once most of the clip is cut away.
    let full = encode_with_edits(700_000, "trim-quality-full", Edits::default());
    let trimmed = encode_with_edits(
        700_000,
        "trim-quality-cut",
        Edits {
            start: Some(0.0),
            end: Some(2.0),
            crop: None,
            force_encode: false,
        },
    );

    assert!(
        trimmed.1.video_kbps > full.1.video_kbps * 2,
        "trimming to a sixth of the clip should raise the bitrate sharply: \
         full={} kbps, trimmed={} kbps",
        full.1.video_kbps,
        trimmed.1.video_kbps
    );
    assert!(
        trimmed.1.height >= full.1.height,
        "trimmed clip should not need a harsher downscale"
    );
}

/// A phone's portrait clip: landscape frames stored with a quarter-turn flag.
fn make_rotated_clip(dir: &Path) -> PathBuf {
    let bins = ffmpeg::resolve();
    let plain = make_clip(dir, "plain.mp4", "1280x720", 30, 6, 4000);
    let out = dir.join("portrait.mp4");

    let status = Command::new(&bins.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(["-display_rotation", "90", "-i"])
        .arg(&plain)
        .args(["-c", "copy"])
        .arg(&out)
        .status()
        .expect("ffmpeg should be available");
    assert!(status.success(), "failed to build the rotated clip");
    out
}

#[test]
fn a_rotated_clip_is_measured_the_way_up_it_plays() {
    let dir = temp_dir("rotated");
    let input = make_rotated_clip(&dir);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    assert_eq!((info.width, info.height), (720, 1280));

    // Most of the width: worked out on the stored 1280x720 this asked for a
    // crop wider than the upright frame ffmpeg actually filters, and failed.
    let edits = Edits {
        crop: Some(CropRect {
            x: 0.05,
            y: 0.0,
            width: 0.9,
            height: 1.0,
        }),
        ..Edits::default()
    };
    let set = settings(10_000_000, &dir);
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("a crop inside the frame should encode")
        .unwrap_or_else(|_| panic!("encode should not be cancelled"));

    let out = ffmpeg::probe(&bins, &outcome.output).expect("output should be readable");
    assert!(
        out.height > out.width,
        "portrait in, portrait out — got {}x{}",
        out.width,
        out.height
    );
    assert_eq!(
        (out.width, out.height),
        (outcome.plan.width, outcome.plan.height),
        "the plan should describe the file it produced"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn copied_audio_below_the_floor_still_plans() {
    let dir = temp_dir("quiet-audio");
    let input = make_clip(&dir, "input.mp4", "640x360", 30, 2, 1000);

    let bins = ffmpeg::resolve();
    let mut info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    // A long voice memo: little audio, and too long for the target to be
    // generous. The squeeze used to clamp between a 48k floor and this 32k
    // ceiling, which panics — and the release build aborts on a panic.
    info.audio_bitrate_kbps = Some(32);
    info.duration = 12.0 * 60.0;

    let mut set = settings(10_000_000, &dir);
    set.audio_codec = AudioCodec::Copy;
    let result = encode::plan(&info, &set, &Edits::default(), &bins);
    assert!(result.is_ok(), "expected a plan, got {result:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cropping_changes_the_output_shape() {
    // A centred square out of a 1280x720 source: the full height, and 720/1280
    // of the width, offset by half the difference.
    let edits = Edits {
        start: None,
        end: None,
        crop: Some(CropRect {
            x: 0.21875,
            y: 0.0,
            width: 0.5625,
            height: 1.0,
        }),
        force_encode: false,
    };
    let (bytes, plan, _) = encode_with_edits(10_000_000, "crop", edits);

    let ratio = plan.width as f64 / plan.height as f64;
    assert!(
        (ratio - 1.0).abs() < 0.06,
        "expected roughly square output, got {}x{}",
        plan.width,
        plan.height
    );
    assert!(bytes <= 10_000_000, "output was {bytes} bytes, over target");
}

#[test]
fn quality_mode_encodes_without_a_size_target() {
    let dir = temp_dir("quality");
    let input = make_clip(&dir, "input.mp4", "640x360", 24, 5, 3000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    // A deliberately tiny target that quality mode is meant to ignore. If the
    // size check ever leaks into this path, the encode fails right here.
    let mut set = settings(50_000, &dir);
    set.mode = TargetMode::Quality;
    set.quality = QualityLevel::Balanced;

    let plan = encode::plan(&info, &set, &Edits::default(), &bins).expect("plan should succeed");

    assert_eq!(plan.mode, TargetMode::Quality);
    assert!(plan.crf.is_some(), "quality mode should choose a CRF");
    assert_eq!(
        plan.video_kbps, 0,
        "there is no bitrate budget when no size is being targeted"
    );
    assert_eq!(
        plan.height, 360,
        "quality mode should keep the resolution it was given"
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &Edits::default(),
        name_hint: None,
    };
    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("quality encode should succeed")
        .unwrap_or_else(|_| panic!("should not be cancelled"));

    assert!(outcome.final_bytes > 0, "produced an empty file");
    assert!(
        outcome.final_bytes > set.target_bytes,
        "the clip should comfortably exceed the ignored target, otherwise this \
         test proves nothing — got {} bytes",
        outcome.final_bytes
    );
    assert_eq!(
        outcome.attempts, 1,
        "quality mode has no ceiling, so it should never retry"
    );

    // The estimate is a model, not a measurement, so this checks it's the right
    // order of magnitude rather than the right number. Anything outside a
    // factor of four is a broken model rather than unusual footage — and a
    // wildly wrong figure on the card is worse than none, since the whole point
    // is to give someone a rough idea before they commit to the encode.
    let estimate = plan
        .estimated_bytes
        .expect("quality mode should estimate a size");
    let ratio = estimate as f64 / outcome.final_bytes as f64;
    assert!(
        (0.25..=4.0).contains(&ratio),
        "estimated {estimate} bytes but encoded {} — off by {ratio:.1}x",
        outcome.final_bytes
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_size_estimate_tracks_the_quality_setting() {
    let dir = temp_dir("estimate-scale");
    let input = make_clip(&dir, "input.mp4", "640x360", 24, 5, 3000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    let estimate_for = |level: QualityLevel| {
        let mut set = settings(10_000_000, &dir);
        set.mode = TargetMode::Quality;
        set.quality = level;
        encode::plan(&info, &set, &Edits::default(), &bins)
            .expect("plan should succeed")
            .estimated_bytes
            .expect("quality mode should estimate a size")
    };

    let small = estimate_for(QualityLevel::Small);
    let balanced = estimate_for(QualityLevel::Balanced);
    let high = estimate_for(QualityLevel::High);

    assert!(
        small < balanced && balanced < high,
        "a higher quality setting has to estimate a larger file — got {small}, \
         {balanced}, {high}"
    );

    // Trimming halves the clip, so it should roughly halve the estimate. This
    // catches the estimate being computed from the source duration rather than
    // the edited one, which is the mistake that would otherwise go unnoticed.
    let mut set = settings(10_000_000, &dir);
    set.mode = TargetMode::Quality;
    let edits = Edits {
        start: Some(0.0),
        end: Some(info.duration / 2.0),
        ..Edits::default()
    };
    let trimmed = encode::plan(&info, &set, &edits, &bins)
        .expect("plan should succeed")
        .estimated_bytes
        .expect("quality mode should estimate a size");
    let full = estimate_for(QualityLevel::Balanced);
    let ratio = trimmed as f64 / full as f64;
    assert!(
        (0.4..=0.6).contains(&ratio),
        "half the clip should estimate about half the size — got {ratio:.2}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quality_mode_never_passes_a_file_through() {
    let dir = temp_dir("quality-passthrough");
    let input = make_clip(&dir, "input.mp4", "640x360", 24, 3, 400);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    let mut set = settings(10_000_000, &dir);
    set.mode = TargetMode::Quality;

    // Asking to compress is explicit, even for a file that would already fit.
    assert!(
        !encode::can_pass_through(&info, &set, &Edits::default(), &input),
        "quality mode should always re-encode"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_already_fits_is_left_alone() {
    let dir = temp_dir("passthrough");
    // Small and short, so it lands well under the target on its own.
    let input = make_clip(&dir, "input.mp4", "640x360", 24, 3, 400);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let set = settings(10_000_000, &dir);

    assert!(
        info.size_bytes < 10_000_000,
        "test clip should already fit, it was {} bytes",
        info.size_bytes
    );
    assert!(
        encode::can_pass_through(&info, &set, &Edits::default(), &input),
        "a small mp4 with no edits should pass through untouched"
    );

    // Any edit means it has to be processed, however small it is.
    let trimmed = Edits {
        start: Some(1.0),
        end: None,
        crop: None,
        force_encode: false,
    };
    assert!(
        !encode::can_pass_through(&info, &set, &trimmed, &input),
        "an edited file always needs re-encoding"
    );

    let _ = std::fs::remove_dir_all(&dir);
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

#[test]
fn keep_mode_trims_without_re_encoding() {
    let dir = temp_dir("keep-trim");
    // Deliberately generous bitrate: if this were re-encoded at any sane
    // setting the result would come out visibly smaller, which is how the test
    // can tell a copy from an encode without inspecting ffmpeg's arguments.
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 8, 6000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    // A target far below what a copy can possibly produce. Keep mode has to
    // ignore it completely — if any of the size logic leaked into this path the
    // job would either shrink the clip or fail outright.
    let mut set = settings(500_000, &dir);
    set.mode = TargetMode::Keep;

    let edits = Edits {
        start: Some(0.0),
        end: Some(4.0),
        crop: None,
        force_encode: false,
    };

    let plan = encode::plan(&info, &set, &edits, &bins).expect("plan should succeed");
    assert!(
        plan.copy_streams,
        "a trim alone should be copied, not encoded"
    );
    assert_eq!(plan.encoder, "copy");
    assert!(plan.crf.is_none(), "a copy has no quality setting to make");

    let cancel = Arc::new(AtomicBool::new(false));
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };
    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("keep-mode trim should succeed")
        .unwrap_or_else(|_| panic!("should not be cancelled"));

    let out = ffmpeg::probe(&bins, Path::new(&outcome.output)).expect("output should probe");

    assert_eq!(
        out.width, info.width,
        "a copy must not change the frame size"
    );
    assert_eq!(out.height, info.height);
    assert!(
        (out.duration - 4.0).abs() < 1.0,
        "the trim should be about four seconds, got {}",
        out.duration
    );

    // The giveaway: a copy keeps the source's bitrate, so half the clip is
    // about half the bytes. A re-encode at any ordinary setting would be far
    // smaller than that, and the size check below would fail.
    let ratio = outcome.final_bytes as f64 / info.size_bytes as f64;
    assert!(
        (0.35..=0.65).contains(&ratio),
        "half a copied clip should be about half the size — got {ratio:.2} \
         ({} of {} bytes), which suggests it was re-encoded",
        outcome.final_bytes,
        info.size_bytes
    );
    assert!(
        outcome.final_bytes > set.target_bytes,
        "the output came in under a target keep mode was supposed to ignore \
         ({} bytes vs {}), so the size logic reached this path",
        outcome.final_bytes,
        set.target_bytes
    );
    assert_eq!(
        outcome.attempts, 1,
        "a copy has nothing to verify and nothing to retry"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keep_mode_re_encodes_when_it_has_to() {
    let dir = temp_dir("keep-crop");
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 4, 3000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    let mut set = settings(10_000_000, &dir);
    set.mode = TargetMode::Keep;

    // Cropping changes the pixels, so copying them is not an option.
    let edits = Edits {
        start: None,
        end: None,
        crop: Some(CropRect {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        }),
        force_encode: false,
    };

    let plan = encode::plan(&info, &set, &edits, &bins).expect("plan should succeed");
    assert!(!plan.copy_streams, "a crop cannot be a stream copy");
    assert!(
        plan.crf.is_some(),
        "the fallback is constant quality, not a bitrate target"
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };
    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("keep-mode crop should succeed")
        .unwrap_or_else(|_| panic!("should not be cancelled"));

    let out = ffmpeg::probe(&bins, Path::new(&outcome.output)).expect("output should probe");
    assert_eq!(out.width, 640, "the crop should have halved the width");
    assert_eq!(out.height, 360);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keep_mode_leaves_an_unedited_file_completely_alone() {
    let dir = temp_dir("keep-untouched");
    // Far over any tier, to prove size plays no part in the decision.
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 6, 8000);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    let mut set = settings(1_000_000, &dir);
    set.mode = TargetMode::Keep;

    assert!(
        encode::can_pass_through(&info, &set, &Edits::default(), &input),
        "with nothing asked for, there is nothing to do"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cropping_a_photo_writes_a_photo() {
    let dir = temp_dir("photo-crop");
    let input = dir.join("shot.jpg");

    let bins = ffmpeg::resolve();
    let ok = std::process::Command::new(&bins.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc2=size=1080x1350"])
        .args(["-frames:v", "1"])
        .arg(&input)
        .status()
        .expect("ffmpeg should run");
    assert!(ok.success(), "couldn't build the test photo");

    let info = ffmpeg::probe(&bins, &input).expect("a photo should probe");
    let set = settings(10_000_000, &dir);

    // Half the frame, centred — the crop the editor produces.
    let edits = Edits {
        start: None,
        end: None,
        crop: Some(CropRect {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        }),
        force_encode: false,
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };
    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("cropping a photo should succeed")
        .unwrap_or_else(|_| panic!("should not be cancelled"));

    // It has to come back as a picture, not as a one-frame video wearing a
    // .jpg name — which is what happened before stills got their own path.
    assert_eq!(
        outcome.output.extension().and_then(|e| e.to_str()),
        Some("jpg"),
        "expected a .jpg, got {:?}",
        outcome.output
    );
    let head = std::fs::read(&outcome.output).expect("readable");
    assert_eq!(&head[0..2], &[0xFF, 0xD8], "not a JPEG");

    let out = ffmpeg::probe(&bins, &outcome.output).expect("output should probe");
    assert_eq!(out.width, 540, "crop should have halved the width");
    assert!(
        (out.height as i32 - 675).abs() <= 2,
        "height was {}",
        out.height
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// GIFs
// ---------------------------------------------------------------------------

/// A real animated GIF, built here so the suite needs no fixture and no network.
fn make_gif(dir: &Path, name: &str, size: &str, fps: u32, secs: u32) -> PathBuf {
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
        .args(["-loop", "0"])
        .arg(&out)
        .status()
        .expect("ffmpeg should run");

    assert!(status.success(), "building the test GIF failed");
    out
}

fn is_gif_file(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.starts_with(b"GIF"))
        .unwrap_or(false)
}

#[test]
fn a_gif_is_handed_over_untouched_by_default() {
    let dir = temp_dir("gif-default");
    // Comfortably over the target below, so size is not what lets it through.
    let input = make_gif(&dir, "in.gif", "320x240", 15, 3);
    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");

    let tiny = settings(50_000, &dir);
    assert!(
        std::fs::metadata(&input).unwrap().len() > 50_000,
        "the test GIF needs to exceed the target for this to mean anything"
    );

    assert!(
        encode::can_pass_through(&info, &tiny, &Edits::default(), &input),
        "a GIF should be left alone rather than squeezed into a size limit"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edited_gif_comes_back_a_gif() {
    let dir = temp_dir("gif-edited");
    let input = make_gif(&dir, "in.gif", "320x240", 15, 4);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let cancel = Arc::new(AtomicBool::new(false));

    // Opening the editor is the deliberate act that unlocks this, so an edit
    // is what the test applies: trimmed to the middle, and cropped.
    let edits = Edits {
        start: Some(1.0),
        end: Some(3.0),
        crop: Some(CropRect {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        }),
        force_encode: false,
    };

    let set = settings(2_000_000, &dir);
    assert!(
        !encode::can_pass_through(&info, &set, &edits, &input),
        "an edited GIF has to be re-encoded, not copied"
    );

    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };

    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("encode should not error")
        .unwrap_or_else(|_| panic!("encode should not be cancelled"));

    assert_eq!(
        outcome.output.extension().and_then(|e| e.to_str()),
        Some("gif"),
        "a trimmed GIF came back as something else"
    );
    assert!(
        is_gif_file(&outcome.output),
        "the file is named .gif but isn't one"
    );

    // The edits actually landed, rather than the file merely surviving.
    let edited = ffmpeg::probe(&bins, &outcome.output).expect("probe the result");
    assert!(
        (edited.duration - 2.0).abs() < 0.6,
        "trim didn't apply: {}s",
        edited.duration
    );
    assert!(
        edited.width < info.width,
        "crop didn't apply: still {}px wide",
        edited.width
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edited_gif_still_gets_under_a_size_limit() {
    let dir = temp_dir("gif-target");
    let input = make_gif(&dir, "in.gif", "480x360", 20, 4);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let cancel = Arc::new(AtomicBool::new(false));

    let original = std::fs::metadata(&input).unwrap().len();
    let target = original / 4;

    // Only a crop, so the shrinking has to come from the retry loop rather than
    // from trimming most of the frames away.
    let edits = Edits {
        start: None,
        end: None,
        crop: Some(CropRect {
            x: 0.0,
            y: 0.0,
            width: 0.9,
            height: 0.9,
        }),
        force_encode: false,
    };

    let set = settings(target, &dir);
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };

    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("encode should not error")
        .unwrap_or_else(|_| panic!("encode should not be cancelled"));

    println!(
        "{original} -> {} bytes (target {target}, {} attempts)",
        outcome.final_bytes, outcome.attempts
    );

    assert!(is_gif_file(&outcome.output), "still has to be a GIF");
    assert!(
        outcome.final_bytes <= target,
        "landed at {} against a target of {target}",
        outcome.final_bytes
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_editor_can_shrink_a_gif_without_cropping_it() {
    let dir = temp_dir("gif-forced");
    let input = make_gif(&dir, "in.gif", "480x360", 20, 3);

    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).expect("probe should succeed");
    let cancel = Arc::new(AtomicBool::new(false));

    let original = std::fs::metadata(&input).unwrap().len();
    let set = settings(original / 3, &dir);

    // No crop, no trim — just "make this smaller", which is a reasonable thing
    // to want and has no other way to be asked for. Without the flag the file
    // is copied and reports itself done having changed nothing.
    let asked = Edits {
        start: None,
        end: None,
        crop: None,
        force_encode: true,
    };
    let untouched = Edits::default();

    assert!(
        encode::can_pass_through(&info, &set, &untouched, &input),
        "left alone, a GIF is handed over as it is"
    );
    assert!(
        !encode::can_pass_through(&info, &set, &asked, &input),
        "asked for directly, it gets re-encoded"
    );

    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &asked,
        name_hint: None,
    };

    let outcome = encode::run_job(&bins, &task, &cancel, |_, _| {})
        .expect("encode should not error")
        .unwrap_or_else(|_| panic!("encode should not be cancelled"));

    println!("{original} -> {} bytes", outcome.final_bytes);
    assert!(is_gif_file(&outcome.output), "still a GIF");
    assert!(
        outcome.final_bytes < original,
        "asked to shrink and didn't: {} vs {original}",
        outcome.final_bytes
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Editor preview
// ---------------------------------------------------------------------------

/// Builds a clip in a codec the webview may or may not decode.
fn make_clip_in(dir: &Path, name: &str, encoder: &str, extra: &[&str]) -> PathBuf {
    let bins = ffmpeg::resolve();
    let out = dir.join(name);

    let status = Command::new(&bins.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24:duration=3",
        ])
        .args(["-c:v", encoder])
        .args(extra)
        .arg(&out)
        .status()
        .expect("ffmpeg should run");

    assert!(status.success(), "building a {encoder} clip failed");
    out
}

#[test]
fn every_codec_the_app_offers_gets_a_playable_preview() {
    let dir = temp_dir("preview-proxy");
    let bins = ffmpeg::resolve();

    // The four the settings panel offers, plus the container the webview is
    // least likely to touch. What the editor can play should not depend on
    // which of these the file happens to be in.
    let cases: [(&str, &str, &[&str]); 5] = [
        ("av1.mp4", "libsvtav1", &["-preset", "12"]),
        (
            "hevc.mp4",
            "libx265",
            &["-preset", "ultrafast", "-tag:v", "hvc1"],
        ),
        ("vp9.webm", "libvpx-vp9", &["-deadline", "realtime"]),
        ("h264.mp4", "libx264", &["-preset", "ultrafast"]),
        ("h264.mkv", "libx264", &["-preset", "ultrafast"]),
    ];

    for (name, encoder, extra) in cases {
        let input = make_clip_in(&dir, name, encoder, extra);
        let source = ffmpeg::probe(&bins, &input).expect("probe the source");

        let proxy = dir.join(format!("proxy-{name}.mp4"));
        ffmpeg::preview_proxy(&bins, &input, &proxy)
            .unwrap_or_else(|e| panic!("no preview for {name}: {e}"));

        let made = ffmpeg::probe(&bins, &proxy).expect("probe the preview");
        println!(
            "{name} -> h264, {}x{}, {:.2}s",
            made.width, made.height, made.duration
        );

        // Trim points are read off this and applied to the original, so a
        // preview that ran to a different length would cut the wrong frames.
        assert!(
            (made.duration - source.duration).abs() < 0.15,
            "{name}: preview is {:.3}s against a source of {:.3}s",
            made.duration,
            source.duration
        );
        assert!(made.width > 0 && made.height > 0, "{name}: empty preview");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Everything in `dir` that the encoder wrote, as opposed to the input.
fn outputs_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("nitrate"))
        .collect();
    names.sort();
    names
}

#[test]
fn two_jobs_with_the_same_name_both_survive() {
    // Two `clip.mp4`s from different folders, encoding at once into the same
    // output folder: both used to pick `clip-nitrate.mp4`, and one silently
    // overwrote the other.
    let root = temp_dir("same-name");
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let inputs: Vec<PathBuf> = ["a", "b"]
        .iter()
        .map(|folder| {
            let dir = root.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            make_clip(&dir, "clip.mp4", "640x360", 30, 3, 3000)
        })
        .collect();

    let handles: Vec<_> = inputs
        .into_iter()
        .map(|input| {
            let out = out.clone();
            std::thread::spawn(move || {
                let bins = ffmpeg::resolve();
                let info = ffmpeg::probe(&bins, &input).unwrap();
                let set = settings(400_000, &out);
                let task = encode::Task {
                    input: &input,
                    info: &info,
                    settings: &set,
                    edits: &Edits::default(),
                    name_hint: None,
                };
                let cancel = Arc::new(AtomicBool::new(false));
                encode::run_job(&bins, &task, &cancel, |_, _| {})
                    .expect("encode should not error")
                    .unwrap_or_else(|_| panic!("not cancelled"))
                    .output
            })
        })
        .collect();
    let mut produced: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    produced.sort();

    assert_ne!(produced[0], produced[1], "both jobs claimed the same file");
    assert_eq!(
        outputs_in(&out),
        vec!["clip-nitrate-2.mp4", "clip-nitrate.mp4"],
        "exactly the two finished files, nothing partial"
    );
    for path in &produced {
        let info = ffmpeg::probe(&ffmpeg::resolve(), path).expect("each should be a whole video");
        assert!(info.duration > 2.0);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_cancelled_encode_leaves_nothing_behind() {
    let dir = temp_dir("cancelled");
    let input = make_clip(&dir, "input.mp4", "1280x720", 30, 20, 6000);
    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).unwrap();
    let mut set = settings(5_000_000, &dir);
    set.preset = "slow".into();
    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &Edits::default(),
        name_hint: None,
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let mut seen_partial = false;
    let result = encode::run_job(&bins, &task, &cancel, |progress, stage| {
        // Well into the second pass, so a partial file exists by now.
        if stage == "Encoding" && progress > 0.5 {
            seen_partial |= outputs_in(&dir).iter().any(|n| n.contains(".partial."));
            flag.store(true, Ordering::Relaxed);
        }
    })
    .expect("encode should not error");

    assert!(result.is_err(), "should report the cancel");
    assert!(
        seen_partial,
        "the encode should have been writing a partial file"
    );
    assert_eq!(outputs_in(&dir), Vec::<String>::new(), "left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reserved_windows_names_get_a_suffix() {
    assert_eq!(encode::safe_stem("CON"), "CON_");
    assert_eq!(encode::safe_stem("nul"), "nul_");
    assert_eq!(encode::safe_stem("Console"), "Console");
    assert_eq!(encode::safe_stem("COM1"), "COM1_");
}

/// Runs the bundled ffmpeg with arguments of the test's choosing.
fn ffmpeg_make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let status = Command::new(ffmpeg::resolve().ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(args)
        .arg(&out)
        .status()
        .expect("ffmpeg should run");
    assert!(status.success(), "building {name} failed");
    out
}

/// Encodes `input` with the suite's settings adjusted by `mutate`.
fn encode_file(
    input: &Path,
    edits: Edits,
    mutate: impl FnOnce(&mut Settings),
) -> Result<encode::EncodeOutcome, String> {
    let bins = ffmpeg::resolve();
    let mut info = ffmpeg::probe(&bins, input).expect("probe should succeed");
    let mut set = settings(3_000_000, input.parent().unwrap());
    mutate(&mut set);
    encode::measure_copied_audio(&bins, input, &mut info, &set);
    let task = encode::Task {
        input,
        info: &info,
        settings: &set,
        edits: &edits,
        name_hint: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    encode::run_job(&bins, &task, &cancel, |_, _| {})
        .map(|r| r.unwrap_or_else(|_| panic!("not cancelled")))
}

const TEST_PICTURE: [&str; 4] = [
    "-f",
    "lavfi",
    "-i",
    "testsrc2=size=640x360:rate=24:duration=2",
];

#[test]
fn a_small_file_discord_cant_play_is_still_encoded() {
    let dir = temp_dir("inline");
    let bins = ffmpeg::resolve();
    let set = settings(10_000_000, &dir);

    // An iPhone's HEVC .mov, well under the limit.
    let hevc = ffmpeg_make(
        &dir,
        "phone.mov",
        &[
            &TEST_PICTURE[..],
            &["-c:v", "libx265", "-x265-params", "log-level=error"],
        ]
        .concat(),
    );
    let info = ffmpeg::probe(&bins, &hevc).unwrap();
    assert!(
        !encode::can_pass_through(&info, &set, &Edits::default(), &hevc),
        "HEVC plays as a download link on most Discord clients"
    );

    // 10-bit H.264 in an MP4: the right codec, a profile browsers won't decode.
    let ten_bit = ffmpeg_make(
        &dir,
        "ten.mp4",
        &[
            &TEST_PICTURE[..],
            &["-c:v", "libx264", "-pix_fmt", "yuv420p10le"],
        ]
        .concat(),
    );
    let info = ffmpeg::probe(&bins, &ten_bit).unwrap();
    assert!(!encode::can_pass_through(
        &info,
        &set,
        &Edits::default(),
        &ten_bit
    ));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hdr_is_tone_mapped_down_to_sdr() {
    let dir = temp_dir("hdr");
    let bins = ffmpeg::resolve();
    let hlg = ffmpeg_make(
        &dir,
        "hlg.mp4",
        &[
            &TEST_PICTURE[..],
            &[
                "-vf",
                "format=yuv420p10le",
                "-c:v",
                "libx265",
                // In the bitstream, where a camera puts it: the MP4 muxer
                // drops a transfer set only as an output option.
                "-x265-params",
                "log-level=error:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc",
            ],
        ]
        .concat(),
    );
    assert!(ffmpeg::probe(&bins, &hlg).unwrap().is_hdr());

    let outcome = encode_file(&hlg, Edits::default(), |_| {}).expect("should encode");
    let out = ffmpeg::probe(&bins, &outcome.output).unwrap();
    assert_eq!(out.pix_fmt.as_deref(), Some("yuv420p"));
    assert_eq!(
        out.color_transfer.as_deref(),
        Some("bt709"),
        "left labelled as HLG, it plays back washed out"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_codec_comes_out_eight_bit_four_two_zero() {
    // A 4:4:4 screen capture. H.264 and H.265 were already converted; VP9
    // wasn't, and wrote a profile 1 file most players refuse.
    let dir = temp_dir("444");
    let source = ffmpeg_make(
        &dir,
        "capture.mp4",
        &[
            &TEST_PICTURE[..],
            &["-c:v", "libx264", "-pix_fmt", "yuv444p"],
        ]
        .concat(),
    );
    let outcome = encode_file(&source, Edits::default(), |s| {
        s.video_codec = VideoCodec::Vp9;
        s.container = Container::Webm;
        s.audio_codec = AudioCodec::Opus;
        s.two_pass = false;
        s.preset = "5".into();
    })
    .expect("should encode");
    let out = ffmpeg::probe(&ffmpeg::resolve(), &outcome.output).unwrap();
    assert_eq!(out.pix_fmt.as_deref(), Some("yuv420p"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn surround_sound_becomes_stereo_opus() {
    let dir = temp_dir("surround");
    let source = ffmpeg_make(
        &dir,
        "film.mkv",
        &[
            &TEST_PICTURE[..],
            &[
                "-f",
                "lavfi",
                "-i",
                "anullsrc=channel_layout=5.1(side):sample_rate=48000",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-c:a",
                "ac3",
            ],
        ]
        .concat(),
    );
    let outcome = encode_file(&source, Edits::default(), |s| {
        s.video_codec = VideoCodec::Vp9;
        s.container = Container::Webm;
        s.audio_codec = AudioCodec::Opus;
        s.two_pass = false;
        s.preset = "5".into();
    })
    .expect("libopus used to refuse 5.1(side) outright");
    let out = ffmpeg::probe(&ffmpeg::resolve(), &outcome.output).unwrap();
    assert_eq!(out.audio_channels, Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn copying_aac_into_webm_converts_it_instead() {
    let dir = temp_dir("aac-webm");
    let source = make_clip(&dir, "input.mp4", "640x360", 24, 2, 800);
    let outcome = encode_file(&source, Edits::default(), |s| {
        s.video_codec = VideoCodec::Vp9;
        s.container = Container::Webm;
        s.audio_codec = AudioCodec::Copy;
        s.two_pass = false;
        s.preset = "5".into();
    })
    .expect("WebM can't hold AAC, and asking it to failed the encode");
    let out = ffmpeg::probe(&ffmpeg::resolve(), &outcome.output).unwrap();
    assert_eq!(out.audio_codec.as_deref(), Some("opus"));
    assert!(
        outcome
            .plan
            .notes
            .iter()
            .any(|n| n.contains("re-encoded as Opus")),
        "should say so: {:?}",
        outcome.plan.notes
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn copied_audio_is_budgeted_at_what_it_really_is() {
    // WebM states no bitrate for its streams, and the fallback was the
    // setting — 128k — whatever the track really spent.
    let dir = temp_dir("measure");
    let bins = ffmpeg::resolve();
    let source = ffmpeg_make(
        &dir,
        "loud.webm",
        &[
            &TEST_PICTURE[..],
            &[
                "-f",
                "lavfi",
                "-i",
                "anoisesrc=d=2:c=pink",
                "-ac",
                "2",
                "-c:v",
                "libvpx-vp9",
                "-deadline",
                "realtime",
                "-c:a",
                "libopus",
                "-b:a",
                "320k",
            ],
        ]
        .concat(),
    );
    let mut info = ffmpeg::probe(&bins, &source).unwrap();
    assert_eq!(info.audio_bitrate_kbps, None, "WebM shouldn't state one");

    let mut set = settings(3_000_000, &dir);
    set.audio_codec = AudioCodec::Copy;
    encode::measure_copied_audio(&bins, &source, &mut info, &set);
    let kbps = info.audio_bitrate_kbps.expect("should be measured");
    assert!((200..=400).contains(&kbps), "measured {kbps} kbps");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keep_mode_trims_past_tracks_an_mp4_cant_hold() {
    let dir = temp_dir("keep-tracks");
    let srt = dir.join("subs.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,500\nHello\n").unwrap();
    // Subtitles, and PCM audio the way cameras record it.
    let source = ffmpeg_make(
        &dir,
        "camera.mkv",
        &[
            &TEST_PICTURE[..],
            &["-f", "lavfi", "-i", "sine=duration=2", "-i"],
            &[srt.to_str().unwrap()],
            &[
                "-map",
                "0",
                "-map",
                "1",
                "-map",
                "2",
                "-c:v",
                "libx264",
                "-c:a",
                "pcm_s16le",
                "-c:s",
                "srt",
            ],
        ]
        .concat(),
    );
    let trim = Edits {
        start: Some(0.5),
        ..Edits::default()
    };
    let outcome = encode_file(&source, trim, |s| s.mode = TargetMode::Keep)
        .expect("`-map 0` took the subtitles along and the copy failed");
    assert!(
        outcome.plan.copy_streams,
        "the video should still be copied"
    );
    let out = ffmpeg::probe(&ffmpeg::resolve(), &outcome.output).unwrap();
    assert_eq!(out.audio_codec.as_deref(), Some("aac"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_source_odd_in_both_directions_encodes() {
    let dir = temp_dir("odd");
    let source = ffmpeg_make(
        &dir,
        "odd.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=853x481:rate=24:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv444p",
        ],
    );
    let outcome = encode_file(&source, Edits::default(), |_| {})
        .expect("scale=-2:481 is a height x264 refuses");
    let out = ffmpeg::probe(&ffmpeg::resolve(), &outcome.output).unwrap();
    assert_eq!((out.width % 2, out.height % 2), (0, 0));
    assert_eq!(
        (out.width, out.height),
        (outcome.plan.width, outcome.plan.height)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_gpu_encoder_that_fails_hands_over_to_software() {
    // No GPU here, so NVENC fails the moment it starts — exactly what a
    // driver reset mid-encode looks like from this side.
    let dir = temp_dir("gpu-fallback");
    let input = make_clip(&dir, "input.mp4", "640x360", 24, 2, 800);
    let bins = ffmpeg::resolve();
    let info = ffmpeg::probe(&bins, &input).unwrap();
    let set = settings(1_000_000, &dir);
    let mut plan = encode::plan(&info, &set, &Edits::default(), &bins).unwrap();
    plan.encoder = "h264_nvenc".into();

    let task = encode::Task {
        input: &input,
        info: &info,
        settings: &set,
        edits: &Edits::default(),
        name_hint: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = encode::run_planned(&bins, &task, plan, &cancel, |_, _| {})
        .expect("should finish in software rather than fail")
        .unwrap_or_else(|_| panic!("not cancelled"));
    assert_eq!(outcome.plan.encoder, "libx264");
    assert!(outcome.final_bytes <= 1_000_000);
    assert!(outcome
        .plan
        .notes
        .iter()
        .any(|n| n.contains("finished in software")));
    let _ = std::fs::remove_dir_all(&dir);
}
