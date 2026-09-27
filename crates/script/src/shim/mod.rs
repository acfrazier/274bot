//! rs2b0t import-remap shim: extra rustyscript modules that stand in for
//! the rs2b0t api tree. Their names (`../../api/...`, `../../paint/...`,
//! `../../runtime/...`) and the `@rs2b0t/api` bundle resolve to these
//! modules; the rs2b0t sources are never executed. Missing members throw
//! `not impl: <throw reason>` — never a fake value.

mod content;
mod interact;
mod modules;
mod paint;

pub(crate) use content::content_json;
#[cfg(test)]
pub(crate) use interact::RejectedRow;
pub use interact::{InspectAvoidWire, InteractReq};
pub(crate) use interact::{MaybeInteractReq, QueuedRow};
pub(crate) use modules::{remap_catalog_imports, shim_modules, BOT_MODULE, MAIN_MODULE, PRELUDE};
#[allow(unused_imports)]
pub(crate) use modules::{remap_hash_bot, remap_rs2b0t_api};
pub use paint::{PaintChromeBand, ScriptPaint, ScriptPaintButton};
