//! rs2b0t import-remap shim: extra rustyscript modules that stand in for
//! the rs2b0t api tree. Their names (`../../api/...`, `../../paint/...`,
//! `../../runtime/...`) and the `@rs2b0t/api` bundle resolve to these
//! modules; the rs2b0t sources are never executed. Missing members throw
//! `not impl: <throw reason>` — never a fake value.

#[cfg(feature = "load")]
mod content;
#[cfg(feature = "load")]
mod modules;
#[cfg(feature = "load")]
mod rows;

pub use crate::interact::{InspectAvoidWire, InteractReq};
pub use crate::paint::{
    host_control_buttons, PaintChromeBand, ScriptPaint, ScriptPaintButton, HOST_PAUSE_CONTROL_ID,
    HOST_STOP_CONTROL_ID,
};
#[cfg(feature = "load")]
pub(crate) use content::content_json;
#[cfg(feature = "load")]
pub(crate) use modules::{remap_catalog_imports, shim_modules, BOT_MODULE, MAIN_MODULE, PRELUDE};
#[cfg(feature = "load")]
#[allow(unused_imports)]
pub(crate) use modules::{remap_hash_bot, remap_rs2b0t_api};
#[cfg(all(test, feature = "load"))]
pub(crate) use rows::RejectedRow;
#[cfg(feature = "load")]
pub(crate) use rows::{MaybeInteractReq, QueuedRow};

#[cfg(feature = "load")]
/// Field writes performed by the native `InvItem` constructor, in order.
#[derive(Clone, Copy)]
pub(crate) enum InvItemFieldInit {
    Copy,
    NullishZero,
    StrictTrue,
    NullishMinusOne,
}

#[cfg(feature = "load")]
pub(crate) static INV_ITEM_FIELDS: [(&str, InvItemFieldInit); 6] = [
    ("name", InvItemFieldInit::Copy),
    ("count", InvItemFieldInit::Copy),
    ("id", InvItemFieldInit::NullishZero),
    ("slot", InvItemFieldInit::NullishZero),
    ("noted", InvItemFieldInit::StrictTrue),
    ("cert", InvItemFieldInit::NullishMinusOne),
];
