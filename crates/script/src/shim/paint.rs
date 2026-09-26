/// One advertised script-local paint control. `id` is the one-shot click
/// token; `label` is display-only and never implies a walk, pause, or packet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ScriptPaintButton {
    pub id: String,
    pub label: String,
}

/// Advertised strip, rail, or tabs band on a recorded paint frame.
/// `selected` is the stored name if it is still in `names`, else the
/// first advertised name. `status` / `brand` are strip-only.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
pub struct PaintChromeBand {
    pub id: String,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub selected: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub brand: Option<String>,
}

/// One recorded paint frame (`Paint.begin(...)` ... `end()`): the title,
/// the accent colour, the rows (gap rows are empty lines), optional
/// one-shot buttons, optional canvas ops, and optional advertised chrome.
/// The host reads it off `__rs2b0t_host.paint` for the script paint views.
/// Older objects omit chrome fields; they deserialize empty.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
pub struct ScriptPaint {
    pub title: Option<String>,
    pub accent: Option<String>,
    pub lines: Vec<String>,
    /// Absent on older `host.paint` objects; empty means no controls.
    #[serde(default)]
    pub buttons: Vec<ScriptPaintButton>,
    /// Recorded Canvas ops for this onPaint call. Absent on older
    /// `host.paint` objects; empty means no applet-space canvas.
    #[serde(default)]
    pub canvas: Vec<crate::canvas::CanvasOp>,
    /// Host-owned rendered-frame generation. Not a JS field; stamped when
    /// the isolate forwards the frame so a stale overlay cannot target a
    /// later script that advertises the same id.
    #[serde(default)]
    pub generation: u64,
    /// Brand strip advertised this frame. Absent on older `host.paint`.
    #[serde(default)]
    pub strip: Option<PaintChromeBand>,
    /// Vertical rail advertised this frame. Absent / None when skipped.
    #[serde(default)]
    pub rail: Option<PaintChromeBand>,
    /// Right-aligned byline. Not a `lines` entry.
    #[serde(default)]
    pub footer: Option<String>,
    /// Enabled-script `tabs()` bands in call order.
    #[serde(default)]
    pub tabs: Vec<PaintChromeBand>,
}
