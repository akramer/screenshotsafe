#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{header, StatusCode};
    use axum::body::Body;
    use tower::ServiceExt;

    use screenshotsafe::*;
    use screenshotsafe::db::Database;
    use screenshotsafe::config::Config;

    /// Create a test app with an in-memory database and temp storage.
    fn test_app(dir: &std::path::Path) -> (axum::Router, SharedState) {
        let db = Database::open_in_memory().unwrap();
        db.run_migrations().unwrap();

        let storage_path = dir.join("storage");
        std::fs::create_dir_all(storage_path.join("originals")).unwrap();
        std::fs::create_dir_all(storage_path.join("rendered")).unwrap();

        let mut config = Config::default();
        config.storage.path = storage_path.to_string_lossy().to_string();
        config.server.public_url = "http://localhost:8080".to_string();

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

    /// Helper: get response body as JSON.
    async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
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

        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123",
            "display_name": "Admin User"
        }));

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
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Second setup should fail
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin2",
            "password": "testpassword456"
        }));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_setup_validates_password_length() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "short"
        }));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_login_success() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Setup first
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
        app.clone().oneshot(req).await.unwrap();

        // Login
        let req = json_request("POST", "/api/auth/login", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let cookie = extract_session_cookie(&resp);
        assert!(cookie.is_some(), "Login should set session cookie");
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Setup first
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
        app.clone().oneshot(req).await.unwrap();

        // Login with wrong password
        let req = json_request("POST", "/api/auth/login", serde_json::json!({
            "username": "admin",
            "password": "wrongpassword"
        }));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_login_nonexistent_user() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        // Setup first
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
        app.clone().oneshot(req).await.unwrap();

        // Login as nonexistent user
        let req = json_request("POST", "/api/auth/login", serde_json::json!({
            "username": "nobody",
            "password": "testpassword123"
        }));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Screenshot Tests ──

    /// Helper: set up a user and return the session cookie string.
    async fn setup_user(app: &axum::Router) -> String {
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
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
        let png_data = minimal_png();
        let boundary = "----TestBoundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"test.png\"\r\nContent-Type: image/png\r\n\r\n",
            boundary = boundary,
        );
        let mut body_bytes = body.into_bytes();
        body_bytes.extend_from_slice(&png_data);
        body_bytes.extend_from_slice(
            format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nTest Screenshot\r\n--{boundary}--\r\n", boundary = boundary).as_bytes()
        );

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/screenshots")
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={}", boundary))
            .header(header::COOKIE, cookie)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        body_json(resp).await
    }

    #[tokio::test]
    async fn test_upload_screenshot() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let body = upload_screenshot(&app, &cookie).await;

        assert!(body["id"].is_string());
        assert!(body["share_id"].is_string());
        assert!(body["share_url"].as_str().unwrap().contains("/s/"));
        assert!(body["raw_url"].as_str().unwrap().ends_with(".png"));
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

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Test Screenshot"));
        assert!(html.contains("og:image")); // OpenGraph meta tag
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
        let share_id = upload_body["share_id"].as_str().unwrap();

        // Delete
        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/api/screenshots/{}", id))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Should no longer be accessible
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
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={}", boundary))
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
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
        body_bytes.extend_from_slice(
            format!("\r\n--{boundary}--\r\n", boundary = boundary).as_bytes()
        );

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/screenshots")
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={}", boundary))
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
        let (app, _state) = test_app(dir.path());

        let cookie = setup_user(&app).await;
        let upload_body = upload_screenshot(&app, &cookie).await;
        let id = upload_body["id"].as_str().unwrap();

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
        let req = json_request("POST", "/api/auth/setup", serde_json::json!({
            "username": "admin",
            "password": "testpassword123"
        }));
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
}
