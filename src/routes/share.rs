use axum::{
    extract::{Path, State},
    http::{header, HeaderMap},
    response::{Html, IntoResponse},
};

use crate::{AppError, SharedState};

/// Dispatch handler: routes /s/{id}.png to image, /s/{id} to share page.
pub async fn share_dispatch(
    state: State<SharedState>,
    headers: HeaderMap,
    Path(share_id_or_file): Path<String>,
) -> crate::Result<axum::response::Response> {
    if let Some(share_id) = share_id_or_file.strip_suffix(".png") {
        Ok(share_image(state, Path(share_id.to_string()))
            .await?
            .into_response())
    } else {
        Ok(share_page(state, headers, Path(share_id_or_file))
            .await?
            .into_response())
    }
}

/// Public share page — displays the screenshot with title and metadata.
pub async fn share_page(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(share_id): Path<String>,
) -> crate::Result<impl IntoResponse> {
    let screenshot = state
        .db
        .get_screenshot_by_share_id(&share_id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.is_expired() {
        return Err(AppError::Gone("This screenshot has expired".into()));
    }

    if screenshot.visibility == "private" {
        return Err(AppError::NotFound);
    }

    let title = screenshot.display_title().to_string();
    let cache_bust = screenshot.updated_at.timestamp();
    let base_url = crate::routes::get_base_url(&state.config.server.public_url, &headers);
    let image_url = format!("{}/s/{}.png?v={}", base_url, share_id, cache_bust);
    let created = screenshot.created_at.format("%B %d, %Y").to_string();
    let expires_info = screenshot
        .expires_at
        .map(|e| format!("Expires {}", e.format("%B %d, %Y")))
        .unwrap_or_else(|| "Does not expire".to_string());

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=5">
    <title>{title}</title>
    <meta name="description" content="Screenshot shared via ScreenshotSafe">
    <meta property="og:title" content="{title}">
    <meta property="og:image" content="{image_url}">
    <meta property="og:type" content="website">
    <meta name="twitter:card" content="summary_large_image">
    <meta name="twitter:title" content="{title}">
    <meta name="twitter:image" content="{image_url}">
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: #0f0f13;
            color: #e0e0e0;
            min-height: 100vh;
            display: flex;
            flex-direction: column;
        }}
        .share-header {{
            padding: 1.5rem 2rem;
            border-bottom: 1px solid rgba(255,255,255,0.06);
            background: rgba(255,255,255,0.02);
        }}
        .share-title {{
            font-size: 1.25rem;
            font-weight: 600;
            color: #f0f0f0;
            word-break: break-word;
        }}
        .share-meta {{
            margin-top: 0.5rem;
            font-size: 0.85rem;
            color: #888;
        }}
        .share-body {{
            flex: 1;
            display: flex;
            justify-content: center;
            align-items: flex-start;
            padding: 2rem;
        }}
        .share-image {{
            max-width: 100%;
            max-height: 85vh;
            border-radius: 8px;
            box-shadow: 0 8px 32px rgba(0,0,0,0.5);
        }}
        .share-footer {{
            padding: 1rem 2rem;
            text-align: center;
            font-size: 0.75rem;
            color: #555;
            border-top: 1px solid rgba(255,255,255,0.06);
        }}
        .share-footer a {{
            color: #6a6aff;
            text-decoration: none;
        }}
        @media (max-width: 768px) {{
            .share-header {{ padding: 1rem; }}
            .share-body {{ padding: 1rem; }}
            .share-image {{ border-radius: 4px; }}
        }}
    </style>
</head>
<body>
    <header class="share-header">
        <h1 class="share-title">{title}</h1>
        <div class="share-meta">
            Shared on {created} · {expires_info}
        </div>
    </header>
    <main class="share-body">
        <img src="{image_url}" alt="{title}" class="share-image">
    </main>
    <footer class="share-footer">
        Powered by <a href="https://github.com/screenshotsafe/screenshotsafe">ScreenshotSafe</a>
    </footer>
</body>
</html>"#,
        title = html_escape(&title),
        image_url = image_url,
        created = created,
        expires_info = expires_info,
    );

    Ok(Html(html))
}

/// Direct PNG image — serves the rendered screenshot file.
pub async fn share_image(
    State(state): State<SharedState>,
    Path(share_id): Path<String>,
) -> crate::Result<impl IntoResponse> {
    let screenshot = state
        .db
        .get_screenshot_by_share_id(&share_id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.is_expired() {
        return Err(AppError::Gone("This screenshot has expired".into()));
    }

    if screenshot.visibility == "private" {
        return Err(AppError::NotFound);
    }

    let rendered_path = screenshot
        .rendered_path
        .as_deref()
        .ok_or(AppError::NotFound)?;

    let data = std::fs::read(rendered_path)?;

    // Use ETag from file modification time for cache validation
    let etag = std::fs::metadata(rendered_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            format!(
                "\"{:?}\"",
                t.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            )
        })
        .unwrap_or_default();

    Ok((
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::ETAG, etag),
        ],
        data,
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
