# Handoff: що налаштувати власнику GitHub repository

Цей документ призначений власнику `BaldojniSylyUkrainy/yt-dlp-BD`. Код уже містить
ручний workflow `.github/workflows/release.yml`, який на стандартних
GitHub-hosted runners:

- тестує і збирає Windows x64 NSIS installer;
- підписує Windows updater-артефакт приватним Tauri key;
- тестує, підписує Developer ID, нотаризує та перевіряє macOS Apple Silicon build;
- генерує один `latest.json` для macOS і Windows;
- створює **draft** GitHub Release, але не публікує його автоматично.

Workflow запускається тільки через `workflow_dispatch`. У ньому немає `push`,
`pull_request`, `schedule` або іншого автоматичного тригера.

## 1. Захистити `main`

Відкрийте **Settings → Rules → Rulesets → New branch ruleset**:

1. Назва: `Protect main`.
2. Target branches: `main`.
3. Enforcement status: `Active`.
4. Увімкніть:
   - Restrict deletions;
   - Block force pushes;
   - Require a pull request before merging;
   - Require at least 1 approval;
   - Dismiss stale approvals when new commits are pushed;
   - Require conversation resolution before merging.
5. Не додавайте широких bypass-правил.

За взаємної довіри між двома collaborators це не захищає від навмисних дій
власника repository, але не дозволяє зовнішньому користувачу підмінити release-код.

## 2. Дозволити тільки потрібні Actions

У **Settings → Actions → General**:

1. Actions permissions: дозволити GitHub-authored actions. Workflow використовує
   тільки `actions/checkout`, `actions/setup-node`, `actions/upload-artifact` і
   `actions/download-artifact`; сторонніх Actions немає.
2. Workflow permissions: **Read and write permissions**.
3. Залишити вимкненим автоматичне схвалення pull requests через `GITHUB_TOKEN`.

Репозиторій має залишатися public. Для public repository стандартні
GitHub-hosted runners безкоштовні й безлімітні; workflow використовує
`windows-latest`, `macos-15` та `ubuntu-latest`. Не замінюйте їх на larger runner
labels: larger runners тарифікуються окремо.

Джерело: [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

## 3. Створити protected environment

Відкрийте **Settings → Environments → New environment**:

1. Назва рівно `release`.
2. Deployment branches and tags: дозволити лише `main`.
3. Required reviewers: додати людину, яка має вручну дозволяти release-job.
4. За потреби увімкнути `Prevent self-review`, щоб автор запуску не міг сам
   відкрити секрети job.

`workflow_dispatch` уже вимагає ручного запуску. Protected environment додає
другий ручний gate перед тим, як macOS/Windows jobs отримають secrets.

## 4. Додати Actions secrets і variable

Якщо collaborator бачить лише **Settings → Secrets and variables → Actions**,
це нормально. У вкладці **Secrets** створіть такі **Repository secrets**:

| Type | Назва | Значення |
|---|---|---|
| Secret | `TAURI_SIGNING_PRIVATE_KEY` | Повний вміст наявного приватного updater key |
| Secret | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Пароль updater key; якщо ключ без пароля, secret можна не створювати |
| Secret | `APPLE_CERTIFICATE` | `.p12` Developer ID Application у base64 одним рядком |
| Secret | `APPLE_CERTIFICATE_PASSWORD` | Пароль, заданий під час експорту `.p12` |
| Secret | `APPLE_API_ISSUER` | Issuer ID App Store Connect API |
| Secret | `APPLE_API_KEY` | Key ID App Store Connect API |
| Secret | `APPLE_API_KEY_CONTENT` | Повний вміст `AuthKey_....p8`, включно з BEGIN/END |

У вкладці **Variables** створіть Repository variable:

| Type | Назва | Значення |
|---|---|---|
| Variable | `APPLE_SIGNING_IDENTITY` | `Developer ID Application: NAME (TEAMID)` |

Jobs із `environment: release` отримають ці repository secrets лише після
environment approval. Зовнішні pull requests із forks їх не отримують.

Альтернатива для власника/admin: додати ті самі значення безпосередньо в
**Settings → Environments → release**. Це сильніше звужує область дії, але не є
обов’язковим для двох довірених collaborators. Не дублюйте однакову назву на
repository й environment рівнях, бо environment value перекриє repository
value.

Apple-значення описані покроково в
[`APPLE_NOTARIZATION_SECRETS_UK.md`](APPLE_NOTARIZATION_SECRETS_UK.md).

Не створюйте новий Tauri updater key: він має відповідати `pubkey`, уже вбудованому
в `src-tauri/tauri.conf.json`. Втрата або заміна цього приватного ключа зламає
оновлення для вже встановлених копій.

## 5. Запустити реліз

Перед запуском:

1. Переконайтесь, що потрібний commit уже в `main`, review завершено, а версії
   узгоджені.
2. Відкрийте **Actions → Manual signed release → Run workflow**.
3. Branch: `main`.
4. `tag`: чотирикомпонентний public tag із `package.json`, наприклад
   `v0.3.0.0`.
5. `notes`: короткий текст для updater-вікна.
6. Натисніть **Run workflow** і вручну approve jobs для environment `release`.

Workflow зупиниться, якщо tag не дорівнює `v${releaseVersion}`, бракує ключа,
підпису, нотаризації, stapling, Windows NSIS або хоча б одного updater signature.

Після зеленого workflow:

1. Відкрийте **Releases**.
2. Знайдіть draft із потрібним tag.
3. Перевірте наявність:
   - Windows `.exe` і `.exe.sig`;
   - macOS `.dmg`, `.app.tar.gz` і `.app.tar.gz.sig`;
   - `latest.json`.
4. За можливості встановіть `.exe` і `.dmg` на чистих тестових машинах.
5. Натисніть **Publish release**.

До публікації draft не стане новим `/releases/latest`, тому користувачі не
отримають напівготовий updater manifest.

## Windows SmartScreen

NSIS updater криптографічно підписаний Tauri updater key, тому застосунок перевіряє
його перед автооновленням. Це не Authenticode certificate. Без окремого Windows
code-signing certificate SmartScreen може показати попередження про невідомого
видавця на першому встановленні. Це не блокує складання або Tauri auto-update;
прибрати таке попередження можна лише додаванням окремого Authenticode
certificate у майбутньому.

## Перший повторний запуск після CI hotfix

Перед повторним запуском обов’язково перезапишіть `APPLE_CERTIFICATE` із
перевіреного оригінального `.p12`:

```bash
P12_FILE="/absolute/path/to/certificate.p12"
openssl pkcs12 -legacy -in "$P12_FILE" -noout
base64 -i "$P12_FILE" | pbcopy
```

Вставте clipboard як нове значення `APPLE_CERTIFICATE`, а потім запустіть
workflow з tag `v0.3.0.0`. CI використовує системні macOS `base64` і
`security import`, тому валідний старий Keychain PKCS#12 із `RC2-40-CBC` більше
не відхиляється OpenSSL 3.
