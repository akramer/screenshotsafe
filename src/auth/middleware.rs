use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, HeaderMap},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::models::User;
use crate::{AppError, SharedState};

/// JWT claims stored in session cookies.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user ID
    pub exp: usize,  // expiration timestamp
}

/// Create a signed JWT for a user session.
pub fn create_session_token(user_id: &uuid::Uuid, secret: &str, ttl_seconds: u64) -> String {
    let exp = chrono::Utc::now().timestamp() as usize + ttl_seconds as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("JWT encoding should not fail")
}

/// Extractor: authenticated user from session cookie ONLY.
/// Use this in route handlers: `AuthUser(user)`.
pub struct AuthUser(pub User);

impl FromRequestParts<SharedState> for AuthUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let state = state.clone();
        let headers = parts.headers.clone();
        let path = parts.uri.path().to_string();
        async move {
            if path.starts_with("/api/") && !session_origin_allowed(&headers, &state) {
                return Err(AppError::forbidden(
                    "cookie-authenticated API request from untrusted origin",
                ));
            }

            // Try session cookie
            if let Some(cookie_header) = headers.get(header::COOKIE) {
                if let Ok(cookie_str) = cookie_header.to_str() {
                    for cookie in cookie_str.split(';') {
                        let cookie = cookie.trim();
                        if let Some(token) = cookie.strip_prefix("session=") {
                            let token_data = decode::<Claims>(
                                token,
                                &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
                                &Validation::default(),
                            );
                            if let Ok(data) = token_data {
                                let user_id: uuid::Uuid =
                                    data.claims.sub.parse().map_err(|_| {
                                        AppError::unauthorized("session token subject is invalid")
                                    })?;
                                if let Some(user) = state.db.get_user_by_id(&user_id)? {
                                    if !user.account_status.is_enabled() {
                                        return Err(AppError::forbidden(format!(
                                            "session user '{}' is {}",
                                            user.username,
                                            user.account_status.as_str()
                                        )));
                                    }
                                    return Ok(AuthUser(user));
                                }
                            }
                        }
                    }
                }
            }

            Err(AppError::unauthorized(
                "missing, invalid, expired, or unknown session cookie",
            ))
        }
    }
}

pub fn session_origin_allowed(headers: &HeaderMap, state: &SharedState) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) else {
        return true;
    };

    let origin = origin.trim_end_matches('/');
    if origin.is_empty() || origin == "null" {
        return false;
    }

    let base_url = crate::routes::get_base_url(&state.config.server.public_url, headers);
    if origin == base_url.trim_end_matches('/') {
        return true;
    }

    if origin.starts_with("chrome-extension://") {
        return true;
    }

    state
        .config
        .auth
        .allowed_extension_origins
        .iter()
        .any(|allowed| origin == allowed.trim_end_matches('/'))
}

/// Extractor: authenticated admin user from session cookie ONLY.
pub struct AdminUser(pub User);

impl FromRequestParts<SharedState> for AdminUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let state = state.clone();
        let mut parts_clone = parts.clone();
        async move {
            let AuthUser(user) = AuthUser::from_request_parts(&mut parts_clone, &state).await?;
            if user.is_admin {
                Ok(AdminUser(user))
            } else {
                Err(AppError::forbidden(format!(
                    "user '{}' is not an administrator",
                    user.username
                )))
            }
        }
    }
}

/// Extractor: optional authenticated user (doesn't fail if not logged in).
pub struct MaybeAuthUser(pub Option<User>);

impl FromRequestParts<SharedState> for MaybeAuthUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let state = state.clone();
        let _headers = parts.headers.clone();
        async move {
            // Reuse AuthUser logic but don't fail
            let mut parts_clone = parts.clone();
            match AuthUser::from_request_parts(&mut parts_clone, &state).await {
                Ok(AuthUser(user)) => Ok(MaybeAuthUser(Some(user))),
                Err(_) => Ok(MaybeAuthUser(None)),
            }
        }
    }
}

/// Extractor: authenticated user from session cookie OR Bearer token.
/// Use this for API routes that allow token authentication (like uploads).
pub struct ApiOrSessionUser(pub User);

impl FromRequestParts<SharedState> for ApiOrSessionUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let state = state.clone();
        let headers = parts.headers.clone();
        let mut parts_clone = parts.clone();
        async move {
            // Try Bearer token first
            if let Some(auth_header) = headers.get(header::AUTHORIZATION) {
                if let Ok(auth_str) = auth_header.to_str() {
                    if let Some(token) = auth_str.strip_prefix("Bearer ") {
                        let token_hash = crate::auth::hash_token(token);
                        if let Some((user, _)) = state.db.get_user_by_token_hash(&token_hash)? {
                            if !user.account_status.is_enabled() {
                                return Err(AppError::forbidden(format!(
                                    "bearer token user '{}' is {}",
                                    user.username,
                                    user.account_status.as_str()
                                )));
                            }
                            return Ok(ApiOrSessionUser(user));
                        }
                    }
                }
            }

            // Fallback to session cookie
            match AuthUser::from_request_parts(&mut parts_clone, &state).await {
                Ok(AuthUser(user)) => Ok(ApiOrSessionUser(user)),
                Err(e) => Err(e),
            }
        }
    }
}
