# ScreenshotSafe

ScreenshotSafe is a self-hosted screenshot capture, annotation, and sharing app. It gives you a private web dashboard, browser extension capture flow, editable screenshots, unlisted share links, API tokens, and optional automatic expiry for shared images.

The backend is a Rust/Axum web app with SQLite storage. The browser extension lives in `extension/`, with Safari packaging support in `safari/` and `docs/safari-extension.md`.

## Features

- First-run admin setup and password login
- Multi-user system
- Private dashboard for uploaded screenshots
- Browser extension capture flow with a pre-upload editor
- Screenshot annotation, crop, and rendered public image updates
- Unlisted share pages at `/s/{share_id}` and raw PNG links at `/s/{share_id}.png`
- API tokens for extension and scripted uploads
- Optional screenshot expiry with hourly cleanup
- Intended for docker deployment

## Requirements

- Rust 1.85 or newer
- SQLite, provided through the bundled `rusqlite` feature
- A Chromium-compatible browser for the unpacked extension
- Full Xcode if you want to generate or rebuild the Safari app wrapper

## Quick Start

Run the server:

```sh
cargo run
```

By default ScreenshotSafe listens on:

```text
http://localhost:8080
```

Open the app, create the first admin account, then use Settings to create an API token for the browser extension.

## Configuration

ScreenshotSafe loads configuration in this order:

1. `--config /path/to/config.toml`
2. `SCREENSHOTSAFE_CONFIG`
3. `config.toml` in the current directory
4. Built-in defaults

Example `config.toml`:

```toml
[server]
bind = "0.0.0.0:8080"
public_url = "https://screenshots.example.com"
max_screenshot_size_bytes = 26214400
# Optional. Omit or set to 0 for no global maximum.
max_expiry_seconds = 7776000

[storage]
path = "./data/storage"

[database]
path = "./data/screenshotsafe.db"

[auth]
session_ttl_seconds = 604800
default_expiry_seconds = 2592000
jwt_secret = "replace-with-a-long-random-secret"
```

Environment variable overrides:

```sh
SSS_BIND=127.0.0.1:8080
SSS_PUBLIC_URL=https://screenshots.example.com
SSS_MAX_SCREENSHOT_SIZE_BYTES=26214400
SSS_MAX_EXPIRY_SECONDS=7776000
SSS_STORAGE_PATH=/data/storage
SSS_DATABASE_PATH=/data/screenshotsafe.db
SSS_JWT_SECRET=replace-with-a-long-random-secret
```

If `jwt_secret` is omitted, ScreenshotSafe generates one and stores it next to the storage directory as `.jwt_secret`.

`max_screenshot_size_bytes` defaults to 25 MiB. `default_expiry_seconds` controls the default retention window for newly uploaded screenshots, and `max_expiry_seconds` optionally caps requested expiry windows. Admins can set per-user overrides for both limits from the Admin page; blank or `0` means the user follows the server setting.

## Browser Extension

The shared WebExtension source is in `extension/`.

For Chrome or another Chromium browser:

1. Open the browser extension management page.
2. Enable developer mode.
3. Load `extension/` as an unpacked extension.
4. In ScreenshotSafe, create an API token from Settings.
5. Open the extension settings and enter your server URL plus API token.

The extension verifies the connection with `/api/ping`, captures the visible tab, opens a local editor, and uploads the finalized screenshot to your server.

The privacy policy for store listings or public installs is available in [docs/privacy-policy.md](docs/privacy-policy.md).

## Safari Extension

Safari uses the same extension payload plus a native wrapper.

Build the Safari payload:

```sh
scripts/build-safari-extension.sh
```

Generate the Xcode wrapper with full Xcode:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer scripts/build-safari-extension.sh --xcode-project
```

More detail is in [docs/safari-extension.md](docs/safari-extension.md).

## API

Most browser API routes require a session cookie. Upload and ping also accept API tokens:

```http
Authorization: Bearer sss_...
```

Upload a screenshot:

```sh
curl -X POST http://localhost:8080/api/screenshots \
  -H "Authorization: Bearer $SCREENSHOTSAFE_TOKEN" \
  -F "image=@screenshot.png" \
  -F "title=Example screenshot" \
  -F "source_url=https://example.com" \
  -F "expires_in=30d"
```

Supported `expires_in` values use `m`, `h`, `d`, or `w`, such as `15m`, `24h`, `30d`, or `1w`.

The upload response includes:

- `share_url`: public unlisted share page
- `raw_url`: direct PNG URL
- `share_id`: short public identifier

## Docker

Build the image:

```sh
docker build -t screenshotsafe .
```

Run it with persistent data:

```sh
docker run --rm -p 8080:8080 \
  -v "$PWD/data:/data" \
  -e SSS_PUBLIC_URL=http://localhost:8080 \
  screenshotsafe
```

The Docker image defaults to:

```text
SSS_STORAGE_PATH=/data/storage
SSS_DATABASE_PATH=/data/screenshotsafe.db
```

## Development

Run tests:

```sh
cargo test
```

Format code:

```sh
cargo fmt
```

Useful project paths:

- `src/`: Rust application, routes, auth, config, database, and image rendering
- `static/`: web UI CSS and editor JavaScript used by the server-rendered app
- `extension/`: shared Chrome/Safari WebExtension source
- `scripts/build-safari-extension.sh`: Safari payload and Xcode wrapper generator
- `tests/`: integration tests

## Security Notes

ScreenshotSafe is intended to be self-hosted. Put it behind HTTPS before using it outside local development, keep your JWT secret stable and private, and treat API tokens like passwords. Shared screenshot URLs are unlisted, not authenticated, so anyone with a share URL can view that rendered image until it expires or is deleted.
