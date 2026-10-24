#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

//! Release navigation generation. Runtime planning stays in `nav-planner`.

pub mod bake;
pub mod bundle;
pub mod canlight;
pub mod collision;
pub mod manifest;
pub mod pack;
pub mod services;
pub mod transport;
mod zones;

/// Failure at a navigation-generation boundary.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    /// A required source, identity, or generation invariant is invalid.
    #[error("{0}")]
    InvalidInput(String),
    /// An artifact or source file could not be read or written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Generated pack data violates the runtime wire contract.
    #[error(transparent)]
    Pack(#[from] nav_planner::pack::PackError),
}

impl From<String> for GenerationError {
    fn from(detail: String) -> Self {
        Self::InvalidInput(detail)
    }
}

impl From<&str> for GenerationError {
    fn from(detail: &str) -> Self {
        Self::InvalidInput(detail.into())
    }
}

impl From<GenerationError> for String {
    fn from(error: GenerationError) -> Self {
        match error {
            GenerationError::InvalidInput(detail) => detail,
            other => other.to_string(),
        }
    }
}

/// Verbose generator diagnostics use the shared host logging setting.
pub fn debug_enabled() -> bool {
    api::hostlog::debug_enabled()
}
