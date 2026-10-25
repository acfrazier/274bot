//! The per-slot status rows as a front end reads them: a retained buffer
//! refreshed once per poll and borrowed row views over it.

use host_play::SlotStatus;

use crate::OperatorSession;

/// A front end's retained copy of the slot status rows. Each refresh reuses
/// the row buffers, so a steady fleet allocates nothing.
#[derive(Debug, Default)]
pub struct StatusBuf(Vec<SlotStatus>);

impl StatusBuf {
    /// Copy the core's current rows into this buffer.
    pub fn refresh<Io>(&mut self, core: &OperatorSession<Io>) {
        core.copy_statuses_into(&mut self.0);
    }

    /// Copy the play's current rows straight from the play, without a core
    /// poll (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn refresh_from_play<Io>(&mut self, core: &OperatorSession<Io>) {
        if let Some(play) = core.play() {
            play.statuses_into(&mut self.0);
        }
    }

    /// Construct test status rows without creating a live Play.
    #[cfg(feature = "test-support")]
    pub fn from_rows_for_test<I>(rows: I) -> Self
    where
        I: IntoIterator<Item = SlotStatus>,
    {
        Self(rows.into_iter().collect())
    }

    /// The row for `name`.
    pub fn get(&self, name: &str) -> Option<SlotView<'_>> {
        self.0.iter().find(|s| s.username == name).map(SlotView)
    }

    /// Every row, in slot order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = SlotView<'_>> + '_ {
        self.0.iter().map(SlotView)
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// The raw rows, for tests which still exercise the adapter boundary.
    #[cfg(any(test, feature = "test-support"))]
    pub fn live_rows(&self) -> &[SlotStatus] {
        &self.0
    }
}

/// One slot's status row.
#[derive(Debug, Clone, Copy)]
pub struct SlotView<'a>(&'a SlotStatus);

impl<'a> SlotView<'a> {
    pub(crate) fn new(status: &'a SlotStatus) -> Self {
        Self(status)
    }

    /// The account the slot runs.
    pub fn username(&self) -> &'a str {
        &self.0.username
    }

    /// Whether the slot holds a server connection.
    pub fn connected(&self) -> bool {
        self.0.connected
    }

    /// Whether the slot is in game.
    pub fn ingame(&self) -> bool {
        self.0.ingame
    }

    /// The client's scene state (2 once the scene is loaded).
    pub fn scene_state(&self) -> i32 {
        self.0.scene_state
    }

    /// Whether the slot is up (connected or logging in).
    pub fn is_up(&self) -> bool {
        self.0.is_up()
    }
    /// Bytes received on the current stream.
    pub fn bytes_in(&self) -> u64 {
        self.0.bytes_in
    }

    /// Bytes sent on the current stream.
    pub fn bytes_out(&self) -> u64 {
        self.0.bytes_out
    }

    /// The player's tile `(x, z, level)` once the slot is in a loaded scene.
    pub fn ready_tile(&self) -> Option<(i32, i32, i32)> {
        self.0.ready_tile()
    }

    /// The newest game chat line.
    pub fn chat_head(&self) -> &'a str {
        &self.0.chat_head
    }

    /// Whether the 289 tutorial pending-text box is showing.
    pub fn tutorial_pending(&self) -> bool {
        self.0.tutorial_pending
    }

    /// Whether the slot's script paints a frame.
    pub fn has_script_paint(&self) -> bool {
        self.0.script_paint.is_some()
    }

    /// The world the slot is on, once known.
    pub fn world(&self) -> Option<u16> {
        self.0.world
    }

    /// The slot's error text, if any.
    pub fn error(&self) -> Option<&'a str> {
        self.0.error.as_deref()
    }

    /// Terminal profile-asset startup failure, if this row owns one.
    pub fn terminal_startup_error(&self) -> Option<&'a str> {
        self.0.terminal_startup_error()
    }
}
