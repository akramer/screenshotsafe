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
        .unwrap_or("localhost:8080");

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");
    format!("{}://{}", scheme, host)
}
