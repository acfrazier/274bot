//! Wording of the Fleet window's run-progress columns, shared by both front
//! ends: runtime, time since the last progress, and levels gained. The data
//! is [`script::ScriptProgress`], read from the host per visible row.

use std::fmt::Write as _;
use std::time::Duration;

/// A short span: `45s`, `12m`, `3h05m`, `2d04h`.
pub fn span_label(span: Duration) -> String {
    let secs = span.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h{:02}m", secs / 3600, secs % 3600 / 60)
    } else {
        format!("{}d{:02}h", secs / 86_400, secs % 86_400 / 3600)
    }
}

/// `—` for a bot with no armed run or a frozen clock.
pub const NONE: &str = "\u{2014}";

/// Runtime cell: time since the run's Start.
pub fn runtime_label(progress: Option<&script::ScriptProgress>) -> String {
    progress.map_or_else(|| NONE.to_string(), |p| span_label(p.running_for))
}

/// Time-since-progress cell: the watchdog's gameplay clock. Absent while
/// paused, out of game, or when nothing runs.
pub fn idle_label(progress: Option<&script::ScriptProgress>) -> String {
    progress
        .and_then(|p| p.idle_for)
        .map_or_else(|| NONE.to_string(), span_label)
}

/// Levels-gained cell: the total, `+3`.
pub fn levels_label(progress: Option<&script::ScriptProgress>) -> String {
    progress.map_or_else(|| NONE.to_string(), |p| format!("+{}", p.levels_gained()))
}

/// The per-skill breakdown behind [`levels_label`], `mining +2, fishing +1`
/// (empty when nothing was gained).
pub fn levels_detail(progress: &script::ScriptProgress) -> String {
    let mut text = String::new();
    for (slot, gained) in progress.gained.iter().enumerate() {
        if *gained == 0 {
            continue;
        }
        if !text.is_empty() {
            text.push_str(", ");
        }
        let _ = write!(text, "{} +{gained}", api::snapshot::stat_name(slot));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(gained: &[(usize, u8)], idle: Option<u64>) -> script::ScriptProgress {
        let mut all = [0u8; script::SKILL_SLOTS];
        for (slot, n) in gained {
            all[*slot] = *n;
        }
        script::ScriptProgress {
            running_for: Duration::from_secs(3 * 3600 + 5 * 60 + 9),
            idle_for: idle.map(Duration::from_secs),
            gained: all,
        }
    }

    #[test]
    fn spans_read_at_the_unit_that_matters() {
        assert_eq!(span_label(Duration::from_secs(45)), "45s");
        assert_eq!(span_label(Duration::from_secs(12 * 60 + 40)), "12m");
        assert_eq!(span_label(Duration::from_secs(3 * 3600 + 5 * 60)), "3h05m");
        assert_eq!(
            span_label(Duration::from_secs(2 * 86_400 + 4 * 3600)),
            "2d04h"
        );
    }

    #[test]
    fn cells_show_a_dash_without_a_run_or_a_running_clock() {
        let frozen = progress(&[], None);
        assert_eq!(idle_label(Some(&frozen)), NONE, "paused: no idle time");
        assert_eq!(runtime_label(Some(&frozen)), "3h05m");
        assert_eq!(idle_label(Some(&progress(&[], Some(90)))), "1m");
    }
    #[test]
    fn unarmed_slots_render_absent_progress_in_all_cells() {
        assert_eq!(runtime_label(None), NONE, "unarmed: no run");
        assert_eq!(idle_label(None), NONE, "unarmed: no clock");
        assert_eq!(levels_label(None), NONE, "unarmed: no gains");
        // An armed-but-zero run must not look absent.
        let zero = script::ScriptProgress {
            running_for: Duration::from_secs(0),
            idle_for: Some(Duration::from_secs(0)),
            gained: [0u8; script::SKILL_SLOTS],
        };
        assert_eq!(runtime_label(Some(&zero)), "0s");
        assert_eq!(idle_label(Some(&zero)), "0s");
        assert_eq!(levels_label(Some(&zero)), "+0");
    }

    #[test]
    fn levels_are_totalled_and_named() {
        let p = progress(&[(8, 2), (10, 1)], Some(1));
        assert_eq!(levels_label(Some(&p)), "+3");
        assert_eq!(levels_detail(&p), "woodcutting +2, fishing +1");
        assert_eq!(levels_detail(&progress(&[], Some(1))), "");
    }
}
