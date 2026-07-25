#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
UPDATER_KEY_PATH="${TAURI_UPDATER_KEY_PATH:-$PROJECT_DIR/.secrets/updater.key}"

if [[ ! -f "$UPDATER_KEY_PATH" ]]; then
  echo "Не знайдено приватний updater key: $UPDATER_KEY_PATH"
  echo "Не створюйте новий ключ, якщо застосунок уже розповсюджувався — відновіть резервну копію."
  exit 1
fi

TAURI_KEY_CONTENT="$(<"$UPDATER_KEY_PATH")"
export TAURI_SIGNING_PRIVATE_KEY="$TAURI_KEY_CONTENT"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"

cd "$PROJECT_DIR"
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

mkdir -p "$BUNDLE_DIR/dmg"
ditto "$APP_PATH" "$BUILD_STAGING_DIR/yt-dlp BD.app"
ln -s /Applications "$BUILD_STAGING_DIR/Applications"
hdiutil create -volname "yt-dlp BD" -srcfolder "$BUILD_STAGING_DIR" -ov -format UDZO "$DMG_PATH"

if [[ "$APPLE_SIGNING_IDENTITY" != "-" ]]; then
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" "$DMG_PATH"
fi

if [[ -n "${APPLE_NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
  xcrun notarytool submit "$DMG_PATH" --keychain-profile "$APPLE_NOTARY_KEYCHAIN_PROFILE" --wait
  xcrun stapler staple "$DMG_PATH"
fi

npm run release:json
echo "Готово. Файли для GitHub Release: $PROJECT_DIR/release"
