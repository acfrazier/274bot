//! Selected content contracts for server-ordered input.
//!
//! These immutable facts authorize no input on their own. Ordinary checked
//! interactions do not use this catalog. Generation joins content aliases to
//! the selected cache and proves the synchronous paths before publication.

use serde::Deserialize;
use thiserror::Error;

use crate::selected::{ClientRevision, SelectedPin, SourceSpan};

mod npc_transforms;
pub use npc_transforms::*;
mod input;
mod request;

pub use input::*;
pub use request::*;

/// A selected contract cannot authorize input when its proof is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ContractError {
    /// The revision, catalog, or bound selected identity has no reviewed proof.
    #[error("server-ordered content contract is unavailable")]
    ContractUnavailable,
    /// The catalog and the caller do not select the same immutable inputs.
    #[error("server-ordered content contract pin does not match")]
    PinMismatch,
    /// A required symbolic component alias is absent.
    #[error("server-ordered content contract is missing a component alias")]
    MissingAlias,
    /// Two selected aliases or components claim the same identity.
    #[error("server-ordered content contract has ambiguous component identities")]
    AmbiguousAlias,
    /// A component does not descend from its selected interface root.
    #[error("server-ordered component has the wrong root ancestry")]
    WrongRoot,
    /// The selected component no longer has the proved control properties.
    #[error("server-ordered component properties do not match the proved control")]
    ComponentChanged,
    /// The banker successor is not one proved, synchronous player page.
    #[error("banker successor does not have a selected-font single-page proof")]
    BankerPageUnproved,
    /// The catalog contains an incomplete or inconsistent content recipe.
    #[error("server-ordered content contract is incomplete or inconsistent")]
    InvalidContract,
}

/// The closed set of reviewed native recipes. This is not a packet API.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractKey {
    /// Knife use, the selected Make button, and an optional resource operation.
    KnifeArm,
    /// Current bank row operation 5 followed by its positive count.
    BankWithdrawX,
    /// Current held row operation 5 followed by its positive count.
    BankDepositX,
    /// Current held row operation 4 in an existing bank.
    BankDepositAll,
    /// Received banker choice, its synchronous player page, and side deposit.
    BankOpenDeposit,
    /// The bank owner's terminal close, including its non-modal effects.
    BankCloseTerminal,
    /// The selected bankbooth numeric operation.
    NumericBankbooth,
    /// Appearance submission followed by the selected acceptance button.
    AcceptCharacterDesign,
}

/// A decoded interface component kind from the selected cache.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentType {
    /// Interface layer with children.
    Layer,
    /// The unused component kind.
    Unused,
    /// Item inventory grid.
    Inventory,
    /// Coloured rectangle.
    Rect,
    /// Text label or control.
    Text,
    /// Sprite image.
    Sprite,
    /// Animated model.
    Model,
    /// Text inventory grid.
    InventoryText,
}

/// A decoded client button kind. Pause controls never select a choice.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ButtonType {
    /// Not a button.
    None,
    /// Ordinary IF_BUTTON control.
    Button,
    /// Target-selection control.
    Target,
    /// Client close control.
    Close,
    /// Toggle control.
    Toggle,
    /// Selection control.
    Select,
    /// RESUME_PAUSEBUTTON continuation.
    Pause,
}

/// Kind-specific properties borrowed from the verified selected catalog.
/// External code cannot construct, deserialize, or clone these properties;
/// only the catalog's private wire decoder builds them.
#[derive(Debug)]
pub struct ComponentProperties {
    inner: ComponentPropertyValues,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ComponentPropertyValues {
    /// Layer layout and child identities.
    Layer {
        /// Scroll extent.
        scroll: i32,
        /// Initial visibility flag.
        hidden: bool,
        /// Selected child component identities.
        child_ids: Vec<i32>,
    },
    /// Item grid flags and operation labels.
    Inventory {
        /// Whether items can be dragged.
        draggable: bool,
        /// Whether item operations are enabled.
        operable: bool,
        /// Whether items can be selected for use.
        usable: bool,
        /// Whether slots can be swapped.
        swappable: bool,
        /// Horizontal spacing between inventory slots.
        margin_x: i32,
        /// Vertical spacing between inventory slots.
        margin_y: i32,
        /// Selected operation labels, preserving absent slots.
        actions: Vec<Option<String>>,
        /// Decoded target mask.
        action_target: i32,
    },
    /// Rectangle drawing mode.
    Rect {
        /// Whether the rectangle is filled.
        fill: bool,
    },
    /// Text and selected font properties.
    Text {
        /// Centred text flag.
        centered: bool,
        /// Selected font identity.
        font_id: i32,
        /// Text shadow flag.
        shadowed: bool,
        /// Initial text.
        text: String,
        /// Active-state text.
        active_text: String,
    },
    /// Selected sprite names.
    Sprite {
        /// Inactive graphic.
        graphic: String,
        /// Active graphic.
        active_graphic: String,
    },
    /// Selected model and animation properties.
    Model {
        /// Inactive model identity.
        model: i32,
        /// Active model identity.
        active_model: i32,
        /// Inactive animation identity.
        anim: i32,
        /// Active animation identity.
        active_anim: i32,
        /// Model zoom.
        zoom: i32,
        /// X angle.
        xan: i32,
        /// Y angle.
        yan: i32,
    },
    /// Text inventory layout and operation labels.
    InventoryText {
        /// Centred text flag.
        centered: bool,
        /// Selected font identity.
        font_id: i32,
        /// Text shadow flag.
        shadowed: bool,
        /// Text colour.
        colour: i32,
        /// Horizontal spacing.
        margin_x: i32,
        /// Vertical spacing.
        margin_y: i32,
        /// Whether operations are enabled.
        operable: bool,
        /// Selected operation labels.
        actions: Vec<Option<String>>,
    },
    /// The unused component has no extra properties.
    Unused {},
}

/// One symbolic component joined to its decoded selected-cache properties.
/// ```compile_fail
/// let _: api::interact::server_ordered::ContractComponent =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct ContractComponent {
    alias: String,
    id: i32,
    root_alias: String,
    root_id: i32,
    parent_id: Option<i32>,
    ancestor_ids: Vec<i32>,
    component_type: ComponentType,
    button_type: ButtonType,
    client_code: i32,
    width: i32,
    height: i32,
    properties: ComponentProperties,
    source: Vec<SourceSpan>,
}

impl ContractComponent {
    /// Return the exact selected alias.
    pub fn alias(&self) -> &str {
        &self.alias
    }
    /// Return the selected numeric identity.
    pub fn id(&self) -> i32 {
        self.id
    }
    /// Return the exact selected root alias.
    pub fn root_alias(&self) -> &str {
        &self.root_alias
    }
    /// Return the selected root identity.
    pub fn root_id(&self) -> i32 {
        self.root_id
    }
    /// Return the immediate decoded parent.
    pub fn parent_id(&self) -> Option<i32> {
        self.parent_id
    }
    /// Borrow the decoded root ancestry.
    pub fn ancestor_ids(&self) -> &[i32] {
        &self.ancestor_ids
    }
    /// Return the decoded component kind.
    pub fn component_type(&self) -> ComponentType {
        self.component_type
    }
    /// Return the decoded button kind.
    pub fn button_type(&self) -> ButtonType {
        self.button_type
    }
    /// Return the client-side action code.
    pub fn client_code(&self) -> i32 {
        self.client_code
    }
    /// Return the selected layout width.
    pub fn width(&self) -> i32 {
        self.width
    }
    /// Return the selected layout height.
    pub fn height(&self) -> i32 {
        self.height
    }
    /// Borrow the kind-specific decoded properties.
    pub fn properties(&self) -> &ComponentProperties {
        &self.properties
    }
    /// Borrow the content and decoder citations.
    pub fn source(&self) -> &[SourceSpan] {
        &self.source
    }
}

/// The authority of a recipe declaration.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum ContractBasis {
    /// The generator checks the selected source before publishing the declaration.
    #[serde(rename = "checked")]
    Checked,
    /// Reviewed host policy. Any engine assumption in it remains unproved.
    #[serde(rename = "host-policy")]
    HostPolicy,
}

/// One named recipe condition with its explicit authority.
/// ```compile_fail
/// let _: api::interact::server_ordered::ContractCondition =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct ContractCondition {
    name: String,
    basis: ContractBasis,
    sources: Vec<SourceSpan>,
}

impl ContractCondition {
    /// Borrow the condition name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Return the authority. Only `Checked` declares a checked source fact.
    pub fn basis(&self) -> ContractBasis {
        self.basis
    }
    /// Borrow the selected-source citations for the checked condition.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
}

/// A dependency between phases of one native content request.
///
/// Only checked edges can declare a synchronous server path. Other edges specify host send order only.
/// ```compile_fail
/// let _: api::interact::server_ordered::ContractPredecessor =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct ContractPredecessor {
    from: String,
    to: String,
    basis: ContractBasis,
    synchronous: bool,
    sources: Vec<SourceSpan>,
}

impl ContractPredecessor {
    /// Return the named predecessor phase.
    pub fn from(&self) -> &str {
        &self.from
    }
    /// Return the exact dependent phase.
    pub fn to(&self) -> &str {
        &self.to
    }
    /// Return whether selected-source checks prove a synchronous server path.
    /// This never describes host send order.
    pub fn synchronous(&self) -> bool {
        self.synchronous
    }
    /// Return whether this is a checked server edge or unproved host send order.
    pub fn basis(&self) -> ContractBasis {
        self.basis
    }
    /// Borrow the checked server-path citations. Host-order edges have none.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
}

/// One generated native recipe and its reviewed outcome policy.
///
/// These facts can only be borrowed from a verified catalog.
/// ```compile_fail
/// let _: api::interact::server_ordered::ContractEntry =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct ContractEntry {
    key: ContractKey,
    components: Vec<String>,
    server_predicates: Vec<ContractCondition>,
    predecessors: Vec<ContractPredecessor>,
    max_packet_cost: u8,
    outcome: Vec<ContractCondition>,
    deadline_ticks: Option<u8>,
    recovery_count: u8,
    cleanup_effects: Vec<ContractCondition>,
    sources: Vec<SourceSpan>,
}

impl ContractEntry {
    /// Return the closed native recipe identity.
    pub fn key(&self) -> ContractKey {
        self.key
    }
    /// Borrow the exact symbolic component aliases.
    pub fn components(&self) -> &[String] {
        &self.components
    }
    /// Borrow the required predicates and their authority.
    /// Host-policy predicates are unproved assumptions, not selected engine facts.
    pub fn server_predicates(&self) -> &[ContractCondition] {
        &self.server_predicates
    }
    /// Borrow the predecessor relations and their authority.
    pub fn predecessors(&self) -> &[ContractPredecessor] {
        &self.predecessors
    }
    /// Return the full reviewed maximum packet cost.
    pub fn max_packet_cost(&self) -> u8 {
        self.max_packet_cost
    }
    /// Borrow the positive outcome signals.
    pub fn outcome(&self) -> &[ContractCondition] {
        &self.outcome
    }
    /// Return the observation allowance in received ticks.
    pub fn deadline_ticks(&self) -> Option<u8> {
        self.deadline_ticks
    }
    /// Return the maximum named recovery attempts.
    pub fn recovery_count(&self) -> u8 {
        self.recovery_count
    }
    /// Borrow the named cleanup requirements and their authority.
    pub fn cleanup_effects(&self) -> &[ContractCondition] {
        &self.cleanup_effects
    }
    /// Borrow the selected source citations.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
}

/// Selected-font proof for the player page after the received banker choice.
/// ```compile_fail
/// let _: api::interact::server_ordered::BankerPageProof =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct BankerPageProof {
    text: String,
    font_alias: String,
    font_id: i32,
    mesanim_alias: String,
    mesanim_id: i32,
    line_width: i32,
    max_lines: u16,
    line_count: u16,
    page_count: u16,
    split_lines: Vec<String>,
    page_root_alias: String,
    continue_component_alias: String,
    sources: Vec<SourceSpan>,
}

impl BankerPageProof {
    /// Borrow the exact selected content text.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Return the selected font alias.
    pub fn font_alias(&self) -> &str {
        &self.font_alias
    }
    /// Return the selected font identity.
    pub fn font_id(&self) -> i32 {
        self.font_id
    }
    /// Return the selected message-animation alias.
    pub fn mesanim_alias(&self) -> &str {
        &self.mesanim_alias
    }
    /// Return the selected message-animation identity.
    pub fn mesanim_id(&self) -> i32 {
        self.mesanim_id
    }
    /// Return the wrapping width in pixels.
    pub fn line_width(&self) -> i32 {
        self.line_width
    }
    /// Return the maximum lines per selected page.
    pub fn max_lines(&self) -> u16 {
        self.max_lines
    }
    /// Return the selected-font line count.
    pub fn line_count(&self) -> u16 {
        self.line_count
    }
    /// Return the proved page count.
    pub fn page_count(&self) -> u16 {
        self.page_count
    }
    /// Borrow the selected-font wrapped lines.
    pub fn split_lines(&self) -> &[String] {
        &self.split_lines
    }
    /// Return the exact selected player-page root.
    pub fn page_root_alias(&self) -> &str {
        &self.page_root_alias
    }
    /// Return the exact selected continuation alias.
    pub fn continue_component_alias(&self) -> &str {
        &self.continue_component_alias
    }
    /// Borrow the source citations.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
}

/// The form explicitly expected by a bank withdrawal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankWithdrawMode {
    /// Withdraw the item itself.
    Item,
    /// Withdraw its certificate when content permits it.
    Note,
}

/// Bank mode and operation facts from selected content.
/// ```compile_fail
/// let _: api::interact::server_ordered::BankContract =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct BankContract {
    mode_varp: ContractIdentity,
    item_mode: i32,
    note_mode: i32,
    main_component: ContractIdentity,
    side_component: ContractIdentity,
    withdraw_x_op: u8,
    deposit_x_op: u8,
    deposit_all_op: u8,
    sources: Vec<SourceSpan>,
}

impl BankContract {
    /// Borrow the selected bank mode varp identity.
    pub fn mode_varp(&self) -> &ContractIdentity {
        &self.mode_varp
    }
    /// Return the selected item-mode value.
    pub fn item_mode(&self) -> i32 {
        self.item_mode
    }
    /// Return the selected note-mode value.
    pub fn note_mode(&self) -> i32 {
        self.note_mode
    }
    /// Borrow the main bank inventory identity.
    pub fn main_component(&self) -> &ContractIdentity {
        &self.main_component
    }
    /// Borrow the side bank inventory identity.
    pub fn side_component(&self) -> &ContractIdentity {
        &self.side_component
    }
    /// Return the bank-row count operation.
    pub fn withdraw_x_op(&self) -> u8 {
        self.withdraw_x_op
    }
    /// Return the held-row count operation.
    pub fn deposit_x_op(&self) -> u8 {
        self.deposit_x_op
    }
    /// Return the held-row deposit-all operation.
    pub fn deposit_all_op(&self) -> u8 {
        self.deposit_all_op
    }
    /// Borrow the mode and operation citations.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
    /// Decode a posted mode without an optimistic default for absent or unknown values.
    pub fn observed_mode(&self, observed: Option<i32>) -> Option<BankWithdrawMode> {
        match observed {
            Some(value) if value == self.item_mode => Some(BankWithdrawMode::Item),
            Some(value) if value == self.note_mode => Some(BankWithdrawMode::Note),
            _ => None,
        }
    }
}

/// A selected content alias joined to its numeric identity.
/// ```compile_fail
/// let _: api::interact::server_ordered::ContractIdentity =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct ContractIdentity {
    alias: String,
    id: i32,
}

impl ContractIdentity {
    /// Return the exact selected alias.
    pub fn alias(&self) -> &str {
        &self.alias
    }
    /// Return the selected numeric identity.
    pub fn id(&self) -> i32 {
        self.id
    }
}

/// Read-only selected knife and normal-log identities for the KnifeArm request.
#[derive(Debug)]
pub struct KnifeContract {
    knife: ContractIdentity,
    logs: Vec<ContractIdentity>,
    sources: Vec<SourceSpan>,
}

impl KnifeContract {
    /// Borrow the exact selected knife identity.
    pub fn knife(&self) -> &ContractIdentity {
        &self.knife
    }

    /// Borrow the selected normal-log identities accepted by this recipe.
    pub fn logs(&self) -> &[ContractIdentity] {
        &self.logs
    }

    /// Borrow the selected item-role source citations.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
}

/// A numeric location operation admitted by selected content.
/// ```compile_fail
/// let _: api::interact::server_ordered::NumericLocContract =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct NumericLocContract {
    alias: String,
    id: i32,
    operation: u8,
    action: String,
    sources: Vec<SourceSpan>,
}

impl NumericLocContract {
    /// Return the exact selected location alias.
    pub fn alias(&self) -> &str {
        &self.alias
    }
    /// Return the selected numeric location identity.
    pub fn id(&self) -> i32 {
        self.id
    }
    /// Return the proved one-based operation.
    pub fn operation(&self) -> u8 {
        self.operation
    }
    /// Return the intended content action.
    pub fn action(&self) -> &str {
        &self.action
    }
    /// Borrow the operation citations.
    pub fn sources(&self) -> &[SourceSpan] {
        &self.sources
    }
}

// The public facts deliberately have no Deserialize implementation. Only these
// private remote definitions can decode them at the selected-data boundary.
mod wire {
    use super::*;
    use serde::de::{DeserializeSeed, SeqAccess, Visitor};
    use std::marker::PhantomData;

    pub(super) trait DecodeFact: Sized {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
    }

    struct FactSeed<T>(PhantomData<T>);

    impl<'de, T: DecodeFact> DeserializeSeed<'de> for FactSeed<T> {
        type Value = T;

        fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
            T::decode(deserializer)
        }
    }

    pub(super) fn decode_list<'de, D: serde::Deserializer<'de>, T: DecodeFact>(
        deserializer: D,
    ) -> Result<Vec<T>, D::Error> {
        struct Facts<T>(PhantomData<T>);
        impl<'de, T: DecodeFact> Visitor<'de> for Facts<T> {
            type Value = Vec<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("selected catalog facts")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<T>, A::Error> {
                let mut facts = Vec::new();
                while let Some(fact) = sequence.next_element_seed(FactSeed(PhantomData))? {
                    facts.push(fact);
                }
                Ok(facts)
            }
        }
        deserializer.deserialize_seq(Facts(PhantomData))
    }

    #[derive(Deserialize)]
    #[serde(remote = "ComponentProperties", transparent)]
    struct PropertiesWire {
        inner: ComponentPropertyValues,
    }

    #[derive(Deserialize)]
    #[serde(remote = "ContractCondition", deny_unknown_fields)]
    struct ConditionWire {
        name: String,
        basis: ContractBasis,
        sources: Vec<SourceSpan>,
    }

    impl DecodeFact for ContractCondition {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            ConditionWire::deserialize(deserializer)
        }
    }

    #[derive(Deserialize)]
    #[serde(remote = "ContractComponent", deny_unknown_fields)]
    struct ComponentWire {
        alias: String,
        id: i32,
        root_alias: String,
        root_id: i32,
        parent_id: Option<i32>,
        ancestor_ids: Vec<i32>,
        component_type: ComponentType,
        button_type: ButtonType,
        client_code: i32,
        width: i32,
        height: i32,
        #[serde(deserialize_with = "PropertiesWire::deserialize")]
        properties: ComponentProperties,
        source: Vec<SourceSpan>,
    }

    impl DecodeFact for ContractComponent {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            ComponentWire::deserialize(deserializer)
        }
    }

    #[derive(Deserialize)]
    #[serde(remote = "ContractPredecessor", deny_unknown_fields)]
    struct PredecessorWire {
        from: String,
        to: String,
        basis: ContractBasis,
        synchronous: bool,
        sources: Vec<SourceSpan>,
    }

    impl DecodeFact for ContractPredecessor {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            PredecessorWire::deserialize(deserializer)
        }
    }

    #[derive(Deserialize)]
    #[serde(remote = "ContractEntry", deny_unknown_fields)]
    struct EntryWire {
        key: ContractKey,
        components: Vec<String>,
        #[serde(deserialize_with = "decode_list")]
        server_predicates: Vec<ContractCondition>,
        #[serde(deserialize_with = "decode_list")]
        predecessors: Vec<ContractPredecessor>,
        max_packet_cost: u8,
        #[serde(deserialize_with = "decode_list")]
        outcome: Vec<ContractCondition>,
        deadline_ticks: Option<u8>,
        recovery_count: u8,
        #[serde(deserialize_with = "decode_list")]
        cleanup_effects: Vec<ContractCondition>,
        sources: Vec<SourceSpan>,
    }

    impl DecodeFact for ContractEntry {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            EntryWire::deserialize(deserializer)
        }
    }

    #[derive(Deserialize)]
    #[serde(remote = "BankerPageProof", deny_unknown_fields)]
    struct PageWire {
        text: String,
        font_alias: String,
        font_id: i32,
        mesanim_alias: String,
        mesanim_id: i32,
        line_width: i32,
        max_lines: u16,
        line_count: u16,
        page_count: u16,
        split_lines: Vec<String>,
        page_root_alias: String,
        continue_component_alias: String,
        sources: Vec<SourceSpan>,
    }

    pub(super) fn decode_page<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<BankerPageProof>, D::Error> {
        #[derive(Deserialize)]
        struct Page(#[serde(with = "PageWire")] BankerPageProof);
        Ok(Option::<Page>::deserialize(deserializer)?.map(|page| page.0))
    }

    #[derive(Deserialize)]
    #[serde(remote = "ContractIdentity", deny_unknown_fields)]
    struct IdentityWire {
        alias: String,
        id: i32,
    }

    impl DecodeFact for ContractIdentity {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            IdentityWire::deserialize(deserializer)
        }
    }

    #[derive(Deserialize)]
    #[serde(remote = "KnifeContract", deny_unknown_fields)]
    pub(super) struct KnifeWire {
        #[serde(with = "IdentityWire")]
        knife: ContractIdentity,
        #[serde(deserialize_with = "decode_list")]
        logs: Vec<ContractIdentity>,
        sources: Vec<SourceSpan>,
    }

    #[derive(Deserialize)]
    #[serde(remote = "BankContract", deny_unknown_fields)]
    pub(super) struct BankWire {
        #[serde(with = "IdentityWire")]
        mode_varp: ContractIdentity,
        item_mode: i32,
        note_mode: i32,
        #[serde(with = "IdentityWire")]
        main_component: ContractIdentity,
        #[serde(with = "IdentityWire")]
        side_component: ContractIdentity,
        withdraw_x_op: u8,
        deposit_x_op: u8,
        deposit_all_op: u8,
        sources: Vec<SourceSpan>,
    }

    #[derive(Deserialize)]
    #[serde(remote = "NumericLocContract", deny_unknown_fields)]
    struct LocWire {
        alias: String,
        id: i32,
        operation: u8,
        action: String,
        sources: Vec<SourceSpan>,
    }

    impl DecodeFact for NumericLocContract {
        fn decode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            LocWire::deserialize(deserializer)
        }
    }
}

/// Untrusted generated representation. Only the selected-data loader can bind it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogWire {
    schema: u16,
    revision: i32,
    engine_commit: String,
    content_commit: String,
    cache_id: String,
    content_id: String,
    #[serde(deserialize_with = "wire::decode_list")]
    components: Vec<ContractComponent>,
    #[serde(deserialize_with = "wire::decode_list")]
    entries: Vec<ContractEntry>,
    #[serde(deserialize_with = "wire::decode_page")]
    banker_page: Option<BankerPageProof>,
    #[serde(with = "wire::BankWire")]
    bank: BankContract,
    #[serde(with = "wire::KnifeWire")]
    knife: KnifeContract,
    #[serde(deserialize_with = "wire::decode_list")]
    numeric_locs: Vec<NumericLocContract>,
    #[serde(with = "npc_transforms::EvidenceWire")]
    npc_transforms: NpcTransformEvidence,
}

/// Process-shared, pin-verified facts. This type has no public constructor or decoder.
///
/// ```compile_fail
/// let _: api::interact::server_ordered::ServerOrderedCatalog =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug)]
pub struct ServerOrderedCatalog {
    facts: CatalogWire,
}

impl ServerOrderedCatalog {
    pub(crate) fn bind(mut facts: CatalogWire, pin: &SelectedPin) -> Result<Self, ContractError> {
        if facts.banker_page().is_err() {
            facts.banker_page = None;
            facts
                .entries
                .retain(|entry| entry.key != ContractKey::BankOpenDeposit);
        }
        facts.validate(pin)?;
        Ok(Self { facts })
    }

    /// Return the selected revision proved by this catalog.
    pub fn revision(&self) -> i32 {
        self.facts.revision()
    }

    /// Return the exact selected alias without inventing a fallback identity.
    pub fn component(&self, alias: &str) -> Result<&ContractComponent, ContractError> {
        self.facts.component(alias)
    }

    /// Borrow all decoded components without constructing a per-bot table.
    pub fn components(&self) -> &[ContractComponent] {
        self.facts.components()
    }

    /// Return a reviewed native recipe or refuse the unavailable contract.
    pub fn entry(&self, key: ContractKey) -> Result<&ContractEntry, ContractError> {
        self.facts.entry(key)
    }

    /// Borrow the complete closed recipe set.
    pub fn entries(&self) -> &[ContractEntry] {
        self.facts.entries()
    }

    /// Borrow the selected-font banker successor proof.
    pub fn banker_page(&self) -> Result<&BankerPageProof, ContractError> {
        self.facts.banker_page()
    }

    /// Borrow exact mode and operation facts for bank preflight.
    pub fn bank(&self) -> &BankContract {
        self.facts.bank()
    }

    /// Borrow selected knife and normal-log role identities.
    pub fn knife(&self) -> &KnifeContract {
        self.facts.knife()
    }

    /// Borrow the selected location numeric-operation allowlist.
    pub fn numeric_locs(&self) -> &[NumericLocContract] {
        self.facts.numeric_locs()
    }

    /// Borrow the positive NPC admission rows and full-evidence digest.
    pub fn npc_transforms(&self) -> &NpcTransformEvidence {
        &self.facts.npc_transforms
    }

    /// Return an NPC numeric operation only when every reachable type has the same proved action.
    pub fn npc_operation(&self, npc_type_id: i32, slot: u8) -> Option<&NpcTransformAction> {
        self.facts.npc_transforms.operation(npc_type_id, slot)
    }
}

impl CatalogWire {
    /// Return the selected revision proved by this catalog.
    pub fn revision(&self) -> i32 {
        self.revision
    }

    /// Return the exact selected alias without inventing a fallback identity.
    pub fn component(&self, alias: &str) -> Result<&ContractComponent, ContractError> {
        self.components
            .iter()
            .find(|component| component.alias == alias)
            .ok_or(ContractError::MissingAlias)
    }

    /// Borrow all decoded components. No per-bot table is constructed.
    pub fn components(&self) -> &[ContractComponent] {
        &self.components
    }

    /// Return a reviewed native recipe or refuse the unavailable contract.
    pub fn entry(&self, key: ContractKey) -> Result<&ContractEntry, ContractError> {
        if key == ContractKey::BankOpenDeposit {
            self.banker_page()?;
        }
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .ok_or(ContractError::ContractUnavailable)
    }

    /// Borrow the complete closed recipe set.
    pub fn entries(&self) -> &[ContractEntry] {
        &self.entries
    }

    /// Borrow the selected-font banker successor proof.
    pub fn banker_page(&self) -> Result<&BankerPageProof, ContractError> {
        let proof = self
            .banker_page
            .as_ref()
            .ok_or(ContractError::BankerPageUnproved)?;
        if proof.page_count != 1
            || proof.line_count == 0
            || proof.line_count > proof.max_lines
            || usize::from(proof.line_count) != proof.split_lines.len()
            || proof.line_width <= 0
            || proof.text.is_empty()
            || proof.sources.is_empty()
        {
            return Err(ContractError::BankerPageUnproved);
        }
        let continuation = self
            .component(&proof.continue_component_alias)
            .map_err(|_| ContractError::BankerPageUnproved)?;
        if continuation.root_alias != proof.page_root_alias
            || continuation.component_type != ComponentType::Text
            || continuation.button_type != ButtonType::Pause
        {
            return Err(ContractError::BankerPageUnproved);
        }
        Ok(proof)
    }

    /// Borrow exact mode and operation facts for bank preflight.
    pub fn bank(&self) -> &BankContract {
        &self.bank
    }

    /// Borrow selected knife and normal-log role identities.
    pub fn knife(&self) -> &KnifeContract {
        &self.knife
    }

    /// Borrow the selected location numeric-operation allowlist.
    pub fn numeric_locs(&self) -> &[NumericLocContract] {
        &self.numeric_locs
    }

    pub(crate) fn validate(&self, pin: &SelectedPin) -> Result<(), ContractError> {
        if self.schema != 1 || pin.revision != ClientRevision::R289 || self.revision != 289 {
            return Err(ContractError::ContractUnavailable);
        }
        if self.engine_commit != pin.engine_commit.as_ref()
            || self.content_commit != pin.content_commit.as_ref()
            || self.cache_id != pin.cache_id.as_ref()
            || self.content_id != pin.content_id.as_ref()
        {
            return Err(ContractError::PinMismatch);
        }
        for (index, component) in self.components.iter().enumerate() {
            if component.alias.is_empty() || component.id <= 0 {
                return Err(ContractError::MissingAlias);
            }
            if self.components[..index]
                .iter()
                .any(|other| other.alias == component.alias || other.id == component.id)
            {
                return Err(ContractError::AmbiguousAlias);
            }
            let root = self.component(&component.root_alias)?;
            if root.id != component.root_id
                || root.id != root.root_id
                || root.component_type != ComponentType::Layer
                || (component.id != root.id
                    && (component.parent_id.is_none()
                        || !component.ancestor_ids.contains(&root.id)))
            {
                return Err(ContractError::WrongRoot);
            }
            // Content: player_kit.if:733-738 and bank_main.if:744-749.
            // Accept submits IDK_SAVEDESIGN before IF_BUTTON; the bank inventory
            // carries the selected client's bank content code.
            let expected_code = match component.alias.as_str() {
                "player_kit:accept" => 326,
                "bank_main:inv" => 206,
                _ => 0,
            };
            if component.client_code != expected_code
                || !component.properties.matches(component.component_type)
            {
                return Err(ContractError::ComponentChanged);
            }
        }
        for (alias, kind, button) in [
            (
                "skill_multi3:make1_a",
                ComponentType::Text,
                ButtonType::Button,
            ),
            ("player_kit:accept", ComponentType::Rect, ButtonType::Button),
            ("multi2:com_1", ComponentType::Text, ButtonType::Button),
            ("bank_main:inv", ComponentType::Inventory, ButtonType::None),
            ("bank_side:inv", ComponentType::Inventory, ButtonType::None),
        ] {
            let component = self.component(alias)?;
            if component.component_type != kind || component.button_type != button {
                return Err(ContractError::ComponentChanged);
            }
        }
        for (index, entry) in self.entries.iter().enumerate() {
            if self.entries[..index]
                .iter()
                .any(|other| other.key == entry.key)
                || !(1..=5).contains(&entry.max_packet_cost)
                || entry.outcome.is_empty()
                || entry.server_predicates.is_empty()
                || entry.sources.is_empty()
            {
                return Err(ContractError::InvalidContract);
            }
            for condition in entry
                .server_predicates
                .iter()
                .chain(&entry.outcome)
                .chain(&entry.cleanup_effects)
            {
                if condition.name.is_empty()
                    || (condition.basis == ContractBasis::HostPolicy
                        && !condition.sources.is_empty())
                    || (condition.basis == ContractBasis::Checked && condition.sources.is_empty())
                {
                    return Err(ContractError::InvalidContract);
                }
            }
            for edge in &entry.predecessors {
                if edge.from.is_empty()
                    || edge.to.is_empty()
                    || (edge.basis == ContractBasis::HostPolicy
                        && (edge.synchronous || !edge.sources.is_empty()))
                    || (edge.basis == ContractBasis::Checked && edge.sources.is_empty())
                {
                    return Err(ContractError::InvalidContract);
                }
            }
            for alias in &entry.components {
                self.component(alias)?;
            }
        }
        for key in [
            ContractKey::KnifeArm,
            ContractKey::BankWithdrawX,
            ContractKey::BankDepositX,
            ContractKey::BankDepositAll,
            ContractKey::BankCloseTerminal,
            ContractKey::NumericBankbooth,
            ContractKey::AcceptCharacterDesign,
        ] {
            self.entry(key)?;
        }
        if self.banker_page().is_ok() {
            self.entry(ContractKey::BankOpenDeposit)?;
        }
        for identity in [&self.bank.main_component, &self.bank.side_component] {
            if self.component(&identity.alias)?.id != identity.id {
                return Err(ContractError::InvalidContract);
            }
        }
        if self.bank.mode_varp.id < 0
            || self.bank.mode_varp.alias.is_empty()
            || self.bank.item_mode == self.bank.note_mode
            || self.bank.withdraw_x_op != 5
            || self.bank.deposit_x_op != 5
            || self.bank.deposit_all_op != 4
            || self.bank.sources.is_empty()
            || self.numeric_locs.is_empty()
            || self.numeric_locs.iter().any(|loc| {
                loc.id < 0
                    || loc.operation != 2
                    || loc.action != "open_bank"
                    || loc.sources.is_empty()
            })
        {
            return Err(ContractError::InvalidContract);
        }
        if self.knife.knife.alias.is_empty()
            || self.knife.knife.id < 0
            || self.knife.sources.is_empty()
            || self.knife.logs.is_empty()
            || self.knife.logs.iter().enumerate().any(|(index, identity)| {
                identity.alias.is_empty()
                    || identity.id < 0
                    || identity.alias == self.knife.knife.alias
                    || identity.id == self.knife.knife.id
                    || self.knife.logs[..index]
                        .iter()
                        .any(|other| other.alias == identity.alias || other.id == identity.id)
            })
        {
            return Err(ContractError::InvalidContract);
        }
        self.npc_transforms.validate()?;
        Ok(())
    }
}

impl ComponentProperties {
    /// Return the layer scroll extent, when present.
    pub fn scroll(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Layer { scroll, .. } => Some(*scroll),
            _ => None,
        }
    }
    /// Return the layer visibility flag, when present.
    pub fn hidden(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Layer { hidden, .. } => Some(*hidden),
            _ => None,
        }
    }
    /// Borrow the layer child identities, when present.
    pub fn child_ids(&self) -> Option<&[i32]> {
        match &self.inner {
            ComponentPropertyValues::Layer { child_ids, .. } => Some(child_ids),
            _ => None,
        }
    }
    /// Return whether inventory items can be dragged.
    pub fn draggable(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Inventory { draggable, .. } => Some(*draggable),
            _ => None,
        }
    }
    /// Return whether inventory operations are enabled.
    pub fn operable(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Inventory { operable, .. }
            | ComponentPropertyValues::InventoryText { operable, .. } => Some(*operable),
            _ => None,
        }
    }
    /// Return whether inventory items can be selected for use.
    pub fn usable(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Inventory { usable, .. } => Some(*usable),
            _ => None,
        }
    }
    /// Return whether inventory slots can be swapped.
    pub fn swappable(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Inventory { swappable, .. } => Some(*swappable),
            _ => None,
        }
    }
    /// Return the horizontal inventory spacing, when present.
    pub fn margin_x(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Inventory { margin_x, .. }
            | ComponentPropertyValues::InventoryText { margin_x, .. } => Some(*margin_x),
            _ => None,
        }
    }
    /// Return the vertical inventory spacing, when present.
    pub fn margin_y(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Inventory { margin_y, .. }
            | ComponentPropertyValues::InventoryText { margin_y, .. } => Some(*margin_y),
            _ => None,
        }
    }
    /// Borrow the inventory operation labels, preserving absent slots.
    pub fn actions(&self) -> Option<&[Option<String>]> {
        match &self.inner {
            ComponentPropertyValues::Inventory { actions, .. }
            | ComponentPropertyValues::InventoryText { actions, .. } => Some(actions),
            _ => None,
        }
    }
    /// Return the inventory target mask, when present.
    pub fn action_target(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Inventory { action_target, .. } => Some(*action_target),
            _ => None,
        }
    }
    /// Return whether the rectangle is filled.
    pub fn fill(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Rect { fill } => Some(*fill),
            _ => None,
        }
    }
    /// Return the text centring flag, when present.
    pub fn centered(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Text { centered, .. }
            | ComponentPropertyValues::InventoryText { centered, .. } => Some(*centered),
            _ => None,
        }
    }
    /// Return the selected font identity, when present.
    pub fn font_id(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Text { font_id, .. }
            | ComponentPropertyValues::InventoryText { font_id, .. } => Some(*font_id),
            _ => None,
        }
    }
    /// Return the text shadow flag, when present.
    pub fn shadowed(&self) -> Option<bool> {
        match &self.inner {
            ComponentPropertyValues::Text { shadowed, .. }
            | ComponentPropertyValues::InventoryText { shadowed, .. } => Some(*shadowed),
            _ => None,
        }
    }
    /// Borrow the initial text, when present.
    pub fn text(&self) -> Option<&str> {
        match &self.inner {
            ComponentPropertyValues::Text { text, .. } => Some(text),
            _ => None,
        }
    }
    /// Borrow the active-state text, when present.
    pub fn active_text(&self) -> Option<&str> {
        match &self.inner {
            ComponentPropertyValues::Text { active_text, .. } => Some(active_text),
            _ => None,
        }
    }
    /// Return the text inventory colour, when present.
    pub fn colour(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::InventoryText { colour, .. } => Some(*colour),
            _ => None,
        }
    }
    /// Borrow the inactive sprite name, when present.
    pub fn graphic(&self) -> Option<&str> {
        match &self.inner {
            ComponentPropertyValues::Sprite { graphic, .. } => Some(graphic),
            _ => None,
        }
    }
    /// Borrow the active sprite name, when present.
    pub fn active_graphic(&self) -> Option<&str> {
        match &self.inner {
            ComponentPropertyValues::Sprite { active_graphic, .. } => Some(active_graphic),
            _ => None,
        }
    }
    /// Return the inactive model identity, when present.
    pub fn model(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { model, .. } => Some(*model),
            _ => None,
        }
    }
    /// Return the active model identity, when present.
    pub fn active_model(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { active_model, .. } => Some(*active_model),
            _ => None,
        }
    }
    /// Return the inactive animation identity, when present.
    pub fn anim(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { anim, .. } => Some(*anim),
            _ => None,
        }
    }
    /// Return the active animation identity, when present.
    pub fn active_anim(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { active_anim, .. } => Some(*active_anim),
            _ => None,
        }
    }
    /// Return the model zoom, when present.
    pub fn zoom(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { zoom, .. } => Some(*zoom),
            _ => None,
        }
    }
    /// Return the model X angle, when present.
    pub fn xan(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { xan, .. } => Some(*xan),
            _ => None,
        }
    }
    /// Return the model Y angle, when present.
    pub fn yan(&self) -> Option<i32> {
        match &self.inner {
            ComponentPropertyValues::Model { yan, .. } => Some(*yan),
            _ => None,
        }
    }

    fn matches(&self, kind: ComponentType) -> bool {
        matches!(
            (&self.inner, kind),
            (ComponentPropertyValues::Layer { .. }, ComponentType::Layer)
                | (ComponentPropertyValues::Unused {}, ComponentType::Unused)
                | (
                    ComponentPropertyValues::Inventory { .. },
                    ComponentType::Inventory
                )
                | (ComponentPropertyValues::Rect { .. }, ComponentType::Rect)
                | (ComponentPropertyValues::Text { .. }, ComponentType::Text)
                | (
                    ComponentPropertyValues::Sprite { .. },
                    ComponentType::Sprite
                )
                | (ComponentPropertyValues::Model { .. }, ComponentType::Model)
                | (
                    ComponentPropertyValues::InventoryText { .. },
                    ComponentType::InventoryText
                )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_data::{for_revision, SelectedGameData};
    use std::sync::Arc;

    fn catalog() -> CatalogWire {
        let mut asset: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../data/game-data/289.json")).unwrap();
        serde_json::from_value(asset["server_ordered"].take()).unwrap()
    }

    fn pin() -> Arc<SelectedPin> {
        for_revision(ClientRevision::R289)
            .unwrap()
            .selected_pin()
            .unwrap()
    }

    #[test]
    fn selected_component_properties_remain_borrowed_and_kind_specific() {
        let data = for_revision(ClientRevision::R289).unwrap();
        let catalog = data.server_ordered_catalog(&pin()).unwrap();
        let bank = catalog.component("bank_main:inv").unwrap().properties();
        assert_eq!(bank.draggable(), Some(true));
        assert_eq!(bank.margin_x(), Some(15));
        assert_eq!(bank.margin_y(), Some(6));
        assert_eq!(bank.actions().unwrap()[4].as_deref(), Some("Withdraw X"));
        assert_eq!(bank.font_id(), None);
        assert_eq!(bank.text(), None);
        let make = catalog
            .component("skill_multi3:make1_a")
            .unwrap()
            .properties();
        assert_eq!(make.centered(), Some(true));
        assert_eq!(make.font_id(), Some(3));
        assert_eq!(make.text(), Some(""));
        assert_eq!(make.actions(), None);
        assert_eq!(
            catalog
                .component("player_kit:accept")
                .unwrap()
                .properties()
                .fill(),
            Some(true)
        );
    }

    #[test]
    fn host_order_does_not_claim_synchronous_server_execution() {
        let catalog = catalog();
        let banker = catalog.entry(ContractKey::BankOpenDeposit).unwrap();
        let resume = banker.predecessors().last().unwrap();
        assert_eq!(resume.from(), "RESUME_PAUSEBUTTON(player_page)");
        assert_eq!(resume.basis(), ContractBasis::HostPolicy);
        assert!(!resume.synchronous());
        assert!(resume.sources().is_empty());
        let predicate = catalog
            .entry(ContractKey::BankWithdrawX)
            .unwrap()
            .server_predicates()
            .iter()
            .find(|condition| condition.name() == "exact_bank_slot_and_object")
            .unwrap();
        assert_eq!(predicate.basis(), ContractBasis::HostPolicy);
        assert!(predicate.sources().is_empty());
        let count = catalog
            .entry(ContractKey::BankWithdrawX)
            .unwrap()
            .predecessors()
            .first()
            .unwrap();
        assert_eq!(count.basis(), ContractBasis::Checked);
        assert!(count.synchronous());
        assert!(!count.sources().is_empty());

        let close = catalog.entry(ContractKey::BankCloseTerminal).unwrap();
        for name in [
            "stop_bank_main_and_side_inventory_listeners",
            "queue_bank_reorganization",
            "clear_weak_queue",
            "drop_protect_when_not_delayed",
            "cancel_suspended_count_or_pause_script",
            "run_expected_bank_if_close_trigger",
        ] {
            let condition = close
                .cleanup_effects()
                .iter()
                .find(|condition| condition.name() == name)
                .unwrap();
            assert_eq!(condition.basis(), ContractBasis::Checked);
            assert!(!condition.sources().is_empty());
        }
        let banker_page = catalog
            .entry(ContractKey::BankOpenDeposit)
            .unwrap()
            .server_predicates()
            .iter()
            .find(|condition| condition.name() == "banker_player_text_is_one_selected_font_page")
            .unwrap();
        assert_eq!(banker_page.basis(), ContractBasis::Checked);
        assert!(!banker_page.sources().is_empty());
    }

    #[test]
    fn unproved_or_uncited_edges_refuse_the_catalog() {
        for checked_without_source in [false, true] {
            let mut catalog = catalog();
            let edge = &mut catalog
                .entries
                .iter_mut()
                .find(|entry| entry.key == ContractKey::BankWithdrawX)
                .unwrap()
                .predecessors[0];
            if checked_without_source {
                edge.sources.clear();
            } else {
                edge.basis = ContractBasis::HostPolicy;
                edge.sources.clear();
            }
            assert_eq!(
                catalog.validate(&pin()),
                Err(ContractError::InvalidContract)
            );
        }
    }
    #[test]
    fn condition_basis_requires_matching_source_citations() {
        for checked_without_sources in [false, true] {
            let mut catalog = catalog();
            let condition = catalog
                .entries
                .iter_mut()
                .find(|entry| entry.key == ContractKey::BankCloseTerminal)
                .unwrap()
                .cleanup_effects
                .iter_mut()
                .find(|condition| condition.name == "clear_weak_queue")
                .unwrap();
            if checked_without_sources {
                condition.sources.clear();
            } else {
                condition.basis = ContractBasis::HostPolicy;
            }
            assert_eq!(
                catalog.validate(&pin()),
                Err(ContractError::InvalidContract)
            );
        }
    }

    #[test]
    fn selected_catalog_is_shared_and_cache_only() {
        let data = for_revision(ClientRevision::R289).unwrap();
        let pin = data.selected_pin().unwrap();
        let first = data.server_ordered_catalog(&pin).unwrap();
        let second = for_revision(ClientRevision::R289).unwrap();
        assert!(std::ptr::eq(
            first,
            second.server_ordered_catalog(&pin).unwrap()
        ));
        let measured = allocation_counter::measure(|| {
            for _ in 0..100 {
                let catalog = data.server_ordered_catalog(&pin).unwrap();
                assert_eq!(catalog.revision(), 289);
                assert_eq!(catalog.entries().len(), 8);
                assert_eq!(catalog.banker_page().unwrap().page_count(), 1);
            }
        });
        assert_eq!(measured.count_total, 0);
    }

    #[test]
    fn decoded_catalog_heap_stays_bounded() {
        let asset: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../data/game-data/289.json")).unwrap();
        let bytes = serde_json::to_vec(&asset["server_ordered"]).unwrap();
        let mut decoded = None;
        let measured = allocation_counter::measure(|| {
            decoded = Some(serde_json::from_slice::<CatalogWire>(&bytes).unwrap());
        });
        let retained = measured.bytes_current as usize + std::mem::size_of::<CatalogWire>();
        println!(
            "server_ordered bytes={} loaded_bytes={retained}",
            bytes.len()
        );
        assert!(
            retained < 131_072,
            "loaded catalog retains {retained} bytes"
        );
        assert!(decoded.is_some());
    }

    #[test]
    fn unbound_facts_cannot_expose_an_unverified_catalog() {
        let unbound: SelectedGameData =
            serde_json::from_slice(include_bytes!("../../data/game-data/289.json")).unwrap();
        assert!(matches!(
            unbound.server_ordered_catalog(&pin()),
            Err(ContractError::ContractUnavailable)
        ));
    }

    #[test]
    fn caller_pin_disagreement_refuses_the_shared_catalog() {
        let data = for_revision(ClientRevision::R289).unwrap();
        let mut changed = (*pin()).clone();
        changed.manifest_sha256[0] ^= 1;
        assert!(matches!(
            data.server_ordered_catalog(&changed),
            Err(ContractError::PinMismatch)
        ));
    }

    #[test]
    fn catalog_pin_disagreement_refuses_each_identity() {
        for field in ["engine_commit", "content_commit", "cache_id", "content_id"] {
            let mut catalog = catalog();
            match field {
                "engine_commit" => catalog.engine_commit.push('0'),
                "content_commit" => catalog.content_commit.push('0'),
                "cache_id" => catalog.cache_id.push('0'),
                "content_id" => catalog.content_id.push('0'),
                _ => unreachable!(),
            }
            assert_eq!(catalog.validate(&pin()), Err(ContractError::PinMismatch));
        }
    }

    #[test]
    fn unproved_revision_cannot_inherit_selected_289_controls() {
        let mut catalog = catalog();
        catalog.revision = 274;
        assert_eq!(
            catalog.validate(&pin()),
            Err(ContractError::ContractUnavailable)
        );
        let data = for_revision(ClientRevision::R274).unwrap();
        assert!(matches!(
            data.server_ordered_catalog(&data.selected_pin().unwrap()),
            Err(ContractError::ContractUnavailable)
        ));
    }

    #[test]
    fn missing_alias_refuses_the_complete_catalog() {
        let mut catalog = catalog();
        catalog
            .components
            .retain(|component| component.alias != "skill_multi3:make1_a");
        assert_eq!(catalog.validate(&pin()), Err(ContractError::MissingAlias));
    }

    #[test]
    fn ambiguous_component_identity_refuses_the_catalog() {
        let mut catalog = catalog();
        catalog.components[1].id = catalog.components[0].id;
        assert_eq!(catalog.validate(&pin()), Err(ContractError::AmbiguousAlias));
    }

    #[test]
    fn changed_root_ancestry_refuses_the_catalog() {
        let mut catalog = catalog();
        let component = catalog
            .components
            .iter_mut()
            .find(|component| component.alias == "skill_multi3:make1_a")
            .unwrap();
        component.ancestor_ids.clear();
        assert_eq!(catalog.validate(&pin()), Err(ContractError::WrongRoot));
    }

    #[test]
    fn changed_component_kind_refuses_the_catalog() {
        let mut catalog = catalog();
        let component = catalog
            .components
            .iter_mut()
            .find(|component| component.alias == "skill_multi3:make1_a")
            .unwrap();
        component.component_type = ComponentType::Layer;
        assert_eq!(
            catalog.validate(&pin()),
            Err(ContractError::ComponentChanged)
        );
    }

    #[test]
    fn changed_client_code_refuses_the_catalog() {
        let mut catalog = catalog();
        catalog.components[0].client_code = 999;
        assert_eq!(
            catalog.validate(&pin()),
            Err(ContractError::ComponentChanged)
        );
    }

    #[test]
    fn changed_button_kind_refuses_the_catalog() {
        let mut catalog = catalog();
        let component = catalog
            .components
            .iter_mut()
            .find(|component| component.alias == "skill_multi3:make1_a")
            .unwrap();
        component.button_type = ButtonType::Pause;
        assert_eq!(
            catalog.validate(&pin()),
            Err(ContractError::ComponentChanged)
        );
    }

    #[test]
    fn multiple_banker_pages_refuse_the_early_sequence() {
        let mut catalog = catalog();
        catalog.banker_page.as_mut().unwrap().page_count = 2;
        assert_independent_recipes_survive(catalog);
    }

    #[test]
    fn banker_lines_without_selected_page_capacity_refuse_the_sequence() {
        let mut catalog = catalog();
        catalog.banker_page.as_mut().unwrap().max_lines = 0;
        assert_independent_recipes_survive(catalog);
    }

    #[test]
    fn missing_banker_page_refuses_only_the_early_sequence() {
        let mut catalog = catalog();
        catalog.banker_page = None;
        assert_independent_recipes_survive(catalog);
    }

    fn assert_independent_recipes_survive(facts: CatalogWire) {
        let catalog = ServerOrderedCatalog::bind(facts, &pin()).unwrap();
        assert!(matches!(
            catalog.entry(ContractKey::BankOpenDeposit),
            Err(ContractError::BankerPageUnproved)
        ));
        assert_eq!(catalog.entries().len(), 7);
        for key in [
            ContractKey::KnifeArm,
            ContractKey::BankWithdrawX,
            ContractKey::BankDepositX,
            ContractKey::BankDepositAll,
            ContractKey::BankCloseTerminal,
            ContractKey::NumericBankbooth,
            ContractKey::AcceptCharacterDesign,
        ] {
            assert_eq!(catalog.entry(key).unwrap().key(), key);
        }
    }

    #[test]
    fn bank_mode_requires_an_explicit_selected_value() {
        let catalog = catalog();
        let bank = &catalog.bank;
        assert_eq!(
            bank.observed_mode(Some(bank.item_mode)),
            Some(BankWithdrawMode::Item)
        );
        assert_eq!(
            bank.observed_mode(Some(bank.note_mode)),
            Some(BankWithdrawMode::Note)
        );
        assert_eq!(bank.observed_mode(None), None);
        assert_eq!(bank.observed_mode(Some(i32::MAX)), None);
    }
}
