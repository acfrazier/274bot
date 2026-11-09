//! QUEST-WATCH harness drivers (QUEST-WATCH-1029).
//!
//! `live_quest_watch_seed` logs one fixed account in, runs the
//! `quest_watch_seed` or `quest_watch_speed` scenario selected by
//! `QUEST_WATCH_SCENARIO`, and writes a snapshot receipt. It exists
//! because every other quest live runner either mints its own account
//! names or Starts the Quester: the morning panel fixture needs named
//! accounts seeded and logged out, with the quest untouched.
//!
//! `build_quest_watch_home` is offline: it writes the fixture vault
//! (one profile per seeded account, the Quester card assigned with
//! that account's one quest) from a JSON spec, then reopens it and
//! writes a readback receipt. The plain-JSON half of the HOME
//! (`servers.json`, `panel-ui.json`) is written by the harness CLI.
//!
//! Live run (Engine L proof shape; the morning run changes only the
//! ports, engine dir and account):
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qw BOT_GAME_PORT=47594 BOT_HTTP_PORT=3080 \
//! BOT_ENGINE_DIR=<engine-dir> BOT_NAV_PACK=<274bot.navpack> BOT_NAV_BUILD=skip \
//! LIVE_EVIDENCE_DIR=<receipt-dir> QUEST_WATCH_ACCOUNT=<name> QUEST_WATCH_PASSWORD=<pw> \
//! QUEST_WATCH_SCENARIO=quest_watch_seed QUEST_WATCH_SEED=<plan.json> \
//! isohome cargo test -p host-play --features live-harness,test-support --lib \
//!   live_quest_watch_seed -- --ignored --nocapture --test-threads=1
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::snapshot::GameSnapshot;
use host::Pump;
use scenario::{RunnerStatus, ScenarioRunner};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings, ScriptAssignment, Vault};

use super::{run_with_template, ProfileOptions};

struct ThrowawayHome {
    path: PathBuf,
    previous: Option<String>,
}

impl ThrowawayHome {
    fn enter(label: &str) -> Self {
        let previous = std::env::var("HOME").ok();
        let path = std::env::temp_dir().join(format!(
            "274bot-quest-watch-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("create throwaway HOME");
        std::env::set_var("HOME", &path);
        Self { path, previous }
    }
}

impl Drop for ThrowawayHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for quest-watch cells"))
}

/// Keep cache_dir unset so `prepare_template` exercises the genuine cold
/// panel/TUI route inside this run's empty HOME, as the journal cells do.
fn live_options(home: &Path) -> ProfileOptions {
    let port: u16 = required_env("BOT_GAME_PORT")
        .parse()
        .expect("BOT_GAME_PORT is a port number");
    let http_port: u16 = required_env("BOT_HTTP_PORT")
        .parse()
        .expect("BOT_HTTP_PORT is a port number");
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(home.join("vault")),
        unpack_dir: Some(home.join("unpack")),
        nav_pack: std::env::var_os("BOT_NAV_PACK").map(PathBuf::from),
        engine_dir: Some(PathBuf::from(required_env("BOT_ENGINE_DIR"))),
        ..ProfileOptions::default()
    }
}

#[derive(Default)]
struct Observed {
    /// Last ingame scene-2 snapshot facts (stats, tile, quest state).
    last_ingame: Option<Value>,
    /// Last loaded bank totals, by item id.
    bank: Option<BTreeMap<i32, i32>>,
    /// Set once the scenario runner leaves the world after the seed.
    logged_out: bool,
}

struct SeedState {
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    observed: Observed,
}

fn totals(rows: &[api::snapshot::ItemView]) -> BTreeMap<i32, i32> {
    let mut out = BTreeMap::new();
    for row in rows {
        if row.count > 0 {
            *out.entry(row.def.id).or_insert(0) += row.count;
        }
    }
    out
}

fn totals_json(totals: &BTreeMap<i32, i32>) -> Value {
    Value::Object(
        totals
            .iter()
            .map(|(id, count)| (id.to_string(), json!(count)))
            .collect(),
    )
}

impl SeedState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if self.snapshot.ingame() && self.snapshot.scene_state() == 2 {
            let stats: Vec<Value> = self
                .snapshot
                .stats()
                .iter()
                .map(|stat| {
                    json!({
                        "index": stat.index,
                        "base": stat.base,
                        "effective": stat.effective,
                    })
                })
                .collect();
            let quests: Vec<Value> = self
                .snapshot
                .quest_statuses()
                .iter()
                .map(|row| json!({ "name": row.name, "colour": row.colour }))
                .collect();
            let varps: Vec<Value> = self
                .snapshot
                .varps()
                .iter()
                .filter(|row| row.value != 0)
                .map(|row| json!({ "index": row.index, "value": row.value }))
                .collect();
            self.observed.last_ingame = Some(json!({
                "tile": self.snapshot.tile().map(|(x, z, level)| json!([x, z, level])),
                "stats": stats,
                "inventory": totals_json(&totals(self.snapshot.inventory())),
                "equipment": self.snapshot.equipment().iter()
                    .filter(|item| item.count > 0)
                    .map(|item| item.def.id)
                    .collect::<Vec<_>>(),
                "quest_statuses": quests,
                "nonzero_varps": varps,
            }));
            if self.snapshot.bank_loaded() {
                self.observed.bank = Some(totals(self.snapshot.bank()));
            }
        } else if self.observed.last_ingame.is_some() {
            self.observed.logged_out = true;
        }
        if matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            return;
        }
        self.runner.tick_with_hold(client, hold);
    }
}

fn run_seed_cell() -> Value {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "quest-watch seed requires LIVE=1"
    );
    api::hostlog::set_debug(true);
    let prefix = required_env("BOT_LIVE_NAME_PREFIX");
    assert!(
        !prefix.is_empty() && prefix.len() <= 4 && prefix.is_ascii(),
        "BOT_LIVE_NAME_PREFIX must be 1-4 ASCII bytes"
    );
    let account = required_env("QUEST_WATCH_ACCOUNT");
    let password = required_env("QUEST_WATCH_PASSWORD");
    assert!(
        account.len() <= 12 && account.starts_with(&prefix),
        "QUEST_WATCH_ACCOUNT {account:?} must start with BOT_LIVE_NAME_PREFIX {prefix:?} and fit 12 chars"
    );
    let scenario_name = required_env("QUEST_WATCH_SCENARIO");
    let scenario = match scenario_name.as_str() {
        "quest_watch_seed" => {
            scenario::quest_watch::quest_watch_seed_scenario_from_env().expect("seed scenario")
        }
        "quest_watch_speed" => {
            scenario::quest_watch::quest_watch_speed_scenario_from_env().expect("speed scenario")
        }
        other => panic!(
            "QUEST_WATCH_SCENARIO must be quest_watch_seed or quest_watch_speed, got {other:?}"
        ),
    };
    let deadline_seconds = scenario.settings.deadline;
    let evidence_dir = PathBuf::from(required_env("LIVE_EVIDENCE_DIR"));
    std::fs::create_dir_all(&evidence_dir).expect("create receipt directory");

    let home = ThrowawayHome::enter(&scenario_name);
    let options = live_options(&home.path);
    let template = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .prepare_template()
        .expect("prepare cold live template");
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(std::slice::from_ref(&account));
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(SeedState {
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        observed: Observed::default(),
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let profile = Profile {
        username: account.clone(),
        password: password.into(),
        uid: 1,
        settings: ProfileSettings::default(),
    };
    let mut play = run_with_template(
        template,
        true,
        vec![profile],
        move |_| (None, None),
        move |client, username, hold| {
            if username == frame_account {
                frame_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .frame(client, hold.hold);
            }
        },
    )
    .expect("start quest-watch Play");
    {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.runner.set_obj_names(play.obj_names());
    }
    play.focus(&account);

    let deadline = Instant::now() + deadline_seconds + Duration::from_secs(30);
    let (result, error) = loop {
        let status = {
            let state = state.lock().unwrap_or_else(|error| error.into_inner());
            state.runner.status()
        };
        match status {
            RunnerStatus::Passed => break ("PASS", None),
            RunnerStatus::Failed(error) => break ("FAIL", Some(error)),
            _ => {}
        }
        let relog_login = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .runner
            .take_relog_login();
        if relog_login {
            if let Some(arm) = play.arm(&account) {
                arm.arm_explicit_login();
            }
        }
        assert!(
            Instant::now() < deadline,
            "{scenario_name} for {account} timed out after {deadline_seconds:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let observed = {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut state.observed)
    };
    let receipt = json!({
        "account": account,
        "scenario": scenario_name,
        "result": result,
        "error": error,
        "logged_out": observed.logged_out,
        "snapshot": observed.last_ingame,
        "bank": observed.bank.as_ref().map(totals_json),
    });
    let receipt_path = evidence_dir.join("receipt.json");
    std::fs::write(
        &receipt_path,
        serde_json::to_string_pretty(&receipt).expect("receipt encodes") + "\n",
    )
    .unwrap_or_else(|error| panic!("write {}: {error}", receipt_path.display()));
    drop(play);
    drop(home);
    assert_eq!(result, "PASS", "{scenario_name} for {account}: {receipt}");
    receipt
}

#[test]
#[ignore = "requires LIVE=1, a fixed QUEST_WATCH_ACCOUNT under BOT_LIVE_NAME_PREFIX, and a local 289 engine; see the module doc"]
fn live_quest_watch_seed() {
    let _ = run_seed_cell();
}

// ---- the offline fixture-vault builder ------------------------------------

#[derive(serde::Deserialize)]
struct HomeSpec {
    vault_path: String,
    passphrase: String,
    receipt_path: String,
    profiles: Vec<HomeProfile>,
}

#[derive(serde::Deserialize)]
struct HomeProfile {
    username: String,
    password: String,
    uid: i32,
    quest: String,
    /// Partner quests only: the other account's username.
    #[serde(default)]
    partner_account: Option<String>,
    /// Partner quests only: `phoenix` or `blackarm`.
    #[serde(default)]
    gang: Option<String>,
}

#[test]
#[ignore = "offline fixture builder; set QUEST_WATCH_HOME_SPEC to the spec JSON path"]
fn build_quest_watch_home() {
    let spec_path = required_env("QUEST_WATCH_HOME_SPEC");
    let text = std::fs::read_to_string(&spec_path).expect("read home spec");
    let spec: HomeSpec = serde_json::from_str(&text).expect("parse home spec");
    assert!(!spec.profiles.is_empty(), "home spec needs profiles");
    let vault_path = PathBuf::from(&spec.vault_path);
    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent).expect("create vault directory");
    }
    let mut vault = Vault::create(&vault_path, &spec.passphrase).expect("create fixture vault");
    for row in &spec.profiles {
        assert!(
            row.username.len() <= 12,
            "username {:?} exceeds 12 chars",
            row.username
        );
        let mut settings = ProfileSettings {
            auto_login: true,
            tutorial_skipped: Some(true),
            script_assignment: Some(ScriptAssignment {
                source_kind: "compiled".into(),
                identity: "Quester".into(),
                display_name: "Quester".into(),
                unavailable: None,
            }),
            ..ProfileSettings::default()
        };
        let mut bag = Map::new();
        bag.insert("quests".into(), json!([row.quest]));
        // Watch-fixture walk permissions: a staged Path may route
        // through a danger zone or the wilderness, and the runner
        // parks instead of crossing without permission (proved live
        // on mortton, whose first walk crosses Afflicted zones). The
        // morning run watches the guides run, so all three are on.
        bag.insert("allow_teleports".into(), json!(true));
        bag.insert("allow_wilderness".into(), json!(true));
        bag.insert("allow_danger_zones".into(), json!(true));
        if let Some(partner) = &row.partner_account {
            bag.insert("partner_account".into(), json!(partner));
        }
        if let Some(gang) = &row.gang {
            assert!(
                matches!(gang.as_str(), "phoenix" | "blackarm"),
                "gang for {} must be phoenix or blackarm, got {gang:?}",
                row.username
            );
            bag.insert("gang".into(), json!(gang));
        }
        // Compiled cards store a versioned envelope, not the bare bag
        // (`vault::CompiledSettingsRecord`); the Quester card's schema
        // version is 3 (`script::quester::card::CARD`).
        let entry = vault::CompiledSettingsRecord {
            schema_version: 3,
            values: bag,
        }
        .into_entry();
        settings
            .script_settings
            .insert("compiled:Quester".into(), entry);
        vault
            .upsert(Profile {
                username: row.username.clone(),
                password: row.password.clone().into(),
                uid: row.uid,
                settings,
            })
            .unwrap_or_else(|error| panic!("upsert {}: {error}", row.username));
    }
    drop(vault);

    let reopened = Vault::unlock(&vault_path, &spec.passphrase).expect("reopen fixture vault");
    let mut rows = Vec::new();
    for row in &spec.profiles {
        let profile = reopened
            .get(&row.username)
            .unwrap_or_else(|| panic!("profile {} missing after reopen", row.username));
        let entry = profile
            .settings
            .script_settings
            .get("compiled:Quester")
            .unwrap_or_else(|| panic!("profile {} has no Quester settings", row.username));
        let (version, values) = vault::CompiledSettingsRecord::view(entry)
            .unwrap_or_else(|error| panic!("profile {} settings envelope: {error}", row.username));
        rows.push(json!({
            "username": profile.username,
            "uid": profile.uid,
            "auto_login": profile.settings.auto_login,
            "tutorial_skipped": profile.settings.tutorial_skipped,
            "assignment": profile.settings.script_assignment.as_ref()
                .map(|a| json!({"source_kind": a.source_kind, "identity": a.identity})),
            "settings_schema_version": version,
            "quests": values.get("quests"),
            "partner_account": values.get("partner_account"),
            "gang": values.get("gang"),
        }));
    }
    let receipt = json!({
        "vault": spec.vault_path,
        "profiles": rows,
    });
    std::fs::write(
        &spec.receipt_path,
        serde_json::to_string_pretty(&receipt).expect("receipt encodes") + "\n",
    )
    .unwrap_or_else(|error| panic!("write {}: {error}", spec.receipt_path));
    for row in &spec.profiles {
        let found = receipt["profiles"]
            .as_array()
            .expect("receipt rows")
            .iter()
            .find(|entry| entry["username"] == json!(row.username))
            .expect("receipt row");
        assert_eq!(found["quests"], json!([row.quest]), "{}", row.username);
        assert_eq!(found["uid"], json!(row.uid), "{}", row.username);
        assert_eq!(
            found["partner_account"],
            serde_json::to_value(&row.partner_account).expect("partner encodes"),
            "{}",
            row.username
        );
        assert_eq!(
            found["gang"],
            serde_json::to_value(&row.gang).expect("gang encodes"),
            "{}",
            row.username
        );
    }
}
