# Safari Extension

ScreenshotSafe's Safari extension uses the same popup and upload code as the
Chrome extension. Safari still requires a native app wrapper, so the repo keeps
the reusable WebExtension payload in `extension/` and generates the Safari
payload in `dist/safari-extension`.

## What Is Shared

- `extension/popup.html`
- `extension/popup.js`
- `extension/background.js`
- `extension/webext-api.js`
- `extension/icons/*`

`webext-api.js` wraps the small API surface used by the popup so Chrome's
callback APIs and Safari's `browser.*` promise APIs can run the same code.

## Build The Safari Payload

```sh
scripts/build-safari-extension.sh
```

This creates:

```text
dist/safari-extension/
  background.js
  icons/
  manifest.json
  popup.html
  popup.js
  webext-api.js
```

## Generate The Xcode Wrapper

Full Xcode is required for Apple's Safari converter. Command Line Tools alone
are not enough.

```sh
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
scripts/build-safari-extension.sh --xcode-project
```

Or run it without changing the global developer directory:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer scripts/build-safari-extension.sh --xcode-project
```

The script uses:

```sh
xcrun safari-web-extension-converter dist/safari-extension \
  --project-location apple \
  --app-name ScreenshotSafe \
  --bundle-identifier com.screenshotsafe.safari \
  --copy-resources \
  --no-prompt \
  --force \
  --no-open
```

The generated project is written to `apple/ScreenshotSafe/`.

Set a real bundle identifier before distribution:

```sh
BUNDLE_IDENTIFIER=com.example.ScreenshotSafe scripts/build-safari-extension.sh --xcode-project
```

## Manual Safari Test

1. Open the generated project in Xcode.
2. Build and run the macOS host app.
3. Open Safari > Settings > Extensions and enable ScreenshotSafe.
4. Sign in to your ScreenshotSafe server in Safari.
5. Open the extension popup, save the server URL, and grant website access when prompted.
6. Capture a visible tab and confirm the share link is created and opens.

## Current Scope

Implemented here:

- Shared popup/background JavaScript for Chrome and Safari.
- Safari-specific manifest with `browser_specific_settings.safari`.
- Safari payload build script.
- Xcode wrapper generation when full Xcode is installed.
- Generated Swift host app wrapper under `apple/ScreenshotSafe/`.

Still remaining:

- Decide whether the host app needs its own settings UI or whether popup
  settings are sufficient for the first Safari release.
- App Store signing, entitlements, and distribution metadata.

References:

- Apple: https://developer.apple.com/documentation/safariservices/optimizing-your-web-extension-for-safari
- Apple: https://developer.apple.com/documentation/safariservices/packaging-a-web-extension-for-safari
- MDN: https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/browser_specific_settings
