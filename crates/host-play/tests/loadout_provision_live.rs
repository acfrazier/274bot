//! Isolated observed loadout provisioning proof through production Play.
//!
//! This is component proof preparation, not a catalog card or frontend claim.
//! The ignored live cell uses ScriptStartHandle and public shim loadout/bank/
//! equip APIs. Root owns actual LIVE launches.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{loadout_slot_for_wearpos, WEARPOS_RIGHTHAND};
use api::interact::{self, Interactions, SendResult};
use api::snapshot::{GameSnapshot, ItemView, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use serde::Serialize;
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const PROVISION_JS: &str = include_str!("fixtures/loadout_provision/provision.js");
const PRESET_NAME: &str = "provision-proof";
const WEAPON_NAME: &str = "Rune scimitar";
const SUPPLY_NAME: &str = "Lobster";
const RUNE_SCIMITAR_ID: i32 = 1333;
const LOBSTER_ID: i32 = 379;
const SUPPLY_QTY: i32 = 17;
const BANK_STOCK_LOBSTER: i32 = 40;
const BANK_STOCK_WEAPON: i32 = 1;
const ATTACK_LEVEL: i32 = 40;
const VARROCK_WEST_BANK: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const PREP_DEADLINE: Duration = Duration::from_secs(90);
const OBSERVE_DEADLINE: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Serialize)]
struct Observation {
    ingame: bool,
    scene_state: i32,
    player: Option<String>,
    tile: Option<(i32, i32, i32)>,
    tick: u32,
    attack: i32,
    inventory_tab_available: bool,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: u64,
    inv_lobster: i32,
    inv_scimitar: i32,
    bank_lobster: i32,
    bank_scimitar: i32,
    eq_scimitar: i32,
    eq_scimitar_slot: Option<i32>,
}

impl Observation {
    fn from_snapshot(snapshot: &GameSnapshot) -> Self {
        let scim = snapshot
            .equipment()
            .iter()
            .find(|item| item.def.id == RUNE_SCIMITAR_ID && item.count > 0);
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            player: snapshot
                .local_player()
                .and_then(|local| local.player.actor.name.clone()),
            tile: snapshot.tile(),
            tick: snapshot.tick(),
            attack: snapshot
                .stats()
                .iter()
                .find(|stat| stat.name.eq_ignore_ascii_case("attack"))
                .map(|stat| stat.base)
                .unwrap_or(0),
            inventory_tab_available: snapshot
                .side_tabs()
                .iter()
                .any(|tab| tab.index == 3 && tab.available),
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_generation: snapshot.bank_session_generation(),
            inv_lobster: count_id(snapshot.inventory(), LOBSTER_ID),
            inv_scimitar: count_id(snapshot.inventory(), RUNE_SCIMITAR_ID),
            bank_lobster: count_id(snapshot.bank(), LOBSTER_ID),
            bank_scimitar: count_id(snapshot.bank(), RUNE_SCIMITAR_ID),
            eq_scimitar: scim.map(|item| item.count).unwrap_or(0),
            eq_scimitar_slot: scim.map(|item| item.slot),
        }
    }

    fn lacks_configured_gear(&self) -> bool {
        self.inv_lobster == 0 && self.inv_scimitar == 0 && self.eq_scimitar == 0
    }

    fn righthand_scimitar(&self) -> bool {
        self.eq_scimitar > 0
            && self.eq_scimitar_slot == Some(WEARPOS_RIGHTHAND)
            && self.eq_scimitar_slot.and_then(loadout_slot_for_wearpos) == Some("righthand")
    }
}

fn count_id(items: &[ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

#[derive(Debug, Clone, Serialize)]
struct ProvisionWitness {
    baseline: Observation,
    fresh_bank: Option<Observation>,
    inventory: Option<Observation>,
    equipped: Option<Observation>,
    post_start_observations: u32,
    saw_ready_empty: bool,
    seeded_equipment_after_open: bool,
}

impl ProvisionWitness {
    fn new(baseline: Observation) -> Self {
        Self {
            baseline,
            fresh_bank: None,
            inventory: None,
            equipped: None,
            post_start_observations: 0,
            saw_ready_empty: false,
            seeded_equipment_after_open: false,
        }
    }

    fn observe(&mut self, observation: Observation) {
        self.post_start_observations = self.post_start_observations.saturating_add(1);
        if self.fresh_bank.is_none() {
            if observation.bank_open && observation.bank_loaded {
                if observation.bank_generation <= self.baseline.bank_generation {
                    return;
                }
                if observation.bank_lobster < SUPPLY_QTY
                    || observation.bank_scimitar < BANK_STOCK_WEAPON
                {
                    self.saw_ready_empty = true;
                    return;
                }
                if observation.eq_scimitar > 0 {
                    self.seeded_equipment_after_open = true;
                    return;
                }
                self.fresh_bank = Some(observation);
            }
            return;
        }
        if self.inventory.is_none() {
            let withdrawn_weapon = observation.inv_scimitar >= 1 || observation.eq_scimitar >= 1;
            if observation.inv_lobster == SUPPLY_QTY && withdrawn_weapon {
                self.inventory = Some(observation.clone());
                if observation.righthand_scimitar() && observation.inv_lobster == SUPPLY_QTY {
                    self.equipped = Some(observation);
                }
            }
            return;
        }
        if self.equipped.is_none()
            && observation.righthand_scimitar()
            && observation.inv_lobster == SUPPLY_QTY
        {
            self.equipped = Some(observation);
        }
    }

    fn qualify(&self) -> Result<Value, String> {
        if !self.baseline.lacks_configured_gear() {
            return Err(
                "baseline already has the configured weapon or supply in inventory or equipment"
                    .into(),
            );
        }
        if self.seeded_equipment_after_open {
            return Err("fresh bank already had the configured weapon equipped".into());
        }
        let Some(bank) = self.fresh_bank.as_ref() else {
            if self.saw_ready_empty {
                return Err("fresh bank opened without sufficient configured stock".into());
            }
            return Err("no later fresh bank generation with loaded stock".into());
        };
        if !bank.bank_open || !bank.bank_loaded {
            return Err("fresh bank was not open and loaded".into());
        }
        if bank.bank_generation <= self.baseline.bank_generation {
            return Err("bank generation did not advance after Start".into());
        }
        if bank.bank_lobster < SUPPLY_QTY || bank.bank_scimitar < BANK_STOCK_WEAPON {
            return Err("fresh bank lacked configured stock".into());
        }
        let Some(inventory) = self.inventory.as_ref() else {
            return Err(
                "no observed inventory withdrawal; queued request is not a provision".into(),
            );
        };
        if inventory.inv_lobster != SUPPLY_QTY {
            return Err(format!(
                "inventory lobster count {} is not the configured {SUPPLY_QTY}",
                inventory.inv_lobster
            ));
        }
        let Some(equipped) = self.equipped.as_ref() else {
            return Err("no observed equipped weapon id/slot".into());
        };
        if !equipped.righthand_scimitar() {
            return Err(format!(
                "equipped scimitar id/slot not righthand: count={} slot={:?}",
                equipped.eq_scimitar, equipped.eq_scimitar_slot
            ));
        }
        if equipped.inv_lobster != SUPPLY_QTY {
            return Err(format!(
                "supplies were not preserved after equip: lobster {}",
                equipped.inv_lobster
            ));
        }
        let later = [inventory, equipped];
        if !later
            .iter()
            .any(|row| row.bank_lobster == bank.bank_lobster - SUPPLY_QTY)
        {
            return Err("lobster bank stock did not decrement by the configured quantity".into());
        }
        if !later
            .iter()
            .any(|row| row.bank_scimitar == bank.bank_scimitar - BANK_STOCK_WEAPON)
        {
            return Err("weapon bank stock did not decrement".into());
        }
        Ok(json!({
            "baseline": self.baseline,
            "fresh_bank": bank,
            "inventory": inventory,
            "equipped": equipped,
            "post_start_observations": self.post_start_observations,
            "weapon_id": RUNE_SCIMITAR_ID,
            "supply_id": LOBSTER_ID,
            "supply_qty": SUPPLY_QTY,
            "righthand_slot": WEARPOS_RIGHTHAND,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    WaitIngame,
    TutSkip,
    WaitTutorial,
    Relog,
    WaitRelog,
    Seed,
    WaitSeed,
    DrainDialogs,
    AckBank,
    WaitAck,
    CloseBank,
    WaitClosed,
    Ready,
}

struct LiveState {
    account: String,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    prep: Prep,
    baseline: Option<Observation>,
    witness: Option<ProvisionWitness>,
    start_error: Option<String>,
    started: bool,
    last_action: Instant,
}

impl LiveState {
    fn publish(&mut self, client: &client::client::Client) -> Observation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        Observation::from_snapshot(&self.snapshot)
    }

    fn capture_baseline(&mut self, observation: &Observation) -> Result<(), String> {
        if !observation.ingame || observation.scene_state != 2 {
            return Err(format!(
                "Start baseline is not attached ingame scene2: {observation:?}"
            ));
        }
        let player = observation
            .player
            .as_deref()
            .ok_or_else(|| "Start baseline has no local player".to_string())?;
        let expected = client::util::jstring::JString::to_screen_name(&self.account);
        if !player.eq_ignore_ascii_case(&expected) {
            return Err(format!(
                "Start baseline player {player:?} is not fresh account {:?}",
                self.account
            ));
        }
        if observation.attack < ATTACK_LEVEL {
            return Err(format!(
                "Start baseline attack {} is below required {ATTACK_LEVEL}",
                observation.attack
            ));
        }
        if !observation.inventory_tab_available {
            return Err("Start baseline inventory tab is not bound after relog".into());
        }
        if !observation.lacks_configured_gear() {
            return Err(
                "Start baseline already has configured weapon or supply in inventory or equipment"
                    .into(),
            );
        }
        if !near(
            observation.tile,
            (
                VARROCK_WEST_BANK.x,
                VARROCK_WEST_BANK.z,
                VARROCK_WEST_BANK.level,
            ),
            6,
        ) {
            return Err(format!(
                "Start baseline is not at the selected bank stand: {:?}",
                observation.tile
            ));
        }
        self.baseline = Some(observation.clone());
        self.witness = Some(ProvisionWitness::new(observation.clone()));
        println!(
            "{}",
            json!({
                "phase": "baseline-after-preparation",
                "account": self.account,
                "observation": observation,
            })
        );
        Ok(())
    }

    fn start_script(&mut self) -> Result<(), String> {
        let Some(handle) = self.start_handle.as_ref() else {
            return Err("Start reached before ScriptStartHandle install".into());
        };
        let _iso = script::IsolatedEnv::enter("loadout-provision-live");
        let mut store = script::LoadoutsStore::with_default_path();
        store.upsert(
            script::Loadout::new(PRESET_NAME)
                .with_slot("righthand", WEAPON_NAME)
                .with_carry(SUPPLY_NAME, SUPPLY_QTY as u32),
        );
        store.save()?;
        let mut bag = Map::new();
        bag.insert("loadout".into(), json!(PRESET_NAME));
        handle.start_load(
            &self.account,
            PROVISION_JS.to_string(),
            script::LoadShape::CompatClass,
            Some(bag),
            vec![],
        )?;
        self.started = true;
        println!(
            "{}",
            json!({
                "phase": "start",
                "account": self.account,
                "preset": PRESET_NAME,
                "weapon": WEAPON_NAME,
                "supply": SUPPLY_NAME,
                "qty": SUPPLY_QTY,
            })
        );
        Ok(())
    }

    fn chat_has(&self, needle: &str) -> bool {
        self.snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.to_ascii_lowercase().contains(needle))
            || self
                .snapshot
                .chat_modal_texts()
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(needle))
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish(client);
        if hold {
            return;
        }
        if self.started {
            if let Some(witness) = self.witness.as_mut() {
                witness.observe(observation);
            }
            return;
        }
        if let Err(error) = self.advance_prep(client, &observation) {
            self.start_error = Some(error);
        }
    }

    fn advance_prep(
        &mut self,
        client: &mut client::client::Client,
        observation: &Observation,
    ) -> Result<(), String> {
        let now = Instant::now();
        match self.prep {
            Prep::WaitIngame => {
                if observation.ingame && observation.scene_state == 2 {
                    self.prep = Prep::TutSkip;
                }
            }
            Prep::TutSkip => {
                interact::cheat(client, "setvar tutorial 1000");
                interact::cheat(client, "getvar tutorial");
                self.last_action = now;
                self.prep = Prep::WaitTutorial;
            }
            Prep::WaitTutorial => {
                if self.chat_has("get tutorial: 1000") {
                    self.prep = Prep::Relog;
                }
            }
            Prep::Relog => {
                println!(
                    "{}",
                    json!({"phase": "before-relog", "observation": observation})
                );
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout iface missing (side icons still tutorial-locked?)".into());
                }
                self.last_action = now;
                self.prep = Prep::WaitRelog;
            }
            Prep::WaitRelog => {
                // Use the shared ScenarioRunner's SideTabAvailable contract:
                // a Play frame need not arrive during the off-world interval.
                if observation.ingame
                    && observation.scene_state == 2
                    && observation.inventory_tab_available
                {
                    println!(
                        "{}",
                        json!({"phase": "after-relog", "observation": observation})
                    );
                    self.prep = Prep::Seed;
                }
            }
            Prep::Seed => {
                interact::cheat(client, &format!("setstat attack {ATTACK_LEVEL}"));
                interact::cheat(client, "givebank rune_scimitar 1");
                interact::cheat(client, &format!("givebank lobster {BANK_STOCK_LOBSTER}"));
                interact::cheat(
                    client,
                    &interact::tele_args(
                        VARROCK_WEST_BANK.level,
                        VARROCK_WEST_BANK.x,
                        VARROCK_WEST_BANK.z,
                    ),
                );
                self.last_action = now;
                self.prep = Prep::WaitSeed;
            }
            Prep::WaitSeed => {
                if observation.ingame
                    && observation.scene_state == 2
                    && observation.attack >= ATTACK_LEVEL
                    && near(
                        observation.tile,
                        (
                            VARROCK_WEST_BANK.x,
                            VARROCK_WEST_BANK.z,
                            VARROCK_WEST_BANK.level,
                        ),
                        6,
                    )
                {
                    self.prep = Prep::DrainDialogs;
                }
            }
            Prep::DrainDialogs => {
                let modals = self.snapshot.modals();
                if modals.main == -1 && modals.chat == -1 {
                    self.prep = Prep::AckBank;
                } else {
                    let _ = interact::close_modal(client);
                }
            }
            Prep::AckBank => {
                if observation.bank_open && observation.bank_loaded {
                    self.prep = Prep::WaitAck;
                    return Ok(());
                }
                if now.duration_since(self.last_action) >= Duration::from_millis(400) {
                    match Interactions::new(&self.snapshot, client).open_nearest_booth() {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => {
                            self.last_action = now;
                            self.prep = Prep::WaitAck;
                        }
                    }
                }
            }
            Prep::WaitAck => {
                if observation.bank_open
                    && observation.bank_loaded
                    && observation.bank_lobster >= SUPPLY_QTY
                    && observation.bank_scimitar >= BANK_STOCK_WEAPON
                {
                    println!(
                        "{}",
                        json!({
                            "phase": "acknowledged-bank",
                            "observation": observation,
                        })
                    );
                    self.prep = Prep::CloseBank;
                } else if !observation.bank_open
                    && now.duration_since(self.last_action) >= Duration::from_millis(400)
                {
                    let _ = Interactions::new(&self.snapshot, client).open_nearest_booth();
                    self.last_action = now;
                }
            }
            Prep::CloseBank => {
                let _ = interact::close_modal(client);
                self.last_action = now;
                self.prep = Prep::WaitClosed;
            }
            Prep::WaitClosed => {
                if !observation.bank_open {
                    self.prep = Prep::Ready;
                } else if now.duration_since(self.last_action) > Duration::from_millis(400) {
                    let _ = interact::close_modal(client);
                    self.last_action = now;
                }
            }
            Prep::Ready => {
                if self.baseline.is_none() {
                    self.capture_baseline(observation)?;
                }
                self.start_script()?;
            }
        }
        Ok(())
    }
}

fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "274bot-loadout-provision-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(script::JsCache::origin_sha(&bytes))
}

fn selected_profile(
    revision: u16,
    nav_pack: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision.to_string()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(if revision == 289 { 44594 } else { 43594 }),
        http_port: Some(if revision == 289 { 1080 } else { 80 }),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("LOADOUT_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("LOADOUT_ENGINE_DIR").map(PathBuf::from),
        vault_path: Some(temp.join("vault")),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.target() != client::BotTarget::Local || profile.client().game_host() != "127.0.0.1" {
        return Err("loadout provision proof requires a loopback-only local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn run_cell() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let _home = script::IsolatedEnv::enter("loadout-provision-live-home");
    let revision = required("LOADOUT_REVISION")?
        .parse::<u16>()
        .map_err(|_| "LOADOUT_REVISION must be 274 or 289".to_string())?;
    if !matches!(revision, 274 | 289) {
        return Err("LOADOUT_REVISION must be 274 or 289".into());
    }
    let nav_pack = PathBuf::from(required("LOADOUT_NAV_PACK")?);
    let temp = TempRoot::new()?;
    let (profile, template) = selected_profile(revision, nav_pack, temp.path())?;
    let names = host_play::mint_live_names(1);
    let account = names
        .first()
        .cloned()
        .ok_or_else(|| "failed to mint live account".to_string())?;
    let credentials = host_play::mint_live_entries_for_target(&names, profile.target());
    let password = credentials
        .first()
        .map(|(_, password)| password.clone())
        .ok_or_else(|| "failed to mint local credential".to_string())?;

    let state = Arc::new(Mutex::new(LiveState {
        account: account.clone(),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        prep: Prep::WaitIngame,
        baseline: None,
        witness: None,
        start_error: None,
        started: false,
        last_action: Instant::now(),
    }));
    let frame_state = Arc::clone(&state);
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            let mut state = frame_state.lock().unwrap();
            if username == state.account {
                state.frame(client, hold);
            }
        },
    )?;
    {
        let mut state = state.lock().unwrap();
        state.start_handle = Some(play.script_start_handle());
    }
    let nav_sha256 = hash_file(profile.nav_pack())?;
    println!(
        "{}",
        json!({
            "phase": "identity",
            "revision": profile.revision().as_i32(),
            "profile": profile.label(),
            "cache_id": profile.cache_id(),
            "nav_pack": profile.nav_pack(),
            "nav_sha256": nav_sha256,
            "account": account,
            "preset": PRESET_NAME,
            "weapon_id": RUNE_SCIMITAR_ID,
            "supply_id": LOBSTER_ID,
            "qty": SUPPLY_QTY,
            "script_sha256": script::JsCache::origin_sha(PROVISION_JS.as_bytes()),
        })
    );

    let account_profile = Profile {
        username: account.clone(),
        password,
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    play.try_spawn_slot(account_profile, None, None, None)?;
    play.focus(&account);

    let prep_deadline = Instant::now() + PREP_DEADLINE;
    let observe_deadline = Instant::now() + PREP_DEADLINE + OBSERVE_DEADLINE;
    let outcome = loop {
        if let Some(error) = play.script_last_error(&account) {
            break Err(format!("script error: {error}"));
        }
        let mut state = state.lock().unwrap();
        if let Some(error) = state.start_error.take() {
            break Err(error);
        }
        if !state.started {
            if Instant::now() >= prep_deadline {
                break Err(format!(
                    "preparation timeout; prep={:?}; observation={:?}",
                    state.prep,
                    Observation::from_snapshot(&state.snapshot)
                ));
            }
        } else if let Some(witness) = state.witness.as_ref() {
            match witness.qualify() {
                Ok(core) => break Ok(core),
                Err(_) if Instant::now() >= observe_deadline => {
                    break Err(format!(
                        "bounded observe timeout; witness={}",
                        json!(witness)
                    ));
                }
                Err(_) => {}
            }
        } else if Instant::now() >= observe_deadline {
            break Err("bounded timeout without a Start baseline".into());
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(20));
    };

    play.stop_slot(&account);
    let outcome = outcome?;
    println!("PASS: loadout_provision_live: {outcome}");
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, LOADOUT_REVISION/NAV_PACK, and local engine"]
fn loadout_provision_live() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_cell));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("FAIL: loadout_provision_live: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("FAIL: loadout_provision_live: {error:?}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> Observation {
        Observation {
            ingame: true,
            scene_state: 2,
            player: Some("provision".into()),
            tile: Some((3185, 3440, 0)),
            tick: 10,
            attack: 40,
            inventory_tab_available: true,
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            inv_lobster: 0,
            inv_scimitar: 0,
            bank_lobster: 0,
            bank_scimitar: 0,
            eq_scimitar: 0,
            eq_scimitar_slot: None,
        }
    }

    #[test]
    fn baseline_rejects_seeded_equipment() {
        let mut baseline = observation();
        baseline.eq_scimitar = 1;
        baseline.eq_scimitar_slot = Some(WEARPOS_RIGHTHAND);
        let error = ProvisionWitness::new(baseline).qualify().unwrap_err();
        assert!(error.contains("baseline already has"), "{error}");
    }

    #[test]
    fn rejects_stale_or_empty_bank() {
        let baseline = observation();
        let mut witness = ProvisionWitness::new(baseline.clone());
        witness.observe(baseline.clone());
        let error = witness.qualify().unwrap_err();
        assert!(error.contains("fresh bank"), "{error}");

        let mut empty = baseline.clone();
        empty.bank_open = true;
        empty.bank_loaded = true;
        empty.bank_generation = 1;
        let mut witness = ProvisionWitness::new(baseline);
        witness.observe(empty);
        let error = witness.qualify().unwrap_err();
        assert!(error.contains("sufficient configured stock"), "{error}");
    }

    #[test]
    fn rejects_queued_request_without_inventory_or_equip() {
        let baseline = observation();
        let mut bank = baseline.clone();
        bank.bank_open = true;
        bank.bank_loaded = true;
        bank.bank_generation = 3;
        bank.bank_lobster = BANK_STOCK_LOBSTER;
        bank.bank_scimitar = BANK_STOCK_WEAPON;
        let mut witness = ProvisionWitness::new(baseline);
        witness.observe(bank);
        let error = witness.qualify().unwrap_err();
        assert!(error.contains("queued request"), "{error}");
    }

    #[test]
    fn accepts_ordered_fresh_bank_withdraw_decrement_and_righthand_equip() {
        let baseline = observation();
        let mut bank = baseline.clone();
        bank.bank_open = true;
        bank.bank_loaded = true;
        bank.bank_generation = 4;
        bank.bank_lobster = BANK_STOCK_LOBSTER;
        bank.bank_scimitar = BANK_STOCK_WEAPON;
        let mut withdrawn = bank.clone();
        withdrawn.inv_lobster = SUPPLY_QTY;
        withdrawn.inv_scimitar = 1;
        withdrawn.bank_lobster = BANK_STOCK_LOBSTER - SUPPLY_QTY;
        withdrawn.bank_scimitar = 0;
        let mut equipped = withdrawn.clone();
        equipped.inv_scimitar = 0;
        equipped.eq_scimitar = 1;
        equipped.eq_scimitar_slot = Some(WEARPOS_RIGHTHAND);
        let mut witness = ProvisionWitness::new(baseline);
        witness.observe(bank);
        witness.observe(withdrawn);
        witness.observe(equipped);
        let core = witness.qualify().expect("ordered provision should pass");
        assert_eq!(core["supply_qty"], SUPPLY_QTY);
        assert_eq!(core["weapon_id"], RUNE_SCIMITAR_ID);
        assert_eq!(core["righthand_slot"], WEARPOS_RIGHTHAND);
    }

    #[test]
    fn rejects_missing_bank_decrement_and_lost_supplies() {
        let baseline = observation();
        let mut bank = baseline.clone();
        bank.bank_open = true;
        bank.bank_loaded = true;
        bank.bank_generation = 2;
        bank.bank_lobster = BANK_STOCK_LOBSTER;
        bank.bank_scimitar = BANK_STOCK_WEAPON;
        let mut withdrawn = bank.clone();
        withdrawn.inv_lobster = SUPPLY_QTY;
        withdrawn.inv_scimitar = 1;
        let mut equipped = withdrawn.clone();
        equipped.eq_scimitar = 1;
        equipped.eq_scimitar_slot = Some(WEARPOS_RIGHTHAND);
        let mut witness = ProvisionWitness::new(baseline.clone());
        witness.observe(bank.clone());
        witness.observe(withdrawn);
        witness.observe(equipped);
        let error = witness.qualify().unwrap_err();
        assert!(error.contains("decrement"), "{error}");

        let mut lost = bank.clone();
        lost.inv_lobster = 0;
        lost.eq_scimitar = 1;
        lost.eq_scimitar_slot = Some(WEARPOS_RIGHTHAND);
        lost.bank_lobster = BANK_STOCK_LOBSTER - SUPPLY_QTY;
        lost.bank_scimitar = 0;
        let mut witness = ProvisionWitness::new(baseline);
        witness.observe(bank);
        witness.observe(lost);
        let error = witness.qualify().unwrap_err();
        assert!(
            error.contains("queued request") || error.contains("inventory"),
            "{error}"
        );
    }
}
