//! A running script's paint frame as the front ends draw it (T3.3):
//! borrowed views over the frame the slot already holds, so a draw clones
//! nothing, plus the canvas rasterizer and a reusable cache of the last
//! drawn canvas.
//!
//! A frame from the standard paint-controls helper also advertises the
//! host's own Pause/Resume and Stop controls under fixed ids. This module
//! owns their meaning: the label follows the run's lifecycle state read
//! with the frame, and activating one yields a [`PaintInput`] bound to the
//! displayed profile, run, frame generation and action. The command seam
//! checks that binding again before it issues the existing Pause, Resume or
//! Stop row (`session::commands`), so a late click never controls another
//! profile or a replacement run, and never turns a Pause into a Resume.

use script::canvas;
use script::canvas::CanvasOp;
use script::shim::{
    PaintChromeBand, ScriptPaint, ScriptPaintButton, HOST_PAUSE_CONTROL_ID, HOST_STOP_CONTROL_ID,
};

use super::{PaintAction, PaintInput, ScriptRunState};
use crate::command::OperatorCommand;
use crate::selection::ProfileIdentity;

/// A host lifecycle control on a helper-generated paint: what activating
/// it does now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintControl {
    /// Pause the run.
    Pause,
    /// Resume the paused run.
    Resume,
    /// Stop the run.
    Stop,
}

impl PaintControl {
    /// The button label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pause => "Pause",
            Self::Resume => "Resume",
            Self::Stop => "Stop",
        }
    }

    /// Whether the control acts on a run in `state`: Pause while starting
    /// or running, Resume while paused, Stop while starting, running or
    /// paused. The lifecycle decides, never a native status phase, so a
    /// running card whose status reads Blocked keeps both controls and an
    /// idle slot with a terminal Blocked detail has none.
    pub const fn applies(self, state: ScriptRunState) -> bool {
        match self {
            Self::Pause => matches!(state, ScriptRunState::Starting | ScriptRunState::Running),
            Self::Resume => matches!(state, ScriptRunState::Paused),
            Self::Stop => matches!(
                state,
                ScriptRunState::Starting | ScriptRunState::Running | ScriptRunState::Paused
            ),
        }
    }

    /// The existing row the control issues.
    pub(crate) const fn command(self) -> OperatorCommand {
        match self {
            Self::Pause => OperatorCommand::PauseScript,
            Self::Resume => OperatorCommand::ResumeScript,
            Self::Stop => OperatorCommand::StopScript,
        }
    }
}

/// The fixed role a host-control button id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlRole {
    PauseResume,
    Stop,
}

impl ControlRole {
    fn of(id: &str) -> Option<Self> {
        match id {
            HOST_PAUSE_CONTROL_ID => Some(Self::PauseResume),
            HOST_STOP_CONTROL_ID => Some(Self::Stop),
            _ => None,
        }
    }

    /// The action the role shows for a run in `state`.
    const fn control(self, state: ScriptRunState) -> PaintControl {
        match (self, state) {
            (Self::PauseResume, ScriptRunState::Paused) => PaintControl::Resume,
            (Self::PauseResume, _) => PaintControl::Pause,
            (Self::Stop, _) => PaintControl::Stop,
        }
    }

    const fn shows(self, control: PaintControl) -> bool {
        matches!(
            (self, control),
            (
                Self::PauseResume,
                PaintControl::Pause | PaintControl::Resume
            ) | (Self::Stop, PaintControl::Stop)
        )
    }
}

/// Whether `id` is one of the host's own control ids (never a script
/// click).
pub(crate) fn is_host_control(id: &str) -> bool {
    ControlRole::of(id).is_some()
}

/// Whether `paint` advertises `control`'s role.
pub(crate) fn advertises_control(paint: &ScriptPaint, control: PaintControl) -> bool {
    paint
        .buttons
        .iter()
        .any(|button| ControlRole::of(&button.id).is_some_and(|role| role.shows(control)))
}

/// Whether `paint` advertises any host control (only then does a read
/// capture the run).
pub(crate) fn advertises_any_control(paint: &ScriptPaint) -> bool {
    paint
        .buttons
        .iter()
        .any(|button| is_host_control(&button.id))
}

/// One script run: the slot's lifetime and the run generation, which moves
/// at every Start and Stop and never at Pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunToken {
    slot: u64,
    generation: u64,
}

impl RunToken {
    pub(crate) const fn new(slot: u64, generation: u64) -> Self {
        Self { slot, generation }
    }
}

/// The run a paint frame was read with: its profile, the run the slot held
/// and that run's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaintRun {
    pub(crate) profile: ProfileIdentity,
    pub(crate) run: Option<RunToken>,
    pub(crate) state: ScriptRunState,
}

/// A host control activation: the action shown, and the profile, run and
/// frame generation it was shown for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ControlActivation {
    pub(crate) control: PaintControl,
    pub(crate) profile: ProfileIdentity,
    pub(crate) run: RunToken,
    pub(crate) generation: u64,
}

/// A script's paint frame held across UI frames (shared with the slot's
/// status row; never deep-copied), with the run it was read with when it
/// advertises host controls.
#[derive(Debug)]
pub struct PaintFrame {
    paint: std::sync::Arc<ScriptPaint>,
    run: Option<PaintRun>,
}

impl PaintFrame {
    pub(crate) fn new(paint: std::sync::Arc<ScriptPaint>, run: Option<PaintRun>) -> Self {
        Self { paint, run }
    }

    /// A paint frame over `paint` (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_script(paint: ScriptPaint) -> Self {
        Self::new(std::sync::Arc::new(paint), None)
    }

    /// A paint frame over `paint` read with a run of `profile` in `state`
    /// (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_run(paint: ScriptPaint, profile: &str, state: ScriptRunState) -> Self {
        Self::new(
            std::sync::Arc::new(paint),
            Some(PaintRun {
                profile: ProfileIdentity::synthetic(profile),
                run: Some(RunToken::new(1, 1)),
                state,
            }),
        )
    }

    /// The frame as a borrowed view.
    pub fn view(&self) -> PaintView<'_> {
        PaintView {
            paint: &self.paint,
            run: self.run,
        }
    }

    /// Another handle on the same frame.
    pub fn share(&self) -> Self {
        Self {
            paint: std::sync::Arc::clone(&self.paint),
            run: self.run,
        }
    }
}

/// A script's paint frame.
#[derive(Debug, Clone, Copy)]
pub struct PaintView<'a> {
    paint: &'a ScriptPaint,
    run: Option<PaintRun>,
}

impl<'a> PaintView<'a> {
    pub(crate) fn new(paint: &'a ScriptPaint) -> Self {
        Self { paint, run: None }
    }

    /// The panel title.
    pub fn title(&self) -> Option<&'a str> {
        self.paint.title.as_deref()
    }

    /// The accent colour text.
    pub fn accent(&self) -> Option<&'a str> {
        self.paint.accent.as_deref()
    }

    /// The text lines.
    pub fn lines(&self) -> &'a [String] {
        &self.paint.lines
    }

    /// The buttons, in order. A host control's label and enablement follow
    /// the run state read with the frame.
    pub fn buttons(&self) -> impl ExactSizeIterator<Item = PaintButtonView<'a>> + 'a {
        let state = self.run.map(|run| run.state);
        self.paint
            .buttons
            .iter()
            .map(move |button| PaintButtonView::new(button, state))
    }

    /// What activating button `index` does: a script click for an ordinary
    /// button, or a host control bound to this frame's profile, run,
    /// generation and shown action. `None` for a missing button or a host
    /// control that does not apply to the run read with the frame.
    pub fn activate(&self, index: usize) -> Option<PaintInput> {
        let button = self.paint.buttons.get(index)?;
        let Some(role) = ControlRole::of(&button.id) else {
            return Some(PaintInput::click(button.id.clone(), self.paint.generation));
        };
        let run = self.run?;
        let control = role.control(run.state);
        if !control.applies(run.state) {
            return None;
        }
        Some(PaintInput(PaintAction::Control(ControlActivation {
            control,
            profile: run.profile,
            run: run.run?,
            generation: self.paint.generation,
        })))
    }

    /// The canvas operations.
    pub fn canvas(&self) -> CanvasOps<'a> {
        CanvasOps(&self.paint.canvas)
    }

    /// The frame generation (a click names the generation it answers).
    pub fn generation(&self) -> u64 {
        self.paint.generation
    }

    /// The chrome strip band.
    pub fn strip(&self) -> Option<ChromeBandView<'a>> {
        self.paint.strip.as_ref().map(ChromeBandView)
    }

    /// The chrome rail band.
    pub fn rail(&self) -> Option<ChromeBandView<'a>> {
        self.paint.rail.as_ref().map(ChromeBandView)
    }

    /// The footer text.
    pub fn footer(&self) -> Option<&'a str> {
        self.paint.footer.as_deref()
    }

    /// The tab bands, in order.
    pub fn tabs(&self) -> impl ExactSizeIterator<Item = ChromeBandView<'a>> + 'a {
        self.paint.tabs.iter().map(ChromeBandView)
    }
}

/// One paint button.
#[derive(Debug, Clone, Copy)]
pub struct PaintButtonView<'a> {
    button: &'a ScriptPaintButton,
    control: Option<PaintControl>,
    enabled: bool,
}

impl<'a> PaintButtonView<'a> {
    fn new(button: &'a ScriptPaintButton, state: Option<ScriptRunState>) -> Self {
        let Some(role) = ControlRole::of(&button.id) else {
            return Self {
                button,
                control: None,
                enabled: true,
            };
        };
        let state = state.unwrap_or(ScriptRunState::Idle);
        let control = role.control(state);
        Self {
            button,
            control: Some(control),
            enabled: control.applies(state),
        }
    }

    /// The advertised id (a widget key; activation goes through
    /// [`PaintView::activate`]).
    pub fn id(&self) -> &'a str {
        &self.button.id
    }

    /// The label: the script's own, or a host control's current action.
    pub fn label(&self) -> &'a str {
        match self.control {
            Some(control) => control.label(),
            None => &self.button.label,
        }
    }

    /// The host control this button is, if any.
    pub fn control(&self) -> Option<PaintControl> {
        self.control
    }

    /// Whether activating the button does anything now (a script button
    /// always forwards its click).
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

/// One chrome band (strip, rail or tab): a named choice list.
#[derive(Debug, Clone, Copy)]
pub struct ChromeBandView<'a>(&'a PaintChromeBand);

impl<'a> ChromeBandView<'a> {
    /// The store key a selection sends back.
    pub fn id(&self) -> &'a str {
        &self.0.id
    }

    /// The choices.
    pub fn names(&self) -> &'a [String] {
        &self.0.names
    }

    /// The selected choice.
    pub fn selected(&self) -> &'a str {
        &self.0.selected
    }

    /// The status text.
    pub fn status(&self) -> Option<&'a str> {
        self.0.status.as_deref()
    }

    /// The brand text.
    pub fn brand(&self) -> Option<&'a str> {
        self.0.brand.as_deref()
    }
}

/// A paint frame's canvas operations.
#[derive(Debug, Clone, Copy)]
pub struct CanvasOps<'a>(&'a [CanvasOp]);

impl<'a> CanvasOps<'a> {
    /// No operations.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The applet-space rectangle `[x, y, w, h]` the operations touch.
    pub fn dirty_bounds(&self) -> Option<[i32; 4]> {
        #[cfg(feature = "canvas-raster")]
        {
            canvas::dirty_bounds(self.0).map(|d| [d.x, d.y, d.w, d.h])
        }
        #[cfg(not(feature = "canvas-raster"))]
        {
            None
        }
    }

    /// Rasterize the operations (bounded; `None` past the operation limit
    /// or with nothing to draw).
    pub fn rasterize(&self) -> Option<CanvasRaster> {
        #[cfg(feature = "canvas-raster")]
        {
            canvas::rasterize(self.0).map(|raster| CanvasRaster {
                x: raster.x,
                y: raster.y,
                width: raster.w,
                height: raster.h,
                rgba: raster.rgba,
            })
        }
        #[cfg(not(feature = "canvas-raster"))]
        {
            None
        }
    }
}

/// A rasterized canvas: an RGBA8 image placed in applet space.
#[derive(Debug)]
pub struct CanvasRaster {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl CanvasRaster {
    /// The applet-space left edge.
    pub fn x(&self) -> i32 {
        self.x
    }

    /// The applet-space top edge.
    pub fn y(&self) -> i32 {
        self.y
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The RGBA8 pixels, row-major.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// The canvas operations a front end drew last, kept to skip an unchanged
/// re-raster. Its buffer is reused, so a steady canvas allocates nothing.
#[derive(Debug, Default)]
pub struct CanvasCache(Vec<CanvasOp>);

impl CanvasCache {
    /// Whether `ops` are the operations drawn last.
    pub fn same_as(&self, ops: CanvasOps<'_>) -> bool {
        self.0.as_slice() == ops.0
    }

    /// Remember `ops` as drawn.
    pub fn store(&mut self, ops: CanvasOps<'_>) {
        super::refill(&mut self.0, ops.0);
    }

    /// Forget the last canvas.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Map an applet-space rectangle onto a game image drawn at `min` with
/// `size`: the screen rectangle `[x, y, w, h]`.
pub fn map_applet_rect(min: [f32; 2], size: [f32; 2], x: i32, y: i32, w: i32, h: i32) -> [f32; 4] {
    canvas::map_applet_rect(min, size, x, y, w, h)
}
