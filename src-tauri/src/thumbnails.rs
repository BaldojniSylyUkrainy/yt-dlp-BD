//! Offline history thumbnails. Every request and redirect is pinned to a checked
//! public address, and only bounded raster images reach the cache.
use super::*;

pub(crate) const MAX_HISTORY_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) const HISTORY_THUMBNAIL_CACHE_LIMIT: usize = 550;

#[derive(Default)]
pub(crate) struct HistoryThumbnailCache {
    pub(crate) operation: tokio::sync::Mutex<()>,
}

pub(crate) fn is_public_thumbnail_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_thumbnail_ip(IpAddr::V4(mapped));
            }
            !(ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

pub(crate) async fn public_thumbnail_client(url: &Url) -> Result<reqwest::Client, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "Посилання на прев’ю не містить адреси сервера".to_string())?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| "Час перевірки адреси сервера вичерпано")?
    .map_err(|error| format!("Не вдалося знайти сервер: {error}"))?
    .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_public_thumbnail_ip(address.ip()))
    {
        return Err("Прев’ю вказує на локальну або службову мережеву адресу".into());
    }
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|error| format!("Не вдалося підготувати безпечне завантаження прев’ю: {error}"))
}

pub(crate) fn history_thumbnail_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("history-thumbnails");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не вдалося створити кеш прев’ю: {error}"))?;
    Ok(directory)
}

pub(crate) fn thumbnail_extension(content_type: &str) -> Option<&'static str> {
    match content_type
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

pub(crate) fn prune_history_thumbnail_cache(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    metadata
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect::<Vec<_>>();
    if files.len() <= HISTORY_THUMBNAIL_CACHE_LIMIT {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let remove_count = files.len() - HISTORY_THUMBNAIL_CACHE_LIMIT;
    for (_, path) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

#[tauri::command]
pub(crate) async fn cache_history_thumbnail(
    app: AppHandle,
    cache: State<'_, HistoryThumbnailCache>,
    url: String,
) -> Result<String, String> {
    let _operation = cache.operation.lock().await;
    let parsed = Url::parse(&url).map_err(|_| "Некоректне посилання на прев’ю".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Прев’ю має використовувати HTTP або HTTPS".into());
    }
    let directory = history_thumbnail_dir(&app)?;
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    for extension in ["jpg", "png", "webp", "gif", "avif"] {
        let cached = directory.join(format!("{digest}.{extension}"));
        if cached.is_file() {
            return Ok(cached.to_string_lossy().into_owned());
        }
    }

    let mut current = parsed;
    let mut redirect_count = 0_u8;
    let mut response = loop {
        let client = public_thumbnail_client(&current).await?;
        let response = client
            .get(current.clone())
            .header("User-Agent", "yt-dlp-desktop")
            .send()
            .await
            .map_err(|error| format!("Не вдалося завантажити прев’ю: {error}"))?;
        if response.status().is_redirection() {
            if redirect_count >= 10 {
                return Err("Сервер прев’ю виконав забагато перенаправлень".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Сервер прев’ю повернув некоректне перенаправлення".to_string())?;
            current = current
                .join(location)
                .map_err(|_| "Сервер прев’ю повернув некоректну нову адресу".to_string())?;
            if !matches!(current.scheme(), "http" | "https") || current.host_str().is_none() {
                return Err("Перенаправлення прев’ю використовує непідтримувану адресу".into());
            }
            redirect_count += 1;
            continue;
        }
        break response
            .error_for_status()
            .map_err(|error| format!("Сервер прев’ю повернув помилку: {error}"))?;
    };
    if response
        .content_length()
        .is_some_and(|size| size > MAX_HISTORY_THUMBNAIL_BYTES)
    {
        return Err("Прев’ю завелике для локального кешу".into());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let extension = thumbnail_extension(content_type)
        .ok_or_else(|| "Сервер повернув непідтримуваний формат прев’ю".to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Не вдалося прочитати прев’ю: {error}"))?
    {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > MAX_HISTORY_THUMBNAIL_BYTES {
            return Err("Прев’ю завелике для локального кешу".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("Сервер повернув порожнє прев’ю".into());
    }
    let destination = directory.join(format!("{digest}.{extension}"));
    let temporary = directory.join(format!(".{digest}.{}.part", Uuid::new_v4()));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|error| format!("Не вдалося записати кеш прев’ю: {error}"))?;
    if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!("Не вдалося завершити кешування прев’ю: {error}"));
    }
    prune_history_thumbnail_cache(&directory);
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
pub(crate) async fn clear_history_thumbnail_cache(
    app: AppHandle,
    cache: State<'_, HistoryThumbnailCache>,
) -> Result<(), String> {
    let _operation = cache.operation.lock().await;
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("history-thumbnails");
    if directory.exists() {
        fs::remove_dir_all(&directory)
            .map_err(|error| format!("Не вдалося очистити кеш прев’ю: {error}"))?;
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("Не вдалося відновити кеш прев’ю: {error}"))
}

#[tauri::command]
pub(crate) async fn delete_history_thumbnail(
    app: AppHandle,
    cache: State<'_, HistoryThumbnailCache>,
    path: String,
) -> Result<(), String> {
    let _operation = cache.operation.lock().await;
    let directory = history_thumbnail_dir(&app)?;
    let candidate = PathBuf::from(path);
    if candidate.parent() != Some(directory.as_path()) {
        return Err("Можна видаляти лише файли кешу прев’ю".into());
    }
    if candidate.exists() {
        fs::remove_file(candidate)
            .map_err(|error| format!("Не вдалося видалити прев’ю з кешу: {error}"))?;
    }
    Ok(())
}
