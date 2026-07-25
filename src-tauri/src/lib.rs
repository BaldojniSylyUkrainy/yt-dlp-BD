use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRequest {
    url: String,
    output_dir: String,
    mode: String,
    quality: String,
    audio_format: String,
    subtitles: bool,
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
async fn install_ffmpeg(app: AppHandle) -> Result<RuntimeStatus, String> {
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    return Err("Portable ffmpeg поки налаштовано лише для macOS Apple Silicon".into());

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        const RELEASES_PAGE: &str = "https://ffmpeg.martin-riedl.de/";
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| format!("Не вдалося підготувати завантаження: {error}"))?;
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
            return runtime_status(app);
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
        runtime_status(app)
    }
}

#[tauri::command]
async fn install_deno(app: AppHandle) -> Result<RuntimeStatus, String> {
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
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
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
        return runtime_status(app);
    }

    let archive = fetch_bytes(&client, &archive_url).await?;
    verify_sha256(&archive, &checksum, "Deno")?;
    extract_binary(&archive, filename, &destination)?;
    fs::write(stamp, expected)
        .map_err(|error| format!("Не вдалося зберегти версію Deno: {error}"))?;
    runtime_status(app)
}

#[tauri::command]
async fn install_ytdlp(app: AppHandle) -> Result<RuntimeStatus, String> {
    #[cfg(target_os = "macos")]
    let asset_name = "yt-dlp_macos";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let asset_name = "yt-dlp.exe";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    let asset_name = "yt-dlp_arm64.exe";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("Ця платформа поки що не підтримується".into());

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| format!("Не вдалося підготувати завантаження: {error}"))?;
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
        return runtime_status(app);
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

    runtime_status(app)
}

#[tauri::command]
async fn update_ytdlp(app: AppHandle) -> Result<RuntimeStatus, String> {
    let result = install_ytdlp(app).await;
    if let Err(error) = &result {
        eprintln!("Не вдалося оновити yt-dlp: {error}");
    }
    result
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
            },
        );
    } else if !line.trim().is_empty() {
        let _ = app.emit(
            "download-event",
            DownloadEvent {
                id: id.into(),
                kind: "log".into(),
                percent: None,
                speed: None,
                eta: None,
                message: Some(line.trim().to_string()),
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

fn read_output<R: Read + Send + 'static>(
    app: AppHandle,
    manager: DownloadManager,
    id: String,
    reader: R,
) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if looks_like_auth_error(&line) {
                if let Ok(mut jobs) = manager.auth_required.lock() {
                    jobs.insert(id.clone());
                }
            }
            emit_line(&app, &id, &line);
        }
    });
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
        "--newline",
        "--no-colors",
        "--progress-delta",
        "0.25",
        "--progress-template",
        "__YTDLP_PROGRESS__%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(info.title)s",
        "-P",
        &request.output_dir,
    ]);
    let ffmpeg = find_ffmpeg(&app)?;
    if let Some(path) = ffmpeg.path.filter(|path| path != "ffmpeg") {
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
            "2160" => "bv*[height<=2160]+ba/b[height<=2160]",
            "1080" => "bv*[height<=1080]+ba/b[height<=1080]",
            "720" => "bv*[height<=720]+ba/b[height<=720]",
            "480" => "bv*[height<=480]+ba/b[height<=480]",
            _ => "bv*+ba/b",
        };
        command.args(["-f", selector]);
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

    let mut child = command
        .spawn()
        .map_err(|error| format!("Не вдалося запустити yt-dlp: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let id = Uuid::new_v4().to_string();
    let child = Arc::new(Mutex::new(child));
    manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id.clone(), child.clone());

    if let Some(stdout) = stdout {
        read_output(app.clone(), manager.inner().clone(), id.clone(), stdout);
    }
    if let Some(stderr) = stderr {
        read_output(app.clone(), manager.inner().clone(), id.clone(), stderr);
    }

    let manager = manager.inner().clone();
    let monitor_app = app.clone();
    let monitor_id = id.clone();
    thread::spawn(move || loop {
        let result = child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok())
            .flatten();
        if let Some(status) = result {
            manager
                .jobs
                .lock()
                .ok()
                .map(|mut jobs| jobs.remove(&monitor_id));
            let cancelled = manager
                .cancelled
                .lock()
                .ok()
                .map(|mut cancelled| cancelled.remove(&monitor_id))
                .unwrap_or(false);
            let auth_required = manager
                .auth_required
                .lock()
                .ok()
                .map(|mut jobs| jobs.remove(&monitor_id))
                .unwrap_or(false);
            let kind = if cancelled {
                "cancelled"
            } else if auth_required && !status.success() {
                "auth_required"
            } else if status.success() {
                "completed"
            } else {
                "failed"
            };
            let _ = monitor_app.emit(
                "download-event",
                DownloadEvent {
                    id: monitor_id,
                    kind: kind.into(),
                    percent: status.success().then_some(100.0),
                    speed: None,
                    eta: None,
                    message: None,
                },
            );
            break;
        }
        thread::sleep(Duration::from_millis(200));
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
    let result = child
        .lock()
        .map_err(|_| "Не вдалося зупинити процес".to_string())?
        .kill()
        .map_err(|error| format!("Не вдалося зупинити завантаження: {error}"));
    result
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
            check_vpn,
            start_download,
            cancel_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
