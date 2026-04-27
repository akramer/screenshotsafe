use serde::Deserialize;
use std::path::PathBuf;

/// Top-level application configuration, loaded from TOML.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_public_url")]
    pub public_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    #[serde(default = "default_storage_path")]
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    #[serde(default = "default_session_ttl")]
    pub session_ttl_seconds: u64,
    #[serde(default = "default_expiry_seconds")]
    pub default_expiry_seconds: Option<u64>,
    pub jwt_secret: Option<String>,
}

fn default_bind() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_public_url() -> String {
    "http://localhost:8080".to_string()
}
fn default_storage_path() -> String {
    "./data/storage".to_string()
}
fn default_db_path() -> String {
    "./data/screenshotsafe.db".to_string()
}
fn default_session_ttl() -> u64 {
    7 * 24 * 3600 // 7 days
}
fn default_expiry_seconds() -> Option<u64> {
    Some(30 * 24 * 3600) // 30 days
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            public_url: default_public_url(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: default_storage_path(),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_ttl_seconds: default_session_ttl(),
            default_expiry_seconds: default_expiry_seconds(),
            jwt_secret: None,
        }
    }
}

impl StorageConfig {
    pub fn originals_path(&self) -> PathBuf {
        PathBuf::from(&self.path).join("originals")
    }

    pub fn rendered_path(&self) -> PathBuf {
        PathBuf::from(&self.path).join("rendered")
    }
}

impl Config {
    /// Load config from a TOML file. Checks `--config` CLI arg, then
    /// `SCREENSHOTSAFE_CONFIG` env var, then `config.toml` in CWD.
    /// If no config file exists, uses defaults.
    pub fn load() -> anyhow::Result<Self> {
        let path = std::env::args()
            .skip_while(|a| a != "--config")
            .nth(1)
            .or_else(|| std::env::var("SCREENSHOTSAFE_CONFIG").ok())
            .unwrap_or_else(|| "config.toml".to_string());

        let config = if std::path::Path::new(&path).exists() {
            let contents = std::fs::read_to_string(&path)?;
            toml::from_str(&contents)?
        } else {
            tracing::info!("No config file found at '{}', using defaults", path);
            Config::default()
        };

        // Allow env var overrides
        let config = Self::apply_env_overrides(config);

        Ok(config)
    }

    fn apply_env_overrides(mut config: Config) -> Config {
        if let Ok(val) = std::env::var("SSS_BIND") {
            config.server.bind = val;
        }
        if let Ok(val) = std::env::var("SSS_PUBLIC_URL") {
            config.server.public_url = val;
        }
        if let Ok(val) = std::env::var("SSS_STORAGE_PATH") {
            config.storage.path = val;
        }
        if let Ok(val) = std::env::var("SSS_DATABASE_PATH") {
            config.database.path = val;
        }
        if let Ok(val) = std::env::var("SSS_JWT_SECRET") {
            config.auth.jwt_secret = Some(val);
        }
        config
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            storage: StorageConfig::default(),
            database: DatabaseConfig::default(),
            auth: AuthConfig::default(),
        }
    }
}
