use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use screenshotsafe::{build_router, config, db, AppState};

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
    std::fs::create_dir_all(&config.storage.originals_path())?;
    std::fs::create_dir_all(&config.storage.rendered_path())?;

    let database = db::Database::open(&config.database.path)?;
    database.run_migrations()?;
    tracing::info!("Database initialized at {}", config.database.path);

    // Load or generate a persistent JWT secret
    let jwt_secret = match &config.auth.jwt_secret {
        Some(secret) => secret.clone(),
        None => {
            let secret_path = std::path::Path::new(&config.storage.path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(".jwt_secret");
            if secret_path.exists() {
                std::fs::read_to_string(&secret_path)?.trim().to_string()
            } else {
                let secret = uuid::Uuid::new_v4().to_string();
                std::fs::write(&secret_path, &secret)?;
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
    });

    let app = build_router(state);
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("ScreenshotSafe listening on {}", bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
