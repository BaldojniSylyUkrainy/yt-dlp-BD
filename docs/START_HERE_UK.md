# З чого почати

Ця інструкція розрахована на перший застосунок і перший GitHub-репозиторій.

## Що вже встановлено

- Node.js;
- Rust для Apple Silicon;
- Xcode Command Line Tools;
- залежності Tauri;
- локальний приватний ключ для автооновлень.

Повний Xcode для поточного desktop-білда не потрібен. `notarytool` уже доступний у Command Line Tools.

## Звичайний запуск під час розробки

У Terminal відкрийте папку проєкту й виконайте:

```bash
npm run tauri dev
```

## Створення локального білда

```bash
npm run build:mac
```

Результат буде в `release/`:

- `.dmg` — файл, який завантажує користувач;
- `.app.tar.gz` — пакет автоматичного оновлення;
- `.app.tar.gz.sig` — підпис пакета оновлення;
- `latest.json` — опис останньої версії для застосунку.

## Apple Developer certificate

Для розповсюдження поза App Store потрібен certificate типу **Developer ID Application**.

1. Увійдіть у Apple Developer → Certificates, Identifiers & Profiles.
2. Створіть `Developer ID Application` certificate через CSR із Keychain Access.
3. Завантажте `.cer` та відкрийте його — certificate потрапить у macOS Keychain.
4. Не експортуйте `.p12` у папку репозиторію і ніколи не комітьте його в GitHub.

Перевірка встановленого certificate:

```bash
security find-identity -v -p codesigning
```

Перед release-build установіть значення, яке покаже команда:

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: YOUR NAME (TEAMID)"
npm run build:mac
```

## Нотаризація

Один раз збережіть credentials у Keychain через `notarytool store-credentials`. Після цього перед release-build вкажіть назву профілю:

```bash
export APPLE_NOTARY_KEYCHAIN_PROFILE="yt-dlp-bd-notary"
npm run build:mac
```

Пароль Apple ID, `.p8` або інші секрети не повинні лежати в репозиторії.

## GitHub і автооновлення

1. Створіть порожній GitHub repository, наприклад `yt-dlp-BD`.
2. Адреса репозиторію вже налаштована як `BaldojniSylyUkrainy/yt-dlp-BD` у `release.config.json`.
3. Так само замініть endpoint у `src-tauri/tauri.conf.json`.
4. Збільште `version` у `package.json`, `src-tauri/Cargo.toml` і `src-tauri/tauri.conf.json`.
5. Запустіть `npm run build:mac`.
6. На GitHub створіть Release із тегом тієї самої версії, наприклад `v0.1.1`.
7. Додайте до Release всі чотири файли з папки `release/`.

Після публікації `latest.json` встановлені копії yt-dlp BD побачать нову версію, перевірять криптографічний підпис і запропонують оновлення.

## Найважливіше про updater key

Файл `.secrets/updater.key` не потрапляє до GitHub. Зробіть його резервну копію в password manager або іншому зашифрованому сховищі. Якщо втратити цей ключ, старі встановлені копії застосунку не зможуть перевірити підпис майбутніх оновлень.
