//! Explicit PREPARE vs RUN-PREPARED fixture split for production harnesses.
//!
//! Dev `--live` scenarios seed with cheats (mainland hop, tele, stats, items).
//! Under `NODE_PRODUCTION` those cheats are rejected (staff 0), so the same
//! scenario hangs forever. Split the path:
//!
//! - **Prepare** (offline): write a real server-native `.sav` via the engine's
//!   own `Player.save()` + `PlayerLoading` roundtrip helper under
//!   `tools/harness/`. No live engine, no setup cheats, no staff.
//! - **Run-prepared** (production engine): reuse the exact same username /
//!   password / save identity (no fresh random suffix), zero setup cheats
//!   including mainland hop, fail-closed prerequisite checks, then
//!   StartScript + script progress only.
//!
//! Existing default live modes are unchanged.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{Proof, Scenario, Seed, Step, StepKind, Wait};

/// Which fixture path the panel/host should apply to a resolved scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureMode {
    /// Unchanged scenario (existing `--live script_*` behavior).
    Default,
    /// Offline server-native save write (no live boot).
    Prepare,
    /// Cheat-free production replay of a prior prepare identity.
    RunPrepared,
}

/// One prepared account slot (identity + save provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureAccount {
    pub username: String,
    /// Login password for the isolated test account (not a real-user secret).
    pub password: String,
    /// Absolute path to the offline-written `.sav`.
    pub sav_path: String,
    pub sav_sha256: String,
    pub sav_bytes: u64,
}

/// Durable identity written after a successful offline prepare and required for run-prepared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureIdentity {
    /// Schema version; unknown versions fail closed.
    pub version: u32,
    /// Registry scenario name (`thiever`, not `script_thiever`).
    pub scenario: String,
    /// Fixture preset id passed to the offline writer (`thiever`).
    pub fixture: String,
    /// Engine profile directory name (`main`).
    pub profile: String,
    /// Exact login accounts; run-prepared must reuse these (no re-mint).
    pub accounts: Vec<FixtureAccount>,
    /// Passphrase for the ephemeral vault blob that held the profiles.
    /// Isolated harness only — not a production vault secret.
    pub vault_passphrase: String,
    /// Wall-clock prepare completion (ms since unix epoch).
    pub prepared_at_unix_ms: u64,
    /// Optional engine git head recorded at prepare time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_git_head: Option<String>,
    /// Server root used to write the save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_root: Option<String>,
}

impl FixtureIdentity {
    pub const VERSION: u32 = 2;

    pub fn usernames(&self) -> Vec<String> {
        self.accounts.iter().map(|a| a.username.clone()).collect()
    }

    pub fn passwords(&self) -> Vec<String> {
        self.accounts.iter().map(|a| a.password.clone()).collect()
    }

    pub fn entries(&self) -> Vec<(String, String)> {
        self.accounts
            .iter()
            .map(|a| (a.username.clone(), a.password.clone()))
            .collect()
    }

    /// Fail-closed validation before a run-prepared boot.
    pub fn validate_for(&self, scenario: &str, profile_count: usize) -> Result<(), String> {
        if self.version != Self::VERSION {
            return Err(format!(
                "fixture identity version {} unsupported (need {})",
                self.version,
                Self::VERSION
            ));
        }
        if self.scenario != scenario {
            return Err(format!(
                "fixture identity scenario {:?} does not match requested {scenario:?}",
                self.scenario
            ));
        }
        if self.accounts.len() != profile_count {
            return Err(format!(
                "fixture identity has {} account(s); scenario seed needs {profile_count}",
                self.accounts.len()
            ));
        }
        if self.accounts.is_empty() {
            return Err("fixture identity requires at least one account".into());
        }
        if self.vault_passphrase.is_empty() {
            return Err("fixture identity has empty vault passphrase".into());
        }
        for (i, a) in self.accounts.iter().enumerate() {
            if a.username.trim().is_empty() || a.password.is_empty() {
                return Err(format!(
                    "fixture identity account {i} has empty credentials"
                ));
            }
            if a.sav_path.trim().is_empty() || a.sav_sha256.trim().is_empty() {
                return Err(format!(
                    "fixture identity account {i} missing sav path/digest"
                ));
            }
            let sav = Path::new(&a.sav_path);
            if !sav.is_file() {
                return Err(format!(
                    "fixture identity account {i} sav missing: {}",
                    a.sav_path
                ));
            }
            let meta = fs::metadata(sav).map_err(|e| format!("stat {}: {e}", a.sav_path))?;
            if meta.len() != a.sav_bytes {
                return Err(format!(
                    "fixture identity account {i} sav size {} != recorded {}",
                    meta.len(),
                    a.sav_bytes
                ));
            }
            let bytes = fs::read(sav).map_err(|e| format!("read {}: {e}", a.sav_path))?;
            let digest = sha256_hex(&bytes);
            if digest != a.sav_sha256 {
                return Err(format!(
                    "fixture identity account {i} sav sha256 mismatch (file changed since prepare)"
                ));
            }
        }
        Ok(())
    }

    pub fn write_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create fixture dir {}: {e}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize fixture identity: {e}"))?;
        fs::write(path, format!("{json}\n"))
            .map_err(|e| format!("write fixture identity {}: {e}", path.display()))
    }

    pub fn read_from(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("read fixture identity {}: {e}", path.display()))?;
        let identity: Self = serde_json::from_str(&raw)
            .map_err(|e| format!("parse fixture identity {}: {e}", path.display()))?;
        if identity.version != Self::VERSION {
            return Err(format!(
                "fixture identity {}: version {} unsupported (need {})",
                path.display(),
                identity.version,
                Self::VERSION
            ));
        }
        Ok(identity)
    }

    /// Copy prepared `.sav` files into an engine `data/players/<profile>/` tree.
    /// Refuses to overwrite an existing file unless `overwrite` is set.
    pub fn install_into_players_dir(
        &self,
        players_root: &Path,
        overwrite: bool,
    ) -> Result<Vec<PathBuf>, String> {
        let dir = players_root.join(&self.profile);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("create players dir {}: {e}", dir.display()))?;
        let mut out = Vec::new();
        for a in &self.accounts {
            let dest = dir.join(format!("{}.sav", a.username));
            if dest.exists() && !overwrite {
                return Err(format!(
                    "refusing to overwrite existing player save {} (pass overwrite)",
                    dest.display()
                ));
            }
            fs::copy(&a.sav_path, &dest)
                .map_err(|e| format!("copy {} -> {}: {e}", a.sav_path, dest.display()))?;
            // Re-check digest after copy.
            let bytes = fs::read(&dest).map_err(|e| format!("read {}: {e}", dest.display()))?;
            let digest = sha256_hex(&bytes);
            if digest != a.sav_sha256 {
                return Err(format!(
                    "installed sav digest mismatch for {} after copy",
                    a.username
                ));
            }
            out.push(dest);
        }
        Ok(out)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Minimal SHA-256 without new crate deps: shell out is too heavy in hot
    // paths; use a tiny pure implementation via `std` is unavailable — prefer
    // the same digest the helper wrote by re-hashing with a small inline.
    // scenario already depends only on serde; implement via `openssl`? No.
    // Use a compact pure Rust SHA-256 (public domain style) below.
    sha256::digest(bytes)
}

/// Tiny SHA-256 so fixture digests do not pull a new workspace crate.
mod sha256 {
    pub fn digest(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        let out = h.finalize();
        out.iter().map(|b| format!("{b:02x}")).collect()
    }

    // Compact implementation (FIPS 180-4).
    struct Sha256 {
        h: [u32; 8],
        len: u64,
        buf: [u8; 64],
        buf_len: usize,
    }

    impl Sha256 {
        fn new() -> Self {
            Self {
                h: [
                    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                    0x1f83d9ab, 0x5be0cd19,
                ],
                len: 0,
                buf: [0; 64],
                buf_len: 0,
            }
        }

        fn update(&mut self, mut data: &[u8]) {
            self.len = self.len.wrapping_add((data.len() as u64).wrapping_mul(8));
            if self.buf_len > 0 {
                let need = 64 - self.buf_len;
                let take = need.min(data.len());
                self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
                self.buf_len += take;
                data = &data[take..];
                if self.buf_len == 64 {
                    let block = self.buf;
                    self.compress(&block);
                    self.buf_len = 0;
                }
            }
            while data.len() >= 64 {
                let mut block = [0u8; 64];
                block.copy_from_slice(&data[..64]);
                self.compress(&block);
                data = &data[64..];
            }
            if !data.is_empty() {
                self.buf[..data.len()].copy_from_slice(data);
                self.buf_len = data.len();
            }
        }

        fn finalize(mut self) -> [u8; 32] {
            let mut block = [0u8; 64];
            block[..self.buf_len].copy_from_slice(&self.buf[..self.buf_len]);
            block[self.buf_len] = 0x80;
            if self.buf_len >= 56 {
                self.compress(&block);
                block = [0u8; 64];
            }
            block[56..].copy_from_slice(&self.len.to_be_bytes());
            self.compress(&block);
            let mut out = [0u8; 32];
            for (i, v) in self.h.iter().enumerate() {
                out[i * 4..(i + 1) * 4].copy_from_slice(&v.to_be_bytes());
            }
            out
        }

        fn compress(&mut self, block: &[u8; 64]) {
            const K: [u32; 64] = [
                0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
                0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
                0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
                0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
                0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
                0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
                0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
                0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
                0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
                0xc67178f2,
            ];
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    block[i * 4],
                    block[i * 4 + 1],
                    block[i * 4 + 2],
                    block[i * 4 + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let mut a = self.h[0];
            let mut b = self.h[1];
            let mut c = self.h[2];
            let mut d = self.h[3];
            let mut e = self.h[4];
            let mut f = self.h[5];
            let mut g = self.h[6];
            let mut h = self.h[7];
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            self.h[0] = self.h[0].wrapping_add(a);
            self.h[1] = self.h[1].wrapping_add(b);
            self.h[2] = self.h[2].wrapping_add(c);
            self.h[3] = self.h[3].wrapping_add(d);
            self.h[4] = self.h[4].wrapping_add(e);
            self.h[5] = self.h[5].wrapping_add(f);
            self.h[6] = self.h[6].wrapping_add(g);
            self.h[7] = self.h[7].wrapping_add(h);
        }
    }
}

/// Default on-disk path for a prepare receipt (`~/.274bot/fixtures/<scenario>.json`).
pub fn default_fixture_path(scenario: &str) -> PathBuf {
    bot_home().join("fixtures").join(format!("{scenario}.json"))
}

/// Default directory for offline `.sav` files for a scenario.
pub fn default_fixture_sav_dir(scenario: &str) -> PathBuf {
    bot_home().join("fixtures").join(scenario)
}

/// `~/.274bot` (or `$HOME/.274bot`).
pub fn bot_home() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".274bot");
    }
    std::env::temp_dir().join("274bot-fixtures-home")
}

/// Locate `tools/harness/run_write_player_fixture.sh` from this crate or env.
pub fn harness_writer_script() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("BOT_FIXTURE_WRITER") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("BOT_FIXTURE_WRITER not a file: {}", path.display()));
    }
    // crates/scenario -> repo root
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest
        .join("../..")
        .join("tools/harness/run_write_player_fixture.sh");
    let candidate = candidate.canonicalize().unwrap_or(candidate);
    if candidate.is_file() {
        return Ok(candidate);
    }
    Err(format!(
        "offline fixture writer not found at {} (set BOT_FIXTURE_WRITER)",
        candidate.display()
    ))
}

/// Offline writer preset id for a registered scenario name.
/// `thiever` keeps its existing preset. The two v2 File scenarios share
/// `bone_burier_v2`. The owned-plant lifecycle reuses the prepared mainland
/// `thiever` world state; its macro-event injection remains post-Start.
/// Unknown names stay fail-closed.
pub fn fixture_preset_for(scenario: &str) -> Result<&'static str, String> {
    match scenario {
        "thiever" | "strange_plant_owned" => Ok("thiever"),
        "bone_burier_v2_ts" | "bone_burier_v2_js" => Ok("bone_burier_v2"),
        other => Err(format!(
            "offline prepare has no server-native preset for {other} yet (known: thiever, strange_plant_owned, bone_burier_v2_ts, bone_burier_v2_js)"
        )),
    }
}

/// Options for an offline prepare of one scenario.
#[derive(Debug, Clone)]
pub struct OfflinePrepareOpts {
    pub scenario: String,
    pub fixture_preset: String,
    pub profile: String,
    pub server_root: PathBuf,
    pub identity_path: PathBuf,
    pub sav_dir: PathBuf,
    pub usernames: Vec<String>,
    pub passwords: Vec<String>,
    pub vault_passphrase: String,
    pub overwrite: bool,
}

/// Run the offline server-native writer for each account and write the identity receipt.
pub fn prepare_offline_fixture(opts: OfflinePrepareOpts) -> Result<FixtureIdentity, String> {
    if opts.usernames.is_empty() {
        return Err("offline prepare needs at least one username".into());
    }
    if opts.usernames.len() != opts.passwords.len() {
        return Err("username/password length mismatch".into());
    }
    if opts.vault_passphrase.is_empty() {
        return Err("vault passphrase required for identity vault mint".into());
    }
    let writer = harness_writer_script()?;
    fs::create_dir_all(&opts.sav_dir)
        .map_err(|e| format!("create sav dir {}: {e}", opts.sav_dir.display()))?;

    let mut accounts = Vec::with_capacity(opts.usernames.len());
    let mut engine_git_head = None;
    let mut server_root_recorded = None;

    for (user, pass) in opts.usernames.iter().zip(opts.passwords.iter()) {
        let sav_path = opts.sav_dir.join(format!("{user}.sav"));
        let receipt_path = opts.sav_dir.join(format!("{user}.receipt.json"));
        if sav_path.exists() && !opts.overwrite {
            return Err(format!(
                "refusing to overwrite existing sav {} without overwrite",
                sav_path.display()
            ));
        }
        let mut cmd = Command::new(&writer);
        cmd.arg("--username")
            .arg(user)
            .arg("--output")
            .arg(&sav_path)
            .arg("--receipt")
            .arg(&receipt_path)
            .arg("--fixture")
            .arg(&opts.fixture_preset)
            .arg("--profile")
            .arg(&opts.profile)
            .arg("--server-root")
            .arg(&opts.server_root);
        if opts.overwrite {
            cmd.arg("--overwrite");
        }
        let output = cmd
            .output()
            .map_err(|e| format!("spawn fixture writer {}: {e}", writer.display()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!(
                "fixture writer failed for {user} (status {:?}):\n{stderr}\n{stdout}",
                output.status.code()
            ));
        }
        let receipt_raw = fs::read_to_string(&receipt_path)
            .map_err(|e| format!("read receipt {}: {e}", receipt_path.display()))?;
        let receipt: serde_json::Value = serde_json::from_str(&receipt_raw)
            .map_err(|e| format!("parse receipt {}: {e}", receipt_path.display()))?;
        let sha = receipt
            .get("sav_sha256")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("receipt missing sav_sha256: {}", receipt_path.display()))?
            .to_string();
        let bytes = receipt
            .get("sav_bytes")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("receipt missing sav_bytes: {}", receipt_path.display()))?;
        if let Some(prov) = receipt.get("provenance") {
            if engine_git_head.is_none() {
                engine_git_head = prov
                    .get("engine_git_head")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            if server_root_recorded.is_none() {
                server_root_recorded = prov
                    .get("server_root")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
        }
        // pass is retained only for isolated test login identity
        let _ = pass;
        accounts.push(FixtureAccount {
            username: user.to_lowercase(),
            password: pass.clone(),
            sav_path: sav_path
                .canonicalize()
                .unwrap_or(sav_path)
                .display()
                .to_string(),
            sav_sha256: sha,
            sav_bytes: bytes,
        });
    }

    let prepared_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let identity = FixtureIdentity {
        version: FixtureIdentity::VERSION,
        scenario: opts.scenario,
        fixture: opts.fixture_preset,
        profile: opts.profile,
        accounts,
        vault_passphrase: opts.vault_passphrase,
        prepared_at_unix_ms,
        engine_git_head,
        server_root: server_root_recorded.or_else(|| Some(opts.server_root.display().to_string())),
    };
    identity.write_to(&opts.identity_path)?;
    Ok(identity)
}

/// Rewrite `scenario` for [`FixtureMode::RunPrepared`].
/// [`FixtureMode::Default`] returns the scenario unchanged.
/// [`FixtureMode::Prepare`] is offline-only and must not be applied to a live scenario.
pub fn apply_fixture_mode(scenario: Scenario, mode: FixtureMode) -> Result<Scenario, String> {
    match mode {
        FixtureMode::Default => Ok(scenario),
        FixtureMode::Prepare => Err(format!(
            "scenario {:?}: FixtureMode::Prepare is offline-only (use prepare_offline_fixture / --prepare-fixture); refusing live cheat prepare",
            scenario.name
        )),
        FixtureMode::RunPrepared => as_run_prepared(scenario),
    }
}

/// Collect durable world proofs a run-prepared boot must observe before Start.
pub fn fixture_prereqs_of(scenario: &Scenario) -> Vec<Proof> {
    if let Some(rows) = scenario.settings.fixture_prereqs {
        return rows.to_vec();
    }
    let start = start_script_index(scenario).unwrap_or(scenario.steps.len());
    scenario.steps[..start]
        .iter()
        .filter_map(|step| match step.wait.arm {
            Proof::Arrived { .. }
            | Proof::ArrivedNear { .. }
            | Proof::Stat { .. }
            | Proof::Item { .. }
            | Proof::ItemId { .. }
            | Proof::EquipmentId { .. }
            | Proof::BankItem { .. }
            | Proof::BankItemId { .. }
            | Proof::SideTabAvailable { .. }
            | Proof::Varp { .. }
            | Proof::VarpExact { .. }
            | Proof::TutorialClosed => Some(step.wait.arm),
            _ => None,
        })
        .collect()
}

fn start_script_index(scenario: &Scenario) -> Option<usize> {
    scenario
        .steps
        .iter()
        .position(|s| matches!(s.kind, StepKind::StartScript))
}

fn ack_steps(prereqs: &[Proof]) -> Vec<Step> {
    prereqs
        .iter()
        .copied()
        .map(|arm| Step {
            name: "acknowledge prepared fixture state",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm,
                budget_ticks: 200,
            },
        })
        .collect()
}

/// Production run-prepared: no mainland/setup cheats; fail-closed prereq gates;
/// then StartScript and the original post-start progress steps only.
pub fn as_run_prepared(mut scenario: Scenario) -> Result<Scenario, String> {
    let start = start_script_index(&scenario).ok_or_else(|| {
        format!(
            "scenario {:?} has no StartScript; run-prepared needs a catalog script scenario",
            scenario.name
        )
    })?;
    let prereqs = fixture_prereqs_of(&scenario);
    if prereqs.is_empty() {
        return Err(format!(
            "scenario {:?} has no fixture prerequisites to gate run-prepared",
            scenario.name
        ));
    }
    let post = scenario.steps.split_off(start);
    scenario.steps = ack_steps(&prereqs);
    scenario.steps.extend(post);
    scenario.seed = Seed {
        profiles: scenario.seed.profiles,
        mainland: false,
    };
    scenario.settings.require_mainland_base = false;
    scenario.settings.nav.engine_speed_ms = None;
    scenario.settings.sustains.clear();
    // Script progress proof stays as the scenario's original terminal proof.
    Ok(scenario)
}

/// Whether any remaining step still issues a driver cheat / mainland-style seed.
pub fn run_prepared_has_setup_cheats(scenario: &Scenario) -> bool {
    if scenario.seed.mainland || scenario.settings.require_mainland_base {
        return true;
    }
    if scenario.settings.nav.engine_speed_ms.is_some() {
        return true;
    }
    if !scenario.settings.sustains.is_empty() {
        return true;
    }
    let start = start_script_index(scenario).unwrap_or(scenario.steps.len());
    scenario.steps[..start].iter().any(|step| match &step.kind {
        // After as_run_prepared, pre-Start steps are no-op ack observes only.
        StepKind::Perform { .. } => !step.name.starts_with("acknowledge prepared"),
        StepKind::Repeat { .. } | StepKind::DrainDialogs { .. } | StepKind::Relog => true,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::get;
    use std::time::Duration;

    fn thiever() -> Scenario {
        get("thiever").expect("thiever registered")
    }

    #[test]
    fn prepare_mode_refuses_live_transform() {
        match apply_fixture_mode(thiever(), FixtureMode::Prepare) {
            Err(err) => assert!(
                err.contains("offline-only"),
                "prepare must not keep live cheat path: {err}"
            ),
            Ok(_) => panic!("prepare must refuse live transform"),
        }
    }

    #[test]
    fn run_prepared_strips_mainland_and_setup_keeps_start() {
        let run = as_run_prepared(thiever()).expect("run-prepared");
        assert!(!run.seed.mainland);
        assert!(!run.settings.require_mainland_base);
        assert!(run.settings.nav.engine_speed_ms.is_none());
        assert!(run.settings.sustains.is_empty());
        assert!(
            run.steps
                .iter()
                .any(|s| matches!(s.kind, StepKind::StartScript)),
            "run-prepared still starts the catalog card"
        );
        assert_eq!(run.settings.start_script, Some("Thiever"));
        assert!(
            !run.steps
                .iter()
                .any(|s| matches!(s.kind, StepKind::DrainDialogs { .. } | StepKind::Relog)),
            "run-prepared must not relog or drain setup dialogs"
        );
        let start = run
            .steps
            .iter()
            .position(|s| matches!(s.kind, StepKind::StartScript))
            .unwrap();
        assert!(
            run.steps[..start]
                .iter()
                .all(|s| matches!(s.kind, StepKind::Perform { .. })),
            "pre-Start steps are fail-closed prereq observes only"
        );
        assert!(
            matches!(run.proof, Proof::StatXpGain { id: 17, min: 1 }),
            "script progress remains the terminal proof"
        );
        assert!(
            !run_prepared_has_setup_cheats(&run),
            "run-prepared must report zero setup cheats"
        );
        assert_eq!(
            run.settings.fixture_loadouts.map(|rows| rows[0].name),
            Some("Memory food"),
            "run-prepared keeps the harness-owned loadout for catalog Start"
        );
    }

    #[test]
    fn default_mode_is_identity() {
        let s = thiever();
        let name = s.name;
        let out = apply_fixture_mode(s, FixtureMode::Default).unwrap();
        assert_eq!(out.name, name);
        assert!(out.seed.mainland);
    }

    #[test]
    fn identity_roundtrip_and_guards() {
        let dir = std::env::temp_dir().join(format!(
            "274bot-fixture-id-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let sav = dir.join("liveabc_0.sav");
        let payload = b"fake-sav-bytes-for-identity-test";
        fs::write(&sav, payload).unwrap();
        let digest = sha256_hex(payload);
        let path = dir.join("thiever.json");
        let id = FixtureIdentity {
            version: FixtureIdentity::VERSION,
            scenario: "thiever".into(),
            fixture: "thiever".into(),
            profile: "main".into(),
            accounts: vec![FixtureAccount {
                username: "liveabc_0".into(),
                password: "secret".into(),
                sav_path: sav.display().to_string(),
                sav_sha256: digest.clone(),
                sav_bytes: payload.len() as u64,
            }],
            vault_passphrase: "vault-pass".into(),
            prepared_at_unix_ms: 1,
            engine_git_head: None,
            server_root: None,
        };
        id.write_to(&path).unwrap();
        let loaded = FixtureIdentity::read_from(&path).unwrap();
        assert_eq!(loaded.scenario, "thiever");
        assert_eq!(loaded.usernames(), vec!["liveabc_0".to_string()]);
        loaded.validate_for("thiever", 1).unwrap();
        assert!(loaded
            .validate_for("bone_burier", 1)
            .unwrap_err()
            .contains("does not match"));
        assert!(loaded
            .validate_for("thiever", 2)
            .unwrap_err()
            .contains("account"));
        assert!(FixtureIdentity::read_from(Path::new("/no/such/fixture.json")).is_err());

        let players = dir.join("players");
        let installed = loaded.install_into_players_dir(&players, false).unwrap();
        assert_eq!(installed.len(), 1);
        assert!(players.join("main").join("liveabc_0.sav").is_file());
        assert!(loaded
            .install_into_players_dir(&players, false)
            .unwrap_err()
            .contains("overwrite"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_rejects_scenarios_without_start_script() {
        let mut s = thiever();
        s.steps
            .retain(|st| !matches!(st.kind, StepKind::StartScript));
        s.settings.start_script = None;
        assert!(as_run_prepared(s).is_err());
    }

    #[test]
    fn fixture_preset_allowlist_keeps_thiever_and_adds_v2() {
        assert_eq!(fixture_preset_for("thiever").unwrap(), "thiever");
        assert_eq!(
            fixture_preset_for("strange_plant_owned").unwrap(),
            "thiever"
        );
        let plant = as_run_prepared(get("strange_plant_owned").unwrap()).unwrap();
        assert_eq!(plant.settings.start_script, Some("TradeBot"));
        assert!(!run_prepared_has_setup_cheats(&plant));
        assert!(
            plant
                .steps
                .iter()
                .any(|step| step.name == "spawn the upstream owned Strange Plant"),
            "run-prepared must retain the post-Start macro-event injection"
        );
        assert_eq!(
            fixture_preset_for("bone_burier_v2_ts").unwrap(),
            "bone_burier_v2"
        );
        assert_eq!(
            fixture_preset_for("bone_burier_v2_js").unwrap(),
            "bone_burier_v2"
        );
        let err = fixture_preset_for("bone_burier").unwrap_err();
        assert!(err.contains("bone_burier"), "{err}");
        assert!(err.contains("thiever"), "{err}");
        assert!(fixture_preset_for("script_trade").is_err());
        assert!(fixture_preset_for("nope").is_err());
    }

    #[test]
    fn bone_burier_v2_fixture_prereqs_exclude_bank_contents() {
        for name in ["bone_burier_v2_ts", "bone_burier_v2_js"] {
            let s = get(name).expect(name);
            let prereqs = fixture_prereqs_of(&s);
            assert!(
                prereqs.iter().any(|p| matches!(
                    p,
                    Proof::Item {
                        name: "Bones",
                        count: 5
                    }
                )),
                "{name} carried bones"
            );
            assert!(
                prereqs
                    .iter()
                    .any(|p| matches!(p, Proof::SideTabAvailable { index: 3 })),
                "{name} side tab"
            );
            assert!(
                prereqs.iter().any(|p| matches!(
                    p,
                    Proof::ArrivedNear {
                        x: 3253,
                        z: 3421,
                        level: 0,
                        radius: 8
                    }
                )),
                "{name} Varrock East bank tile"
            );
            assert!(
                !prereqs.iter().any(|p| matches!(
                    p,
                    Proof::BankItem { .. } | Proof::BankItemAtMost { .. } | Proof::BankClosed
                )),
                "{name} must not require a bank row at login"
            );
            let run = as_run_prepared(s).expect("run-prepared");
            assert!(!run.seed.mainland);
            assert!(!run_prepared_has_setup_cheats(&run));
            assert!(run.settings.start_file.is_some());
            assert_eq!(run.settings.start_script, None);
        }
    }

    #[test]
    fn thiever_fixture_prereqs_are_explicit() {
        let s = thiever();
        let prereqs = fixture_prereqs_of(&s);
        assert!(
            prereqs
                .iter()
                .any(|p| matches!(p, Proof::Stat { id: 17, min: 50 })),
            "thieving 50"
        );
        assert!(
            prereqs
                .iter()
                .any(|p| matches!(p, Proof::Stat { id: 3, min: 50 })),
            "hitpoints 50"
        );
        assert!(
            prereqs.iter().any(|p| matches!(
                p,
                Proof::Item {
                    name: "Lobster",
                    count: 10
                }
            )),
            "lobster food"
        );
        assert!(
            prereqs.iter().any(|p| matches!(
                p,
                Proof::ArrivedNear {
                    x: 2661,
                    z: 3306,
                    level: 0,
                    radius: 10
                }
            )),
            "guard stand"
        );
        let _ = Duration::from_secs(1);
    }

    #[test]
    fn offline_prepare_writer_roundtrip_when_engine_present() {
        let eng = std::env::var("BOT_SERVER_ROOT")
            .unwrap_or_else(|_| "/Users/acfrazier/experiments/Server/engine".into());
        let eng = PathBuf::from(eng);
        if !eng.join("data/pack/server/obj.dat").is_file() {
            eprintln!("skip: no engine pack at {}", eng.display());
            return;
        }
        if harness_writer_script().is_err() {
            eprintln!("skip: harness writer missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "274bot-offline-prep-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let identity_path = dir.join("thiever.json");
        let sav_dir = dir.join("saves");
        let identity = prepare_offline_fixture(OfflinePrepareOpts {
            scenario: "thiever".into(),
            fixture_preset: "thiever".into(),
            profile: "main".into(),
            server_root: eng,
            identity_path: identity_path.clone(),
            sav_dir,
            usernames: vec!["offprep01".into()],
            passwords: vec!["offprep-pass".into()],
            vault_passphrase: "vault-offprep".into(),
            overwrite: true,
        })
        .expect("offline prepare");
        identity.validate_for("thiever", 1).unwrap();
        assert_eq!(identity.accounts[0].username, "offprep01");
        assert!(identity.accounts[0].sav_bytes > 0);
        assert_eq!(identity.accounts[0].sav_sha256.len(), 64);
        let players = dir.join("players");
        identity.install_into_players_dir(&players, false).unwrap();
        assert!(players.join("main/offprep01.sav").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_prepare_bone_burier_v2_when_engine_present() {
        let eng = std::env::var("BOT_SERVER_ROOT")
            .unwrap_or_else(|_| "/Users/acfrazier/experiments/Server/engine".into());
        let eng = PathBuf::from(eng);
        if !eng.join("data/pack/server/obj.dat").is_file() {
            eprintln!("skip: no engine pack at {}", eng.display());
            return;
        }
        if harness_writer_script().is_err() {
            eprintln!("skip: harness writer missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "274bot-offline-bone-v2-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let identity = prepare_offline_fixture(OfflinePrepareOpts {
            scenario: "bone_burier_v2_ts".into(),
            fixture_preset: fixture_preset_for("bone_burier_v2_ts")
                .expect("v2 preset")
                .into(),
            profile: "main".into(),
            server_root: eng,
            identity_path: dir.join("bone_burier_v2_ts.json"),
            sav_dir: dir.join("saves"),
            usernames: vec!["offbone01".into()],
            passwords: vec!["offbone-pass".into()],
            vault_passphrase: "vault-offbone".into(),
            overwrite: true,
        })
        .expect("offline prepare v2");
        identity.validate_for("bone_burier_v2_ts", 1).unwrap();
        assert_eq!(identity.fixture, "bone_burier_v2");
        assert!(identity.accounts[0].sav_bytes > 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
