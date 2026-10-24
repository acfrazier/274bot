//! rs2b0t import-remap shim: extra rustyscript modules that stand in for
//! the rs2b0t api tree. Their names (`../../api/...`, `../../paint/...`,
//! `../../runtime/...`) and the `@rs2b0t/api` bundle resolve to these
//! modules; the rs2b0t sources are never executed. Missing members throw
//! `not impl: <throw reason>` — never a fake value.

mod content;
mod modules;
mod rows;

pub use content::content_json;
pub use modules::{remap_catalog_imports, shim_modules, BOT_MODULE, MAIN_MODULE, PRELUDE};
#[allow(unused_imports)]
pub(crate) use modules::{remap_hash_bot, remap_rs2b0t_api};
#[cfg(test)]
pub use rows::RejectedRow;
pub use rows::{MaybeInteractReq, QueuedRow};

/// Field writes performed by the native `InvItem` constructor, in order.
#[derive(Clone, Copy)]
pub enum InvItemFieldInit {
    Copy,
    NullishZero,
    StrictTrue,
    NullishMinusOne,
}

pub static INV_ITEM_FIELDS: [(&str, InvItemFieldInit); 6] = [
    ("name", InvItemFieldInit::Copy),
    ("count", InvItemFieldInit::Copy),
    ("id", InvItemFieldInit::Copy),
    ("slot", InvItemFieldInit::NullishZero),
    ("noted", InvItemFieldInit::StrictTrue),
    ("cert", InvItemFieldInit::NullishMinusOne),
];
