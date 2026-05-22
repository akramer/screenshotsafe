#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="$ROOT_DIR/extension"
DIST_DIR="$ROOT_DIR/dist/safari-extension"
APP_NAME="${APP_NAME:-ScreenshotSafe}"
BUNDLE_IDENTIFIER="${BUNDLE_IDENTIFIER:-com.screenshotsafe.safari}"

mkdir -p "$DIST_DIR/icons"

cp "$SOURCE_DIR/background.js" "$DIST_DIR/background.js"
cp "$SOURCE_DIR/editor.html" "$DIST_DIR/editor.html"
cp "$SOURCE_DIR/editor.js" "$DIST_DIR/editor.js"
cp "$SOURCE_DIR/options.html" "$DIST_DIR/options.html"
cp "$SOURCE_DIR/options.js" "$DIST_DIR/options.js"
cp "$SOURCE_DIR/popup.html" "$DIST_DIR/popup.html"
cp "$SOURCE_DIR/popup.js" "$DIST_DIR/popup.js"
cp "$SOURCE_DIR/webext-api.js" "$DIST_DIR/webext-api.js"
cp "$SOURCE_DIR/icons/icon16.png" "$DIST_DIR/icons/icon16.png"
cp "$SOURCE_DIR/icons/icon48.png" "$DIST_DIR/icons/icon48.png"
cp "$SOURCE_DIR/icons/icon128.png" "$DIST_DIR/icons/icon128.png"
cp "$SOURCE_DIR/manifest.safari.json" "$DIST_DIR/manifest.json"

echo "Built Safari web-extension payload at $DIST_DIR"

if [[ "${1:-}" == "--xcode-project" ]]; then
    if ! xcrun --find safari-web-extension-converter >/dev/null 2>&1; then
        echo "safari-web-extension-converter was not found. Install full Xcode and select it with xcode-select." >&2
        exit 1
    fi

    xcrun safari-web-extension-converter "$DIST_DIR" \
        --project-location "$ROOT_DIR/apple" \
        --app-name "$APP_NAME" \
        --bundle-identifier "$BUNDLE_IDENTIFIER" \
        --copy-resources \
        --no-prompt \
        --force \
        --no-open
fi
