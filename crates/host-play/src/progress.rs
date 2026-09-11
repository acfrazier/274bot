//! Small, copy-free progress values for checked profile preparation.

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileProgressStage {
    SelectingServerProfile,
    CheckingGameFiles,
    CheckingNavigationFiles,
    LoadingGameData,
    PreparingNavigation,
    FinalChecks,
}

impl ProfileProgressStage {
    pub fn description(self) -> &'static str {
        match self {
            Self::SelectingServerProfile => "Selecting server profile",
            Self::CheckingGameFiles => "Checking game files",
            Self::CheckingNavigationFiles => "Checking navigation files",
            Self::LoadingGameData => "Loading game data",
            Self::PreparingNavigation => "Preparing navigation",
            Self::FinalChecks => "Final checks",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileProgressUnit {
    Bytes,
    Files,
    Steps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileProgress {
    pub stage: ProfileProgressStage,
    pub completed: u64,
    pub total: u64,
    pub unit: ProfileProgressUnit,
}

impl ProfileProgress {
    pub fn bytes(stage: ProfileProgressStage, completed: u64, total: u64) -> Self {
        Self {
            stage,
            completed,
            total,
            unit: ProfileProgressUnit::Bytes,
        }
    }

    pub fn files(stage: ProfileProgressStage, completed: u64, total: u64) -> Self {
        Self {
            stage,
            completed,
            total,
            unit: ProfileProgressUnit::Files,
        }
    }

    pub fn steps(stage: ProfileProgressStage, completed: u64, total: u64) -> Self {
        Self {
            stage,
            completed,
            total,
            unit: ProfileProgressUnit::Steps,
        }
    }
}

#[derive(Clone)]
pub struct ProfileProgressObserver {
    report: Arc<dyn Fn(ProfileProgress) + Send + Sync>,
}

impl ProfileProgressObserver {
    pub fn new(report: impl Fn(ProfileProgress) + Send + Sync + 'static) -> Self {
        Self {
            report: Arc::new(report),
        }
    }

    pub fn report(&self, progress: ProfileProgress) {
        (self.report)(progress);
    }
}

impl Default for ProfileProgressObserver {
    fn default() -> Self {
        Self::new(|_| {})
    }
}
