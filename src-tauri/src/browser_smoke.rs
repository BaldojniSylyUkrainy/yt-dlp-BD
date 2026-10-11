//! Debug-only native harness. Uses the production discovery window, no personal
//! profile or main-app frontend, and no production IPC/capability additions.
use super::*;

pub fn requested() -> bool {
    std::env::var_os("BALDOJ_BROWSER_SMOKE_URL").is_some()
}

pub fn progress(stage: &str) {
    if requested() {
        eprintln!("Browser smoke: {stage}");
        if let Some(path) = std::env::var_os("BALDOJ_BROWSER_SMOKE_REPORT") {
            let _ = fs::write(path, serde_json::json!({"stage": stage}).to_string());
        }
    }
}

fn finish(app: &AppHandle, result: serde_json::Value, code: i32) {
    if let Some(path) = std::env::var_os("BALDOJ_BROWSER_SMOKE_REPORT") {
        let _ = fs::write(path, result.to_string());
    }
    app.exit(code);
}

pub fn capture(app: &AppHandle, url: &Url) -> bool {
    if !requested() || url.host_str() != Some("smoke-finished") {
        return false;
    }
    let expected = std::env::var("BALDOJ_BROWSER_SMOKE_NONCE").unwrap_or_default();
    if !expected.is_empty()
        && url
            .query_pairs()
            .any(|(key, value)| key == "nonce" && value == expected)
    {
        finish(
            app,
            serde_json::json!({"completed": true, "nonce": expected}),
            0,
        );
    }
    true
}

pub fn run(mut context: tauri::Context<tauri::Wry>) {
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .manage(discovery::DiscoveryState::default())
        .build(context)
        .expect("native smoke app");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            let handle = handle.clone();
            progress("event-loop-ready");
            tauri::async_runtime::spawn(async move {
                let url = std::env::var("BALDOJ_BROWSER_SMOKE_URL").unwrap_or_default();
                let result =
                    discovery::open_media_browser(handle.clone(), url, Uuid::new_v4().to_string())
                        .await;
                if let Err(error) = result {
                    finish(&handle, serde_json::json!({"error": error}), 1);
                }
            });
        }
    });
}
