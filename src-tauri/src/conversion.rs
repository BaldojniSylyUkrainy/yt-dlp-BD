//! Decide processing from streams, not website names or filename extensions.
use super::*;

// Inputs are already downloaded files. A disguised playlist/container must
// never open a network connection outside the downloader's guarded transport.
pub(crate) const LOCAL_INPUT_PROTOCOLS: [&str; 2] = ["-protocol_whitelist", "file,pipe"];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProcessingPlan {
    pub copy_video: bool,
    pub copy_audio: bool,
    pub tone_map: bool,
}

pub(crate) fn from_streams(value: &serde_json::Value) -> ProcessingPlan {
    let Some(streams) = value.get("streams").and_then(|s| s.as_array()) else {
        return ProcessingPlan::default();
    };
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let tone_map = video.is_some_and(|s| {
        matches!(
            s["color_transfer"].as_str(),
            Some("smpte2084" | "arib-std-b67")
        )
    });
    ProcessingPlan {
        copy_video: !tone_map
            && video.is_some_and(|s| {
                s["codec_name"] == "h264"
                    && matches!(s["pix_fmt"].as_str(), Some("yuv420p" | "yuvj420p"))
            }),
        copy_audio: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .all(|s| s["codec_name"] == "aac"),
        tone_map,
    }
}

pub(crate) fn probe(ffmpeg: &Path, input: &Path) -> Result<ProcessingPlan, String> {
    type Plans = HashMap<(probe_cache::Identity, probe_cache::Identity), (Instant, ProcessingPlan)>;
    static CACHE: OnceLock<Mutex<Plans>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = probe_cache::identity(&ffprobe_path(ffmpeg)).zip(probe_cache::identity(input));
    if let Some(key) = &key {
        if let Some((time, plan)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            if time.elapsed() < Duration::from_secs(300) {
                return Ok(*plan);
            }
        }
    }
    let plan = probe_uncached(ffmpeg, input)?;
    if let Some(key) = key {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), plan));
    }
    Ok(plan)
}

fn probe_uncached(ffmpeg: &Path, input: &Path) -> Result<ProcessingPlan, String> {
    let mut command = Command::new(ffprobe_path(ffmpeg));
    command.args(LOCAL_INPUT_PROTOCOLS);
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,pix_fmt,color_transfer",
            "-of",
            "json",
        ])
        .arg(input);
    let output = process_runner::capture(&mut command, Duration::from_secs(15))
        .map_err(|e| format!("ffprobe — перевірка потоків: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe — перевірка потоків: {}; {}",
            output.status,
            redacted_probe_stderr(&output.stderr)
        ));
    }
    let mut value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "ffprobe повернув неправильні дані потоків")?;
    // Some HEVC/AV1 containers only expose transfer characteristics after a
    // frame is decoded. Probe a bounded handful of packets, never the full file.
    if let Some(video) = value["streams"]
        .as_array_mut()
        .and_then(|streams| streams.iter_mut().find(|s| s["codec_type"] == "video"))
    {
        let transfer_unknown = video["color_transfer"]
            .as_str()
            .is_none_or(|v| matches!(v, "unknown" | "unspecified"));
        let high_depth = video["pix_fmt"]
            .as_str()
            .is_some_and(|p| p.contains("10") || p.contains("12") || p.contains("16"));
        if transfer_unknown && high_depth {
            let frame_output = process_runner::capture(
                Command::new(ffprobe_path(ffmpeg))
                    .args(LOCAL_INPUT_PROTOCOLS)
                    .args([
                        "-v",
                        "error",
                        "-read_intervals",
                        "%+#8",
                        "-select_streams",
                        "v:0",
                        "-show_entries",
                        "frame=color_transfer",
                        "-of",
                        "json",
                    ])
                    .arg(input),
                Duration::from_secs(15),
            )
            .map_err(|e| format!("ffprobe — перевірка HDR: {e}"))?;
            if !frame_output.status.success() {
                return Err(format!(
                    "ffprobe — перевірка HDR: {}; {}",
                    frame_output.status,
                    redacted_probe_stderr(&frame_output.stderr)
                ));
            }
            let frames: serde_json::Value = serde_json::from_slice(&frame_output.stdout)
                .map_err(|_| "ffprobe повернув неправильні дані HDR-кадрів")?;
            if let Some(transfer) = frames["frames"]
                .as_array()
                .and_then(|f| f.iter().find_map(|v| v["color_transfer"].as_str()))
            {
                video["color_transfer"] = transfer.into();
            } else {
                return Err(
                    "Не вдалося визначити HDR/SDR для 10/12-бітного відео; оригінал збережено"
                        .into(),
                );
            }
        }
    }
    Ok(from_streams(&value))
}

fn redacted_probe_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .take(12)
        .map(diagnostics::redact)
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) fn args(plan: ProcessingPlan, encoder: VideoEncoder, bitrate: u64) -> Vec<String> {
    let mut args = video_encoding_args(
        encoder,
        bitrate,
        bitrate.saturating_mul(11) / 10,
        bitrate.saturating_mul(2),
    );
    if plan.copy_video {
        args = [
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ]
        .map(str::to_string)
        .to_vec();
    } else if encoder == VideoEncoder::Software {
        // Quality-driven software encoding; a fixed bitrate can inflate a small source.
        let mut i = 0;
        while i < args.len() {
            if ["-b:v", "-maxrate", "-bufsize"].contains(&args[i].as_str()) {
                args.drain(i..i + 2);
            } else {
                i += 1;
            }
        }
        args.extend(["-crf", "18", "-preset", "medium"].map(str::to_string));
    }
    if plan.copy_audio {
        let mut i = 0;
        while i < args.len() {
            if ["-b:a", "-profile:a"].contains(&args[i].as_str()) {
                args.drain(i..i + 2);
            } else {
                i += 1;
            }
        }
        if let Some(i) = args.iter().position(|a| a == "-c:a") {
            args[i + 1] = "copy".into();
        }
    }
    if plan.tone_map {
        args.extend(["-vf", "zscale=transfer=linear:npl=100,format=gbrpf32le,tonemap=hable,zscale=primaries=bt709:transfer=bt709:matrix=bt709,format=yuv420p", "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709"].map(str::to_string));
    }
    args
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(super) fn encoder_probe_args(encoder: VideoEncoder) -> Vec<&'static str> {
    vec![
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-f",
        "lavfi",
        "-i",
        "color=size=128x128:rate=30",
        "-frames:v",
        "1",
        "-an",
        "-c:v",
        encoder.codec(),
        "-f",
        "null",
        "-",
    ]
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(super) fn probe_video_encoder(ffmpeg_path: &Path, encoder: VideoEncoder) -> bool {
    let mut command = Command::new(ffmpeg_path);
    configure_command(&mut command);
    command
        .args(encoder_probe_args(encoder))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);

    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(super) fn first_available_encoder<F>(candidates: &[VideoEncoder], mut probe: F) -> VideoEncoder
where
    F: FnMut(VideoEncoder) -> bool,
{
    candidates
        .iter()
        .copied()
        .find(|encoder| probe(*encoder))
        .unwrap_or(VideoEncoder::Software)
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn windows_encoder_candidates() -> [VideoEncoder; 3] {
    [
        VideoEncoder::Nvenc,
        VideoEncoder::QuickSync,
        VideoEncoder::Amf,
    ]
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(super) fn select_video_encoder_uncached(ffmpeg_path: &Path) -> VideoEncoder {
    #[cfg(target_os = "macos")]
    {
        return first_available_encoder(&[VideoEncoder::VideoToolbox], |encoder| {
            probe_video_encoder(ffmpeg_path, encoder)
        });
    }
    #[cfg(target_os = "windows")]
    {
        // A synthetic one-frame probe can reject a perfectly usable Windows
        // encoder while the NVIDIA/Intel/AMD driver is still warming up. The
        // real conversion is the authoritative capability test on Windows.
        let _ = ffmpeg_path;
        return VideoEncoder::Nvenc;
    }
    #[allow(unreachable_code)]
    VideoEncoder::Software
}

pub(super) fn cached_video_encoder_selection<F>(
    cache: &Mutex<EncoderSelectionCache>,
    ffmpeg_path: &Path,
    modified: Option<SystemTime>,
    select: F,
) -> VideoEncoder
where
    F: FnOnce() -> VideoEncoder,
{
    if let Ok(cache) = cache.lock() {
        if let Some((cached_path, cached_modified, cached_encoder)) = cache.as_ref() {
            if cached_path == ffmpeg_path && *cached_modified == modified {
                return *cached_encoder;
            }
        }
    }

    let selected = select();
    if let Ok(mut cache) = cache.lock() {
        *cache = Some((ffmpeg_path.to_path_buf(), modified, selected));
    }
    selected
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(super) fn select_video_encoder(ffmpeg_path: &Path) -> VideoEncoder {
    let modified = fs::metadata(ffmpeg_path)
        .and_then(|metadata| metadata.modified())
        .ok();
    cached_video_encoder_selection(encoder_selection_cache(), ffmpeg_path, modified, || {
        select_video_encoder_uncached(ffmpeg_path)
    })
}

#[cfg(any(not(target_os = "windows"), test))]
pub(super) fn conversion_attempt_encoders(selected: VideoEncoder) -> Vec<VideoEncoder> {
    if selected != VideoEncoder::Software {
        vec![selected, VideoEncoder::Software]
    } else {
        vec![selected]
    }
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn windows_conversion_attempt_encoders(
    cached: Option<VideoEncoder>,
) -> Vec<VideoEncoder> {
    let mut candidates = windows_encoder_candidates().to_vec();
    if let Some(preferred) = cached.filter(|encoder| candidates.contains(encoder)) {
        candidates.retain(|encoder| *encoder != preferred);
        candidates.insert(0, preferred);
    }
    candidates.push(VideoEncoder::Software);
    candidates
}

#[cfg(target_os = "windows")]
pub(super) fn cached_windows_video_encoder(ffmpeg_path: &Path) -> Option<VideoEncoder> {
    let modified = fs::metadata(ffmpeg_path)
        .and_then(|metadata| metadata.modified())
        .ok();
    encoder_selection_cache()
        .lock()
        .ok()
        .and_then(|cache| match cache.as_ref() {
            Some((cached_path, cached_modified, encoder))
                if cached_path == ffmpeg_path && *cached_modified == modified =>
            {
                Some(*encoder)
            }
            _ => None,
        })
}

#[cfg(target_os = "windows")]
pub(super) fn remember_windows_video_encoder(ffmpeg_path: &Path, encoder: VideoEncoder) {
    // Failure can be specific to this input, resolution, or temporary GPU load.
    if encoder == VideoEncoder::Software {
        return;
    }
    let modified = fs::metadata(ffmpeg_path)
        .and_then(|metadata| metadata.modified())
        .ok();
    if let Ok(mut cache) = encoder_selection_cache().lock() {
        *cache = Some((ffmpeg_path.to_path_buf(), modified, encoder));
    }
}

#[cfg(test)]
pub(super) fn parse_media_codecs(metadata: &serde_json::Value) -> Option<(String, Vec<String>)> {
    let streams = metadata.get("streams")?.as_array()?;
    let video = streams.iter().find_map(|stream| {
        if stream.get("codec_type")?.as_str()? != "video" {
            return None;
        }
        Some(stream.get("codec_name")?.as_str()?.to_ascii_lowercase())
    })?;
    let audio = streams
        .iter()
        .filter_map(|stream| {
            if stream.get("codec_type")?.as_str()? != "audio" {
                return None;
            }
            Some(stream.get("codec_name")?.as_str()?.to_ascii_lowercase())
        })
        .collect();
    Some((video, audio))
}

pub(super) fn is_ready_compatible_mp4(
    media: &DownloadedMedia,
    ffmpeg_path: &Path,
) -> Result<bool, String> {
    let is_mp4 = media
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
    if !is_mp4 {
        return Ok(false);
    }
    // Extractor metadata can reject incompatibility cheaply, never prove HDR/bit depth.
    if media
        .video_codec
        .as_deref()
        .is_some_and(|c| !(c.starts_with("h264") || c.starts_with("avc1") || c == "avc"))
        || media
            .audio_codec
            .as_deref()
            .is_some_and(|c| !(c == "none" || c.starts_with("aac") || c.starts_with("mp4a")))
    {
        return Ok(false);
    }
    probe(ffmpeg_path, &media.path).map(|plan| plan.copy_video && plan.copy_audio && !plan.tone_map)
}

pub(super) fn conversion_error_code(error: &str) -> &'static str {
    if error.to_lowercase().contains("недостатньо вільного місця") {
        return "low_disk";
    }
    let code = download_error_code(error);
    if matches!(code, "low_disk" | "auth_required" | "cancelled") {
        code
    } else {
        "conversion_failed"
    }
}

#[test]
fn storage_errors_survive_conversion_classification() {
    assert_eq!(
        conversion_error_code("Недостатньо вільного місця. Конвертацію зупинено"),
        "low_disk"
    );
    assert_eq!(conversion_error_code("No space left on device"), "low_disk");
    assert_eq!(conversion_error_code("Encoder failed"), "conversion_failed");
}

pub(super) fn requires_app_conversion(
    media: &DownloadedMedia,
    output_format: &str,
    ffmpeg_path: &Path,
) -> Result<bool, String> {
    if output_format != "mp4" {
        return Ok(true);
    }
    is_ready_compatible_mp4(media, ffmpeg_path).map(|ready| !ready)
}

pub(super) fn video_encoding_args(
    encoder: VideoEncoder,
    bitrate: u64,
    maxrate: u64,
    bufsize: u64,
) -> Vec<String> {
    vec![
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a:0?".into(),
        "-map_metadata".into(),
        "0".into(),
        "-c:v".into(),
        encoder.codec().into(),
        "-b:v".into(),
        bitrate.to_string(),
        "-maxrate".into(),
        maxrate.to_string(),
        "-bufsize".into(),
        bufsize.to_string(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-profile:a".into(),
        "aac_low".into(),
        "-b:a".into(),
        "192k".into(),
        "-tag:v".into(),
        "avc1".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]
}

pub(super) fn cleanup_failed_conversion(temporary: &Path) {
    let _ = fs::remove_file(temporary);
}

pub(super) fn create_download_temp_dir(output_dir: &Path, id: &str) -> Result<PathBuf, String> {
    Uuid::parse_str(id).map_err(|_| "Некоректний ідентифікатор завантаження".to_string())?;
    let directory = output_dir.join(format!(".yt-dlp-bd-{id}.tmp"));
    fs::create_dir(&directory)
        .map_err(|error| format!("Не вдалося створити тимчасову папку завантаження: {error}"))?;
    Ok(directory)
}

pub(super) fn cleanup_download_temp_dir(directory: &Path) {
    let owned = directory
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.starts_with(".yt-dlp-bd-") && name.ends_with(".tmp"));
    if owned {
        let _ = fs::remove_dir_all(directory);
    }
}

pub(super) fn reset_download_temp_dir(directory: &Path) -> Result<(), String> {
    cleanup_download_temp_dir(directory);
    fs::create_dir(directory)
        .map_err(|error| format!("Не вдалося очистити тимчасові файли перед повтором: {error}"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_conversion_attempt(
    app: &AppHandle,
    manager: &DownloadManager,
    id: &str,
    ffmpeg_path: &Path,
    input: &Path,
    temporary: &Path,
    output_dir: &Path,
    media: Option<&MediaInfo>,
    duration: f64,
    file_position: (usize, usize),
    output_format: &str,
    encoder: VideoEncoder,
    processing: conversion::ProcessingPlan,
) -> Result<(), ConversionAttemptError> {
    let (file_index, file_count) = file_position;
    diagnostics::record(
        id,
        &format!(
            "Обробка: {}; copy_video={}; copy_audio={}; HDR→SDR={}",
            encoder.label(),
            processing.copy_video,
            processing.copy_audio,
            processing.tone_map
        ),
    );
    let mut command = Command::new(ffmpeg_path);
    configure_command(&mut command);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-stats_period",
            "0.25",
            "-progress",
            "pipe:1",
        ])
        .args(LOCAL_INPUT_PROTOCOLS)
        .arg("-i")
        .arg(input);
    if let Some(media) = media {
        let bitrate = target_video_bitrate(Some(media.height), Some(media.fps), None);
        command.args(conversion::args(processing, encoder, bitrate));
    } else {
        command.args(["-map", "0:a:0", "-vn", "-map_metadata", "0"]);
        match output_format {
            "m4a" => {
                command.args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]);
            }
            "opus" => {
                command.args(["-c:a", "libopus", "-b:a", "160k"]);
            }
            "wav" => {
                command.args(["-c:a", "pcm_s16le", "-ar", "48000"]);
            }
            _ => {
                command.args(["-c:a", "libmp3lame", "-b:a", "192k"]);
            }
        }
    }
    command
        .arg(temporary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    command.process_group(0);

    let (child, stdout, stderr) =
        job_process::spawn(manager, id, &mut command).map_err(|error| {
            if download_was_cancelled(manager, id) {
                ConversionAttemptError::Cancelled
            } else {
                ConversionAttemptError::Failed(format!("Не вдалося запустити ffmpeg: {error}"))
            }
        })?;

    let errors = Arc::new(Mutex::new(String::new()));
    let error_reader = stderr.map(|stderr| {
        let errors = errors.clone();
        thread::spawn(move || {
            let text = read_lossy_to_string(BufReader::new(stderr)).unwrap_or_default();
            if let Ok(mut target) = errors.lock() {
                *target = text;
            }
        })
    });

    let mut out_time_us = 0.0;
    let mut speed_value = 0.0;
    let format_label = output_format.to_ascii_uppercase();
    let encoder_label = if processing.copy_video {
        if processing.copy_audio {
            "Без перекодування"
        } else {
            "Копіюємо відео · AAC"
        }
    } else if media.is_some() {
        encoder.label()
    } else {
        "FFmpeg"
    };
    let mut speed_label = encoder_label.to_string();
    let mut low_disk = false;
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = line.strip_prefix("out_time_us=") {
                out_time_us = value.trim().parse::<f64>().unwrap_or(out_time_us);
            } else if let Some(value) = line.strip_prefix("speed=") {
                let value = value.trim();
                speed_label = if value.is_empty() || value == "N/A" {
                    encoder_label.into()
                } else {
                    format!("{encoder_label} · {value}")
                };
                speed_value = value
                    .trim_end_matches('x')
                    .parse::<f64>()
                    .unwrap_or(speed_value);
            } else if line.starts_with("progress=") {
                if available_disk_space(output_dir)
                    .map(low_disk_guard_triggered)
                    .unwrap_or(true)
                {
                    low_disk = true;
                    let _ = stop_child_process_group(&child);
                    break;
                }
                let elapsed = out_time_us / 1_000_000.0;
                let overall_percent = conversion_percent(elapsed, duration, file_index, file_count);
                let remaining = if speed_value > 0.0 {
                    format_eta((duration - elapsed).max(0.0) / speed_value)
                } else {
                    "—".into()
                };
                let _ = app.emit(
                    "download-event",
                    DownloadEvent {
                        id: id.into(),
                        kind: "conversion_progress".into(),
                        percent: Some(overall_percent),
                        speed: Some(speed_label.clone()),
                        eta: Some(remaining),
                        message: Some(if file_count > 1 {
                            format!(
                                "Створюємо {} — файл {} із {}… Не закривайте застосунок",
                                format_label,
                                file_index + 1,
                                file_count
                            )
                        } else {
                            format!("Створюємо {format_label}… Не закривайте застосунок")
                        }),
                        storage: None,
                        outputs: None,
                        title: None,
                        thumbnail: None,
                        uploader: None,
                        extractor: None,
                        error_code: None,
                    },
                );
            }
        }
    }

    let status = loop {
        let wait = child
            .lock()
            .map_err(|_| ConversionAttemptError::Failed("Внутрішня помилка FFmpeg".into()))?
            .try_wait()
            .map_err(|error| {
                ConversionAttemptError::Failed(format!(
                    "Не вдалося перевірити стан FFmpeg: {error}"
                ))
            })?;
        if let Some(status) = wait {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if let Some(reader) = error_reader {
        let _ = reader.join();
    }

    if download_was_cancelled(manager, id) {
        return Err(ConversionAttemptError::Cancelled);
    }
    if low_disk {
        return Err(ConversionAttemptError::LowDisk);
    }
    if !status.success() {
        let detail = errors
            .lock()
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "ffmpeg завершився з помилкою".into());
        return Err(ConversionAttemptError::Failed(detail));
    }
    Ok(())
}

pub(super) fn convert_downloaded_media(
    app: &AppHandle,
    manager: &DownloadManager,
    id: &str,
    ffmpeg_path: &Path,
    downloaded: &DownloadedMedia,
    file_position: (usize, usize),
    output_format: &str,
) -> Result<PathBuf, String> {
    let input = &downloaded.path;
    if download_was_cancelled(manager, id) {
        return Err("Завантаження скасовано".into());
    }

    let is_video = output_format == "mp4";
    let processing = if is_video {
        conversion::probe(ffmpeg_path, input)?
    } else {
        conversion::ProcessingPlan::default()
    };
    let media = is_video
        .then(|| {
            downloaded
                .duration
                .map(|duration| MediaInfo {
                    duration,
                    height: downloaded.height.unwrap_or(1080),
                    fps: downloaded.fps.unwrap_or(30.0),
                })
                .map(Ok)
                .unwrap_or_else(|| media_info(ffmpeg_path, input))
        })
        .transpose()?;
    let duration = if let Some(media) = media.as_ref() {
        media.duration
    } else if let Some(duration) = downloaded.duration {
        duration
    } else {
        media_duration(ffmpeg_path, input)?
    };
    let output = available_output_path(input, output_format);
    let output_dir = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    if available_disk_space(&output_dir)
        .map(low_disk_guard_triggered)
        .unwrap_or(true)
    {
        return Err(
            "Недостатньо вільного місця. Конвертацію не розпочато, щоб захистити диск.".into(),
        );
    }
    let temporary = output_dir
        .join(format!(".yt-dlp-bd-{id}.tmp"))
        .join(format!(
            ".{}.{}.part.{}",
            output
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("video"),
            Uuid::new_v4(),
            output_format
        ));
    #[cfg(target_os = "windows")]
    let attempts = if is_video && !processing.copy_video {
        windows_conversion_attempt_encoders(cached_windows_video_encoder(ffmpeg_path))
    } else {
        vec![VideoEncoder::Software]
    };
    #[cfg(not(target_os = "windows"))]
    let attempts = conversion_attempt_encoders(if is_video && !processing.copy_video {
        select_video_encoder(ffmpeg_path)
    } else {
        VideoEncoder::Software
    });
    let format_label = output_format.to_ascii_uppercase();

    for (index, encoder) in attempts.iter().copied().enumerate() {
        if download_was_cancelled(manager, id) {
            cleanup_failed_conversion(&temporary);
            return Err("Завантаження скасовано".into());
        }
        if index > 0 {
            let _ = app.emit(
                "download-event",
                DownloadEvent {
                    id: id.into(),
                    kind: "conversion_progress".into(),
                    percent: Some(conversion_percent(
                        0.0,
                        duration,
                        file_position.0,
                        file_position.1,
                    )),
                    speed: Some(encoder.label().into()),
                    eta: Some("—".into()),
                    message: Some(format!(
                        "Апаратний кодер не завершив обробку — продовжуємо через {}…",
                        encoder.label()
                    )),
                    storage: None,
                    outputs: None,
                    title: None,
                    thumbnail: None,
                    uploader: None,
                    extractor: None,
                    error_code: None,
                },
            );
        }
        cleanup_failed_conversion(&temporary);
        match run_conversion_attempt(
            app,
            manager,
            id,
            ffmpeg_path,
            input,
            &temporary,
            &output_dir,
            media.as_ref(),
            duration,
            file_position,
            output_format,
            encoder,
            processing,
        ) {
            Ok(()) => {
                #[cfg(target_os = "windows")]
                if is_video && !processing.copy_video {
                    remember_windows_video_encoder(ffmpeg_path, encoder);
                }
                if let Err(error) = fs::rename(&temporary, &output) {
                    cleanup_failed_conversion(&temporary);
                    return Err(format!(
                        "Не вдалося зберегти готовий {format_label}: {error}"
                    ));
                }
                let _ = fs::remove_file(input);
                return Ok(output);
            }
            Err(ConversionAttemptError::Cancelled) => {
                cleanup_failed_conversion(&temporary);
                return Err("Завантаження скасовано".into());
            }
            Err(ConversionAttemptError::LowDisk) => {
                cleanup_failed_conversion(&temporary);
                return Err(
                    "Недостатньо вільного місця. Конвертацію зупинено, щоб захистити диск.".into(),
                );
            }
            Err(ConversionAttemptError::Failed(detail)) => {
                diagnostics::record(id, &format!("{}: {detail}", encoder.label()));
                cleanup_failed_conversion(&temporary);
                if index + 1 == attempts.len() {
                    return Err(format!("Не вдалося створити {format_label}: {detail}"));
                }
                eprintln!(
                    "Кодер {} не завершив конвертацію, повторюємо через libx264: {detail}",
                    encoder.label()
                );
            }
        }
    }
    Err(format!("Не вдалося створити {format_label}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_diagnostics_preserve_reason_without_private_paths() {
        let message = redacted_probe_stderr(
            b"/Users/private/video.mp4: failed\nmoov atom not found\nCookie: SECRET",
        );
        assert!(message.contains("moov atom not found"));
        assert!(!message.contains("/Users/") && !message.contains("SECRET"));
    }

    #[test]
    fn missing_probe_is_an_error_not_a_conversion_request() {
        let missing = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let media = DownloadedMedia {
            identity: MediaIdentity::default(),
            path: missing.join("ready.mp4"),
            duration: Some(1.0),
            height: None,
            fps: None,
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
        };
        let error = requires_app_conversion(&media, "mp4", &missing.join("ffmpeg"))
            .expect_err("A failed probe must never request speculative conversion");
        assert!(error.contains("ffprobe"));
        assert!(!friendly_conversion_error(&error).contains("пошкоджений"));
    }

    #[test]
    fn copy_streams_and_convert_only_incompatible_audio() {
        let mut value = serde_json::json!({"streams":[{"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p"},{"codec_type":"audio","codec_name":"aac"}]});
        assert_eq!(
            from_streams(&value),
            ProcessingPlan {
                copy_video: true,
                copy_audio: true,
                tone_map: false
            }
        );
        value["streams"][1]["codec_name"] = "opus".into();
        assert!(!from_streams(&value).copy_audio);
        assert!(from_streams(&value).copy_video);
        value["streams"][0]["color_transfer"] = "smpte2084".into();
        assert!(from_streams(&value).tone_map);
        assert!(!from_streams(&value).copy_video);
    }
    #[test]
    fn software_uses_quality_target_and_hdr_has_explicit_mapping() {
        let args = args(
            ProcessingPlan {
                tone_map: true,
                ..Default::default()
            },
            VideoEncoder::Software,
            14000000,
        );
        assert!(args.iter().any(|a| a == "-crf"));
        assert!(!args.iter().any(|a| a == "-b:v"));
        assert!(args.iter().any(|a| a.contains("tonemap=hable")));
    }
}
