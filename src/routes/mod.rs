pub mod api;
pub mod pages;
pub mod share;

use axum::http::HeaderMap;

pub fn get_base_url(public_url: &str, headers: &HeaderMap) -> String {
    if !public_url.is_empty() {
        return public_url.trim_end_matches('/').to_string();
    }

    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|host| is_safe_host(host))
        .unwrap_or("localhost:8080");

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .filter(|scheme| matches!(*scheme, "http" | "https"))
        .unwrap_or("http");
    format!("{}://{}", scheme, host)
}

fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && !host.contains("..")
        && !host.contains('@')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

#[cfg(test)]
mod tests {
    use super::get_base_url;
    use axum::http::{header, HeaderMap, HeaderValue};

    #[test]
    fn get_base_url_rejects_header_injection() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static(r#"example.com" onerror="alert(1)"#),
        );
        headers.insert("x-forwarded-proto", HeaderValue::from_static("javascript"));

        assert_eq!(get_base_url("", &headers), "http://localhost:8080");
    }

    #[test]
    fn get_base_url_accepts_safe_forwarded_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static("screens.example:8443"),
        );
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));

        assert_eq!(get_base_url("", &headers), "https://screens.example:8443");
    }
}
