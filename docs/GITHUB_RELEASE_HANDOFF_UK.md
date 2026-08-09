# Запуск signed release

Repository `BaldojniSylyUkrainy/yt-dlp-BD`, protected environment і всі потрібні
Actions secrets уже налаштовані. Ручний workflow
`.github/workflows/release.yml` на стандартних GitHub-hosted runners:

- тестує і збирає Windows x64 NSIS installer;
- підписує Windows updater-артефакт приватним Tauri key;
- тестує, підписує Developer ID, нотаризує та перевіряє macOS Apple Silicon build;
- формує й тим самим Tauri key підписує manifest точних версій, URL і SHA-256 для yt-dlp, Deno та FFmpeg;
- перевіряє upstream SHA-256 і додає Windows FFmpeg як незмінний versioned asset конкретного Release;
- генерує один `latest.json` для macOS і Windows;
- створює **draft** GitHub Release, але не публікує його автоматично.

Workflow запускається тільки через `workflow_dispatch`. У ньому немає `push`,
`pull_request`, `schedule` або іншого автоматичного тригера.

## Запустити реліз

Номер наступного релізу визначайте за
[правилом версіювання](VERSIONING_UK.md): реліз лише з виправленнями збільшує
PATCH (`0.3.2` → `0.3.3`), а реліз із будь-якою новою функцією — MINOR зі
скиданням PATCH (`0.3.2` → `0.4.0`). Якщо нова функція та виправлення виходять
разом, застосовується MINOR. Для явно замовленого hotfix збільшується четверта
публічна компонента. Внутрішній Tauri PATCH завжди дорівнює
`PUBLIC_PATCH * 10 + HOTFIX`: наприклад, public `0.5.1.1` має внутрішню версію
`0.5.11`, а public `0.5.2.0` — `0.5.20`.

Перед запуском:

1. Переконайтесь, що потрібний commit уже в `main`, review завершено, а версії
   узгоджені.
2. Оновіть `RELEASE_NOTES.md`: вкажіть поточну версію з `package.json` і
   зрозумілими користувачеві пунктами, що саме змінилося. Один і той самий текст
   автоматично потрапить у GitHub Release та updater-вікно застосунку.
3. Відкрийте **Actions → Manual signed release → Run workflow**.
4. Branch: `main`.
5. `tag`: чотирикомпонентний public tag із `package.json`, наприклад
   `v0.6.1.0`.
6. Натисніть **Run workflow** і вручну approve jobs для environment `release`.

Workflow зупиниться, якщо tag не дорівнює `v${releaseVersion}`, бракує ключа,
підпису, нотаризації, stapling, Windows NSIS, хоча б одного updater signature або
актуального змістовного `RELEASE_NOTES.md`.

Після зеленого workflow:

1. Відкрийте **Releases**.
2. Знайдіть draft із потрібним tag.
3. Перевірте наявність:
   - `BaldojnyiDownloader-…-Windows-x64-Setup.exe` і його `.sig`;
   - `BaldojnyiDownloader-…-Mac-Apple-Silicon.dmg`;
   - `BaldojnyiDownloader-…-Mac-Apple-Silicon-AutoUpdate.app.tar.gz` і його `.sig`;
   - `BaldojnyiDownloader-…-Runtime-Windows-x64-FFmpeg.zip`;
   - `runtime-components.json` і його `.sig`;
   - `latest.json`.
4. За можливості встановіть `.exe` і `.dmg` на чистих тестових машинах.
5. Натисніть **Publish release**.

До публікації draft не стане новим `/releases/latest`, тому користувачі не
отримають напівготовий updater manifest.

`runtime-components.json` у `/releases/latest` є спільним для всіх підтримуваних
версій застосунку. Його `schemaVersion: 1` треба зберігати зворотно сумісним.
Якщо колись знадобиться несумісна schema, спочатку слід додати versioned endpoint
та міграцію в застосунок, а вже потім публікувати новий формат. Чисте встановлення
не зможе завантажити runtime-компоненти з draft: повний bootstrap перевіряється
після Publish release, тоді як installer/UI слід перевірити ще на draft.

## Увімкнути обов’язкові immutable Actions після merge

Workflow-файли в `0.6.1.0` уже використовують тільки повні 40-символьні commit
SHA. Одразу після merge цього commit власник repository має відкрити
**Settings → Actions → General → Actions permissions**, увімкнути
**Require actions to be pinned to a full-length commit SHA** і зберегти зміни.
Не вмикайте цю політику до merge: старий `main` із `@v5`/`@v7` перестане
запускати release workflow. Після ввімкнення GitHub відхилятиме будь-який новий
workflow із рухомим action tag.

## Windows SmartScreen

NSIS updater криптографічно підписаний Tauri updater key, тому застосунок перевіряє
його перед автооновленням. Це не Authenticode certificate. Без окремого Windows
code-signing certificate SmartScreen може показати попередження про невідомого
видавця на першому встановленні. Це не блокує складання або Tauri auto-update;
прибрати таке попередження можна лише додаванням окремого Authenticode
certificate у майбутньому.
