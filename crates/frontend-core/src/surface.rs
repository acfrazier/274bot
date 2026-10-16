//! The small native adapter each front end supplies. The core decides when a
//! slot lifetime starts or ends; the surface only supplies and releases its
//! own per-slot devices and drops its own per-lifetime facts.

use std::sync::Arc;

use host::{FrameBuf, SlotInput};
use vault::Profile;

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
        profile: &mut Profile,
        retained: Option<Self::Io>,
    ) -> SlotAttach<Self::Io>;

    /// A slot lifetime boundary for `name` (spawn or removal): username
    /// reuse must not inherit the previous lifetime's published facts.
    fn lifetime_reset(&mut self, name: &str);

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
        profile: &mut Profile,
        _retained: Option<()>,
    ) -> SlotAttach<()> {
        profile.settings.raster = vault::RasterMode::Off;
        SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, name: &str) {
        (self.reset)(name);
    }

    fn released(&mut self, _name: &str) {}
}
