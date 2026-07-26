# Safari Extension

ScreenshotSafe's Safari extension is maintained as part of the checked-in Apple
project. It intentionally differs from the Chromium extension so Safari never
needs host permission for an arbitrary self-hosted ScreenshotSafe server.

## Canonical Sources

- Safari WebExtension resources:
  `apple/ScreenshotSafe/Shared (Safari Extension)/Resources/`
- Native message handler:
  `apple/ScreenshotSafe/Shared (Safari Extension)/SafariWebExtensionHandler.swift`
- Shared native configuration and upload code:
  `apple/ScreenshotSafe/Shared (Native)/`
- Xcode project:
  `apple/ScreenshotSafe/ScreenshotSafe.xcodeproj`

The Chromium extension in `extension/` is separate. Do not copy Chromium
resources over the Safari resources.

## Configuration And Upload

The containing app owns a versioned list of connected servers, one default
connection, and the default expiry. Non-secret metadata is stored in the
`group.com.screenshotsafe.safari` app group, while each bearer token is stored
in the shared Keychain.

Users can connect by entering a server domain and completing the server's PKCE
authorization flow in `ASWebAuthenticationSession`. On iOS, they can instead
scan the one-time setup QR shown after creating an API token on a desktop. That
QR contains a versioned JSON payload with the server origin and complete token;
it must be treated like a password.

The Safari WebExtension has no server host permissions:

1. The extension asks the native handler whether app configuration exists.
2. Its connection page asks the native handler to call `/api/ping` with the
   bearer API token.
3. Safari captures and edits the screenshot locally.
4. The editor sends the finalized PNG and metadata to the native handler.
5. The native upload client calls `/api/screenshots` with the bearer token.
6. Safari navigates to the authenticated editor page for the new screenshot.

The raw API token is never returned to WebExtension JavaScript.

## Build

Full Xcode is required. Build the checked-in macOS project:

```sh
scripts/build-safari-extension.sh
```

Build the checked-in iOS project:

```sh
scripts/build-safari-extension.sh --ios
```

To select Xcode without changing the system-wide developer directory:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer scripts/build-safari-extension.sh
```

The build script calls `xcodebuild` with signing disabled and writes derived
data under ignored `build/apple-derived-data/`. It does not invoke
`safari-web-extension-converter`, copy resources, or generate a project.

## Why The Converter Is Not Used

`safari-web-extension-converter` is useful for creating an initial wrapper from
a browser extension. Running it with `--force` over this repository would
regenerate files under `apple/ScreenshotSafe/` and could overwrite native app,
share-extension, entitlement, and project changes. The tracked Xcode project is
now maintained directly.

## Manual Safari Test

1. In the app, enter a ScreenshotSafe server domain and choose **Log in**, or on
   iOS create an API token from a desktop session and scan its setup QR.
2. Confirm the server appears as **Connected** and is selected as the default.
3. Build and run the macOS or iOS containing app.
4. Enable ScreenshotSafe in Safari.
5. Open **Check ScreenshotSafe Connection** and confirm the native check succeeds.
6. Confirm Safari does not ask for access to the ScreenshotSafe server website.
7. Capture a visible tab, crop or redact it, and upload.
8. Confirm the authenticated editor page opens for the new screenshot and the
   API token's last-used time changes on the server.
9. Connect a second server, change the default, and confirm the next Safari and
   share-extension uploads use the newly selected server.

References:

- Apple: https://developer.apple.com/documentation/safariservices/messaging-between-the-app-and-javascript-in-a-safari-web-extension
- Apple: https://developer.apple.com/documentation/safariservices/packaging-a-web-extension-for-safari
