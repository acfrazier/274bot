//! Pre-start disclosure of the exact zone crossings selected by Quester's queue.
use std::sync::Arc;

use script::quester::path::{PathDocument, StepDocument};
use script::quester::queue::{Queue, QueueSettings};
use script::quester::registry::{self, PathRegistry};
use serde_json::{Map, Value};

/// One authored zone crossing, with the exact identifier the guide accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuesterStartZone {
    pub id: String,
    /// Step responsible for this declaration; `None` means stage scope.
    pub step_id: Option<String>,
    pub name: String,
    pub reason: String,
    /// Cached presentation label. It includes the exact accepted `id`.
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
}

impl QuesterStartZones {
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

/// Caches the selected guide metadata until either queue settings or the
/// immutable Path registry snapshot changes.
#[derive(Debug, Default)]
pub struct QuesterStartZoneCache {
    settings: Option<QueueSettingsSignature>,
    registry: Option<PathRegistry>,
    view: Option<Result<QuesterStartZones, String>>,
}

impl QuesterStartZoneCache {
    /// Refresh from the selected profile's effective Quester settings.
    /// Returns `true` when the cached view changes.
    pub fn update(&mut self, bag: &Map<String, Value>) -> bool {
        let current_registry = registry::snapshot();
        if self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.matches(bag))
            && self
                .registry
                .as_ref()
                .is_some_and(|cached| same_snapshot(cached, &current_registry))
        {
            return false;
        }

        let settings = QueueSettingsSignature::capture(bag);
        let view = resolve_selected_zones(&current_registry, bag);
        self.settings = Some(settings);
        self.registry = Some(current_registry);
        self.view = Some(view);
        true
    }

    /// Store an unavailable-settings error from the effective-settings reader.
    pub fn set_unavailable(&mut self, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        if self.settings.is_none()
            && self.registry.is_none()
            && self
                .view
                .as_ref()
                .and_then(|view| view.as_ref().err())
                .is_some_and(|previous| previous == &reason)
        {
            return false;
        }
        self.settings = None;
        self.registry = None;
        self.view = Some(Err(reason));
        true
    }

    /// Clear the view when Quester is not the selected card.
    pub fn clear(&mut self) -> bool {
        if self.view.is_none() && self.settings.is_none() && self.registry.is_none() {
            return false;
        }
        self.settings = None;
        self.registry = None;
        self.view = None;
        true
    }

    /// The latest complete view, or its explicit failure.
    pub fn view(&self) -> Option<Result<&QuesterStartZones, &str>> {
        self.view.as_ref().map(|view| match view {
            Ok(zones) => Ok(zones),
            Err(reason) => Err(reason.as_str()),
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct QueueSettingsSignature {
    quests: Option<Value>,
    include_draft_guides: Option<Value>,
    order_override: Option<Value>,
    skip: Option<Value>,
}

impl QueueSettingsSignature {
    fn capture(bag: &Map<String, Value>) -> Self {
        Self {
            quests: bag.get("quests").cloned(),
            include_draft_guides: bag.get("include_draft_guides").cloned(),
            order_override: bag.get("order_override").cloned(),
            skip: bag.get("skip").cloned(),
        }
    }

    fn matches(&self, bag: &Map<String, Value>) -> bool {
        self.quests.as_ref() == bag.get("quests")
            && self.include_draft_guides.as_ref() == bag.get("include_draft_guides")
            && self.order_override.as_ref() == bag.get("order_override")
            && self.skip.as_ref() == bag.get("skip")
    }
}

fn same_snapshot(left: &PathRegistry, right: &PathRegistry) -> bool {
    match (left, right) {
        (PathRegistry::Bundled, PathRegistry::Bundled) => true,
        (PathRegistry::Folder(left), PathRegistry::Folder(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

fn resolve_selected_zones(
    registry: &PathRegistry,
    bag: &Map<String, Value>,
) -> Result<QuesterStartZones, String> {
    let settings = queue_settings(bag)?;
    let mut queue = Queue::from_registry(registry.clone(), settings)
        .map_err(|error| format!("Quester queue settings are unavailable: {error}"))?;
    let mut view = QuesterStartZones::default();

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
    use api::selected::{ClientRevision, FamilyPreparation};
    use api::snapshot::WorldTile;
    use nav_planner::collision::WorldCollision;
    use nav_planner::contract::{NavLease, PackDigest};
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
        NavLease::wrap(world, PackDigest::of(b"prestart-zone-fixture"))
            .expect("bind the fixture's navigation lease")
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
    fn queue_defaults_skip_drafts_and_apply_opt_in_and_order_override() {
        let _source_lock = crate::quester_paths::registry_source_test_lock();
        let mut cache = QuesterStartZoneCache::default();
        assert!(cache.update(&Map::new()));
        let default_ids = quest_ids(preview(&cache));
        assert!(!default_ids.contains(&"arthur"));

        let mut draft_opt_in = Map::new();
        draft_opt_in.insert("include_draft_guides".into(), json!(true));
        assert!(cache.update(&draft_opt_in));
        assert!(quest_ids(preview(&cache)).contains(&"arthur"));

        let mut ordered = Map::new();
        ordered.insert("order_override".into(), json!(["arthur"]));
        assert!(cache.update(&ordered));
        assert_eq!(quest_ids(preview(&cache)).first().copied(), Some("arthur"));

        let mut explicit = bag(&["arthur"]);
        explicit.insert("skip".into(), json!(["arthur"]));
        explicit.insert("order_override".into(), json!(["arthur"]));
        assert!(cache.update(&explicit));
        assert!(preview(&cache).is_empty());
    }

    #[test]
    fn empty_declarations_and_live_settings_changes_are_visible() {
        let _source_lock = crate::quester_paths::registry_source_test_lock();
        let mut cache = QuesterStartZoneCache::default();
        assert!(cache.update(&bag(&["sheep"])));
        assert!(preview(&cache).is_empty());

        let mut arthur = bag(&["arthur"]);
        arthur.insert("include_draft_guides".into(), json!(true));
        assert!(cache.update(&arthur));
        assert!(!preview(&cache).is_empty());
        assert!(!cache.update(&arthur), "unchanged settings reuse the cache");
        assert!(
            cache.update(&bag(&["sheep"])),
            "live settings refresh the cache"
        );
        assert!(preview(&cache).is_empty());
    }
}
