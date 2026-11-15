//! Positive NPC admission rows. Full closure evidence is development-only.

use serde::Deserialize;

use super::ContractError;

/// One numeric operation admitted by the complete selected transform proof.
///
/// This type cannot be constructed by an external consumer.
/// ```compile_fail
/// let _: api::interact::server_ordered::NpcTransformAction =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct NpcTransformAction {
    npc_type_id: i32,
    slot: u8,
}

impl NpcTransformAction {
    /// Return the selected NPC type identity.
    pub fn npc_type_id(&self) -> i32 {
        self.npc_type_id
    }

    /// Return the proved one-based operation slot.
    pub fn slot(&self) -> u8 {
        self.slot
    }
}

/// One built-in Gatherer fishing slot admitted from its selected ordinary NPC type.
///
/// This narrow row does not establish a complete transform closure. Only the
/// built-in Gatherer knife-arm dispatcher may use it.
///
/// ```compile_fail
/// let _: api::interact::server_ordered::BuiltinGathererFishingOperation =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct BuiltinGathererFishingOperation {
    npc_type_id: i32,
    slot: u8,
}

impl BuiltinGathererFishingOperation {
    /// Return the selected NPC type identity.
    pub fn npc_type_id(&self) -> i32 {
        self.npc_type_id
    }

    /// Return the proved one-based fishing operation slot.
    pub fn slot(&self) -> u8 {
        self.slot
    }
}

#[derive(Deserialize)]
#[serde(remote = "NpcTransformAction", deny_unknown_fields)]
struct ActionWire {
    npc_type_id: i32,
    slot: u8,
}

impl super::wire::DecodeFact for NpcTransformAction {
    fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ActionWire::deserialize(deserializer)
    }
}

#[derive(Deserialize)]
#[serde(remote = "BuiltinGathererFishingOperation", deny_unknown_fields)]
struct BuiltinFishingWire {
    npc_type_id: i32,
    slot: u8,
}

impl super::wire::DecodeFact for BuiltinGathererFishingOperation {
    fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BuiltinFishingWire::deserialize(deserializer)
    }
}

/// The runtime subset of the generated NPC proof, shared by the selected catalog.
///
/// ```compile_fail
/// let _: api::interact::server_ordered::NpcTransformEvidence =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct NpcTransformEvidence {
    schema: u16,
    operations: Vec<NpcTransformAction>,
    /// Static selected-content fishing slots for the built-in Gatherer only.
    builtin_gatherer_fishing_operations: Vec<BuiltinGathererFishingOperation>,
}

#[derive(Deserialize)]
#[serde(remote = "NpcTransformEvidence", deny_unknown_fields)]
pub(super) struct EvidenceWire {
    schema: u16,
    #[serde(deserialize_with = "super::wire::decode_list")]
    operations: Vec<NpcTransformAction>,
    #[serde(deserialize_with = "super::wire::decode_list")]
    builtin_gatherer_fishing_operations: Vec<BuiltinGathererFishingOperation>,
}

impl NpcTransformEvidence {
    /// Borrow only the operations with complete positive closure proofs.
    pub fn operations(&self) -> &[NpcTransformAction] {
        &self.operations
    }

    /// Borrow the selected-content fishing slots available only to the built-in Gatherer.
    pub fn builtin_gatherer_fishing_operations(&self) -> &[BuiltinGathererFishingOperation] {
        &self.builtin_gatherer_fishing_operations
    }

    pub(super) fn builtin_gatherer_fishing_operation(
        &self,
        npc_type_id: i32,
        slot: u8,
    ) -> Option<&BuiltinGathererFishingOperation> {
        self.builtin_gatherer_fishing_operations
            .binary_search_by_key(&(npc_type_id, slot), |action| {
                (action.npc_type_id, action.slot)
            })
            .ok()
            .map(|index| &self.builtin_gatherer_fishing_operations[index])
    }

    pub(super) fn operation(&self, npc_type_id: i32, slot: u8) -> Option<&NpcTransformAction> {
        self.operations
            .binary_search_by_key(&(npc_type_id, slot), |action| {
                (action.npc_type_id, action.slot)
            })
            .ok()
            .map(|index| &self.operations[index])
    }

    pub(super) fn validate(&self) -> Result<(), ContractError> {
        if self.schema != 2
            || self.operations.windows(2).any(|rows| {
                (rows[0].npc_type_id, rows[0].slot) >= (rows[1].npc_type_id, rows[1].slot)
            })
            || self
                .operations
                .iter()
                .any(|action| action.npc_type_id < 0 || !(1..=5).contains(&action.slot))
            || self
                .builtin_gatherer_fishing_operations
                .windows(2)
                .any(|rows| {
                    (rows[0].npc_type_id, rows[0].slot) >= (rows[1].npc_type_id, rows[1].slot)
                })
            || self
                .builtin_gatherer_fishing_operations
                .iter()
                .any(|action| {
                    action.npc_type_id < 0
                        || matches!(action.npc_type_id, 390 | 403..=406)
                        || !(1..=5).contains(&action.slot)
                })
        {
            return Err(ContractError::InvalidContract);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Facts(#[serde(with = "EvidenceWire")] NpcTransformEvidence);

    fn decode(json: &str) -> NpcTransformEvidence {
        serde_json::from_str::<Facts>(json).unwrap().0
    }

    #[test]
    fn closure_and_builtin_fishing_rows_have_separate_admission_queries() {
        let facts = decode(
            r#"{"schema":2,"operations":[{"npc_type_id":7,"slot":2}],"builtin_gatherer_fishing_operations":[{"npc_type_id":8,"slot":1}]}"#,
        );
        assert_eq!(facts.validate(), Ok(()));
        let action = facts.operation(7, 2).unwrap();
        assert_eq!(action.npc_type_id(), 7);
        assert_eq!(action.slot(), 2);
        assert!(facts.operation(7, 1).is_none());
        assert!(facts.operation(8, 2).is_none());
        let fishing = facts.builtin_gatherer_fishing_operation(8, 1).unwrap();
        assert_eq!(fishing.npc_type_id(), 8);
        assert_eq!(fishing.slot(), 1);
        assert!(facts.builtin_gatherer_fishing_operation(7, 2).is_none());
    }

    #[test]
    fn invalid_runtime_operation_rows_refuse_binding() {
        for rows in [
            r#"[{"npc_type_id":-1,"slot":1}]"#.to_owned(),
            r#"[{"npc_type_id":7,"slot":2},{"npc_type_id":7,"slot":2}]"#.to_owned(),
        ] {
            let facts = decode(&format!(
                r#"{{"schema":2,"operations":{rows},"builtin_gatherer_fishing_operations":[]}}"#
            ));
            assert_eq!(facts.validate(), Err(ContractError::InvalidContract));
        }
    }

    #[test]
    fn static_fishing_rows_cannot_replace_full_closure_or_admit_event_npcs() {
        for npc_type_id in [390, 403, 404, 405, 406] {
            let facts = decode(&format!(
                r#"{{"schema":2,"operations":[],"builtin_gatherer_fishing_operations":[{{"npc_type_id":{npc_type_id},"slot":1}}]}}"#
            ));
            assert_eq!(facts.validate(), Err(ContractError::InvalidContract));
        }

        let facts = decode(
            r#"{"schema":2,"operations":[],"builtin_gatherer_fishing_operations":[{"npc_type_id":309,"slot":1}]}"#,
        );
        assert_eq!(facts.validate(), Ok(()));
        assert!(facts.operation(309, 1).is_none());
        assert!(facts.builtin_gatherer_fishing_operation(309, 1).is_some());
    }

    #[test]
    fn old_schema_or_unknown_fields_refuse_binding() {
        let legacy = r#"{"schema":1,"operations":[],"builtin_gatherer_fishing_operations":[]}"#;
        assert_eq!(
            decode(legacy).validate(),
            Err(ContractError::InvalidContract)
        );

        let unexpected = r#"{"schema":2,"operations":[],"builtin_gatherer_fishing_operations":[],"unrelated":[]}"#;
        assert!(serde_json::from_str::<Facts>(unexpected).is_err());
    }
}
