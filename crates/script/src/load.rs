//! JS Load shape detection. No V8 in-tree yet: a cheap marker scan
//! decides which loader a future Load would route a source to. rustyscript
//! arrives next task.

/// Which loader a JS source belongs to.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadShape {
    /// Old rs2b0t `defineBot(...)` / manifest-flagged source.
    CompatDefineBot,
    /// Modern source exporting a `tick` function.
    NativeTick,
    /// Not a recognized bot shape.
    Reject,
}

/// Classify a JS source by marker scan. Compat markers win over the
/// native `tick` export when a source carries both.
pub fn detect_shape(source: &str) -> LoadShape {
    if source.contains("defineBot(") || source.contains("__rs2b0tManifest") {
        LoadShape::CompatDefineBot
    } else if source.contains("export function tick") || source.contains("export async function tick") {
        LoadShape::NativeTick
    } else {
        LoadShape::Reject
    }
}
