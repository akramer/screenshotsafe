use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::{auth, AppError, SharedState};
use crate::models::User;

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

/// Extractor: authenticated user from session cookie or Bearer token.
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
        async move {
            // Try Bearer token first (for API clients)
            if let Some(auth_header) = headers.get(header::AUTHORIZATION) {
                if let Ok(auth_str) = auth_header.to_str() {
                    if let Some(token) = auth_str.strip_prefix("Bearer ") {
                        let token_hash = auth::hash_token(token);
                        if let Some((user, _)) = state.db.get_user_by_token_hash(&token_hash)? {
                            return Ok(AuthUser(user));
                        }
                    }
                }
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
                                let user_id: uuid::Uuid = data
                                    .claims
                                    .sub
                                    .parse()
                                    .map_err(|_| AppError::Unauthorized)?;
                                if let Some(user) = state.db.get_user_by_id(&user_id)? {
                                    return Ok(AuthUser(user));
                                }
                            }
                        }
                    }
                }
            }

            Err(AppError::Unauthorized)
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
