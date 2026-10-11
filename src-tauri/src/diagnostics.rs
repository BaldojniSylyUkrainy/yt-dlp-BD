use super::*;
use std::collections::VecDeque;

type Logs = VecDeque<(String, VecDeque<String>)>;
fn logs() -> &'static Mutex<Logs> {
    static LOGS: OnceLock<Mutex<Logs>> = OnceLock::new();
    LOGS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(crate) fn redact(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if [
        "authorization:",
        "cookie:",
        "set-cookie:",
        "bearer ",
        "password=",
        "token=",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return "[Повідомлення з даними авторизації приховано]".into();
    }
    // A path may contain unquoted spaces. Conservatively hide the whole line:
    // token redaction would leak the rest of the person's name and filename.
    if line.contains("/Users/")
        || line.contains("/home/")
        || line.contains("/Volumes/")
        || line.contains("/private/")
        || line.contains("/var/folders/")
        || line.contains("/tmp/")
        || line.contains(":\\")
        || line.contains("\\\\")
        || line.as_bytes().windows(4).any(|part| {
            part[0].is_ascii_alphabetic() && part[1] == b':' && part[2] == b'/' && part[3] != b'/'
        })
    {
        return "[Повідомлення з локальним шляхом приховано]".into();
    }
    line.split_whitespace()
        .map(|word| {
            if word.contains("://") {
                return "[URL приховано]".to_string();
            }
            if word.contains("/Users/")
                || word.contains("/home/")
                || word.contains(":\\")
                || word.starts_with("\\\\")
            {
                return "[Локальний шлях приховано]".into();
            }
            word.chars().take(512).collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1500)
        .collect()
}

pub(crate) fn record(id: &str, line: &str) {
    let line = redact(line);
    let mut logs = logs().lock().unwrap_or_else(|e| e.into_inner());
    if !logs.iter().any(|(key, _)| key == id) {
        if logs.len() >= 20 {
            logs.pop_front();
        }
        logs.push_back((id.into(), VecDeque::new()));
    }
    if let Some((_, entries)) = logs.iter_mut().find(|(key, _)| key == id) {
        if entries.len() >= 40 {
            entries.pop_front();
        }
        entries.push_back(line);
    }
}

#[tauri::command]
pub(crate) async fn download_diagnostics(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || report(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

fn report(app: &AppHandle, id: &str) -> Result<String, String> {
    let directory = runtime_dir(app)?;
    let ffmpeg = find_ffmpeg(app)?
        .version
        .unwrap_or_else(|| "невідомо".into());
    let deno = find_deno(app)?.version.unwrap_or_else(|| "невідомо".into());
    let version =
        fs::read_to_string(directory.join(".yt-dlp.version")).unwrap_or_else(|_| "невідомо".into());
    let logs = logs().lock().map_err(|e| e.to_string())?;
    let entries = logs
        .iter()
        .find(|(key, _)| key == id)
        .map(|(_, lines)| lines.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_else(|| "Діагностика цієї операції вже недоступна".into());
    Ok(format!(
        "Baldojnyi Downloader {}\nOS: {} / {}\nyt-dlp: {}\nFFmpeg: {}\nDeno: {}\n\n{}",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        version.trim(),
        redact(&ffmpeg),
        redact(&deno),
        entries
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_credentials_and_paths() {
        assert!(
            !redact("/var/folders/ab/private-test/video.mp4: invalid data")
                .contains("private-test")
        );
        assert!(
            !redact("ERROR https://x.test/v?signature=SECRET /Users/name/private")
                .contains("SECRET")
        );
        assert!(!redact("Cookie: session=SECRET").contains("SECRET"));
        assert!(!redact("C:\\Users\\Name\\file.mp4").contains("Name"));
        for line in [
            r"Destination: C:\Users\Олег Петренко\Downloads\приватне відео.mp4",
            "/Users/Олег Петренко/приватне відео.mp4",
            r"\\server\private folder\video.mp4",
        ] {
            assert!(!redact(line).contains("Петренко"));
            assert!(!redact(line).contains("video.mp4"));
            assert!(!redact(line).contains("відео"));
        }
    }
}
