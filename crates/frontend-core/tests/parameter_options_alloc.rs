//! Own test binary: shared preserved-label lookup and warm Quester zone text must not allocate.
//! Renderers use cached values when they show unavailable-quest labels or Quester scope.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use frontend_core::scripts::{resolve_parameter_options, Loadouts, Scripts, SettingView};
use frontend_core::test_support::{
    empty_play, session::with_settings_bag, FixtureProfile, IsolatedStores, VaultFixture,
};
use frontend_core::OperatorSession;
use host_play::{InstancePermit, PlayOptions, Transport};
use vault::CompiledSettingsRecord;

struct Counting;

thread_local! {
    static EVENTS: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter uses a non-allocating TLS cell.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn events() -> usize {
    EVENTS.with(Cell::get)
}
fn quest_options() -> frontend_core::scripts::ParameterOptions {
    let schema = (script::quester::card::CARD.schema)();
    let quests = schema
        .iter()
        .find(|field| field.id == "quests")
        .expect("Quester schema has a quest roster");
    let bag = serde_json::Map::from_iter([("quests".into(), serde_json::json!(["priest"]))]);
    let loadouts = Loadouts::default();
    resolve_parameter_options(SettingView::from_script(quests), &bag, &loadouts, None).unwrap()
}

fn repeated_lookup_allocations(
    options: &frontend_core::scripts::ParameterOptions,
    stored: &serde_json::Value,
) -> usize {
    let before = events();
    for _ in 0..128 {
        std::hint::black_box(
            options
                .preserved_stored_labels(std::hint::black_box(stored))
                .count(),
        );
    }
    events() - before
}

#[test]
fn repeated_unavailable_label_lookups_allocate_nothing() {
    let options = quest_options();
    let stored = serde_json::json!(["hauntedmine"]);
    let labels = options.preserved_stored_labels(&stored).collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(labels[0].contains("Haunted Mine"));

    assert_eq!(
        repeated_lookup_allocations(&options, &stored),
        0,
        "cached labels are borrowed on every repeated lookup"
    );
}

#[test]
fn repeated_lookup_without_a_preserved_value_allocates_nothing() {
    let options = quest_options();
    let stored = serde_json::json!(["priest"]);
    assert_eq!(options.preserved_stored_labels(&stored).count(), 0);

    assert_eq!(
        repeated_lookup_allocations(&options, &stored),
        0,
        "non-preserved selections need no temporary storage"
    );
}

fn new_scripts(directory: &std::path::Path) -> Scripts {
    #[cfg(feature = "js")]
    {
        frontend_core::test_support::scripts::install_catalog_cards();
        Scripts::new(
            script_js::JsLibrary::with_cache(
                directory.join("js-scripts.json"),
                directory.join("js-cache"),
            ),
            script::ScriptSettingsStore::at(directory.join("script-settings.json"))
                .expect("script settings"),
        )
    }
    #[cfg(not(feature = "js"))]
    {
        let _ = directory;
        Scripts::new()
    }
}

#[test]
fn warm_quester_script_section_shares_its_zone_text_without_allocating() {
    if !api::test_support::game_data_available() {
        return;
    }

    let stores = IsolatedStores::enter("quester-zone-frame-allocations");
    let directory = stores.dir();
    let id = script::CompiledId("Quester");
    let card = script::compiled_card(id).expect("Quester card");
    let values = serde_json::Map::from_iter([
        ("quests".to_owned(), serde_json::json!(["desertrescue"])),
        ("include_draft_guides".to_owned(), serde_json::json!(true)),
    ]);
    let entry = CompiledSettingsRecord {
        schema_version: card.schema_version,
        values,
    }
    .into_entry();
    let key = script::compiled_identity_key(id);
    let mut vault = VaultFixture::create(&directory.join("vault"), "test-passphrase-01");
    vault.put(with_settings_bag(
        FixtureProfile::new("alice", "pw").uid(1),
        &key,
        entry,
    ));
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let mut play = empty_play(&PlayOptions {
        host: "127.0.0.1".into(),
        transport: Transport::Tcp,
        port: 43594,
        cache_dir: directory.join("cache").to_string_lossy().into_owned(),
        lowmem: true,
        mainland: false,
    });
    play.bind_script_test_data(data);
    let mut core = OperatorSession::<()>::new(InstancePermit::SkipLock);
    core.set_spawn_workers(false);
    vault.start_session(&mut core, play);
    let mut scripts = new_scripts(directory);
    assert!(scripts.refresh_quester_start_zones(&core, Some("alice")));
    let view = scripts
        .shared_quester_start_zone_view()
        .expect("the selected Quester scope is cached");
    let zones = view.as_ref().as_ref().expect("the queue settings resolve");
    let notice = zones.notice();
    let label = zones
        .scopes
        .first()
        .expect("the selected guide declares a crossing")
        .display_label
        .as_str();

    let before = events();
    for _ in 0..256 {
        assert!(!scripts.refresh_quester_start_zones(&core, Some("alice")));
        let current = scripts
            .shared_quester_start_zone_view()
            .expect("the cached view stays present");
        assert!(std::sync::Arc::ptr_eq(&view, &current));
        let current_zones = current
            .as_ref()
            .as_ref()
            .expect("the same queue stays valid");
        assert_eq!(current_zones.notice(), notice);
        assert_eq!(current_zones.scopes[0].display_label, label);
    }
    assert_eq!(
        events() - before,
        0,
        "a warm Script section frame allocated"
    );
}
