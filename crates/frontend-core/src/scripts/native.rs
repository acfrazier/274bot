//! Native projection values; M-297 connects the existing Scripts command owner.
use api::selected::RunKey;
use script::native::ScriptStatus;
use script::shim::ScriptPaint;
use script::{RunState, ScriptLifecycleReceipt, SettingDef};
use std::sync::Arc;

pub enum SchemaView<'a> {
    Unavailable(&'a str),
    Ready {
        version: u16,
        fields: &'a [SettingDef],
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCommand {
    Pause,
    Resume,
    Stop,
    Retry,
}
#[derive(Debug, Clone)]
pub struct NativeTarget {
    pub profile: String,
    pub run: RunKey,
}
pub struct NativeDetail {
    pub lifecycle: RunState,
    pub status: Option<Arc<ScriptStatus>>,
    pub paint: Option<Arc<ScriptPaint>>,
    pub terminal: Option<ScriptLifecycleReceipt>,
}
