// Compat (JS API v1) declarations — generated declarations gated against the live shim.
use script::compat_dts::{
    check_compat_dts_drift, collect_compat_surface_from, compat_dts_path, drift_against_shim,
    load_authored_dts, load_authored_dts_from,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn compat_dts_is_fresh() {
    check_compat_dts_drift().unwrap_or_else(|e| panic!("{e}"));
}

/// Re-run the frozen emitter and named runtime overlay without rewriting index.d.ts.
/// The integration owner runs this with RS2B0T pointing at the frozen 00d39a17e0 source.
#[test]
#[ignore = "requires RS2B0T set to the frozen rs2b0t source"]
fn generator_check_matches_frozen_emit_and_overlay() {
    let rs2b0t = std::env::var_os("RS2B0T").unwrap_or_else(|| {
        panic!(
            "RS2B0T is unset; set RS2B0T to the frozen rs2b0t 00d39a17e0 source before \
             running `cargo test -p script --test compat_dts -- --include-ignored`"
        )
    });
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let generator = manifest.join("compat-js/generate.cjs");
    assert!(
        generator.is_absolute(),
        "generator path must be absolute: {}",
        generator.display()
    );
    let output = Command::new("node")
        .arg(&generator)
        .arg("--check")
        .env("RS2B0T", rs2b0t)
        .current_dir(manifest)
        .output()
        .unwrap_or_else(|e| panic!("failed to run node {} --check: {e}", generator.display()));
    assert!(
        output.status.success(),
        "generator --check failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Exercise the declaration-tree export used by frozen recounts. Set COMPAT_DTS_TREE
/// to keep the output at a caller-selected path.
#[test]
#[ignore]
fn write_compat_dts_tree_to_env() {
    let persistent = std::env::var_os("COMPAT_DTS_TREE");
    let dir = persistent.as_deref().map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("compat-dts-tree-{}", std::process::id()))
    });
    script::compat_dts::write_compat_dts_tree(&dir).expect("write compat dts tree");
    eprintln!("wrote tree {}", dir.display());
    if persistent.is_none() {
        fs::remove_dir_all(&dir).expect("remove temporary compat dts tree");
    }
}

fn drift_for_game_source(source: &str) -> Vec<String> {
    let authored = load_authored_dts().expect("generated compat declarations");
    let shim = collect_compat_surface_from(&[("/rs2b0t/bot/api/game/Game.js", source)]);
    drift_against_shim(&authored, &shim)
}

#[test]
fn drift_gate_fails_when_shim_gains_a_method() {
    let drifts = drift_for_game_source("export const Game = { extraProbeMethod() {} };");
    assert!(
        drifts.iter().any(|drift| {
            drift.contains("Game")
                && drift.contains("extraProbeMethod")
                && drift.contains("missing member")
        }),
        "a shim-only Game method must be reported: {drifts:?}"
    );
}

#[test]
fn drift_gate_rejects_a_new_required_shim_parameter() {
    let drifts =
        drift_for_game_source("export const Game = { ingame(required) { return true; } };");
    assert!(
        drifts.iter().any(|drift| {
            drift.contains("Game.ingame arity") && drift.contains("shim requires 1")
        }),
        "a new required shim parameter must be reported: {drifts:?}"
    );
}

#[test]
fn drift_gate_rejects_an_async_shim_mutation() {
    let drifts = drift_for_game_source("export const Game = { async ingame() { return true; } };");
    assert!(
        drifts
            .iter()
            .any(|drift| drift.contains("Game.ingame async-ness")),
        "an async shim mutation must be reported: {drifts:?}"
    );
}

/// Consumer TypeScript probe: correct uses compile and deliberately wrong Game uses are
/// covered by @ts-expect-error. Also checks index.d.ts with skipLibCheck: false.
/// Always runs the pinned TypeScript 5.8.3 (a `tsc` on PATH may be 7.x).
#[test]
#[ignore = "requires npx and TypeScript 5.8.3"]
fn tsc_consumer_probe_rejects_wrong_uses() {
    let probe = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat_dts_probe");
    let output = tsc_command()
        .arg("--noEmit")
        .arg("-p")
        .arg(&probe)
        .output()
        .unwrap_or_else(|e| panic!("tsc probe failed to spawn: {e}"));
    assert!(
        output.status.success(),
        "tsc consumer probe failed (skipLibCheck: false):\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The original and R5 consumers must have zero diagnostics with strict mode and
/// skipLibCheck: false against the final generated, overlay-applied declarations.
#[test]
#[ignore = "requires npx and TypeScript 5.8.3"]
fn tsc_valid_consumer_probe_has_zero_diagnostics() {
    let project =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat_dts_probe/consumer.tsconfig.json");
    let output = tsc_command()
        .arg("--noEmit")
        .arg("-p")
        .arg(&project)
        .output()
        .unwrap_or_else(|e| panic!("tsc valid consumer probe failed to spawn: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    let diag_count = combined
        .lines()
        .filter(|line| line.contains("error TS"))
        .count();
    assert!(
        output.status.success() && diag_count == 0,
        "valid consumers must have 0 diagnostics against generated declarations (saw {diag_count}):\n{stdout}\n{stderr}"
    );
}

fn tsc_command() -> Command {
    let mut npx = Command::new("npx");
    npx.args(["-p", "typescript@5.8.3", "--yes", "tsc"]);
    npx
}

/// Locate the barrel structurally so TypeScript printer quoting and whitespace are irrelevant.
fn with_gatherer_barrel_export(source: &str) -> String {
    let declaration = "declare module";
    let mut search_from = 0;
    while let Some(relative) = source[search_from..].find(declaration) {
        let start = search_from + relative;
        let after_keyword = start + declaration.len();
        let rest = &source[after_keyword..];
        let Some(quote_offset) = rest.find(&['\'', '"'][..]) else {
            search_from = after_keyword;
            continue;
        };
        let quote = source.as_bytes()[after_keyword + quote_offset];
        let specifier_start = after_keyword + quote_offset + 1;
        let Some(specifier_length) = source[specifier_start..]
            .bytes()
            .position(|byte| byte == quote)
        else {
            search_from = specifier_start;
            continue;
        };
        let specifier_end = specifier_start + specifier_length;
        if &source[specifier_start..specifier_end] != "@rs2b0t/api" {
            search_from = specifier_end + 1;
            continue;
        }
        let Some(open_offset) = source[specifier_end + 1..].find('{') else {
            panic!("@rs2b0t/api declaration has no body");
        };
        let open = specifier_end + 1 + open_offset;
        let close = matching_brace(source, open).expect("@rs2b0t/api body is balanced");
        let mut patched = String::with_capacity(source.len() + 64);
        patched.push_str(&source[..close]);
        patched.push_str("\n  export { Gatherer } from '*api/gather/Gatherer.js';\n");
        patched.push_str(&source[close..]);
        return patched;
    }
    panic!("generated declarations have no @rs2b0t/api barrel");
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut index = open;
    while index < bytes.len() {
        let byte = bytes[index];
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
        } else if block_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                block_comment = false;
                index += 1;
            }
        } else if let Some(delimiter) = quote {
            if byte == b'\\' {
                index += 1;
            } else if byte == delimiter {
                quote = None;
            }
        } else {
            match byte {
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    line_comment = true;
                    index += 1;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    block_comment = true;
                    index += 1;
                }
                b'\'' | b'"' | b'`' => quote = Some(byte),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        index += 1;
    }
    None
}

fn write_gatherer_extension(dir: &Path, with_barrel: bool, with_file: bool) {
    let source = fs::read_to_string(compat_dts_path()).expect("generated declarations");
    fs::create_dir_all(dir).expect("create extension test directory");
    let index = if with_barrel {
        with_gatherer_barrel_export(&source)
    } else {
        source
    };
    fs::write(dir.join("index.d.ts"), index).expect("write extension index");
    if with_file {
        let gather_dir = dir.join("api/gather");
        fs::create_dir_all(&gather_dir).expect("create Gatherer module directory");
        fs::write(
            gather_dir.join("Gatherer.d.ts"),
            r#"/** O-SCRIPT-API extension fixture. */
export type GatherMode = 'woodcut' | 'mine' | 'fish' | 'harvest';
export interface GatherStart {
  ok: boolean;
  site: string;
}
export const Gatherer: {
  start(mode: GatherMode): Promise<GatherStart>;
};
"#,
        )
        .expect("write Gatherer extension");
    }
}

#[test]
fn extension_accepted_when_typed_module_and_barrel_exist() {
    let dir = std::env::temp_dir().join("compat-dts-ext-ok");
    let _ = fs::remove_dir_all(&dir);
    write_gatherer_extension(&dir, true, true);
    let authored = load_authored_dts_from(&dir).expect("load extension directory");
    let drifts = drift_against_shim(&authored, &script::compat_dts::collect_compat_surface());
    assert!(
        drifts.is_empty(),
        "typed Gatherer module plus barrel export must be accepted: {drifts:?}"
    );
    let gatherer = authored
        .modules
        .iter()
        .find(|module| module.specifier.ends_with("gather/Gatherer.js"))
        .expect("Gatherer module");
    let start = gatherer
        .exports
        .iter()
        .find_map(|export| match export {
            script::compat_dts::CompatExport::Object { name, members } if name == "Gatherer" => {
                members.iter().find(|member| member.name == "start")
            }
            _ => None,
        })
        .expect("Gatherer.start");
    assert!(start.is_async, "Promise-returning extension method");
    assert_eq!(start.params.len(), 1);
    assert_eq!(start.params[0].name, "mode");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn extension_rejected_without_barrel_export() {
    let dir = std::env::temp_dir().join("compat-dts-ext-nobarrel");
    let _ = fs::remove_dir_all(&dir);
    write_gatherer_extension(&dir, false, true);
    let authored = load_authored_dts_from(&dir).expect("load extension directory");
    let drifts = drift_against_shim(&authored, &script::compat_dts::collect_compat_surface());
    assert!(
        drifts
            .iter()
            .any(|drift| drift.contains("Gatherer") && drift.contains("barrel")),
        "missing barrel export must fail: {drifts:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn extension_rejected_without_module_file() {
    let dir = std::env::temp_dir().join("compat-dts-ext-nofile");
    let _ = fs::remove_dir_all(&dir);
    write_gatherer_extension(&dir, true, false);
    let authored = load_authored_dts_from(&dir).expect("load extension directory");
    let has_gatherer = authored
        .modules
        .iter()
        .any(|module| module.specifier.ends_with("gather/Gatherer.js"));
    assert!(
        !has_gatherer,
        "barrel-only Gatherer without a module file must not invent a module"
    );
    let _ = fs::remove_dir_all(&dir);
}
