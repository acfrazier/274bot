//! Per-script parameter defaults: the key/value rows the panel uncollapses
//! under Parameters. Empty for every id until the ports fill a schema.

use crate::registry::CompiledId;

/// Default parameter rows for `id`, in display order.
pub fn defaults(id: CompiledId) -> Vec<(String, String)> {
    match id.0 {
        // No port ships a schema yet: every compiled id defaults to nothing.
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiled_ids;

    #[test]
    fn every_compiled_id_defaults_to_no_rows_yet() {
        for id in compiled_ids() {
            assert!(
                defaults(*id).is_empty(),
                "{} must not ship defaults until its port fills a schema",
                id.0
            );
        }
    }
}
