//! Stage previously generated local navigation payloads for the host binary.

#[path = "src/nav_assets.rs"]
mod nav_assets;
use std::path::PathBuf;

const ENV_KEYS: [&str; 4] = [
    "BOT_NAV_BUILD",
    "BOT_NAV_REVISION",
    "BOT_NAV_EXTRA_REVISIONS",
    "BOT_NAV_RESOURCE_DIR",
];

fn main() {
    for key in ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!("cargo:rerun-if-changed=src/nav_assets.rs");

    if std::env::var("BOT_NAV_BUILD").as_deref() == Ok("skip") {
        return;
    }

    let revisions = selected_revisions();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.join("../..");
    let source_root = nav_assets::local_asset_root(&workspace_root);
    let out_dir = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| fail("Cargo did not provide OUT_DIR"));
    let resource_override = std::env::var_os("BOT_NAV_RESOURCE_DIR").map(PathBuf::from);
    let resource_root = nav_assets::resource_root_for_build(&out_dir, resource_override.as_deref())
        .unwrap_or_else(|error| fail(&error));

    for revision in revisions {
        let layout = nav_assets::artifact_layout(revision);
        let payloads = [
            (&layout.relative_pack, "navpack"),
            (&layout.relative_flags, "navflags"),
            (&layout.relative_reach, "navreach"),
            (&layout.relative_canlight, "navcanlight"),
            (&layout.relative_pois, "navpois"),
        ];
        for (relative, kind) in payloads {
            let source = source_root.join(relative);
            println!("cargo:rerun-if-changed={}", source.display());
            if !source.is_file() {
                fail(&format!(
                    "local navigation {kind} {} is missing for revision {revision}; run `python tools/local-data/build.py` before building, or set BOT_NAV_BUILD=skip to omit navigation staging",
                    source.display()
                ));
            }
        }

        let staged_dir = resource_root.join(&layout.dir);
        std::fs::create_dir_all(&staged_dir).unwrap_or_else(|error| {
            fail(&format!(
                "resource directory {}: {error}",
                staged_dir.display()
            ))
        });
        for (relative, kind) in payloads {
            let source = source_root.join(relative);
            let destination = resource_root.join(relative);
            nav_assets::stage_payload(&source, &destination).unwrap_or_else(|error| {
                fail(&format!(
                    "stage local navigation {kind} {} to {}: {error}",
                    source.display(),
                    destination.display()
                ))
            });
        }
    }
}

fn parse_revision(raw: &str, key: &str) -> u16 {
    match raw {
        "274" => 274,
        "289" => 289,
        "377" => 377,
        other => fail(&format!("{key} must be 274, 289 or 377, got {other:?}")),
    }
}

fn selected_revisions() -> Vec<u16> {
    let primary = match std::env::var("BOT_NAV_REVISION") {
        Ok(value) => parse_revision(&value, "BOT_NAV_REVISION"),
        Err(_) => 289,
    };
    let mut revisions = vec![primary];
    if let Ok(extra) = std::env::var("BOT_NAV_EXTRA_REVISIONS") {
        for token in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let rev = parse_revision(token, "BOT_NAV_EXTRA_REVISIONS");
            if !revisions.contains(&rev) {
                revisions.push(rev);
            }
        }
    }
    revisions
}

fn fail(message: &(impl std::fmt::Display + ?Sized)) -> ! {
    panic!("navigation build: {message}");
}
