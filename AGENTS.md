# ScreenshotSafe Agent Guide

Use this file as the first stop for automated coding agents and future maintainers who need to make a bounded change quickly.

## Project Shape

- `src/`: Rust/Axum backend, routes, auth, configuration, database access, and image rendering.
- `static/`: server-rendered web UI assets, including the in-browser screenshot editor used by app pages.
- `extension/`: canonical shared WebExtension source for Chromium and Safari payload generation.
- `apple/ScreenshotSafe/`: native iOS/macOS Safari app and share-extension wrapper generated around copied extension resources.
- `scripts/build-safari-extension.sh`: copies `extension/` into `dist/safari-extension/` and can regenerate the Xcode wrapper with full Xcode.
- `tests/`: Axum integration tests using in-memory SQLite and temporary image storage.

## Core Flows

- First run: `GET /setup` renders setup, `POST /api/auth/setup` creates the initial enabled admin and sets a session cookie.
- Login: password sessions use the `session` JWT cookie; OAuth can start/link through `/api/auth/oauth/start`.
- Upload: extension, scripted clients, or web UI call `POST /api/screenshots`; the server stores original and rendered PNG files and creates a DB row.
- Edit: the editor saves annotations and optional crop with `PUT /api/screenshots/{id}/annotations`; the server rerenders the public image and preview.
- Share: public unlisted links are served by `/s/{share_id}`, `/s/{share_id}.png`, and `/s/{share_id}.preview.png`.
- Cleanup: `spawn_expired_screenshot_cleanup` runs hourly and deletes expired DB rows plus backing image files.

## Commands

- Run backend locally: `cargo run`
- Run tests: `cargo test`
- Format Rust: `cargo fmt`
- Build Safari payload: `scripts/build-safari-extension.sh`
- Regenerate Xcode wrapper: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer scripts/build-safari-extension.sh --xcode-project`

## Canonical Sources

- Edit shared extension behavior in `extension/`.
- Treat `dist/safari-extension/` as generated output from `scripts/build-safari-extension.sh`.
- The Apple project contains native wrapper code and copied Safari extension resources. Prefer changing `extension/` first when behavior is shared across browser extensions.
- Backend routes are registered in `src/lib.rs`; handler implementations live mostly in `src/routes/api.rs`, `src/routes/pages.rs`, and `src/routes/share.rs`.
- SQLite schema is created and migrated in `src/db.rs`.

## Useful Docs

- Architecture map: `docs/architecture.md`
- Security and data invariants: `docs/invariants.md`
- HTTP route index: `docs/api.md`
- Test matrix: `docs/testing.md`
- Safari-specific packaging notes: `docs/safari-extension.md`

## Change Guidance

- Keep user-ownership checks close to route handlers; most screenshot APIs must only touch the authenticated user's rows.
- Keep file cleanup in step with DB deletion for originals, rendered images, and generated previews.
- If adding persistent fields, update the schema/migration block in `src/db.rs`, the model in `src/models.rs`, and the affected tests.
- If adding extension APIs, document whether they accept session cookies, bearer API tokens, or both.
- Shared screenshot URLs are unlisted rather than private; do not put sensitive data in share metadata that should require authentication.
