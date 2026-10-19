//! The small native adapter each front end supplies. The core decides when a
//! slot lifetime starts or ends; the surface only supplies and releases its
//! own per-slot devices and drops its own per-lifetime facts.

use std::sync::Arc;

use vault::Profile;

/// The per-slot frame mailbox and input channel the panel exchanges with
/// each slot every frame (named re-exports by ruling D1).
pub use host::{FrameBuf, SlotInput};

/// One operator input event for a slot's client, in applet coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEv {
    /// The pointer moved.
    Move { x: i32, y: i32 },
    /// A mouse button went down (1 left, 2 right).
    Down { button: i32, x: i32, y: i32 },
    /// The mouse button went up.
    Up,
    /// A key changed.
    Key { down: bool, ch: i32 },
}

impl InputEv {
    fn to_host(self) -> host::InputEv {
        match self {
            Self::Move { x, y } => host::InputEv::Move { x, y },
            Self::Down { button, x, y } => host::InputEv::Down { button, x, y },
            Self::Up => host::InputEv::Up,
            Self::Key { down, ch } => host::InputEv::Key { down, ch },
        }
    }
}

/// The operator's input feed into one slot's client: connected to the
/// slot's input channel, it forwards each event to the host.
#[derive(Debug)]
pub struct InputSender(Box<std::sync::mpsc::Sender<host::InputEv>>);

impl InputSender {
    /// Connect a new feed to `input` (replacing any earlier feed).
    pub fn connect(input: &SlotInput) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        Self(Box::new(tx))
    }

    /// A feed whose events land in the returned probe instead of a slot
    /// (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn probe() -> (Self, InputProbe) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self(Box::new(tx)), InputProbe(Box::new(rx)))
    }

    /// Another handle on the same feed.
    pub fn duplicate(&self) -> Self {
        Self(Box::new((*self.0).clone()))
    }

    /// Forward `event`; `false` once the slot dropped its end.
    pub fn send(&self, event: InputEv) -> bool {
        self.0.send(event.to_host()).is_ok()
    }
}

use crate::command::RasterMode;

/// The disposable profile copy handed to one worker spawn: a surface may
/// force its raster mode or apply a session-only memory choice. It is
/// never written back to the vault.
pub struct SpawnProfile<'a> {
    profile: &'a mut Profile,
}

impl<'a> SpawnProfile<'a> {
    pub(crate) fn new(profile: &'a mut Profile) -> Self {
        Self { profile }
    }

    /// The profile's username.
    pub fn username(&self) -> &str {
        &self.profile.username
    }

    /// The spawn's raster mode.
    pub fn raster(&self) -> RasterMode {
        RasterMode::from_vault(self.profile.settings.raster)
    }

    /// Force the spawn's raster mode.
    pub fn set_raster(&mut self, raster: RasterMode) {
        self.profile.settings.raster = raster.to_vault();
    }

    /// The spawn's low-memory handshake.
    pub fn lowmem(&self) -> bool {
        self.profile.settings.lowmem
    }

    /// Apply a session-only memory choice to this spawn.
    pub fn set_lowmem(&mut self, lowmem: bool) {
        self.profile.settings.lowmem = lowmem;
    }
}

/// Host handles for one worker spawn plus the surface-owned value the core
/// retains for that slot (and hands back when a terminal lifetime restarts
/// or a cancelled removal re-adds the member).
pub struct SlotAttach<Io> {
    io: Io,
    input: Option<Arc<SlotInput>>,
    mailbox: Option<Arc<FrameBuf>>,
}

impl<Io> SlotAttach<Io> {
    /// Host handles for one worker spawn.
    pub fn new(io: Io, input: Option<Arc<SlotInput>>, mailbox: Option<Arc<FrameBuf>>) -> Self {
        Self { io, input, mailbox }
    }

    /// Surface-owned value retained for this slot.
    pub fn io(&self) -> &Io {
        &self.io
    }

    /// Consume the attach into its parts.
    pub fn into_parts(self) -> (Io, Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>) {
        (self.io, self.input, self.mailbox)
    }

    /// Slot input handle, when the surface supplies one.
    pub fn input(&self) -> Option<&Arc<SlotInput>> {
        self.input.as_ref()
    }

    /// Frame mailbox, when the surface supplies one.
    pub fn mailbox(&self) -> Option<&Arc<FrameBuf>> {
        self.mailbox.as_ref()
    }
}

pub trait SlotSurface {
    /// Per-slot devices the surface keeps (panel: input channel and frame
    /// mailbox; headless: nothing).
    type Io;

    /// Prepare one worker spawn for `name`. `profile` is the disposable copy
    /// handed to the host, never written back to the vault; a headless
    /// surface forces its raster off here. `retained` is the previous
    /// lifetime's IO when the member is being restarted or re-added.
    fn attach(
        &mut self,
        name: &str,
        profile: &mut SpawnProfile<'_>,
        retained: Option<Self::Io>,
    ) -> SlotAttach<Self::Io>;

    /// A slot lifetime boundary for `name` (spawn or removal): username
    /// reuse must not inherit the previous lifetime's published facts. The
    /// core already dropped its own frame facts for `name`; `walk_was_armed`
    /// says whether that ended an armed walk.
    fn lifetime_reset(&mut self, name: &str, walk_was_armed: bool);

    /// `name` left the fleet: release per-slot devices (audio, capture).
    fn released(&mut self, name: &str);
}

/// A surface with no per-slot devices: raster forced off, no frame mailbox,
/// no input channel. Front ends that publish per-slot facts wrap their own
/// reset in [`HeadlessSurface::with_reset`].
pub struct HeadlessSurface<F: FnMut(&str)> {
    reset: F,
}

impl HeadlessSurface<fn(&str)> {
    pub fn new() -> Self {
        Self { reset: |_| {} }
    }
}

impl Default for HeadlessSurface<fn(&str)> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: FnMut(&str)> HeadlessSurface<F> {
    pub fn with_reset(reset: F) -> Self {
        Self { reset }
    }
}

impl<F: FnMut(&str)> SlotSurface for HeadlessSurface<F> {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> SlotAttach<()> {
        profile.set_raster(RasterMode::Off);
        SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, name: &str, _walk_was_armed: bool) {
        (self.reset)(name);
    }

    fn released(&mut self, _name: &str) {}
}

/// Map a point in a slot image of `image_w`×`image_h` to applet
/// coordinates for a click; `None` outside the image.
pub fn map_image_to_applet(
    local_x: f32,
    local_y: f32,
    image_w: f32,
    image_h: f32,
) -> Option<(i32, i32)> {
    host::map_image_to_applet(local_x, local_y, image_w, image_h)
}

impl InputEv {
    #[cfg(any(test, feature = "test-support"))]
    fn from_host(event: host::InputEv) -> Self {
        match event {
            host::InputEv::Move { x, y } => Self::Move { x, y },
            host::InputEv::Down { button, x, y } => Self::Down { button, x, y },
            host::InputEv::Up => Self::Up,
            host::InputEv::Key { down, ch } => Self::Key { down, ch },
        }
    }
}

/// The receiving end of [`InputSender::probe`] (tests).
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct InputProbe(Box<std::sync::mpsc::Receiver<host::InputEv>>);

#[cfg(any(test, feature = "test-support"))]
impl InputProbe {
    /// The next sent event, if one is waiting.
    pub fn try_recv(&self) -> Result<InputEv, std::sync::mpsc::TryRecvError> {
        self.0.try_recv().map(InputEv::from_host)
    }

    /// Every waiting event.
    pub fn try_iter(&self) -> impl Iterator<Item = InputEv> + '_ {
        self.0.try_iter().map(InputEv::from_host)
    }
}

#[cfg(test)]
mod tests {
    use super::InputEv;

    #[test]
    fn every_input_event_converts_to_the_same_host_event() {
        let cases = [
            (
                InputEv::Move { x: 1, y: 2 },
                host::InputEv::Move { x: 1, y: 2 },
            ),
            (
                InputEv::Down {
                    button: 2,
                    x: 3,
                    y: 4,
                },
                host::InputEv::Down {
                    button: 2,
                    x: 3,
                    y: 4,
                },
            ),
            (InputEv::Up, host::InputEv::Up),
            (
                InputEv::Key { down: true, ch: 65 },
                host::InputEv::Key { down: true, ch: 65 },
            ),
        ];
        // The host event has no `PartialEq`; its `Debug` form names the
        // variant and every field.
        for (event, host) in cases {
            assert_eq!(format!("{:?}", event.to_host()), format!("{host:?}"));
        }
    }
}
