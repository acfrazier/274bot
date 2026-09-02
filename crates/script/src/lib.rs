//! Compiled script runner: the `Script` trait, per-tick context, and the
//! per-uid `SlotScript` state machine. Load (out-of-tree JS) lives behind
//! the `load` feature: a picker library of JS cards plus a rustyscript/V8
//! isolate spawned only on Start.

pub mod ctx;
#[cfg(feature = "load")]
pub mod isolate_fb;
#[cfg(feature = "load")]
pub mod js_cache;
pub mod load;
pub mod loadouts_store;
pub mod params;
pub mod settings_store;
pub mod ported;
pub mod registry;
pub mod rs2b0t_registry;
#[cfg(feature = "load")]
pub mod shim;
pub mod slot;

pub use ctx::{DetectedRandom, FindOptions, RandomClaim, Script, ScriptCtx};
#[cfg(feature = "load")]
pub use js_cache::{default_js_cache_root, CacheMeta, CachedJs, JsCache};
pub use load::{
    default_js_store, detect_shape, is_reserved, JsCard, JsLibrary, LoadShape, ScriptSel,
};
pub use params::defaults;
pub use loadouts_store::{
    default_loadouts_path, resolve_setting_options, Loadout, LoadoutsStore,
};
pub use settings_store::{
    card_key, coerce_setting_value, default_script_settings_path, format_setting_value, merge_bag,
    parameter_rows, setting_visible, ScriptSettingsStore,
};
pub use registry::{compiled_ids, factory, is_whale, CompiledId};
pub use rs2b0t_registry::{
    clear_rs2b0t_import_at, default_rs2b0t_import_file, default_rs2b0t_path_file, parse_registry,
    parse_registry_with_sources, persist_rs2b0t_root, persist_rs2b0t_root_at, registry_index_path,
    rs2b0t_import_deferred, rs2b0t_import_deferred_at, rs2b0t_root, rs2b0t_root_at,
    script_file_path, set_rs2b0t_import_deferred_at, RegistryCard, ScriptKind, ScriptSource,
    SettingDef,
};
pub use slot::{RunState, SlotScript};

#[cfg(feature = "load")]
pub use load::{transpile_ts, LoadIsolate};
