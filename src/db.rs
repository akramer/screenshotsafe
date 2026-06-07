use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::sync::Mutex;

use crate::models::{AccountStatus, ApiToken, OAuthIdentity, Screenshot, ThemePreference, User};
use crate::Result;

type ScreenshotFilePaths = Vec<(String, Option<String>)>;

/// Parse a datetime string that may be RFC3339 or SQLite's `datetime()` format.
fn parse_datetime(s: &str) -> DateTime<Utc> {
    // Try RFC3339 first (e.g. "2026-04-26T21:28:45+00:00")
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return dt.with_timezone(&Utc);
    }
    // Try SQLite datetime format (e.g. "2026-04-26 21:28:45")
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return naive.and_utc();
    }
    // Last resort: try without timezone
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return naive.and_utc();
    }
    tracing::warn!("Failed to parse datetime: {}", s);
    Utc::now()
}

/// Parse an optional datetime string.
fn parse_datetime_opt(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    Some(parse_datetime(s))
}

/// Thread-safe database wrapper around a SQLite connection.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) a SQLite database at the given path.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        std::fs::create_dir_all(
            std::path::Path::new(path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        )?;
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run all schema migrations.
    pub fn run_migrations(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT,
                display_name TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                account_status TEXT NOT NULL DEFAULT 'enabled',
                max_screenshot_size_bytes INTEGER,
                max_expiry_seconds INTEGER,
                theme_preference TEXT NOT NULL DEFAULT 'os_default',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS oauth_identities (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                provider TEXT NOT NULL,
                subject TEXT NOT NULL,
                email TEXT,
                display_name TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_login_at TEXT,
                UNIQUE(provider, subject)
            );

            CREATE TABLE IF NOT EXISTS api_tokens (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_used_at TEXT,
                expires_at TEXT
            );

            CREATE TABLE IF NOT EXISTS screenshots (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                share_id TEXT NOT NULL UNIQUE,
                title TEXT,
                source_url TEXT,
                original_filename TEXT NOT NULL,
                original_path TEXT NOT NULL,
                rendered_path TEXT,
                annotations TEXT NOT NULL DEFAULT '[]',
                crop_rect TEXT,
                visibility TEXT NOT NULL DEFAULT 'unlisted',
                expires_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_screenshots_share_id ON screenshots(share_id);
            CREATE INDEX IF NOT EXISTS idx_screenshots_user_id ON screenshots(user_id);
            CREATE INDEX IF NOT EXISTS idx_screenshots_expires_at ON screenshots(expires_at);
            CREATE INDEX IF NOT EXISTS idx_api_tokens_token_hash ON api_tokens(token_hash);
            CREATE INDEX IF NOT EXISTS idx_oauth_identities_user_id ON oauth_identities(user_id);
            ",
        )?;
        add_column_if_missing(&conn, "users", "is_admin", "INTEGER NOT NULL DEFAULT 0")?;
        add_column_if_missing(
            &conn,
            "users",
            "account_status",
            "TEXT NOT NULL DEFAULT 'enabled'",
        )?;
        add_column_if_missing(&conn, "users", "max_screenshot_size_bytes", "INTEGER")?;
        add_column_if_missing(&conn, "users", "max_expiry_seconds", "INTEGER")?;
        add_column_if_missing(
            &conn,
            "users",
            "theme_preference",
            "TEXT NOT NULL DEFAULT 'os_default'",
        )?;
        conn.execute(
            "UPDATE users SET is_admin = 1
             WHERE NOT EXISTS (SELECT 1 FROM users WHERE is_admin = 1)
               AND id = (SELECT id FROM users ORDER BY created_at ASC LIMIT 1)",
            [],
        )?;
        add_column_if_missing(
            &conn,
            "screenshots",
            "image_dpi",
            "REAL NOT NULL DEFAULT 100.0",
        )?;
        Ok(())
    }

    // ── User operations ──

    pub fn user_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: usize = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn admin_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM users WHERE is_admin = 1 AND account_status = 'enabled'",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn create_user(&self, user: &User) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (id, username, password_hash, display_name, is_admin, account_status, max_screenshot_size_bytes, max_expiry_seconds, theme_preference, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                user.id.to_string(),
                user.username,
                user.password_hash,
                user.display_name,
                user.is_admin,
                user.account_status.as_str(),
                user.max_screenshot_size_bytes.map(|v| v as i64),
                user.max_expiry_seconds.map(|v| v as i64),
                user.theme_preference.as_str(),
                user.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn create_initial_admin(&self, user: &User) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "INSERT INTO users (id, username, password_hash, display_name, is_admin, account_status, max_screenshot_size_bytes, max_expiry_seconds, theme_preference, created_at)
             SELECT ?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9
             WHERE NOT EXISTS (SELECT 1 FROM users)",
            params![
                user.id.to_string(),
                user.username,
                user.password_hash,
                user.display_name,
                user.account_status.as_str(),
                user.max_screenshot_size_bytes.map(|v| v as i64),
                user.max_expiry_seconds.map(|v| v as i64),
                user.theme_preference.as_str(),
                user.created_at.to_rfc3339(),
            ],
        )?;
        Ok(rows > 0)
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, display_name, is_admin, account_status, max_screenshot_size_bytes, max_expiry_seconds, theme_preference, created_at
             FROM users ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], Self::user_from_row)?;
        let users: std::result::Result<Vec<_>, _> = rows.collect();
        Ok(users?)
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT id, username, password_hash, display_name, is_admin, account_status, max_screenshot_size_bytes, max_expiry_seconds, theme_preference, created_at FROM users WHERE username = ?1",
                params![username],
                Self::user_from_row,
            )
            .optional()?;
        Ok(result)
    }

    pub fn get_user_by_id(&self, id: &uuid::Uuid) -> Result<Option<User>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT id, username, password_hash, display_name, is_admin, account_status, max_screenshot_size_bytes, max_expiry_seconds, theme_preference, created_at FROM users WHERE id = ?1",
                params![id.to_string()],
                Self::user_from_row,
            )
            .optional()?;
        Ok(result)
    }

    pub fn update_user_password_hash(&self, id: &uuid::Uuid, password_hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![password_hash, id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_user_limits(
        &self,
        id: &uuid::Uuid,
        max_screenshot_size_bytes: Option<u64>,
        max_expiry_seconds: Option<u64>,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE users SET max_screenshot_size_bytes = ?1, max_expiry_seconds = ?2 WHERE id = ?3",
            params![
                max_screenshot_size_bytes.map(|v| v as i64),
                max_expiry_seconds.map(|v| v as i64),
                id.to_string(),
            ],
        )?;
        Ok(rows > 0)
    }

    pub fn update_user_account_status(
        &self,
        id: &uuid::Uuid,
        account_status: AccountStatus,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE users SET account_status = ?1 WHERE id = ?2",
            params![account_status.as_str(), id.to_string()],
        )?;
        Ok(rows > 0)
    }

    pub fn update_user_theme_preference(
        &self,
        id: &uuid::Uuid,
        theme_preference: ThemePreference,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE users SET theme_preference = ?1 WHERE id = ?2",
            params![theme_preference.as_str(), id.to_string()],
        )?;
        Ok(rows > 0)
    }

    pub fn get_user_by_oauth_identity(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<(User, OAuthIdentity)>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT u.id, u.username, u.password_hash, u.display_name, u.is_admin, u.account_status, u.max_screenshot_size_bytes, u.max_expiry_seconds, u.theme_preference, u.created_at,
                        oi.id, oi.user_id, oi.provider, oi.subject, oi.email, oi.display_name, oi.created_at, oi.last_login_at
                 FROM oauth_identities oi JOIN users u ON oi.user_id = u.id
                 WHERE oi.provider = ?1 AND oi.subject = ?2",
                params![provider, subject],
                |row| Ok((Self::user_from_oauth_join_row(row)?, Self::oauth_identity_from_join_row(row)?)),
            )
            .optional()?;
        Ok(result)
    }

    pub fn list_oauth_identities_for_user(
        &self,
        user_id: &uuid::Uuid,
    ) -> Result<Vec<OAuthIdentity>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, provider, subject, email, display_name, created_at, last_login_at
             FROM oauth_identities WHERE user_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![user_id.to_string()], Self::oauth_identity_from_row)?;
        let identities: std::result::Result<Vec<_>, _> = rows.collect();
        Ok(identities?)
    }

    pub fn create_oauth_identity(&self, identity: &OAuthIdentity) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO oauth_identities (id, user_id, provider, subject, email, display_name, created_at, last_login_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                identity.id.to_string(),
                identity.user_id.to_string(),
                identity.provider,
                identity.subject,
                identity.email,
                identity.display_name,
                identity.created_at.to_rfc3339(),
                identity.last_login_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn delete_oauth_identity_for_user(
        &self,
        id: &uuid::Uuid,
        user_id: &uuid::Uuid,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM oauth_identities WHERE id = ?1 AND user_id = ?2",
            params![id.to_string(), user_id.to_string()],
        )?;
        Ok(rows > 0)
    }

    pub fn create_user_with_oauth_identity(
        &self,
        user: &User,
        identity: &OAuthIdentity,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO users (id, username, password_hash, display_name, is_admin, account_status, max_screenshot_size_bytes, max_expiry_seconds, theme_preference, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                user.id.to_string(),
                user.username,
                user.password_hash,
                user.display_name,
                user.is_admin,
                user.account_status.as_str(),
                user.max_screenshot_size_bytes.map(|v| v as i64),
                user.max_expiry_seconds.map(|v| v as i64),
                user.theme_preference.as_str(),
                user.created_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "INSERT INTO oauth_identities (id, user_id, provider, subject, email, display_name, created_at, last_login_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                identity.id.to_string(),
                identity.user_id.to_string(),
                identity.provider,
                identity.subject,
                identity.email,
                identity.display_name,
                identity.created_at.to_rfc3339(),
                identity.last_login_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn update_oauth_identity_login(
        &self,
        provider: &str,
        subject: &str,
        email: Option<&str>,
        display_name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE oauth_identities
             SET email = COALESCE(?1, email),
                 display_name = COALESCE(?2, display_name),
                 last_login_at = ?3
             WHERE provider = ?4 AND subject = ?5",
            params![
                email,
                display_name,
                Utc::now().to_rfc3339(),
                provider,
                subject
            ],
        )?;
        Ok(())
    }

    pub fn delete_user(&self, id: &uuid::Uuid) -> Result<Option<ScreenshotFilePaths>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut stmt =
            tx.prepare("SELECT original_path, rendered_path FROM screenshots WHERE user_id = ?1")?;
        let paths: ScreenshotFilePaths = stmt
            .query_map(params![id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);

        let rows = tx.execute("DELETE FROM users WHERE id = ?1", params![id.to_string()])?;
        tx.commit()?;

        if rows == 0 {
            Ok(None)
        } else {
            Ok(Some(paths))
        }
    }

    fn user_from_row(row: &Row<'_>) -> rusqlite::Result<User> {
        Ok(User {
            id: row.get::<_, String>(0)?.parse().unwrap(),
            username: row.get(1)?,
            password_hash: row.get(2)?,
            display_name: row.get(3)?,
            is_admin: row.get(4)?,
            account_status: AccountStatus::from(row.get::<_, String>(5)?.as_str()),
            max_screenshot_size_bytes: optional_u64(row.get(6)?),
            max_expiry_seconds: optional_u64(row.get(7)?),
            theme_preference: ThemePreference::from(row.get::<_, String>(8)?.as_str()),
            created_at: parse_datetime(&row.get::<_, String>(9)?),
        })
    }

    fn user_from_oauth_join_row(row: &Row<'_>) -> rusqlite::Result<User> {
        Ok(User {
            id: row.get::<_, String>(0)?.parse().unwrap(),
            username: row.get(1)?,
            password_hash: row.get(2)?,
            display_name: row.get(3)?,
            is_admin: row.get(4)?,
            account_status: AccountStatus::from(row.get::<_, String>(5)?.as_str()),
            max_screenshot_size_bytes: optional_u64(row.get(6)?),
            max_expiry_seconds: optional_u64(row.get(7)?),
            theme_preference: ThemePreference::from(row.get::<_, String>(8)?.as_str()),
            created_at: parse_datetime(&row.get::<_, String>(9)?),
        })
    }

    fn oauth_identity_from_join_row(row: &Row<'_>) -> rusqlite::Result<OAuthIdentity> {
        let last_login_at: Option<String> = row.get(17)?;
        Ok(OAuthIdentity {
            id: row.get::<_, String>(10)?.parse().unwrap(),
            user_id: row.get::<_, String>(11)?.parse().unwrap(),
            provider: row.get(12)?,
            subject: row.get(13)?,
            email: row.get(14)?,
            display_name: row.get(15)?,
            created_at: parse_datetime(&row.get::<_, String>(16)?),
            last_login_at: last_login_at.and_then(|s| parse_datetime_opt(&s)),
        })
    }

    fn oauth_identity_from_row(row: &Row<'_>) -> rusqlite::Result<OAuthIdentity> {
        let last_login_at: Option<String> = row.get(7)?;
        Ok(OAuthIdentity {
            id: row.get::<_, String>(0)?.parse().unwrap(),
            user_id: row.get::<_, String>(1)?.parse().unwrap(),
            provider: row.get(2)?,
            subject: row.get(3)?,
            email: row.get(4)?,
            display_name: row.get(5)?,
            created_at: parse_datetime(&row.get::<_, String>(6)?),
            last_login_at: last_login_at.and_then(|s| parse_datetime_opt(&s)),
        })
    }

    // ── Screenshot operations ──

    pub fn create_screenshot(&self, s: &Screenshot) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO screenshots (id, user_id, share_id, title, source_url, original_filename, original_path, rendered_path, annotations, crop_rect, image_dpi, visibility, expires_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                s.id.to_string(),
                s.user_id.to_string(),
                s.share_id,
                s.title,
                s.source_url,
                s.original_filename,
                s.original_path,
                s.rendered_path,
                serde_json::to_string(&s.annotations).unwrap(),
                s.crop_rect.as_ref().map(|c| serde_json::to_string(c).unwrap()),
                s.image_dpi,
                s.visibility,
                s.expires_at.map(|t| t.to_rfc3339()),
                s.created_at.to_rfc3339(),
                s.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_screenshot_by_id(&self, id: &uuid::Uuid) -> Result<Option<Screenshot>> {
        let conn = self.conn.lock().unwrap();
        Self::query_screenshot(&conn, "WHERE id = ?1", params![id.to_string()])
    }

    pub fn get_screenshot_by_share_id(&self, share_id: &str) -> Result<Option<Screenshot>> {
        let conn = self.conn.lock().unwrap();
        Self::query_screenshot(&conn, "WHERE share_id = ?1", params![share_id])
    }

    fn query_screenshot(
        conn: &Connection,
        where_clause: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<Screenshot>> {
        let sql = format!(
            "SELECT id, user_id, share_id, title, source_url, original_filename, original_path, rendered_path, annotations, crop_rect, image_dpi, visibility, expires_at, created_at, updated_at FROM screenshots {}",
            where_clause
        );
        let result = conn
            .query_row(&sql, params, |row| {
                let annotations_str: String = row.get(8)?;
                let crop_str: Option<String> = row.get(9)?;
                let expires_str: Option<String> = row.get(12)?;
                Ok(Screenshot {
                    id: row.get::<_, String>(0)?.parse().unwrap(),
                    user_id: row.get::<_, String>(1)?.parse().unwrap(),
                    share_id: row.get(2)?,
                    title: row.get(3)?,
                    source_url: row.get(4)?,
                    original_filename: row.get(5)?,
                    original_path: row.get(6)?,
                    rendered_path: row.get(7)?,
                    annotations: serde_json::from_str(&annotations_str).unwrap_or_default(),
                    crop_rect: crop_str.and_then(|s| serde_json::from_str(&s).ok()),
                    image_dpi: row.get(10)?,
                    visibility: row.get(11)?,
                    expires_at: expires_str.and_then(|s| parse_datetime_opt(&s)),
                    created_at: parse_datetime(&row.get::<_, String>(13)?),
                    updated_at: parse_datetime(&row.get::<_, String>(14)?),
                })
            })
            .optional()?;
        Ok(result)
    }

    pub fn list_screenshots_for_user(
        &self,
        user_id: &uuid::Uuid,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Screenshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, share_id, title, source_url, original_filename, original_path, rendered_path, annotations, crop_rect, image_dpi, visibility, expires_at, created_at, updated_at
             FROM screenshots WHERE user_id = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            params![user_id.to_string(), limit as i64, offset as i64],
            |row| {
                let annotations_str: String = row.get(8)?;
                let crop_str: Option<String> = row.get(9)?;
                let expires_str: Option<String> = row.get(12)?;
                Ok(Screenshot {
                    id: row.get::<_, String>(0)?.parse().unwrap(),
                    user_id: row.get::<_, String>(1)?.parse().unwrap(),
                    share_id: row.get(2)?,
                    title: row.get(3)?,
                    source_url: row.get(4)?,
                    original_filename: row.get(5)?,
                    original_path: row.get(6)?,
                    rendered_path: row.get(7)?,
                    annotations: serde_json::from_str(&annotations_str).unwrap_or_default(),
                    crop_rect: crop_str.and_then(|s| serde_json::from_str(&s).ok()),
                    image_dpi: row.get(10)?,
                    visibility: row.get(11)?,
                    expires_at: expires_str.and_then(|s| parse_datetime_opt(&s)),
                    created_at: parse_datetime(&row.get::<_, String>(13)?),
                    updated_at: parse_datetime(&row.get::<_, String>(14)?),
                })
            },
        )?;
        let screenshots: std::result::Result<Vec<_>, _> = rows.collect();
        Ok(screenshots?)
    }

    pub fn update_screenshot_annotations(
        &self,
        id: &uuid::Uuid,
        annotations: &[crate::models::Annotation],
        crop_rect: &Option<crate::models::CropRect>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE screenshots SET annotations = ?1, crop_rect = ?2, updated_at = datetime('now') WHERE id = ?3",
            params![
                serde_json::to_string(annotations).unwrap(),
                crop_rect.as_ref().map(|c| serde_json::to_string(c).unwrap()),
                id.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn update_screenshot_rendered_path(
        &self,
        id: &uuid::Uuid,
        rendered_path: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE screenshots SET rendered_path = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![rendered_path, id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_screenshot_metadata(
        &self,
        id: &uuid::Uuid,
        title: Option<&str>,
        source_url: Option<Option<&str>>,
        visibility: Option<&str>,
        expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
        image_dpi: Option<f64>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if let Some(title) = title {
            conn.execute(
                "UPDATE screenshots SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![title, id.to_string()],
            )?;
        }
        if let Some(source_url) = source_url {
            conn.execute(
                "UPDATE screenshots SET source_url = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![source_url, id.to_string()],
            )?;
        }
        if let Some(visibility) = visibility {
            conn.execute(
                "UPDATE screenshots SET visibility = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![visibility, id.to_string()],
            )?;
        }
        if let Some(expires_at) = expires_at {
            conn.execute(
                "UPDATE screenshots SET expires_at = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![
                    expires_at.map(|t| t.to_rfc3339()),
                    id.to_string(),
                ],
            )?;
        }
        if let Some(image_dpi) = image_dpi {
            conn.execute(
                "UPDATE screenshots SET image_dpi = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![image_dpi, id.to_string()],
            )?;
        }
        Ok(())
    }

    pub fn delete_screenshot(&self, id: &uuid::Uuid) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM screenshots WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(rows > 0)
    }

    pub fn screenshot_count_for_user(&self, user_id: &uuid::Uuid) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM screenshots WHERE user_id = ?1",
            params![user_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    // ── API Token operations ──

    pub fn create_api_token(&self, token: &ApiToken) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO api_tokens (id, user_id, token_hash, label, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                token.id.to_string(),
                token.user_id.to_string(),
                token.token_hash,
                token.label,
                token.created_at.to_rfc3339(),
                token.expires_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn get_user_by_token_hash(&self, token_hash: &str) -> Result<Option<(User, uuid::Uuid)>> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT u.id, u.username, u.password_hash, u.display_name, u.is_admin, u.account_status, u.max_screenshot_size_bytes, u.max_expiry_seconds, u.theme_preference, u.created_at, t.id
                 FROM api_tokens t JOIN users u ON t.user_id = u.id
                 WHERE t.token_hash = ?1 AND (t.expires_at IS NULL OR t.expires_at > datetime('now'))",
                params![token_hash],
                |row| {
                    Ok((
                        User {
                            id: row.get::<_, String>(0)?.parse().unwrap(),
                            username: row.get(1)?,
                            password_hash: row.get(2)?,
                            display_name: row.get(3)?,
                            is_admin: row.get(4)?,
                            account_status: AccountStatus::from(row.get::<_, String>(5)?.as_str()),
                            max_screenshot_size_bytes: optional_u64(row.get(6)?),
                            max_expiry_seconds: optional_u64(row.get(7)?),
                            theme_preference: ThemePreference::from(row.get::<_, String>(8)?.as_str()),
                            created_at: parse_datetime(&row.get::<_, String>(9)?),
                        },
                        row.get::<_, String>(10)?.parse::<uuid::Uuid>().unwrap(),
                    ))
                },
            )
            .optional()?;

        // Update last_used_at
        if let Some((_, token_id)) = &result {
            conn.execute(
                "UPDATE api_tokens SET last_used_at = datetime('now') WHERE id = ?1",
                params![token_id.to_string()],
            )?;
        }

        Ok(result)
    }

    pub fn list_tokens_for_user(&self, user_id: &uuid::Uuid) -> Result<Vec<ApiToken>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, token_hash, label, created_at, last_used_at, expires_at
             FROM api_tokens WHERE user_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id.to_string()], |row| {
            let last_used_str: Option<String> = row.get(5)?;
            let expires_str: Option<String> = row.get(6)?;
            Ok(ApiToken {
                id: row.get::<_, String>(0)?.parse().unwrap(),
                user_id: row.get::<_, String>(1)?.parse().unwrap(),
                token_hash: row.get(2)?,
                label: row.get(3)?,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
                last_used_at: last_used_str.and_then(|s| parse_datetime_opt(&s)),
                expires_at: expires_str.and_then(|s| parse_datetime_opt(&s)),
            })
        })?;
        let tokens: std::result::Result<Vec<_>, _> = rows.collect();
        Ok(tokens?)
    }

    pub fn delete_token(&self, id: &uuid::Uuid, user_id: &uuid::Uuid) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2",
            params![id.to_string(), user_id.to_string()],
        )?;
        Ok(rows > 0)
    }

    /// Delete expired screenshots and return their file paths for cleanup.
    pub fn delete_expired_screenshots(&self) -> Result<Vec<(String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT original_path, rendered_path FROM screenshots WHERE expires_at IS NOT NULL AND datetime(expires_at) <= datetime('now')",
        )?;
        let paths: Vec<(String, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        conn.execute(
            "DELETE FROM screenshots WHERE expires_at IS NOT NULL AND datetime(expires_at) <= datetime('now')",
            [],
        )?;
        Ok(paths)
    }
}

fn optional_u64(value: Option<i64>) -> Option<u64> {
    value.and_then(|v| u64::try_from(v).ok())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, definition),
        [],
    )?;
    Ok(())
}
