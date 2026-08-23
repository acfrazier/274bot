//! Pure label helpers for the auto-login queue card. No ImGui.

/// How many queued slots are ahead of the given 1-based `position`.
pub fn bots_in_front(position: u32) -> u32 {
    position.saturating_sub(1)
}

/// `"{n} bot(s) in front"`, singular `bot` when exactly one is ahead.
pub fn queue_ahead_label(position: u32) -> String {
    let n = bots_in_front(position);
    if n == 1 {
        "1 bot in front".to_string()
    } else {
        format!("{n} bots in front")
    }
}

/// `"{position} of {total}"`, or `None` when either is below 1.
pub fn queue_k_of_n(position: i32, total: i32) -> Option<String> {
    if position < 1 || total < 1 {
        return None;
    }
    Some(format!("{position} of {total}"))
}

/// Fixed card title, matching rs2b0t.
pub const QUEUE_CARD_TITLE: &str = "AUTO-LOGIN QUEUE";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_labels_match_rs2b0t() {
        assert_eq!(bots_in_front(1), 0);
        assert_eq!(queue_ahead_label(1), "0 bots in front");
        assert_eq!(queue_ahead_label(2), "1 bot in front");
        assert_eq!(queue_k_of_n(1, 2).as_deref(), Some("1 of 2"));
        assert_eq!(queue_k_of_n(-1, -1), None);
    }
}
