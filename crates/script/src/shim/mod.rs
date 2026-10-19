//! rs2b0t import-remap shim: extra rustyscript modules that stand in for
//! the rs2b0t api tree. Their names (`../../api/...`, `../../paint/...`,
//! `../../runtime/...`) and the `@rs2b0t/api` bundle resolve to these
//! modules; the rs2b0t sources are never executed. Missing members throw
//! `not impl: <throw reason>` — never a fake value.

#[cfg(feature = "load")]
mod content;
mod interact;
#[cfg(feature = "load")]
mod modules;
mod paint;

#[cfg(feature = "load")]
pub(crate) use content::content_json;
#[cfg(all(test, feature = "load"))]
pub(crate) use interact::RejectedRow;
pub use interact::{InspectAvoidWire, InteractReq};
#[cfg(feature = "load")]
pub(crate) use interact::{MaybeInteractReq, QueuedRow};
#[cfg(feature = "load")]
pub(crate) use modules::{remap_catalog_imports, shim_modules, BOT_MODULE, MAIN_MODULE, PRELUDE};
#[cfg(feature = "load")]
#[allow(unused_imports)]
pub(crate) use modules::{remap_hash_bot, remap_rs2b0t_api};
pub use paint::{PaintChromeBand, ScriptPaint, ScriptPaintButton};

/// Field writes performed by the native `InvItem` constructor, in order.
#[derive(Clone, Copy)]
pub(crate) enum InvItemFieldInit {
    Copy,
    NullishZero,
    StrictTrue,
    NullishMinusOne,
}

pub(crate) const INV_ITEM_FIELDS: [(&str, InvItemFieldInit); 6] = [
    ("name", InvItemFieldInit::Copy),
    ("count", InvItemFieldInit::Copy),
    ("id", InvItemFieldInit::NullishZero),
    ("slot", InvItemFieldInit::NullishZero),
    ("noted", InvItemFieldInit::StrictTrue),
    ("cert", InvItemFieldInit::NullishMinusOne),
];
