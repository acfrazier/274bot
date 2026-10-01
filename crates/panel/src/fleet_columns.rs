//! The Fleet window's toggleable status columns and how their choice is
//! stored in the panel preferences.
//!
//! Only the toggles the operator changed are written, so an unknown key from
//! a newer build survives a save by an older one, and a corrupt file is never
//! rewritten by merely opening the window.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetColumn {
    /// World and login state.
    State,
    /// The profile's assigned card.
    Card,
    /// Script run state (running / paused / error).
    Run,
    /// Time since the run's Start.
    Runtime,
    /// The bot's newest log line.
    LastLog,
    /// Time since gameplay last progressed (the watchdog's clock).
    Idle,
    /// Levels gained since Start.
    Levels,
}

impl FleetColumn {
    pub const ALL: [FleetColumn; 7] = [
        Self::State,
        Self::Card,
        Self::Run,
        Self::Runtime,
        Self::LastLog,
        Self::Idle,
        Self::Levels,
    ];

    /// The key in the panel preferences.
    pub fn id(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Card => "card",
            Self::Run => "run",
            Self::Runtime => "runtime",
            Self::LastLog => "last_log",
            Self::Idle => "idle",
            Self::Levels => "levels",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::State => "World / login",
            Self::Card => "Card",
            Self::Run => "Run state",
            Self::Runtime => "Runtime",
            Self::LastLog => "Last log line",
            Self::Idle => "Since progress",
            Self::Levels => "Levels gained",
        }
    }

    /// The columns the window always had are on until turned off.
    pub fn default_on(self) -> bool {
        matches!(self, Self::State | Self::Card | Self::Run)
    }

    /// Whether the cell reads the run's progress from the host.
    pub fn needs_progress(self) -> bool {
        matches!(self, Self::Runtime | Self::Idle | Self::Levels)
    }
}

pub fn visible(prefs: &HashMap<String, bool>, column: FleetColumn) -> bool {
    prefs
        .get(column.id())
        .copied()
        .unwrap_or_else(|| column.default_on())
}

/// Record the operator's choice. Returning to the default removes the key.
pub fn set_visible(prefs: &mut HashMap<String, bool>, column: FleetColumn, on: bool) {
    if on == column.default_on() {
        prefs.remove(column.id());
    } else {
        prefs.insert(column.id().to_string(), on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_start_at_their_defaults_and_toggle_independently() {
        let mut prefs = HashMap::new();
        set_visible(&mut prefs, FleetColumn::Idle, true);
        set_visible(&mut prefs, FleetColumn::Card, false);
        assert!(visible(&prefs, FleetColumn::Idle));
        assert!(!visible(&prefs, FleetColumn::Card));
        assert!(visible(&prefs, FleetColumn::State), "others are unchanged");
    }

    /// Saving a toggle keeps whatever else the file held, and switching back
    /// to the default leaves no key behind.
    #[test]
    fn toggling_keeps_unknown_keys_and_drops_default_ones() {
        let mut prefs: HashMap<String, bool> =
            HashMap::from([("from_a_newer_build".to_string(), true)]);
        set_visible(&mut prefs, FleetColumn::Levels, true);
        assert_eq!(prefs.len(), 2);
        set_visible(&mut prefs, FleetColumn::Levels, false);
        assert_eq!(
            prefs,
            HashMap::from([("from_a_newer_build".to_string(), true)])
        );
    }
}
