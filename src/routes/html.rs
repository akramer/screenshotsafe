//! HTML helpers shared by the server-rendered app pages and public share pages.

/// Rewrites `<time data-local-time>` elements into the viewer's locale and time zone.
pub const LOCAL_TIME_SCRIPT: &str = r#"<script>
        (() => {
            const formats = {
                date: { month: 'short', day: 'numeric', year: 'numeric' },
                datetime: {
                    month: 'short',
                    day: 'numeric',
                    year: 'numeric',
                    hour: 'numeric',
                    minute: '2-digit',
                    timeZoneName: 'short'
                },
                'long-date': { month: 'long', day: 'numeric', year: 'numeric' }
            };

            document.querySelectorAll('[data-local-time]').forEach((el) => {
                const value = el.getAttribute('datetime') || el.dataset.datetime;
                if (!value) return;

                const date = new Date(value);
                if (Number.isNaN(date.getTime())) return;

                const options = formats[el.dataset.localFormat] || formats.datetime;
                const formatted = new Intl.DateTimeFormat(undefined, options).format(date);
                el.textContent = `${el.dataset.localPrefix || ''}${formatted}${el.dataset.localSuffix || ''}`;
            });
        })();
    </script>"#;

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Renders a `<time>` element that `LOCAL_TIME_SCRIPT` localizes in the browser,
/// falling back to `fallback_format` (UTC) when scripts don't run.
pub fn local_time(
    datetime: chrono::DateTime<chrono::Utc>,
    local_format: &str,
    fallback_format: &str,
) -> String {
    format!(
        r#"<time datetime="{}" data-local-time data-local-format="{}">{}</time>"#,
        datetime.to_rfc3339(),
        html_escape(local_format),
        html_escape(&datetime.format(fallback_format).to_string()),
    )
}
