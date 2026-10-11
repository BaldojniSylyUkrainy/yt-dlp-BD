//! Untrusted pages have no Tauri capability. They can only suggest bounded URLs;
//! a user selection in the trusted main window is required to download one.
use super::*;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Candidates {
    pub session_id: String,
    pub job_id: String,
    pub page_url: String,
    pub urls: Vec<String>,
}
#[derive(Default)]
pub(crate) struct DiscoveryState {
    candidates: Mutex<HashMap<String, Search>>,
    selected: Mutex<HashMap<String, String>>,
}

struct Search {
    job_id: String,
    page: String,
    urls: Vec<String>,
    created: Instant,
}
const SEARCH_TTL: Duration = Duration::from_secs(15 * 60);

#[tauri::command]
pub(crate) fn dismiss_media_candidates(app: AppHandle, session_id: String) {
    let state = app.state::<DiscoveryState>();
    state
        .candidates
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session_id);
    state
        .selected
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session_id);
}

fn begin(app: &AppHandle, job_id: &str, page: &str) -> String {
    let state = app.state::<DiscoveryState>();
    let mut map = state.candidates.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, search| search.job_id != job_id && search.created.elapsed() < SEARCH_TTL);
    if map.len() >= 50 {
        map.clear();
    }
    state
        .selected
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|id, _| map.contains_key(id));
    let id = Uuid::new_v4().to_string();
    map.insert(
        id.clone(),
        Search {
            job_id: job_id.into(),
            page: page.into(),
            urls: Vec::new(),
            created: Instant::now(),
        },
    );
    id
}

pub(crate) fn safe_url(value: &str) -> Option<Url> {
    if value.len() > 8192 {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !host.contains('.')
    {
        return None;
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if !is_public_thumbnail_ip(ip) {
            return None;
        }
    }
    Some(url)
}

pub(crate) fn publish(app: &AppHandle, job_id: &str, page_url: &str, urls: Vec<String>) {
    let Some(page) = safe_url(page_url).and_then(|page| normalize_media_url(page.as_str()).ok())
    else {
        return;
    };
    let state = app.state::<DiscoveryState>();
    let existing = state
        .candidates
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(_, s)| {
            s.job_id == job_id && s.page == page.as_str() && s.created.elapsed() < SEARCH_TTL
        })
        .map(|(id, _)| id.clone());
    let session = existing.unwrap_or_else(|| begin(app, job_id, page.as_str()));
    publish_session(app, &session, urls);
}

fn publish_session(app: &AppHandle, session: &str, urls: Vec<String>) {
    let state = app.state::<DiscoveryState>();
    let mut map = state.candidates.lock().unwrap_or_else(|e| e.into_inner());
    let Some(search) = map
        .get_mut(session)
        .filter(|s| s.created.elapsed() < SEARCH_TTL)
    else {
        return;
    };
    let entries = &mut search.urls;
    for url in urls.into_iter().take(30) {
        if let Some(url) = safe_url(&url) {
            if entries.len() < 30 && !entries.contains(&url.to_string()) {
                entries.push(url.into());
            }
        }
    }
    if !entries.is_empty() {
        if let Some(window) = app.get_webview_window("media-discovery") {
            // Only a hint: the untrusted page never authorizes a download.
            let _ = window.eval("window.dispatchEvent(new Event('baldoj-source-found'))");
        }
        let _ = app.emit_to(
            "main",
            "media-candidates",
            Candidates {
                session_id: session.into(),
                job_id: search.job_id.clone(),
                page_url: search.page.clone(),
                urls: entries.clone(),
            },
        );
    }
}

pub(crate) fn take_selection(
    app: &AppHandle,
    session: Option<&str>,
    page: &str,
) -> Result<Option<String>, String> {
    let Some(session) = session else {
        return Ok(None);
    };
    let state = app.state::<DiscoveryState>();
    let mut searches = state.candidates.lock().map_err(|e| e.to_string())?;
    if !searches
        .get(session)
        .is_some_and(|s| s.page == page && s.created.elapsed() < SEARCH_TTL)
    {
        return Err("Джерело застаріло. Повторіть пошук у браузері".into());
    }
    let selected = state
        .selected
        .lock()
        .map_err(|e| e.to_string())?
        .remove(session)
        .ok_or("Повторно оберіть джерело відео")?;
    searches.remove(session);
    Ok(Some(selected))
}

pub(crate) fn should_retry(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    ![
        "403",
        "429",
        "drm",
        "login",
        "private",
        "no space",
        "permission",
        "cookie",
        "unavailable",
        "removed",
    ]
    .iter()
    .any(|s| error.contains(s))
        && [
            "unsupported url",
            "no suitable extractor",
            "unable to extract",
            "no video formats",
            "no video found",
        ]
        .iter()
        .any(|s| error.contains(s))
}

#[tauri::command]
pub(crate) async fn select_media_candidate(
    app: AppHandle,
    session_id: String,
    index: usize,
) -> Result<(), String> {
    let state = app.state::<DiscoveryState>();
    let url = state
        .candidates
        .lock()
        .map_err(|e| e.to_string())?
        .get(&session_id)
        .filter(|s| s.created.elapsed() < SEARCH_TTL)
        .and_then(|s| s.urls.get(index))
        .cloned()
        .ok_or("Джерело вже недоступне")?;
    let parsed = safe_url(&url).ok_or("Небезпечна адреса джерела")?;
    // Resolve and reject private/special addresses before handing off to yt-dlp.
    let _ = public_thumbnail_client(&parsed).await?;
    state
        .selected
        .lock()
        .map_err(|e| e.to_string())?
        .insert(session_id, url);
    if let Some(window) = app.get_webview_window("media-discovery") {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn open_media_browser(
    app: AppHandle,
    page_url: String,
    job_id: String,
) -> Result<(), String> {
    crate::browser_security::preflight()?;
    Uuid::parse_str(&job_id).map_err(|_| "Некоректна спроба завантаження")?;
    let page = safe_url(&page_url)
        .and_then(|p| normalize_media_url(p.as_str()).ok())
        .ok_or("Потрібне публічне HTTP(S)-посилання")?;
    let _ = public_thumbnail_client(&page).await?;
    if let Some(window) = app.get_webview_window("media-discovery") {
        window.close().map_err(|e| e.to_string())?;
    }
    let captured_app = app.clone();
    let session = begin(&app, &job_id, page.as_str());
    let lifetime_session = session.clone();
    let proxy = Arc::new(
        crate::browser_proxy::BrowserProxy::start()
            .await
            .map_err(|_| crate::browser_security::POLICY_ERROR)?,
    );
    let ready = Arc::new(AtomicBool::new(false));
    let navigation_ready = ready.clone();
    // Never load a remote byte before native policy configuration completes.
    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        "media-discovery",
        tauri::WebviewUrl::External(Url::parse("about:blank").unwrap()),
    )
    .visible(false)
    .title("Пошук відео — запустіть потрібний плеєр, потім оберіть джерело в застосунку")
    .inner_size(1000.0, 720.0)
    .theme(Some(tauri::Theme::Dark))
    .background_color(tauri::window::Color(17, 17, 17, 255))
    .incognito(true)
    .on_page_load(|_, payload| {
        #[cfg(debug_assertions)]
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            crate::browser_smoke::progress("page-loaded");
        }
        #[cfg(not(debug_assertions))]
        let _ = payload;
    })
    .on_download(|_, _| false)
    .initialization_script_for_all_frames(include_str!("../resources/media_browser_guide.js"))
    .initialization_script_for_all_frames(include_str!("../resources/media_capture.js"))
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_navigation(move |url| {
        if url.as_str() == "about:blank" {
            return true;
        }
        if !navigation_ready.load(Ordering::Acquire) {
            return false;
        }
        if url.scheme() == "baldoj-media" {
            #[cfg(debug_assertions)]
            if crate::browser_smoke::capture(&captured_app, url) {
                return false;
            }
            if url.host_str() == Some("return") {
                if let Some(window) = captured_app.get_webview_window("media-discovery") {
                    let _ = window.close();
                }
                if let Some(main) = captured_app.get_webview_window("main") {
                    let _ = main.set_focus();
                }
                return false;
            }
            if url.as_str().len() < 300_000 {
                if let Some((_, payload)) = url.query_pairs().find(|(key, _)| key == "payload") {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) {
                        let urls = value["urls"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .take(30)
                                    .filter_map(|v| v.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        publish_session(&captured_app, &session, urls);
                    }
                }
            }
            return false; // No custom-scheme navigation ever leaves this callback.
        }
        safe_url(url.as_str()).is_some()
    });
    // A separate environment is mandatory on Windows: the main window must
    // neither inherit this proxy nor cause Edge to reuse an unprotected one.
    #[cfg(target_os = "windows")]
    let profile = tempfile::Builder::new()
        .prefix("baldoj-media-browser-")
        .tempdir()
        .map_err(|_| crate::browser_security::POLICY_ERROR)?;
    #[cfg(target_os = "windows")]
    let builder = builder
        .data_directory(profile.path().to_path_buf())
        .additional_browser_args(&crate::browser_security::windows_arguments(proxy.address));
    let window = builder.build().map_err(|e| e.to_string())?;
    let lifetime_proxy = proxy.clone();
    window.on_window_event(move |event| {
        #[cfg(target_os = "windows")]
        let _keep_profile_alive = &profile;
        if matches!(event, tauri::WindowEvent::Destroyed) {
            lifetime_proxy.stop();
        }
    });
    if let Err(error) = crate::browser_security::configure(&window, proxy.address).await {
        proxy.stop();
        let _ = window.destroy();
        return Err(error);
    }
    ready.store(true, Ordering::Release);
    #[cfg(debug_assertions)]
    crate::browser_smoke::progress("native-policy-configured");
    if let Err(error) = window.navigate(page).and_then(|_| window.show()) {
        proxy.stop();
        let _ = window.destroy();
        return Err(error.to_string());
    }
    let mut expired = proxy.expired.clone();
    tauri::async_runtime::spawn(async move {
        if expired.changed().await.is_ok() && *expired.borrow() {
            dismiss_media_candidates(app.clone(), lifetime_session.clone());
            let _ = app.emit_to(
                "main",
                "media-session-expired",
                serde_json::json!({
                    "sessionId": lifetime_session, "jobId": job_id,
                }),
            );
            let _ = window.set_title("Час пошуку минув — відкрийте пошук повторно");
            let _ = window.eval("window.dispatchEvent(new Event('baldoj-session-expired'))");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_does_not_retry_access_or_storage_failures() {
        assert!(should_retry("ERROR: Unsupported URL"));
        for error in [
            "403 unable to extract",
            "DRM no video formats",
            "no space",
            "login required",
            "cookies unable to extract",
        ] {
            assert!(!should_retry(error));
        }
    }
    #[test]
    fn rejects_local_sources_and_credentials() {
        for url in [
            "file:///etc/passwd",
            "http://127.0.0.1/v.mp4",
            "http://localhost/v",
            "https://u:p@example.com/v",
        ] {
            assert!(safe_url(url).is_none());
        }
        assert!(safe_url("https://cdn.example.com/v.mp4?signature=abc").is_some());
    }

    #[test]
    fn discovered_page_uses_the_same_identity_as_download_start() {
        let page = safe_url("https://www.tiktok.com/@sample/video/123?is_from_webapp=1").unwrap();
        assert_eq!(
            normalize_media_url(page.as_str()).unwrap().as_str(),
            "https://www.tiktok.com/@sample/video/123"
        );
    }
}
