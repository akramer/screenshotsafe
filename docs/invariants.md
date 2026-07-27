# Invariants

These are the assumptions that should stay true as the project evolves. When changing code, preserve these or update this document with the new intended rule.

## Authentication And Authorization

- The first created user is an enabled admin.
- There must always be at least one enabled admin.
- A user cannot disable or delete their own admin access through normal admin APIs.
- Disabled and pending users cannot authenticate with password login, session-cookie auth, or bearer API-token auth.
- `AuthUser` means session cookie only.
- `ApiOrSessionUser` means bearer API token first, then session cookie.
- `AdminUser` means session-cookie auth plus `is_admin`.
- Cookie-authenticated `/api/*` requests must come from the app origin, a trusted Chrome extension origin, or a configured allowed extension origin. Safari WebExtension origins are not trusted by default.
- API tokens are stored as hashes. The raw token is only returned when created.
- Chromium extension authorization redirects contain only short-lived,
  single-use codes; permanent API tokens must not appear in redirect URLs.

## OAuth

- OAuth identities are keyed by provider plus stable subject.
- Email is display/filtering metadata, not the primary OAuth identity key.
- `link_only` rejects unknown OAuth identities.
- `pending` may create a local pending account but must not issue a session until an admin enables it.
- `auto_enabled` may create an enabled non-admin account after provider/domain checks pass.
- If domain restrictions are configured, userinfo must include an allowed email address and must not report `email_verified = false`.

## Screenshots

- Every screenshot belongs to exactly one user.
- Non-admin screenshot APIs should only operate on the authenticated user's screenshots.
- Share IDs are unique and public-facing; screenshot UUIDs are internal API identifiers.
- Public share links are unlisted, not authenticated.
- Deleted or expired screenshots should not be visible through API or share routes.
- When a screenshot is deleted, its original, rendered, and preview files should be removed when present.
- When annotations or crop change, the rendered image and preview should be regenerated from the original image.
- Full-image hits count successful, origin-observed `GET /s/{share_id}.png` requests, including successful `304` validations. Share pages, previews, `HEAD`, and unsuccessful requests do not count.
- Hit-count persistence must be batched rather than performing a write transaction for every image request.
- Persisting hit counts must not change screenshot `updated_at`, because that timestamp versions public image URLs.

## Storage

- SQLite stores metadata and file paths; image bytes are stored on disk.
- The configured storage directory must contain separate originals and rendered subdirectories.
- Preview images live beside rendered images and use the `.preview.png` suffix.
- Cleanup may tolerate already-missing files, but should warn on unexpected removal failures.
- The persistent JWT secret must remain stable across restarts. If `auth.jwt_secret` is omitted, the generated `.jwt_secret` file must be reused.

## Limits And Expiry

- The server maximum lifetime is a default for users, not a global ceiling.
- An administrator may give a user an inherited, shorter, longer, or unlimited maximum lifetime.
- A user's default expiry is inherited from the server or explicitly chosen by that user.
- Explicit per-upload expiry must not exceed the user's effective maximum; invalid requests are rejected rather than silently shortened.
- `never` is allowed only when the user's effective maximum is unlimited.
- Changing retention settings affects new uploads and explicit expiry edits, not existing screenshots.
- `expires_in` duration strings use `m`, `h`, `d`, or `w`.
- Expired screenshots should be removed by the hourly cleanup task and should also be unavailable when fetched after expiry.

## Browser Extension And Safari

- `extension/` is the canonical Chromium WebExtension source.
- Chromium stores a local list of server origins and bearer tokens, with exactly
  zero or one active upload server.
- Chromium server entries are origin-only, and non-loopback connections must use
  HTTPS.
- A screenshot draft remains bound to the server selected when it was captured.
- `apple/ScreenshotSafe/Shared (Safari Extension)/Resources/` is the canonical Safari WebExtension source.
- The checked-in Xcode project is maintained directly and must not be regenerated over with `safari-web-extension-converter`.
- Safari WebExtension code must not request server host permissions or send server requests directly.
- Safari configuration, connection checks, and uploads go through the native extension handler and app-group settings.
- Extension uploads should work with bearer API tokens and should not require a browser session cookie.

## Public Data

- Treat title, source URL, rendered image, and preview image as public to anyone with the share URL.
- Do not expose original image paths, token hashes, password hashes, or OAuth secrets through responses.
- Raw public PNG and preview URLs should serve image bytes only for valid, unexpired share IDs.
- Hit counts are owner-visible metadata and should not be displayed on public share pages.
