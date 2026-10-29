//! The script vocabulary the front ends name (T3.3): the run state, where
//! a card came from and how it runs, the loader shape and failure stage, a
//! compiled card's id, a picker choice and a card's setting definition.
//! The `script` crate's types stay behind these; conversions are
//! crate-private.

use api::selected::RunKey;
use script::ScriptSel;
#[cfg(feature = "js")]
use {script_js::LoadShape as ScriptLoadShape, script_js::LoadStage as ScriptLoadStage};

/// The script slot's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptRunState {
    /// No script.
    Idle,
    /// The Load-setup window: spawned, not ready.
    Starting,
    /// Running.
    Running,
    /// Operator Pause, or the not-up gate.
    Paused,
    /// The Load-join window.
    Stopping,
    /// The run failed.
    Error,
}

impl ScriptRunState {
    pub(crate) fn from_script(state: script::RunState) -> Self {
        match state {
            script::RunState::Idle => Self::Idle,
            script::RunState::Starting => Self::Starting,
            script::RunState::Running => Self::Running,
            script::RunState::Paused => Self::Paused,
            script::RunState::Stopping => Self::Stopping,
            script::RunState::Error => Self::Error,
        }
    }

    pub(crate) fn to_script(self) -> script::RunState {
        match self {
            Self::Idle => script::RunState::Idle,
            Self::Starting => script::RunState::Starting,
            Self::Running => script::RunState::Running,
            Self::Paused => script::RunState::Paused,
            Self::Stopping => script::RunState::Stopping,
            Self::Error => script::RunState::Error,
        }
    }

    /// Starting, running or paused: a run a logout would interrupt.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Paused)
    }

    /// Starting, running, paused or stopping: the run still owns its slot.
    /// This is the one definition of an "active script" for Stop eligibility
    /// and for tying a published script walk to its run.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Paused | Self::Stopping
        )
    }
}

/// Where a script card's metadata came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardSource {
    /// The rs2b0t catalogue.
    Catalog,
    /// A loaded file.
    File,
    /// Built in.
    Builtin,
}

impl CardSource {
    pub(crate) fn from_script(source: script::ScriptSource) -> Self {
        match source {
            script::ScriptSource::Catalog => Self::Catalog,
            script::ScriptSource::File => Self::File,
            script::ScriptSource::Builtin => Self::Builtin,
        }
    }

    pub(crate) fn to_script(self) -> script::ScriptSource {
        match self {
            Self::Catalog => script::ScriptSource::Catalog,
            Self::File => script::ScriptSource::File,
            Self::Builtin => script::ScriptSource::Builtin,
        }
    }
}

/// How a card runs in the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardRuntime {
    /// An rs2b0t compat script.
    Compat,
    /// A native-tick JS script.
    NativeTick,
    /// A compiled (native) card.
    Compiled,
}

#[cfg(feature = "js")]
impl CardRuntime {
    pub(crate) fn from_script(kind: script::ScriptKind) -> Self {
        match kind {
            script::ScriptKind::Compat => Self::Compat,
            script::ScriptKind::NativeTick => Self::NativeTick,
            script::ScriptKind::Compiled => Self::Compiled,
        }
    }
}

#[cfg(feature = "js")]
/// Which loader a JS source belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadShape {
    /// Old rs2b0t `defineBot(...)` / manifest-flagged source.
    CompatDefineBot,
    /// Catalogue shape: a default-export bot subclass.
    CompatClass,
    /// Modern source exporting a `tick` function.
    NativeTick,
    /// Not a recognised bot shape.
    Reject,
}

#[cfg(feature = "js")]
impl LoadShape {
    pub(crate) fn from_script(shape: ScriptLoadShape) -> Self {
        match shape {
            ScriptLoadShape::CompatDefineBot => Self::CompatDefineBot,
            ScriptLoadShape::CompatClass => Self::CompatClass,
            ScriptLoadShape::NativeTick => Self::NativeTick,
            ScriptLoadShape::Reject => Self::Reject,
        }
    }

    pub(crate) fn to_script(self) -> ScriptLoadShape {
        match self {
            Self::CompatDefineBot => ScriptLoadShape::CompatDefineBot,
            Self::CompatClass => ScriptLoadShape::CompatClass,
            Self::NativeTick => ScriptLoadShape::NativeTick,
            Self::Reject => ScriptLoadShape::Reject,
        }
    }
}

#[cfg(feature = "js")]
/// Stage of a retained per-source load failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadStage {
    /// Reading the file.
    Read,
    /// Parsing or transpiling it.
    ParseTranspile,
    /// Resolving its imports.
    ImportResolution,
    /// Loading it in the runtime.
    RuntimeLoad,
}

#[cfg(feature = "js")]
impl LoadStage {
    pub(crate) fn from_script(stage: ScriptLoadStage) -> Self {
        match stage {
            ScriptLoadStage::Read => Self::Read,
            ScriptLoadStage::ParseTranspile => Self::ParseTranspile,
            ScriptLoadStage::ImportResolution => Self::ImportResolution,
            ScriptLoadStage::RuntimeLoad => Self::RuntimeLoad,
        }
    }

    /// The stage's label.
    pub fn as_str(self) -> &'static str {
        self.to_script().as_str()
    }

    fn to_script(self) -> ScriptLoadStage {
        match self {
            Self::Read => ScriptLoadStage::Read,
            Self::ParseTranspile => ScriptLoadStage::ParseTranspile,
            Self::ImportResolution => ScriptLoadStage::ImportResolution,
            Self::RuntimeLoad => ScriptLoadStage::RuntimeLoad,
        }
    }
}

/// A compiled card's id: the exact name the picker shows and Start keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompiledCardId(&'static str);

impl CompiledCardId {
    pub(crate) fn from_script(id: script::CompiledId) -> Self {
        Self(id.0)
    }

    pub(crate) fn to_script(self) -> script::CompiledId {
        script::CompiledId(self.0)
    }

    /// The id text.
    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// The built-in Quester card.
    pub const QUESTER: Self = Self(script::quester::card::CARD.id.0);

    /// The compiled card with this exact id, if one is built in.
    pub fn lookup(name: &str) -> Option<Self> {
        script::compiled_id(name).map(Self::from_script)
    }
}

/// A picker choice: a compiled card, or a loaded JS card by source and
/// name.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptChoice(ScriptSel);

impl ScriptChoice {
    /// The same choice, copied (an explicit copy: the choice owns its
    /// lookup text).
    pub fn duplicate(&self) -> Self {
        Self(self.0.clone())
    }

    pub(crate) fn from_script(sel: ScriptSel) -> Self {
        Self(sel)
    }

    pub(crate) fn as_script(&self) -> &ScriptSel {
        &self.0
    }

    /// A compiled card.
    pub fn compiled(id: CompiledCardId) -> Self {
        Self(ScriptSel::Compiled(id.to_script()))
    }

    /// A loaded JS card.
    pub fn loaded(source: CardSource, name: &str) -> Self {
        Self(ScriptSel::Loaded(source.to_script(), name.to_string()))
    }

    /// The compiled card, if this is one.
    pub fn compiled_id(&self) -> Option<CompiledCardId> {
        match &self.0 {
            ScriptSel::Compiled(id) => Some(CompiledCardId::from_script(*id)),
            ScriptSel::Loaded(..) => None,
        }
    }

    /// The loaded card's source and name, if this is one.
    pub fn loaded_card(&self) -> Option<(CardSource, &str)> {
        match &self.0 {
            ScriptSel::Loaded(source, name) => Some((CardSource::from_script(*source), name)),
            ScriptSel::Compiled(_) => None,
        }
    }

    /// The card's source (a compiled card is built in).
    pub fn source(&self) -> CardSource {
        match &self.0 {
            ScriptSel::Loaded(source, _) => CardSource::from_script(*source),
            ScriptSel::Compiled(_) => CardSource::Builtin,
        }
    }

    /// The label the picker shows and Start keys on.
    pub fn label(&self) -> &str {
        match &self.0 {
            ScriptSel::Compiled(id) => id.0,
            ScriptSel::Loaded(_, name) => name,
        }
    }
}

/// One setting field of a card's settings schema.
#[derive(Debug, Clone, Copy)]
pub struct SettingView<'a>(&'a script::SettingDef);

impl<'a> SettingView<'a> {
    pub(crate) fn new(def: &'a script::SettingDef) -> Self {
        Self(def)
    }

    /// A view over a script setting definition (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_script(def: &'a script::SettingDef) -> Self {
        Self(def)
    }

    pub(crate) fn def(&self) -> &'a script::SettingDef {
        self.0
    }

    /// Whether this field shows for `bag` (its `showIf` condition holds).
    pub fn visible(&self, bag: &serde_json::Map<String, serde_json::Value>) -> bool {
        script::setting_visible(self.0.show_if.as_deref(), bag)
    }

    /// `value` coerced to this field's declared type.
    pub fn coerce(&self, value: &serde_json::Value) -> serde_json::Value {
        script::coerce_setting_value(&self.0.ty, value)
    }

    /// The field id (the bag key).
    pub fn id(&self) -> &'a str {
        &self.0.id
    }

    /// The field type (`bool`, `number`, `select`, …).
    pub fn ty(&self) -> &'a str {
        &self.0.ty
    }

    /// The default value text.
    pub fn default(&self) -> Option<&'a str> {
        self.0.default.as_deref()
    }

    /// The display label.
    pub fn label(&self) -> Option<&'a str> {
        self.0.label.as_deref()
    }

    /// The label, or the id when there is none.
    pub fn display_label(&self) -> &'a str {
        self.0.label.as_deref().unwrap_or(&self.0.id)
    }

    /// Minimum, maximum and step of a number field.
    pub fn bounds(&self) -> (Option<&'a str>, Option<&'a str>, Option<&'a str>) {
        (
            self.0.min.as_deref(),
            self.0.max.as_deref(),
            self.0.step.as_deref(),
        )
    }

    /// The declared options.
    pub fn options(&self) -> &'a [String] {
        &self.0.options
    }

    /// The declared option labels (parallel to [`Self::options`]).
    pub fn option_labels(&self) -> &'a [String] {
        &self.0.option_labels
    }

    /// The display group.
    pub fn group(&self) -> Option<&'a str> {
        self.0.group.as_deref()
    }

    /// The visibility condition.
    pub fn show_if(&self) -> Option<&'a str> {
        self.0.show_if.as_deref()
    }

    /// Where the options come from at run time.
    pub fn options_from(&self) -> Option<&'a str> {
        self.0.options_from.as_deref()
    }

    /// The comma-list toggle id.
    pub fn csv_toggle(&self) -> Option<&'a str> {
        self.0.csv_toggle.as_deref()
    }

    /// The help text.
    pub fn help(&self) -> Option<&'a str> {
        self.0.help.as_deref()
    }

    /// Whether the field has a selected-item option descriptor.
    pub fn has_item_options(&self) -> bool {
        self.0.item_option_spec.is_some()
    }
}

/// A card's settings schema, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct SchemaRef<'a>(&'a [script::SettingDef]);

impl<'a> SchemaRef<'a> {
    pub(crate) fn new(fields: &'a [script::SettingDef]) -> Self {
        Self(fields)
    }

    /// A view over script setting definitions (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_script(fields: &'a [script::SettingDef]) -> Self {
        Self(fields)
    }

    pub(crate) fn defs(&self) -> &'a [script::SettingDef] {
        self.0
    }

    /// No fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The number of fields.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The fields in schema order.
    pub fn iter(&self) -> impl Iterator<Item = SettingView<'a>> + 'a {
        self.0.iter().map(SettingView::new)
    }

    /// The field with id `id`.
    pub fn get(&self, id: &str) -> Option<SettingView<'a>> {
        self.0.iter().find(|def| def.id == id).map(SettingView::new)
    }

    /// The field at `index`.
    pub fn field(&self, index: usize) -> Option<SettingView<'a>> {
        self.0.get(index).map(SettingView::new)
    }

    /// The `(label, value)` rows of `bag` over this schema (the parameter
    /// summary a strip shows).
    pub fn parameter_rows(
        &self,
        bag: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<(String, String)> {
        script::parameter_rows(self.0, bag)
    }
}

/// An owned copy of a settings schema, for a front end that keeps the
/// schema it draws across frames (its buffer is reused on refresh).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OwnedSchema(Vec<script::SettingDef>);

impl OwnedSchema {
    /// The schema, borrowed.
    pub fn as_ref(&self) -> SchemaRef<'_> {
        SchemaRef::new(&self.0)
    }

    /// Replace the copy with `schema`, reusing the buffer. Copies nothing
    /// when the copy already equals `schema`, so a per-frame call over an
    /// unchanged schema does not allocate.
    pub fn copy_from(&mut self, schema: SchemaRef<'_>) {
        if self.0.as_slice() != schema.defs() {
            super::refill(&mut self.0, schema.defs());
        }
    }

    /// Drop every field.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(feature = "js")]
/// A loaded JS card.
#[derive(Debug, Clone, Copy)]
pub struct LoadedCard<'a>(&'a script_js::JsCard);

#[cfg(feature = "js")]
impl<'a> LoadedCard<'a> {
    pub(crate) fn new(card: &'a script_js::JsCard) -> Self {
        Self(card)
    }

    /// The picker name.
    pub fn name(&self) -> &'a str {
        &self.0.name
    }

    /// The origin file.
    pub fn path(&self) -> &'a std::path::Path {
        &self.0.path
    }

    /// Where the card came from.
    pub fn source(&self) -> CardSource {
        CardSource::from_script(self.0.source)
    }

    /// How it runs.
    pub fn runtime(&self) -> CardRuntime {
        CardRuntime::from_script(self.0.kind)
    }

    /// Its loader shape.
    pub fn shape(&self) -> LoadShape {
        LoadShape::from_script(self.0.shape)
    }

    /// Its settings schema.
    pub fn schema(&self) -> SchemaRef<'a> {
        SchemaRef::new(&self.0.settings_schema)
    }

    /// The origin text's SHA-256.
    pub fn sha256(&self) -> &'a str {
        &self.0.sha256
    }

    /// The first import that does not resolve, if any (Start refused).
    pub fn unloadable(&self) -> Option<&'a str> {
        self.0.unloadable.as_deref()
    }

    /// The description.
    pub fn description(&self) -> &'a str {
        &self.0.description
    }

    /// The category.
    pub fn category(&self) -> &'a str {
        &self.0.category
    }

    /// The tags.
    pub fn tags(&self) -> &'a [String] {
        &self.0.tags
    }

    /// The persistence identity key.
    /// Whether the card's JavaScript is compiled and ready to start.
    pub fn is_transpiled(&self) -> bool {
        !self.0.js.is_empty()
    }

    pub fn identity_key(&self) -> String {
        self.0.identity_key()
    }

    /// This card as a picker choice.
    pub fn choice(&self) -> ScriptChoice {
        ScriptChoice::from_script(ScriptSel::Loaded(self.0.source, self.0.identity_id()))
    }
}

#[cfg(feature = "js")]
/// A retained per-source load failure.
#[derive(Debug, Clone, Copy)]
pub struct LoadFailureView<'a>(&'a script_js::LoadFailure);

#[cfg(feature = "js")]
impl<'a> LoadFailureView<'a> {
    pub(crate) fn new(failure: &'a script_js::LoadFailure) -> Self {
        Self(failure)
    }

    /// The failure's identity key.
    pub fn identity_key(&self) -> &'a str {
        &self.0.identity_key
    }

    /// The card name.
    pub fn name(&self) -> &'a str {
        &self.0.name
    }

    /// The source file.
    pub fn path(&self) -> &'a std::path::Path {
        &self.0.path
    }

    /// The failing stage.
    pub fn stage(&self) -> LoadStage {
        LoadStage::from_script(self.0.stage)
    }

    /// The diagnostic text.
    pub fn diagnostic(&self) -> &'a str {
        &self.0.diagnostic
    }

    /// Line and column, when known.
    pub fn position(&self) -> (Option<u32>, Option<u32>) {
        (self.0.line, self.0.column)
    }

    /// The sibling module that failed, if any.
    pub fn sibling(&self) -> Option<&'a str> {
        self.0.sibling.as_deref()
    }

    /// The one-line `name: stage …` text.
    pub fn named_line(&self) -> String {
        self.0.named_line()
    }

    /// The copyable multi-line text.
    pub fn copyable_text(&self) -> String {
        self.0.copyable_text()
    }
}

/// A native card's published status.
#[derive(Debug, Clone, Copy)]
pub struct StatusView<'a>(&'a script::native::ScriptStatus);

impl<'a> StatusView<'a> {
    pub(crate) fn new(status: &'a script::native::ScriptStatus) -> Self {
        Self(status)
    }

    /// A view over a native script status (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_script(status: &'a script::native::ScriptStatus) -> Self {
        Self(status)
    }

    pub(crate) fn status(&self) -> &'a script::native::ScriptStatus {
        self.0
    }

    /// The compiled card that published it.
    pub fn card(&self) -> CompiledCardId {
        CompiledCardId::from_script(self.0.card)
    }

    /// The native run identity published with this status.
    pub fn run(&self) -> RunKey {
        self.0.run
    }

    /// The settings revision the run applies.
    pub fn active_settings(&self) -> u64 {
        self.0.active_settings
    }

    /// A settings revision the run has not applied yet.
    pub fn pending_settings(&self) -> Option<u64> {
        self.0.pending_settings
    }

    /// The terminal failure's message, if the run failed.
    pub fn failure_message(&self) -> Option<&'a str> {
        self.0
            .failure
            .as_ref()
            .map(|failure| failure.message.as_ref())
    }
}

/// A parameter value as the editors show it.
pub fn format_setting_value(value: &serde_json::Value) -> String {
    script::format_setting_value(value)
}
