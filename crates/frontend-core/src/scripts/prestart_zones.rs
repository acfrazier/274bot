//! Pre-start disclosure of the exact zone crossings selected by Quester's queue.
use crate::selection::ProfileIdentity;
use script::quester::path::{PathDocument, StepDocument};
use script::quester::queue::{Queue, QueueSettings};
use script::quester::registry::{self, PathRegistry};
use std::fmt::Write as _;
use std::sync::Arc;

pub const START_ZONE_SCOPE_TITLE: &str = "Start zone scope";
pub const START_ZONE_PANEL_SCROLL_HINT: &str = "Scroll this panel to read the full list.";
pub const START_ZONE_TUI_SCROLL_HINT: &str = "Scroll this list with j/k or PgUp/PgDn.";
pub const START_ZONE_UNAVAILABLE_NOTICE: &str = "The selected Quester scope is unavailable.";
use serde_json::{Map, Value};

/// One authored zone crossing, with the exact identifier the guide accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuesterStartZone {
    pub id: String,
    /// Step responsible for this declaration; `None` means stage scope.
    pub step_id: Option<String>,
    pub name: String,
    pub reason: String,
    /// Cached presentation label with scope and exact accepted `id`.
    pub display_label: String,
}

/// Zone declarations scoped to a guide stage, recipe, prelude, or progress reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuesterStartZoneScope {
    pub quest_id: String,
    pub quest_name: String,
    pub role_id: Option<String>,
    /// Sequence stage, or a named prelude, recipe, or progress-reader scope.
    pub stage_id: String,
    pub zones: Vec<QuesterStartZone>,
    /// Cached quest and declaration scope identity for both renderers.
    pub display_label: String,
}

/// Complete pre-start disclosure for the selected Quester queue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuesterStartZones {
    pub scopes: Vec<QuesterStartZoneScope>,
    /// The one warning shown when the queue includes guides that live
    /// testing has not qualified. Set only when the user allowed them.
    unqualified_warning: Option<String>,
}

impl QuesterStartZones {
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    /// The shared warning for unqualified guides, when the user allowed them.
    pub fn unqualified_warning(&self) -> Option<&str> {
        self.unqualified_warning.as_deref()
    }

    /// The shared operator-facing explanation for this frozen disclosure.
    pub fn notice(&self) -> &'static str {
        if self.is_empty() {
            "No danger zones are declared. Quester walks avoid every danger zone."
        } else {
            "Start accepts only these zones. The food guard is armed for the whole walk; protection runs only where a step requests it. Every other danger zone stays avoided."
        }
    }

    /// Append the shared per-profile disclosure used by plan and report text.
    pub fn append_plan_disclosure(&self, profile: &str, text: &mut String) {
        let _ = write!(text, "{profile}: {}", self.notice());
        if let Some(warning) = &self.unqualified_warning {
            let _ = write!(text, "\n  {warning}");
        }
        for scope in &self.scopes {
            let _ = write!(text, "\n  {}", scope.display_label);
            for zone in &scope.zones {
                let _ = write!(text, "\n    {} — {}", zone.display_label, zone.reason);
            }
        }
    }
}

/// Caches effective settings and derived disclosure by profile, profile
/// generation, and the published registry revision.
#[derive(Debug, Default)]
pub struct QuesterStartZoneCache {
    profile: Option<ProfileIdentity>,
    profiles_generation: Option<u64>,
    registry_revision: Option<u64>,
    settings: Option<Map<String, Value>>,
    view: Option<Arc<Result<QuesterStartZones, String>>>,
}

impl QuesterStartZoneCache {
    /// Whether the durable effective settings are already cached for this
    /// profile version.
    pub fn settings_match(&self, profile: ProfileIdentity, generation: u64) -> bool {
        self.profile == Some(profile) && self.profiles_generation == Some(generation)
    }

    /// Refresh from one durable effective-settings read.
    pub fn update_settings(
        &mut self,
        profile: ProfileIdentity,
        generation: u64,
        settings: Result<Map<String, Value>, String>,
    ) -> bool {
        let (current_registry, registry_revision) = registry::snapshot_with_revision();
        self.profile = Some(profile);
        self.profiles_generation = Some(generation);
        self.registry_revision = Some(registry_revision);
        let view = match settings {
            Ok(settings) => {
                let view = resolve_selected_zones(&current_registry, &settings);
                self.settings = Some(settings);
                view
            }
            Err(reason) => {
                self.settings = None;
                Err(reason)
            }
        };
        self.view = Some(Arc::new(view));
        true
    }

    /// Store an unavailable-settings error for this profile version.
    pub fn update_unavailable(
        &mut self,
        profile: ProfileIdentity,
        generation: u64,
        reason: impl Into<String>,
    ) -> bool {
        let reason = reason.into();
        let registry_revision = registry::revision();
        if self.settings_match(profile, generation)
            && self.registry_revision == Some(registry_revision)
            && self
                .view
                .as_ref()
                .and_then(|view| view.as_ref().as_ref().err())
                .is_some_and(|previous| previous.as_str() == reason.as_str())
        {
            return false;
        }
        self.profile = Some(profile);
        self.profiles_generation = Some(generation);
        self.registry_revision = Some(registry_revision);
        self.settings = None;
        self.view = Some(Arc::new(Err(reason)));
        true
    }

    /// Re-resolve cached settings only when the registry publishes a new
    /// version. The common frame path reads one atomic and allocates nothing.
    pub fn refresh_registry(&mut self) -> bool {
        let Some(cached_revision) = self.registry_revision else {
            return false;
        };
        if registry::revision() == cached_revision {
            return false;
        }
        let (current_registry, current_revision) = registry::snapshot_with_revision();
        if current_revision == cached_revision {
            return false;
        }
        self.registry_revision = Some(current_revision);
        let Some(settings) = self.settings.as_ref() else {
            return false;
        };
        self.view = Some(Arc::new(resolve_selected_zones(
            &current_registry,
            settings,
        )));
        true
    }

    /// Clear the view when Quester is not the selected card.
    pub fn clear(&mut self) -> bool {
        if self.view.is_none()
            && self.profile.is_none()
            && self.profiles_generation.is_none()
            && self.registry_revision.is_none()
            && self.settings.is_none()
        {
            return false;
        }
        self.profile = None;
        self.profiles_generation = None;
        self.registry_revision = None;
        self.settings = None;
        self.view = None;
        true
    }

    /// The latest complete view, or its explicit failure.
    pub fn view(&self) -> Option<Result<&QuesterStartZones, &str>> {
        self.view.as_deref().map(|view| match view {
            Ok(zones) => Ok(zones),
            Err(reason) => Err(reason.as_str()),
        })
    }

    /// Share the immutable, versioned view with a front-end without copying
    /// its text or zone list.
    pub fn shared_view(&self) -> Option<Arc<Result<QuesterStartZones, String>>> {
        self.view.as_ref().map(Arc::clone)
    }
}

pub(crate) fn resolve_selected_zones(
    registry: &PathRegistry,
    bag: &Map<String, Value>,
) -> Result<QuesterStartZones, String> {
    let settings = queue_settings(bag)?;
    let allow_unqualified = settings.include_draft_guides;
    let mut queue = Queue::from_registry(registry.clone(), settings)
        .map_err(|error| format!("Quester queue settings are unavailable: {error}"))?;
    let mut view = QuesterStartZones::default();
    if allow_unqualified {
        view.unqualified_warning =
            script::quester::queue::unqualified_warning(&queue.unqualified_guides());
    }

    while let Some(position) = queue.next_candidate() {
        let id = queue
            .id(position)
            .ok_or_else(|| "selected Quester queue row has no guide id".to_owned())?;
        let bytes = queue
            .path_bytes(position)
            .ok_or_else(|| format!("selected Quester guide {id} has no current registry bytes"))?;
        let document: PathDocument = serde_json::from_slice(bytes.as_ref())
            .map_err(|error| format!("selected Quester guide {id} cannot be read: {error}"))?;
        append_document_zones(&mut view, document);
        queue.mark_done(position);
    }

    Ok(view)
}

fn queue_settings(bag: &Map<String, Value>) -> Result<QueueSettings, String> {
    Ok(QueueSettings {
        quests: string_values(bag, "quests")?,
        include_draft_guides: boolean_value(bag, "include_draft_guides")?,
        order_override: string_values(bag, "order_override")?,
        skip: string_values(bag, "skip")?,
        ..QueueSettings::default()
    })
}

fn string_values(bag: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = bag.get(key) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(format!("Quester setting {key} must be an array of strings"));
    };
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("Quester setting {key}[{index}] must be a string"))
        })
        .collect()
}

fn boolean_value(bag: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match bag.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("Quester setting {key} must be a boolean")),
    }
}

fn append_document_zones(view: &mut QuesterStartZones, document: PathDocument) {
    if let Some(header) = document.quest.as_ref() {
        for (recipe, steps) in &header.acquire {
            let mut zones = Vec::new();
            for step in steps {
                append_step_zones(&mut zones, step);
            }
            if !zones.is_empty() {
                let scope_id = format!("recipe {recipe}");
                view.scopes.push(scope(
                    &document.id.0,
                    &document.display_name,
                    None,
                    &scope_id,
                    zones,
                ));
            }
        }
    }

    for role in document.roles {
        let role_id = role.role.map(|role| role.0.to_string());
        let mut prelude_zones = Vec::new();
        for step in &role.prelude {
            append_step_zones(&mut prelude_zones, step);
        }
        if !prelude_zones.is_empty() {
            view.scopes.push(scope(
                &document.id.0,
                &document.display_name,
                role_id.as_deref(),
                "prelude",
                prelude_zones,
            ));
        }

        if let Some(progress_reader) = role.progress_reader.as_ref() {
            let mut zones = Vec::new();
            append_step_zones(&mut zones, progress_reader);
            if !zones.is_empty() {
                view.scopes.push(scope(
                    &document.id.0,
                    &document.display_name,
                    role_id.as_deref(),
                    "progress reader",
                    zones,
                ));
            }
        }

        for sequence in role.sequences {
            let mut zones = sequence
                .cross
                .into_iter()
                .map(|zone| start_zone(zone.id, zone.name, zone.reason, None))
                .collect::<Vec<_>>();
            for step in &sequence.steps {
                append_step_zones(&mut zones, step);
            }
            if !zones.is_empty() {
                view.scopes.push(scope(
                    &document.id.0,
                    &document.display_name,
                    role_id.as_deref(),
                    sequence.stage.0.as_ref(),
                    zones,
                ));
            }
        }
    }
}

fn append_step_zones(zones: &mut Vec<QuesterStartZone>, step: &StepDocument) {
    append_step_args_zones(
        zones,
        step.id.0.as_ref(),
        &step.kind,
        step.comment.as_deref(),
        &step.args,
    );
}

fn append_step_args_zones(
    zones: &mut Vec<QuesterStartZone>,
    step_id: &str,
    kind: &str,
    comment: Option<&str>,
    args: &Value,
) {
    if matches!(kind, "walk" | "combat") {
        let reason = comment
            .filter(|comment| !comment.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Required by step {step_id}"));
        if let Some(ids) = args.get("cross").and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                zones.push(start_zone(
                    id.to_owned(),
                    id.to_owned(),
                    reason.clone(),
                    Some(step_id.to_owned()),
                ));
            }
        }
    }
    visit_nested_step_zones(zones, args);
}

fn visit_nested_step_zones(zones: &mut Vec<QuesterStartZone>, value: &Value) {
    if let Some(step) = nested_step(value) {
        append_step_args_zones(zones, step.id, step.kind, step.comment, step.args);
        return;
    }
    match value {
        Value::Object(object) => {
            for value in object.values() {
                visit_nested_step_zones(zones, value);
            }
        }
        Value::Array(values) => {
            for value in values {
                visit_nested_step_zones(zones, value);
            }
        }
        _ => {}
    }
}

struct NestedStep<'a> {
    id: &'a str,
    kind: &'a str,
    comment: Option<&'a str>,
    args: &'a Value,
}

fn nested_step(value: &Value) -> Option<NestedStep<'_>> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?;
    let kind = object.get("kind")?.as_str()?;
    object
        .get("version")?
        .as_u64()
        .and_then(|version| u16::try_from(version).ok())?;
    let args = object.get("args")?;
    object.get("skip_if")?.as_object()?;
    object.get("settle")?.as_object()?;
    Some(NestedStep {
        id,
        kind,
        comment: object.get("comment").and_then(Value::as_str),
        args,
    })
}

fn start_zone(
    id: String,
    name: String,
    reason: String,
    step_id: Option<String>,
) -> QuesterStartZone {
    let display_label = match step_id.as_deref() {
        Some(step_id) => format!("{name} [{id}] · step {step_id}"),
        None => format!("{name} [{id}]"),
    };
    QuesterStartZone {
        id,
        step_id,
        name,
        reason,
        display_label,
    }
}

fn scope(
    quest_id: &str,
    quest_name: &str,
    role_id: Option<&str>,
    stage_id: &str,
    zones: Vec<QuesterStartZone>,
) -> QuesterStartZoneScope {
    let role = role_id.map_or_else(String::new, |role| format!(" · role {role}"));
    let context = if stage_id == "prelude"
        || stage_id == "progress reader"
        || stage_id.starts_with("recipe ")
    {
        stage_id.to_owned()
    } else {
        format!("stage {stage_id}")
    };
    let display_label = format!("{quest_name} ({quest_id}) · {context}{role}");
    let mut zones = zones;
    for zone in &mut zones {
        zone.display_label = format!("{display_label} · {}", zone.display_label);
    }
    QuesterStartZoneScope {
        quest_id: quest_id.to_owned(),
        quest_name: quest_name.to_owned(),
        role_id: role_id.map(str::to_owned),
        stage_id: stage_id.to_owned(),
        zones,
        display_label,
    }
}

#[cfg(test)]
mod tests {
    use super::{QuesterStartZoneCache, QuesterStartZones};
    use crate::selection::ProfileIdentity;
    use api::selected::{ClientRevision, FamilyPreparation};
    use api::snapshot::WorldTile;
    use nav_planner::collision::WorldCollision;
    use nav_planner::contract::NavLease;
    use nav_planner::router::AvoidRect;
    use nav_planner::transport::{TransportGraph, WildernessRules};
    use nav_planner::world::NavWorld;
    use nav_planner::zones::{Zone, ZoneKind, ZoneTable};
    use script::quester::registry::{FolderSource, PathRegistry};
    use serde_json::{json, Map, Value};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct RegistryFixture {
        _remove: RemoveFolder,
        registry: PathRegistry,
    }

    struct RemoveFolder(PathBuf);

    impl Drop for RemoveFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_zone_lease(ids: &[&str]) -> NavLease {
        let wilderness = WildernessRules::default();
        let table = ZoneTable::from_parts(
            ids.iter()
                .enumerate()
                .map(|(index, _)| {
                    Zone::hazard(
                        AvoidRect {
                            min_x: 0,
                            max_x: 0,
                            min_z: 0,
                            max_z: 0,
                            level: Some(0),
                        },
                        0,
                        u16::try_from(index).expect("fixture zone index fits"),
                    )
                })
                .collect(),
            ids.iter()
                .map(|id| ZoneKind::new(*id, *id, -1, 0, false, false))
                .collect(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            1,
            1,
            &wilderness,
        )
        .expect("build the fixture's validated zone table");
        let world = Arc::new(NavWorld::from_parts(
            WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 1,
                height: 1,
                walk: vec![0; 4],
                blocked: vec![0],
                flags: None,
            },
            TransportGraph {
                wilderness,
                zones: Some(table),
                ..TransportGraph::default()
            },
            Vec::new(),
        ));
        NavLease::wrap(world).expect("bind the fixture's navigation lease")
    }

    fn registry_fixture() -> RegistryFixture {
        let folder = std::env::temp_dir().join(format!(
            "quester-start-zones-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let remove = RemoveFolder(folder.clone());
        let mut document: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../script/paths/289/cook.json"
        )))
        .unwrap();
        document["display_name"] = json!("Cook Fixture");
        document["roles"][0]["sequences"][0]["cross"] = json!([{
            "id": "fixture-desert-pass",
            "name": "Fixture desert pass",
            "reason": "The guide needs this pass to reach the cave."
        }]);
        let mut walk = document["quest"]["acquire"]
            .as_object()
            .expect("Cook has acquire recipes")
            .values()
            .flat_map(|recipe| recipe.as_array().into_iter().flatten())
            .find(|step| step["kind"] == "walk")
            .expect("Cook has an acquire walk step")
            .clone();
        walk["id"] = json!("fixture-sequence-walk");
        walk["args"]["cross"] = json!(["fixture-river-bank"]);
        walk["comment"] = json!("The guide crosses this bank to reach the mill.");
        document["roles"][0]["sequences"][0]["steps"]
            .as_array_mut()
            .unwrap()
            .push(walk.clone());
        let mut prelude_walk = walk;
        prelude_walk["id"] = json!("fixture-prelude-walk");
        document["roles"][0]["prelude"] = json!([prelude_walk]);
        std::fs::write(
            folder.join("cook.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        std::fs::write(
            folder.join("index.json"),
            r#"{"schema":1,"paths":[{"id":"cook","file":"cook.json"}]}"#,
        )
        .unwrap();

        let selected = FamilyPreparation::run(|_| {
            api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
        })
        .expect("spawn selected game data worker")
        .join()
        .expect("selected game data worker");
        let quests = api::quest_facts::QuestCatalog::from_identity(selected.quest_identity())
            .expect("selected 289 quest catalog");
        let source = FolderSource {
            enabled: true,
            folder,
        };
        let lease = fixture_zone_lease(&["fixture-desert-pass", "fixture-river-bank"]);
        let registry = FamilyPreparation::run(move |worker| {
            PathRegistry::load(&source, &selected, &quests, worker, lease.zones())
                .expect("load fixture Path registry")
        })
        .expect("spawn Path registry worker")
        .join()
        .expect("Path registry worker");

        RegistryFixture {
            _remove: remove,
            registry,
        }
    }

    fn bag(quests: &[&str]) -> Map<String, Value> {
        Map::from_iter([("quests".into(), json!(quests))])
    }

    fn update_cache(
        cache: &mut QuesterStartZoneCache,
        generation: u64,
        settings: Map<String, Value>,
    ) -> bool {
        cache.update_settings(
            ProfileIdentity::synthetic("prestart-zone-cache-test"),
            generation,
            Ok(settings),
        )
    }

    fn preview(cache: &QuesterStartZoneCache) -> &QuesterStartZones {
        cache.view().expect("the cache has a view").unwrap()
    }

    fn quest_ids(zones: &QuesterStartZones) -> Vec<&str> {
        zones
            .scopes
            .iter()
            .map(|scope| scope.quest_id.as_str())
            .fold(Vec::new(), |mut ids, id| {
                if ids.last().copied() != Some(id) {
                    ids.push(id);
                }
                ids
            })
    }

    #[test]
    fn selected_guide_discloses_stage_step_and_prelude_declarations() {
        if !api::test_support::game_data_available() {
            return;
        }
        let fixture = registry_fixture();
        let zones = super::resolve_selected_zones(&fixture.registry, &bag(&["cook"]))
            .expect("selected Cook disclosure");
        let scope = zones
            .scopes
            .iter()
            .find(|scope| scope.quest_id == "cook" && scope.stage_id == "cook:0")
            .expect("selected Cook sequence scope is shown");
        assert_eq!(scope.quest_name, "Cook Fixture");
        assert_eq!(scope.stage_id, "cook:0");
        assert!(scope.display_label.contains("Cook Fixture (cook)"));
        let authored = scope
            .zones
            .iter()
            .find(|zone| zone.id == "fixture-desert-pass")
            .expect("stage declaration is shown");
        assert_eq!(authored.step_id, None);
        assert_eq!(authored.name, "Fixture desert pass");
        assert_eq!(
            authored.reason,
            "The guide needs this pass to reach the cave."
        );
        let sequence_step = scope
            .zones
            .iter()
            .find(|zone| zone.id == "fixture-river-bank")
            .expect("sequence step declaration is shown");
        assert_eq!(
            sequence_step.step_id.as_deref(),
            Some("fixture-sequence-walk")
        );
        assert!(sequence_step
            .display_label
            .contains("step fixture-sequence-walk"));
        assert!(sequence_step
            .display_label
            .contains("Cook Fixture (cook) · stage cook:0"));
        assert_eq!(
            sequence_step.reason,
            "The guide crosses this bank to reach the mill."
        );
        assert_eq!(
            scope.zones.len(),
            2,
            "stage and route grants are both shown"
        );

        let prelude = zones
            .scopes
            .iter()
            .find(|scope| scope.stage_id == "prelude")
            .expect("prelude step scope is shown");
        let prelude_step = prelude
            .zones
            .first()
            .expect("prelude step declaration is shown");
        assert_eq!(
            prelude_step.step_id.as_deref(),
            Some("fixture-prelude-walk")
        );
        assert!(prelude_step
            .display_label
            .contains("step fixture-prelude-walk"));
        assert!(prelude_step
            .display_label
            .contains("Cook Fixture (cook) · prelude"));
        assert_eq!(
            prelude_step.reason,
            "The guide crosses this bank to reach the mill."
        );
    }

    #[test]
    fn nested_recipe_step_and_progress_reader_grants_keep_scope() {
        let mut document: script::quester::path::PathDocument = serde_json::from_str(include_str!(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../script/paths/289/cook.json")
        ))
        .unwrap();
        let walk = document
            .quest
            .as_ref()
            .expect("Cook has a quest header")
            .acquire
            .values()
            .flatten()
            .find(|step| step.kind == "walk")
            .expect("Cook has an acquire walk step")
            .clone();
        let mut nested = walk.clone();
        nested.id = api::selected::FactKey::new("nested-recipe-walk");
        nested.args = json!({"cross": ["recipe-zone"]});
        nested.comment = Some("Nested recipe route reason.".into());
        let nested = serde_json::to_value(nested).unwrap();

        let mut recipe_step = walk.clone();
        recipe_step.id = api::selected::FactKey::new("recipe-wrapper");
        recipe_step.args = json!({"loop": {"steps": [{"action": nested}]}});
        recipe_step.comment = Some("Wrapper reason belongs to no nested grant.".into());
        document
            .quest
            .as_mut()
            .expect("Cook has a quest header")
            .acquire
            .insert("fixture-recipe".into(), vec![recipe_step]);

        let mut progress_reader = walk;
        progress_reader.id = api::selected::FactKey::new("fixture-progress-reader");
        progress_reader.args = json!({"cross": ["reader-zone"]});
        progress_reader.comment = Some("Progress reader route reason.".into());
        document.roles[0].progress_reader = Some(progress_reader);

        let mut zones = QuesterStartZones::default();
        super::append_document_zones(&mut zones, document);
        let recipe = zones
            .scopes
            .iter()
            .find(|scope| scope.stage_id == "recipe fixture-recipe")
            .expect("acquire recipe disclosure is scoped by recipe");
        assert!(recipe.display_label.contains("recipe fixture-recipe"));
        assert_eq!(recipe.zones.len(), 1);
        let nested = &recipe.zones[0];
        assert_eq!(nested.id, "recipe-zone");
        assert_eq!(nested.step_id.as_deref(), Some("nested-recipe-walk"));
        assert!(nested.display_label.contains("step nested-recipe-walk"));
        assert_eq!(nested.reason, "Nested recipe route reason.");

        let reader = zones
            .scopes
            .iter()
            .find(|scope| scope.stage_id == "progress reader")
            .expect("progress reader disclosure is scoped by reader");
        assert!(reader.display_label.contains("progress reader"));
        assert_eq!(reader.zones.len(), 1);
        assert_eq!(
            reader.zones[0].step_id.as_deref(),
            Some("fixture-progress-reader")
        );
        assert_eq!(reader.zones[0].reason, "Progress reader route reason.");
    }

    #[test]
    fn one_warning_names_the_unqualified_guides_only_when_the_setting_is_on() {
        let registry = PathRegistry::default();
        let mut picks = bag(&["priest", "arthur"]);
        let off = super::resolve_selected_zones(&registry, &picks).unwrap();
        assert_eq!(off.unqualified_warning(), None, "no warning while off");

        picks.insert("include_draft_guides".into(), json!(true));
        let on = super::resolve_selected_zones(&registry, &picks).unwrap();
        let warning = on.unqualified_warning().expect("warning while on");
        assert!(warning.contains("unqualified"), "{warning}");
        assert!(warning.contains("arthur"), "{warning}");
        assert!(
            !warning.contains("priest"),
            "a qualified guide is not named: {warning}"
        );
        let mut text = String::new();
        on.append_plan_disclosure("alice", &mut text);
        assert!(
            text.contains(warning),
            "the fleet plan shows it too: {text}"
        );

        let qualified_only = super::resolve_selected_zones(&registry, &{
            let mut only = bag(&["priest"]);
            only.insert("include_draft_guides".into(), json!(true));
            only
        })
        .unwrap();
        assert_eq!(
            qualified_only.unqualified_warning(), None,
            "nothing unqualified runs, so nothing to warn about"
        );
    }

    #[test]
    fn queue_defaults_skip_drafts_and_apply_opt_in_and_order_override() {
        let _source_lock = crate::quester_paths::registry_source_test_lock();
        let mut cache = QuesterStartZoneCache::default();
        assert!(update_cache(&mut cache, 0, Map::new()));
        let default_ids = quest_ids(preview(&cache));
        assert!(!default_ids.contains(&"arthur"));

        let mut draft_opt_in = Map::new();
        draft_opt_in.insert("include_draft_guides".into(), json!(true));
        assert!(update_cache(&mut cache, 1, draft_opt_in));
        assert!(quest_ids(preview(&cache)).contains(&"arthur"));

        let mut ordered = Map::new();
        ordered.insert("order_override".into(), json!(["arthur"]));
        assert!(update_cache(&mut cache, 2, ordered));
        assert_eq!(quest_ids(preview(&cache)).first().copied(), Some("arthur"));

        let mut explicit = bag(&["arthur"]);
        explicit.insert("skip".into(), json!(["arthur"]));
        explicit.insert("order_override".into(), json!(["arthur"]));
        assert!(update_cache(&mut cache, 3, explicit));
        assert!(preview(&cache).is_empty());
    }

    #[test]
    fn empty_declarations_and_live_settings_changes_are_visible() {
        let _source_lock = crate::quester_paths::registry_source_test_lock();
        let mut cache = QuesterStartZoneCache::default();
        let identity = ProfileIdentity::synthetic("prestart-zone-cache-test");
        assert!(update_cache(&mut cache, 1, bag(&["sheep"])));
        assert!(preview(&cache).is_empty());
        assert_eq!(
            preview(&cache).notice(),
            "No danger zones are declared. Quester walks avoid every danger zone."
        );

        let mut arthur = bag(&["arthur"]);
        arthur.insert("include_draft_guides".into(), json!(true));
        assert!(update_cache(&mut cache, 2, arthur));
        assert!(!preview(&cache).is_empty());
        assert_eq!(
            preview(&cache).notice(),
            "Start accepts only these zones. The food guard is armed for the whole walk; protection runs only where a step requests it. Every other danger zone stays avoided."
        );
        assert!(cache.settings_match(identity, 2));
        assert!(
            !cache.refresh_registry(),
            "unchanged registry reuses the cache"
        );

        assert!(update_cache(&mut cache, 3, bag(&["sheep"])));
        assert!(preview(&cache).is_empty());
    }

    #[test]
    fn desert_rescue_discloses_both_captain_crossings() {
        let zones =
            super::resolve_selected_zones(&PathRegistry::default(), &bag(&["desertrescue"]))
                .expect("bundled Desert Rescue queue resolves");
        let ids: Vec<_> = zones
            .scopes
            .iter()
            .flat_map(|scope| scope.zones.iter().map(|zone| zone.id.as_str()))
            .collect();
        for id in [
            "ugthanki@3268,3052,0",
            "ugthanki@3281,3056,0",
            "ugthanki@3305,3089,0",
        ] {
            assert_eq!(
                ids.iter().filter(|&&found| found == id).count(),
                2,
                "both captain walks disclose {id}: {ids:?}"
            );
        }
    }
}
