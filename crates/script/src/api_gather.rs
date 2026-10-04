//! Shared host-side wire and output types for native gathering sessions.

use crate::native::{ConfigError, ScriptFailure, ScriptStatus};
use std::sync::Arc;

/// The same public refusal at isolate admission and host preparation.
pub(crate) fn settings_refusal(error: ConfigError) -> String {
    if error.field.is_empty() {
        "invalid-settings".into()
    } else {
        format!("invalid-setting:{}:{}", error.field, error.code)
    }
}

/// Host-owned phase of a live gathering session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatherPhase {
    Preparing,
    Running,
}

/// The status object posted by the host for a live gather page.
#[derive(Debug, Clone)]
pub struct GatherPage {
    pub token: u64,
    pub phase: GatherPhase,
    /// Cloned only when the Gatherer publishes a value-changed status.
    pub status: Option<Arc<ScriptStatus>>,
}

/// Counts captured from the last published Gatherer status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatherCounts {
    pub yielded: u32,
    pub dropped: u32,
    pub deposited: u32,
    pub trips: u32,
    pub xp: i32,
}

/// Structured failure published by a blocked Gatherer card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatherFailure {
    pub code: Arc<str>,
    pub message: Arc<str>,
}

impl From<ScriptFailure> for GatherFailure {
    fn from(failure: ScriptFailure) -> Self {
        Self {
            code: failure.code,
            message: failure.message,
        }
    }
}

/// One retained terminal outcome, correlated to the isolate's request token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatherEnd {
    Stopped {
        token: u64,
        counts: GatherCounts,
    },
    Blocked {
        token: u64,
        failure: GatherFailure,
        counts: GatherCounts,
    },
    Refused {
        token: u64,
        reason: Arc<str>,
    },
    Failed {
        token: u64,
        reason: Arc<str>,
        counts: GatherCounts,
    },
}

impl GatherEnd {
    pub fn token(&self) -> u64 {
        match self {
            Self::Stopped { token, .. }
            | Self::Blocked { token, .. }
            | Self::Refused { token, .. }
            | Self::Failed { token, .. } => *token,
        }
    }
}

impl From<GatherEnd> for serde_json::Value {
    fn from(end: GatherEnd) -> Self {
        let counts = |counts: GatherCounts| {
            serde_json::json!({
                "yielded": counts.yielded,
                "dropped": counts.dropped,
                "deposited": counts.deposited,
                "trips": counts.trips,
                "xp": counts.xp,
            })
        };
        match end {
            GatherEnd::Stopped { token, counts: c } => serde_json::json!({
                "end": "stopped",
                "token": token,
                "counts": counts(c),
            }),
            GatherEnd::Blocked {
                token,
                failure,
                counts: c,
            } => serde_json::json!({
                "end": "blocked",
                "token": token,
                "failure": {
                    "code": failure.code,
                    "message": failure.message,
                },
                "counts": counts(c),
            }),
            GatherEnd::Refused { token, reason } => serde_json::json!({
                "end": "refused",
                "token": token,
                "reason": reason,
            }),
            GatherEnd::Failed {
                token,
                reason,
                counts: c,
            } => serde_json::json!({
                "end": "failed",
                "token": token,
                "reason": reason,
                "counts": counts(c),
            }),
        }
    }
}
