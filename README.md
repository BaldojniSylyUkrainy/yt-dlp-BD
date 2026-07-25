# yt-dlp BD

**Baldojnyi Downloader** — нативний desktop-застосунок для зручної роботи з офіційним [yt-dlp](https://github.com/yt-dlp/yt-dlp).

Застосунок не вбудовує застарілу копію yt-dlp у свій білд. Під час першого запуску він завантажує офіційний runtime у приховану user-data папку, перевіряє SHA-256 і надалі оновлює його незалежно від самого GUI.

## Що вже працює

- нативний Tauri 2 застосунок для macOS Apple Silicon;
- автоматичне встановлення й оновлення офіційного стабільного yt-dlp;
- власні керовані ffmpeg/ffprobe, що автоматично відновлюються й оновлюються;
- автоматичне встановлення офіційного Deno для повної підтримки YouTube;
- відео, audio-only, вибір якості, субтитри та папка збереження;
- живий прогрес і скасування завантаження;
- запит cookies із браузера, коли сайт вимагає логін, перевірку віку або anti-bot підтвердження;
- попередження для доменів `.ru` та `.рф`, якщо VPN не виявлено;
- окремий підписаний пакет Tauri updater;
- локальна команда, що створює `.app`, `.dmg`, updater і `latest.json`.

## Швидкий старт для розробки

```bash
npm install
npm run tauri dev
```

## Локальний macOS build

```bash
npm run build:mac
```

Готові файли з’являться в папці `release/`. Детальна інструкція для першого GitHub-релізу та Apple-підпису: [docs/START_HERE_UK.md](docs/START_HERE_UK.md).

## Секрети

Ніколи не додавайте до GitHub:

- `.p12` Apple Developer certificate;
- приватний Tauri updater key;
- Apple API key `.p8`;
- паролі або токени.

Приватний updater key зберігається в `.secrets/updater.key`; ця папка виключена через `.gitignore`. Зробіть резервну копію ключа у надійному сховищі: без нього вже встановлені копії застосунку не прийматимуть майбутні оновлення.

## Runtime-джерела

- yt-dlp: офіційні [стабільні релізи](https://github.com/yt-dlp/yt-dlp/releases);
- Deno: офіційні [Deno releases](https://github.com/denoland/deno/releases);
- ffmpeg/ffprobe для Apple Silicon: підписані macOS release builds із [Martin Riedl’s FFmpeg Build Server](https://ffmpeg.martin-riedl.de/), з обов’язковою SHA-256 перевіркою.

Завантажуйте лише матеріали, на які маєте відповідні права.
