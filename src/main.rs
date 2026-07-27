use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use screenshotsafe::{
    build_router, config, db, flush_hit_counts, spawn_expired_screenshot_cleanup,
    spawn_hit_count_flush, AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "screenshotsafe=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::Config::load()?;
    tracing::info!("Loaded config, binding to {}", config.server.bind);

    // Ensure storage directories exist
    tokio::fs::create_dir_all(&config.storage.originals_path()).await?;
    tokio::fs::create_dir_all(&config.storage.rendered_path()).await?;

    let database = db::Database::open(&config.database.path)?;
    database.run_migrations()?;
    database.initialize_server_retention(
        config.auth.default_expiry_seconds,
        config.server.max_expiry_seconds,
    )?;
    tracing::info!("Database initialized at {}", config.database.path);

    // Load or generate a persistent JWT secret
    let jwt_secret = match &config.auth.jwt_secret {
        Some(secret) => secret.clone(),
        None => {
            let secret_path = std::path::Path::new(&config.storage.path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(".jwt_secret");
            if tokio::fs::try_exists(&secret_path).await? {
                tokio::fs::read_to_string(&secret_path)
                    .await?
                    .trim()
                    .to_string()
            } else {
                let secret = uuid::Uuid::new_v4().to_string();
                tokio::fs::write(&secret_path, &secret).await?;
                tracing::info!("Generated new JWT secret at {}", secret_path.display());
                secret
            }
        }
    };

    let bind_addr = config.server.bind.clone();

    let state = Arc::new(AppState {
        db: database,
        config,
        jwt_secret,
        rate_limiter: Default::default(),
        hit_counter: Default::default(),
    });

    spawn_expired_screenshot_cleanup(state.clone());
    let hit_count_flush_task = spawn_hit_count_flush(state.clone());

    let app = build_router(state.clone());
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("ScreenshotSafe listening on {}", bind_addr);
    let server_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;

    hit_count_flush_task.abort();
    let _ = hit_count_flush_task.await;
    let flushed = flush_hit_counts(&state)?;
    if flushed > 0 {
        tracing::info!("Persisted {} full-image hits during shutdown", flushed);
    }
    server_result?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
