pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod image_processing;
pub mod models;
pub mod routes;
pub mod share_id;

pub use error::{AppError, Result};

use std::sync::Arc;

use axum::{
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

const FAVICON_PNG: &[u8] = include_bytes!("../extension/icons/icon128.png");

/// Shared application state accessible from all route handlers.
pub struct AppState {
    pub db: db::Database,
    pub config: config::Config,
    pub jwt_secret: String,
}

pub type SharedState = Arc<AppState>;

/// Remove expired screenshot records and their backing image files.
pub fn cleanup_expired_screenshots(state: &AppState) -> Result<usize> {
    let paths = state.db.delete_expired_screenshots()?;
    let deleted_count = paths.len();

    for (original_path, rendered_path) in paths {
        remove_screenshot_file(&original_path);
        if let Some(path) = rendered_path {
            remove_screenshot_file(&path);
        }
    }

    Ok(deleted_count)
}

fn remove_screenshot_file(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => tracing::warn!("Failed to remove expired screenshot file {}: {}", path, err),
    }
}

/// Start a background task that periodically deletes expired screenshots.
pub fn spawn_expired_screenshot_cleanup(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;
            match cleanup_expired_screenshots(&state) {
                Ok(0) => {}
                Ok(count) => tracing::info!("Deleted {} expired screenshots", count),
                Err(err) => tracing::warn!("Failed to delete expired screenshots: {}", err),
            }
        }
    });
}

/// Build the full Axum router with all routes.
pub fn build_router(state: SharedState) -> Router {
    let public_routes =
        Router::new().route("/s/{share_id_or_file}", get(routes::share::share_dispatch));

    let auth_pages = Router::new()
        .route("/", get(routes::pages::dashboard))
        .route("/setup", get(routes::pages::setup_page))
        .route("/login", get(routes::pages::login_page))
        .route("/screenshots/{id}/edit", get(routes::pages::editor_page))
        .route("/settings", get(routes::pages::settings_page))
        .route("/admin", get(routes::pages::admin_page));

    let api_routes = Router::new()
        .route("/api/ping", get(routes::api::ping))
        .route("/api/auth/setup", post(routes::api::setup))
        .route("/api/auth/login", post(routes::api::login))
        .route("/api/auth/logout", post(routes::api::logout))
        .route("/api/auth/password", put(routes::api::change_password))
        .route("/api/admin/users", get(routes::api::admin_list_users))
        .route("/api/admin/users", post(routes::api::admin_create_user))
        .route(
            "/api/admin/users/{id}",
            delete(routes::api::admin_delete_user),
        )
        .route("/api/screenshots", post(routes::api::upload_screenshot))
        .route("/api/screenshots", get(routes::api::list_screenshots))
        .route(
            "/api/screenshots/{id}",
            patch(routes::api::update_screenshot),
        )
        .route(
            "/api/screenshots/{id}",
            delete(routes::api::delete_screenshot),
        )
        .route(
            "/api/screenshots/{id}/annotations",
            put(routes::api::save_annotations),
        )
        .route(
            "/api/screenshots/{id}/original",
            get(routes::api::serve_original),
        )
        .route("/api/auth/tokens", post(routes::api::create_token))
        .route("/api/auth/tokens", get(routes::api::list_tokens))
        .route("/api/auth/tokens/{id}", delete(routes::api::revoke_token));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(public_routes)
        .merge(auth_pages)
        .merge(api_routes)
        .route("/favicon.ico", get(favicon))
        .nest_service("/static", ServeDir::new("static"))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn favicon() -> Response {
    let mut response = FAVICON_PNG.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/png"),
    );
    response
}
