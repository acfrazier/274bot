use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use script_js::{ApiFamily, JsLibrary, LoadIsolate, LoadShape};

const GUIDE: &str = include_str!("../../../docs/api/writing-scripts.md");
const EXAMPLE_MARKER: &str = "<!-- ci-example: quickstart-v2 -->";
static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-js-v2-authoring-guide-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create authoring guide scratch directory");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn quickstart_source() -> String {
    let after_marker = GUIDE
        .split_once(EXAMPLE_MARKER)
        .expect("guide includes the CI example marker")
        .1;
    let mut lines = after_marker.lines();
    assert!(
        lines.any(|line| line.trim() == "```ts"),
        "guide marker is followed by a TypeScript code block"
    );

    let mut source = String::new();
    for line in lines {
        if line.trim() == "```" {
            return source;
        }
        source.push_str(line);
        source.push('\n');
    }
    panic!("guide TypeScript code block has a closing fence");
}

#[test]
fn writing_guide_quickstart_loads_validates_and_executes_as_v2() {
    let scratch = TempDir::new();
    let examples_dir = scratch.path().join("examples");
    let host_types_dir = scratch.path().join("host-js");
    std::fs::create_dir_all(&examples_dir).expect("create example source directory");
    std::fs::create_dir_all(&host_types_dir).expect("create host declaration directory");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("host-js/index.d.ts"),
        host_types_dir.join("index.d.ts"),
    )
    .expect("copy generated host declarations");
    let source_path = examples_dir.join("quickstart.ts");
    std::fs::write(&source_path, quickstart_source()).expect("write extracted guide example");

    let mut library = JsLibrary::with_cache(
        scratch.path().join("cards.json"),
        scratch.path().join("cache"),
    );
    let card = library
        .load(&source_path)
        .expect("manifest example loads as a native card");
    assert_eq!(card.checked.family(), ApiFamily::V2);
    assert_eq!(card.checked.shape(), LoadShape::NativeTick);
    assert!(card.manifest.is_some());
    assert!(card.unloadable.is_none(), "{:?}", card.unloadable);
    assert!(!card.js.contains("export const manifest"));

    assert_eq!(card.settings_schema.len(), 1);
    let count = &card.settings_schema[0];
    assert_eq!(count.id, "count");
    assert_eq!(count.ty, "number");
    assert_eq!(count.default.as_deref(), Some("3"));
    assert_eq!(count.min.as_deref(), Some("1"));
    assert_eq!(count.max.as_deref(), Some("10"));

    let shape = card.shape();
    let runtime_source =
        script_js::transpile_ts(&card.origin).expect("transpile extracted guide example");
    let isolate =
        LoadIsolate::spawn(runtime_source, shape, vec![]).expect("spawn extracted guide example");
    let mut settings = serde_json::Map::new();
    settings.insert("count".into(), serde_json::json!(7));
    isolate.post_settings_bag(&settings);
    isolate.on_game_tick(1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while isolate.script_stop_receipt().is_none() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let receipt = isolate
        .script_stop_receipt()
        .expect("guide example executes tick and stops");
    isolate.join();

    assert_eq!(receipt.reason, "Quick start complete. Count=7");
}
