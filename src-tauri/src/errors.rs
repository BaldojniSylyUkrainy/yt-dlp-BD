//! Turn raw yt-dlp/FFmpeg diagnostics into user-facing text and stable codes.
use super::*;

/// Find an exact number or short word. Media IDs, sizes and speeds such as
/// `7429…` or `403.21KiB/s` routinely contain the same digits by coincidence.
fn has_token(value: &str, token: &str) -> bool {
    value.match_indices(token).any(|(start, matched)| {
        let before = value[..start].chars().next_back();
        let after = value[start + matched.len()..].chars().next();
        let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric());
        // `403.21KiB/s` continues as a decimal number, `403:` or `403.` does not.
        let decimal = after == Some('.')
            && value[start + matched.len() + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
        boundary(before) && boundary(after) && !decimal
    })
}

pub(crate) fn is_forbidden_error(message: &str) -> bool {
    let value = message.to_ascii_lowercase();
    has_token(&value, "403") || value.contains("forbidden")
}

fn is_rate_limited(value: &str) -> bool {
    has_token(value, "429")
        || ["too many requests", "rate limit", "rate-limit"]
            .iter()
            .any(|pattern| value.contains(pattern))
}

fn is_network_failure(value: &str) -> bool {
    [
        "timed out",
        "timeout",
        "connection",
        "network",
        "temporary failure",
        "name resolution",
        "unable to connect",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
}

pub(crate) fn friendly_download_error(message: &str) -> String {
    if message.contains(crate::DESTINATION_UNAVAILABLE_MESSAGE) {
        return crate::DESTINATION_UNAVAILABLE_MESSAGE.into();
    }
    if message.contains(INSTAGRAM_PHOTO_ONLY_MARKER) {
        return "У цьому пості лише фото. Щоб зберегти їх, виберіть режим «Відео».".into();
    }
    let value = message.to_ascii_lowercase();
    if value.contains("media sources found") {
        return "Знайдено джерела відео. Оберіть потрібне у вікні вибору.".into();
    }
    if value.contains("automatic discovery found no media") {
        return "Не вдалося знайти відео у сторінці. Спробуйте пошук у браузері.".into();
    }
    if value.contains("недостатньо вільного місця") || value.contains("no space left")
    {
        "Недостатньо вільного місця. Процес зупинено, щоб захистити диск.".into()
    } else if ["truncated_id", "incomplete youtube id", "looks truncated"]
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
        "failed to decrypt with dpapi",
        "failed to decrypt cookie",
        "app-bound encryption",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "Windows не зміг розшифрувати cookies цього браузера. Увійдіть на сайт у Firefox і повторіть через Firefox або виберіть інший доступний браузер."
            .into()
    } else if [
        "could not copy chrome cookie database",
        "could not copy edge cookie database",
        "database is locked",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "Браузер заблокував свою базу cookies. Повністю закрийте його, відкрийте знову, увійдіть на сайт і повторіть завантаження."
            .into()
    } else if value.contains("could not find") && value.contains("cookies database") {
        "Cookies обраного браузера не знайдено. Оберіть браузер, у якому ви вже увійшли на цей сайт."
            .into()
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
    } else if value.contains("cookies.binarycookies")
        || (value.contains("safari") && value.contains("operation not permitted"))
    {
        "macOS не дозволив застосунку прочитати cookies Safari. Використайте Chrome, Firefox, Edge або Brave."
            .into()
    } else if ["unsupported url", "no suitable extractor"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Цей сайт або тип посилання поки не підтримується.".into()
    } else if has_token(&value, "drm") {
        "Відео захищене DRM і не може бути завантажене.".into()
    } else if value.contains("requested format is not available") {
        "Для цього матеріалу немає потрібного формату. Спробуйте вибрати «Найкраща доступна»."
            .into()
    } else if [
        "geo-restrict",
        "geo restrict",
        "geo-block",
        "your country",
        "your region",
        "your location",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "Відео недоступне у вашому регіоні.".into()
    } else if is_rate_limited(&value) {
        "Сайт тимчасово обмежив запити. Зачекайте кілька хвилин і спробуйте ще раз.".into()
    } else if [
        "video unavailable",
        "video is unavailable",
        "not available",
        "removed",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "Відео недоступне або було видалене.".into()
    } else if has_token(&value, "404") || value.contains("not found") {
        "За цим посиланням нічого не знайдено. Перевірте, чи воно скопійоване повністю.".into()
    } else if is_forbidden_error(&value) {
        "Сайт відхилив доступ до медіафайла. Спробуйте ще раз або скористайтеся іншим підключенням."
            .into()
    } else if is_network_failure(&value) {
        "Не вдалося з’єднатися із сайтом. Перевірте інтернет і спробуйте ще раз.".into()
    } else {
        "Не вдалося прочитати це посилання. Перевірте його та спробуйте ще раз.".into()
    }
}

pub(crate) fn friendly_conversion_error(message: &str) -> String {
    if message.contains(crate::DESTINATION_UNAVAILABLE_MESSAGE) {
        return crate::DESTINATION_UNAVAILABLE_MESSAGE.into();
    }
    let value = message.to_ascii_lowercase();
    if value.contains("недостатньо вільного місця") || value.contains("no space left")
    {
        "Недостатньо вільного місця. Готові файли збережено, решту обробки зупинено.".into()
    } else if value.contains("скасовано") {
        "Обробку скасовано. Уже готові файли залишилися у вибраній папці.".into()
    } else if value.contains("invalid data found") || value.contains("moov atom not found") {
        "Завантажений медіафайл пошкоджений або має формат, який не вдалося прочитати. Уже готові файли збережено.".into()
    } else if value.contains("ffprobe") {
        "Не вдалося перевірити завантажений файл. Спробуйте повторити; технічна причина доступна у звіті в історії.".into()
    } else if value.contains("encoder")
        || value.contains("videotoolbox")
        || value.contains("libx264")
    {
        "Не вдалося запустити відеокодування. Уже готові файли збережено; спробуйте повторити проблемний матеріал.".into()
    } else {
        "Не вдалося завершити конвертацію одного з файлів. Уже готові результати збережено у вибраній папці.".into()
    }
}

pub(crate) fn download_error_code(message: &str) -> &'static str {
    if message.contains(crate::DESTINATION_UNAVAILABLE_MESSAGE) {
        return "destination_unavailable";
    }
    if message.contains(INSTAGRAM_PHOTO_ONLY_MARKER) {
        return "photo_only";
    }
    let value = message.to_lowercase();
    if value.contains("media sources found") {
        return "selection_required";
    }
    if value.contains("недостатньо вільного місця") || value.contains("no space left")
    {
        "low_disk"
    } else if is_rate_limited(&value) {
        "rate_limited"
    } else if [
        "sign in",
        "login required",
        "cookies are required",
        "private video",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "auth_required"
    } else if ["unsupported url", "no suitable extractor"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "unsupported"
    } else if [
        "failed to decrypt with dpapi",
        "failed to decrypt cookie",
        "app-bound encryption",
        "cookie database",
        "cookies database",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "auth_required"
    } else if has_token(&value, "404")
        || [
            "not found",
            "removed",
            "unavailable",
            "requested format is not available",
        ]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "unavailable"
    } else if is_network_failure(&value) {
        "network"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_match_only_as_whole_numbers() {
        for error in [
            "ERROR: [TikTok] 7429123456789012345: Requested format is not available",
            "ERROR: [youtube] ab429cd: Video unavailable",
        ] {
            assert_ne!(download_error_code(error), "rate_limited", "{error}");
        }
        assert_eq!(
            download_error_code("ERROR: HTTP Error 429: Too Many Requests"),
            "rate_limited"
        );
        assert!(is_forbidden_error("HTTP Error 403: Forbidden"));
        assert!(!is_forbidden_error(
            "__YTDLP_PROGRESS__ 52%|403.21KiB/s|00:03"
        ));
        assert!(!is_forbidden_error("ERROR: [TikTok] 7403123: no video"));
    }

    #[test]
    fn rate_limits_and_network_failures_do_not_blame_the_link() {
        assert!(
            friendly_download_error("HTTP Error 429: Too Many Requests").contains("обмежив запити")
        );
        assert!(friendly_download_error("Read timed out").contains("інтернет"));
        assert_eq!(download_error_code("Read timed out"), "network");
    }

    #[test]
    fn unavailable_format_is_not_reported_as_a_deleted_video() {
        let error = "ERROR: [generic] v1: Requested format is not available. Use --list-formats";
        assert!(friendly_download_error(error).contains("формату"));
        assert_eq!(download_error_code(error), "unavailable");
    }

    #[test]
    fn words_inside_ids_do_not_change_the_diagnosis() {
        assert!(!friendly_download_error("ERROR: [site] hdrmix: nothing here").contains("DRM"));
        assert!(
            !friendly_download_error("ERROR: [site] georgia-regional: not found")
                .contains("регіоні")
        );
    }
}
