# Apple Multi-Server Configuration and Login

Status: Approved for implementation  
Date: 2026-07-26

## Summary

Replace the Apple app's single server URL/API-token form with a native connection manager that closely matches the Chromium extension:

- show every authorized ScreenshotSafe server;
- show the account and current connection status for each server;
- let the user select one default upload server;
- add a server by entering its domain and choosing **Log in**;
- authorize in a system-managed browser session;
- return to the app with a short-lived authorization code protected by PKCE;
- exchange the code for a revocable API token, verify it, and store it in the shared Keychain;
- keep the Safari and share extensions limited to the selected default connection.

The recommended callback mechanism for domain-based login is `ASWebAuthenticationSession` with the private-use callback URI `com.screenshotsafe:/authorize/ios` or `com.screenshotsafe:/authorize/macos`. iOS also retains a desktop-to-phone QR setup path containing the server origin and complete bearer token. The existing `screenshotsafe://configure` URL handler and manual token-entry fields are removed.

## Goals

1. Give the macOS and iOS apps feature parity with Chrome's multi-server configuration model.
2. Make browser-based authorization the primary setup path; users should not have to type or paste an API token.
3. Use the user's existing ScreenshotSafe web session, including the server's password or OAuth login flow.
4. Keep bearer tokens out of callback URLs, app-group preferences, and Safari WebExtension JavaScript.
5. Preserve Safari's current architecture: the native handler performs authenticated network requests, so the WebExtension still needs no arbitrary host permissions.
6. Retain convenient QR credential transfer so an iOS device can be configured from a desktop-authenticated server session.
7. Remove the legacy URL handler and manual-token configuration surface.

## Non-goals

- Selecting a different server for each individual upload.
- A different default server per Safari profile.
- Synchronizing server credentials through iCloud.
- Changing screenshot upload, editing, or sharing APIs.
- Replacing the server's existing login and OAuth providers.
- Making the Safari WebExtension manage credentials directly.

## Current State

### Chromium

The Chromium extension already stores a versioned connection registry containing:

- a list of normalized server origins;
- one active server origin;
- the token and account metadata for each server;
- connection status and last-check time.

Its **Log in** action runs the existing authorization-code flow with PKCE:

1. Open `GET /extension/authorize`.
2. Let the server authenticate the user and obtain consent.
3. Receive a one-time code at the browser extension's callback URI.
4. Exchange the code at `POST /api/auth/extension/token`.
5. Verify the token with `GET /api/ping`.
6. save the connection and make it active.

### Apple

The Apple app currently stores one `serverUrl`, one raw `apiToken`, and one `defaultExpiry` value in app-group `UserDefaults`. The app, Safari native handler, and share extension all load that single tuple. A `screenshotsafe://configure` URL can import a raw token.

This has three limitations:

- it cannot represent multiple authorized servers;
- the token is stored alongside ordinary preferences rather than in the Keychain;
- setup requires manual token entry, a configure link, or a QR code.

## Proposed User Experience

### Main screen

Use two sections.

**Connected servers**

Each row shows:

- a radio control on macOS or checkmark selection on iOS indicating the default;
- normalized origin, such as `https://screenshots.example.com`;
- display name and username returned by `/api/ping`;
- status: Connected, Login required, Account disabled, Unreachable, Server needs an upgrade, Server error, or Checking;
- the last check time;
- contextual actions: **Retry**, **Log in again**, and **Log out**.

Selecting a row's default control immediately changes the upload destination used by Safari and both share extensions. Changing the default does not reauthorize or revoke anything.

An empty state says: “No ScreenshotSafe servers are connected yet.”

**Add a server**

- Text field label: **ScreenshotSafe server**
- Placeholder: `screenshots.example.com`
- Primary button: **Log in**

Pressing Return from the text field performs the same action as **Log in**.

The input normalization rules match Chrome:

- infer `https://` when the scheme is omitted;
- allow `http://` only for loopback development hosts;
- reject credentials, paths, queries, and fragments;
- store only the canonical origin;
- treat origins case-insensitively and prevent duplicates.

### Successful login

After login and verification:

- add or replace the connection for that origin;
- make it the default, matching Chrome's current behavior;
- clear the add-server text field;
- show “Connected to `<origin>`. It is now used for uploads.”

### Reauthorization

**Log in again** runs a new PKCE flow for the same origin. The old token remains usable until the new token has been exchanged, verified, and stored. The app then attempts to revoke the old token.

### Logout

**Log out** first calls `DELETE /api/auth/extension/token` with that server's bearer token.

- If revocation succeeds, or the token is already invalid, remove the local connection.
- If the server cannot be reached, ask whether to forget the local connection anyway and explain that the user should revoke the old token from the server later.
- If the removed server was the default, choose the first connected server, then the first remaining server. If none remain, clear the default.

### Refresh behavior

Refresh all connections when the screen appears, when the user explicitly refreshes, and once per minute while the screen remains in the foreground. Do not run a permanent background poll.

### Existing expiry and QR controls

Keep **Default expiry** as a global upload preference below the connection sections. It is independent of which server is selected, as it is today.

Keep an iOS **Scan setup QR code** action next to the add-server form. The user creates an API token while signed into ScreenshotSafe on a desktop, then scans the one-time QR display with the iOS app. The payload contains the canonical server origin and the complete bearer token.

The scanner applies the same origin normalization and HTTPS requirements as typed input, verifies the credential with `/api/ping`, stores it directly in the shared Keychain, creates or replaces the registry entry, and makes the server the default. Show the recognized server origin and account before reporting success.

Remove the API-token text field. QR scanning is the only direct credential-import mechanism.

## Authorization Design

### Why `ASWebAuthenticationSession`

`ASWebAuthenticationSession` is the preferred Apple API for authenticating through a web service. It provides a browser-owned authentication surface, shares the appropriate website session, and delivers the callback only to the authentication session that initiated it—even if another app has registered the same callback scheme. On macOS it uses a supporting default browser or Safari; on iOS it presents a secure browser view.

This is preferable to:

- **Opening a normal browser plus relying only on app-delegate URL delivery:** less tightly associated with the initiating attempt and more lifecycle state to recover.
- **A loopback HTTP listener:** workable on macOS but not a good cross-platform iOS design.
- **Universal links:** stronger app ownership, but they require a fixed HTTPS domain, associated-domains entitlements, and an `apple-app-site-association` deployment. That would add a central ScreenshotSafe service dependency to otherwise self-hosted servers.
- **Polling/device flow:** adds server state and latency without improving this first-party app flow.

### Callback URI

Replace the registered `screenshotsafe` URL scheme with `com.screenshotsafe` in the iOS and macOS app targets. Remove the old configure-URL handling from the macOS app delegate and iOS scene delegate.

Use exact, platform-specific redirect URIs:

- iOS: `com.screenshotsafe:/authorize/ios`
- macOS: `com.screenshotsafe:/authorize/macos`

Using the bundle identifier as a reverse-domain private-use scheme follows native-app authorization guidance and avoids overloading the older generic scheme. The single slash is intentional for a private-use scheme without a URI authority.

The app initializes `ASWebAuthenticationSession` with callback scheme `com.screenshotsafe`, retains the session strongly until completion, supplies the current app window as its presentation anchor, and leaves `prefersEphemeralWebBrowserSession` false so existing server login cookies can be reused.

The macOS project currently targets macOS 10.14. This proposal raises the deployment target to macOS 10.15 rather than adding a deprecated `SFAuthenticationSession` fallback.

### Protocol

```mermaid
sequenceDiagram
    actor User
    participant App as "ScreenshotSafe app"
    participant Browser as "System authentication session"
    participant Server as "Selected ScreenshotSafe server"
    participant Keychain as "Shared Keychain"

    User->>App: Enter domain and choose Log in
    App->>App: Normalize origin; create state and PKCE verifier/challenge
    App->>Browser: Open /extension/authorize
    Browser->>Server: GET authorize request
    Server->>Browser: Existing login/OAuth and consent page
    User->>Browser: Approve
    Browser->>Server: POST authorization approval
    Server-->>Browser: Redirect with one-time code and state
    Browser-->>App: com.screenshotsafe:/authorize/{platform}
    App->>App: Verify exact callback URI and state
    App->>Server: POST /api/auth/extension/token with code and verifier
    Server-->>App: API token and account metadata
    App->>Server: GET /api/ping with bearer token
    Server-->>App: Verified account metadata
    App->>Keychain: Store token
    App->>App: Store metadata and select default
```

The authorization request continues to use:

- a cryptographically random state value;
- a cryptographically random PKCE verifier;
- `S256` challenge method;
- a five-minute, single-use authorization code;
- the exact same redirect URI during authorization and exchange.

The callback carries only `code`, `state`, or `error`; it never carries the API token. The app exchanges the code directly with the selected origin using `URLSession`.

Only one login attempt may be active at a time. Starting another cancels the first and discards its in-memory state and verifier.

Before opening the authentication session, make an unauthenticated `GET` request to the full authorization URL with redirect following disabled. A successful page or login redirect means the server supports the callback. A `404`, or the existing “invalid extension redirect URI” response, is reported as an outdated server; transport and TLS errors are reported as unreachable. This provides the same early compatibility feedback as Chrome without requiring WebExtension host permissions.

### QR credential transfer

The QR code is a second, iOS-focused configuration path. It intentionally contains a complete bearer credential so a user who is already signed into the server on a desktop does not also have to authenticate on the phone.

Encode a versioned UTF-8 JSON payload rather than a launchable URL:

```json
{
  "type": "screenshotsafe_configuration",
  "version": 1,
  "server_url": "https://screenshots.example.com",
  "token": "sss_..."
}
```

Do not register this payload as a URL handler or place it on the pasteboard. The in-app scanner is its only consumer.

After scanning:

1. Parse the payload and require the exact type and supported version.
2. Normalize and validate `server_url` with the same rules as typed input.
3. Require a plausibly formatted ScreenshotSafe token and reject oversized or unexpected data.
4. Call `/api/ping` with the bearer token.
5. On success, store the token directly in a new shared-Keychain credential, save the returned account metadata, and make the connection the default.
6. On failure, store nothing and keep the scanner available for a retry.

Scanning a connection for an origin that already exists is treated as reauthorization. Keep the old credential until the scanned credential has passed `/api/ping` and the registry transaction has committed; then attempt to revoke the old token. If the scanned token exactly matches the current Keychain token, refresh metadata without replacing or revoking it.

The QR is a bearer-secret display. The server page must state that anyone who can scan or photograph it can use the token, show it only in the one-time new-token result, and provide an immediate **Revoke token** action. The app must never log the payload, serialize the token into app-group defaults, or include it in error reporting.

### Server changes

Reuse the existing routes and authorization-code database table for compatibility with Chrome:

- `GET /extension/authorize`
- `POST /api/auth/extension/authorize`
- `POST /api/auth/extension/token`
- `DELETE /api/auth/extension/token`

Generalize redirect validation to accept exactly:

- the existing validated Chromium callback pattern;
- `com.screenshotsafe:/authorize/ios`;
- `com.screenshotsafe:/authorize/macos`.

Do not accept arbitrary custom schemes or arbitrary paths.

The authorization page should derive client-specific copy from the validated redirect:

- heading: “Connect ScreenshotSafe for iOS” or “Connect ScreenshotSafe for macOS”;
- consent text: “Allow the ScreenshotSafe app to upload screenshots as …?”;
- default token label: `ScreenshotSafe for iOS — <date>` or `ScreenshotSafe for macOS — <date>`.

Chrome copy and its callback behavior remain unchanged.

Older servers will reject the Apple callback URI and generate the obsolete configure-URL QR format. The app should report: “This ScreenshotSafe server needs an update before the app can be configured.” Legacy configure links and manual token entry are not compatibility fallbacks.

## Native Data Model

Store non-secret registry metadata as one versioned property-list or Codable JSON value in app-group `UserDefaults`:

```text
ConnectionRegistry
  schemaVersion: 2
  defaultConnectionID: UUID?
  connections:
    - id: UUID
      origin: String
      credentialID: UUID
      displayName: String
      username: String
      lastStatus: ConnectionStatus
      lastCheckedAt: Date?
  defaultExpiry: String
```

Do not store bearer tokens or PKCE verifiers in this value.

Store each token as a Keychain generic-password item:

```text
service: com.screenshotsafe.server-token
account: <credential UUID>
access group: group.com.screenshotsafe.safari
value: <bearer token bytes>
synchronizable: false
accessibility: when unlocked, this device only
```

Use the shared application group as the Keychain access group so the containing app, Safari native handler, and share extensions can read the selected token. On macOS, use the data-protection Keychain when required for application-group sharing.

`ScreenshotSafeSettingsStore.load()` becomes a compatibility adapter named along the lines of `loadActiveSettings()`: it reads the registry's default connection, fetches that connection's token from the Keychain, and returns the existing upload settings shape. This keeps upload and native-message call sites small and ensures extensions receive only the active connection.

The Safari WebExtension continues to receive only:

- whether an active connection is configured;
- active server URL;
- default expiry.

It never receives a token or the complete connection list.

## Consistency and Failure Handling

Keychain and `UserDefaults` cannot be updated in one transaction. Use this order when adding a connection:

1. Exchange and verify the new token.
2. Write it under a new credential UUID in the Keychain.
3. Write registry metadata that points the connection at the new credential UUID, plus the new default ID.
4. If step 3 fails, delete the newly written Keychain item and keep the old registry.
5. After a reconnect commits, revoke and remove the superseded credential.

On load:

- ignore malformed or duplicate origins;
- mark a connection with a missing Keychain item as **Login required**;
- clear a default ID that does not exist;
- remove orphaned token items during a repair pass;
- never silently choose an unverified token as connected.

The native handler and share extensions load active settings for every user-initiated request, so cross-process notifications are not required. In-process notifications remain useful for updating the open app UI.

## Legacy Configuration Removal

Do not migrate a legacy `serverUrl` or `apiToken` into the new registry. Existing users start with an empty connection list and configure it again through browser login or a newly generated desktop QR.

On first launch of the new schema:

1. Preserve the non-secret `defaultExpiry` preference.
2. If a legacy origin and token exist, make a best-effort authenticated request to revoke that token.
3. Delete the legacy `serverUrl` and `apiToken` values regardless of whether revocation succeeds.
4. If revocation failed, tell the user that the old token may still be active and can be revoked from that server's API-token settings.
5. Create an empty versioned registry.

Remove all parsing of `screenshotsafe://configure` URLs, the `screenshotsafe` URL-scheme registration, and API-token fields from the Apple app. Retain the iOS QR scanner and camera permission. The scanner accepts only the versioned configuration payload and sends the embedded credential through verification and Keychain storage without invoking the browser authorization coordinator.

The server continues to support ordinary API tokens for scripted clients. Remove the launchable `configure_url` from its token-management response and the **Open ScreenshotSafe** link from the page. Keep `configure_qr_svg`, but generate it from the versioned JSON payload containing `server_url` and the newly created raw token. Label it **Scan with the ScreenshotSafe iOS app** and display it only alongside the one-time raw-token result.

## Security Properties

- Passwords and SSO credentials are entered only into the server page shown by the system authentication session.
- The authorization callback contains a single-use code, not a bearer token.
- PKCE prevents an intercepted custom-scheme code from being redeemed without the app's verifier.
- State binds the callback to the active attempt.
- The app compares scheme, path, state, and selected server origin before exchanging.
- Server redirect validation is an exact allowlist, not a prefix match.
- Authorization codes are hashed at rest, expire after five minutes, and are consumed atomically, preserving current server behavior.
- Tokens are device-local Keychain items and are not synchronized through iCloud.
- A setup QR is explicitly treated as a bearer-secret display; scanning or photographing it grants the same access as copying the token.
- The WebExtension JavaScript never receives raw tokens and still has no arbitrary server host permission.
- Reauthorization does not destroy the last working token before the replacement is committed.

## Implementation Shape

Suggested native types:

- `ScreenshotSafeConnection`
- `ScreenshotSafeConnectionRegistry`
- `ScreenshotSafeConnectionStore`
- `ScreenshotSafeTokenStore`
- `ScreenshotSafeServerURLNormalizer`
- `ScreenshotSafeAuthorizationCoordinator`
- existing `ScreenshotSafeUploadClient`, updated to accept resolved active settings

Suggested phases:

1. Generalize and test the server authorization flow for the two Apple callback URIs.
2. Add the registry, shared Keychain store, normalization, and legacy cleanup.
3. Add the authorization coordinator using `ASWebAuthenticationSession`.
4. Replace the macOS configuration form with the connected-server list and add-server section.
5. Replace the iOS configuration form with the equivalent native list.
6. Remove the legacy configure scheme and manual-token UI; change the iOS scanner and server QR to the versioned token-bearing format.
7. Update Safari/share-extension active-settings reads.
8. Update Safari packaging documentation and complete signed-device testing.

## Testing

### Server automated tests

- Both exact Apple redirect URIs are accepted.
- Variations in scheme, path, host/authority, query, and fragment are rejected.
- Existing Chromium redirects remain accepted.
- Apple consent copy and token labels are correct.
- State and PKCE failures are rejected.
- A code cannot be exchanged twice.
- Exchange rejects a redirect URI different from the authorized URI.
- Token revocation still deletes only the authenticated token.

### Native automated tests

- URL normalization matches Chrome, including HTTPS and loopback cases.
- Registry decoding sanitizes duplicates and invalid defaults.
- Legacy cleanup is idempotent, preserves default expiry, and never imports the old token.
- QR parsing requires the supported payload type/version, validates the origin and token bounds, and stores nothing before `/api/ping` succeeds.
- QR imports write the token only to the shared Keychain and never to app-group metadata.
- Tokens are absent from serialized app-group metadata.
- Callback parsing requires the exact platform path and matching state.
- Cancel, missing code, wrong state, exchange failure, failed ping, and failed Keychain writes leave no partial connection.
- Reauthorization preserves the old connection until commit.
- Removing the default chooses the documented fallback.

Add a small XCTest target if practical; the current Apple project has no test target.

### Manual tests

Run on both macOS and iOS:

- password login;
- each configured server-side OAuth provider;
- already-authenticated browser session;
- cancellation before and after server login;
- app cold start and app already running;
- two different servers and switching the default;
- revoked token and disabled account;
- unreachable and outdated server;
- loopback development server;
- token-bearing QR setup on iOS from a desktop-created API token;
- rejection of old `screenshotsafe://configure` QR payloads, malformed JSON, invalid origins, and invalid tokens;
- Safari capture/upload after switching;
- native share-extension upload after switching;
- upgrade from an existing configure-link/QR installation removes the old local credential and requires browser login or a newly generated desktop QR;
- signed builds verifying shared Keychain access from every target.

## Acceptance Criteria

- The app lists all locally authorized ScreenshotSafe servers and clearly identifies one default.
- A domain plus **Log in** completes authorization without copying an API token.
- Login returns to the initiating app cleanly on both platforms.
- A successful connection immediately works in Safari and the share extension.
- Switching the default changes subsequent uploads without reauthorization.
- Logging out revokes the server token when reachable.
- Existing single-server users are prompted to use browser login or scan a newly generated desktop QR after upgrade.
- The app contains no configure-link or manual-token setup path.
- The iOS QR scanner can configure a server and full token created from a desktop session without requiring web login on the phone.
- QR-imported tokens are verified before being stored directly in the shared Keychain.
- Tokens are stored in the shared Keychain and never exposed to WebExtension JavaScript.
- Chrome's existing connection and authorization flows continue to pass.

## Approved Product Decisions

The approved design makes the following product choices:

1. A newly logged-in server becomes the default, matching Chrome.
2. The default is global across Safari profiles and share extensions.
3. Default expiry remains global rather than per server.
4. Existing Apple credentials are not migrated; users use browser login or a newly generated desktop QR after upgrading.
5. The old configure URL scheme and manual-token UI are removed.
6. The iOS QR scanner remains and accepts a versioned payload containing the server origin and complete token.
7. A QR-imported credential is verified and stored without requiring web login on the phone.
8. General-purpose server API tokens remain available for scripted clients; their one-time creation result can be scanned directly into the iOS app.
9. macOS minimum version moves from 10.14 to 10.15.
10. The app uses `com.screenshotsafe:/authorize/{platform}` as its only registered callback scheme.

## References

- [Apple: ASWebAuthenticationSession](https://developer.apple.com/documentation/authenticationservices/aswebauthenticationsession)
- [Apple: Sharing access to Keychain items among apps](https://developer.apple.com/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps)
- [Apple: App Groups entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.application-groups)
- [IETF RFC 8252: OAuth 2.0 for Native Apps](https://www.rfc-editor.org/info/rfc8252/)
- [ScreenshotSafe Safari extension architecture](safari-extension.md)
