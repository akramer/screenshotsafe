use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::middleware::{ApiOrSessionUser, AuthUser};
use crate::models::{Annotation, ApiToken, CropRect, Screenshot, User};
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
    Json(req): Json<SetupRequest>,
) -> crate::Result<impl IntoResponse> {
    // Only allow setup if no users exist
    if state.db.user_count()? > 0 {
        return Err(AppError::BadRequest("Setup already completed".into()));
    }

    if req.username.is_empty() || req.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Username required, password must be at least 8 characters".into(),
        ));
    }

    let password_hash = auth::hash_password(&req.password)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?;

    let user = User {
        id: Uuid::new_v4(),
        username: req.username.clone(),
        password_hash: Some(password_hash),
        display_name: req.display_name.unwrap_or_else(|| req.username.clone()),
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

    let cookie = format!(
        "session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        token, state.config.auth.session_ttl_seconds
    );

    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "ok": true,
            "user": {
                "id": user.id,
                "username": user.username,
                "display_name": user.display_name,
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
    Json(req): Json<LoginRequest>,
) -> crate::Result<impl IntoResponse> {
    let user = state
        .db
        .get_user_by_username(&req.username)?
        .ok_or(AppError::Unauthorized)?;

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

    let cookie = format!(
        "session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        token, state.config.auth.session_ttl_seconds
    );

    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "ok": true,
            "user": {
                "id": user.id,
                "username": user.username,
                "display_name": user.display_name,
            }
        })),
    ))
}

// ── Logout ──

pub async fn logout() -> impl IntoResponse {
    let cookie = "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0";
    (
        [(header::SET_COOKIE, cookie.to_string())],
        Json(serde_json::json!({ "ok": true })),
    )
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
            _ => {}
        }
    }

    let image_data = image_data.ok_or(AppError::BadRequest("No image provided".into()))?;

    // Validate it's actually an image
    image::load_from_memory(&image_data)
        .map_err(|_| AppError::BadRequest("Invalid image data".into()))?;

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

    // Calculate expiration
    let expires_at = parse_expires_in(expires_in.as_deref()).or_else(|| {
        state
            .config
            .auth
            .default_expiry_seconds
            .map(|s| Utc::now() + chrono::Duration::seconds(s as i64))
    });

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
        visibility: "unlisted".to_string(),
        expires_at,
        created_at: Utc::now(),
        updated_at: Utc::now(),
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
            "created_at": screenshot.created_at,
        })),
    ))
}

fn parse_expires_in(s: Option<&str>) -> Option<chrono::DateTime<Utc>> {
    let s = s?;
    let s = s.trim();
    if s.is_empty() || s == "0" || s == "never" {
        return None;
    }

    // Parse formats like "30d", "24h", "1w"
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    let seconds = match unit {
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        "w" => num * 604800,
        _ => return None,
    };
    Some(Utc::now() + chrono::Duration::seconds(seconds))
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
    pub visibility: Option<String>,
    pub expires_in: Option<String>,
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

    let expires_at = req.expires_in.as_deref().map(|s| parse_expires_in(Some(s)));

    state.db.update_screenshot_metadata(
        &id,
        req.title.as_deref(),
        req.visibility.as_deref(),
        expires_at,
    )?;

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
    )?;

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
