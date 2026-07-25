# Підписи та секрети GitHub

Ніколи не додавайте в repository, GitHub Release, Issue або Artifact такі файли:

- Apple Developer ID certificate (`.p12` / `.pfx`) і його пароль;
- App Store Connect API key (`AuthKey_*.p8`);
- приватний ключ оновлень Tauri;
- будь-які `.key`, `.pem`, provisioning profiles або паролі.

Файл `.gitignore` блокує типові формати ключів локально. У публічному repository також увімкнені GitHub Secret Scanning і Push Protection. Це додаткові бар'єри, але не заміна правильному зберіганню секретів.

## Як передати секрети майбутньому GitHub Actions

Секрети додаються тільки через **Settings → Environments → release → Environment secrets**. Job публікації повинен посилатися на environment `release`; для нього варто ввімкнути ручне підтвердження власником repository.

Плановані назви:

- `APPLE_CERTIFICATE` — `.p12`, закодований у base64 перед додаванням у Secret;
- `APPLE_CERTIFICATE_PASSWORD`;
- `APPLE_API_KEY` — вміст App Store Connect `.p8`;
- `APPLE_API_KEY_ID`;
- `APPLE_API_ISSUER`;
- `TAURI_SIGNING_PRIVATE_KEY`;
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Base64 не є шифруванням. Результат base64 також не можна вставляти в код або надсилати у звичайному чаті — лише безпосередньо в поле GitHub Secret.

GitHub не показує збережене значення Secret назад. Workflow отримує лише ті секрети, які явно передані конкретному job. Не друкуйте секрети в лог і не запускайте release-job для неперевіреного коду.
