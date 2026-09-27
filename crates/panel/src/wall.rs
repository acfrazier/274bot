//! Wall renderer state (Task 9): the multibox chooser, the grid overlay and
//! the render-all warning. Membership and the logout latch live in the
//! shared operator core ([`frontend_core::Fleet`]).

/// Wall UI state kept outside the imgui widgets.
#[derive(Debug, Default, Clone)]
pub struct Wall {
    pub chooser_open: bool,
    pub opened_once: bool,
    pub grid: bool,
    /// Scary "render all" confirm: wanted while the user has unchecked
    /// "only render selected" but not yet accepted (OK) or backed out.
    pub render_all_warn_open: bool,
    /// "I understand" checkbox state inside that confirm.
    pub render_all_understood: bool,
}

impl Wall {
    /// Multibox on: open the chooser on the first time only.
    pub fn on_multibox_on(&mut self) {
        if !self.opened_once {
            self.chooser_open = true;
            self.opened_once = true;
        }
    }

    /// Multibox off: drop the grid overlay and any open chooser or
    /// render-all warning.
    pub fn on_multibox_off(&mut self) {
        self.grid = false;
        self.chooser_open = false;
        self.render_all_warn_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::Wall;

    #[test]
    fn first_multibox_opens_chooser_later_does_not() {
        let mut w = Wall::default();
        w.on_multibox_on();
        assert!(w.chooser_open);
        w.chooser_open = false;
        w.on_multibox_on();
        assert!(!w.chooser_open);
    }

    #[test]
    fn multibox_off_clears_grid_and_chooser() {
        let mut w = Wall::default();
        w.on_multibox_on();
        w.grid = true;
        w.render_all_warn_open = true;
        w.on_multibox_off();
        assert!(!w.grid);
        assert!(!w.chooser_open);
        assert!(!w.render_all_warn_open);
    }
}
