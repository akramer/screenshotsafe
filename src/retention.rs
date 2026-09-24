use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserMaximumMode {
    Inherit,
    Duration,
    Unlimited,
}

impl UserMaximumMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Duration => "duration",
            Self::Unlimited => "unlimited",
        }
    }
}

impl From<&str> for UserMaximumMode {
    fn from(value: &str) -> Self {
        match value {
            "duration" => Self::Duration,
            "unlimited" => Self::Unlimited,
            _ => Self::Inherit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserDefaultMode {
    Inherit,
    Duration,
    Never,
}

impl UserDefaultMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Duration => "duration",
            Self::Never => "never",
        }
    }
}

impl From<&str> for UserDefaultMode {
    fn from(value: &str) -> Self {
        match value {
            "duration" => Self::Duration,
            "never" => Self::Never,
            _ => Self::Inherit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerRetentionSettings {
    pub default_expiry_seconds: Option<u64>,
    pub default_max_expiry_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserRetentionSettings {
    pub maximum_mode: UserMaximumMode,
    pub maximum_seconds: Option<u64>,
    pub default_mode: UserDefaultMode,
    pub default_seconds: Option<u64>,
}

impl Default for UserRetentionSettings {
    fn default() -> Self {
        Self {
            maximum_mode: UserMaximumMode::Inherit,
            maximum_seconds: None,
            default_mode: UserDefaultMode::Inherit,
            default_seconds: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EffectiveRetentionPolicy {
    pub server_default_expiry_seconds: Option<u64>,
    pub server_default_max_expiry_seconds: Option<u64>,
    pub user_maximum_mode: UserMaximumMode,
    pub user_maximum_seconds: Option<u64>,
    pub user_default_mode: UserDefaultMode,
    pub user_default_seconds: Option<u64>,
    pub effective_default_expiry_seconds: Option<u64>,
    pub effective_max_expiry_seconds: Option<u64>,
    pub allow_never: bool,
}

pub fn effective_policy(
    server: ServerRetentionSettings,
    user: UserRetentionSettings,
) -> EffectiveRetentionPolicy {
    let effective_max = match user.maximum_mode {
        UserMaximumMode::Inherit => server.default_max_expiry_seconds,
        UserMaximumMode::Duration => user.maximum_seconds,
        UserMaximumMode::Unlimited => None,
    };

    let preferred_default = match user.default_mode {
        UserDefaultMode::Inherit => server.default_expiry_seconds,
        UserDefaultMode::Duration => user.default_seconds,
        UserDefaultMode::Never => None,
    };
    let effective_default = match (preferred_default, effective_max) {
        (Some(default), Some(maximum)) => Some(default.min(maximum)),
        (None, Some(maximum)) => Some(maximum),
        (default, None) => default,
    };

    EffectiveRetentionPolicy {
        server_default_expiry_seconds: server.default_expiry_seconds,
        server_default_max_expiry_seconds: server.default_max_expiry_seconds,
        user_maximum_mode: user.maximum_mode,
        user_maximum_seconds: user.maximum_seconds,
        user_default_mode: user.default_mode,
        user_default_seconds: user.default_seconds,
        effective_default_expiry_seconds: effective_default,
        effective_max_expiry_seconds: effective_max,
        allow_never: effective_max.is_none(),
    }
}

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;

/// Splits a duration into its largest whole unit, e.g. 172800 -> (2, DAY).
/// Durations that aren't a whole number of minutes are returned as seconds.
pub fn duration_parts(seconds: u64) -> (u64, u64) {
    for unit in [WEEK, DAY, HOUR, MINUTE] {
        if seconds >= unit && seconds.is_multiple_of(unit) {
            return (seconds / unit, unit);
        }
    }
    (seconds, 1)
}

/// Formats a duration for display, e.g. "2 days" or "1 hour".
pub fn format_duration(seconds: u64) -> String {
    let (value, unit) = duration_parts(seconds);
    let name = match unit {
        WEEK => "week",
        DAY => "day",
        HOUR => "hour",
        MINUTE => "minute",
        _ => "second",
    };
    format!("{value} {name}{}", if value == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_maximum_can_exceed_server_default_maximum() {
        let policy = effective_policy(
            ServerRetentionSettings {
                default_expiry_seconds: Some(30),
                default_max_expiry_seconds: Some(90),
            },
            UserRetentionSettings {
                maximum_mode: UserMaximumMode::Duration,
                maximum_seconds: Some(180),
                ..Default::default()
            },
        );

        assert_eq!(policy.effective_max_expiry_seconds, Some(180));
        assert_eq!(policy.effective_default_expiry_seconds, Some(30));
    }

    #[test]
    fn finite_maximum_caps_an_inherited_never_default() {
        let policy = effective_policy(
            ServerRetentionSettings {
                default_expiry_seconds: None,
                default_max_expiry_seconds: Some(90),
            },
            UserRetentionSettings::default(),
        );

        assert_eq!(policy.effective_default_expiry_seconds, Some(90));
        assert!(!policy.allow_never);
    }

    #[test]
    fn formats_durations_in_largest_whole_unit() {
        assert_eq!(format_duration(0), "0 seconds");
        assert_eq!(format_duration(1), "1 second");
        assert_eq!(format_duration(90), "90 seconds");
        assert_eq!(format_duration(MINUTE), "1 minute");
        assert_eq!(format_duration(90 * MINUTE), "90 minutes");
        assert_eq!(format_duration(HOUR), "1 hour");
        assert_eq!(format_duration(2 * DAY), "2 days");
        assert_eq!(format_duration(WEEK), "1 week");
        assert_eq!(format_duration(4 * WEEK), "4 weeks");
    }
}
