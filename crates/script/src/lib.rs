//! Compiled script runner: the `Script` trait, per-tick context, and the
//! per-uid `SlotScript` state machine. Load (out-of-tree JS) lives behind
//! the `load` feature: a picker library of JS cards plus a rustyscript/V8
//! isolate spawned only on Start.

pub mod ctx;
pub mod load;
pub mod params;
pub mod ported;
pub mod registry;
pub mod rs2b0t_registry;
pub mod slot;

pub use ctx::{DetectedRandom, FindOptions, RandomClaim, Script, ScriptCtx};
pub use load::{
    default_js_store, detect_shape, is_reserved, JsCard, JsLibrary, LoadShape, ScriptSel,
};
pub use params::defaults;
pub use registry::{compiled_ids, factory, is_whale, CompiledId};
pub use rs2b0t_registry::{
    default_rs2b0t_path_file, parse_registry, persist_rs2b0t_root, persist_rs2b0t_root_at,
    rs2b0t_root, rs2b0t_root_at, script_file_path, RegistryCard,
};
pub use slot::{RunState, SlotScript};

#[cfg(feature = "load")]
pub use load::{transpile_ts, LoadIsolate};
