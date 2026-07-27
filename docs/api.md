# API Route Index

Routes are registered in `src/lib.rs`. Handlers live mainly in `src/routes/api.rs`, `src/routes/pages.rs`, and `src/routes/share.rs`.

## Auth Modes

- Public: no authentication required.
- Session: `session` JWT cookie required through `AuthUser`.
- Admin session: `session` JWT cookie required and user must be admin through `AdminUser`.
- API token or session: `Authorization: Bearer sss_...` accepted first, then session cookie through `ApiOrSessionUser`.
- Optional session: route can inspect a session if present but does not require one.

Cookie-authenticated `/api/*` calls are origin-checked. Chrome extension origins are accepted for the Chromium client, and additional origins can be configured with `allowed_extension_origins`. Safari uses native bearer-token requests and its WebExtension origin is not trusted for cookie authentication.

## Page Routes

| Method | Path | Auth | Purpose |
| --- | --- | --- | --- |
| `GET` | `/` | Session | Dashboard listing the user's screenshots. |
| `GET` | `/setup` | Public | First-run setup page when no users exist. |
| `GET` | `/login` | Optional session | Login page. |
| `GET` | `/extension/authorize` | Optional session | Interactive Chromium-extension or Apple-app authorization and approval page. |
| `GET` | `/screenshots/{id}/edit` | Session | Screenshot editor page for an owned screenshot. |
| `GET` | `/settings` | Session | User settings, API tokens, password, OAuth linking. |
| `GET` | `/admin` | Admin session | Admin user management page. |
| `GET` | `/admin/users/{id}` | Admin session | Admin edit page for a user. |

## Public Share Routes

| Method | Path | Auth | Purpose |
| --- | --- | --- | --- |
| `GET` | `/s/{share_id}` | Public | Public unlisted share page. |
| `GET` | `/s/{share_id}.png` | Public | Public rendered PNG. Successful `200` responses and `304` cache validations are buffered as full-image hits. |
| `GET` | `/s/{share_id}.preview.png` | Public | Public preview PNG, generated lazily when needed. |
| `GET` | `/favicon.ico` | Public | Extension favicon served as site favicon. |
| `GET` | `/static/*` | Public | Static CSS and JavaScript assets. |

## Authentication API

| Method | Path | Auth | Body / Query | Side Effects |
| --- | --- | --- | --- | --- |
| `GET` | `/api/ping` | API token or session | None | Returns server/auth status for clients. |
| `POST` | `/api/auth/setup` | Public before setup | JSON: `username`, `password`, optional `display_name` | Creates first enabled admin and sets session cookie. |
| `POST` | `/api/auth/login` | Public | JSON: `username`, `password` | Sets session cookie. |
| `POST` | `/api/auth/logout` | Public | None | Clears session cookie. |
| `GET` | `/api/auth/oauth/start` | Optional session | Query: optional `link=true` | Redirects to configured OAuth provider. |
| `GET` | `/api/auth/oauth/callback` | Public | OAuth callback query | Links identity or signs in/creates user depending on account mode. |
| `DELETE` | `/api/auth/oauth/identities/{id}` | Session | None | Disconnects one linked OAuth identity when allowed. |
| `POST` | `/api/auth/extension/authorize` | Session | JSON: validated Chrome or Apple `redirect_uri`, `state`, PKCE challenge/method, optional `token_label` | Creates a five-minute, single-use authorization code after user approval. Blank labels identify Chrome, iOS, or macOS from the exact callback URI. |
| `POST` | `/api/auth/extension/token` | Public | JSON: authorization `code`, PKCE verifier, `redirect_uri` | Atomically consumes the code and returns a new extension API token once. |
| `DELETE` | `/api/auth/extension/token` | API token | None | Revokes the bearer token used for the request. |
| `PUT` | `/api/auth/password` | Session | JSON: current/new password fields | Changes password hash. |

## Admin API

| Method | Path | Auth | Body | Side Effects |
| --- | --- | --- | --- | --- |
| `GET` | `/api/admin/users` | Admin session | None | Lists users and admin-visible account details. |
| `POST` | `/api/admin/users` | Admin session | JSON: username, password, optional display/admin flag/limits | Creates a user. |
| `PATCH` | `/api/admin/users/{id}` | Admin session | JSON: optional password, account status, and/or limits | Updates user password, status, and limits. |
| `DELETE` | `/api/admin/users/{id}` | Admin session | None | Deletes a user and associated DB rows/files when allowed. |
| `GET`, `PATCH` | `/api/admin/server-retention` | Admin session | Optional server default expiry and default maximum seconds | Reads or updates server retention defaults. |
| `GET`, `PATCH` | `/api/admin/users/{id}/retention` | Admin session | Maximum mode (`inherit`, `duration`, or `unlimited`) and optional seconds | Reads or updates the administrator-owned user maximum. |

## User Retention API

| Method | Path | Auth | Body | Side Effects |
| --- | --- | --- | --- | --- |
| `GET` | `/api/user/retention` | API token or session | None | Returns configured and effective retention settings. |
| `PUT` | `/api/user/retention` | API token or session | Default mode (`inherit`, `duration`, or `never`) and optional seconds | Updates the user's default for new screenshots. |

`GET /api/ping` also includes the resolved retention policy so connected clients
can label the account default and hide choices that exceed the effective user
maximum.

## Screenshot API

| Method | Path | Auth | Body / Query | Side Effects |
| --- | --- | --- | --- | --- |
| `POST` | `/api/screenshots` | API token or session | Multipart `image`; optional `title`, `source_url`, `expires_in`, `image_dpi` | Stores original, rendered image, preview, and DB row. |
| `GET` | `/api/screenshots` | Session | Query: optional `page`, `per_page` | Lists screenshots owned by the session user. |
| `PATCH` | `/api/screenshots/{id}` | Session | JSON: optional `title`, `source_url`, `visibility`, `expires_in`, `image_dpi` | Updates screenshot metadata for an owned screenshot and rerenders if DPI changes. |
| `DELETE` | `/api/screenshots/{id}` | Session | None | Deletes an owned screenshot and backing image files. |
| `PUT` | `/api/screenshots/{id}/annotations` | Session | JSON: `annotations`, optional `crop` | Saves annotations/crop and regenerates rendered image and preview. |
| `GET` | `/api/screenshots/{id}/original` | Session | None | Serves original image bytes for an owned screenshot. |
| `GET` | `/api/screenshots/{id}/preview` | Session | None | Serves or creates preview image bytes for an owned screenshot. |

Authenticated screenshot-list results include the persisted `hit_count`. Active counts normally lag by no more than the 5-second flush interval; the first hit after at least 5 seconds of inactivity triggers an immediate background flush.

Upload responses include the screenshot metadata, actual `expires_at`, and public
`share_url`, `raw_url`, and `share_id`.

## API Token API

| Method | Path | Auth | Body | Side Effects |
| --- | --- | --- | --- | --- |
| `POST` | `/api/auth/tokens` | Session | JSON: `label` | Creates an API token and returns the raw token once, plus an iOS setup QR containing a versioned JSON payload with the server origin and complete token. |
| `GET` | `/api/auth/tokens` | Session | None | Lists token metadata for the session user. |
| `DELETE` | `/api/auth/tokens/{id}` | Session | None | Revokes one token owned by the session user. |

## Notes For New Routes

- Choose the narrowest auth extractor that matches the client surface.
- If a route can be used by the extension without a web session, document bearer-token support here.
- Extension authorization codes are hashed, expire after five minutes, are bound
  to a validated Chromium or exact platform-specific Apple callback and PKCE
  challenge, and may be consumed only once.
- Keep owner checks explicit for any `{id}` route that touches screenshots, tokens, or identities.
- If a route writes files and DB rows, document the side effects so cleanup/test expectations stay visible.
