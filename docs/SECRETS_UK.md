# Підписи та секрети GitHub

Ніколи не додавайте в repository, GitHub Release, Issue або Artifact такі файли:

- Apple Developer ID certificate (`.p12` / `.pfx`) і його пароль;
- App Store Connect API key (`AuthKey_*.p8`);
- приватний ключ оновлень Tauri;
- будь-які `.key`, `.pem`, provisioning profiles або паролі.

Файл `.gitignore` блокує типові формати ключів локально. У публічному repository також увімкнені GitHub Secret Scanning і Push Protection. Це додаткові бар'єри, але не заміна правильному зберіганню секретів.

## Як передати секрети майбутньому GitHub Actions

Collaborator із write-доступом може додати секрети через **Settings → Secrets
and variables → Actions → Repository secrets**. Job публікації посилається на
environment `release`, тому environment approval усе одно лишається ручним
gate. Якщо значення додає власник/admin, він може натомість використати
**Settings → Environments → release → Environment secrets**. Не дублюйте одну
назву на обох рівнях: environment secret перекриває repository secret.

Плановані назви:

- `APPLE_CERTIFICATE` — `.p12`, перевірений через
  `openssl pkcs12 -legacy -in "$P12_FILE" -noout` і скопійований через
  `base64 -i "$P12_FILE" | pbcopy`;
- `APPLE_CERTIFICATE_PASSWORD`;
- `APPLE_API_KEY` — Key ID з App Store Connect;
- `APPLE_API_ISSUER`;
- `APPLE_API_KEY_CONTENT` — вміст App Store Connect `.p8`, який workflow записує у тимчасовий файл;
- `TAURI_SIGNING_PRIVATE_KEY`;
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Окремо задайте `APPLE_SIGNING_IDENTITY` як repository variable або release
environment variable: це повне ім’я імпортованого `Developer ID Application`
certificate. Воно потрібне release-скрипту, але не є приватним ключем і не
потребує Secret.

Base64 не є шифруванням. Результат base64 також не можна вставляти в код або надсилати у звичайному чаті — лише безпосередньо в поле GitHub Secret.

Під час notarization workflow має створити тимчасовий файл `.p8` з `APPLE_API_KEY_CONTENT`, встановити для нього права `600`, передати його локальний шлях через `APPLE_API_KEY_PATH` і гарантовано видалити файл у cleanup step. Tauri/notarytool очікують таку трійку:

- `APPLE_API_ISSUER` — Issuer ID;
- `APPLE_API_KEY` — Key ID;
- `APPLE_API_KEY_PATH` — шлях до тимчасово materialized `.p8`.

Для `.p12` workflow використовує системні macOS `base64` і `security import`.
Не додавайте CI-перевірку через звичайний `openssl pkcs12`: OpenSSL 3 без
legacy provider відхиляє валідні Keychain-експорти з `RC2-40-CBC`.

Не друкуйте в лог значення секрету, base64-вміст, повний environment dump або шлях до тимчасового ключа.

GitHub не показує збережене значення Secret назад. Workflow отримує лише ті секрети, які явно передані конкретному job. Не друкуйте секрети в лог і не запускайте release-job для неперевіреного коду.

`scripts/build-macos.sh` також підтримує локальний `APPLE_NOTARY_KEYCHAIN_PROFILE`. Для CI він приймає updater private key безпосередньо через `TAURI_SIGNING_PRIVATE_KEY`, а локально — через ignored-файл `.secrets/updater.key`. Release-build з ad-hoc identity, неповними Apple credentials або відсутнім updater key завершується помилкою до публікації артефактів.

Той самий updater key підписує `runtime-components.json`. Це не створює нового
секрету: застосунок перевіряє manifest уже вбудованим публічним updater key, а
приватний ключ доступний лише захищеному `release` environment. Manifest не
містить cookies, паролів або інших приватних даних — лише версії, дозволені
HTTPS URL і SHA-256 runtime-компонентів.

Операційний запуск signed release:
[GITHUB_RELEASE_HANDOFF_UK.md](GITHUB_RELEASE_HANDOFF_UK.md).
