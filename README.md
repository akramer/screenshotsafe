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

[auth.oauth]
enabled = false
provider = "example"
display_name = "Example SSO"
client_id = ""
client_secret = ""
issuer_url = "https://provider.example"
# Optional override. If omitted, this is derived from issuer_url.
discovery_url = "https://provider.example/.well-known/openid-configuration"
authorize_url = "https://provider.example/oauth/authorize"
token_url = "https://provider.example/oauth/token"
userinfo_url = "https://provider.example/oauth/userinfo"
scope = "openid email profile"
redirect_url = "https://screenshots.example.com/api/auth/oauth/callback"
account_mode = "link_only" # link_only, pending, or auto_enabled
allowed_email_domains = ["example.com"]
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
SSS_OAUTH_ENABLED=true
SSS_OAUTH_PROVIDER=example
SSS_OAUTH_DISPLAY_NAME="Example SSO"
SSS_OAUTH_CLIENT_ID=client-id
SSS_OAUTH_CLIENT_SECRET=client-secret
SSS_OAUTH_ISSUER_URL=https://provider.example
SSS_OAUTH_DISCOVERY_URL=https://provider.example/.well-known/openid-configuration
SSS_OAUTH_AUTHORIZE_URL=https://provider.example/oauth/authorize
SSS_OAUTH_TOKEN_URL=https://provider.example/oauth/token
SSS_OAUTH_USERINFO_URL=https://provider.example/oauth/userinfo
SSS_OAUTH_REDIRECT_URL=https://screenshots.example.com/api/auth/oauth/callback
SSS_OAUTH_ACCOUNT_MODE=pending
SSS_OAUTH_ALLOWED_EMAIL_DOMAINS=example.com,example.org
```

If `jwt_secret` is omitted, ScreenshotSafe generates one and stores it next to the storage directory as `.jwt_secret`.

`max_screenshot_size_bytes` defaults to 25 MiB. `default_expiry_seconds` controls the default retention window for newly uploaded screenshots, and `max_expiry_seconds` optionally caps requested expiry windows. Admins can set per-user overrides for both limits from the Admin page; blank or `0` means the user follows the server setting.

OAuth uses one configured identity provider. If `issuer_url` or `discovery_url` is set, ScreenshotSafe reads the OpenID discovery document and uses the discovered authorization, token, and userinfo endpoints unless explicit endpoint URLs are configured. `account_mode = "link_only"` only allows OAuth identities that users have linked from Settings. `account_mode = "pending"` creates disabled-by-default pending accounts for admins to enable. `account_mode = "auto_enabled"` creates enabled non-admin accounts immediately. When `allowed_email_domains` is set, the OAuth userinfo response must include an allowed verified email domain.

## OAuth Authentication

ScreenshotSafe can add OAuth sign-in alongside the built-in password login. Password login remains available for accounts with a password, and OAuth identities are stored separately from local users so the configured provider identity can be linked to a local account.

OAuth is configured under `[auth.oauth]`, and only one provider can be active at a time. The implementation expects that provider to expose an authorization endpoint, token endpoint, and userinfo endpoint. `provider` is the stable internal key used for identity matching. `display_name` is optional user-facing text for buttons and messages, such as "Sign in with Google" or "Connect Acme SSO". OIDC providers work well because their userinfo response normally includes a stable `sub` field. For non-OIDC providers, ScreenshotSafe can also use an `id` field from userinfo.

Example:

```toml
[auth.oauth]
enabled = true
provider = "google"
display_name = "Google"
client_id = "..."
client_secret = "..."
issuer_url = "https://accounts.google.com"
scope = "openid email profile"
redirect_url = "https://screenshots.example.com/api/auth/oauth/callback"
account_mode = "pending"
allowed_email_domains = ["example.com"]
```

Register this redirect URI with your OAuth provider:

```text
https://screenshots.example.com/api/auth/oauth/callback
```

If `redirect_url` is omitted, ScreenshotSafe builds it from `server.public_url` or the request host. For production, set both `server.public_url` and `auth.oauth.redirect_url` explicitly so provider callbacks are stable.

When `issuer_url` is set, ScreenshotSafe reads:

```text
{issuer_url}/.well-known/openid-configuration
```

You can set `discovery_url` directly for providers that publish the document somewhere else. If `authorize_url`, `token_url`, or `userinfo_url` are configured explicitly, those values override the discovery document for that specific endpoint.

### Account Modes

`link_only` is the safest default. OAuth can only be used after a signed-in user links the configured provider identity from Settings. Unknown OAuth identities are rejected at login. Use this for private installs where admins create accounts manually.

`pending` allows self-service OAuth requests without granting immediate access. When an unknown OAuth identity signs in, ScreenshotSafe creates a local non-admin account with `account_status = "pending"` and does not issue a session. An admin must enable the account from the Admin page before the user can sign in.

`auto_enabled` creates a local enabled non-admin account the first time an unknown OAuth identity signs in. Use this only when your provider and `allowed_email_domains` setting already define the trusted user population.

### Linking Existing Accounts

When OAuth is enabled, signed-in users see an OAuth section in Settings. The Connect OAuth button starts a login with the configured provider and links the returned identity to the current local account. Future OAuth logins with that provider identity sign in as the linked user. Users can disconnect linked OAuth identities from Settings, but ScreenshotSafe blocks removing the only OAuth identity from an account that does not have a password.

OAuth identities are matched by the configured provider name plus the provider's stable user subject:

```text
provider + subject
```

The subject comes from the userinfo `sub` field, or from `id` if `sub` is unavailable. Email is stored for display and optional domain filtering, but it is not used as the primary identity key.

For OAuth-created accounts in `pending` or `auto_enabled` mode, ScreenshotSafe uses the full userinfo email address as the local username when one is available. If that username already exists, it appends a numeric suffix such as `alice@example.com2`. If the provider does not return an email address, ScreenshotSafe falls back to `preferred_username`, `login`, `name`, and finally the configured provider name.

### Admin Approval And Disabling

Admins can enable, disable, or leave accounts pending from the Admin UI. Disabled and pending users cannot use password login, session-cookie auth, or API-token auth. ScreenshotSafe also prevents an admin from disabling their own account and prevents disabling or deleting the last enabled admin.

### Email Domain Restrictions

Set `allowed_email_domains` to restrict OAuth sign-in to specific email domains:

```toml
allowed_email_domains = ["example.com", "example.org"]
```

With this setting enabled, userinfo must include an allowed email address. If the provider sends `email_verified = false`, ScreenshotSafe denies access. If your provider does not expose `email_verified`, configure domain restrictions only when you trust that provider's email claims.

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
