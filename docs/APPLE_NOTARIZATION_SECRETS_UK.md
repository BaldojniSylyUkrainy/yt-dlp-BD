# Apple Developer: де взяти секрети для GitHub notarization

Секретні значення нижче додаються через
**GitHub repository → Settings → Secrets and variables → Actions → Secrets**.
Якщо власник repository керує секретами сам, він може натомість додати їх у
**Settings → Environments → release → Environment secrets**. Не створюйте
однакові назви одночасно на обох рівнях: environment secret має вищий пріоритет.

`APPLE_SIGNING_IDENTITY` не є секретом і додається у вкладці **Variables** на
тому самому рівні. Нічого з цих значень не додавайте в `.env`, Issue, чат,
GitHub Artifact або файли проєкту.

## Передумови

Потрібне активне членство Apple Developer Program і доступ до:

- Certificates, Identifiers & Profiles;
- App Store Connect → Users and Access → Integrations.

Для розповсюдження поза Mac App Store використовується
**Developer ID Application**, не Apple Development і не Mac App Distribution.

Офіційна довідка:

- [Tauri macOS code signing](https://v2.tauri.app/distribute/sign/macos/);
- [Apple notarytool authentication](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).

## 1. Developer ID Application certificate

На Mac, де створюється certificate:

1. Відкрийте **Keychain Access → Certificate Assistant → Request a Certificate
   From a Certificate Authority** і збережіть CSR.
2. У [Apple Developer Certificates](https://developer.apple.com/account/resources/certificates/list)
   натисніть `+`, оберіть **Developer ID Application**, завантажте CSR і створіть
   certificate.
3. Завантажте `.cer` та відкрийте його. У Keychain Access certificate має
   з’явитися разом із вкладеним private key.
4. У **My Certificates** розкрийте certificate, виділіть саме запис certificate
   з private key і виконайте **Export Items…** у форматі `.p12`.
5. Задайте сильний тимчасовий пароль для `.p12`.

Перевірте точне ім’я identity:

```bash
security find-identity -v -p codesigning
```

Рядок виглядає так:

```text
Developer ID Application: YOUR NAME (TEAMID)
```

Додайте його як Environment **variable**:

```text
APPLE_SIGNING_IDENTITY
```

Перевірте саме експортований файл і пароль. Старі версії Keychain можуть
використовувати `RC2-40-CBC`, тому для локального OpenSSL 3 потрібен `-legacy`:

```bash
P12_FILE="/absolute/path/to/certificate.p12"
openssl pkcs12 -legacy -in "$P12_FILE" -noout \
  && echo "✅ Пароль правильний, .p12 валідний"
```

Скопіюйте Base64 у clipboard системною macOS-утилітою, не друкуючи його в
термінал:

```bash
base64 -i "$P12_FILE" | pbcopy
```

Вставте clipboard у GitHub secret:

```text
APPLE_CERTIFICATE
```

Пароль `.p12` вставте в:

```text
APPLE_CERTIFICATE_PASSWORD
```

Workflow декодує secret системною macOS `base64`, а `.p12` імпортує системною
`security import`. Він навмисно не перевіряє PKCS#12 через OpenSSL: сучасний
OpenSSL без legacy provider відхиляє коректні Keychain-експорти з
`RC2-40-CBC`. Саме `security import` є остаточною CI-перевіркою Base64, файла і
пароля.

Після збереження secret не залишайте незашифровані копії `.p12` поза захищеним
сховищем. Не видаляйте certificate і private key із Keychain до перевірки
першого release.

## 2. App Store Connect API key для notarization

1. Відкрийте **App Store Connect → Users and Access → Integrations → Team Keys**.
2. Створіть новий key із доступом **Developer**. Для notarization Admin-доступ не
   потрібен.
3. Одразу зафіксуйте:
   - **Issuer ID**;
   - **Key ID**.
4. Завантажте `AuthKey_<KEY_ID>.p8`. Apple дозволяє завантажити цей файл лише
   один раз, тому відразу зробіть зашифровану резервну копію.

Додайте GitHub Actions secrets:

| GitHub secret | Що вставити |
|---|---|
| `APPLE_API_ISSUER` | Issuer ID |
| `APPLE_API_KEY` | Key ID, не вміст `.p8` |
| `APPLE_API_KEY_CONTENT` | Повний текст `.p8` разом із BEGIN/END PRIVATE KEY |

Workflow записує `APPLE_API_KEY_CONTENT` у тимчасовий файл із правами `600`,
передає його `notarytool`, а cleanup step видаляє файл незалежно від результату.

## 3. Tauri updater key

Apple certificate не замінює updater key. Для macOS і Windows використовується
одна й та сама наявна пара Tauri updater keys.

На машині, де є ignored-файл `.secrets/updater.key`:

```bash
pbcopy < .secrets/updater.key
```

Вставте значення безпосередньо в GitHub Actions secret:

```text
TAURI_SIGNING_PRIVATE_KEY
```

Якщо під час створення ключа задавався пароль, додайте його в
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Якщо пароля не було, цей secret не потрібен.

Не запускайте `tauri signer generate` для заміни ключа: public key уже вбудований
у застосунок. Новий private key не зможе підписати оновлення для існуючих
інсталяцій.

## 4. Безпечна перевірка

Після додавання secrets GitHub не покаже їх назад. Перевірка робиться лише
ручним запуском `Manual signed release` з `main`. Для поточного релізу у полі
`tag` введіть `v0.3.0.0`.

Безпечний workflow:

- не має автоматичних тригерів;
- отримує secrets лише після approval environment `release`;
- не передає secrets pull-request workflows або зовнішнім forks;
- не друкує значення secrets;
- fail-closed перевіряє Developer ID signature, hardened runtime, timestamp,
  notarization `Accepted`, stapling і Gatekeeper.

Якщо `.p8`, `.p12`, updater private key або їх пароль випадково потрапив у commit,
Artifact чи відкритий чат, одного видалення недостатньо: відповідний key/certificate
треба відкликати або замінити.
