use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::middleware::{AdminUser, ApiOrSessionUser, AuthUser, MaybeAuthUser};
use crate::config::OAuthAccountMode;
use crate::models::{
    AccountStatus, Annotation, ApiToken, CropRect, OAuthIdentity, Screenshot, User,
};
use crate::{auth, image_processing, share_id, AppError, SharedState};

// ── Setup (first-run) ──

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
}

pub async fn setup(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(req): Json<SetupRequest>,
) -> crate::Result<impl IntoResponse> {
    // Only allow setup if no users exist
    if state.db.user_count()? > 0 {
        return Err(AppError::BadRequest("Setup already completed".into()));
    }

    let username = req.username.trim();
    if username.is_empty() || req.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Username required, password must be at least 8 characters".into(),
        ));
    }

    let password_hash = auth::hash_password(&req.password)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?;

    let user = User {
        id: Uuid::new_v4(),
        username: username.to_string(),
        password_hash: Some(password_hash),
        display_name: req
            .display_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| username.to_string()),
        is_admin: true,
        account_status: AccountStatus::Enabled,
        max_screenshot_size_bytes: None,
        max_expiry_seconds: None,
        created_at: Utc::now(),
    };

    state.db.create_user(&user)?;
    tracing::info!("Initial user '{}' created", user.username);

    // Auto-login: create session token
    let token = auth::middleware::create_session_token(
        &user.id,
        &state.jwt_secret,
        state.config.auth.session_ttl_seconds,
    );

    let cookie = session_cookie(&state, &headers, &token);

    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "ok": true,
            "user": {
                "id": user.id,
                "username": user.username,
                "display_name": user.display_name,
                "is_admin": user.is_admin,
            }
        })),
    ))
}

// ── Login ──

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> crate::Result<impl IntoResponse> {
    let user = state
        .db
        .get_user_by_username(&req.username)?
        .ok_or(AppError::Unauthorized)?;
    if !user.account_status.is_enabled() {
        return Err(AppError::Forbidden);
    }

    let hash = user
        .password_hash
        .as_deref()
        .ok_or(AppError::Unauthorized)?;
    if !auth::verify_password(&req.password, hash) {
        return Err(AppError::Unauthorized);
    }

    let token = auth::middleware::create_session_token(
        &user.id,
        &state.jwt_secret,
        state.config.auth.session_ttl_seconds,
    );

    let cookie = session_cookie(&state, &headers, &token);

    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "ok": true,
            "user": {
                "id": user.id,
                "username": user.username,
                "display_name": user.display_name,
                "is_admin": user.is_admin,
            }
        })),
    ))
}

// ── OAuth login/link ──

#[derive(Debug, Serialize, Deserialize)]
struct OAuthStateClaims {
    sub: Option<String>,
    nonce: String,
    exp: usize,
}

#[derive(Deserialize)]
pub struct OAuthStartQuery {
    link: Option<bool>,
}

#[derive(Deserialize)]
pub struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct OpenIdDiscoveryDocument {
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    userinfo_endpoint: Option<String>,
}

#[derive(Debug, Clone)]
struct OAuthEndpoints {
    authorize_url: String,
    token_url: String,
    userinfo_url: String,
}

#[derive(Deserialize)]
struct OAuthUserInfo {
    sub: Option<String>,
    id: Option<serde_json::Value>,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    preferred_username: Option<String>,
    login: Option<String>,
}

pub async fn oauth_start(
    State(state): State<SharedState>,
    headers: HeaderMap,
    user: MaybeAuthUser,
    Query(params): Query<OAuthStartQuery>,
) -> crate::Result<impl IntoResponse> {
    let oauth = &state.config.auth.oauth;
    validate_oauth_config(oauth)?;
    let endpoints = resolve_oauth_endpoints(oauth).await?;

    let link_user_id = if params.link.unwrap_or(false) {
        Some(
            user.0
                .as_ref()
                .ok_or(AppError::Unauthorized)?
                .id
                .to_string(),
        )
    } else {
        None
    };

    let oauth_state = OAuthStateClaims {
        sub: link_user_id,
        nonce: Uuid::new_v4().to_string(),
        exp: chrono::Utc::now().timestamp() as usize + 10 * 60,
    };
    let state_token = encode(
        &Header::default(),
        &oauth_state,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("OAuth state failed: {}", e)))?;

    let redirect_uri = oauth_redirect_uri(&state, &headers);
    let mut authorize_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
        endpoints.authorize_url,
        urlencoding::encode(&oauth.client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&oauth.scope),
        urlencoding::encode(&state_token),
    );
    if oauth
        .scope
        .split_whitespace()
        .any(|scope| scope == "openid")
    {
        authorize_url.push_str("&nonce=");
        authorize_url.push_str(&urlencoding::encode(&oauth_state.nonce));
    }

    let cookie = format!(
        "oauth_state={}; HttpOnly; SameSite=Lax; Path=/api/auth/oauth; Max-Age=600",
        state_token
    );

    Ok((
        [(header::SET_COOKIE, cookie)],
        Redirect::temporary(&authorize_url),
    ))
}

pub async fn oauth_callback(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<OAuthCallbackQuery>,
) -> crate::Result<Response> {
    let oauth = &state.config.auth.oauth;
    validate_oauth_config(oauth)?;
    let endpoints = resolve_oauth_endpoints(oauth).await?;

    if params.error.is_some() {
        return Ok(Redirect::to("/login?oauth=error").into_response());
    }
    let code = params
        .code
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing OAuth code".into()))?;
    let state_token = params
        .state
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing OAuth state".into()))?;
    if !oauth_state_cookie_matches(&headers, state_token) {
        return Err(AppError::BadRequest("Invalid OAuth state".into()));
    }

    let state_claims = decode::<OAuthStateClaims>(
        state_token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| AppError::BadRequest("Invalid OAuth state".into()))?
    .claims;

    let redirect_uri = oauth_redirect_uri(&state, &headers);
    let client = reqwest::Client::new();
    let token_resp = client
        .post(&endpoints.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &oauth.client_id),
            ("client_secret", &oauth.client_secret),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("OAuth token request failed: {}", e)))?;
    if !token_resp.status().is_success() {
        return Ok(Redirect::to("/login?oauth=error").into_response());
    }
    let token: OAuthTokenResponse = token_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("OAuth token response failed: {}", e)))?;

    let userinfo_resp = client
        .get(&endpoints.userinfo_url)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("OAuth userinfo request failed: {}", e)))?;
    if !userinfo_resp.status().is_success() {
        return Ok(Redirect::to("/login?oauth=error").into_response());
    }
    let userinfo: OAuthUserInfo = userinfo_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("OAuth userinfo response failed: {}", e)))?;
    let subject_value = userinfo
        .sub
        .clone()
        .or_else(|| userinfo.id.as_ref().and_then(json_value_to_string))
        .ok_or_else(|| AppError::BadRequest("OAuth userinfo is missing a subject".into()))?;
    let subject = subject_value.as_str();
    let email = userinfo
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if !oauth_email_allowed(oauth, email, userinfo.email_verified) {
        return Ok(Redirect::to("/login?oauth=denied").into_response());
    }
    let display_name = oauth_display_name(&userinfo, email);

    if let Some(link_user_id) = state_claims.sub {
        let user_id = link_user_id
            .parse::<Uuid>()
            .map_err(|_| AppError::BadRequest("Invalid OAuth state".into()))?;
        let user = state
            .db
            .get_user_by_id(&user_id)?
            .ok_or(AppError::Unauthorized)?;
        if !user.account_status.is_enabled() {
            return Err(AppError::Forbidden);
        }
        if let Some((linked_user, _)) = state
            .db
            .get_user_by_oauth_identity(&oauth.provider, subject)?
        {
            if linked_user.id != user.id {
                return Ok(Redirect::to("/settings?oauth=already_linked").into_response());
            }
        } else {
            let identity = OAuthIdentity {
                id: Uuid::new_v4(),
                user_id: user.id,
                provider: oauth.provider.clone(),
                subject: subject.to_string(),
                email: email.map(str::to_string),
                display_name: display_name.clone(),
                created_at: Utc::now(),
                last_login_at: Some(Utc::now()),
            };
            state.db.create_oauth_identity(&identity)?;
        }
        return Ok(with_session_cookie(
            Redirect::to("/settings?oauth=linked").into_response(),
            &state,
            &headers,
            &user.id,
        ));
    }

    if let Some((user, _identity)) = state
        .db
        .get_user_by_oauth_identity(&oauth.provider, subject)?
    {
        state.db.update_oauth_identity_login(
            &oauth.provider,
            subject,
            email,
            display_name.as_deref(),
        )?;
        if !user.account_status.is_enabled() {
            return Ok(Redirect::to("/login?oauth=pending").into_response());
        }
        return Ok(with_session_cookie(
            Redirect::to("/").into_response(),
            &state,
            &headers,
            &user.id,
        ));
    }

    match oauth.account_mode {
        OAuthAccountMode::LinkOnly => Ok(Redirect::to("/login?oauth=not_linked").into_response()),
        OAuthAccountMode::Pending | OAuthAccountMode::AutoEnabled => {
            let account_status = if oauth.account_mode == OAuthAccountMode::AutoEnabled {
                AccountStatus::Enabled
            } else {
                AccountStatus::Pending
            };
            let username = unique_oauth_username(&state, &oauth.provider, &userinfo, email)?;
            let user = User {
                id: Uuid::new_v4(),
                username,
                password_hash: None,
                display_name: display_name.unwrap_or_else(|| "OAuth User".to_string()),
                is_admin: false,
                account_status,
                max_screenshot_size_bytes: None,
                max_expiry_seconds: None,
                created_at: Utc::now(),
            };
            let identity = OAuthIdentity {
                id: Uuid::new_v4(),
                user_id: user.id,
                provider: oauth.provider.clone(),
                subject: subject.to_string(),
                email: email.map(str::to_string),
                display_name: Some(user.display_name.clone()),
                created_at: Utc::now(),
                last_login_at: Some(Utc::now()),
            };
            state.db.create_user_with_oauth_identity(&user, &identity)?;

            if account_status.is_enabled() {
                Ok(with_session_cookie(
                    Redirect::to("/").into_response(),
                    &state,
                    &headers,
                    &user.id,
                ))
            } else {
                Ok(Redirect::to("/login?oauth=pending").into_response())
            }
        }
    }
}

fn validate_oauth_config(oauth: &crate::config::OAuthConfig) -> crate::Result<()> {
    if !oauth.enabled {
        return Err(AppError::NotFound);
    }
    if oauth.provider.is_empty() || oauth.client_id.is_empty() || oauth.client_secret.is_empty() {
        return Err(AppError::Internal("OAuth is not fully configured".into()));
    }
    Ok(())
}

async fn resolve_oauth_endpoints(
    oauth: &crate::config::OAuthConfig,
) -> crate::Result<OAuthEndpoints> {
    let discovery = if oauth.discovery_url.is_empty() && oauth.issuer_url.is_empty() {
        None
    } else {
        Some(fetch_openid_discovery(oauth).await?)
    };

    let authorize_url = non_empty(&oauth.authorize_url)
        .or_else(|| {
            discovery
                .as_ref()
                .and_then(|doc| non_empty_opt(&doc.authorization_endpoint))
        })
        .ok_or_else(|| {
            AppError::Internal("OAuth authorization endpoint is not configured".into())
        })?;
    let token_url = non_empty(&oauth.token_url)
        .or_else(|| {
            discovery
                .as_ref()
                .and_then(|doc| non_empty_opt(&doc.token_endpoint))
        })
        .ok_or_else(|| AppError::Internal("OAuth token endpoint is not configured".into()))?;
    let userinfo_url = non_empty(&oauth.userinfo_url)
        .or_else(|| {
            discovery
                .as_ref()
                .and_then(|doc| non_empty_opt(&doc.userinfo_endpoint))
        })
        .ok_or_else(|| AppError::Internal("OAuth userinfo endpoint is not configured".into()))?;

    Ok(OAuthEndpoints {
        authorize_url,
        token_url,
        userinfo_url,
    })
}

async fn fetch_openid_discovery(
    oauth: &crate::config::OAuthConfig,
) -> crate::Result<OpenIdDiscoveryDocument> {
    let discovery_url = openid_discovery_url(oauth).ok_or_else(|| {
        AppError::Internal("OAuth discovery URL or issuer URL is not configured".into())
    })?;
    let response = reqwest::Client::new()
        .get(&discovery_url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("OpenID discovery request failed: {}", e)))?;
    if !response.status().is_success() {
        return Err(AppError::Internal(format!(
            "OpenID discovery request failed with status {}",
            response.status()
        )));
    }
    response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("OpenID discovery response failed: {}", e)))
}

fn openid_discovery_url(oauth: &crate::config::OAuthConfig) -> Option<String> {
    non_empty(&oauth.discovery_url).or_else(|| {
        non_empty(&oauth.issuer_url).map(|issuer| {
            format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            )
        })
    })
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn non_empty_opt(value: &Option<String>) -> Option<String> {
    value.as_deref().and_then(non_empty)
}

fn oauth_redirect_uri(state: &SharedState, headers: &HeaderMap) -> String {
    let oauth = &state.config.auth.oauth;
    if !oauth.redirect_url.is_empty() {
        oauth.redirect_url.clone()
    } else {
        format!(
            "{}/api/auth/oauth/callback",
            crate::routes::get_base_url(&state.config.server.public_url, headers)
        )
    }
}

fn oauth_state_cookie_matches(headers: &HeaderMap, state_token: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookies| {
            cookies.split(';').any(|cookie| {
                cookie
                    .trim()
                    .strip_prefix("oauth_state=")
                    .map(|value| value == state_token)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn oauth_email_allowed(
    oauth: &crate::config::OAuthConfig,
    email: Option<&str>,
    email_verified: Option<bool>,
) -> bool {
    if oauth.allowed_email_domains.is_empty() {
        return true;
    }
    if email_verified == Some(false) {
        return false;
    }
    let Some(email) = email else {
        return false;
    };
    let Some((_, domain)) = email.rsplit_once('@') else {
        return false;
    };
    oauth
        .allowed_email_domains
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(domain))
}

fn oauth_display_name(userinfo: &OAuthUserInfo, email: Option<&str>) -> Option<String> {
    userinfo
        .name
        .as_ref()
        .or(userinfo.preferred_username.as_ref())
        .or(userinfo.login.as_ref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| email.map(|email| email.to_string()))
}

fn json_value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn unique_oauth_username(
    state: &SharedState,
    provider: &str,
    userinfo: &OAuthUserInfo,
    email: Option<&str>,
) -> crate::Result<String> {
    let base = email.map(sanitize_email_username).unwrap_or_else(|| {
        let fallback = userinfo
            .preferred_username
            .as_deref()
            .or(userinfo.login.as_deref())
            .or(userinfo.name.as_deref())
            .unwrap_or(provider);
        sanitize_username(fallback)
    });
    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            format!("{}{}", base, suffix + 1)
        };
        if state.db.get_user_by_username(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(AppError::Internal(
        "Unable to generate a unique OAuth username".into(),
    ))
}

fn sanitize_email_username(value: &str) -> String {
    let email = value.trim().to_ascii_lowercase();
    if email.contains('@') && !email.starts_with('@') && !email.ends_with('@') {
        email
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '@' | '-' | '_' | '.') {
                    c
                } else {
                    '-'
                }
            })
            .filter(|c| !c.is_ascii_control())
            .collect()
    } else {
        sanitize_username(value)
    }
}

fn sanitize_username(value: &str) -> String {
    let username: String = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .filter(|c| !c.is_ascii_control())
        .collect();
    let username = username.trim_matches('-').to_string();
    if username.is_empty() {
        "oauth-user".to_string()
    } else {
        username
    }
}

fn with_session_cookie(
    mut response: Response,
    state: &SharedState,
    headers: &HeaderMap,
    user_id: &Uuid,
) -> Response {
    let token = auth::middleware::create_session_token(
        user_id,
        &state.jwt_secret,
        state.config.auth.session_ttl_seconds,
    );
    let cookie = session_cookie(state, headers, &token);
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie.parse().expect("session cookie should be valid"),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        "oauth_state=; HttpOnly; SameSite=Lax; Path=/api/auth/oauth; Max-Age=0"
            .parse()
            .expect("OAuth state clearing cookie should be valid"),
    );
    response
}

fn session_cookie(state: &SharedState, headers: &HeaderMap, token: &str) -> String {
    let same_site = if request_uses_https(state, headers) {
        "SameSite=None; Secure"
    } else {
        "SameSite=Lax"
    };

    format!(
        "session={}; HttpOnly; {}; Path=/; Max-Age={}",
        token, same_site, state.config.auth.session_ttl_seconds
    )
}

fn request_uses_https(state: &SharedState, headers: &HeaderMap) -> bool {
    if state.config.server.public_url.starts_with("https://") {
        return true;
    }

    if headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .map(|proto| proto.split(',').any(|value| value.trim() == "https"))
        .unwrap_or(false)
    {
        return true;
    }

    if headers
        .get("forwarded")
        .and_then(|h| h.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .any(|part| part.trim().eq_ignore_ascii_case("proto=https"))
        })
        .unwrap_or(false)
    {
        return true;
    }

    headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .map(|origin| origin.starts_with("https://"))
        .unwrap_or(false)
}

// ── Admin users ──

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub is_admin: Option<bool>,
    pub max_screenshot_size_bytes: Option<u64>,
    pub max_expiry_seconds: Option<u64>,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub max_screenshot_size_bytes: Option<Option<u64>>,
    pub max_expiry_seconds: Option<Option<u64>>,
    pub password: Option<String>,
    pub account_status: Option<AccountStatus>,
}

pub async fn admin_list_users(
    State(state): State<SharedState>,
    AdminUser(_admin): AdminUser,
) -> crate::Result<Json<Vec<serde_json::Value>>> {
    let users = state.db.list_users()?;
    let result = users
        .into_iter()
        .map(|user| {
            serde_json::json!({
                "id": user.id,
                "username": user.username,
                "display_name": user.display_name,
                "is_admin": user.is_admin,
                "account_status": user.account_status,
                "max_screenshot_size_bytes": user.max_screenshot_size_bytes,
                "max_expiry_seconds": user.max_expiry_seconds,
                "created_at": user.created_at,
            })
        })
        .collect();
    Ok(Json(result))
}

pub async fn admin_create_user(
    State(state): State<SharedState>,
    AdminUser(_admin): AdminUser,
    Json(req): Json<CreateUserRequest>,
) -> crate::Result<impl IntoResponse> {
    let username = req.username.trim();
    if username.is_empty() || req.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Username required, password must be at least 8 characters".into(),
        ));
    }

    if state.db.get_user_by_username(username)?.is_some() {
        return Err(AppError::BadRequest("Username already exists".into()));
    }

    let password_hash = auth::hash_password(&req.password)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?;
    let user = User {
        id: Uuid::new_v4(),
        username: username.to_string(),
        password_hash: Some(password_hash),
        display_name: req
            .display_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| username.to_string()),
        is_admin: req.is_admin.unwrap_or(false),
        account_status: AccountStatus::Enabled,
        max_screenshot_size_bytes: normalize_user_limit(req.max_screenshot_size_bytes),
        max_expiry_seconds: normalize_user_limit(req.max_expiry_seconds),
        created_at: Utc::now(),
    };

    state.db.create_user(&user)?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": user.id,
            "username": user.username,
            "display_name": user.display_name,
            "is_admin": user.is_admin,
            "account_status": user.account_status,
            "max_screenshot_size_bytes": user.max_screenshot_size_bytes,
            "max_expiry_seconds": user.max_expiry_seconds,
            "created_at": user.created_at,
        })),
    ))
}

pub async fn admin_update_user_limits(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> crate::Result<Json<serde_json::Value>> {
    let user = state.db.get_user_by_id(&id)?.ok_or(AppError::NotFound)?;

    let max_screenshot_size_bytes = req
        .max_screenshot_size_bytes
        .map(normalize_user_limit)
        .unwrap_or(user.max_screenshot_size_bytes);
    let max_expiry_seconds = req
        .max_expiry_seconds
        .map(normalize_user_limit)
        .unwrap_or(user.max_expiry_seconds);
    let updated =
        state
            .db
            .update_user_limits(&id, max_screenshot_size_bytes, max_expiry_seconds)?;
    if !updated {
        return Err(AppError::NotFound);
    }
    if let Some(password) = req.password {
        if password.len() < 8 {
            return Err(AppError::BadRequest(
                "Password must be at least 8 characters".into(),
            ));
        }
        let password_hash = auth::hash_password(&password)
            .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?;
        state.db.update_user_password_hash(&id, &password_hash)?;
    }
    if let Some(account_status) = req.account_status {
        if id == admin.id && !account_status.is_enabled() {
            return Err(AppError::BadRequest(
                "You cannot disable your own user account".into(),
            ));
        }
        if user.is_admin && !account_status.is_enabled() && state.db.admin_count()? <= 1 {
            return Err(AppError::BadRequest(
                "Cannot disable the last admin user".into(),
            ));
        }
        state.db.update_user_account_status(&id, account_status)?;
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "account_status": req.account_status.unwrap_or(user.account_status),
        "max_screenshot_size_bytes": max_screenshot_size_bytes,
        "max_expiry_seconds": max_expiry_seconds,
    })))
}

pub async fn admin_delete_user(
    State(state): State<SharedState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> crate::Result<Json<serde_json::Value>> {
    if id == admin.id {
        return Err(AppError::BadRequest(
            "You cannot delete your own user account".into(),
        ));
    }

    let user = state.db.get_user_by_id(&id)?.ok_or(AppError::NotFound)?;
    if user.is_admin && user.account_status.is_enabled() && state.db.admin_count()? <= 1 {
        return Err(AppError::BadRequest(
            "Cannot delete the last admin user".into(),
        ));
    }

    let paths = state.db.delete_user(&id)?.ok_or(AppError::NotFound)?;
    for (original_path, rendered_path) in paths {
        remove_file_if_present(&original_path);
        if let Some(path) = rendered_path {
            remove_file_if_present(&path);
            let preview_path = image_processing::preview_path_for_rendered_path(&path);
            remove_file_if_present(&preview_path.to_string_lossy());
        }
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

fn remove_file_if_present(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => tracing::warn!(
            "Failed to remove user-owned screenshot file {}: {}",
            path,
            err
        ),
    }
}

fn bake_preview_for_rendered(rendered_path: &std::path::Path) -> crate::Result<()> {
    let preview_path = image_processing::preview_path_for_rendered(rendered_path);
    image_processing::render_preview_image(
        &rendered_path.to_string_lossy(),
        &preview_path.to_string_lossy(),
    )
}

// ── Logout ──

pub async fn logout() -> impl IntoResponse {
    let cookie = "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0";
    (
        [(header::SET_COOKIE, cookie.to_string())],
        Json(serde_json::json!({ "ok": true })),
    )
}

// ── Change password ──

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Json(req): Json<ChangePasswordRequest>,
) -> crate::Result<Json<serde_json::Value>> {
    if req.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "New password must be at least 8 characters".into(),
        ));
    }

    let hash = user
        .password_hash
        .as_deref()
        .ok_or(AppError::Unauthorized)?;
    if !auth::verify_password(&req.current_password, hash) {
        return Err(AppError::Unauthorized);
    }

    let new_hash = auth::hash_password(&req.new_password)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?;
    state.db.update_user_password_hash(&user.id, &new_hash)?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn disconnect_oauth_identity(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
) -> crate::Result<Json<serde_json::Value>> {
    let identities = state.db.list_oauth_identities_for_user(&user.id)?;
    if !identities.iter().any(|identity| identity.id == id) {
        return Err(AppError::NotFound);
    }

    if user.password_hash.is_none() && identities.len() <= 1 {
        return Err(AppError::BadRequest(
            "Add a password before disconnecting your only OAuth identity".into(),
        ));
    }

    state.db.delete_oauth_identity_for_user(&id, &user.id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Screenshot upload ──

pub async fn upload_screenshot(
    State(state): State<SharedState>,
    ApiOrSessionUser(user): ApiOrSessionUser,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> crate::Result<impl IntoResponse> {
    let mut image_data: Option<Vec<u8>> = None;
    let mut filename = "screenshot.png".to_string();
    let mut title: Option<String> = None;
    let mut source_url: Option<String> = None;
    let mut expires_in: Option<String> = None;
    let mut image_dpi: Option<f64> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "image" => {
                if let Some(fname) = field.file_name() {
                    filename = fname.to_string();
                }
                image_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Read error: {}", e)))?
                        .to_vec(),
                );
            }
            "title" => {
                title = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Read error: {}", e)))?,
                );
            }
            "source_url" => {
                source_url = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Read error: {}", e)))?,
                );
            }
            "expires_in" => {
                expires_in = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Read error: {}", e)))?,
                );
            }
            "image_dpi" => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("Read error: {}", e)))?;
                image_dpi = parse_image_dpi(&value);
            }
            _ => {}
        }
    }

    let image_data = image_data.ok_or(AppError::BadRequest("No image provided".into()))?;
    let max_screenshot_size_bytes = effective_max_screenshot_size_bytes(&state, &user);
    if image_data.len() as u64 > max_screenshot_size_bytes {
        return Err(AppError::BadRequest(format!(
            "Screenshot exceeds the maximum size of {}",
            format_bytes(max_screenshot_size_bytes)
        )));
    }

    // Validate it's actually an image
    image::load_from_memory(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".into()))?;
    let image_dpi = image_dpi
        .or_else(|| png_dpi_from_phys_chunk(&image_data))
        .unwrap_or(100.0);

    let id = Uuid::new_v4();
    let sid = share_id::generate();

    // Save original file
    let original_path = state
        .config
        .storage
        .originals_path()
        .join(format!("{}.png", id));
    std::fs::write(&original_path, &image_data)?;

    // Copy as initial rendered version
    let rendered_path = state
        .config
        .storage
        .rendered_path()
        .join(format!("{}.png", sid));
    std::fs::write(&rendered_path, &image_data)?;
    bake_preview_for_rendered(&rendered_path)?;

    let created_at = Utc::now();

    // Calculate expiration
    let expires_at = resolve_expires_at(
        expires_in.as_deref(),
        state.config.auth.default_expiry_seconds,
        effective_max_expiry_seconds(&state, &user),
        created_at,
    )?;

    let screenshot = Screenshot {
        id,
        user_id: user.id,
        share_id: sid.clone(),
        title,
        source_url,
        original_filename: filename,
        original_path: original_path.to_string_lossy().to_string(),
        rendered_path: Some(rendered_path.to_string_lossy().to_string()),
        annotations: vec![],
        crop_rect: None,
        image_dpi,
        visibility: "unlisted".to_string(),
        expires_at,
        created_at,
        updated_at: created_at,
    };

    state.db.create_screenshot(&screenshot)?;

    let base_url = crate::routes::get_base_url(&state.config.server.public_url, &headers);
    let share_url = format!("{}/s/{}", base_url, sid);
    let raw_url = format!("{}/s/{}.png", base_url, sid);

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "share_id": sid,
            "share_url": share_url,
            "raw_url": raw_url,
            "image_dpi": screenshot.image_dpi,
            "created_at": screenshot.created_at,
        })),
    ))
}

fn parse_image_dpi(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|dpi| dpi.is_finite() && *dpi > 0.0)
        .map(normalize_image_dpi)
}

fn normalize_image_dpi(dpi: f64) -> f64 {
    if dpi.is_finite() && dpi > 0.0 {
        dpi.clamp(1.0, 2400.0)
    } else {
        100.0
    }
}

fn png_dpi_from_phys_chunk(data: &[u8]) -> Option<f64> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 8 || &data[..8] != PNG_SIGNATURE {
        return None;
    }

    let mut offset = 8usize;
    while offset.checked_add(12)? <= data.len() {
        let length = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let chunk_type = &data[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length)?;
        let next = data_end.checked_add(4)?;
        if next > data.len() {
            return None;
        }

        if chunk_type == b"pHYs" && length == 9 {
            let x_ppu = u32::from_be_bytes(data[data_start..data_start + 4].try_into().ok()?);
            let y_ppu = u32::from_be_bytes(data[data_start + 4..data_start + 8].try_into().ok()?);
            let unit = data[data_start + 8];
            if unit == 1 && x_ppu > 0 && y_ppu > 0 {
                let avg_pixels_per_meter = (x_ppu as f64 + y_ppu as f64) / 2.0;
                return Some((avg_pixels_per_meter * 0.0254).clamp(1.0, 2400.0));
            }
            return None;
        }

        offset = next;
    }

    None
}

fn normalize_user_limit(value: Option<u64>) -> Option<u64> {
    value.filter(|v| *v > 0 && i64::try_from(*v).is_ok())
}

fn effective_max_screenshot_size_bytes(state: &SharedState, user: &User) -> u64 {
    user.max_screenshot_size_bytes
        .unwrap_or(state.config.server.max_screenshot_size_bytes)
}

fn effective_max_expiry_seconds(state: &SharedState, user: &User) -> Option<u64> {
    user.max_expiry_seconds
        .or(state.config.server.max_expiry_seconds)
}

fn resolve_expires_at(
    requested: Option<&str>,
    default_expiry_seconds: Option<u64>,
    max_expiry_seconds: Option<u64>,
    base_time: chrono::DateTime<Utc>,
) -> crate::Result<Option<chrono::DateTime<Utc>>> {
    let seconds = match requested {
        Some(value) => parse_expiry_seconds(value)?,
        None => default_expiry_seconds,
    };
    let Some(seconds) = seconds else {
        return Ok(None);
    };
    let capped_seconds = max_expiry_seconds.map_or(seconds, |max| seconds.min(max));
    if i64::try_from(capped_seconds).is_err() {
        return Err(AppError::BadRequest("Expiry value is too large".into()));
    }
    Ok(Some(
        base_time + chrono::Duration::seconds(capped_seconds as i64),
    ))
}

fn parse_expiry_seconds(s: &str) -> crate::Result<Option<u64>> {
    let s = s.trim();
    if s.is_empty() || s == "0" || s == "never" {
        return Ok(None);
    }

    // Parse formats like "30d", "24h", "1w"
    if s.len() < 2 {
        return Err(AppError::BadRequest("Invalid expiry value".into()));
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: u64 = num_str
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid expiry value".into()))?;
    let multiplier = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err(AppError::BadRequest("Invalid expiry value".into())),
    };
    let seconds = num
        .checked_mul(multiplier)
        .filter(|seconds| i64::try_from(*seconds).is_ok())
        .ok_or_else(|| AppError::BadRequest("Expiry value is too large".into()))?;
    Ok(Some(seconds))
}

fn format_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{openid_discovery_url, sanitize_email_username, sanitize_username};
    use crate::config::OAuthConfig;

    #[test]
    fn oauth_email_username_keeps_full_email_address() {
        assert_eq!(
            sanitize_email_username("Alice.Example+tag@Example.COM"),
            "alice.example-tag@example.com"
        );
    }

    #[test]
    fn oauth_non_email_username_uses_regular_sanitizer() {
        assert_eq!(sanitize_email_username("Not An Email"), "not-an-email");
        assert_eq!(sanitize_username("Display Name"), "display-name");
    }

    #[test]
    fn openid_discovery_url_prefers_explicit_discovery_url() {
        let oauth = OAuthConfig {
            issuer_url: "https://issuer.example".to_string(),
            discovery_url: "https://issuer.example/custom-discovery".to_string(),
            ..OAuthConfig::default()
        };

        assert_eq!(
            openid_discovery_url(&oauth),
            Some("https://issuer.example/custom-discovery".to_string())
        );
    }

    #[test]
    fn openid_discovery_url_uses_issuer_url() {
        let oauth = OAuthConfig {
            issuer_url: "https://issuer.example/tenant/".to_string(),
            ..OAuthConfig::default()
        };

        assert_eq!(
            openid_discovery_url(&oauth),
            Some("https://issuer.example/tenant/.well-known/openid-configuration".to_string())
        );
    }
}

// ── List screenshots ──

#[derive(Deserialize)]
pub struct ListParams {
    pub page: Option<usize>,
    pub per_page: Option<usize>,
}

#[derive(Serialize)]
pub struct ListResponse {
    pub screenshots: Vec<ScreenshotSummary>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

#[derive(Serialize)]
pub struct ScreenshotSummary {
    pub id: Uuid,
    pub share_id: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub visibility: String,
    pub share_url: String,
    pub raw_url: String,
    pub created_at: chrono::DateTime<Utc>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

pub async fn list_screenshots(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    Query(params): Query<ListParams>,
) -> crate::Result<Json<ListResponse>> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let screenshots = state
        .db
        .list_screenshots_for_user(&user.id, per_page, offset)?;
    let total = state.db.screenshot_count_for_user(&user.id)?;

    let base_url = crate::routes::get_base_url(&state.config.server.public_url, &headers);
    let summaries: Vec<ScreenshotSummary> = screenshots
        .into_iter()
        .map(|s| {
            let share_url = format!("{}/s/{}", base_url, s.share_id);
            let raw_url = format!("{}/s/{}.png", base_url, s.share_id);
            ScreenshotSummary {
                id: s.id,
                share_id: s.share_id,
                title: s.title,
                source_url: s.source_url,
                visibility: s.visibility,
                share_url,
                raw_url,
                created_at: s.created_at,
                expires_at: s.expires_at,
            }
        })
        .collect();

    Ok(Json(ListResponse {
        screenshots: summaries,
        total,
        page,
        per_page,
    }))
}

// ── Update screenshot metadata ──

#[derive(Deserialize)]
pub struct UpdateRequest {
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub visibility: Option<String>,
    pub expires_in: Option<String>,
    pub image_dpi: Option<f64>,
}

pub async fn update_screenshot(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRequest>,
) -> crate::Result<Json<serde_json::Value>> {
    let screenshot = state
        .db
        .get_screenshot_by_id(&id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.user_id != user.id {
        return Err(AppError::NotFound);
    }

    if let Some(vis) = &req.visibility {
        if vis != "unlisted" && vis != "private" {
            return Err(AppError::BadRequest("Invalid visibility value".into()));
        }
    }

    let expires_at = match req.expires_in.as_deref() {
        Some(value) => Some(resolve_expires_at(
            Some(value),
            state.config.auth.default_expiry_seconds,
            effective_max_expiry_seconds(&state, &user),
            screenshot.created_at,
        )?),
        None => None,
    };
    let source_url = req.source_url.as_ref().map(|url| {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let image_dpi = req.image_dpi.map(normalize_image_dpi);
    let dpi_changed = image_dpi
        .map(|dpi| (dpi - screenshot.image_dpi).abs() > f64::EPSILON)
        .unwrap_or(false);

    state.db.update_screenshot_metadata(
        &id,
        req.title.as_deref(),
        source_url,
        req.visibility.as_deref(),
        expires_at,
        image_dpi,
    )?;

    if let Some(image_dpi) = image_dpi.filter(|_| dpi_changed) {
        let rendered_path = state
            .config
            .storage
            .rendered_path()
            .join(format!("{}.png", screenshot.share_id));
        let rendered_path_str = rendered_path.to_string_lossy().to_string();

        image_processing::render_screenshot(
            &screenshot.original_path,
            &rendered_path_str,
            &screenshot.annotations,
            &screenshot.crop_rect,
            image_dpi,
        )?;
        bake_preview_for_rendered(&rendered_path)?;

        state
            .db
            .update_screenshot_rendered_path(&id, &rendered_path_str)?;
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Delete screenshot ──

pub async fn delete_screenshot(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
) -> crate::Result<Json<serde_json::Value>> {
    let screenshot = state
        .db
        .get_screenshot_by_id(&id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.user_id != user.id {
        return Err(AppError::NotFound);
    }

    // Delete files
    let _ = std::fs::remove_file(&screenshot.original_path);
    if let Some(rp) = &screenshot.rendered_path {
        let _ = std::fs::remove_file(rp);
        let _ = std::fs::remove_file(image_processing::preview_path_for_rendered_path(rp));
    }

    state.db.delete_screenshot(&id)?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Save annotations ──

#[derive(Deserialize)]
pub struct SaveAnnotationsRequest {
    pub annotations: Vec<Annotation>,
    pub crop: Option<CropRect>,
}

pub async fn save_annotations(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<SaveAnnotationsRequest>,
) -> crate::Result<Json<serde_json::Value>> {
    let screenshot = state
        .db
        .get_screenshot_by_id(&id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.user_id != user.id {
        return Err(AppError::NotFound);
    }

    // Save annotations to DB
    state
        .db
        .update_screenshot_annotations(&id, &req.annotations, &req.crop)?;

    // Re-render the public image
    let rendered_path = state
        .config
        .storage
        .rendered_path()
        .join(format!("{}.png", screenshot.share_id));

    let rendered_path_str = rendered_path.to_string_lossy().to_string();

    image_processing::render_screenshot(
        &screenshot.original_path,
        &rendered_path_str,
        &req.annotations,
        &req.crop,
        screenshot.image_dpi,
    )?;
    bake_preview_for_rendered(&rendered_path)?;

    state
        .db
        .update_screenshot_rendered_path(&id, &rendered_path_str)?;

    let base_url = crate::routes::get_base_url(&state.config.server.public_url, &headers);
    Ok(Json(serde_json::json!({
        "ok": true,
        "rendered_url": format!("{}/s/{}.png", base_url, screenshot.share_id),
    })))
}

// ── Serve original image (for editor) ──

pub async fn serve_original(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
) -> crate::Result<impl IntoResponse> {
    let screenshot = state
        .db
        .get_screenshot_by_id(&id)?
        .ok_or(AppError::NotFound)?;

    if screenshot.user_id != user.id {
        return Err(AppError::NotFound);
    }

    let data = std::fs::read(&screenshot.original_path)?;

    Ok(([(header::CONTENT_TYPE, "image/png".to_string())], data))
}

// ── API Tokens ──

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub label: Option<String>,
}

pub async fn create_token(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateTokenRequest>,
) -> crate::Result<impl IntoResponse> {
    let raw_token = share_id::generate_api_token();
    let token_hash = auth::hash_token(&raw_token);

    let token = ApiToken {
        id: Uuid::new_v4(),
        user_id: user.id,
        token_hash,
        label: req.label.unwrap_or_default(),
        created_at: Utc::now(),
        last_used_at: None,
        expires_at: None,
    };

    state.db.create_api_token(&token)?;

    // Return the raw token only this once — it's stored hashed
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": token.id,
            "token": raw_token,
            "label": token.label,
            "created_at": token.created_at,
        })),
    ))
}

pub async fn list_tokens(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
) -> crate::Result<Json<Vec<serde_json::Value>>> {
    let tokens = state.db.list_tokens_for_user(&user.id)?;
    let result: Vec<serde_json::Value> = tokens
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "label": t.label,
                "created_at": t.created_at,
                "last_used_at": t.last_used_at,
                "expires_at": t.expires_at,
            })
        })
        .collect();
    Ok(Json(result))
}

pub async fn revoke_token(
    State(state): State<SharedState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
) -> crate::Result<Json<serde_json::Value>> {
    let deleted = state.db.delete_token(&id, &user.id)?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Ping ──

pub async fn ping(
    ApiOrSessionUser(_user): ApiOrSessionUser,
) -> crate::Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "ok": true })))
}
