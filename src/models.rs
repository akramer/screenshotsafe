use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A registered user.
#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    pub display_name: String,
    pub is_admin: bool,
    pub max_screenshot_size_bytes: Option<u64>,
    pub max_expiry_seconds: Option<u64>,
    pub created_at: DateTime<Utc>,
}

/// An API token for client authentication.
#[derive(Debug, Clone, Serialize)]
pub struct ApiToken {
    pub id: Uuid,
    pub user_id: Uuid,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// A screenshot with its metadata and annotation state.
#[derive(Debug, Clone, Serialize)]
pub struct Screenshot {
    pub id: Uuid,
    pub user_id: Uuid,
    pub share_id: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub original_filename: String,
    pub original_path: String,
    pub rendered_path: Option<String>,
    pub annotations: Vec<Annotation>,
    pub crop_rect: Option<CropRect>,
    pub image_dpi: f64,
    pub visibility: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A crop rectangle applied to the original image before rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// An annotation object drawn on top of the image.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Annotation {
    Redact {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        color: String,
        #[serde(default)]
        filled: bool,
        #[serde(default = "default_stroke_width", rename = "strokeWidth")]
        stroke_width: f64,
    },
    Arrow {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        color: String,
        #[serde(default = "default_stroke_width", rename = "strokeWidth")]
        stroke_width: f64,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        color: String,
        #[serde(default = "default_stroke_width", rename = "strokeWidth")]
        stroke_width: f64,
    },
    Text {
        x: f64,
        y: f64,
        text: String,
        #[serde(default = "default_font_size", rename = "fontSize")]
        font_size: f64,
        color: String,
    },
}

fn default_stroke_width() -> f64 {
    3.0
}

fn default_font_size() -> f64 {
    24.0
}

/// Display title for a screenshot, falling back through source_url and filename.
impl Screenshot {
    pub fn display_title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.source_url.as_deref())
            .unwrap_or(&self.original_filename)
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|exp| exp < Utc::now()).unwrap_or(false)
    }
}
