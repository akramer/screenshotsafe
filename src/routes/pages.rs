use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect},
};

use crate::auth::middleware::{AdminUser, AuthUser, MaybeAuthUser};
use crate::{AppError, SharedState};

const FAVICON_LINK: &str = r#"<link rel="icon" type="image/png" href="/favicon.ico">"#;

/// Dashboard page — lists all screenshots for the logged-in user.
/// Redirects to /setup if no users exist, or /login if not authenticated.
pub async fn dashboard(
    State(state): State<SharedState>,
    headers: HeaderMap,
    user: MaybeAuthUser,
) -> crate::Result<impl IntoResponse> {
    // If no users exist, redirect to setup
    if state.db.user_count()? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }

    let user = match user.0 {
        Some(u) => u,
        None => return Ok(Redirect::to("/login").into_response()),
    };

    let screenshots = state.db.list_screenshots_for_user(&user.id, 50, 0)?;

    let base_url = crate::routes::get_base_url(&state.config.server.public_url, &headers);
    let admin_link = if user.is_admin {
        r#"<a href="/admin" class="btn btn-sm btn-outline">Admin</a>"#
    } else {
        ""
    };

    let screenshot_cards: String = if screenshots.is_empty() {
        r#"<div class="empty-state">
            <div class="empty-icon">📸</div>
            <h2>No screenshots yet</h2>
            <p>Upload your first screenshot using the API or Chrome extension.</p>
        </div>"#
            .to_string()
    } else {
        screenshots
            .iter()
            .map(|s| {
                let title = s.display_title();
                let share_url = format!("{}/s/{}", base_url, s.share_id);
                let raw_url = format!("{}/s/{}.png", base_url, s.share_id);
                let expired_class = if s.is_expired() { " expired" } else { "" };
                let expires_info = s.expires_at
                    .map(|e| format!("<span class=\"meta-item\">Expires: {}</span>", e.format("%b %d, %Y")))
                    .unwrap_or_default();
                format!(
                    r#"<div class="screenshot-card{}">
                        <a href="/screenshots/{}/edit" class="card-image-link">
                            <img src="{}" alt="{}" loading="lazy" />
                        </a>
                        <div class="card-info">
                            <h3 class="card-title">{}</h3>
                            <div class="card-meta">
                                <span class="meta-item">{}</span>
                                {}
                            </div>
                            <div class="card-actions">
                                <a href="{}" class="btn btn-sm" target="_blank">Share</a>
                                <button class="btn btn-sm btn-outline copy-btn" data-url="{}">Copy Link</button>
                                <button class="btn btn-sm btn-danger delete-btn" data-id="{}">Delete</button>
                            </div>
                        </div>
                    </div>"#,
                    expired_class,
                    s.id,
                    raw_url,
                    html_escape(title),
                    html_escape(title),
                    s.created_at.format("%b %d, %Y %H:%M"),
                    expires_info,
                    share_url,
                    share_url,
                    s.id,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>ScreenshotSafe — Dashboard</title>
    {favicon}
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <nav class="navbar">
        <a href="/" class="nav-brand">📸 ScreenshotSafe</a>
        <div class="nav-right">
            <span class="nav-user">{display_name}</span>
            {admin_link}
            <a href="/settings" class="btn btn-sm btn-outline">Settings</a>
            <button id="logout-btn" class="btn btn-sm btn-outline">Logout</button>
        </div>
    </nav>
    <main class="container">
        <div class="page-header">
            <h1>Your Screenshots</h1>
        </div>
        <div class="screenshot-grid">
            {screenshot_cards}
        </div>
    </main>
    <script>
        document.getElementById('logout-btn')?.addEventListener('click', async () => {{
            await fetch('/api/auth/logout', {{ method: 'POST' }});
            window.location.href = '/login';
        }});

        document.querySelectorAll('.copy-btn').forEach(btn => {{
            btn.addEventListener('click', () => {{
                navigator.clipboard.writeText(btn.dataset.url);
                btn.textContent = 'Copied!';
                setTimeout(() => btn.textContent = 'Copy Link', 2000);
            }});
        }});

        document.querySelectorAll('.delete-btn').forEach(btn => {{
            btn.addEventListener('click', async () => {{
                if (!confirm('Delete this screenshot?')) return;
                const resp = await fetch(`/api/screenshots/${{btn.dataset.id}}`, {{ method: 'DELETE' }});
                if (resp.ok) window.location.reload();
            }});
        }});
    </script>
</body>
</html>"#,
        favicon = FAVICON_LINK,
        display_name = html_escape(&user.display_name),
        admin_link = admin_link,
        screenshot_cards = screenshot_cards,
    );

    Ok(Html(html).into_response())
}

/// Setup page — shown on first run when no users exist.
pub async fn setup_page(State(state): State<SharedState>) -> crate::Result<impl IntoResponse> {
    if state.db.user_count()? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }

    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>ScreenshotSafe — Setup</title>
    <link rel="icon" type="image/png" href="/favicon.ico">
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <div class="auth-container">
        <div class="auth-card">
            <div class="auth-header">
                <h1>📸 ScreenshotSafe</h1>
                <p>Create your admin account to get started.</p>
            </div>
            <form id="setup-form">
                <div class="form-group">
                    <label for="username">Username</label>
                    <input type="text" id="username" name="username" required autocomplete="username">
                </div>
                <div class="form-group">
                    <label for="password">Password</label>
                    <input type="password" id="password" name="password" required minlength="8" autocomplete="new-password">
                    <span class="form-hint">Minimum 8 characters</span>
                </div>
                <div class="form-group">
                    <label for="display_name">Display Name (optional)</label>
                    <input type="text" id="display_name" name="display_name" autocomplete="name">
                </div>
                <div id="error-msg" class="error-msg" style="display:none"></div>
                <button type="submit" class="btn btn-primary btn-full">Create Account</button>
            </form>
        </div>
    </div>
    <script>
        document.getElementById('setup-form').addEventListener('submit', async (e) => {
            e.preventDefault();
            const errEl = document.getElementById('error-msg');
            errEl.style.display = 'none';

            const body = {
                username: document.getElementById('username').value,
                password: document.getElementById('password').value,
                display_name: document.getElementById('display_name').value || undefined,
            };

            const resp = await fetch('/api/auth/setup', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
            });

            if (resp.ok) {
                window.location.href = '/';
            } else {
                const data = await resp.json();
                errEl.textContent = data.error || 'Setup failed';
                errEl.style.display = 'block';
            }
        });
    </script>
</body>
</html>"#;

    Ok(Html(html).into_response())
}

/// Login page.
pub async fn login_page(
    State(state): State<SharedState>,
    user: MaybeAuthUser,
) -> crate::Result<impl IntoResponse> {
    if state.db.user_count()? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    if user.0.is_some() {
        return Ok(Redirect::to("/").into_response());
    }

    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>ScreenshotSafe — Login</title>
    <link rel="icon" type="image/png" href="/favicon.ico">
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <div class="auth-container">
        <div class="auth-card">
            <div class="auth-header">
                <h1>📸 ScreenshotSafe</h1>
                <p>Sign in to manage your screenshots.</p>
            </div>
            <form id="login-form">
                <div class="form-group">
                    <label for="username">Username</label>
                    <input type="text" id="username" name="username" required autocomplete="username">
                </div>
                <div class="form-group">
                    <label for="password">Password</label>
                    <input type="password" id="password" name="password" required autocomplete="current-password">
                </div>
                <div id="error-msg" class="error-msg" style="display:none"></div>
                <button type="submit" class="btn btn-primary btn-full">Sign In</button>
            </form>
        </div>
    </div>
    <script>
        document.getElementById('login-form').addEventListener('submit', async (e) => {
            e.preventDefault();
            const errEl = document.getElementById('error-msg');
            errEl.style.display = 'none';

            const body = {
                username: document.getElementById('username').value,
                password: document.getElementById('password').value,
            };

            const resp = await fetch('/api/auth/login', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
            });

            if (resp.ok) {
                window.location.href = '/';
            } else {
                errEl.textContent = 'Invalid username or password';
                errEl.style.display = 'block';
            }
        });
    </script>
</body>
</html>"#;

    Ok(Html(html).into_response())
}

/// Editor page for a screenshot.
pub async fn editor_page(
    State(state): State<SharedState>,
    headers: HeaderMap,
    AuthUser(user): AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> crate::Result<impl IntoResponse> {
    let screenshot = state
        .db
        .get_screenshot_by_id(&id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.user_id != user.id {
        return Err(AppError::NotFound);
    }

    let annotations_json = serde_json::to_string(&screenshot.annotations).unwrap_or("[]".into());
    let crop_json = screenshot
        .crop_rect
        .as_ref()
        .map(|c| serde_json::to_string(c).unwrap())
        .unwrap_or("null".into());
    let base_url = crate::routes::get_base_url(&state.config.server.public_url, &headers);
    let share_url = format!("{}/s/{}", base_url, screenshot.share_id);
    let raw_url = format!("{}/s/{}.png", base_url, screenshot.share_id);
    let image_dpi = if screenshot.image_dpi.fract().abs() < f64::EPSILON {
        format!("{:.0}", screenshot.image_dpi)
    } else {
        format!("{:.1}", screenshot.image_dpi)
    };
    let expiration_keep_label = screenshot
        .expires_at
        .map(|d| format!("Keep current ({})", d.format("%b %d, %Y %H:%M UTC")))
        .unwrap_or_else(|| "Keep current (never)".to_string());
    let expires_never_selected = if screenshot.expires_at.is_none() {
        "selected"
    } else {
        ""
    };

    let html = EDITOR_TEMPLATE
        .replace("{{TITLE}}", &html_escape(screenshot.display_title()))
        .replace(
            "{{TITLE_ESCAPED}}",
            &html_escape(screenshot.title.as_deref().unwrap_or("")),
        )
        .replace(
            "{{SOURCE_URL}}",
            &html_escape(screenshot.source_url.as_deref().unwrap_or("")),
        )
        .replace(
            "{{SOURCE_URL_HREF}}",
            &screenshot
                .source_url
                .as_deref()
                .filter(|url| is_safe_external_url(url))
                .map(|url| html_escape(url.trim()))
                .unwrap_or_default(),
        )
        .replace(
            "{{SOURCE_LINK_HIDDEN}}",
            if screenshot
                .source_url
                .as_deref()
                .map(is_safe_external_url)
                .unwrap_or(false)
            {
                ""
            } else {
                " hidden"
            },
        )
        .replace(
            "{{VIS_UNLISTED}}",
            if screenshot.visibility == "unlisted" || screenshot.visibility == "public" {
                "selected"
            } else {
                ""
            },
        )
        .replace(
            "{{VIS_PRIVATE}}",
            if screenshot.visibility == "private" {
                "selected"
            } else {
                ""
            },
        )
        .replace(
            "{{EXPIRATION_KEEP_LABEL}}",
            &html_escape(&expiration_keep_label),
        )
        .replace("{{EXPIRES_NEVER_SELECTED}}", expires_never_selected)
        .replace("{{SHARE_URL}}", &share_url)
        .replace("{{RAW_URL}}", &raw_url)
        .replace("{{ID}}", &screenshot.id.to_string())
        .replace("{{ANNOTATIONS}}", &annotations_json)
        .replace("{{CROP}}", &crop_json)
        .replace("{{IMAGE_DPI}}", &image_dpi);

    Ok(Html(html).into_response())
}

const EDITOR_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Edit — {{TITLE}}</title>
    <link rel="icon" type="image/png" href="/favicon.ico">
    <link rel="stylesheet" href="/static/css/style.css">
    <link rel="stylesheet" href="/static/css/editor.css?v=dpi-edit-1">
</head>
<body>
    <nav class="navbar">
        <a href="/" class="nav-brand">📸 ScreenshotSafe</a>
        <div class="nav-right">
            <a href="/" class="btn btn-sm btn-outline">← Dashboard</a>
        </div>
    </nav>
    <main class="editor-container">
        <div class="editor-toolbar" id="toolbar">
            <div class="tool-group">
                <button class="tool-btn active" data-tool="select" title="Select">
                    <span class="tool-icon">↖</span>
                </button>
                <button class="tool-btn" data-tool="redact" title="Redact (black rectangle)">
                    <span class="tool-icon">■</span>
                </button>
                <button class="tool-btn" data-tool="rect" title="Rectangle">
                    <span class="tool-icon">□</span>
                </button>
                <button class="tool-btn" data-tool="arrow" title="Arrow">
                    <span class="tool-icon">↗</span>
                </button>
                <button class="tool-btn" data-tool="line" title="Line">
                    <span class="tool-icon">─</span>
                </button>
                <button class="tool-btn" data-tool="text" title="Text">
                    <span class="tool-icon">T</span>
                </button>
                <button class="tool-btn" data-tool="crop" title="Crop">
                    <span class="tool-icon">✂</span>
                </button>
                <div style="width: 1px; height: 24px; background: var(--border); margin: 0 4px;"></div>
                <button class="tool-btn" id="zoom-in-btn" title="Zoom In (Scroll Up)">
                    <span class="tool-icon">🔍+</span>
                </button>
                <button class="tool-btn" id="zoom-out-btn" title="Zoom Out (Scroll Down)">
                    <span class="tool-icon">🔍-</span>
                </button>
                <button class="tool-btn" id="zoom-fit-btn" title="Zoom Fit">
                    <span class="tool-icon">🖥</span>
                </button>
            </div>
            <div class="tool-group">
                <label class="tool-label">Color:
                    <input type="color" id="annotation-color" value="#ff0000">
                </label>
                <label class="tool-label">Stroke:
                    <input type="range" id="stroke-width" min="1" max="10" value="3">
                </label>
            </div>
            <div class="tool-group">
                <button class="tool-btn" id="undo-btn" title="Undo">↩</button>
                <button class="tool-btn" id="redo-btn" title="Redo">↪</button>
                <button class="tool-btn" id="reset-btn" title="Reset all">Reset</button>
                <button class="save-btn-compat" id="save-btn" type="button" hidden aria-hidden="true" tabindex="-1">Save</button>
                <span class="save-status" id="save-status" aria-live="polite">Saved</span>
            </div>
        </div>
        <div class="editor-canvas-wrap">
            <canvas id="editor-canvas"></canvas>
        </div>
        <div class="editor-sidebar">
            <div class="form-group">
                <label for="screenshot-title">Title</label>
                <input type="text" id="screenshot-title" value="{{TITLE_ESCAPED}}">
            </div>
            <div class="form-group">
                <label for="screenshot-source-url">Source URL</label>
                <input type="text" id="screenshot-source-url" value="{{SOURCE_URL}}" placeholder="https://example.com/page">
                <a href="{{SOURCE_URL_HREF}}" class="editor-source-link" id="source-url-link" target="_blank" rel="noopener noreferrer"{{SOURCE_LINK_HIDDEN}}>Open source URL</a>
            </div>
            <div class="form-group">
                <label for="screenshot-image-dpi">DPI</label>
                <input type="number" id="screenshot-image-dpi" value="{{IMAGE_DPI}}" min="1" max="2400" step="1">
            </div>
            <div class="form-group">
                <label for="screenshot-visibility">Visibility</label>
                <select id="screenshot-visibility">
                    <option value="unlisted" {{VIS_UNLISTED}}>Shared with private link</option>
                    <option value="private" {{VIS_PRIVATE}}>Unshared</option>
                </select>
            </div>
            <div class="form-group">
                <label for="screenshot-expires-in">Expires</label>
                <select id="screenshot-expires-in">
                    <option value="">{{EXPIRATION_KEEP_LABEL}}</option>
                    <option value="never" {{EXPIRES_NEVER_SELECTED}}>Never</option>
                    <option value="1h">In 1 hour</option>
                    <option value="24h">In 24 hours</option>
                    <option value="7d">In 7 days</option>
                    <option value="30d">In 30 days</option>
                </select>
            </div>
            <div class="form-group">
                <label>Share Link</label>
                <div class="input-group">
                    <input type="text" id="share-url" value="{{SHARE_URL}}" readonly>
                    <button class="btn btn-sm" id="copy-share-btn">Copy</button>
                </div>
            </div>
            <div class="form-group">
                <label>Direct Image</label>
                <div class="input-group">
                    <input type="text" id="raw-url" value="{{RAW_URL}}" readonly>
                    <button class="btn btn-sm" id="copy-raw-btn">Copy</button>
                </div>
            </div>
        </div>
    </main>
    <script src="/static/js/fabric.min.js"></script>
    <script>
        window.SCREENSHOT_ID = "{{ID}}";
        window.ORIGINAL_IMAGE_URL = "/api/screenshots/{{ID}}/original";
        window.ANNOTATIONS = {{ANNOTATIONS}};
        window.CROP_RECT = {{CROP}};
        window.IMAGE_DPI = {{IMAGE_DPI}};
    </script>
    <script src="/static/js/editor.js?v=dpi-edit-1"></script>
</body>
</html>"##;

/// Settings page for API tokens.
pub async fn settings_page(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
) -> crate::Result<impl IntoResponse> {
    let tokens = state.db.list_tokens_for_user(&user.id)?;

    let token_rows: String = if tokens.is_empty() {
        "<tr><td colspan=\"4\" class=\"empty-cell\">No API tokens yet.</td></tr>".to_string()
    } else {
        tokens
            .iter()
            .map(|t| {
                let last_used = t
                    .last_used_at
                    .map(|d| d.format("%b %d, %Y %H:%M").to_string())
                    .unwrap_or_else(|| "Never".to_string());
                format!(
                    r#"<tr>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td><button class="btn btn-sm btn-danger revoke-btn" data-id="{}">Revoke</button></td>
                    </tr>"#,
                    html_escape(&t.label),
                    t.created_at.format("%b %d, %Y"),
                    last_used,
                    t.id,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>ScreenshotSafe — Settings</title>
    {favicon}
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <nav class="navbar">
        <a href="/" class="nav-brand">📸 ScreenshotSafe</a>
        <div class="nav-right">
            <a href="/" class="btn btn-sm btn-outline">← Dashboard</a>
        </div>
    </nav>
    <main class="container">
        <h1>Settings</h1>

        <section class="settings-section">
            <h2>Password</h2>
            <p>Change the password used to sign in to ScreenshotSafe.</p>
            <form id="password-form" class="password-form">
                <div id="password-message" class="settings-message" style="display:none"></div>
                <div class="form-group">
                    <label for="current-password">Current password</label>
                    <input type="password" id="current-password" autocomplete="current-password" required>
                </div>
                <div class="form-group">
                    <label for="new-password">New password</label>
                    <input type="password" id="new-password" autocomplete="new-password" minlength="8" required>
                </div>
                <div class="form-group">
                    <label for="confirm-password">Confirm new password</label>
                    <input type="password" id="confirm-password" autocomplete="new-password" minlength="8" required>
                </div>
                <button class="btn btn-primary" type="submit">Change Password</button>
            </form>
        </section>

        <section class="settings-section">
            <h2>API Tokens</h2>
            <p>Use API tokens to authenticate the Chrome extension or other clients.</p>
            <div class="token-create">
                <input type="text" id="token-label" placeholder="Token label (e.g. Chrome Extension)">
                <button class="btn btn-primary" id="create-token-btn">Create Token</button>
            </div>
            <div id="new-token-display" class="new-token-display" style="display:none">
                <strong>Your new token (copy it now — it won't be shown again):</strong>
                <code id="new-token-value"></code>
                <button class="btn btn-sm" id="copy-token-btn">Copy</button>
            </div>
            <table class="tokens-table">
                <thead>
                    <tr>
                        <th>Label</th>
                        <th>Created</th>
                        <th>Last Used</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody id="tokens-body">
                    {token_rows}
                </tbody>
            </table>
        </section>
    </main>
    <script>
        const passwordForm = document.getElementById('password-form');
        const passwordMessage = document.getElementById('password-message');

        function showPasswordMessage(text, isError) {{
            passwordMessage.textContent = text;
            passwordMessage.className = `settings-message ${{isError ? 'settings-message-error' : 'settings-message-success'}}`;
            passwordMessage.style.display = 'block';
        }}

        passwordForm.addEventListener('submit', async (event) => {{
            event.preventDefault();
            const currentPassword = document.getElementById('current-password').value;
            const newPassword = document.getElementById('new-password').value;
            const confirmPassword = document.getElementById('confirm-password').value;

            if (newPassword !== confirmPassword) {{
                showPasswordMessage('New passwords do not match.', true);
                return;
            }}

            const resp = await fetch('/api/auth/password', {{
                method: 'PUT',
                headers: {{ 'Content-Type': 'application/json' }},
                body: JSON.stringify({{
                    current_password: currentPassword,
                    new_password: newPassword,
                }}),
            }});

            if (resp.ok) {{
                passwordForm.reset();
                showPasswordMessage('Password changed.', false);
            }} else {{
                let message = 'Unable to change password.';
                try {{
                    const data = await resp.json();
                    if (data.error) message = data.error;
                }} catch (_) {{}}
                if (resp.status === 401) message = 'Current password is incorrect.';
                showPasswordMessage(message, true);
            }}
        }});

        document.getElementById('create-token-btn').addEventListener('click', async () => {{
            const label = document.getElementById('token-label').value;
            const resp = await fetch('/api/auth/tokens', {{
                method: 'POST',
                headers: {{ 'Content-Type': 'application/json' }},
                body: JSON.stringify({{ label }}),
            }});
            if (resp.ok) {{
                const data = await resp.json();
                document.getElementById('new-token-value').textContent = data.token;
                document.getElementById('new-token-display').style.display = 'block';
                document.getElementById('token-label').value = '';

                // Add new row to table instead of reloading
                const tbody = document.getElementById('tokens-body');
                // Remove "No API tokens yet" row if present
                const emptyCell = tbody.querySelector('.empty-cell');
                if (emptyCell) emptyCell.closest('tr').remove();

                const tr = document.createElement('tr');
                const created = new Date(data.created_at).toLocaleDateString('en-US', {{ month: 'short', day: 'numeric', year: 'numeric' }});
                tr.innerHTML = `<td>${{data.label || ''}}</td><td>${{created}}</td><td>Never</td><td><button class="btn btn-sm btn-danger revoke-btn" data-id="${{data.id}}">Revoke</button></td>`;
                tbody.prepend(tr);
                tr.querySelector('.revoke-btn').addEventListener('click', async () => {{
                    if (!confirm('Revoke this token?')) return;
                    const r = await fetch(`/api/auth/tokens/${{data.id}}`, {{ method: 'DELETE' }});
                    if (r.ok) window.location.reload();
                }});
            }}
        }});

        document.getElementById('copy-token-btn')?.addEventListener('click', () => {{
            const token = document.getElementById('new-token-value').textContent;
            navigator.clipboard.writeText(token);
            const btn = document.getElementById('copy-token-btn');
            btn.textContent = 'Copied!';
            setTimeout(() => btn.textContent = 'Copy', 2000);
        }});

        document.querySelectorAll('.revoke-btn').forEach(btn => {{
            btn.addEventListener('click', async () => {{
                if (!confirm('Revoke this token?')) return;
                const resp = await fetch(`/api/auth/tokens/${{btn.dataset.id}}`, {{ method: 'DELETE' }});
                if (resp.ok) window.location.reload();
            }});
        }});
    </script>
</body>
</html>"#,
        favicon = FAVICON_LINK,
        token_rows = token_rows,
    );

    Ok(Html(html).into_response())
}

/// Administration page for managing users.
pub async fn admin_page(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
) -> crate::Result<impl IntoResponse> {
    let users = state.db.list_users()?;
    let user_rows = users
        .iter()
        .map(|user| {
            let role = if user.is_admin { "Admin" } else { "User" };
            let delete_button = if user.id == admin.id {
                "<span class=\"admin-muted\">Current user</span>".to_string()
            } else {
                format!(
                    r#"<button class="btn btn-sm btn-danger delete-user-btn" data-id="{}" data-username="{}">Delete</button>"#,
                    user.id,
                    html_escape(&user.username),
                )
            };
            format!(
                r#"<tr>
                    <td>{}</td>
                    <td>{}</td>
                    <td><span class="role-pill{}">{}</span></td>
                    <td>{}</td>
                    <td>{}</td>
                </tr>"#,
                html_escape(&user.username),
                html_escape(&user.display_name),
                if user.is_admin { " role-pill-admin" } else { "" },
                role,
                user.created_at.format("%b %d, %Y"),
                delete_button,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>ScreenshotSafe — Admin</title>
    {favicon}
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <nav class="navbar">
        <a href="/" class="nav-brand">📸 ScreenshotSafe</a>
        <div class="nav-right">
            <a href="/" class="btn btn-sm btn-outline">Dashboard</a>
            <a href="/settings" class="btn btn-sm btn-outline">Settings</a>
        </div>
    </nav>
    <main class="container">
        <div class="page-header">
            <h1>Administration</h1>
        </div>

        <section class="settings-section">
            <h2>Add User</h2>
            <form id="user-form" class="admin-user-form">
                <div id="user-message" class="settings-message" style="display:none"></div>
                <div class="admin-form-grid">
                    <div class="form-group">
                        <label for="username">Username</label>
                        <input type="text" id="username" autocomplete="username" required>
                    </div>
                    <div class="form-group">
                        <label for="display-name">Display Name</label>
                        <input type="text" id="display-name" autocomplete="name">
                    </div>
                    <div class="form-group">
                        <label for="password">Password</label>
                        <input type="password" id="password" autocomplete="new-password" minlength="8" required>
                    </div>
                    <label class="checkbox-row">
                        <input type="checkbox" id="is-admin">
                        <span>Admin user</span>
                    </label>
                </div>
                <button class="btn btn-primary" type="submit">Add User</button>
            </form>
        </section>

        <section class="settings-section">
            <h2>Users</h2>
            <table class="tokens-table users-table">
                <thead>
                    <tr>
                        <th>Username</th>
                        <th>Display Name</th>
                        <th>Role</th>
                        <th>Created</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
                    {user_rows}
                </tbody>
            </table>
        </section>
    </main>
    <script>
        const form = document.getElementById('user-form');
        const message = document.getElementById('user-message');

        function showMessage(text, isError) {{
            message.textContent = text;
            message.className = `settings-message ${{isError ? 'settings-message-error' : 'settings-message-success'}}`;
            message.style.display = 'block';
        }}

        form.addEventListener('submit', async (event) => {{
            event.preventDefault();
            const resp = await fetch('/api/admin/users', {{
                method: 'POST',
                headers: {{ 'Content-Type': 'application/json' }},
                body: JSON.stringify({{
                    username: document.getElementById('username').value,
                    display_name: document.getElementById('display-name').value || undefined,
                    password: document.getElementById('password').value,
                    is_admin: document.getElementById('is-admin').checked,
                }}),
            }});

            if (resp.ok) {{
                window.location.reload();
            }} else {{
                let text = 'Unable to add user.';
                try {{
                    const data = await resp.json();
                    if (data.error) text = data.error;
                }} catch (_) {{}}
                showMessage(text, true);
            }}
        }});

        document.querySelectorAll('.delete-user-btn').forEach((btn) => {{
            btn.addEventListener('click', async () => {{
                if (!confirm(`Delete user "${{btn.dataset.username}}"? Their screenshots and API tokens will also be deleted.`)) return;
                const resp = await fetch(`/api/admin/users/${{btn.dataset.id}}`, {{ method: 'DELETE' }});
                if (resp.ok) {{
                    window.location.reload();
                }} else {{
                    let text = 'Unable to delete user.';
                    try {{
                        const data = await resp.json();
                        if (data.error) text = data.error;
                    }} catch (_) {{}}
                    alert(text);
                }}
            }});
        }});
    </script>
</body>
</html>"#,
        favicon = FAVICON_LINK,
        user_rows = user_rows,
    );

    Ok(Html(html).into_response())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn is_safe_external_url(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty() && (url.starts_with("http://") || url.starts_with("https://"))
}
