use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{AppError, SharedState};

const SENSITIVE_AUTH_LIMIT: u32 = 10;
const SENSITIVE_AUTH_WINDOW: Duration = Duration::from_secs(60);
const STALE_ENTRY_TTL: Duration = Duration::from_secs(60 * 10);

#[derive(Debug, Default)]
pub struct RateLimiter {
    sensitive_auth: Mutex<HashMap<String, RateBucket>>,
}

#[derive(Debug)]
struct RateBucket {
    window_start: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn check_sensitive_auth(&self, headers: &HeaderMap, path: &str) -> RateLimitResult {
        let key = format!("{}:{}", path, client_key(headers));
        check_bucket(
            &self.sensitive_auth,
            key,
            SENSITIVE_AUTH_LIMIT,
            SENSITIVE_AUTH_WINDOW,
        )
    }
}

pub struct RateLimitResult {
    pub allowed: bool,
    pub retry_after: Duration,
}

pub async fn sensitive_auth_rate_limit(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> crate::Result<Response> {
    let limit = state
        .rate_limiter
        .check_sensitive_auth(req.headers(), req.uri().path());
    if !limit.allowed {
        return Err(AppError::rate_limited(limit.retry_after));
    }

    Ok(next.run(req).await)
}

fn check_bucket(
    buckets: &Mutex<HashMap<String, RateBucket>>,
    key: String,
    limit: u32,
    window: Duration,
) -> RateLimitResult {
    let now = Instant::now();
    let mut buckets = buckets.lock().expect("rate limit mutex poisoned");

    buckets.retain(|_, bucket| now.duration_since(bucket.window_start) <= STALE_ENTRY_TTL);

    let bucket = buckets.entry(key).or_insert_with(|| RateBucket {
        window_start: now,
        count: 0,
    });

    let elapsed = now.duration_since(bucket.window_start);
    if elapsed >= window {
        bucket.window_start = now;
        bucket.count = 0;
    }

    if bucket.count >= limit {
        return RateLimitResult {
            allowed: false,
            retry_after: window.saturating_sub(now.duration_since(bucket.window_start)),
        };
    }

    bucket.count += 1;
    RateLimitResult {
        allowed: true,
        retry_after: Duration::ZERO,
    }
}

fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("direct-client")
        .to_string()
}

pub fn retry_after_header_value(duration: Duration) -> String {
    duration.as_secs().max(1).to_string()
}

pub fn rate_limit_response(retry_after: Duration) -> Response {
    let mut response = (
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({ "error": "Too many requests" })),
    )
        .into_response();
    if let Ok(value) = retry_after_header_value(retry_after).parse() {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}
