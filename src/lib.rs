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
    routing::{delete, get, patch, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

/// Shared application state accessible from all route handlers.
pub struct AppState {
    pub db: db::Database,
    pub config: config::Config,
    pub jwt_secret: String,
}

pub type SharedState = Arc<AppState>;

/// Build the full Axum router with all routes.
pub fn build_router(state: SharedState) -> Router {
    let public_routes = Router::new()
        .route("/s/{share_id_or_file}", get(routes::share::share_dispatch));

    let auth_pages = Router::new()
        .route("/", get(routes::pages::dashboard))
        .route("/setup", get(routes::pages::setup_page))
        .route("/login", get(routes::pages::login_page))
        .route(
            "/screenshots/{id}/edit",
            get(routes::pages::editor_page),
        )
        .route("/settings", get(routes::pages::settings_page));

    let api_routes = Router::new()
        .route("/api/auth/setup", post(routes::api::setup))
        .route("/api/auth/login", post(routes::api::login))
        .route("/api/auth/logout", post(routes::api::logout))
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
        .route(
            "/api/auth/tokens/{id}",
            delete(routes::api::revoke_token),
        );

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(public_routes)
        .merge(auth_pages)
        .merge(api_routes)
        .nest_service("/static", ServeDir::new("static"))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
