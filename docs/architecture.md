# Architecture

ScreenshotSafe is a self-hosted screenshot capture, annotation, and sharing app. It is made of a Rust/Axum server, static web editor assets, a Chromium WebExtension, and a native Apple project containing separate Safari and share extensions.

## Backend

- `src/main.rs` loads config, creates storage directories, opens SQLite, runs migrations, prepares the JWT secret, starts expiry cleanup and hit-count flushing, and serves the Axum router.
- `src/lib.rs` defines `AppState`, registers all page/API/share routes, serves `/static`, applies CORS, and starts the trace layer.
- `src/hit_counter.rs` buffers full-image hits in memory and persists each non-empty batch in one SQLite transaction.
- `src/config.rs` loads config from CLI path, environment, `config.toml`, and defaults.
- `src/db.rs` owns the SQLite connection, schema migrations, and database access methods.
- `src/models.rs` defines persisted model types, annotations, crop rectangles, account status, screenshots, users, API tokens, and OAuth identities.
- `src/auth/` implements password hashing, token hashing, JWT session creation, and Axum auth extractors.
- `src/routes/api.rs` implements JSON/multipart API handlers and most state-changing behavior.
- `src/routes/pages.rs` implements server-rendered HTML pages.
- `src/routes/share.rs` implements public unlisted share pages and raw/preview image responses.
- `src/image_processing.rs` renders annotations/crops into images and creates social preview PNGs.
- `src/share_id.rs` generates short public IDs for share URLs.

## Request Boundaries

- Page routes use session-cookie auth through `AuthUser` or `AdminUser`.
- Most API routes use session-cookie auth and validate trusted request origins for cookie-authenticated `/api/*` calls.
- Upload and ping support bearer API tokens through `ApiOrSessionUser` or explicit token handling.
- Public share routes do not require auth. Possession of an unexpired share URL is enough to view the rendered image.

## Storage

SQLite stores users, OAuth identities, API token hashes, and screenshot metadata. Image bytes live on disk under the configured storage path.

- Originals are written under `storage.originals_path()`.
- Rendered public images are written under `storage.rendered_path()`.
- Preview images are generated next to rendered images with a `.preview.png` suffix.
- Database rows store filesystem paths for original and rendered files.
- Expiry cleanup deletes expired DB rows first and then removes original, rendered, and preview files.

## Frontend Surfaces

- `static/` contains web UI CSS and JavaScript served by the backend.
- `static/js/editor.js` is used by the server-rendered editor page.
- `extension/` is the canonical Chromium extension source. It handles capture, options, popup UI, local pre-upload editing, and server upload.
- `apple/ScreenshotSafe/Shared (Safari Extension)/Resources/` is the canonical Safari WebExtension source. It captures and edits in Safari, then sends configuration checks and uploads to the native extension handler.
- The Safari native handler reads server and API-token configuration from the shared app group and performs bearer-token requests with `URLSession`.
- `scripts/build-safari-extension.sh` builds the checked-in Xcode project. It does not copy Chromium files or invoke `safari-web-extension-converter`.

## Main Data Flows

### Setup And Login

1. `GET /setup` renders the first-run setup page if no users exist.
2. `POST /api/auth/setup` creates the first enabled admin user.
3. `POST /api/auth/login` verifies a password and sets the `session` JWT cookie.
4. OAuth sign-in/linking starts at `/api/auth/oauth/start` and returns through `/api/auth/oauth/callback`.

### Chromium Extension Authorization

1. The extension requests runtime host access for a user-entered server.
2. It generates a random state and PKCE-S256 verifier/challenge, then starts
   Chrome's interactive web-auth flow at `/extension/authorize`.
3. ScreenshotSafe preserves the authorization request through password or OAuth
   login and displays an explicit approval page.
4. Approval returns a short-lived, single-use code through Chrome's
   `chromiumapp.org` callback.
5. The extension verifies state and exchanges the code plus PKCE verifier at
   `/api/auth/extension/token`.
6. The server atomically consumes the code, stores a hash of a new API token, and
   returns the raw token once.
7. The extension stores connections locally and uses the selected server's
   bearer token for ping and upload requests.

### Screenshot Upload

1. Client posts multipart data to `POST /api/screenshots`.
2. Handler validates auth, image size, expiry, and image data.
3. Original bytes are stored on disk.
4. A rendered image and preview are written.
5. A screenshot row is inserted with owner, paths, share ID, title, source URL, annotations, crop, expiry, and timestamps.
6. Response includes metadata plus public `share_url` and `raw_url`.

### Annotation Save

1. Editor sends annotations and optional crop to `PUT /api/screenshots/{id}/annotations`.
2. Handler verifies the screenshot belongs to the authenticated user.
3. Annotation/crop JSON is stored in SQLite.
4. Original image is rerendered into a new rendered PNG and preview.
5. Public share URLs now serve the updated rendered image.

### Public Share

1. `/s/{share_id}` renders a minimal public HTML page.
2. `/s/{share_id}.png` serves the rendered PNG.
3. `/s/{share_id}.preview.png` serves or lazily creates a scaled preview.
4. Expired screenshots are treated as not found.
5. Successful origin-observed `GET` requests for the full PNG, including `304` cache validations, are buffered as hits.
6. Pending hits are added to screenshot counters every 5 seconds in one transaction without changing screenshot `updated_at`.
7. After at least 5 seconds with no hits, the first new hit wakes the background task for an immediate flush.

## Cross-Cutting Rules

- User-visible limits can come from per-user settings or server defaults.
- Admin routes must use `AdminUser`.
- Disabled and pending accounts cannot authenticate through password, session cookie, or API token.
- Bearer API tokens are stored as hashes, never as plaintext.
- Chromium extension authorization never places a permanent token in a redirect
  URL. Its temporary codes are hashed, PKCE-bound, short-lived, and single-use.
- Share IDs are public identifiers; database UUIDs remain internal API identifiers.
- Full-image hit counts are eventually consistent and can lag by one flush interval. A process crash can lose the current in-memory batch.
