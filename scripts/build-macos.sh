#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
UPDATER_KEY_PATH="${TAURI_UPDATER_KEY_PATH:-$PROJECT_DIR/.secrets/updater.key}"

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  if [[ ! -f "$UPDATER_KEY_PATH" ]]; then
    echo "Не знайдено приватний updater key: $UPDATER_KEY_PATH"
    echo "Передайте TAURI_SIGNING_PRIVATE_KEY у CI або відновіть локальний файл із резервної копії."
    echo "Не створюйте новий ключ, якщо застосунок уже розповсюджувався."
    exit 1
  fi
  TAURI_SIGNING_PRIVATE_KEY="$(<"$UPDATER_KEY_PATH")"
  export TAURI_SIGNING_PRIVATE_KEY
fi

export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" || "$APPLE_SIGNING_IDENTITY" == "-" ]]; then
  echo "Release-build вимагає APPLE_SIGNING_IDENTITY типу Developer ID Application."
  exit 1
fi

NOTARY_AUTH_ARGS=()
if [[ -n "${APPLE_NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
  NOTARY_AUTH_ARGS=(--keychain-profile "$APPLE_NOTARY_KEYCHAIN_PROFILE")
else
  if [[ -z "${APPLE_API_ISSUER:-}" || -z "${APPLE_API_KEY:-}" || -z "${APPLE_API_KEY_PATH:-}" ]]; then
    echo "Не налаштовано Apple notarization."
    echo "Передайте APPLE_NOTARY_KEYCHAIN_PROFILE або повну трійку APPLE_API_ISSUER, APPLE_API_KEY, APPLE_API_KEY_PATH."
    exit 1
  fi
  if [[ ! -f "$APPLE_API_KEY_PATH" ]]; then
    echo "Не знайдено App Store Connect API key за шляхом APPLE_API_KEY_PATH."
    exit 1
  fi
  NOTARY_AUTH_ARGS=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
fi

cd "$PROJECT_DIR"

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo "Release-build заборонено: tracked-файли мають незакомічені зміни."
  exit 1
fi

npm run tauri build -- --bundles app --target aarch64-apple-darwin

VERSION="$(node -p "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json')).version")"
BUNDLE_DIR="$PROJECT_DIR/src-tauri/target/aarch64-apple-darwin/release/bundle"
APP_PATH="$BUNDLE_DIR/macos/yt-dlp BD.app"
DMG_PATH="$BUNDLE_DIR/dmg/yt-dlp BD_${VERSION}_aarch64.dmg"
BUILD_STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/yt-dlp-bd-dmg.XXXXXX")"

case "$BUILD_STAGING_DIR" in
  */yt-dlp-bd-dmg.*) ;;
  *) echo "Некоректна тимчасова папка"; exit 1 ;;
esac

cleanup() {
  rm -rf "$BUILD_STAGING_DIR"
}
trap cleanup EXIT

if [[ ! -d "$APP_PATH" ]]; then
  echo "Tauri не створив очікуваний app bundle: $APP_PATH"
  exit 1
fi

APP_SIGNATURE_INFO="$(codesign --display --verbose=4 "$APP_PATH" 2>&1)"
if ! grep -q "Authority=Developer ID Application:" <<<"$APP_SIGNATURE_INFO"; then
  echo "App bundle не має Developer ID Application signature."
  exit 1
fi
if ! grep -q "flags=.*runtime" <<<"$APP_SIGNATURE_INFO"; then
  echo "App bundle зібрано без hardened runtime."
  exit 1
fi
if ! grep -q "Timestamp=" <<<"$APP_SIGNATURE_INFO"; then
  echo "App bundle не має secure signing timestamp."
  exit 1
fi
if codesign --display --entitlements :- "$APP_PATH" 2>/dev/null | grep -q "com.apple.security.get-task-allow"; then
  echo "Release app містить заборонений debug entitlement com.apple.security.get-task-allow."
  exit 1
fi
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

mkdir -p "$BUNDLE_DIR/dmg"
ditto "$APP_PATH" "$BUILD_STAGING_DIR/yt-dlp BD.app"
ln -s /Applications "$BUILD_STAGING_DIR/Applications"
hdiutil create -volname "yt-dlp BD" -srcfolder "$BUILD_STAGING_DIR" -ov -format UDZO "$DMG_PATH"

codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG_PATH"
codesign --verify --strict --verbose=2 "$DMG_PATH"
hdiutil verify "$DMG_PATH"

NOTARY_RESULT_PATH="$BUILD_STAGING_DIR/notary-result.json"
xcrun notarytool submit "$DMG_PATH" "${NOTARY_AUTH_ARGS[@]}" --wait --output-format json >"$NOTARY_RESULT_PATH"
cat "$NOTARY_RESULT_PATH"

NOTARY_STATUS="$(NOTARY_RESULT_PATH="$NOTARY_RESULT_PATH" node -e \
  "const fs=require('fs'); const value=JSON.parse(fs.readFileSync(process.env.NOTARY_RESULT_PATH,'utf8')); process.stdout.write(value.status || '')")"
NOTARY_ID="$(NOTARY_RESULT_PATH="$NOTARY_RESULT_PATH" node -e \
  "const fs=require('fs'); const value=JSON.parse(fs.readFileSync(process.env.NOTARY_RESULT_PATH,'utf8')); process.stdout.write(value.id || '')")"
if [[ "$NOTARY_STATUS" != "Accepted" || -z "$NOTARY_ID" ]]; then
  echo "Apple notarization не завершилась статусом Accepted."
  exit 1
fi

xcrun notarytool log "$NOTARY_ID" "$BUILD_STAGING_DIR/notary-log.json" "${NOTARY_AUTH_ARGS[@]}"
cat "$BUILD_STAGING_DIR/notary-log.json"
xcrun stapler staple "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"
spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG_PATH"

npm run release:json
echo "Готово. Файли для GitHub Release: $PROJECT_DIR/release"
