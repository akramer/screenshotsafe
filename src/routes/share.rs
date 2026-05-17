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
    let share_url = format!("{}/s/{}", base_url, share_id);
    let direct_image_url = format!("{}/s/{}.png", base_url, share_id);
    let image_url = format!("{}/s/{}.png?v={}", base_url, share_id, cache_bust);
    let created = screenshot.created_at.format("%B %d, %Y").to_string();
    let expires_info = screenshot
        .expires_at
        .map(|e| format!("Expires {}", e.format("%B %d, %Y")))
        .unwrap_or_else(|| "Does not expire".to_string());
    let title_html = render_title_markdown_links(&title);
    let source_link = screenshot
        .source_url
        .as_deref()
        .and_then(source_url_link)
        .map(|link| format!(" · {}", link))
        .unwrap_or_default();

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
        .share-title a {{
            color: #8ea8ff;
            text-decoration: underline;
            text-underline-offset: 0.16em;
        }}
        .share-meta {{
            margin-top: 0.5rem;
            font-size: 0.85rem;
            color: #888;
        }}
        .share-meta a {{
            color: #8ea8ff;
            text-decoration: none;
        }}
        .share-meta a:hover {{
            text-decoration: underline;
        }}
        .share-actions {{
            display: flex;
            flex-wrap: wrap;
            gap: 0.5rem;
            margin-top: 1rem;
        }}
        .share-action {{
            appearance: none;
            border: 1px solid rgba(255,255,255,0.14);
            border-radius: 6px;
            background: rgba(255,255,255,0.06);
            color: #f4f4f5;
            cursor: pointer;
            display: inline-flex;
            align-items: center;
            justify-content: center;
            min-height: 2.25rem;
            padding: 0.45rem 0.75rem;
            font: inherit;
            font-size: 0.85rem;
            font-weight: 600;
            line-height: 1;
            text-decoration: none;
            transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
        }}
        .share-action:hover {{
            background: rgba(255,255,255,0.1);
            border-color: rgba(255,255,255,0.22);
            text-decoration: none;
        }}
        .share-action:focus-visible {{
            outline: 2px solid #8ea8ff;
            outline-offset: 2px;
        }}
        .share-action[data-status="success"] {{
            border-color: rgba(91, 214, 138, 0.52);
            color: #a7f3c1;
        }}
        .share-action[data-status="error"] {{
            border-color: rgba(255, 130, 130, 0.5);
            color: #ffb4b4;
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
            .share-actions {{ gap: 0.4rem; }}
            .share-action {{
                flex: 1 1 100%;
                min-width: 0;
            }}
            .share-body {{ padding: 1rem; }}
            .share-image {{ border-radius: 4px; }}
        }}
    </style>
</head>
<body>
    <header class="share-header">
        <h1 class="share-title">{title_html}</h1>
        <div class="share-meta">
            Shared on {created} · {expires_info}{source_link}
        </div>
        <div class="share-actions" aria-label="Share actions">
            <button class="share-action" type="button" id="copy-page-link" data-url="{share_url}">Copy Page Link</button>
            <a class="share-action" href="{direct_image_url}" target="_blank" rel="noopener">Open Image</a>
            <button class="share-action" type="button" id="copy-image" data-url="{image_url}">Copy Image</button>
        </div>
    </header>
    <main class="share-body">
        <img src="{image_url}" alt="{title}" class="share-image">
    </main>
    <footer class="share-footer">
        Powered by <a href="https://github.com/screenshotsafe/screenshotsafe">ScreenshotSafe</a>
    </footer>
    <script>
        const setButtonStatus = (button, text, status) => {{
            const original = button.dataset.label || button.textContent;
            button.dataset.label = original;
            button.textContent = text;
            button.dataset.status = status;
            window.clearTimeout(button._statusTimer);
            button._statusTimer = window.setTimeout(() => {{
                button.textContent = original;
                delete button.dataset.status;
            }}, 1800);
        }};

        document.getElementById('copy-page-link')?.addEventListener('click', async (event) => {{
            const button = event.currentTarget;
            try {{
                await navigator.clipboard.writeText(button.dataset.url);
                setButtonStatus(button, 'Copied', 'success');
            }} catch (_error) {{
                setButtonStatus(button, 'Could not copy', 'error');
            }}
        }});

        document.getElementById('copy-image')?.addEventListener('click', async (event) => {{
            const button = event.currentTarget;
            try {{
                if (!window.ClipboardItem) {{
                    throw new Error('Image clipboard is not supported');
                }}
                const response = await fetch(button.dataset.url);
                if (!response.ok) {{
                    throw new Error('Could not load image');
                }}
                const blob = await response.blob();
                await navigator.clipboard.write([
                    new ClipboardItem({{ [blob.type || 'image/png']: blob }})
                ]);
                setButtonStatus(button, 'Copied', 'success');
            }} catch (_error) {{
                setButtonStatus(button, 'Could not copy', 'error');
            }}
        }});
    </script>
</body>
</html>"#,
        title = html_escape(&title),
        title_html = title_html,
        share_url = html_escape(&share_url),
        direct_image_url = html_escape(&direct_image_url),
        image_url = image_url,
        created = created,
        expires_info = expires_info,
        source_link = source_link,
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

fn render_title_markdown_links(input: &str) -> String {
    let mut rendered = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(open_index) = rest.find('[') {
        rendered.push_str(&html_escape(&rest[..open_index]));
        let candidate = &rest[open_index..];

        let Some(separator_index) = candidate.find("](") else {
            rendered.push_str(&html_escape(candidate));
            return rendered;
        };

        let label = &candidate[1..separator_index];
        let url_start = separator_index + 2;
        let Some(close_offset) = candidate[url_start..].find(')') else {
            rendered.push_str(&html_escape(candidate));
            return rendered;
        };

        let url_end = url_start + close_offset;
        let url = &candidate[url_start..url_end];
        let markdown = &candidate[..=url_end];

        if is_safe_title_url(url) {
            rendered.push_str(&format!(
                r#"<a href="{}" target="_blank" rel="noopener noreferrer">{}</a>"#,
                html_escape(url),
                html_escape(label)
            ));
        } else {
            rendered.push_str(&html_escape(markdown));
        }

        rest = &candidate[url_end + 1..];
    }

    rendered.push_str(&html_escape(rest));
    rendered
}

fn is_safe_title_url(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty()
        && (url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:"))
}

fn source_url_link(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }

    Some(format!(
        r#"<a href="{}" target="_blank" rel="noopener noreferrer">Source page</a>"#,
        html_escape(url)
    ))
}

#[cfg(test)]
mod tests {
    use super::render_title_markdown_links;

    #[test]
    fn renders_safe_markdown_links() {
        assert_eq!(
            render_title_markdown_links("Page title [original](https://example.com/page?a=1&b=2)"),
            r#"Page title <a href="https://example.com/page?a=1&amp;b=2" target="_blank" rel="noopener noreferrer">original</a>"#
        );
    }

    #[test]
    fn escapes_plain_title_text() {
        assert_eq!(
            render_title_markdown_links(r#"<script>alert("x")</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn rejects_unsafe_link_urls() {
        assert_eq!(
            render_title_markdown_links("[bad](javascript:alert(1))"),
            "[bad](javascript:alert(1))"
        );
    }
}
