# Testing

## Quick Commands

- Run all automated tests: `cargo test`
- Run one integration test by name: `cargo test test_name`
- Format Rust before committing backend changes: `cargo fmt`
- Run the app locally: `cargo run`

## Automated Coverage

Integration tests live in `tests/integration_tests.rs`. They build the Axum router directly, use in-memory SQLite, and create temporary storage directories for image files.

Current tests cover these broad areas:

- First-run setup and password login.
- Session cookie creation and authenticated requests.
- Login page behavior for extension-origin flows.
- Admin and account-status behavior.
- Screenshot upload/list/update/delete behavior.
- Annotation save and image rerender behavior.
- Expiry and cleanup behavior.
- API token creation/use/revocation behavior.
- OAuth account and identity edge cases where practical without an external provider.

Unit-style tests also live near implementation code where useful, such as preview dimension behavior in `src/routes/share.rs` or image helpers.

## Manual Checks

Use these when a change touches frontend, extension, Safari, OAuth provider behavior, or deployment packaging.

| Area | Check |
| --- | --- |
| Web dashboard | `cargo run`, create/login user, upload image, list and delete screenshots. |
| Web editor | Open `/screenshots/{id}/edit`, add annotations, crop if relevant, save, reload share URL. |
| Public share | Verify `/s/{share_id}`, `/s/{share_id}.png`, and `/s/{share_id}.preview.png`. |
| Chromium extension | Load `extension/` unpacked, configure server URL/token, capture visible tab, edit, upload. |
| Safari payload | Run `scripts/build-safari-extension.sh` and inspect `dist/safari-extension/`. |
| Safari wrapper | With full Xcode, run the `--xcode-project` build command and open/build the generated project. |
| OAuth | Test against the configured provider with callback URL matching deployment config. |
| Docker | Build image and run with persistent `/data` volume. |

## Risk-Based Test Guidance

- Route/auth changes: run `cargo test` and add integration tests for allowed and denied auth paths.
- DB schema/model changes: add migration assertions or integration coverage that exercises existing and new rows.
- File lifecycle changes: test originals, rendered images, and previews are created/deleted together.
- Image rendering changes: include a small fixture-style upload or helper test that proves output bytes are produced.
- Extension-only changes: manually test the browser extension flow; backend tests will not exercise capture APIs.
- Safari wrapper changes: run the Safari build script and, when wrapper files change, verify with Xcode.
- OAuth changes: automated tests can cover account-mode logic, but provider redirects and userinfo claims need manual or mocked-provider testing.

## Test Data And Isolation

- Integration tests should keep using temporary directories rather than repo-local `data/`.
- Prefer in-memory SQLite unless persistence behavior is the point of the test.
- Avoid depending on wall-clock timing when possible; set explicit `expires_at` values for expiry tests.
- Do not commit real API tokens, OAuth secrets, JWT secrets, screenshots, or generated private data.

## Known Gaps

- Browser capture APIs are not automated.
- Safari native wrapper behavior is not covered by `cargo test`.
- OAuth provider discovery, hosted redirects, and userinfo responses require manual or mocked-provider validation.
- Visual layout of the editor/dashboard is not currently screenshot-tested.
