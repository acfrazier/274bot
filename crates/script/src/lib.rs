//! Compiled script runner: the `Script` trait, per-tick context, and the
//! per-uid `SlotScript` state machine. Load (out-of-tree JS) lives behind
//! the `load` feature: a picker library of JS cards plus a rustyscript/V8
//! isolate spawned only on Start.

pub mod ctx;
pub mod load;
pub mod params;
pub mod ported;
pub mod registry;
pub mod slot;

pub use ctx::{Script, ScriptCtx};
pub use load::{JsCard, JsLibrary, LoadShape, ScriptSel, default_js_store, detect_shape};
pub use params::defaults;
pub use registry::{CompiledId, compiled_ids, factory, is_whale};
pub use slot::{RunState, SlotScript};

#[cfg(feature = "load")]
pub use load::LoadIsolate;
