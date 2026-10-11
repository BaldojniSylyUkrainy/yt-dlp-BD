//! Real codec tests enabled explicitly with BALDOJ_TEST_FFMPEG.
use super::*;

fn run(ffmpeg: &Path, args: &[&str], output: &Path) {
    let result = process_runner::capture(
        Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            .args(args)
            .arg(output),
        Duration::from_secs(40),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn video_hash(ffmpeg: &Path, path: &Path) -> Vec<u8> {
    let result = process_runner::capture(
        Command::new(ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-f", "hash", "-"]),
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(result.status.success());
    result.stdout
}

#[test]
#[ignore = "requires an explicit managed FFmpeg path"]
fn remux_preserves_every_decoded_video_pixel_in_cyrillic_path() {
    let ffmpeg = PathBuf::from(std::env::var("BALDOJ_TEST_FFMPEG").expect("BALDOJ_TEST_FFMPEG"));
    let dir = std::env::temp_dir().join(format!(
        "baldoj-smoke-{}-кирилиця та пробіли",
        Uuid::new_v4()
    ));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("відео.mkv");
    run(
        &ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ],
        &source,
    );
    let plan = conversion::probe(&ffmpeg, &source).unwrap();
    assert!(plan.copy_video && plan.copy_audio);
    let output = dir.join("готове.mp4");
    let result = process_runner::capture(
        Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(conversion::args(plan, VideoEncoder::Software, 14000000))
            .arg(&output),
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(video_hash(&ffmpeg, &source), video_hash(&ffmpeg, &output));
    let ready = DownloadedMedia {
        path: output.clone(),
        identity: MediaIdentity::default(),
        duration: Some(1.0),
        height: Some(240),
        fps: Some(24.0),
        video_codec: Some("h264".into()),
        audio_codec: Some("aac".into()),
    };
    assert!(!requires_app_conversion(&ready, "mp4", &ffmpeg).unwrap());
    // A corrupt browser-delivered MP4 must expose the probe failure, not
    // become a request to run the converter and probe the same file again.
    let corrupt_path = dir.join("пошкоджене.mp4");
    fs::write(&corrupt_path, b"not an mp4").unwrap();
    let corrupt = DownloadedMedia {
        path: corrupt_path.clone(),
        ..ready.clone()
    };
    let error = requires_app_conversion(&corrupt, "mp4", &ffmpeg).unwrap_err();
    assert!(error.contains("ffprobe") && error.contains("exit status"));
    assert!(!error.contains(&dir.to_string_lossy().to_string()));
    assert_eq!(fs::read(&corrupt_path).unwrap(), b"not an mp4");
    let ten_bit = dir.join("10 bit.mp4");
    run(
        &ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x128:rate=10",
            "-t",
            "0.3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p10le",
            "-x264-params",
            "colorprim=bt709:transfer=bt709:colormatrix=bt709",
            "-color_trc",
            "bt709",
            "-color_primaries",
            "bt709",
            "-colorspace",
            "bt709",
        ],
        &ten_bit,
    );
    let incompatible = DownloadedMedia {
        path: ten_bit,
        ..ready
    };
    assert!(requires_app_conversion(&incompatible, "mp4", &ffmpeg).unwrap());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "requires FFmpeg with libx265 and zscale"]
fn hdr_is_explicitly_tone_mapped_to_bt709() {
    let ffmpeg = PathBuf::from(std::env::var("BALDOJ_TEST_FFMPEG").expect("BALDOJ_TEST_FFMPEG"));
    let dir = std::env::temp_dir().join(format!("baldoj-hdr-smoke-{}", Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("hdr.mkv");
    run(&ffmpeg, &["-f", "lavfi", "-i", "testsrc2=size=128x128:rate=10", "-t", "0.3", "-c:v", "libx265", "-x265-params", "pools=1:frame-threads=1:colorprim=9:transfer=16:colormatrix=9:hdr10=1:repeat-headers=1", "-pix_fmt", "yuv420p10le", "-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc"], &source);
    let plan = conversion::probe(&ffmpeg, &source).unwrap();
    assert!(plan.tone_map && !plan.copy_video);
    let output = dir.join("sdr.mp4");
    let result = process_runner::capture(
        Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(conversion::args(plan, VideoEncoder::Software, 2000000))
            .arg(&output),
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = process_runner::capture(
        Command::new(ffprobe_path(&ffmpeg))
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=color_transfer,pix_fmt",
                "-of",
                "json",
            ])
            .arg(&output),
        Duration::from_secs(10),
    )
    .unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(metadata["streams"][0]["color_transfer"], "bt709");
    assert_eq!(metadata["streams"][0]["pix_fmt"], "yuv420p");
    fs::remove_dir_all(&dir).unwrap();
}
