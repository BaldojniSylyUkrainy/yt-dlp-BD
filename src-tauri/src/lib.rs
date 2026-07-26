use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Cursor, Read},
    net::ToSocketAddrs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const YTDLP_RELEASE_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentStatus {
    installed: bool,
    version: Option<String>,
    path: Option<String>,
    managed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    yt_dlp: ComponentStatus,
    ffmpeg: ComponentStatus,
    deno: ComponentStatus,
    runtime_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VpnStatus {
    detected: bool,
    interface_name: Option<String>,
    confidence: String,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaPreview {
    title: String,
    thumbnail: Option<String>,
    duration: Option<f64>,
    uploader: Option<String>,
    extractor: Option<String>,
    webpage_url: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRequest {
    url: String,
    output_dir: String,
    mode: String,
    quality: String,
    audio_format: String,
    subtitles: bool,
    multi_item: bool,
    cookies_browser: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadEvent {
    id: String,
    kind: String,
    percent: Option<f32>,
    speed: Option<String>,
    eta: Option<String>,
    message: Option<String>,
    storage: Option<StorageEstimate>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageEstimate {
    estimated_size: u64,
    required_space: u64,
    available_space: Option<u64>,
    sufficient: Option<bool>,
}

#[derive(Clone)]
struct StorageTracker {
    output_dir: PathBuf,
    estimates: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    video: bool,
    audio_format: String,
}

struct SelectedMediaEstimate {
    media_id: String,
    source_size: u64,
    duration: Option<f64>,
    height: Option<u64>,
    fps: Option<f64>,
    total_bitrate_kbps: Option<f64>,
}

struct MediaInfo {
    duration: f64,
    height: u64,
    fps: f64,
}

#[derive(Clone, Default)]
struct DownloadManager {
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    cancelled: Arc<Mutex<HashSet<String>>>,
    auth_required: Arc<Mutex<HashSet<String>>>,
}

fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("runtime");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не вдалося створити runtime-папку: {error}"))?;
    Ok(directory)
}

fn yt_dlp_path(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let filename = "yt-dlp.exe";
    #[cfg(not(target_os = "windows"))]
    let filename = "yt-dlp";
    Ok(runtime_dir(app)?.join(filename))
}

fn command_version(path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(path).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn find_ffmpeg(app: &AppHandle) -> Result<ComponentStatus, String> {
    let directory = runtime_dir(app)?;
    let managed_path = directory.join(if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    let probe_path = directory.join(if cfg!(target_os = "windows") {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });

    if managed_path.is_file() && probe_path.is_file() {
        if let (Some(version), Some(_)) = (
            command_version(&managed_path, &["-version"]),
            command_version(&probe_path, &["-version"]),
        ) {
            let first_line = version.lines().next().unwrap_or(&version).to_string();
            return Ok(ComponentStatus {
                installed: true,
                version: Some(first_line),
                path: Some(managed_path.to_string_lossy().to_string()),
                managed: true,
            });
        }
    }

    Ok(ComponentStatus {
        installed: false,
        version: None,
        path: None,
        managed: true,
    })
}

fn find_deno(app: &AppHandle) -> Result<ComponentStatus, String> {
    let managed_path = runtime_dir(app)?.join(if cfg!(target_os = "windows") {
        "deno.exe"
    } else {
        "deno"
    });
    if managed_path.is_file() {
        if let Some(version) = command_version(&managed_path, &["--version"]) {
            return Ok(ComponentStatus {
                installed: true,
                version: Some(version.lines().next().unwrap_or(&version).to_string()),
                path: Some(managed_path.to_string_lossy().to_string()),
                managed: true,
            });
        }
    }

    Ok(ComponentStatus {
        installed: false,
        version: None,
        path: None,
        managed: true,
    })
}

#[tauri::command]
fn runtime_status(app: AppHandle) -> Result<RuntimeStatus, String> {
    let directory = runtime_dir(&app)?;
    let path = yt_dlp_path(&app)?;
    let version = path
        .is_file()
        .then(|| command_version(&path, &["--version"]))
        .flatten();

    Ok(RuntimeStatus {
        yt_dlp: ComponentStatus {
            installed: version.is_some(),
            version,
            path: path.is_file().then(|| path.to_string_lossy().to_string()),
            managed: true,
        },
        ffmpeg: find_ffmpeg(&app)?,
        deno: find_deno(&app)?,
        runtime_dir: directory.to_string_lossy().to_string(),
    })
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let bytes = client
        .get(url)
        .header("User-Agent", "yt-dlp-desktop")
        .send()
        .await
        .map_err(|error| format!("Помилка мережі: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Сервер повернув помилку: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("Не вдалося прочитати відповідь: {error}"))?;
    Ok(bytes.to_vec())
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Не вдалося підготувати завантаження: {error}"))
}

fn ffmpeg_release_asset(page: &str, filename: &str) -> Result<String, String> {
    let release_section = page
        .split_once("<h2>Download Release Build</h2>")
        .map(|(_, section)| section)
        .ok_or_else(|| "Сервер ffmpeg не повідомив про стабільний реліз".to_string())?;
    let architecture_prefix = "/download/macos/arm64/";

    for candidate in release_section.split("href=\"").skip(1) {
        let Some(path) = candidate.split('"').next() else {
            continue;
        };
        if path.starts_with(architecture_prefix) && path.ends_with(filename) {
            return Ok(format!("https://ffmpeg.martin-riedl.de{path}"));
        }
    }

    Err(format!(
        "Сервер ffmpeg не надав {filename} для macOS Apple Silicon"
    ))
}

fn checksum_value(checksum_file: &[u8], label: &str) -> Result<String, String> {
    let checksum = String::from_utf8(checksum_file.to_vec())
        .map_err(|_| format!("Контрольна сума {label} має неправильний формат"))?;
    let expected = checksum
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("Контрольна сума {label} порожня"))?
        .to_ascii_lowercase();
    Ok(expected)
}

fn verify_sha256(bytes: &[u8], checksum_file: &[u8], label: &str) -> Result<(), String> {
    let expected = checksum_value(checksum_file, label)?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(format!("Контрольна сума {label} не збігається"));
    }
    Ok(())
}

fn extract_binary(
    archive_bytes: &[u8],
    binary_name: &str,
    destination: &Path,
) -> Result<(), String> {
    let reader = Cursor::new(archive_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| format!("Не вдалося відкрити архів {binary_name}: {error}"))?;
    let index = (0..archive.len())
        .find(|index| {
            archive
                .by_index(*index)
                .ok()
                .and_then(|file| {
                    Path::new(file.name())
                        .file_name()
                        .map(|name| name == binary_name)
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("В архіві не знайдено {binary_name}"))?;
    let mut file = archive
        .by_index(index)
        .map_err(|error| format!("Не вдалося прочитати {binary_name}: {error}"))?;
    let temporary = destination.with_extension("download");
    let mut output = fs::File::create(&temporary)
        .map_err(|error| format!("Не вдалося створити {binary_name}: {error}"))?;
    std::io::copy(&mut file, &mut output)
        .map_err(|error| format!("Не вдалося розпакувати {binary_name}: {error}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Не вдалося дозволити запуск {binary_name}: {error}"))?;
    }
    if destination.exists() {
        fs::remove_file(destination)
            .map_err(|error| format!("Не вдалося замінити старий {binary_name}: {error}"))?;
    }
    fs::rename(&temporary, destination)
        .map_err(|error| format!("Не вдалося встановити {binary_name}: {error}"))
}

#[tauri::command]
async fn install_ffmpeg(app: AppHandle) -> Result<(), String> {
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    return Err("Portable ffmpeg поки налаштовано лише для macOS Apple Silicon".into());

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        const RELEASES_PAGE: &str = "https://ffmpeg.martin-riedl.de/";
        let client = download_client()?;
        let page = fetch_bytes(&client, RELEASES_PAGE).await?;
        let page = String::from_utf8(page)
            .map_err(|_| "Сторінка релізів ffmpeg має неправильний формат".to_string())?;
        let ffmpeg_url = ffmpeg_release_asset(&page, "ffmpeg.zip")?;
        let ffprobe_url = ffmpeg_release_asset(&page, "ffprobe.zip")?;
        eprintln!("Перевіряємо стабільний ffmpeg: {ffmpeg_url}");
        let ffmpeg_checksum_url = format!("{ffmpeg_url}.sha256");
        let ffprobe_checksum_url = format!("{ffprobe_url}.sha256");
        let (ffmpeg_checksum, ffprobe_checksum) = tokio::try_join!(
            fetch_bytes(&client, &ffmpeg_checksum_url),
            fetch_bytes(&client, &ffprobe_checksum_url)
        )?;
        let ffmpeg_expected = checksum_value(&ffmpeg_checksum, "ffmpeg")?;
        let ffprobe_expected = checksum_value(&ffprobe_checksum, "ffprobe")?;
        let directory = runtime_dir(&app)?;
        let ffmpeg_path = directory.join("ffmpeg");
        let ffprobe_path = directory.join("ffprobe");
        let ffmpeg_stamp = directory.join(".ffmpeg.sha256");
        let ffprobe_stamp = directory.join(".ffprobe.sha256");
        let current = command_version(&ffmpeg_path, &["-version"]).is_some()
            && command_version(&ffprobe_path, &["-version"]).is_some()
            && fs::read_to_string(&ffmpeg_stamp)
                .map(|value| value.trim() == ffmpeg_expected)
                .unwrap_or(false)
            && fs::read_to_string(&ffprobe_stamp)
                .map(|value| value.trim() == ffprobe_expected)
                .unwrap_or(false);
        if current {
            eprintln!("Керований ffmpeg вже актуальний");
            return Ok(());
        }

        eprintln!("Завантажуємо керовані ffmpeg та ffprobe");
        let (ffmpeg_zip, ffprobe_zip) = tokio::try_join!(
            fetch_bytes(&client, &ffmpeg_url),
            fetch_bytes(&client, &ffprobe_url)
        )?;
        verify_sha256(&ffmpeg_zip, &ffmpeg_checksum, "ffmpeg")?;
        verify_sha256(&ffprobe_zip, &ffprobe_checksum, "ffprobe")?;
        extract_binary(&ffmpeg_zip, "ffmpeg", &ffmpeg_path)?;
        extract_binary(&ffprobe_zip, "ffprobe", &ffprobe_path)?;
        fs::write(ffmpeg_stamp, ffmpeg_expected)
            .map_err(|error| format!("Не вдалося зберегти версію ffmpeg: {error}"))?;
        fs::write(ffprobe_stamp, ffprobe_expected)
            .map_err(|error| format!("Не вдалося зберегти версію ffprobe: {error}"))?;
        Ok(())
    }
}

#[tauri::command]
async fn install_deno(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let asset_name = if cfg!(target_arch = "aarch64") {
        "deno-aarch64-apple-darwin.zip"
    } else {
        "deno-x86_64-apple-darwin.zip"
    };
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let asset_name = "deno-x86_64-pc-windows-msvc.zip";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    let asset_name = "deno-aarch64-pc-windows-msvc.zip";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("Ця платформа поки що не підтримується".into());

    let base = "https://github.com/denoland/deno/releases/latest/download";
    let archive_url = format!("{base}/{asset_name}");
    let checksum_url = format!("{base}/{asset_name}.sha256sum");
    let client = download_client()
        .map_err(|error| format!("Не вдалося підготувати завантаження Deno: {error}"))?;
    let checksum = fetch_bytes(&client, &checksum_url).await?;
    let expected = checksum_value(&checksum, "Deno")?;
    let filename = if cfg!(target_os = "windows") {
        "deno.exe"
    } else {
        "deno"
    };
    let directory = runtime_dir(&app)?;
    let destination = directory.join(filename);
    let stamp = directory.join(".deno.sha256");
    let current = command_version(&destination, &["--version"]).is_some()
        && fs::read_to_string(&stamp)
            .map(|value| value.trim() == expected)
            .unwrap_or(false);
    if current {
        return Ok(());
    }

    let archive = fetch_bytes(&client, &archive_url).await?;
    verify_sha256(&archive, &checksum, "Deno")?;
    extract_binary(&archive, filename, &destination)?;
    fs::write(stamp, expected)
        .map_err(|error| format!("Не вдалося зберегти версію Deno: {error}"))?;
    Ok(())
}

#[tauri::command]
async fn install_ytdlp(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let asset_name = "yt-dlp_macos";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let asset_name = "yt-dlp.exe";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    let asset_name = "yt-dlp_arm64.exe";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("Ця платформа поки що не підтримується".into());

    let client = download_client()?;
    let binary_url = format!("{YTDLP_RELEASE_BASE}/{asset_name}");
    let checksums_url = format!("{YTDLP_RELEASE_BASE}/SHA2-256SUMS");

    let checksums = fetch_bytes(&client, &checksums_url).await?;
    let checksums = String::from_utf8(checksums)
        .map_err(|_| "Файл контрольних сум має неправильний формат".to_string())?;
    let expected = checksums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?))
        })
        .find_map(|(hash, filename)| {
            (filename.trim_start_matches('*') == asset_name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| "Не вдалося знайти контрольну суму yt-dlp".to_string())?;

    let destination = yt_dlp_path(&app)?;
    let current = fs::read(&destination)
        .map(|binary| format!("{:x}", Sha256::digest(binary)) == expected)
        .unwrap_or(false)
        && command_version(&destination, &["--version"]).is_some();
    if current {
        return Ok(());
    }

    eprintln!("Завантажуємо стабільний yt-dlp");
    let binary = fetch_bytes(&client, &binary_url).await?;
    let actual = format!("{:x}", Sha256::digest(&binary));
    if actual != expected {
        return Err("Контрольна сума yt-dlp не збігається. Файл не встановлено".into());
    }

    let temporary = destination.with_extension("download");
    fs::write(&temporary, &binary)
        .map_err(|error| format!("Не вдалося записати yt-dlp: {error}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Не вдалося дозволити запуск yt-dlp: {error}"))?;
    }

    if destination.exists() {
        fs::remove_file(&destination)
            .map_err(|error| format!("Не вдалося замінити старий yt-dlp: {error}"))?;
    }
    fs::rename(&temporary, &destination)
        .map_err(|error| format!("Не вдалося завершити встановлення yt-dlp: {error}"))?;

    Ok(())
}

#[tauri::command]
async fn update_ytdlp(app: AppHandle) -> Result<(), String> {
    let result = install_ytdlp(app).await;
    if let Err(error) = &result {
        eprintln!("Не вдалося оновити yt-dlp: {error}");
    }
    result
}

fn clean_probe_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let raw = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("yt-dlp не зміг перевірити це посилання")
        .trim()
        .trim_start_matches("ERROR:")
        .trim()
        .to_string();
    friendly_download_error(&raw)
}

fn friendly_download_error(message: &str) -> String {
    let value = message.to_ascii_lowercase();
    if ["truncated_id", "incomplete youtube id", "looks truncated"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Посилання YouTube неповне. Скопіюйте його ще раз з адресного рядка або через «Поділитися»."
            .into()
    } else if ["private video", "this video is private"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Відео приватне. Для завантаження потрібен доступ через ваш браузер.".into()
    } else if [
        "sign in",
        "login required",
        "log in",
        "cookies are required",
        "confirm your age",
        "age-restricted",
        "members-only",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "Сервіс просить увійти в обліковий запис. Спробуйте завантаження з cookies браузера.".into()
    } else if ["unsupported url", "no suitable extractor"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Цей сайт або тип посилання поки не підтримується.".into()
    } else if value.contains("drm") {
        "Відео захищене DRM і не може бути завантажене.".into()
    } else if ["geo", "country", "region"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Відео недоступне у вашому регіоні.".into()
    } else if ["video unavailable", "not available", "removed"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Відео недоступне або було видалене.".into()
    } else if ["404", "not found"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "За цим посиланням нічого не знайдено. Перевірте, чи воно скопійоване повністю.".into()
    } else if ["403", "forbidden"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Сайт відхилив доступ до медіафайла. Спробуйте ще раз або скористайтеся іншим підключенням."
            .into()
    } else {
        "Не вдалося прочитати це посилання. Перевірте його та спробуйте ще раз.".into()
    }
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

async fn probe_oembed(url: &str, endpoint: &str, extractor: &str) -> Result<MediaPreview, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|error| format!("Не вдалося підготувати швидку перевірку: {error}"))?;
    let response = client
        .get(endpoint)
        .query(&[("url", url), ("format", "json")])
        .header("User-Agent", "yt-dlp-desktop")
        .send()
        .await
        .map_err(|_| "Сервіс не відповів під час швидкої перевірки".to_string())?;
    if !response.status().is_success() {
        return Err("Відео не існує, закрите або недоступне".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Не вдалося прочитати дані про відео".to_string())?;
    let metadata: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Сервіс повернув пошкоджені дані про відео".to_string())?;
    let title = metadata
        .get("title")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "У посиланні не знайдено доступного відео".to_string())?;
    Ok(MediaPreview {
        title: title.to_string(),
        thumbnail: metadata
            .get("thumbnail_url")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        duration: metadata.get("duration").and_then(|value| value.as_f64()),
        uploader: metadata
            .get("author_name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        extractor: Some(extractor.to_string()),
        webpage_url: Some(url.to_string()),
    })
}

#[tauri::command]
async fn probe_url(app: AppHandle, url: String) -> Result<MediaPreview, String> {
    let parsed = Url::parse(&url).map_err(|_| "Вставте повне посилання".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Підтримуються лише HTTP та HTTPS посилання".into());
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if host_matches(&host, "youtube.com") || host_matches(&host, "youtu.be") {
        return probe_oembed(&url, "https://www.youtube.com/oembed", "YouTube").await;
    }
    if host_matches(&host, "vimeo.com") {
        return probe_oembed(&url, "https://vimeo.com/api/oembed.json", "Vimeo").await;
    }

    let yt_dlp = yt_dlp_path(&app)?;
    if !yt_dlp.is_file() {
        return Err("yt-dlp ще не встановлено".into());
    }

    let mut command = tokio::process::Command::new(yt_dlp);
    command.args([
        "--ignore-config",
        "--simulate",
        "--no-warnings",
        "--no-playlist",
        "--no-ignore-no-formats-error",
        "--socket-timeout",
        "6",
        "--extractor-retries",
        "1",
        "--print",
        "%(.{title,thumbnail,duration,uploader,channel,extractor,extractor_key,webpage_url})#j",
    ]);
    if let Some(path) = find_deno(&app)?.path {
        if path == "deno" {
            command.args(["--js-runtimes", "deno"]);
        } else {
            command.args(["--js-runtimes", &format!("deno:{path}")]);
        }
    }
    command.arg(&url).kill_on_drop(true);

    let output = tokio::time::timeout(Duration::from_secs(12), command.output())
        .await
        .map_err(|_| "Перевірка тривала надто довго. Спробуйте ще раз".to_string())?
        .map_err(|error| format!("Не вдалося запустити yt-dlp: {error}"))?;
    if !output.status.success() {
        return Err(clean_probe_error(&output.stderr));
    }

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "yt-dlp повернув пошкоджені дані про відео".to_string())?;
    let title = metadata
        .get("title")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "У посиланні не знайдено доступного відео".to_string())?;
    let string_field = |name: &str| {
        metadata
            .get(name)
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    Ok(MediaPreview {
        title: title.to_string(),
        thumbnail: string_field("thumbnail"),
        duration: metadata.get("duration").and_then(|value| value.as_f64()),
        uploader: string_field("uploader").or_else(|| string_field("channel")),
        extractor: string_field("extractor_key").or_else(|| string_field("extractor")),
        webpage_url: string_field("webpage_url"),
    })
}

fn is_tunnel_interface(interface_name: &str) -> bool {
    let value = interface_name.to_ascii_lowercase();
    ["utun", "tun", "tap", "wg", "wireguard", "ppp"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

#[tauri::command]
fn check_vpn(host: String) -> Result<VpnStatus, String> {
    if host.is_empty()
        || host.len() > 253
        || !host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
    {
        return Err("Некоректне доменне ім’я".into());
    }

    #[cfg(target_os = "macos")]
    {
        let ip = (host.as_str(), 443)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addresses| addresses.next())
            .map(|address| address.ip().to_string());
        let interface_name = ip.as_ref().and_then(|ip| {
            Command::new("/sbin/route")
                .args(["-n", "get", ip])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .find_map(|line| line.trim().strip_prefix("interface:").map(str::trim))
                        .map(str::to_string)
                })
        });
        let routed_through_tunnel = interface_name
            .as_deref()
            .map(is_tunnel_interface)
            .unwrap_or(false);
        let detected = routed_through_tunnel;
        let detail = if routed_through_tunnel {
            "Маршрут саме до цього ресурсу проходить через VPN-тунель"
        } else {
            "Маршрут до цього ресурсу не проходить через VPN-тунель"
        };
        return Ok(VpnStatus {
            detected,
            interface_name,
            confidence: "high".into(),
            detail: detail.into(),
        });
    }

    #[cfg(not(target_os = "macos"))]
    Ok(VpnStatus {
        detected: false,
        interface_name: None,
        confidence: "unknown".into(),
        detail: "Перевірку VPN для цієї платформи ще не реалізовано".into(),
    })
}

fn emit_line(app: &AppHandle, id: &str, line: &str) {
    const MARKER: &str = "__YTDLP_PROGRESS__";
    const POSTPROCESS_MARKER: &str = "__YTDLP_POSTPROCESS__";
    if let Some(payload) = line.strip_prefix(MARKER) {
        let mut values = payload.splitn(4, '|');
        let percent = values
            .next()
            .map(str::trim)
            .and_then(|value| value.trim_end_matches('%').trim().parse::<f32>().ok());
        let speed = values
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let eta = values
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let message = values
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let _ = app.emit(
            "download-event",
            DownloadEvent {
                id: id.into(),
                kind: "progress".into(),
                percent,
                speed: speed.map(str::to_string),
                eta: eta.map(str::to_string),
                message: message.map(str::to_string),
                storage: None,
            },
        );
    } else if let Some(payload) = line.strip_prefix(POSTPROCESS_MARKER) {
        let mut values = payload.splitn(3, '|');
        let status = values.next().map(str::trim).unwrap_or_default();
        let postprocessor = values.next().map(str::trim).unwrap_or_default();
        let _title = values.next().map(str::trim).unwrap_or_default();
        if status == "started" {
            let message = if postprocessor.contains("Merger")
                || postprocessor.contains("Convertor")
                || postprocessor.contains("Remuxer")
            {
                "Об’єднуємо завантажені потоки… Не закривайте застосунок"
            } else {
                "Обробляємо файл… Не закривайте застосунок"
            };
            let _ = app.emit(
                "download-event",
                DownloadEvent {
                    id: id.into(),
                    kind: "postprocess".into(),
                    percent: None,
                    speed: None,
                    eta: None,
                    message: Some(message.into()),
                    storage: None,
                },
            );
        }
    } else if !line.trim().is_empty() {
        let normalized = line.to_ascii_lowercase();
        let is_postprocess_log = ["[merger]", "[videoconvertor]", "[videoremuxer]"]
            .iter()
            .any(|marker| normalized.contains(marker));
        let _ = app.emit(
            "download-event",
            DownloadEvent {
                id: id.into(),
                kind: if is_postprocess_log {
                    "postprocess".into()
                } else {
                    "log".into()
                },
                percent: None,
                speed: is_postprocess_log.then(|| "VideoToolbox".into()),
                eta: None,
                message: Some(if is_postprocess_log {
                    "Об’єднуємо завантажені потоки… Не закривайте застосунок".into()
                } else {
                    line.trim().to_string()
                }),
                storage: None,
            },
        );
    }
}

fn looks_like_auth_error(line: &str) -> bool {
    let value = line.to_ascii_lowercase();
    [
        "sign in to confirm",
        "login required",
        "log in to",
        "use --cookies-from-browser",
        "cookies are required",
        "confirm your age",
        "age-restricted",
        "members-only",
        "private video",
        "this video is private",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
}

fn json_number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn parse_size_estimate(payload: &str) -> Option<SelectedMediaEstimate> {
    let metadata: serde_json::Value = serde_json::from_str(payload).ok()?;
    let media_id = metadata
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let duration = json_number(metadata.get("duration"));
    let total_bitrate_kbps = json_number(metadata.get("tbr"));
    let bytes = json_number(metadata.get("filesize"))
        .or_else(|| json_number(metadata.get("filesize_approx")))
        .or_else(|| {
            let duration = duration?;
            let bitrate_kbps = total_bitrate_kbps?;
            Some(duration * bitrate_kbps * 1_000.0 / 8.0)
        })?;
    (bytes.is_finite() && bytes > 0.0 && bytes <= u64::MAX as f64).then_some(
        SelectedMediaEstimate {
            media_id,
            source_size: bytes.round() as u64,
            duration,
            height: json_number(metadata.get("height")).map(|value| value.round() as u64),
            fps: json_number(metadata.get("fps")),
            total_bitrate_kbps,
        },
    )
}

fn target_video_bitrate(height: Option<u64>, fps: Option<f64>, fallback_kbps: Option<f64>) -> u64 {
    let base = match height {
        Some(value) if value >= 2160 => 45_000_000,
        Some(value) if value >= 1440 => 24_000_000,
        Some(value) if value >= 1080 => 14_000_000,
        Some(value) if value >= 720 => 7_500_000,
        Some(value) if value >= 480 => 4_000_000,
        Some(_) => 2_000_000,
        None => {
            (fallback_kbps.unwrap_or(4_000.0) * 2_500.0).clamp(2_000_000.0, 45_000_000.0) as u64
        }
    };
    if fps.unwrap_or(30.0) > 45.0 {
        base.saturating_mul(3) / 2
    } else {
        base
    }
}

fn estimated_final_size(estimate: &SelectedMediaEstimate, tracker: &StorageTracker) -> u64 {
    let Some(duration) = estimate
        .duration
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return estimate.source_size;
    };
    let bitrate = if tracker.video {
        let video_maxrate =
            target_video_bitrate(estimate.height, estimate.fps, estimate.total_bitrate_kbps)
                .saturating_mul(11)
                / 10;
        video_maxrate.saturating_add(192_000)
    } else {
        match tracker.audio_format.as_str() {
            "wav" => 1_536_000,
            "opus" => 160_000,
            _ => 192_000,
        }
    };
    let bytes = duration * bitrate as f64 / 8.0 * 1.02;
    if bytes.is_finite() && bytes > 0.0 && bytes <= u64::MAX as f64 {
        bytes.ceil() as u64
    } else {
        estimate.source_size
    }
}

#[cfg(unix)]
fn available_disk_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = if stats.f_frsize > 0 {
        stats.f_frsize as u64
    } else {
        stats.f_bsize as u64
    };
    (stats.f_bavail as u64).checked_mul(block_size)
}

#[cfg(not(unix))]
fn available_disk_space(_path: &Path) -> Option<u64> {
    None
}

#[tauri::command]
fn folder_free_space(path: String) -> Result<u64, String> {
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err("Обрана папка недоступна".into());
    }
    available_disk_space(&path)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".into())
}

fn emit_storage_estimate(
    app: &AppHandle,
    id: &str,
    tracker: &StorageTracker,
    estimate: SelectedMediaEstimate,
) {
    let source_size = estimate.source_size;
    let final_size = estimated_final_size(&estimate, tracker);
    let (estimated_size, required_space) = tracker
        .estimates
        .lock()
        .ok()
        .map(|mut estimates| {
            estimates.insert(estimate.media_id, (source_size, final_size));
            estimates.values().copied().fold(
                (0_u64, 0_u64),
                |(final_total, working_total), (source, final_output)| {
                    (
                        final_total.saturating_add(final_output),
                        working_total.saturating_add(source.saturating_add(final_output)),
                    )
                },
            )
        })
        .unwrap_or((final_size, source_size.saturating_add(final_size)));
    let available_space = available_disk_space(&tracker.output_dir);
    let _ = app.emit(
        "download-event",
        DownloadEvent {
            id: id.into(),
            kind: "storage_estimate".into(),
            percent: None,
            speed: None,
            eta: None,
            message: None,
            storage: Some(StorageEstimate {
                estimated_size,
                required_space,
                available_space,
                sufficient: available_space.map(|available| available >= required_space),
            }),
        },
    );
}

fn read_output<R: Read + Send + 'static>(
    app: AppHandle,
    manager: DownloadManager,
    id: String,
    reader: R,
    downloaded_files: Option<Arc<Mutex<Vec<PathBuf>>>>,
    storage_tracker: Option<StorageTracker>,
    last_error: Arc<Mutex<Option<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if let Some(payload) = line.strip_prefix("__YTDLP_SIZE__") {
                if let (Some(tracker), Some(estimate)) =
                    (storage_tracker.as_ref(), parse_size_estimate(payload))
                {
                    emit_storage_estimate(&app, &id, tracker, estimate);
                }
                continue;
            } else if let Some(path) = line.strip_prefix("__YTDLP_FILE__") {
                if let Some(files) = downloaded_files.as_ref() {
                    if let Ok(mut files) = files.lock() {
                        let path = PathBuf::from(path.trim());
                        if !files.contains(&path) {
                            files.push(path);
                        }
                    }
                }
                continue;
            }
            if looks_like_auth_error(&line) {
                if let Ok(mut jobs) = manager.auth_required.lock() {
                    jobs.insert(id.clone());
                }
            }
            let normalized = line.to_ascii_lowercase();
            if normalized.contains("error:")
                || normalized.contains("http error")
                || normalized.contains("unable to download")
            {
                if let Ok(mut error) = last_error.lock() {
                    *error = Some(line.trim().trim_start_matches("ERROR:").trim().to_string());
                }
            }
            emit_line(&app, &id, &line);
        }
    })
}

fn download_was_cancelled(manager: &DownloadManager, id: &str) -> bool {
    manager
        .cancelled
        .lock()
        .ok()
        .map(|jobs| jobs.contains(id))
        .unwrap_or(false)
}

fn ffprobe_path(ffmpeg_path: &Path) -> PathBuf {
    if ffmpeg_path.components().count() > 1 {
        ffmpeg_path.with_file_name(if cfg!(target_os = "windows") {
            "ffprobe.exe"
        } else {
            "ffprobe"
        })
    } else {
        PathBuf::from(if cfg!(target_os = "windows") {
            "ffprobe.exe"
        } else {
            "ffprobe"
        })
    }
}

fn parse_frame_rate(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator > 0.0).then_some(numerator / denominator)
}

fn media_info(ffmpeg_path: &Path, input: &Path) -> Result<MediaInfo, String> {
    let output = Command::new(ffprobe_path(ffmpeg_path))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "format=duration:stream=height,avg_frame_rate",
            "-of",
            "json",
        ])
        .arg(input)
        .output()
        .map_err(|error| format!("Не вдалося запустити ffprobe: {error}"))?;
    if !output.status.success() {
        return Err("ffprobe не зміг визначити тривалість відео".into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "ffprobe повернув некоректні дані про відео".to_string())?;
    let duration = json_number(metadata.pointer("/format/duration"))
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "ffprobe повернув некоректну тривалість відео".to_string())?;
    let stream = metadata
        .get("streams")
        .and_then(|value| value.as_array())
        .and_then(|streams| streams.first());
    let height = stream
        .and_then(|stream| json_number(stream.get("height")))
        .map(|value| value.round() as u64)
        .unwrap_or(1080);
    let fps = stream
        .and_then(|stream| stream.get("avg_frame_rate"))
        .and_then(|value| value.as_str())
        .and_then(parse_frame_rate)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(30.0);
    Ok(MediaInfo {
        duration,
        height,
        fps,
    })
}

fn available_output_path(input: &Path) -> PathBuf {
    let preferred = input.with_extension("mp4");
    if !preferred.exists() {
        return preferred;
    }
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("video");
    for index in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({index}).mp4"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-{}.mp4", Uuid::new_v4()))
}

fn format_eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "—".into();
    }
    let total = seconds.round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn convert_to_compatible_mp4(
    app: &AppHandle,
    manager: &DownloadManager,
    id: &str,
    ffmpeg_path: &Path,
    input: &Path,
    file_index: usize,
    file_count: usize,
) -> Result<(), String> {
    if download_was_cancelled(manager, id) {
        return Err("Завантаження скасовано".into());
    }

    let media = media_info(ffmpeg_path, input)?;
    let duration = media.duration;
    let bitrate = target_video_bitrate(Some(media.height), Some(media.fps), None);
    let maxrate = bitrate.saturating_mul(11) / 10;
    let bufsize = bitrate.saturating_mul(2);
    let bitrate = bitrate.to_string();
    let maxrate = maxrate.to_string();
    let bufsize = bufsize.to_string();
    let output = available_output_path(input);
    let temporary = output.with_file_name(format!(
        ".{}.{}.part.mp4",
        output
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("video"),
        Uuid::new_v4()
    ));
    let mut command = Command::new(ffmpeg_path);
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
            "-i",
        ])
        .arg(input)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c:v",
            "h264_videotoolbox",
            "-b:v",
            &bitrate,
            "-maxrate",
            &maxrate,
            "-bufsize",
            &bufsize,
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-tag:v",
            "avc1",
            "-movflags",
            "+faststart",
        ])
        .arg(&temporary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Не вдалося запустити ffmpeg: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id.to_string(), child.clone());

    let errors = Arc::new(Mutex::new(String::new()));
    let error_reader = stderr.map(|stderr| {
        let errors = errors.clone();
        thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            if let Ok(mut target) = errors.lock() {
                *target = text;
            }
        })
    });

    let mut out_time_us = 0.0;
    let mut speed_value = 0.0;
    let mut speed_label = "VideoToolbox".to_string();
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = line.strip_prefix("out_time_us=") {
                out_time_us = value.trim().parse::<f64>().unwrap_or(out_time_us);
            } else if let Some(value) = line.strip_prefix("speed=") {
                let value = value.trim();
                speed_label = if value.is_empty() || value == "N/A" {
                    "VideoToolbox".into()
                } else {
                    format!("Кодування {value}")
                };
                speed_value = value
                    .trim_end_matches('x')
                    .parse::<f64>()
                    .unwrap_or(speed_value);
            } else if line.starts_with("progress=") {
                let elapsed = out_time_us / 1_000_000.0;
                let file_percent = (elapsed / duration * 100.0).clamp(0.0, 100.0);
                let overall_percent = ((file_index as f64 + file_percent / 100.0)
                    / file_count.max(1) as f64
                    * 100.0) as f32;
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
                                "Створюємо сумісний MP4 — файл {} із {}… Не закривайте застосунок",
                                file_index + 1,
                                file_count
                            )
                        } else {
                            "Створюємо сумісний MP4… Не закривайте застосунок".into()
                        }),
                        storage: None,
                    },
                );
            }
        }
    }

    let status = loop {
        if let Some(status) = child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok())
            .flatten()
        {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if let Some(reader) = error_reader {
        let _ = reader.join();
    }

    if download_was_cancelled(manager, id) {
        let _ = fs::remove_file(&temporary);
        return Err("Завантаження скасовано".into());
    }
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        let detail = errors
            .lock()
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "ffmpeg завершився з помилкою".into());
        return Err(format!("Не вдалося створити сумісний MP4: {detail}"));
    }

    fs::rename(&temporary, &output)
        .map_err(|error| format!("Не вдалося зберегти готовий MP4: {error}"))?;
    let _ = fs::remove_file(input);
    Ok(())
}

#[tauri::command]
fn start_download(
    app: AppHandle,
    manager: State<'_, DownloadManager>,
    request: DownloadRequest,
) -> Result<String, String> {
    let parsed = Url::parse(&request.url).map_err(|_| "Вставте повне посилання".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Підтримуються лише HTTP та HTTPS посилання".into());
    }
    let is_youtube = parsed
        .host_str()
        .map(|host| {
            host_matches(&host.to_ascii_lowercase(), "youtube.com")
                || host_matches(&host.to_ascii_lowercase(), "youtu.be")
        })
        .unwrap_or(false);
    let output_dir = PathBuf::from(&request.output_dir);
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Не вдалося відкрити папку завантажень: {error}"))?;

    let yt_dlp = yt_dlp_path(&app)?;
    if !yt_dlp.is_file() {
        return Err("Спочатку встановіть yt-dlp".into());
    }
    let runtime = runtime_dir(&app)?;
    let mut command = Command::new(yt_dlp);
    command.args([
        "--ignore-config",
        "--no-simulate",
        "--newline",
        "--no-colors",
        "--progress",
        "--print",
        "video:__YTDLP_SIZE__%(.{id,filesize,filesize_approx,duration,tbr,height,fps})j",
        "--progress-delta",
        "0.25",
        "--progress-template",
        "__YTDLP_PROGRESS__%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(info.title)s",
        "--progress-template",
        "postprocess:__YTDLP_POSTPROCESS__%(progress.status)s|%(progress.postprocessor)s|%(info.title)s",
        "-P",
        &request.output_dir,
    ]);
    if request.multi_item {
        command.args([
            "--sleep-requests",
            "0.5",
            "--sleep-subtitles",
            "2",
            "--sleep-interval",
            "3",
            "--max-sleep-interval",
            "7",
        ]);
    }
    let ffmpeg = find_ffmpeg(&app)?;
    if let Some(path) = ffmpeg
        .path
        .as_ref()
        .filter(|path| path.as_str() != "ffmpeg")
    {
        let ffmpeg_directory = Path::new(&path).parent().unwrap_or(&runtime);
        command.args([
            "--ffmpeg-location",
            ffmpeg_directory.to_string_lossy().as_ref(),
        ]);
    }

    if request.mode == "audio" {
        command.args(["-x", "--audio-format", &request.audio_format]);
    } else {
        let selector = match request.quality.as_str() {
            "2160" => "bestvideo[height<=2160]+bestaudio/best[height<=2160]",
            "1080" => "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
            "720" => "bestvideo[height<=720]+bestaudio/best[height<=720]",
            "480" => "bestvideo[height<=480]+bestaudio/best[height<=480]",
            _ => "bestvideo+bestaudio/best",
        };
        command.args([
            "-f",
            selector,
            "--merge-output-format",
            "mkv",
            "--remux-video",
            "mkv",
            "--print",
            "after_move:__YTDLP_FILE__%(filepath)s",
        ]);
    }
    if request.subtitles {
        command.args(["--write-subs", "--write-auto-subs", "--sub-langs", "uk,en"]);
    }
    if let Some(browser) = request.cookies_browser.as_deref() {
        let supported = [
            "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi",
        ];
        if !supported.contains(&browser) {
            return Err("Непідтримуваний браузер для cookies".into());
        }
        command.args(["--cookies-from-browser", browser]);
    }
    let deno = find_deno(&app)?;
    if let Some(path) = deno.path {
        if path == "deno" {
            command.args(["--js-runtimes", "deno"]);
        } else {
            command.args(["--js-runtimes", &format!("deno:{path}")]);
        }
    }
    command
        .arg(&request.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    command.process_group(0);

    let retry_program: OsString = command.get_program().to_os_string();
    let retry_args: Vec<OsString> = command.get_args().map(OsString::from).collect();
    let mut child = command
        .spawn()
        .map_err(|error| format!("Не вдалося запустити yt-dlp: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let id = Uuid::new_v4().to_string();
    let child = Arc::new(Mutex::new(child));
    let downloaded_files = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let storage_tracker = StorageTracker {
        output_dir,
        estimates: Arc::new(Mutex::new(HashMap::new())),
        video: request.mode != "audio",
        audio_format: request.audio_format.clone(),
    };
    let last_error = Arc::new(Mutex::new(None::<String>));
    manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id.clone(), child.clone());

    let stdout_reader = stdout.map(|stdout| {
        read_output(
            app.clone(),
            manager.inner().clone(),
            id.clone(),
            stdout,
            Some(downloaded_files.clone()),
            Some(storage_tracker.clone()),
            last_error.clone(),
        )
    });
    let stderr_reader = stderr.map(|stderr| {
        read_output(
            app.clone(),
            manager.inner().clone(),
            id.clone(),
            stderr,
            Some(downloaded_files.clone()),
            Some(storage_tracker.clone()),
            last_error.clone(),
        )
    });

    let manager = manager.inner().clone();
    let monitor_app = app.clone();
    let monitor_id = id.clone();
    let convert_video = request.mode != "audio";
    let ffmpeg_path = ffmpeg
        .path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    thread::spawn(move || {
        let mut active_child = child;
        let mut stdout_reader = stdout_reader;
        let mut stderr_reader = stderr_reader;
        let mut retry_attempt = 0_u8;
        loop {
            let result = active_child
                .lock()
                .ok()
                .and_then(|mut child| child.try_wait().ok())
                .flatten();
            if let Some(status) = result {
                if let Some(reader) = stdout_reader.take() {
                    let _ = reader.join();
                }
                if let Some(reader) = stderr_reader.take() {
                    let _ = reader.join();
                }
                let mut cancelled = download_was_cancelled(&manager, &monitor_id);
                let failure_message = last_error.lock().ok().and_then(|error| error.clone());
                let youtube_403 = is_youtube
                    && !status.success()
                    && failure_message
                        .as_deref()
                        .map(|message| message.contains("403"))
                        .unwrap_or(false);
                if youtube_403 && !cancelled && retry_attempt < 2 {
                    retry_attempt += 1;
                    let retry_message = if retry_attempt == 1 {
                        "YouTube перервав потік. Оновлюємо посилання й продовжуємо…"
                    } else {
                        "Перемикаємо спосіб отримання відео й продовжуємо…"
                    };
                    let _ = monitor_app.emit(
                        "download-event",
                        DownloadEvent {
                            id: monitor_id.clone(),
                            kind: "retrying".into(),
                            percent: None,
                            speed: None,
                            eta: None,
                            message: Some(retry_message.into()),
                            storage: None,
                        },
                    );
                    if let Ok(mut error) = last_error.lock() {
                        *error = None;
                    }
                    let mut retry_command = Command::new(&retry_program);
                    retry_command
                        .args(&retry_args)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    if retry_attempt == 2 {
                        retry_command.args([
                            "--extractor-args",
                            "youtube:player_client=web_embedded,android_vr",
                        ]);
                    }
                    #[cfg(unix)]
                    retry_command.process_group(0);

                    match retry_command.spawn() {
                        Ok(mut retry_child) => {
                            let retry_stdout = retry_child.stdout.take();
                            let retry_stderr = retry_child.stderr.take();
                            active_child = Arc::new(Mutex::new(retry_child));
                            manager.jobs.lock().ok().map(|mut jobs| {
                                jobs.insert(monitor_id.clone(), active_child.clone())
                            });
                            stdout_reader = retry_stdout.map(|stdout| {
                                read_output(
                                    monitor_app.clone(),
                                    manager.clone(),
                                    monitor_id.clone(),
                                    stdout,
                                    Some(downloaded_files.clone()),
                                    Some(storage_tracker.clone()),
                                    last_error.clone(),
                                )
                            });
                            stderr_reader = retry_stderr.map(|stderr| {
                                read_output(
                                    monitor_app.clone(),
                                    manager.clone(),
                                    monitor_id.clone(),
                                    stderr,
                                    Some(downloaded_files.clone()),
                                    Some(storage_tracker.clone()),
                                    last_error.clone(),
                                )
                            });
                            continue;
                        }
                        Err(error) => {
                            if let Ok(mut last_error) = last_error.lock() {
                                *last_error =
                                    Some(format!("Не вдалося повторно запустити yt-dlp: {error}"));
                            }
                        }
                    }
                }
                let auth_required = manager
                    .auth_required
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id))
                    .unwrap_or(false);
                let mut conversion_error = None;
                if status.success() && !cancelled && convert_video {
                    let files = downloaded_files
                        .lock()
                        .ok()
                        .map(|files| files.clone())
                        .unwrap_or_default();
                    if files.is_empty() {
                        conversion_error = Some(
                            "yt-dlp завершився, але не повідомив шлях до завантаженого відео"
                                .into(),
                        );
                    } else {
                        for (index, file) in files.iter().enumerate() {
                            if let Err(error) = convert_to_compatible_mp4(
                                &monitor_app,
                                &manager,
                                &monitor_id,
                                &ffmpeg_path,
                                file,
                                index,
                                files.len(),
                            ) {
                                conversion_error = Some(error);
                                break;
                            }
                        }
                    }
                    cancelled = download_was_cancelled(&manager, &monitor_id);
                }
                manager
                    .jobs
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id));
                manager
                    .cancelled
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id));
                let kind = if cancelled {
                    "cancelled"
                } else if auth_required && !status.success() {
                    "auth_required"
                } else if status.success() && conversion_error.is_none() {
                    "completed"
                } else {
                    "failed"
                };
                let _ = monitor_app.emit(
                    "download-event",
                    DownloadEvent {
                        id: monitor_id,
                        kind: kind.into(),
                        percent: (status.success() && conversion_error.is_none()).then_some(100.0),
                        speed: None,
                        eta: None,
                        message: conversion_error.or_else(|| {
                            (!status.success())
                                .then(|| last_error.lock().ok().and_then(|error| error.clone()))
                                .flatten()
                                .map(|error| friendly_download_error(&error))
                        }),
                        storage: None,
                    },
                );
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    });

    Ok(id)
}

#[tauri::command]
fn cancel_download(manager: State<'_, DownloadManager>, id: String) -> Result<(), String> {
    let child = manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .get(&id)
        .cloned()
        .ok_or_else(|| "Активне завантаження не знайдено".to_string())?;
    manager
        .cancelled
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id);
    #[cfg(unix)]
    {
        let process_group = child
            .lock()
            .map_err(|_| "Не вдалося зупинити процес".to_string())?
            .id() as i32;
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(format!("Не вдалося зупинити завантаження: {error}"))
    }

    #[cfg(not(unix))]
    child
        .lock()
        .map_err(|_| "Не вдалося зупинити процес".to_string())?
        .kill()
        .map_err(|error| format!("Не вдалося зупинити завантаження: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DownloadManager::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            install_ytdlp,
            install_ffmpeg,
            install_deno,
            update_ytdlp,
            probe_url,
            check_vpn,
            folder_free_space,
            start_download,
            cancel_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
