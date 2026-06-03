#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, StatusCode};
    use chrono::{Duration, Utc};
    use tower::ServiceExt;

    use screenshotsafe::config::Config;
    use screenshotsafe::db::Database;
    use screenshotsafe::models::{AccountStatus, OAuthIdentity, User};
    use screenshotsafe::*;

    /// Create a test app with an in-memory database and temp storage.
    fn test_app(dir: &std::path::Path) -> (axum::Router, SharedState) {
        test_app_with_config(dir, |config| {
            config.server.public_url = "http://localhost:8080".to_string();
        })
    }

    fn test_app_with_config(
        dir: &std::path::Path,
        configure: impl FnOnce(&mut Config),
    ) -> (axum::Router, SharedState) {
        let db = Database::open_in_memory().unwrap();
        db.run_migrations().unwrap();

        let storage_path = dir.join("storage");
        std::fs::create_dir_all(storage_path.join("originals")).unwrap();
        std::fs::create_dir_all(storage_path.join("rendered")).unwrap();

        let mut config = Config::default();
        config.storage.path = storage_path.to_string_lossy().to_string();
        configure(&mut config);

        let state = Arc::new(AppState {
            db,
            config,
            jwt_secret: "test-secret-key-for-jwt".to_string(),
        });

        let app = build_router(state.clone());
        (app, state)
    }

    /// Helper: create a JSON request.
    fn json_request(method: &str, uri: &str, body: serde_json::Value) -> axum::http::Request<Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap()
    }

    /// Helper: create an authenticated JSON request.
    fn authed_json_request(
        method: &str,
        uri: &str,
        cookie: &str,
        body: serde_json::Value,
    ) -> axum::http::Request<Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, cookie)
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap()
    }

    fn authed_request(method: &str, uri: &str, cookie: &str) -> axum::http::Request<Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap()
    }

    /// Helper: get response body as JSON.
    async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Helper: get response body as text.
    async fn body_text(response: axum::http::Response<Body>) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Helper: extract session cookie from response.
    fn extract_session_cookie(response: &axum::http::Response<Body>) -> Option<String> {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .find_map(|v| {
                let s = v.to_str().ok()?;
                if s.starts_with("session=") {
                    Some(s.to_string())
                } else {
                    None
                }
            })
    }

    // ── Setup & Auth Tests ──

    #[tokio::test]
    async fn test_setup_creates_user_and_returns_session() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        // No users initially
        assert_eq!(state.db.user_count().unwrap(), 0);

        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123",
                "display_name": "Admin User"
            }),
        );

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Should have set a session cookie
        let cookie = extract_session_cookie(&resp);
        assert!(cookie.is_some(), "Should set session cookie");

        let body = body_json(resp).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["user"]["username"], "admin");
        assert_eq!(body["user"]["display_name"], "Admin User");

        // User should exist now
        assert_eq!(state.db.user_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_setup_rejects_second_call() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // First setup
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Second setup should fail
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin2",
                "password": "testpassword456"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_setup_validates_password_length() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "short"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_login_success() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Setup first
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        app.clone().oneshot(req).await.unwrap();

        // Login
        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let cookie = extract_session_cookie(&resp);
        assert!(cookie.is_some(), "Login should set session cookie");
    }

    #[tokio::test]
    async fn test_login_page_shows_extension_login_message() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        app.clone().oneshot(req).await.unwrap();

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/login?extension=login_required")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_text(resp).await;
        assert!(body.contains("Extension not able to access ScreenshotSafe"));
    }

    #[tokio::test]
    async fn test_https_login_cookie_allows_cross_site_extension_use() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |config| {
            config.server.public_url = "https://screens.example".to_string();
        });

        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        let cookie = extract_session_cookie(&resp).unwrap();

        assert!(cookie.contains("SameSite=None"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn test_https_login_cookie_can_be_inferred_from_forwarded_proto() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |_| {});

        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        )
        .map(|body| body);
        let (mut parts, body) = req.into_parts();
        parts
            .headers
            .insert("x-forwarded-proto", "https".parse().unwrap());
        let req = axum::http::Request::from_parts(parts, body);

        let resp = app.clone().oneshot(req).await.unwrap();
        let cookie = extract_session_cookie(&resp).unwrap();

        assert!(cookie.contains("SameSite=None"));
        assert!(cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn test_https_login_cookie_can_be_inferred_from_origin() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |_| {});

        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        )
        .map(|body| body);
        let (mut parts, body) = req.into_parts();
        parts
            .headers
            .insert(header::ORIGIN, "https://screens.example".parse().unwrap());
        let req = axum::http::Request::from_parts(parts, body);

        let resp = app.clone().oneshot(req).await.unwrap();
        let cookie = extract_session_cookie(&resp).unwrap();

        assert!(cookie.contains("SameSite=None"));
        assert!(cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Setup first
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        app.clone().oneshot(req).await.unwrap();

        // Login with wrong password
        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "admin",
                "password": "wrongpassword"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_login_nonexistent_user() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Setup first
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        app.clone().oneshot(req).await.unwrap();

        // Login as nonexistent user
        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "nobody",
                "password": "testpassword123"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_change_password_updates_login_password() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());
        let cookie = setup_user(&app).await;

        let req = authed_json_request(
            "PUT",
            "/api/auth/password",
            &cookie,
            serde_json::json!({
                "current_password": "testpassword123",
                "new_password": "newpassword456"
            }),
        );

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "admin",
                "password": "newpassword456"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_change_password_rejects_wrong_current_password() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());
        let cookie = setup_user(&app).await;

        let req = authed_json_request(
            "PUT",
            "/api/auth/password",
            &cookie,
            serde_json::json!({
                "current_password": "wrongpassword",
                "new_password": "newpassword456"
            }),
        );

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_change_password_validates_new_password_length() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());
        let cookie = setup_user(&app).await;

        let req = authed_json_request(
            "PUT",
            "/api/auth/password",
            &cookie,
            serde_json::json!({
                "current_password": "testpassword123",
                "new_password": "short"
            }),
        );

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_setup_creates_admin_user() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let cookie = setup_user(&app).await;

        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();
        assert!(admin.is_admin);

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/admin")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_can_create_list_and_delete_users() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let cookie = setup_user(&app).await;

        let req = authed_json_request(
            "POST",
            "/api/admin/users",
            &cookie,
            serde_json::json!({
                "username": "alice",
                "password": "testpassword456",
                "display_name": "Alice Example",
                "is_admin": false
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        assert_eq!(created["username"], "alice");
        assert_eq!(created["is_admin"], false);

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/admin/users")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let users = body_json(resp).await;
        assert_eq!(users.as_array().unwrap().len(), 2);

        let alice = state.db.get_user_by_username("alice").unwrap().unwrap();
        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/api/admin/users/{}", alice.id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.db.get_user_by_username("alice").unwrap().is_none());
    }

    #[tokio::test]
    async fn test_admin_edit_user_page() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let cookie = setup_user(&app).await;
        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();

        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/admin/users/{}", admin.id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains(r#"id="max-screenshot-size-bytes""#));
        assert!(html.contains(r#"id="max-expiry-seconds""#));
        assert!(html.contains(r#"id="password-reset-form""#));
    }

    #[tokio::test]
    async fn test_admin_can_reset_user_password_without_clearing_limits() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let admin_cookie = setup_user(&app).await;

        let req = authed_json_request(
            "POST",
            "/api/admin/users",
            &admin_cookie,
            serde_json::json!({
                "username": "alice",
                "password": "testpassword456",
                "is_admin": false,
                "max_screenshot_size_bytes": 12345,
                "max_expiry_seconds": 3600
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let alice = state.db.get_user_by_username("alice").unwrap().unwrap();

        let req = authed_json_request(
            "PATCH",
            &format!("/api/admin/users/{}", alice.id),
            &admin_cookie,
            serde_json::json!({ "password": "newpassword789" }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let alice = state.db.get_user_by_username("alice").unwrap().unwrap();
        assert_eq!(alice.max_screenshot_size_bytes, Some(12345));
        assert_eq!(alice.max_expiry_seconds, Some(3600));

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "alice",
                "password": "testpassword456"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "alice",
                "password": "newpassword789"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_non_admin_cannot_manage_users() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());
        let admin_cookie = setup_user(&app).await;

        let req = authed_json_request(
            "POST",
            "/api/admin/users",
            &admin_cookie,
            serde_json::json!({
                "username": "bob",
                "password": "testpassword456",
                "is_admin": false
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "bob",
                "password": "testpassword456"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let user_cookie = extract_session_cookie(&resp).unwrap();

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/admin/users")
            .header(header::COOKIE, &user_cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_admin_cannot_delete_self() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let cookie = setup_user(&app).await;
        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();

        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/api/admin/users/{}", admin.id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(state.db.get_user_by_username("admin").unwrap().is_some());
    }

    #[tokio::test]
    async fn test_pending_user_cannot_login_until_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let admin_cookie = setup_user(&app).await;

        let req = authed_json_request(
            "POST",
            "/api/admin/users",
            &admin_cookie,
            serde_json::json!({
                "username": "pending-user",
                "password": "testpassword456",
                "is_admin": false
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let user = state
            .db
            .get_user_by_username("pending-user")
            .unwrap()
            .unwrap();
        state
            .db
            .update_user_account_status(&user.id, AccountStatus::Pending)
            .unwrap();

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "pending-user",
                "password": "testpassword456"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let req = authed_json_request(
            "PATCH",
            &format!("/api/admin/users/{}", user.id),
            &admin_cookie,
            serde_json::json!({ "account_status": "enabled" }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = json_request(
            "POST",
            "/api/auth/login",
            serde_json::json!({
                "username": "pending-user",
                "password": "testpassword456"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_cannot_disable_self() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let cookie = setup_user(&app).await;
        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();

        let req = authed_json_request(
            "PATCH",
            &format!("/api/admin/users/{}", admin.id),
            &cookie,
            serde_json::json!({ "account_status": "disabled" }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_user_can_disconnect_oauth_identity_when_password_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let cookie = setup_user(&app).await;
        let user = state.db.get_user_by_username("admin").unwrap().unwrap();
        let identity = OAuthIdentity {
            id: uuid::Uuid::new_v4(),
            user_id: user.id,
            provider: "test".to_string(),
            subject: "subject-1".to_string(),
            email: Some("admin@example.com".to_string()),
            display_name: Some("Admin".to_string()),
            created_at: Utc::now(),
            last_login_at: None,
        };
        state.db.create_oauth_identity(&identity).unwrap();

        let req = authed_request(
            "DELETE",
            &format!("/api/auth/oauth/identities/{}", identity.id),
            &cookie,
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state
            .db
            .list_oauth_identities_for_user(&user.id)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn test_oauth_only_user_cannot_disconnect_only_oauth_identity() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());
        let user = User {
            id: uuid::Uuid::new_v4(),
            username: "oauth@example.com".to_string(),
            password_hash: None,
            display_name: "OAuth User".to_string(),
            is_admin: false,
            account_status: AccountStatus::Enabled,
            max_screenshot_size_bytes: None,
            max_expiry_seconds: None,
            created_at: Utc::now(),
        };
        let identity = OAuthIdentity {
            id: uuid::Uuid::new_v4(),
            user_id: user.id,
            provider: "test".to_string(),
            subject: "subject-1".to_string(),
            email: Some("oauth@example.com".to_string()),
            display_name: Some("OAuth User".to_string()),
            created_at: Utc::now(),
            last_login_at: None,
        };
        state
            .db
            .create_user_with_oauth_identity(&user, &identity)
            .unwrap();
        let session = auth::middleware::create_session_token(
            &user.id,
            &state.jwt_secret,
            state.config.auth.session_ttl_seconds,
        );
        let cookie = format!("session={}", session);

        let req = authed_request(
            "DELETE",
            &format!("/api/auth/oauth/identities/{}", identity.id),
            &cookie,
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            state
                .db
                .list_oauth_identities_for_user(&user.id)
                .unwrap()
                .len(),
            1
        );
    }

    // ── Screenshot Tests ──

    /// Helper: set up a user and return the session cookie string.
    async fn setup_user(app: &axum::Router) -> String {
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        extract_session_cookie(&resp).unwrap()
    }

    /// Helper: create a minimal PNG in memory.
    fn minimal_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 0, 0, 255]));
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            100,
            100,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        buf
    }

    /// Helper: upload a screenshot and return the response body.
    async fn upload_screenshot(app: &axum::Router, cookie: &str) -> serde_json::Value {
        let resp = upload_screenshot_response(app, cookie, &[]).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        body_json(resp).await
    }

    async fn upload_screenshot_response(
        app: &axum::Router,
        cookie: &str,
        fields: &[(&str, &str)],
    ) -> axum::http::Response<Body> {
        upload_screenshot_response_with_origin(app, cookie, fields, None).await
    }

    async fn upload_screenshot_response_with_origin(
        app: &axum::Router,
        cookie: &str,
        fields: &[(&str, &str)],
        origin: Option<&str>,
    ) -> axum::http::Response<Body> {
        let png_data = minimal_png();
        let boundary = "----TestBoundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"test.png\"\r\nContent-Type: image/png\r\n\r\n",
            boundary = boundary,
        );
        let mut body_bytes = body.into_bytes();
        body_bytes.extend_from_slice(&png_data);
        body_bytes
            .extend_from_slice(format!("\r\n--{boundary}\r\n", boundary = boundary).as_bytes());
        let mut all_fields = vec![("title", "Test Screenshot")];
        all_fields.extend_from_slice(fields);
        for (idx, (name, value)) in all_fields.iter().enumerate() {
            body_bytes.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}",
                    name, value
                )
                .as_bytes(),
            );
            if idx + 1 == all_fields.len() {
                body_bytes.extend_from_slice(
                    format!("\r\n--{boundary}--\r\n", boundary = boundary).as_bytes(),
                );
            } else {
                body_bytes.extend_from_slice(
                    format!("\r\n--{boundary}\r\n", boundary = boundary).as_bytes(),
                );
            }
        }

        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri("/api/screenshots")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .header(header::COOKIE, cookie);

        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }

        let req = builder.body(Body::from(body_bytes)).unwrap();

        app.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn test_upload_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let body = upload_screenshot(&app, &cookie).await;
        let id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();

        assert!(body["id"].is_string());
        assert!(body["share_id"].is_string());
        assert!(body["share_url"].as_str().unwrap().contains("/s/"));
        assert!(body["raw_url"].as_str().unwrap().ends_with(".png"));

        let screenshot = state.db.get_screenshot_by_id(&id).unwrap().unwrap();
        let rendered_path = screenshot.rendered_path.unwrap();
        let preview_path = image_processing::preview_path_for_rendered_path(&rendered_path);
        assert!(preview_path.exists());
        assert_eq!(
            std::fs::metadata(&preview_path).unwrap().len(),
            std::fs::metadata(&rendered_path).unwrap().len()
        );
    }

    #[tokio::test]
    async fn test_cookie_api_auth_rejects_untrusted_origin() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let resp = upload_screenshot_response_with_origin(
            &app,
            &cookie,
            &[],
            Some("https://attacker.example"),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_cookie_api_auth_allows_safari_extension_origin() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |config| {
            config.server.public_url = "https://screens.example".to_string();
        });

        let origin = "safari-web-extension://d644d2b6-fa70-416c-b57f-79871710eed6";
        let cookie = setup_user(&app).await;
        let resp = upload_screenshot_response_with_origin(&app, &cookie, &[], Some(origin)).await;

        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_cookie_api_auth_allows_chrome_extension_origin() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |config| {
            config.server.public_url = "https://screens.example".to_string();
        });

        let cookie = setup_user(&app).await;
        let resp = upload_screenshot_response_with_origin(
            &app,
            &cookie,
            &[],
            Some("chrome-extension://abcdefghijklmnopabcdefghijklmnop"),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_cors_allows_credentials_for_safari_extension_origin() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |config| {
            config.server.public_url = "https://screens.example".to_string();
        });

        let origin = "safari-web-extension://d644d2b6-fa70-416c-b57f-79871710eed6";
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/api/ping")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "x-screenshotsafe-debug",
            )
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some(origin)
        );
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .and_then(|v| v.to_str().ok()),
            Some("true")
        );
    }

    #[tokio::test]
    async fn test_cors_allows_credentials_for_chrome_extension_origin() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app_with_config(dir.path(), |config| {
            config.server.public_url = "https://screens.example".to_string();
        });

        let origin = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/api/ping")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some(origin)
        );
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .and_then(|v| v.to_str().ok()),
            Some("true")
        );
    }

    #[tokio::test]
    async fn test_upload_rejects_per_user_screenshot_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();
        state
            .db
            .update_user_limits(&admin.id, Some(10), None)
            .unwrap();

        let resp = upload_screenshot_response(&app, &cookie, &[]).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert!(body["error"].as_str().unwrap().contains("maximum size"));
    }

    #[tokio::test]
    async fn test_upload_clamps_per_user_expiry_limit_from_creation_time() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();
        state
            .db
            .update_user_limits(&admin.id, None, Some(3600))
            .unwrap();

        let resp = upload_screenshot_response(&app, &cookie, &[("expires_in", "2h")]).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = body_json(resp).await;
        let id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();
        let screenshot = state.db.get_screenshot_by_id(&id).unwrap().unwrap();
        assert_eq!(
            screenshot.expires_at.unwrap(),
            screenshot.created_at + Duration::seconds(3600)
        );
    }

    #[tokio::test]
    async fn test_update_clamps_expiry_limit_from_creation_time() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let admin = state.db.get_user_by_username("admin").unwrap().unwrap();
        state
            .db
            .update_user_limits(&admin.id, None, Some(3600))
            .unwrap();

        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();
        let parsed_id: uuid::Uuid = id.parse().unwrap();
        let created_at = state
            .db
            .get_screenshot_by_id(&parsed_id)
            .unwrap()
            .unwrap()
            .created_at;

        let req = authed_json_request(
            "PATCH",
            &format!("/api/screenshots/{}", id),
            &cookie,
            serde_json::json!({ "expires_in": "2h" }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screenshot = state.db.get_screenshot_by_id(&parsed_id).unwrap().unwrap();
        assert_eq!(
            screenshot.expires_at.unwrap(),
            created_at + Duration::seconds(3600)
        );
    }

    #[tokio::test]
    async fn test_update_screenshot_source_url() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();
        let parsed_id: uuid::Uuid = id.parse().unwrap();

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/api/screenshots/{}", id))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"source_url":" https://example.com/page "}"#))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screenshot = state.db.get_screenshot_by_id(&parsed_id).unwrap().unwrap();
        assert_eq!(
            screenshot.source_url.as_deref(),
            Some("https://example.com/page")
        );

        let req = axum::http::Request::builder()
            .method("PATCH")
            .uri(format!("/api/screenshots/{}", id))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"source_url":""}"#))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screenshot = state.db.get_screenshot_by_id(&parsed_id).unwrap().unwrap();
        assert_eq!(screenshot.source_url, None);
    }

    #[tokio::test]
    async fn test_editor_page_uses_autosave_assets() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();

        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/screenshots/{}/edit", id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains(r#"id="save-status""#));
        assert!(html.contains(r#"id="save-btn""#));
        assert!(html.contains(r#"id="delete-selected-btn""#));
        assert!(html.contains(r#"/static/css/editor.css?v=touch-editor-2"#));
        assert!(html.contains(r#"/static/js/editor.js?v=touch-editor-2"#));

        let editor_css = std::fs::read_to_string("static/css/editor.css").unwrap();
        assert!(editor_css.contains("grid-template-columns: 1fr 300px;"));
        assert!(editor_css.contains("@media (max-width: 900px)"));
        assert!(editor_css.contains("@media (max-width: 640px)"));
        assert!(editor_css.contains("height: calc(100dvh - 52px);"));
        assert!(editor_css.contains("touch-action: none;"));
        assert!(editor_css.contains(".mobile-delete-btn"));

        let editor_js = std::fs::read_to_string("static/js/editor.js").unwrap();
        assert!(editor_js.contains("const AUTOSAVE_DELAY_MS = 2000;"));
        assert!(editor_js.contains("window.addEventListener('pagehide', flushAutosaveOnPageExit);"));
        assert!(editor_js.contains("keepalive: true"));
        assert!(editor_js.contains("function setupTouchGestures()"));
        assert!(editor_js.contains("ResizeObserver"));
        assert!(editor_js.contains("touchstart"));
        assert!(editor_js.contains("getTouchDistance"));
        assert!(editor_js.contains("function deleteSelectedObjects()"));
        assert!(editor_js.contains("delete-selected-btn"));
    }

    #[tokio::test]
    async fn test_favicon_serves_ico_icon() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/favicon.ico")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/x-icon"
        );

        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let expected = std::fs::read("extension/icons/favicon.ico").unwrap();
        assert_eq!(bytes.as_ref(), expected.as_slice());
    }

    #[tokio::test]
    async fn test_upload_requires_auth() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Don't set up a user - just try uploading
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/screenshots")
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=test")
            .body(Body::from("--test--\r\n"))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_list_screenshots() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;

        // Upload 2 screenshots
        upload_screenshot(&app, &cookie).await;
        upload_screenshot(&app, &cookie).await;

        // List
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/screenshots")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_json(resp).await;
        assert_eq!(body["total"], 2);
        assert_eq!(body["screenshots"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_share_page() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let share_id = upload_body["share_id"].as_str().unwrap();

        // Access share page (no auth needed)
        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}", share_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Test Screenshot"));
        assert!(html.contains("og:image")); // OpenGraph meta tag
        assert!(html.contains(&format!("/s/{}.preview.png", share_id)));
        assert!(html.contains(r#"property="og:image:width" content="100""#));
        assert!(html.contains(r#"property="og:image:height" content="100""#));
        assert!(html.contains(r#"name="twitter:card" content="summary_large_image""#));
        assert!(html.contains("twitter:image"));
        assert!(html.contains(r#"id="copy-page-link""#));
        assert!(html.contains(r#"id="copy-image""#));
        assert!(html.contains("Open Image"));
        assert!(html.contains(&format!(r#"href="/s/{}.png""#, share_id)));
    }

    #[tokio::test]
    async fn test_share_image() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let share_id = upload_body["share_id"].as_str().unwrap();

        // Access direct image
        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}.png", share_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
    }

    #[tokio::test]
    async fn test_share_preview_image() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id: uuid::Uuid = upload_body["id"].as_str().unwrap().parse().unwrap();
        let share_id = upload_body["share_id"].as_str().unwrap();
        let screenshot = state.db.get_screenshot_by_id(&id).unwrap().unwrap();
        let rendered_path = screenshot.rendered_path.unwrap();
        let preview_path = image_processing::preview_path_for_rendered_path(&rendered_path);
        assert!(preview_path.exists());

        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}.preview.png", share_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );

        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (100, 100));

        std::fs::remove_file(&preview_path).unwrap();
        assert!(!preview_path.exists());

        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}.preview.png", share_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (100, 100));
        assert!(preview_path.exists());
    }

    #[tokio::test]
    async fn test_share_nonexistent_returns_404() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/s/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();
        let parsed_id: uuid::Uuid = id.parse().unwrap();
        let share_id = upload_body["share_id"].as_str().unwrap();
        let screenshot = state.db.get_screenshot_by_id(&parsed_id).unwrap().unwrap();
        let rendered_path = screenshot.rendered_path.unwrap();
        let preview_path = image_processing::preview_path_for_rendered_path(&rendered_path);
        assert!(preview_path.exists());

        // Delete
        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/api/screenshots/{}", id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!preview_path.exists());

        // Should no longer be accessible
        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}", share_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_cleanup_expired_screenshots_deletes_records_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id: uuid::Uuid = upload_body["id"].as_str().unwrap().parse().unwrap();
        let share_id = upload_body["share_id"].as_str().unwrap();

        let screenshot = state.db.get_screenshot_by_id(&id).unwrap().unwrap();
        let original_path = screenshot.original_path.clone();
        let rendered_path = screenshot.rendered_path.clone().unwrap();
        let preview_path = image_processing::preview_path_for_rendered_path(&rendered_path);
        assert!(std::path::Path::new(&original_path).exists());
        assert!(std::path::Path::new(&rendered_path).exists());
        assert!(preview_path.exists());

        state
            .db
            .update_screenshot_metadata(
                &id,
                None,
                None,
                None,
                Some(Some(Utc::now() - Duration::seconds(1))),
                None,
            )
            .unwrap();

        let deleted = cleanup_expired_screenshots(&state).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(!std::path::Path::new(&original_path).exists());
        assert!(!std::path::Path::new(&rendered_path).exists());
        assert!(!preview_path.exists());
        assert!(state.db.get_screenshot_by_id(&id).unwrap().is_none());

        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}", share_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── API Token Tests ──

    #[tokio::test]
    async fn test_create_and_use_api_token() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;

        // Create token
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/auth/tokens")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &cookie)
            .body(Body::from(r#"{"label":"Test Token"}"#))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = body_json(resp).await;
        let token = body["token"].as_str().unwrap();
        assert!(token.starts_with("sss_"), "Token should have sss_ prefix");

        // Verify token CANNOT be used to list screenshots
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/screenshots")
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Verify token CAN be used for uploads
        let png_data = minimal_png();
        let boundary = "----TestBoundary";
        let body_str = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"test.png\"\r\nContent-Type: image/png\r\n\r\n",
            boundary = boundary,
        );
        let mut body_bytes = body_str.into_bytes();
        body_bytes.extend_from_slice(&png_data);
        body_bytes.extend_from_slice(
            format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nTest Screenshot via Token\r\n--{boundary}--\r\n", boundary = boundary).as_bytes()
        );

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/screenshots")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_create_api_token_requires_name() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;

        for body in [
            serde_json::json!({}),
            serde_json::json!({ "label": "" }),
            serde_json::json!({ "label": "   " }),
        ] {
            let req = authed_json_request("POST", "/api/auth/tokens", &cookie, body);
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

            let body = body_json(resp).await;
            assert_eq!(body["error"], "Token name is required.");
        }
    }

    #[tokio::test]
    async fn test_revoke_api_token() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;

        // Create token
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/auth/tokens")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &cookie)
            .body(Body::from(r#"{"label":"Revoke Me"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let body = body_json(resp).await;
        let token = body["token"].as_str().unwrap().to_string();
        let token_id = body["id"].as_str().unwrap();

        // Revoke token
        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/api/auth/tokens/{}", token_id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Token should no longer work for uploads
        let png_data = minimal_png();
        let boundary = "----TestBoundary";
        let body_str = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"test.png\"\r\nContent-Type: image/png\r\n\r\n",
            boundary = boundary,
        );
        let mut body_bytes = body_str.into_bytes();
        body_bytes.extend_from_slice(&png_data);
        body_bytes
            .extend_from_slice(format!("\r\n--{boundary}--\r\n", boundary = boundary).as_bytes());

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/screenshots")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .body(Body::from(body_bytes))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Annotation Tests ──

    #[tokio::test]
    async fn test_save_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let (app, state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();
        let parsed_id: uuid::Uuid = id.parse().unwrap();
        let screenshot = state.db.get_screenshot_by_id(&parsed_id).unwrap().unwrap();
        let rendered_path = screenshot.rendered_path.unwrap();
        let preview_path = image_processing::preview_path_for_rendered_path(&rendered_path);
        std::fs::remove_file(&preview_path).unwrap();

        // Save annotations
        let req = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("/api/screenshots/{}/annotations", id))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &cookie)
            .body(Body::from(serde_json::to_string(&serde_json::json!({
                "annotations": [
                    { "type": "redact", "x": 10, "y": 10, "w": 50, "h": 30 },
                    { "type": "rect", "x": 60, "y": 60, "w": 30, "h": 20, "color": "#ff0000", "filled": false, "strokeWidth": 3 },
                    { "type": "arrow", "x1": 0, "y1": 0, "x2": 50, "y2": 50, "color": "#00ff00", "strokeWidth": 2 }
                ],
                "crop": null
            })).unwrap()))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_json(resp).await;
        assert_eq!(body["ok"], true);
        assert!(body["rendered_url"].as_str().unwrap().ends_with(".png"));
        assert!(preview_path.exists());
    }

    // ── Page redirect tests ──

    #[tokio::test]
    async fn test_root_redirects_to_setup_when_no_users() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/setup");
    }

    #[tokio::test]
    async fn test_root_redirects_to_login_when_not_authed() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Create user first
        let req = json_request(
            "POST",
            "/api/auth/setup",
            serde_json::json!({
                "username": "admin",
                "password": "testpassword123"
            }),
        );
        app.clone().oneshot(req).await.unwrap();

        // Visit root without auth
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/login");
    }

    #[tokio::test]
    async fn test_dashboard_uses_preview_images() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();
        let share_id = upload_body["share_id"].as_str().unwrap();

        let req = authed_request("GET", "/", &cookie);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();

        assert!(html.contains(&format!(r#"<img src="/api/screenshots/{}/preview""#, id)));
        assert!(!html.contains(&format!("/s/{}.preview.png", share_id)));
        assert!(html.contains(&format!(
            r#"<a href="/screenshots/{}/edit" class="btn btn-sm btn-outline">Edit</a>"#,
            id
        )));
        assert!(html.contains(r#">Copy Shared Link</button>"#));
        assert!(html.contains(&format!(
            r#"data-url="http://localhost:8080/s/{}""#,
            share_id
        )));
    }

    #[tokio::test]
    async fn test_dashboard_preview_works_for_private_screenshots() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();
        let share_id = upload_body["share_id"].as_str().unwrap();

        let req = authed_json_request(
            "PATCH",
            &format!("/api/screenshots/{}", id),
            &cookie,
            serde_json::json!({ "visibility": "private" }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/s/{}.preview.png", share_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = authed_request("GET", &format!("/api/screenshots/{}/preview", id), &cookie);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
    }
}
