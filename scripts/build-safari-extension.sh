#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT="$ROOT_DIR/apple/ScreenshotSafe/ScreenshotSafe.xcodeproj"
DERIVED_DATA_DIR="$ROOT_DIR/build/apple-derived-data"
PLATFORM="${1:---macos}"

case "$PLATFORM" in
    --macos)
        SCHEME="ScreenshotSafe (macOS)"
        DESTINATION="platform=macOS"
        ;;
    --ios)
        SCHEME="ScreenshotSafe (iOS)"
        DESTINATION="generic/platform=iOS Simulator"
        ;;
    -h|--help)
        echo "Usage: scripts/build-safari-extension.sh [--macos|--ios]"
        echo
        echo "Builds the checked-in ScreenshotSafe Xcode project without regenerating it."
        exit 0
        ;;
    *)
        echo "Unknown option: $PLATFORM" >&2
        echo "Usage: scripts/build-safari-extension.sh [--macos|--ios]" >&2
        exit 2
        ;;
esac

if [[ ! -d "$PROJECT" ]]; then
    echo "Missing checked-in Xcode project: $PROJECT" >&2
    exit 1
fi

if ! xcrun --find xcodebuild >/dev/null 2>&1; then
    echo "xcodebuild was not found. Install full Xcode and select it with xcode-select." >&2
    exit 1
fi

xcrun xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -configuration Debug \
    -destination "$DESTINATION" \
    -derivedDataPath "$DERIVED_DATA_DIR" \
    CODE_SIGNING_ALLOWED=NO \
    build
