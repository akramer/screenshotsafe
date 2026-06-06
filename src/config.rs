use serde::Deserialize;
use std::path::PathBuf;

/// Top-level application configuration, loaded from TOML.
#[derive(Debug, Deserialize, Clone, Default)]
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
    #[serde(default = "default_max_screenshot_size_bytes")]
    pub max_screenshot_size_bytes: u64,
    #[serde(default)]
    pub max_expiry_seconds: Option<u64>,
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
    #[serde(default)]
    pub allowed_extension_origins: Vec<String>,
    pub jwt_secret: Option<String>,
    #[serde(default)]
    pub oauth: OAuthConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OAuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_oauth_provider")]
    pub provider: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub issuer_url: String,
    #[serde(default)]
    pub discovery_url: String,
    #[serde(default)]
    pub authorize_url: String,
    #[serde(default)]
    pub token_url: String,
    #[serde(default)]
    pub userinfo_url: String,
    #[serde(default = "default_oauth_scope")]
    pub scope: String,
    #[serde(default)]
    pub redirect_url: String,
    #[serde(default)]
    pub allowed_email_domains: Vec<String>,
    #[serde(default)]
    pub account_mode: OAuthAccountMode,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OAuthAccountMode {
    #[default]
    LinkOnly,
    Pending,
    AutoEnabled,
}

fn default_bind() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_public_url() -> String {
    "".to_string()
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
fn default_max_screenshot_size_bytes() -> u64 {
    25 * 1024 * 1024 // 25 MiB
}
fn default_oauth_provider() -> String {
    "oauth".to_string()
}
fn default_oauth_scope() -> String {
    "openid email profile".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            public_url: default_public_url(),
            max_screenshot_size_bytes: default_max_screenshot_size_bytes(),
            max_expiry_seconds: None,
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
            allowed_extension_origins: Vec::new(),
            jwt_secret: None,
            oauth: OAuthConfig::default(),
        }
    }
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_oauth_provider(),
            display_name: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            issuer_url: String::new(),
            discovery_url: String::new(),
            authorize_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            scope: default_oauth_scope(),
            redirect_url: String::new(),
            allowed_email_domains: Vec::new(),
            account_mode: OAuthAccountMode::LinkOnly,
        }
    }
}

impl OAuthConfig {
    pub fn idp_name(&self) -> &str {
        if self.display_name.is_empty() {
            "OAuth"
        } else {
            &self.display_name
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

        Ok(Self::normalize(config))
    }

    fn apply_env_overrides(mut config: Config) -> Config {
        if let Ok(val) = std::env::var("SSS_BIND") {
            config.server.bind = val;
        }
        if let Ok(val) = std::env::var("SSS_PUBLIC_URL") {
            config.server.public_url = val;
        }
        if let Ok(val) = std::env::var("SSS_MAX_SCREENSHOT_SIZE_BYTES") {
            if let Ok(bytes) = val.parse::<u64>() {
                config.server.max_screenshot_size_bytes = bytes;
            }
        }
        if let Ok(val) = std::env::var("SSS_MAX_EXPIRY_SECONDS") {
            config.server.max_expiry_seconds = parse_optional_u64(&val);
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
        if let Ok(val) = std::env::var("SSS_ALLOWED_EXTENSION_ORIGINS") {
            config.auth.allowed_extension_origins = parse_csv_list(&val);
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_ENABLED") {
            config.auth.oauth.enabled = parse_bool(&val);
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_PROVIDER") {
            config.auth.oauth.provider = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_DISPLAY_NAME") {
            config.auth.oauth.display_name = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_CLIENT_ID") {
            config.auth.oauth.client_id = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_CLIENT_SECRET") {
            config.auth.oauth.client_secret = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_ISSUER_URL") {
            config.auth.oauth.issuer_url = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_DISCOVERY_URL") {
            config.auth.oauth.discovery_url = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_AUTHORIZE_URL") {
            config.auth.oauth.authorize_url = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_TOKEN_URL") {
            config.auth.oauth.token_url = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_USERINFO_URL") {
            config.auth.oauth.userinfo_url = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_SCOPE") {
            config.auth.oauth.scope = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_REDIRECT_URL") {
            config.auth.oauth.redirect_url = val;
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_ALLOWED_EMAIL_DOMAINS") {
            config.auth.oauth.allowed_email_domains = val
                .split(',')
                .map(|domain| domain.trim().to_ascii_lowercase())
                .filter(|domain| !domain.is_empty())
                .collect();
        }
        if let Ok(val) = std::env::var("SSS_OAUTH_ACCOUNT_MODE") {
            config.auth.oauth.account_mode = parse_oauth_account_mode(&val);
        }
        config
    }

    fn normalize(mut config: Config) -> Config {
        if config.server.max_screenshot_size_bytes == 0 {
            config.server.max_screenshot_size_bytes = default_max_screenshot_size_bytes();
        }
        config.server.max_expiry_seconds = config
            .server
            .max_expiry_seconds
            .filter(|seconds| *seconds > 0 && i64::try_from(*seconds).is_ok());
        config.auth.allowed_extension_origins = config
            .auth
            .allowed_extension_origins
            .into_iter()
            .filter_map(|origin| normalize_origin(&origin))
            .collect();
        config.auth.oauth.provider = config.auth.oauth.provider.trim().to_string();
        config.auth.oauth.display_name = config.auth.oauth.display_name.trim().to_string();
        config.auth.oauth.issuer_url = config
            .auth
            .oauth
            .issuer_url
            .trim_end_matches('/')
            .to_string();
        config.auth.oauth.discovery_url = config.auth.oauth.discovery_url.trim().to_string();
        config.auth.oauth.authorize_url = config.auth.oauth.authorize_url.trim().to_string();
        config.auth.oauth.token_url = config.auth.oauth.token_url.trim().to_string();
        config.auth.oauth.userinfo_url = config.auth.oauth.userinfo_url.trim().to_string();
        config.auth.oauth.allowed_email_domains = config
            .auth
            .oauth
            .allowed_email_domains
            .into_iter()
            .map(|domain| domain.trim().trim_start_matches('@').to_ascii_lowercase())
            .filter(|domain| !domain.is_empty())
            .collect();
        config
    }
}

fn parse_optional_u64(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || value == "0" || value.eq_ignore_ascii_case("none") {
        None
    } else {
        value.parse::<u64>().ok()
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn normalize_origin(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_oauth_account_mode(value: &str) -> OAuthAccountMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "pending" => OAuthAccountMode::Pending,
        "auto_enabled" | "auto-enabled" | "auto" => OAuthAccountMode::AutoEnabled,
        _ => OAuthAccountMode::LinkOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::OAuthConfig;

    #[test]
    fn oauth_idp_name_defaults_to_oauth() {
        let config = OAuthConfig::default();
        assert_eq!(config.idp_name(), "OAuth");
    }

    #[test]
    fn oauth_idp_name_uses_display_name() {
        let config = OAuthConfig {
            display_name: "Acme SSO".to_string(),
            ..OAuthConfig::default()
        };
        assert_eq!(config.idp_name(), "Acme SSO");
    }
}
