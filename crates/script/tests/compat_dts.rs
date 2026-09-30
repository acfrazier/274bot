// Compat (JS API v1) declarations — generated declarations gated against the live shim.
use script::compat_dts::{
    check_compat_dts_drift, collect_compat_surface_from, compat_dts_path, drift_against_shim,
    load_authored_dts, load_authored_dts_from, shim_source_pairs,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
/// Rebuild the live shim surface after changing exactly one real shim source.
fn drift_for_shim_replacement(suffix: &str, before: &str, after: &str) -> Vec<String> {
    let mut sources = shim_source_pairs();
    let (specifier, source) = sources
        .iter_mut()
        .find(|(specifier, _)| specifier.ends_with(suffix))
        .unwrap_or_else(|| panic!("no shim source ending in {suffix}"));
    assert_eq!(
        source.matches(before).count(),
        1,
        "expected exactly one `{before}` in {specifier}"
    );
    *source = source.replacen(before, after, 1);
    let sources: Vec<_> = sources
        .iter()
        .map(|(specifier, source)| (specifier.as_str(), source.as_str()))
        .collect();
    let shim = collect_compat_surface_from(&sources);
    drift_against_shim(
        &load_authored_dts().expect("generated compat declarations"),
        &shim,
    )
}

#[test]
fn drift_gate_rejects_async_to_sync_bank_member_mutation() {
    let drifts = drift_for_shim_replacement(
        "/api/bank/Bank.js",
        "async depositAllExcept(",
        "depositAllExcept(",
    );
    assert!(
        drifts
            .iter()
            .any(|drift| drift.contains("Bank.depositAllExcept async-ness")),
        "a synchronous Bank.depositAllExcept must not satisfy a Promise declaration: {drifts:?}"
    );
}

#[test]
fn drift_gate_rejects_async_to_sync_function_mutation() {
    let drifts = drift_for_shim_replacement(
        "/api/bank/BankLocations.js",
        "export async function nearestBankReachable(",
        "export function nearestBankReachable(",
    );
    assert!(
        drifts
            .iter()
            .any(|drift| drift.contains("nearestBankReachable async-ness")),
        "a synchronous nearestBankReachable must not satisfy a Promise declaration: {drifts:?}"
    );
}

#[test]
fn drift_gate_accepts_sync_or_async_bot_loop_contract() {
    let authored = load_authored_dts().expect("generated compat declarations");
    let shim = script::compat_dts::collect_compat_surface();
    let drifts = drift_against_shim(&authored, &shim);
    assert!(
        !drifts.iter().any(|drift| drift.contains("LoopingBot.loop async-ness")),
        "LoopingBot.loop legitimately permits synchronous or Promise results: {drifts:?}"
    );
    let loop_decl = authored
        .modules
        .iter()
        .flat_map(|module| module.exports.iter())
        .find_map(|export| match export {
            script::compat_dts::CompatExport::Class { name, members, .. } if name == "LoopingBot" => {
                members.iter().find(|member| member.name == "loop")
            }
            _ => None,
        })
        .expect("authored LoopingBot.loop declaration");
    assert!(loop_decl.is_async, "declaration union includes Promise");
    assert!(
        loop_decl.allows_non_promise,
        "declaration union also includes synchronous results"
    );
}

#[test]
fn authored_tree_preserves_header_and_sibling_imports() {
    let dir = unique_temp_path("tree");
    script::compat_dts::write_compat_dts_tree(&dir).expect("write declaration tree");
    let index = fs::read_to_string(compat_dts_path()).expect("read generated declarations");
    let header_end = index.find("declare module").expect("ambient module header");
    let header = &index[..header_end];
    let banking =
        fs::read_to_string(dir.join("api/bank/Banking.d.ts")).expect("read Banking tree module");
    assert!(
        header.contains("MIT License") && banking.starts_with(header),
        "declaration-tree files retain the generated MIT preamble"
    );
    assert!(
        banking.contains("from \"./BankLocations.js\"")
            && banking.contains("from \"./bankRules.js\""),
        "same-directory declaration imports need explicit ./ specifiers:\n{banking}"
    );
    fs::remove_dir_all(&dir).expect("remove generated declaration tree");
}

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

/// Exercise an extension in an isolated copy: add a runtime barrel re-export
/// and typed sidecar, regenerate, and verify freshness plus the Rust drift gate.
#[test]
#[ignore = "requires npm, node, and RS2B0T set to the frozen rs2b0t source"]
fn scratch_barrel_sidecar_extension_passes_freshness_and_drift_gates() {
    let rs2b0t = std::env::var_os("RS2B0T")
        .expect("set RS2B0T to the frozen rs2b0t 00d39a17e0 source");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = unique_temp_path("runtime-extension");
    let crate_copy = scratch.join("crate");
    let compat_js = crate_copy.join("compat-js");
    let shim_dir = crate_copy.join("src/shim");
    copy_tree(&manifest.join("compat-js"), &compat_js).expect("copy compat-js emitter inputs");
    copy_tree(&manifest.join("src/shim"), &shim_dir).expect("copy shim barrel and modules");

    let barrel_path = shim_dir.join("declared_surface.js");
    let mut barrel = fs::read_to_string(&barrel_path).expect("read isolated runtime barrel");
    assert!(
        !barrel.contains("export { Gatherer }"),
        "the temporary extension must not already be present in the runtime barrel"
    );
    barrel.push_str("\nexport { Gatherer } from '../../api/gather/Gatherer.js';\n");
    fs::write(&barrel_path, barrel).expect("add isolated runtime barrel re-export");
    let sidecar_dir = compat_js.join("api/gather");
    fs::create_dir_all(&sidecar_dir).expect("create Gatherer sidecar directory");
    fs::write(
        sidecar_dir.join("Gatherer.d.ts"),
        r#"/** Isolated O-SCRIPT-API extension proof. */
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
    .expect("write isolated Gatherer sidecar");

    let install = Command::new("npm")
        .arg("ci")
        .current_dir(&compat_js)
        .output()
        .expect("spawn npm ci for isolated generator");
    assert!(
        install.status.success(),
        "isolated npm ci failed:\n{}\n{}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr)
    );

    let generate = Command::new("node")
        .arg("generate.cjs")
        .env("RS2B0T", &rs2b0t)
        .current_dir(&compat_js)
        .output()
        .expect("spawn frozen declaration generator");
    assert!(
        generate.status.success(),
        "isolated generator failed:\n{}\n{}",
        String::from_utf8_lossy(&generate.stdout),
        String::from_utf8_lossy(&generate.stderr)
    );
    let freshness = Command::new("node")
        .args(["generate.cjs", "--check"])
        .env("RS2B0T", &rs2b0t)
        .current_dir(&compat_js)
        .output()
        .expect("spawn isolated generator freshness check");
    assert!(
        freshness.status.success(),
        "isolated generator --check failed:\n{}\n{}",
        String::from_utf8_lossy(&freshness.stdout),
        String::from_utf8_lossy(&freshness.stderr)
    );

    let authored = load_authored_dts_from(&compat_js).expect("load generated extension surface");
    let gatherer = authored
        .modules
        .iter()
        .find(|module| module.specifier.ends_with("api/gather/Gatherer.js"))
        .expect("load authored Gatherer sidecar");
    assert!(gatherer.extension_file, "Gatherer comes from the typed sidecar");
    let drifts = drift_against_shim(
        &authored,
        &script::compat_dts::collect_compat_surface(),
    );
    assert!(
        drifts.is_empty(),
        "rendered runtime barrel plus typed sidecar must satisfy the live drift gate: {drifts:?}"
    );

    let wrong_revision = scratch.join("wrong-rs2b0t");
    fs::create_dir_all(&wrong_revision).expect("create wrong-revision repository");
    let init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&wrong_revision)
        .output()
        .expect("spawn git init for wrong-revision probe");
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    fs::write(wrong_revision.join("revision.txt"), "not the frozen source\n")
        .expect("write wrong-revision fixture");
    let add = Command::new("git")
        .args(["add", "revision.txt"])
        .current_dir(&wrong_revision)
        .output()
        .expect("spawn git add for wrong-revision probe");
    assert!(
        add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Compat DTS Test",
            "-c",
            "user.email=compat-dts@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "wrong frozen source",
        ])
        .current_dir(&wrong_revision)
        .output()
        .expect("spawn git commit for wrong-revision probe");
    assert!(
        commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
    let actual_wrong_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&wrong_revision)
        .output()
        .expect("read wrong-revision commit");
    assert!(actual_wrong_commit.status.success());
    let actual_wrong_commit = String::from_utf8_lossy(&actual_wrong_commit.stdout)
        .trim()
        .to_string();
    assert_ne!(
        actual_wrong_commit,
        "00d39a17e056df6c5e461f3f2cfd3598ff9720b6"
    );
    let rejected = Command::new("node")
        .arg("generate.cjs")
        .env("RS2B0T", &wrong_revision)
        .current_dir(&compat_js)
        .output()
        .expect("spawn wrong-revision rejection probe");
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(!rejected.status.success(), "wrong revision unexpectedly accepted");
    assert!(
        stderr.contains(
            "RS2B0T revision check failed: expected 00d39a17e0 (00d39a17e056df6c5e461f3f2cfd3598ff9720b6)"
        ),
        "wrong-revision failure must have the stable prefix: {stderr}"
    );
    assert!(
        stderr.contains(&format!("git HEAD is {actual_wrong_commit}")),
        "wrong-revision diagnostic must report the actual commit {actual_wrong_commit}: {stderr}"
    );
    fs::remove_dir_all(&scratch).expect("remove isolated generator copy");
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

/// Compile an actual per-module consumer so same-directory `./` imports resolve.
#[test]
#[ignore = "requires npx and TypeScript 5.8.3"]
fn tsc_generated_bank_tree_resolves_sibling_imports() {
    let dir = unique_temp_path("bank-tree-consumer");
    script::compat_dts::write_compat_dts_tree(&dir).expect("write declaration tree");
    let consumer = dir.join("consumer.ts");
    fs::write(
        &consumer,
        r#"
import { Banking, PERIODIC_BANK_SETTINGS, depositAllExcept } from "./api/bank/Banking.js";

void Banking.open();
void depositAllExcept(["junk"]);
const settings: typeof PERIODIC_BANK_SETTINGS = PERIODIC_BANK_SETTINGS;
void settings;
// @ts-expect-error periodic bank settings are structured data, not a number
const invalid: number = PERIODIC_BANK_SETTINGS;
void invalid;
"#,
    )
    .expect("write bank-tree consumer");
    let output = tsc_command()
        .args([
            "--noEmit",
            "--strict",
            "--target",
            "ESNext",
            "--module",
            "ESNext",
            "--moduleResolution",
            "Bundler",
            "--skipLibCheck",
            "false",
        ])
        .arg(&consumer)
        .output()
        .unwrap_or_else(|e| panic!("tsc generated-tree consumer failed to spawn: {e}"));
    assert!(
        output.status.success(),
        "generated-tree consumer failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(&dir).expect("remove generated-tree consumer");
}

fn tsc_command() -> Command {
    let mut npx = Command::new("npx");
    npx.args(["-p", "typescript@5.8.3", "--yes", "tsc"]);
    npx
}

fn unique_temp_path(label: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "compat-dts-{label}-{}-{timestamp}",
        std::process::id()
    ))
}

fn copy_tree(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_string_lossy().as_ref(), "node_modules" | ".git") {
            continue;
        }
        let from = entry.path();
        let to = target.join(name);
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(from, to)?;
        }
    }
    Ok(())
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
