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
    operation_fingerprint: String,
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

    /// Borrow the ordered OP/AP handler fingerprint, not a display label.
    pub fn operation_fingerprint(&self) -> &str {
        &self.operation_fingerprint
    }
}

#[derive(Deserialize)]
#[serde(remote = "NpcTransformAction", deny_unknown_fields)]
struct ActionWire {
    npc_type_id: i32,
    slot: u8,
    operation_fingerprint: String,
}

impl super::wire::DecodeFact for NpcTransformAction {
    fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ActionWire::deserialize(deserializer)
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
    evidence_sha256: String,
    operations: Vec<NpcTransformAction>,
}

#[derive(Deserialize)]
#[serde(remote = "NpcTransformEvidence", deny_unknown_fields)]
pub(super) struct EvidenceWire {
    schema: u16,
    evidence_sha256: String,
    #[serde(deserialize_with = "super::wire::decode_list")]
    operations: Vec<NpcTransformAction>,
}

impl NpcTransformEvidence {
    /// Borrow the digest of the development-only complete evidence artifact.
    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }

    /// Borrow only the operations with complete positive closure proofs.
    pub fn operations(&self) -> &[NpcTransformAction] {
        &self.operations
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
        if self.schema != 1
            || !is_digest(&self.evidence_sha256)
            || self.operations.windows(2).any(|rows| {
                (rows[0].npc_type_id, rows[0].slot) >= (rows[1].npc_type_id, rows[1].slot)
            })
            || self.operations.iter().any(|action| {
                action.npc_type_id < 0
                    || !(1..=5).contains(&action.slot)
                    || !is_digest(&action.operation_fingerprint)
            })
        {
            return Err(ContractError::InvalidContract);
        }
        Ok(())
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: &str) -> NpcTransformEvidence {
        #[derive(Deserialize)]
        struct Facts(#[serde(with = "EvidenceWire")] NpcTransformEvidence);
        serde_json::from_str::<Facts>(json).unwrap().0
    }

    #[test]
    fn positive_rows_are_the_only_runtime_admission_authority() {
        let facts = decode(&format!(
            r#"{{"schema":1,"evidence_sha256":"{}","operations":[{{"npc_type_id":7,"slot":2,"operation_fingerprint":"{}"}}]}}"#,
            "a".repeat(64),
            "b".repeat(64)
        ));
        assert_eq!(facts.validate(), Ok(()));
        let action = facts.operation(7, 2).unwrap();
        assert_eq!(action.npc_type_id(), 7);
        assert_eq!(action.slot(), 2);
        assert_eq!(action.operation_fingerprint(), "b".repeat(64));
        assert!(facts.operation(7, 1).is_none());
        assert!(facts.operation(8, 2).is_none());
    }

    #[test]
    fn invalid_runtime_digests_or_operation_rows_refuse_binding() {
        for (digest, rows) in [
            ("not-a-digest".to_owned(), "[]".to_owned()),
            (
                "a".repeat(64),
                r#"[{"npc_type_id":-1,"slot":1,"operation_fingerprint":"bad"}]"#.to_owned(),
            ),
            (
                "a".repeat(64),
                format!(
                    r#"[{{"npc_type_id":7,"slot":2,"operation_fingerprint":"{0}"}},{{"npc_type_id":7,"slot":2,"operation_fingerprint":"{0}"}}]"#,
                    "b".repeat(64)
                ),
            ),
        ] {
            let facts = decode(&format!(
                r#"{{"schema":1,"evidence_sha256":"{digest}","operations":{rows}}}"#
            ));
            assert_eq!(facts.validate(), Err(ContractError::InvalidContract));
        }
    }
}
