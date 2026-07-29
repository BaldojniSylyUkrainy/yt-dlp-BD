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

Release-build є fail-closed: команда не створить `release/`, якщо немає updater key, Developer ID Application identity або Apple notarization credentials. Мінімальна підтримувана версія системи — macOS 12.3.

Результат буде в `release/`:

- `.dmg` — файл, який завантажує користувач;
- `.app.tar.gz` — пакет автоматичного оновлення;
- `.app.tar.gz.sig` — підпис пакета оновлення;
- `latest.json` — опис останньої версії для застосунку.

Публічні файли мають стабільний читабельний формат:
`BaldojnyiDownloader-<public-version>-Mac-Apple-Silicon.dmg`,
`BaldojnyiDownloader-<public-version>-Mac-Apple-Silicon-AutoUpdate.app.tar.gz` і
`BaldojnyiDownloader-<public-version>-Windows-x64-Setup.exe`. Внутрішні назви,
які створює Tauri, перейменовуються лише після складання; `latest.json` завжди
містить URL уже перейменованих підписаних updater-пакетів.

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

Ad-hoc identity `-` для release-build заборонена.

## Нотаризація

Один раз збережіть credentials у Keychain через `notarytool store-credentials`. Після цього перед release-build вкажіть назву профілю:

```bash
export APPLE_NOTARY_KEYCHAIN_PROFILE="yt-dlp-bd-notary"
npm run build:mac
```

Пароль Apple ID, `.p8` або інші секрети не повинні лежати в репозиторії.

У GitHub Actions замість Keychain profile використовується трійка `APPLE_API_ISSUER`, `APPLE_API_KEY` і `APPLE_API_KEY_PATH`. Workflow створює тимчасовий `.p8` із protected secret перед запуском команди. Після складання скрипт обов’язково перевіряє Developer ID signature, hardened runtime, timestamp, DMG, результат Apple notarization, stapled ticket і Gatekeeper assessment.

## GitHub і автооновлення

Правило вибору наступного номера версії описане у
[VERSIONING_UK.md](VERSIONING_UK.md). Коротко: лише виправлення збільшують PATCH,
явно замовлений hotfix — четверту публічну компоненту та наступний внутрішній
PATCH, а будь-яка нова функція збільшує MINOR і скидає PATCH до нуля.

Repository уже налаштований як `BaldojniSylyUkrainy/yt-dlp-BD`. macOS і Windows
release збирає ручний GitHub-hosted workflow, а не локальний upload окремих
файлів. Повний handoff для власника repository:
[GITHUB_RELEASE_HANDOFF_UK.md](GITHUB_RELEASE_HANDOFF_UK.md).

Підготовлений public release tag — `v0.5.1.1`, а внутрішня Tauri/SemVer version —
`0.5.2`. Workflow перевіряє public tag за `package.json.releaseVersion` і
створює draft Release. Після
ручної перевірки та публікації `latest.json` встановлені копії побачать нову
версію, перевірять криптографічний підпис своєї платформи й запропонують
оновлення.

## Найважливіше про updater key

Файл `.secrets/updater.key` не потрапляє до GitHub. Зробіть його резервну копію в password manager або іншому зашифрованому сховищі. Якщо втратити цей ключ, старі встановлені копії застосунку не зможуть перевірити підпис майбутніх оновлень.
