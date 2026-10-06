use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FamilyPreparation};
use script::quester::compile::compile_path_for_gang;
use script::quester::pair::Gang;
use script::quester::registry::{self, FolderSource, PathRegistry};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const PATH_ID: &str = "paired-cook-draft";

struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "quest-pair-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn source(&self) -> FolderSource {
        FolderSource {
            enabled: true,
            folder: self.0.clone(),
        }
    }
    fn write(&self, document: &serde_json::Value) {
        std::fs::write(
            self.0.join(format!("{PATH_ID}.json")),
            serde_json::to_vec(document).unwrap(),
        )
        .unwrap();
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn paired_document() -> serde_json::Value {
    let mut document: serde_json::Value =
        serde_json::from_str(include_str!("../paths/289/cook.json")).unwrap();
    document["id"] = serde_json::json!(PATH_ID);
    document["partner"] = serde_json::json!({
        "protocol": "arrav",
        "roles": [
            {"id": "phoenix", "gang": "Phoenix"},
            {"id": "blackarm", "gang": "BlackArm"}
        ]
    });
    let mut phoenix = document["roles"][0].clone();
    phoenix["role"] = serde_json::json!("phoenix");
    let mut blackarm = phoenix.clone();
    blackarm["role"] = serde_json::json!("blackarm");
    document["roles"] = serde_json::json!([phoenix, blackarm]);
    document
}

fn with_data(
    test: impl FnOnce(Arc<SelectedGameData>, QuestCatalog, &mut FamilyPreparation) + Send + 'static,
) {
    FamilyPreparation::run(move |worker| {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        test(selected, quests, worker);
    })
    .unwrap()
    .join()
    .unwrap();
}

#[test]
fn quest_pair_r2_registry_load_validates_both_declared_gangs() {
    with_data(|selected, quests, worker| {
        let folder = Folder::new();
        folder.write(&paired_document());
        let paths = PathRegistry::load(&folder.source(), &selected, &quests, worker).unwrap();
        assert!(paths.diagnostics().is_empty(), "{:?}", paths.report());
        let bytes = paths
            .bytes(PATH_ID)
            .expect("both valid roles admit the paired document");
        for gang in [Gang::Phoenix, Gang::BlackArm] {
            let path =
                compile_path_for_gang(bytes.as_ref(), &selected, &quests, worker, Some(gang))
                    .unwrap();
            assert_eq!(
                path.role.as_ref().unwrap().0.as_ref(),
                match gang {
                    Gang::Phoenix => "phoenix",
                    Gang::BlackArm => "blackarm",
                }
            );
        }
    });
}

// Reload owns process-wide source state. This separate integration-test process
// prevents the public Reload test from changing sources under unrelated Start tests.
struct RestoreSource(FolderSource);
impl Drop for RestoreSource {
    fn drop(&mut self) {
        registry::set_source(self.0.clone());
    }
}

#[test]
fn quest_pair_r2_registry_public_reload_revalidates_edited_paired_roles() {
    with_data(|selected, quests, worker| {
        let folder = Folder::new();
        let mut document = paired_document();
        folder.write(&document);
        let _restore = RestoreSource(registry::source());
        registry::set_source(folder.source());
        let first = registry::reload(&selected, worker).unwrap();
        assert!(first.diagnostics().is_empty(), "{:?}", first.report());
        let bytes = first
            .bytes(PATH_ID)
            .expect("Reload accepts a valid paired document");
        let first_path = compile_path_for_gang(
            bytes.as_ref(),
            &selected,
            &quests,
            worker,
            Some(Gang::BlackArm),
        )
        .unwrap();
        document["roles"][1]["sequences"][0]["steps"][0]["comment"] =
            serde_json::json!("reloaded BlackArm role");
        folder.write(&document);
        let second = registry::reload(&selected, worker).unwrap();
        assert!(second.diagnostics().is_empty(), "{:?}", second.report());
        let bytes = second.bytes(PATH_ID).unwrap();
        let second_path = compile_path_for_gang(
            bytes.as_ref(),
            &selected,
            &quests,
            worker,
            Some(Gang::BlackArm),
        )
        .unwrap();
        assert_ne!(first_path.digest, second_path.digest);
        assert_eq!(
            second_path.sequences[0].steps[0].comment.as_deref(),
            Some("reloaded BlackArm role")
        );
        document["roles"][1]["sequences"][0]["steps"][0]["kind"] =
            serde_json::json!("invalid-family");
        folder.write(&document);
        let invalid = registry::reload(&selected, worker).unwrap();
        assert!(
            invalid.bytes(PATH_ID).is_none(),
            "Reload must revoke invalid edited roles"
        );
        assert!(invalid.report().unwrap().contains("BlackArm"));
    });
}

#[test]
fn quest_pair_r2_registry_invalid_role_reports_its_gang_and_original_compile_error() {
    with_data(|selected, quests, worker| {
        for (side, gang) in [(0, "Phoenix"), (1, "BlackArm")] {
            let folder = Folder::new();
            let mut document = paired_document();
            document["roles"][side]["sequences"][0]["steps"][0]["kind"] =
                serde_json::json!("invalid-family");
            folder.write(&document);
            let paths = PathRegistry::load(&folder.source(), &selected, &quests, worker).unwrap();
            assert!(
                paths.bytes(PATH_ID).is_none(),
                "one invalid role rejects the entire document"
            );
            let diagnostics = paths.diagnostics();
            assert_eq!(diagnostics.len(), 1, "one per-file CompileError");
            let issue = &diagnostics[0];
            assert_eq!(issue.file, folder.0.join(format!("{PATH_ID}.json")));
            assert_eq!(issue.error.code.as_ref(), "unknown-handler");
            assert!(
                issue.error.step.is_some(),
                "the authored step identity must survive"
            );
            assert!(issue.to_string().contains(gang), "missing gang: {issue}");
        }
    });
}
