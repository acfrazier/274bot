use super::plan::seed_pack::{NameTable, ResolvedName};
use api::game_data::SelectedGameData;
use api::scenario_observe::{
    AppliedEvent, DecodedDomainEvent, DecodedDomainSelection, InventoryForm, InventoryScope,
    ScenarioObserveWatermark,
};
use api::snapshot::{self, GameSnapshot};
use client::client::Client;
use client::io::JagFile;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

pub(super) const DEFAULT_OBSERVED_TICK_BUDGET: u32 = 20;
const TELEPORT_OBSERVED_TICK_BUDGET: u32 = 120;
const BANK_UNCONFIRMED_OBSERVED_TICKS: u32 = 1;
const MAX_PACK_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ENGINE_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CONFIG_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROFILE_TEXT_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const MAX_REASON_BYTES: usize = 160;
pub(super) const MAX_COMMAND_BYTES: usize = 80;
const ACCESS_REFUSAL: &str = "Please finish what you are doing first.";
const INVENTORY_WIPED: &str = "Inventory wiped.";
const ENERGY_RESTORED: &str = "Run energy restored";

/// The fixed fixture commands that the scenario runtime may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinSeed {
    Teleport,
    Give,
    GiveBank,
    ClearInventory,
    ClearBank,
    SetStat,
    ResetAndAdvanceStat,
    SetVar,
    EnergyFullRunOn,
}

pub const SEED_ENTRIES: &[(&str, BuiltinSeed)] = &[
    ("seed.teleport", BuiltinSeed::Teleport),
    ("seed.give", BuiltinSeed::Give),
    ("seed.give_bank", BuiltinSeed::GiveBank),
    ("seed.clear_inventory", BuiltinSeed::ClearInventory),
    ("seed.clear_bank", BuiltinSeed::ClearBank),
    ("seed.set_stat", BuiltinSeed::SetStat),
    (
        "seed.reset_and_advance_stat",
        BuiltinSeed::ResetAndAdvanceStat,
    ),
    ("seed.set_var", BuiltinSeed::SetVar),
    ("seed.energy_full_run_on", BuiltinSeed::EnergyFullRunOn),
];
impl BuiltinSeed {
    pub fn parse(name: &str) -> Option<Self> {
        SEED_ENTRIES
            .iter()
            .find_map(|(entry, seed)| (*entry == name).then_some(*seed))
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Teleport => "seed.teleport",
            Self::Give => "seed.give",
            Self::GiveBank => "seed.give_bank",
            Self::ClearInventory => "seed.clear_inventory",
            Self::ClearBank => "seed.clear_bank",
            Self::SetStat => "seed.set_stat",
            Self::ResetAndAdvanceStat => "seed.reset_and_advance_stat",
            Self::SetVar => "seed.set_var",
            Self::EnergyFullRunOn => "seed.energy_full_run_on",
        }
    }
}

/// Controls the two explicitly unconfirmed bank commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedPolicy {
    ConfirmedOnly,
    AllowUnconfirmed,
}

pub(super) struct SeedRequest<'a> {
    pub operation: BuiltinSeed,
    pub args: &'a Map<String, Value>,
    pub why: &'a str,
    pub policy: SeedPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariableKind {
    Integer,
    String,
    Unsupported(u8),
}

#[derive(Debug, Clone)]
struct VarpDefinition {
    debug_name: String,
    kind: VariableKind,
    scope: u8,
    client_code: u16,
    protect: bool,
    transmit: bool,
}

#[derive(Debug, Clone)]
struct RawVarbitDefinition {
    debug_name: String,
    base_varp: i32,
    start_bit: u8,
    end_bit: u8,
}

#[derive(Debug, Clone)]
struct VariableDefinition {
    name: String,
    id: i32,
    kind: VariableKind,
    scope: u8,
    base_varp: i32,
    start_bit: Option<u8>,
    end_bit: Option<u8>,
    protect: bool,
    transmit: bool,
}

impl VariableDefinition {
    fn mask(&self) -> Option<u32> {
        let (start, end) = (self.start_bit?, self.end_bit?);
        let width = u32::from(end - start) + 1;
        Some(if width == 32 {
            u32::MAX
        } else {
            (1_u32 << width) - 1
        })
    }

    fn numeric_range(&self) -> (i64, i64) {
        match self.mask() {
            Some(mask) => (0, i64::from(mask.min(i32::MAX as u32))),
            None => (i64::from(i32::MIN), i64::from(i32::MAX)),
        }
    }

    fn decode_value(&self, base_value: i32) -> Option<i32> {
        let (start, _) = (self.start_bit?, self.end_bit?);
        let mask = self.mask()?;
        Some(((base_value as u32 >> u32::from(start)) & mask) as i32)
    }
}

#[derive(Debug, Clone)]
struct SkillDefinition {
    id: i32,
    name: String,
}

#[derive(Debug)]
pub struct FixtureDefinitions {
    revision: i32,
    artifact_sha256: String,
    skills: BTreeMap<String, SkillDefinition>,
    xp_tenths_by_index: Vec<i32>,
    variables: BTreeMap<String, VariableDefinition>,
    objects: BTreeMap<String, i32>,
    option_run_id: i32,
    option_run_on: i32,
}

impl FixtureDefinitions {
    /// Load the bounded, selected-profile contract used by every fixture.
    pub fn load(
        revision: i32,
        engine: &Path,
        content: &Path,
        bound_jag_dir: &Path,
        profile_class: crate::profile::ProfileClass,
    ) -> Result<Self, String> {
        if !matches!(revision, 274 | 289 | 377) {
            return Err("fixture definitions do not support this profile revision".into());
        }
        if profile_class != crate::profile::ProfileClass::Local {
            return Err("fixture definitions require a Local profile".into());
        }

        let server_pack = engine.join("data/pack/server");
        let mut hasher = Sha256::new();
        hasher.update(b"host-play-scenario-fixtures-v1\0");
        hasher.update(revision.to_be_bytes());

        let varp_dat = read_hashed(
            &server_pack.join("varp.dat"),
            "engine server varp.dat",
            MAX_PACK_BYTES,
            &mut hasher,
        )?;
        let varbit_dat = read_hashed(
            &server_pack.join("varbit.dat"),
            "engine server varbit.dat",
            MAX_PACK_BYTES,
            &mut hasher,
        )?;
        let obj_dat = read_hashed(
            &server_pack.join("obj.dat"),
            "engine server obj.dat",
            MAX_PACK_BYTES,
            &mut hasher,
        )?;
        let varp_pack = read_hashed(
            &content.join("pack/varp.pack"),
            "selected content varp.pack",
            MAX_PACK_BYTES,
            &mut hasher,
        )?;
        let varbit_pack = read_hashed(
            &content.join("pack/varbit.pack"),
            "selected content varbit.pack",
            MAX_PACK_BYTES,
            &mut hasher,
        )?;
        let obj_pack = read_hashed(
            &content.join("pack/obj.pack"),
            "selected content obj.pack",
            MAX_PACK_BYTES,
            &mut hasher,
        )?;
        let config_bytes = read_hashed(
            &bound_jag_dir.join("config"),
            "bound JAG config archive",
            MAX_CONFIG_ARCHIVE_BYTES,
            &mut hasher,
        )?;

        let player_stat_source = read_hashed(
            &engine.join("src/engine/entity/PlayerStat.ts"),
            "selected engine PlayerStat.ts",
            MAX_ENGINE_SOURCE_BYTES,
            &mut hasher,
        )?;
        let player_source = read_hashed(
            &engine.join("src/engine/entity/Player.ts"),
            "selected engine Player.ts",
            MAX_ENGINE_SOURCE_BYTES,
            &mut hasher,
        )?;
        let cheat_handler_source = read_hashed(
            &engine.join("src/network/game/client/handler/ClientCheatHandler.ts"),
            "selected engine ClientCheatHandler.ts",
            MAX_ENGINE_SOURCE_BYTES,
            &mut hasher,
        )?;
        let stat_encoder_source = read_hashed(
            &engine.join("src/network/game/server/codec/UpdateStatEncoder.ts"),
            "selected engine UpdateStatEncoder.ts",
            MAX_ENGINE_SOURCE_BYTES,
            &mut hasher,
        )?;
        let network_player_source = read_hashed(
            &engine.join("src/engine/entity/NetworkPlayer.ts"),
            "selected engine NetworkPlayer.ts",
            MAX_ENGINE_SOURCE_BYTES,
            &mut hasher,
        )?;
        // The revision-377 content keeps its varp configs in the unpacked
        // `_unpack/377/all.varp` and its debug procs under `_test/scripts/debug`
        // (`[option_run]` at all.varp:277; the refusal at debug/debug.rs2).
        let (controls_varp_path, debug_path) = if revision == 377 {
            (
                "scripts/_unpack/377/all.varp",
                "scripts/_test/scripts/debug/debug.rs2",
            )
        } else {
            (
                "scripts/interface_controls/configs/player_controls.varp",
                "scripts/_test/scripts/engine/debug.rs2",
            )
        };
        let controls_varp_source = read_hashed(
            &content.join(controls_varp_path),
            "selected option_run varp config",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;
        let controls_constant_source = read_hashed(
            &content.join("scripts/interface_controls/configs/player_controls.constant"),
            "selected player_controls.constant",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;
        let controls_interface_source = read_hashed(
            &content.join("scripts/interface_controls/interfaces/controls.if"),
            "selected controls.if",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;
        let cheat_other_source = read_hashed(
            &content.join("scripts/_test/scripts/cheats/cheat_other.rs2"),
            "selected cheat_other.rs2",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;
        let cheat_clear_inventory_source = read_hashed(
            &content.join("scripts/_test/scripts/cheats/cheat_clearinv.rs2"),
            "selected cheat_clearinv.rs2",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;
        let cheat_bank_source = read_hashed(
            &content.join("scripts/_test/scripts/cheats/cheat_bank.rs2"),
            "selected cheat_bank.rs2",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;
        let cheat_debug_source = read_hashed(
            &content.join(debug_path),
            "selected debug.rs2",
            MAX_PROFILE_TEXT_BYTES,
            &mut hasher,
        )?;

        let (varps, raw_varbits) =
            decode_varp_and_varbit_records_with_archive(varp_dat, varbit_dat, config_bytes)?;
        let varp_names = parse_name_pack(&varp_pack, "varp.pack")?;
        let varbit_names = parse_name_pack(&varbit_pack, "varbit.pack")?;
        let object_names_by_id = parse_name_pack(&obj_pack, "obj.pack")?;
        let object_count = record_count(&obj_dat, "obj.dat")?;
        validate_pack_ids(&object_names_by_id, object_count, "obj.pack")?;
        validate_pack_ids(&varp_names, varps.len(), "varp.pack")?;
        validate_pack_ids(&varbit_names, raw_varbits.len(), "varbit.pack")?;
        let objects: BTreeMap<String, i32> = object_names_by_id
            .into_iter()
            .map(|(id, name)| (name, id))
            .collect();

        let mut variables = BTreeMap::new();
        for (id, name) in varp_names {
            let definition = varps
                .get(usize::try_from(id).map_err(|_| "invalid varp id".to_string())?)
                .ok_or_else(|| "varp.pack references a missing server definition".to_string())?;
            if !definition.debug_name.is_empty() && definition.debug_name != name {
                return Err("varp.pack name does not match its server definition".into());
            }
            if variables.contains_key(&name) {
                return Err("varp and varbit names are ambiguous in selected content".into());
            }
            variables.insert(
                name.clone(),
                VariableDefinition {
                    name,
                    id,
                    kind: definition.kind,
                    scope: definition.scope,
                    base_varp: id,
                    start_bit: None,
                    end_bit: None,
                    protect: definition.protect,
                    transmit: definition.transmit,
                },
            );
        }

        for (id, raw) in raw_varbits.into_iter().enumerate() {
            let id = i32::try_from(id).map_err(|_| "varbit id is out of range".to_string())?;
            let Some(name) = varbit_names.get(&id) else {
                continue;
            };
            if !raw.debug_name.is_empty() && raw.debug_name != *name {
                return Err("varbit.pack name does not match its server definition".into());
            }
            if variables.contains_key(name) {
                return Err("varp and varbit names are ambiguous in selected content".into());
            }
            if raw.base_varp < 0 {
                // Neither selected pack defines its base or bits (the R377
                // server pack names varbits only), so it stays unavailable:
                // a varbit seed or read by this name is refused, never guessed.
                continue;
            }
            let base = usize::try_from(raw.base_varp)
                .ok()
                .and_then(|base| varps.get(base))
                .ok_or_else(|| "varbit references a missing base varp".to_string())?;
            if base.kind != VariableKind::Integer {
                return Err("varbit references a non-integer base varp".into());
            }
            variables.insert(
                name.clone(),
                VariableDefinition {
                    name: name.clone(),
                    id,
                    kind: VariableKind::Integer,
                    scope: base.scope,
                    base_varp: raw.base_varp,
                    start_bit: Some(raw.start_bit),
                    end_bit: Some(raw.end_bit),
                    protect: base.protect,
                    transmit: base.transmit,
                },
            );
        }
        validate_unique_variable_names(&variables)?;

        let player_stat_source = source_text(player_stat_source, "PlayerStat.ts")?;
        let player_source = source_text(player_source, "Player.ts")?;
        let cheat_handler_source = source_text(cheat_handler_source, "ClientCheatHandler.ts")?;
        let stat_encoder_source = source_text(stat_encoder_source, "UpdateStatEncoder.ts")?;
        let network_player_source = source_text(network_player_source, "NetworkPlayer.ts")?;
        let controls_varp_source = source_text(controls_varp_source, "player_controls.varp")?;
        let controls_constant_source =
            source_text(controls_constant_source, "player_controls.constant")?;
        let controls_interface_source = source_text(controls_interface_source, "controls.if")?;
        let cheat_other_source = source_text(cheat_other_source, "cheat_other.rs2")?;
        let cheat_clear_inventory_source =
            source_text(cheat_clear_inventory_source, "cheat_clearinv.rs2")?;
        let cheat_bank_source = source_text(cheat_bank_source, "cheat_bank.rs2")?;
        let cheat_debug_source = source_text(cheat_debug_source, "debug.rs2")?;

        verify_seed_sources(SeedSources {
            cheat_handler: &cheat_handler_source,
            stat_encoder: &stat_encoder_source,
            network_player: &network_player_source,
            controls_varp: &controls_varp_source,
            controls_interface: &controls_interface_source,
            cheat_other: &cheat_other_source,
            cheat_clear_inventory: &cheat_clear_inventory_source,
            cheat_bank: &cheat_bank_source,
            cheat_debug: &cheat_debug_source,
        })?;
        let (mut skills, enabled) = parse_engine_skills(&player_stat_source)?;
        validate_client_skills(&mut skills, &enabled)?;
        let xp_tenths_by_index = parse_engine_xp_table(&player_source)?;
        let option_run_on = parse_named_constant(&controls_constant_source, "^player_run_on")?;
        if option_run_on != 1 {
            return Err(
                "selected player_controls.constant does not define ^player_run_on=1".into(),
            );
        }
        let option_run = variables
            .get("option_run")
            .ok_or_else(|| "selected profile has no option_run definition".to_string())?;
        if option_run.kind != VariableKind::Integer || !option_run.transmit {
            return Err("option_run is not a transmittable integer varp".into());
        }
        if !matches!(option_run.scope, 0 | 1) {
            return Err("option_run has an unsupported varp scope".into());
        }
        if option_run.start_bit.is_some() {
            return Err("option_run must resolve to a varp, not a varbit".into());
        }
        let engine_run_varp_id = varps
            .iter()
            .rposition(|definition| definition.client_code == 7)
            .and_then(|id| i32::try_from(id).ok());
        if engine_run_varp_id != Some(option_run.id) {
            return Err("option_run does not match the engine run varp".into());
        }
        let option_run_id = option_run.id;
        let artifact_sha256 = format!("{:x}", hasher.finalize());
        Ok(Self {
            revision,
            artifact_sha256,
            skills,
            xp_tenths_by_index,
            variables,
            objects,
            option_run_id,
            option_run_on,
        })
    }

    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }
    pub(super) fn resolve_pack_name(
        &self,
        selected: &SelectedGameData,
        table: NameTable,
        name: &str,
    ) -> Result<ResolvedName, String> {
        validate_name(name, table.as_str())?;
        let (resolved_name, id) = match table {
            NameTable::Item => {
                let engine_id = self.objects.get(name).copied().ok_or_else(|| {
                    "item alias is absent from selected engine definitions".to_string()
                })?;
                let item = selected
                    .item_by_alias(name)
                    .ok_or_else(|| "item alias is absent from selected game data".to_string())?;
                if item.id != engine_id {
                    return Err("item alias does not match selected engine and game data".into());
                }
                (name.to_owned(), engine_id)
            }
            NameTable::Varp | NameTable::Varbit => {
                let variable = self.variables.get(name).ok_or_else(|| {
                    "variable name is absent from selected definitions".to_string()
                })?;
                let is_varbit = variable.start_bit.is_some();
                if variable.kind != VariableKind::Integer
                    || is_varbit != matches!(table, NameTable::Varbit)
                {
                    return Err(format!(
                        "selected variable {name:?} is not a numeric {}",
                        table.as_str()
                    ));
                }
                (variable.name.clone(), variable.id)
            }
            NameTable::Skill => {
                let skill = self.skills.get(name).ok_or_else(|| {
                    "skill name is absent from selected engine definitions".to_string()
                })?;
                let index = usize::try_from(skill.id)
                    .map_err(|_| "selected skill has an invalid id".to_string())?;
                if !snapshot::stat_used(index) || snapshot::stat_name(index) != name {
                    return Err("selected skill does not match the client skill table".into());
                }
                (skill.name.clone(), skill.id)
            }
            NameTable::Npc => {
                let npc = selected
                    .npc_by_config(name)
                    .ok_or_else(|| "NPC name is absent from selected game data".to_string())?;
                (npc.config.clone(), npc.id)
            }
            NameTable::Loc => {
                let loc = selected
                    .loc_by_config(name)
                    .ok_or_else(|| "loc name is absent from selected game data".to_string())?;
                (loc.config.clone(), loc.id)
            }
        };
        Ok(ResolvedName {
            name: resolved_name,
            id,
        })
    }

    /// Read-only control for the ordered File closure barrier: the ordinary
    /// `option_run` varp. The loader already refused a varp/varbit name
    /// collision, so `getvar` takes its read-only varp branch
    /// (`ClientCheatHandler.ts:279-318`), never the protected-varbit branch.
    pub fn barrier_varp(&self) -> Result<&str, String> {
        let definition = self
            .variables
            .get("option_run")
            .filter(|definition| {
                definition.start_bit.is_none() && definition.kind == VariableKind::Integer
            })
            .ok_or("selected content has no ordinary option_run varp for the closure barrier")?;
        Ok(&definition.name)
    }

    /// Resolve the exact observer domains before the host arms the adapter.
    pub fn selection(
        &self,
        operation: BuiltinSeed,
        args: &Map<String, Value>,
    ) -> Result<DecodedDomainSelection, String> {
        Ok(self.prepare_seed(operation, args)?.domain_selection())
    }

    /// Resolve the domains for a read-only `server_var` predicate.
    pub fn server_var_selection(
        &self,
        args: &Map<String, Value>,
    ) -> Result<DecodedDomainSelection, String> {
        self.prepare_getvar(args)?;
        Ok(DecodedDomainSelection::new())
    }

    /// Send one fixed built-in seed at the caller's pre-send watermark.
    pub fn start(
        &self,
        client: &mut Client,
        snapshot: &GameSnapshot,
        game_data: &Arc<SelectedGameData>,
        request: SeedRequest<'_>,
        watermark: ScenarioObserveWatermark,
    ) -> Result<PendingSeed, String> {
        let SeedRequest {
            operation,
            args,
            why,
            policy,
        } = request;
        validate_reason(why)?;
        require_ingame_snapshot(snapshot)?;
        if game_data.revision() != self.revision {
            return Err("selected game data revision does not match fixture definitions".into());
        }
        let prepared = self.prepare_seed(operation, args)?;
        let start_tick = snapshot.tick();
        let chat_sequence = chat_head_sequence(snapshot);
        let (action, tick_budget) = match prepared {
            PreparedSeed::Teleport { x, z, level } => {
                require_fixture_idle(snapshot)?;
                // ClientCheatHandler `tele`: plane, map square, local tile.
                let command = checked_command(format!(
                    "tele {level},{},{},{},{}",
                    x >> 6,
                    z >> 6,
                    x & 63,
                    z & 63,
                ))?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::Teleport {
                        x,
                        z,
                        level,
                        command,
                    },
                    TELEPORT_OBSERVED_TICK_BUDGET,
                )
            }
            PreparedSeed::Give {
                alias,
                item_id,
                count,
            } => {
                require_inventory_ready(snapshot)?;
                let selected = game_data
                    .item_by_alias(&alias)
                    .ok_or_else(|| "item alias is absent from selected game data".to_string())?;
                if selected.id != item_id {
                    return Err("item alias id does not match selected server data".into());
                }
                let before_count = inventory_count(snapshot, item_id)?;
                let command = checked_command(format!("give {alias} {count}"))?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::Give {
                        alias,
                        item_id,
                        count,
                        before_count,
                        command,
                        receipt: None,
                    },
                    DEFAULT_OBSERVED_TICK_BUDGET,
                )
            }
            PreparedSeed::GiveBank {
                alias,
                item_id,
                count,
            } => {
                if policy == SeedPolicy::ConfirmedOnly {
                    return Err(
                        "give_bank is unconfirmed and confirmed-only policy refuses it".into(),
                    );
                }
                let selected = game_data
                    .item_by_alias(&alias)
                    .ok_or_else(|| "item alias is absent from selected game data".to_string())?;
                if selected.id != item_id {
                    return Err("item alias id does not match selected server data".into());
                }
                let command = checked_command(format!("givebank {alias} {count}"))?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::GiveBank {
                        alias,
                        item_id,
                        count,
                        command,
                    },
                    BANK_UNCONFIRMED_OBSERVED_TICKS,
                )
            }
            PreparedSeed::ClearInventory => {
                require_inventory_ready(snapshot)?;
                let command = checked_command("~clearinv".to_string())?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::ClearInventory {
                        command,
                        reply: None,
                        receipt: None,
                    },
                    DEFAULT_OBSERVED_TICK_BUDGET,
                )
            }
            PreparedSeed::ClearBank => {
                if policy == SeedPolicy::ConfirmedOnly {
                    return Err(
                        "clear_bank is unconfirmed and confirmed-only policy refuses it".into(),
                    );
                }
                require_fixture_idle(snapshot)?;
                let command = checked_command("~clearbank".to_string())?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::ClearBank { command },
                    BANK_UNCONFIRMED_OBSERVED_TICKS,
                )
            }
            PreparedSeed::SetStat {
                skill,
                skill_id,
                level,
                xp_tenths,
            } => {
                require_snapshot_skill(snapshot, skill_id, &skill)?;
                let command = checked_command(format!("setstat {skill} {level}"))?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::Stat {
                        skill,
                        skill_id,
                        level,
                        xp_wire: xp_tenths / 10,
                        command,
                        receipt: None,
                    },
                    DEFAULT_OBSERVED_TICK_BUDGET,
                )
            }
            PreparedSeed::ResetAndAdvanceStat {
                skill,
                skill_id,
                level,
                xp_tenths,
            } => {
                require_fixture_idle(snapshot)?;
                require_snapshot_skill(snapshot, skill_id, &skill)?;
                let command = checked_command(format!("advancestat {skill} {level}"))?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::Stat {
                        skill,
                        skill_id,
                        level,
                        xp_wire: xp_tenths / 10,
                        command,
                        receipt: None,
                    },
                    DEFAULT_OBSERVED_TICK_BUDGET,
                )
            }
            PreparedSeed::SetVar {
                variable,
                value,
                confirmation,
            } => {
                if variable.protect {
                    require_fixture_idle(snapshot)?;
                }
                if confirmation == SetVarConfirmation::Operation && !variable.transmit {
                    return Err(
                        "set_var operation confirmation requires a transmitting base varp".into(),
                    );
                }
                let set_command = checked_command(format!("setvar {} {value}", variable.name))?;
                let get_command = checked_command(format!("getvar {}", variable.name))?;
                send_cheat_once(client, &set_command)?;
                send_cheat_once(client, &get_command)?;
                (
                    PendingAction::SetVar {
                        variable: variable.clone(),
                        value,
                        confirmation,
                        set_command,
                        get_command,
                        readback: GetVarWait::new(
                            &variable.name,
                            ExpectedValue::Integer(value),
                            chat_sequence,
                        ),
                        readback_reply: None,
                        receipt: None,
                    },
                    DEFAULT_OBSERVED_TICK_BUDGET,
                )
            }
            PreparedSeed::EnergyFullRunOn {
                run_varp_id,
                run_on_value,
            } => {
                require_fixture_idle(snapshot)?;
                let command = checked_command("~energy".to_string())?;
                send_cheat_once(client, &command)?;
                (
                    PendingAction::EnergyFullRunOn {
                        command,
                        run_varp_id,
                        run_on_value,
                        reply: None,
                        receipt: None,
                    },
                    DEFAULT_OBSERVED_TICK_BUDGET,
                )
            }
        };

        Ok(PendingSeed {
            operation,
            why: why.to_owned().into_boxed_str(),
            artifact_sha256: self.artifact_sha256.clone().into_boxed_str(),
            baseline: watermark,
            start_tick,
            chat_sequence,
            tick_budget,
            action,
        })
    }

    /// Send one ordinary `getvar` query for a `server_var` predicate.
    pub fn start_server_var(
        &self,
        client: &mut Client,
        snapshot: &GameSnapshot,
        args: &Map<String, Value>,
        why: &str,
        watermark: ScenarioObserveWatermark,
    ) -> Result<PendingGetvar, String> {
        validate_reason(why)?;
        require_ingame_snapshot(snapshot)?;
        let (definition, expected) = self.prepare_getvar(args)?;
        if definition.protect && definition.start_bit.is_some() {
            require_fixture_idle(snapshot)?;
        }
        let command = checked_command(format!("getvar {}", definition.name))?;
        let chat_sequence = chat_head_sequence(snapshot);
        send_cheat_once(client, &command)?;
        Ok(PendingGetvar {
            why: why.to_owned().into_boxed_str(),
            artifact_sha256: self.artifact_sha256.clone().into_boxed_str(),
            baseline: watermark,
            start_tick: snapshot.tick(),
            tick_budget: DEFAULT_OBSERVED_TICK_BUDGET,
            command,
            definition: definition.clone(),
            readback: GetVarWait::new(&definition.name, expected, chat_sequence),
            reply: None,
        })
    }

    fn prepare_seed(
        &self,
        operation: BuiltinSeed,
        args: &Map<String, Value>,
    ) -> Result<PreparedSeed, String> {
        match operation {
            BuiltinSeed::Teleport => {
                require_keys(args, &["x", "z", "level"], &[])?;
                Ok(PreparedSeed::Teleport {
                    x: integer_arg(args, "x", 0, 16_383)?,
                    z: integer_arg(args, "z", 0, 16_383)?,
                    level: integer_arg(args, "level", 0, 3)?,
                })
            }
            BuiltinSeed::Give | BuiltinSeed::GiveBank => {
                require_keys(args, &["item", "count"], &[])?;
                let alias = string_arg(args, "item")?.to_owned();
                validate_name(&alias, "item")?;
                let item_id = *self
                    .objects
                    .get(&alias)
                    .ok_or_else(|| "item alias is not present in selected obj.pack".to_string())?;
                let count = integer_arg(args, "count", 1, i32::MAX as i64)?;
                if operation == BuiltinSeed::Give {
                    Ok(PreparedSeed::Give {
                        alias,
                        item_id,
                        count,
                    })
                } else {
                    Ok(PreparedSeed::GiveBank {
                        alias,
                        item_id,
                        count,
                    })
                }
            }
            BuiltinSeed::ClearInventory => {
                require_keys(args, &[], &[])?;
                Ok(PreparedSeed::ClearInventory)
            }
            BuiltinSeed::ClearBank => {
                require_keys(args, &[], &[])?;
                Ok(PreparedSeed::ClearBank)
            }
            BuiltinSeed::SetStat | BuiltinSeed::ResetAndAdvanceStat => {
                require_keys(args, &["skill", "level"], &[])?;
                let skill = string_arg(args, "skill")?.to_owned();
                validate_name(&skill, "skill")?;
                let skill_def = self
                    .skills
                    .get(&skill)
                    .ok_or_else(|| "skill is not enabled by selected engine content".to_string())?;
                let (min, max) = if operation == BuiltinSeed::SetStat {
                    (1, 99)
                } else {
                    (2, 99)
                };
                let level = integer_arg(args, "level", min, max)?;
                let xp_tenths = engine_xp_tenths(&self.xp_tenths_by_index, level)?;
                if operation == BuiltinSeed::SetStat {
                    Ok(PreparedSeed::SetStat {
                        skill,
                        skill_id: skill_def.id,
                        level,
                        xp_tenths,
                    })
                } else {
                    Ok(PreparedSeed::ResetAndAdvanceStat {
                        skill,
                        skill_id: skill_def.id,
                        level,
                        xp_tenths,
                    })
                }
            }
            BuiltinSeed::SetVar => {
                require_keys(args, &["name", "value"], &["confirmation"])?;
                let name = string_arg(args, "name")?;
                validate_name(name, "variable")?;
                let variable = self.variables.get(name).ok_or_else(|| {
                    "variable is not present in selected profile definitions".to_string()
                })?;
                if !matches!(variable.scope, 0 | 1) {
                    return Err("variable has an unsupported varp scope".into());
                }
                if variable.kind != VariableKind::Integer {
                    return Err("set_var only accepts numeric INT varps or varbits".into());
                }
                let (min, max) = variable.numeric_range();
                let value = integer_arg(args, "value", min, max)?;
                let confirmation = match args.get("confirmation") {
                    None => SetVarConfirmation::State,
                    Some(Value::String(value)) if value == "state" => SetVarConfirmation::State,
                    Some(Value::String(value)) if value == "operation" => {
                        SetVarConfirmation::Operation
                    }
                    _ => return Err("set_var confirmation must be state or operation".into()),
                };
                if confirmation == SetVarConfirmation::Operation && !variable.transmit {
                    return Err(
                        "set_var operation confirmation requires a transmitting base varp".into(),
                    );
                }
                Ok(PreparedSeed::SetVar {
                    variable: variable.clone(),
                    value,
                    confirmation,
                })
            }
            BuiltinSeed::EnergyFullRunOn => {
                require_keys(args, &[], &[])?;
                Ok(PreparedSeed::EnergyFullRunOn {
                    run_varp_id: self.option_run_id,
                    run_on_value: self.option_run_on,
                })
            }
        }
    }

    fn prepare_getvar(
        &self,
        args: &Map<String, Value>,
    ) -> Result<(VariableDefinition, ExpectedValue), String> {
        require_keys(args, &["name", "value"], &[])?;
        let name = string_arg(args, "name")?;
        validate_name(name, "variable")?;
        let definition = self
            .variables
            .get(name)
            .ok_or_else(|| "variable is not present in selected profile definitions".to_string())?;
        if !matches!(definition.scope, 0 | 1) {
            return Err("variable has an unsupported varp scope".into());
        }
        let expected = match definition.kind {
            VariableKind::Integer => {
                let (min, max) = definition.numeric_range();
                ExpectedValue::Integer(integer_arg(args, "value", min, max)?)
            }
            VariableKind::String => match args.get("value") {
                Some(Value::String(value)) => ExpectedValue::String(value.clone()),
                _ => return Err("string varp checks require a string value".into()),
            },
            VariableKind::Unsupported(type_code) => {
                return Err(format!(
                    "server_var does not support variable type {type_code}"
                ));
            }
        };
        Ok((definition.clone(), expected))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetVarConfirmation {
    State,
    Operation,
}

#[derive(Debug, Clone)]
enum ExpectedValue {
    Integer(i32),
    String(String),
}

impl ExpectedValue {
    fn text(&self) -> String {
        match self {
            Self::Integer(value) => value.to_string(),
            Self::String(value) => value.clone(),
        }
    }

    fn json_value(&self) -> Value {
        match self {
            Self::Integer(value) => json!(value),
            Self::String(value) => json!(value),
        }
    }
}

enum PreparedSeed {
    Teleport {
        x: i32,
        z: i32,
        level: i32,
    },
    Give {
        alias: String,
        item_id: i32,
        count: i32,
    },
    GiveBank {
        alias: String,
        item_id: i32,
        count: i32,
    },
    ClearInventory,
    ClearBank,
    SetStat {
        skill: String,
        skill_id: i32,
        level: i32,
        xp_tenths: i32,
    },
    ResetAndAdvanceStat {
        skill: String,
        skill_id: i32,
        level: i32,
        xp_tenths: i32,
    },
    SetVar {
        variable: VariableDefinition,
        value: i32,
        confirmation: SetVarConfirmation,
    },
    EnergyFullRunOn {
        run_varp_id: i32,
        run_on_value: i32,
    },
}

impl PreparedSeed {
    fn domain_selection(&self) -> DecodedDomainSelection {
        match self {
            Self::Give { .. } | Self::ClearInventory => {
                DecodedDomainSelection::new().carried_inventory()
            }
            Self::SetStat { skill_id, .. } | Self::ResetAndAdvanceStat { skill_id, .. } => {
                DecodedDomainSelection::new().stat_skill(*skill_id)
            }
            Self::SetVar {
                variable,
                confirmation: SetVarConfirmation::Operation,
                ..
            } if variable.transmit => DecodedDomainSelection::new().varp(variable.base_varp),
            Self::EnergyFullRunOn { run_varp_id, .. } => {
                DecodedDomainSelection::new().varp(*run_varp_id)
            }
            Self::Teleport { .. }
            | Self::GiveBank { .. }
            | Self::ClearBank
            | Self::SetVar { .. } => DecodedDomainSelection::new(),
        }
    }
}

struct GetVarWait {
    prefix: String,
    expected: ExpectedValue,
    expected_text: String,
    chat_sequence: Option<i32>,
}

impl GetVarWait {
    fn new(name: &str, expected: ExpectedValue, chat_sequence: Option<i32>) -> Self {
        let prefix = format!("get {name}: ");
        let expected_text = expected.text();
        Self {
            prefix,
            expected,
            expected_text,
            chat_sequence,
        }
    }

    fn observe_reply(&self, snapshot: &GameSnapshot) -> Result<Option<GetVarReply>, String> {
        let mut found = None;
        for line in snapshot.chat_lines() {
            if line.type_ != 0 || !sequence_after(line.sequence, self.chat_sequence) {
                continue;
            }
            if line.text == ACCESS_REFUSAL {
                return Err("getvar was refused by the selected engine".into());
            }
            let Some(actual) = line.text.strip_prefix(&self.prefix) else {
                continue;
            };
            if found.is_some() {
                return Err("multiple fresh getvar replies make the result ambiguous".into());
            }
            if actual != self.expected_text {
                return Err("fresh getvar reply does not equal the requested value".into());
            }
            found = Some(GetVarReply {
                line: line.text.clone(),
                sequence: line.sequence,
                value: self.expected.clone(),
            });
        }
        Ok(found)
    }
}

#[derive(Debug, Clone)]
struct GetVarReply {
    line: String,
    sequence: i32,
    value: ExpectedValue,
}

pub struct PendingGetvar {
    why: Box<str>,
    artifact_sha256: Box<str>,
    baseline: ScenarioObserveWatermark,
    start_tick: u32,
    tick_budget: u32,
    command: String,
    definition: VariableDefinition,
    readback: GetVarWait,
    reply: Option<GetVarReply>,
}

impl PendingGetvar {
    pub fn poll(
        &mut self,
        snapshot: &GameSnapshot,
        watermark: ScenarioObserveWatermark,
    ) -> Result<Option<Value>, String> {
        check_watermark(&self.baseline, &watermark)?;
        if has_fresh_access_refusal(snapshot, self.readback.chat_sequence) {
            return Err("getvar was refused by the selected engine".into());
        }
        if self.reply.is_none() {
            self.reply = self.readback.observe_reply(snapshot)?;
        }
        if let Some(reply) = &self.reply {
            let protected_varbit = self.definition.protect && self.definition.start_bit.is_some();
            if !protected_varbit || protected_effects_match(&self.definition, snapshot) {
                return Ok(Some(json!({
                    "operation": "server_var",
                    "status": "confirmed",
                    "confirmation": "fresh-getvar-reply",
                    "claim": "established-state",
                    "why": self.why.as_ref(),
                    "name": self.definition.name,
                    "expected": self.readback.expected.json_value(),
                    "observed": reply.value.json_value(),
                    "reply": reply.line,
                    "chat_sequence": reply.sequence,
                    "command": self.command,
                    "artifact_sha256": self.artifact_sha256.as_ref(),
                    "protected_varbit_effects_observed": protected_varbit,
                })));
            }
        }
        let elapsed = snapshot.tick().wrapping_sub(self.start_tick);
        if elapsed >= self.tick_budget {
            return Err(
                "server_var did not satisfy its exact reply and protected-path effects within 20 observed ticks"
                    .into(),
            );
        }
        Ok(None)
    }
}

pub struct PendingSeed {
    operation: BuiltinSeed,
    why: Box<str>,
    artifact_sha256: Box<str>,
    baseline: ScenarioObserveWatermark,
    start_tick: u32,
    chat_sequence: Option<i32>,
    tick_budget: u32,
    action: PendingAction,
}

enum PendingAction {
    Teleport {
        x: i32,
        z: i32,
        level: i32,
        command: String,
    },
    Give {
        alias: String,
        item_id: i32,
        count: i32,
        before_count: i64,
        command: String,
        receipt: Option<InventoryObservation>,
    },
    GiveBank {
        alias: String,
        item_id: i32,
        count: i32,
        command: String,
    },
    ClearInventory {
        command: String,
        reply: Option<ChatObservation>,
        receipt: Option<InventoryObservation>,
    },
    ClearBank {
        command: String,
    },
    Stat {
        skill: String,
        skill_id: i32,
        level: i32,
        xp_wire: i32,
        command: String,
        receipt: Option<StatObservation>,
    },
    SetVar {
        variable: VariableDefinition,
        value: i32,
        confirmation: SetVarConfirmation,
        set_command: String,
        get_command: String,
        readback: GetVarWait,
        readback_reply: Option<GetVarReply>,
        receipt: Option<VarpObservation>,
    },
    EnergyFullRunOn {
        command: String,
        run_varp_id: i32,
        run_on_value: i32,
        reply: Option<ChatObservation>,
        receipt: Option<VarpObservation>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DomainReceipt {
    source_serial: u64,
    application_serial: u64,
    application_order: u32,
}

#[derive(Debug, Clone)]
struct InventoryObservation {
    receipt: DomainReceipt,
    component: i32,
    slot: Option<usize>,
    item_id: Option<i32>,
    count: Option<i32>,
    form: &'static str,
}

#[derive(Debug, Clone)]
struct StatObservation {
    receipt: DomainReceipt,
    skill_id: i32,
    current_level: i32,
    wire_xp: i32,
}

#[derive(Debug, Clone)]
struct VarpObservation {
    receipt: DomainReceipt,
    id: i32,
    value: i32,
}

#[derive(Debug, Clone)]
pub(super) struct ChatObservation {
    pub(super) line: String,
    pub(super) sequence: i32,
}

impl PendingSeed {
    pub(super) fn sent_commands(&self) -> (&str, Option<&str>) {
        match &self.action {
            PendingAction::Teleport { command, .. }
            | PendingAction::Give { command, .. }
            | PendingAction::GiveBank { command, .. }
            | PendingAction::ClearInventory { command, .. }
            | PendingAction::ClearBank { command }
            | PendingAction::Stat { command, .. }
            | PendingAction::EnergyFullRunOn { command, .. } => (command, None),
            PendingAction::SetVar {
                set_command,
                get_command,
                ..
            } => (set_command, Some(get_command)),
        }
    }

    pub fn poll(
        &mut self,
        _client: &mut Client,
        snapshot: &GameSnapshot,
        watermark: ScenarioObserveWatermark,
        events: &[Option<AppliedEvent>],
    ) -> Result<Option<Value>, String> {
        check_watermark(&self.baseline, &watermark)?;
        if has_fresh_access_refusal(snapshot, self.chat_sequence) {
            return Err(
                "fixture command was refused: Please finish what you're doing first.".into(),
            );
        }
        self.update_receipts(events, &watermark)?;
        self.update_replies(snapshot)?;
        let elapsed = snapshot.tick().wrapping_sub(self.start_tick);

        match &self.action {
            PendingAction::Teleport {
                x,
                z,
                level,
                command,
            } => {
                if elapsed > 0 && teleport_state_matches(snapshot, *x, *z, *level) {
                    return Ok(Some(json!({
                        "operation": self.operation.name(),
                        "status": "confirmed",
                        "confirmation": "established-state",
                        "why": self.why.as_ref(),
                        "requested": {"x": x, "z": z, "level": level},
                        "command": command,
                        "send": "sent",
                        "observed": {
                            "tile_x": snapshot.tile().map(|tile| tile.0),
                            "tile_z": snapshot.tile().map(|tile| tile.1),
                            "scene_level": snapshot.scene().level,
                            "scene_available": snapshot.scene().available,
                            "player_moving": false,
                            "interfaces_closed": interfaces_closed(snapshot),
                            "map_flag_clear": snapshot.map_flag().is_none(),
                            "player_info_ticks": elapsed,
                            "interaction_cancelled": "declared-engine-effect-not-inferred",
                        },
                        "artifact_sha256": self.artifact_sha256.as_ref(),
                    })));
                }
            }
            PendingAction::Give {
                alias,
                item_id,
                count,
                before_count,
                command,
                receipt,
            } => {
                if let Some(receipt) = receipt {
                    let after_count = inventory_count(snapshot, *item_id)?;
                    let expected = *before_count + i64::from(*count);
                    if snapshot.inventory_size() > 0
                        && after_count == expected
                        && receipt.item_id == Some(*item_id)
                    {
                        return Ok(Some(json!({
                            "operation": self.operation.name(),
                            "status": "confirmed",
                            "confirmation": "confirmed-operation",
                            "why": self.why.as_ref(),
                            "requested": {"item": alias, "item_id": item_id, "count": count},
                            "command": command,
                            "send": "sent",
                            "inventory": {
                                "before_count": before_count,
                                "after_count": after_count,
                                "delta": count,
                                "receipt": receipt_json(receipt),
                            },
                            "artifact_sha256": self.artifact_sha256.as_ref(),
                        })));
                    }
                }
            }
            PendingAction::GiveBank {
                alias,
                item_id,
                count,
                command,
            } => {
                if elapsed >= BANK_UNCONFIRMED_OBSERVED_TICKS {
                    return Ok(Some(unconfirmed_bank_result(
                        self,
                        command,
                        json!({"item": alias, "item_id": item_id, "count": count}),
                        elapsed,
                    )));
                }
            }
            PendingAction::ClearInventory {
                command,
                reply,
                receipt,
            } => {
                if let Some(reply) = reply {
                    if snapshot.inventory_size() > 0 && snapshot.inv().is_empty() {
                        return Ok(Some(json!({
                            "operation": self.operation.name(),
                            "status": "confirmed",
                            "confirmation": "confirmed-operation",
                            "why": self.why.as_ref(),
                            "command": command,
                            "send": "sent",
                            "reply": reply.line,
                            "chat_sequence": reply.sequence,
                            "inventory_empty": true,
                            "inventory_receipt": receipt.as_ref().map(inventory_receipt_json),
                            "artifact_sha256": self.artifact_sha256.as_ref(),
                        })));
                    }
                }
            }
            PendingAction::ClearBank { command } => {
                if elapsed >= BANK_UNCONFIRMED_OBSERVED_TICKS {
                    return Ok(Some(unconfirmed_bank_result(
                        self,
                        command,
                        json!({}),
                        elapsed,
                    )));
                }
            }
            PendingAction::Stat {
                skill,
                skill_id,
                level,
                xp_wire,
                command,
                receipt,
            } => {
                if let Some(receipt) = receipt {
                    if receipt.current_level == *level
                        && receipt.wire_xp == *xp_wire
                        && snapshot_stat_matches(snapshot, *skill_id, *level, *xp_wire)
                    {
                        return Ok(Some(json!({
                            "operation": self.operation.name(),
                            "status": "confirmed",
                            "confirmation": "confirmed-operation",
                            "why": self.why.as_ref(),
                            "requested": {"skill": skill, "skill_id": skill_id, "level": level},
                            "command": command,
                            "send": "sent",
                            "stat": {
                                "effective": level,
                                "base": level,
                                "xp_wire": xp_wire,
                                "receipt": stat_receipt_json(receipt),
                            },
                            "artifact_sha256": self.artifact_sha256.as_ref(),
                        })));
                    }
                }
            }
            PendingAction::SetVar {
                variable,
                value,
                confirmation,
                set_command,
                get_command,
                readback_reply,
                receipt,
                ..
            } => {
                if let Some(reply) = readback_reply {
                    if protected_effects_match(variable, snapshot) {
                        let base_varp_value = if variable.transmit {
                            snapshot_base_varp_value(variable, snapshot)
                        } else {
                            None
                        };
                        let client_value_matches = !variable.transmit
                            || base_varp_value.is_some_and(|observed| {
                                variable_value_matches(variable, *value, observed)
                            });
                        let matching_receipt = receipt.as_ref().filter(|observed| {
                            varp_state_matches(variable, *value, observed, snapshot)
                        });
                        let confirmed = match confirmation {
                            SetVarConfirmation::State => client_value_matches,
                            SetVarConfirmation::Operation => {
                                matching_receipt.is_some_and(|event| {
                                    event.receipt.source_serial > self.baseline.source_serial
                                })
                            }
                        };
                        if confirmed {
                            return Ok(Some(set_var_result(
                                self,
                                variable,
                                *value,
                                *confirmation,
                                [set_command, get_command],
                                SetVarReadback {
                                    reply,
                                    base_varp_value,
                                    receipt: matching_receipt,
                                },
                            )));
                        }
                    }
                }
            }
            PendingAction::EnergyFullRunOn {
                command,
                run_varp_id,
                run_on_value,
                reply,
                receipt,
            } => {
                if let (Some(reply), Some(receipt)) = (reply, receipt) {
                    let run_varp = snapshot
                        .varps()
                        .iter()
                        .find(|row| row.index == *run_varp_id)
                        .map(|row| row.value);
                    let energy = snapshot.local_player().map(|player| player.energy);
                    if elapsed > 0
                        && run_varp == Some(*run_on_value)
                        && receipt.value == *run_on_value
                        && energy == Some(100)
                        && snapshot.runenergy() == 100
                        && interfaces_closed(snapshot)
                    {
                        return Ok(Some(json!({
                            "operation": self.operation.name(),
                            "status": "confirmed",
                            "confirmation": "direct-reply-plus-established-state",
                            "why": self.why.as_ref(),
                            "command": command,
                            "send": "sent",
                            "reply": reply.line,
                            "chat_sequence": reply.sequence,
                            "energy_percent": 100,
                            "option_run": {
                                "id": run_varp_id,
                                "value": run_on_value,
                                "receipt": varp_receipt_json(receipt),
                            },
                            "interfaces_closed": true,
                            "player_info_ticks": elapsed,
                            "artifact_sha256": self.artifact_sha256.as_ref(),
                        })));
                    }
                }
            }
        }

        if elapsed >= self.tick_budget {
            return Err(format!(
                "{} did not meet its required confirmation within {} observed ticks",
                self.operation.name(),
                self.tick_budget
            ));
        }
        Ok(None)
    }

    fn update_replies(&mut self, snapshot: &GameSnapshot) -> Result<(), String> {
        match &mut self.action {
            PendingAction::ClearInventory { reply, .. } if reply.is_none() => {
                *reply = find_fresh_exact_chat(snapshot, self.chat_sequence, INVENTORY_WIPED)?;
            }
            PendingAction::SetVar {
                readback,
                readback_reply,
                ..
            } if readback_reply.is_none() => {
                *readback_reply = readback.observe_reply(snapshot)?;
            }
            PendingAction::EnergyFullRunOn { reply, .. } if reply.is_none() => {
                *reply = find_fresh_exact_chat(snapshot, self.chat_sequence, ENERGY_RESTORED)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn update_receipts(
        &mut self,
        events: &[Option<AppliedEvent>],
        current: &ScenarioObserveWatermark,
    ) -> Result<(), String> {
        match &mut self.action {
            PendingAction::Give {
                item_id, receipt, ..
            } => update_inventory_receipt(
                &self.baseline,
                current,
                events,
                Some(*item_id),
                Some(InventoryScope::Carried),
                receipt,
            ),
            PendingAction::ClearInventory { receipt, .. } => update_inventory_receipt(
                &self.baseline,
                current,
                events,
                None,
                Some(InventoryScope::Carried),
                receipt,
            ),
            PendingAction::Stat {
                skill_id, receipt, ..
            } => update_stat_receipt(&self.baseline, current, events, *skill_id, receipt),
            PendingAction::SetVar {
                variable,
                confirmation,
                receipt,
                ..
            } => update_varp_receipt(
                &self.baseline,
                current,
                events,
                variable.base_varp,
                receipt,
                *confirmation == SetVarConfirmation::Operation,
            ),
            PendingAction::EnergyFullRunOn {
                run_varp_id,
                receipt,
                ..
            } => update_varp_receipt(&self.baseline, current, events, *run_varp_id, receipt, true),
            PendingAction::Teleport { .. }
            | PendingAction::GiveBank { .. }
            | PendingAction::ClearBank { .. } => Ok(()),
        }
    }
}

fn update_inventory_receipt(
    baseline: &ScenarioObserveWatermark,
    current: &ScenarioObserveWatermark,
    events: &[Option<AppliedEvent>],
    expected_item_id: Option<i32>,
    scope: Option<InventoryScope>,
    destination: &mut Option<InventoryObservation>,
) -> Result<(), String> {
    for applied in events.iter().flatten() {
        let (stamp, component, slot, item_id, count, form) = match &applied.event {
            DecodedDomainEvent::InventoryMarker {
                stamp,
                component,
                form,
            } => (stamp, *component, None, None, None, *form),
            DecodedDomainEvent::InventorySlot {
                stamp,
                component,
                slot,
                item_id,
                count,
                form,
            } => (
                stamp,
                *component,
                Some(*slot),
                *item_id,
                Some(*count),
                *form,
            ),
            _ => continue,
        };
        let form = match form {
            InventoryForm::Full => "full",
            InventoryForm::Partial => "partial",
            InventoryForm::StopTransmit => continue,
        };
        if scope.is_some_and(|expected| applied.inventory_scope != Some(expected)) {
            continue;
        }
        if expected_item_id.is_some_and(|expected| item_id != Some(expected)) {
            continue;
        }
        let Some(receipt) = fresh_domain_receipt(baseline, current, applied, stamp, true)? else {
            continue;
        };
        let candidate = InventoryObservation {
            receipt,
            component,
            slot,
            item_id,
            count,
            form,
        };
        if is_newer(receipt, destination.as_ref().map(|old| old.receipt)) {
            *destination = Some(candidate);
        }
    }
    Ok(())
}

fn update_stat_receipt(
    baseline: &ScenarioObserveWatermark,
    current: &ScenarioObserveWatermark,
    events: &[Option<AppliedEvent>],
    skill_id: i32,
    destination: &mut Option<StatObservation>,
) -> Result<(), String> {
    for applied in events.iter().flatten() {
        let DecodedDomainEvent::Stat {
            stamp,
            skill: observed_skill,
            current_level,
            wire_xp,
        } = &applied.event
        else {
            continue;
        };
        if *observed_skill != skill_id {
            continue;
        }
        let Some(receipt) = fresh_domain_receipt(baseline, current, applied, stamp, true)? else {
            continue;
        };
        let candidate = StatObservation {
            receipt,
            skill_id: *observed_skill,
            current_level: *current_level,
            wire_xp: *wire_xp,
        };
        if is_newer(receipt, destination.as_ref().map(|old| old.receipt)) {
            *destination = Some(candidate);
        }
    }
    Ok(())
}

fn update_varp_receipt(
    baseline: &ScenarioObserveWatermark,
    current: &ScenarioObserveWatermark,
    events: &[Option<AppliedEvent>],
    varp_id: i32,
    destination: &mut Option<VarpObservation>,
    require_source: bool,
) -> Result<(), String> {
    for applied in events.iter().flatten() {
        let DecodedDomainEvent::Varp {
            stamp, id, value, ..
        } = &applied.event
        else {
            continue;
        };
        if *id != varp_id {
            continue;
        }
        let Some(receipt) =
            fresh_domain_receipt(baseline, current, applied, stamp, require_source)?
        else {
            continue;
        };
        let candidate = VarpObservation {
            receipt,
            id: *id,
            value: *value,
        };
        if is_newer(receipt, destination.as_ref().map(|old| old.receipt)) {
            *destination = Some(candidate);
        }
    }
    Ok(())
}

fn fresh_domain_receipt(
    baseline: &ScenarioObserveWatermark,
    current: &ScenarioObserveWatermark,
    applied: &AppliedEvent,
    stamp: &api::scenario_observe::DecodedDomainStamp,
    require_source: bool,
) -> Result<Option<DomainReceipt>, String> {
    if applied.session != baseline.session || stamp.client_session != baseline.client_session {
        return Err("fixture observer returned an event from a different session".into());
    }
    if stamp.application_serial > current.application_serial
        || stamp.source_serial > current.source_serial
    {
        return Err("fixture observer returned an event beyond its current watermark".into());
    }
    if stamp.application_serial <= baseline.application_serial
        || (require_source && stamp.source_serial <= baseline.source_serial)
    {
        return Ok(None);
    }
    Ok(Some(DomainReceipt {
        source_serial: stamp.source_serial,
        application_serial: stamp.application_serial,
        application_order: stamp.application_order,
    }))
}

fn is_newer(candidate: DomainReceipt, current: Option<DomainReceipt>) -> bool {
    current.is_none_or(|old| {
        (candidate.application_serial, candidate.application_order)
            > (old.application_serial, old.application_order)
    })
}

pub(super) fn check_watermark(
    baseline: &ScenarioObserveWatermark,
    current: &ScenarioObserveWatermark,
) -> Result<(), String> {
    if baseline.session != current.session || baseline.client_session != current.client_session {
        return Err("fixture operation crossed a client session boundary".into());
    }
    if current.source_serial < baseline.source_serial
        || current.application_serial < baseline.application_serial
    {
        return Err("fixture observer watermark moved backward".into());
    }
    Ok(())
}

struct SetVarReadback<'a> {
    reply: &'a GetVarReply,
    base_varp_value: Option<i32>,
    receipt: Option<&'a VarpObservation>,
}

fn set_var_result(
    pending: &PendingSeed,
    variable: &VariableDefinition,
    value: i32,
    confirmation: SetVarConfirmation,
    commands: [&str; 2],
    readback: SetVarReadback<'_>,
) -> Value {
    let SetVarReadback {
        reply,
        base_varp_value,
        receipt,
    } = readback;
    let client_value = base_varp_value.map(|base_value| {
        json!({
            "base_varp": variable.base_varp,
            "base_value": base_value,
            "value": variable.decode_value(base_value).unwrap_or(base_value),
            "receipt": receipt.map(varp_receipt_json),
        })
    });
    json!({
        "operation": pending.operation.name(),
        "status": "confirmed",
        "confirmation": match confirmation {
            SetVarConfirmation::State => "established-state",
            SetVarConfirmation::Operation => "confirmed-operation",
        },
        "why": pending.why.as_ref(),
        "requested": {
            "name": variable.name,
            "id": variable.id,
            "base_varp": variable.base_varp,
            "value": value,
            "confirmation": match confirmation {
                SetVarConfirmation::State => "state",
                SetVarConfirmation::Operation => "operation",
            },
        },
        "commands": commands,
        "send": "sent",
        "getvar_reply": reply.line,
        "getvar_sequence": reply.sequence,
        "client_value": client_value,
        "protected_variable_effects_observed": variable.protect,
        "artifact_sha256": pending.artifact_sha256.as_ref(),
    })
}

fn snapshot_base_varp_value(variable: &VariableDefinition, snapshot: &GameSnapshot) -> Option<i32> {
    snapshot
        .varps()
        .iter()
        .find(|row| row.index == variable.base_varp)
        .map(|row| row.value)
}

fn variable_value_matches(variable: &VariableDefinition, requested: i32, base_value: i32) -> bool {
    match variable.start_bit {
        None => base_value == requested,
        Some(_) => variable.decode_value(base_value) == Some(requested),
    }
}

fn varp_state_matches(
    variable: &VariableDefinition,
    requested: i32,
    receipt: &VarpObservation,
    snapshot: &GameSnapshot,
) -> bool {
    receipt.id == variable.base_varp
        && snapshot_base_varp_value(variable, snapshot) == Some(receipt.value)
        && variable_value_matches(variable, requested, receipt.value)
}

fn snapshot_stat_matches(snapshot: &GameSnapshot, skill_id: i32, level: i32, xp_wire: i32) -> bool {
    snapshot.stats().iter().any(|stat| {
        stat.index == skill_id
            && stat.used
            && stat.effective == level
            && stat.base == level
            && stat.xp == xp_wire
    })
}

fn require_snapshot_skill(
    snapshot: &GameSnapshot,
    skill_id: i32,
    name: &str,
) -> Result<(), String> {
    let index = usize::try_from(skill_id).map_err(|_| "invalid selected skill id".to_string())?;
    if !snapshot::stat_used(index) || snapshot::stat_name(index) != name {
        return Err("selected engine skill does not match the client skill table".into());
    }
    if !snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == skill_id && stat.used && stat.name == name)
    {
        return Err("selected skill is absent from the current client snapshot".into());
    }
    Ok(())
}

fn teleport_state_matches(snapshot: &GameSnapshot, x: i32, z: i32, level: i32) -> bool {
    if !snapshot.ingame() || snapshot.scene_state() != 2 || !snapshot.scene().available {
        return false;
    }
    let Some((actual_x, actual_z, _)) = snapshot.tile() else {
        return false;
    };
    let scene = snapshot.scene();
    if actual_x != x || actual_z != z || scene.level != level {
        return false;
    }
    let Some(max_x) = scene.base_x.checked_add(scene.width) else {
        return false;
    };
    let Some(max_z) = scene.base_z.checked_add(scene.height) else {
        return false;
    };
    if actual_x < scene.base_x || actual_x >= max_x || actual_z < scene.base_z || actual_z >= max_z
    {
        return false;
    }
    snapshot
        .local_player()
        .is_some_and(|player| !player.player.actor.moving)
        && interfaces_closed(snapshot)
        && snapshot.map_flag().is_none()
}

pub(super) fn require_fixture_idle(snapshot: &GameSnapshot) -> Result<(), String> {
    if snapshot.modals().main >= 0 || snapshot.modals().chat >= 0 {
        return Err("fixture command requires no main or chat modal".into());
    }
    if snapshot
        .local_player()
        .is_none_or(|player| player.player.actor.moving)
    {
        return Err("fixture command requires a stationary local player".into());
    }
    Ok(())
}

fn protected_effects_match(variable: &VariableDefinition, snapshot: &GameSnapshot) -> bool {
    if variable.protect {
        interfaces_closed(snapshot) && snapshot.map_flag().is_none()
    } else {
        true
    }
}

fn interfaces_closed(snapshot: &GameSnapshot) -> bool {
    snapshot.modals().main < 0 && snapshot.modals().side < 0 && snapshot.modals().chat < 0
}

fn require_inventory_ready(snapshot: &GameSnapshot) -> Result<(), String> {
    if snapshot.inventory_size() <= 0 {
        return Err("carried inventory is not present in the current snapshot".into());
    }
    Ok(())
}

pub(super) fn require_ingame_snapshot(snapshot: &GameSnapshot) -> Result<(), String> {
    if !snapshot.attached() || !snapshot.ingame() || snapshot.local_player().is_none() {
        return Err("fixture command requires an attached in-game local player".into());
    }
    Ok(())
}

fn inventory_count(snapshot: &GameSnapshot, item_id: i32) -> Result<i64, String> {
    snapshot.inv().iter().try_fold(0_i64, |total, (id, count)| {
        if *id != item_id {
            return Ok(total);
        }
        total
            .checked_add(i64::from(*count))
            .ok_or_else(|| "carried inventory count overflowed".to_string())
    })
}

pub(super) fn chat_head_sequence(snapshot: &GameSnapshot) -> Option<i32> {
    snapshot.chat_lines().first().map(|line| line.sequence)
}

fn sequence_after(sequence: i32, baseline: Option<i32>) -> bool {
    match baseline {
        Some(baseline) => sequence.wrapping_sub(baseline) > 0,
        None => true,
    }
}

pub(super) fn has_fresh_access_refusal(snapshot: &GameSnapshot, baseline: Option<i32>) -> bool {
    snapshot.chat_lines().iter().any(|line| {
        line.type_ == 0 && sequence_after(line.sequence, baseline) && line.text == ACCESS_REFUSAL
    })
}

pub(super) fn find_fresh_exact_chat(
    snapshot: &GameSnapshot,
    baseline: Option<i32>,
    expected: &str,
) -> Result<Option<ChatObservation>, String> {
    let mut found = None;
    for line in snapshot.chat_lines() {
        if line.type_ != 0 || !sequence_after(line.sequence, baseline) || line.text != expected {
            continue;
        }
        if found.is_some() {
            return Err("multiple fresh exact fixture replies make the result ambiguous".into());
        }
        found = Some(ChatObservation {
            line: line.text.clone(),
            sequence: line.sequence,
        });
    }
    Ok(found)
}

fn receipt_json(receipt: &InventoryObservation) -> Value {
    inventory_receipt_json(receipt)
}

fn inventory_receipt_json(receipt: &InventoryObservation) -> Value {
    json!({
        "source_serial": receipt.receipt.source_serial,
        "application_serial": receipt.receipt.application_serial,
        "application_order": receipt.receipt.application_order,
        "component": receipt.component,
        "slot": receipt.slot,
        "item_id": receipt.item_id,
        "count": receipt.count,
        "form": receipt.form,
    })
}

fn stat_receipt_json(receipt: &StatObservation) -> Value {
    json!({
        "source_serial": receipt.receipt.source_serial,
        "application_serial": receipt.receipt.application_serial,
        "application_order": receipt.receipt.application_order,
        "skill_id": receipt.skill_id,
        "current_level": receipt.current_level,
        "wire_xp": receipt.wire_xp,
    })
}

fn varp_receipt_json(receipt: &VarpObservation) -> Value {
    json!({
        "source_serial": receipt.receipt.source_serial,
        "application_serial": receipt.receipt.application_serial,
        "application_order": receipt.receipt.application_order,
        "varp_id": receipt.id,
        "value": receipt.value,
    })
}

fn unconfirmed_bank_result(
    pending: &PendingSeed,
    command: &str,
    requested: Value,
    elapsed: u32,
) -> Value {
    json!({
        "operation": pending.operation.name(),
        "status": "unconfirmed",
        "confirmation": "unconfirmed",
        "why": pending.why.as_ref(),
        "requested": requested,
        "command": command,
        "send": "sent",
        "observed_ticks": elapsed,
        "effect_confirmed": false,
        "artifact_sha256": pending.artifact_sha256.as_ref(),
    })
}

pub(super) fn validate_reason(why: &str) -> Result<(), String> {
    if why.is_empty() || why.len() > MAX_REASON_BYTES {
        return Err("why must contain 1 through 160 bytes".into());
    }
    Ok(())
}

fn require_keys(
    args: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
) -> Result<(), String> {
    for key in required {
        if !args.contains_key(*key) {
            return Err(format!("missing required argument {key}"));
        }
    }
    for key in args.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(format!("unexpected fixture argument {key}"));
        }
    }
    Ok(())
}

fn string_arg<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("fixture argument {key} must be a string"))
}

fn integer_arg(
    args: &Map<String, Value>,
    key: &str,
    minimum: i64,
    maximum: i64,
) -> Result<i32, String> {
    let value = args
        .get(key)
        .ok_or_else(|| format!("missing required argument {key}"))?;
    let number = value
        .as_number()
        .ok_or_else(|| format!("fixture argument {key} must be an integer"))?;
    let parsed = if let Some(value) = number.as_i64() {
        value
    } else if let Some(value) = number.as_u64() {
        i64::try_from(value).map_err(|_| format!("fixture argument {key} is out of range"))?
    } else if let Some(value) = number.as_f64() {
        if !value.is_finite() || value.fract() != 0.0 {
            return Err(format!(
                "fixture argument {key} must be finite and integral"
            ));
        }
        if value < minimum as f64 || value > maximum as f64 {
            return Err(format!("fixture argument {key} is out of range"));
        }
        value as i64
    } else {
        return Err(format!("fixture argument {key} must be an integer"));
    };
    if parsed < minimum || parsed > maximum {
        return Err(format!("fixture argument {key} is out of range"));
    }
    i32::try_from(parsed).map_err(|_| format!("fixture argument {key} is out of range"))
}

pub(super) fn validate_name(name: &str, field: &str) -> Result<(), String> {
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| !(0x21..=0x7e).contains(&byte) || byte.is_ascii_uppercase())
    {
        return Err(format!(
            "{field} must be printable lower-case ASCII without whitespace"
        ));
    }
    Ok(())
}

pub(super) fn checked_command(command: String) -> Result<String, String> {
    if command.len() > MAX_COMMAND_BYTES {
        return Err("fixture command exceeds the engine's 80-byte line limit".into());
    }
    Ok(command)
}

pub(super) fn send_cheat_once(client: &mut Client, command: &str) -> Result<(), String> {
    match api::interact::cheat(client, command) {
        client::CheatSend::Sent => Ok(()),
        client::CheatSend::Refused(reason) => {
            Err(format!("fixture command send was refused: {reason:?}"))
        }
    }
}

fn read_hashed(
    path: &Path,
    label: &str,
    maximum: u64,
    hasher: &mut Sha256,
) -> Result<Vec<u8>, String> {
    let bytes = read_bounded(path, label, maximum)?;
    let label_len =
        u64::try_from(label.len()).map_err(|_| "fixture label is too long".to_string())?;
    let byte_len =
        u64::try_from(bytes.len()).map_err(|_| "fixture input is too large".to_string())?;
    hasher.update(label_len.to_be_bytes());
    hasher.update(label.as_bytes());
    hasher.update(byte_len.to_be_bytes());
    hasher.update(&bytes);
    Ok(bytes)
}

fn read_bounded(path: &Path, label: &str, maximum: u64) -> Result<Vec<u8>, String> {
    let file =
        File::open(path).map_err(|error| format!("could not open {label}: {:?}", error.kind()))?;
    let declared = file
        .metadata()
        .map_err(|error| format!("could not read {label} metadata: {:?}", error.kind()))?
        .len();
    if declared > maximum {
        return Err(format!("{label} exceeds its bounded input size"));
    }
    let capacity = usize::try_from(declared).map_err(|_| format!("{label} is too large"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {label}: {:?}", error.kind()))?;
    if bytes.len() > capacity || bytes.len() as u64 != declared {
        return Err(format!("{label} changed while it was being read"));
    }
    Ok(bytes)
}

fn source_text(bytes: Vec<u8>, label: &str) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| format!("{label} is not valid UTF-8"))
}

fn parse_name_pack(bytes: &[u8], label: &str) -> Result<BTreeMap<i32, String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{label} is not valid UTF-8"))?;
    let mut by_id = BTreeMap::new();
    let mut names = BTreeSet::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((id_text, name_text)) = line.split_once('=') else {
            return Err(format!("{label} line {} is not id=name", line_number + 1));
        };
        let id = id_text
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("{label} contains an invalid id"))?;
        if id < 0 {
            return Err(format!("{label} contains a negative id"));
        }
        let name = name_text.trim();
        validate_name(name, label)?;
        if by_id.insert(id, name.to_owned()).is_some() || !names.insert(name.to_owned()) {
            return Err(format!("{label} contains a duplicate id or name"));
        }
    }
    Ok(by_id)
}

fn validate_pack_ids(
    pack: &BTreeMap<i32, String>,
    definition_count: usize,
    label: &str,
) -> Result<(), String> {
    if pack
        .keys()
        .any(|id| usize::try_from(*id).map_or(true, |id| id >= definition_count))
    {
        return Err(format!("{label} references an id outside its server table"));
    }
    Ok(())
}

fn record_count(bytes: &[u8], label: &str) -> Result<usize, String> {
    if bytes.len() < 2 {
        return Err(format!("{label} has no record count"));
    }
    Ok(usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    label: &'static str,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], label: &'static str) -> Self {
        Self {
            bytes,
            offset: 0,
            label,
        }
    }

    fn byte(&mut self) -> Result<u8, String> {
        let value = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| format!("{} is truncated at byte {}", self.label, self.offset))?;
        self.offset += 1;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let high = self.byte()?;
        let low = self.byte()?;
        Ok(u16::from_be_bytes([high, low]))
    }

    fn packed_name(&mut self) -> Result<String, String> {
        let start = self.offset;
        while let Some(byte) = self.bytes.get(self.offset) {
            // Engine Packet.gjstr defaults to LF, not NUL.
            if *byte == b'\n' {
                let string = std::str::from_utf8(&self.bytes[start..self.offset])
                    .map_err(|_| format!("{} contains invalid UTF-8", self.label))?
                    .to_owned();
                self.offset += 1;
                return Ok(string);
            }
            self.offset += 1;
        }
        Err(format!("{} has an unterminated string", self.label))
    }

    fn finish(self) -> Result<(), String> {
        if self.offset != self.bytes.len() {
            return Err(format!("{} contains trailing bytes", self.label));
        }
        Ok(())
    }
}

fn decode_varp_and_varbit_records_with_archive(
    server_varp_bytes: Vec<u8>,
    server_varbit_bytes: Vec<u8>,
    config_bytes: Vec<u8>,
) -> Result<(Vec<VarpDefinition>, Vec<RawVarbitDefinition>), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let jag = JagFile::new(config_bytes);
        let client_varp_bytes = jag
            .read("varp.dat")
            .ok_or_else(|| "selected client config has no varp.dat".to_string())?;
        let client_varbit_bytes = jag
            .read("varbit.dat")
            .ok_or_else(|| "selected client config has no varbit.dat".to_string())?;
        let varps = decode_varps(&server_varp_bytes, &client_varp_bytes)?;
        let varbits = decode_varbits(&server_varbit_bytes, &client_varbit_bytes)?;
        Ok((varps, varbits))
    }))
    .map_err(|_| "selected client config archive could not be decoded".to_string())?
}

fn decode_varps(server_bytes: &[u8], client_bytes: &[u8]) -> Result<Vec<VarpDefinition>, String> {
    let mut server = Cursor::new(server_bytes, "server varp.dat");
    let mut client = Cursor::new(client_bytes, "client varp.dat");
    let server_count = usize::from(server.u16()?);
    let client_count = usize::from(client.u16()?);
    if server_count != client_count {
        return Err("client and server varp counts do not match".into());
    }
    let mut definitions = Vec::with_capacity(server_count);
    for _ in 0..server_count {
        let mut definition = VarpDefinition {
            debug_name: String::new(),
            kind: VariableKind::Integer,
            scope: 0,
            client_code: 0,
            protect: true,
            transmit: false,
        };
        decode_varp_record(&mut server, &mut definition)?;
        decode_varp_record(&mut client, &mut definition)?;
        definitions.push(definition);
    }
    server.finish()?;
    client.finish()?;
    Ok(definitions)
}

fn decode_varp_record(
    cursor: &mut Cursor<'_>,
    definition: &mut VarpDefinition,
) -> Result<(), String> {
    loop {
        let code_offset = cursor.offset;
        let code = cursor.byte()?;
        match code {
            0 => return Ok(()),
            1 => definition.scope = cursor.byte()?,
            2 => {
                definition.kind = match cursor.byte()? {
                    b'i' => VariableKind::Integer,
                    b's' => VariableKind::String,
                    type_code => VariableKind::Unsupported(type_code),
                };
            }
            4 => definition.protect = false,
            5 => definition.client_code = cursor.u16()?,
            6 => definition.transmit = true,
            250 => definition.debug_name = cursor.packed_name()?,
            _ => {
                return Err(format!(
                    "{} has unsupported opcode {code} at byte {code_offset}",
                    cursor.label
                ));
            }
        }
    }
}

fn decode_varbits(
    server_bytes: &[u8],
    client_bytes: &[u8],
) -> Result<Vec<RawVarbitDefinition>, String> {
    let mut server = Cursor::new(server_bytes, "server varbit.dat");
    let mut client = Cursor::new(client_bytes, "client varbit.dat");
    let server_count = usize::from(server.u16()?);
    let client_count = usize::from(client.u16()?);
    if server_count != client_count {
        return Err("client and server varbit counts do not match".into());
    }
    let mut definitions = Vec::with_capacity(server_count);
    for _ in 0..server_count {
        let mut definition = RawVarbitDefinition {
            debug_name: String::new(),
            base_varp: -1,
            start_bit: u8::MAX,
            end_bit: u8::MAX,
        };
        decode_varbit_record(&mut server, &mut definition)?;
        decode_varbit_record(&mut client, &mut definition)?;
        let undefined = definition.base_varp < 0
            && definition.start_bit == u8::MAX
            && definition.end_bit == u8::MAX;
        if !undefined
            && (definition.base_varp < 0
                || definition.start_bit > definition.end_bit
                || definition.end_bit >= 32)
        {
            return Err("varbit definitions contain invalid inclusive bit bounds".into());
        }
        definitions.push(definition);
    }
    server.finish()?;
    client.finish()?;
    Ok(definitions)
}

fn decode_varbit_record(
    cursor: &mut Cursor<'_>,
    definition: &mut RawVarbitDefinition,
) -> Result<(), String> {
    loop {
        let code = cursor.byte()?;
        match code {
            0 => return Ok(()),
            1 => {
                definition.base_varp = i32::from(cursor.u16()?);
                definition.start_bit = cursor.byte()?;
                definition.end_bit = cursor.byte()?;
            }
            250 => definition.debug_name = cursor.packed_name()?,
            _ => {
                return Err(format!("{} has unsupported opcode {code}", cursor.label));
            }
        }
    }
}

fn validate_unique_variable_names(
    variables: &BTreeMap<String, VariableDefinition>,
) -> Result<(), String> {
    for (name, variable) in variables {
        if name != &variable.name {
            return Err("variable name index is inconsistent".into());
        }
    }
    Ok(())
}

fn parse_engine_skills(
    source: &str,
) -> Result<(BTreeMap<String, SkillDefinition>, Vec<bool>), String> {
    let enum_body = delimited_body(source, "enum PlayerStat", b'{', b'}')?;
    let enum_source = strip_comments(enum_body);
    let mut enum_ids = BTreeMap::new();
    let mut next_id = 0_usize;
    for raw in enum_source.split(',') {
        let entry = raw.trim();
        if entry.is_empty() {
            continue;
        }
        let (name, explicit_id) = match entry.split_once('=') {
            Some((name, id)) => (
                name.trim(),
                Some(
                    id.trim()
                        .parse::<usize>()
                        .map_err(|_| "PlayerStat enum has an invalid explicit id".to_string())?,
                ),
            ),
            None => (entry, None),
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("PlayerStat enum contains an invalid identifier".into());
        }
        let id = explicit_id.unwrap_or(next_id);
        if id != next_id || enum_ids.insert(name.to_owned(), id).is_some() {
            return Err("PlayerStat enum ids are not unique and contiguous".into());
        }
        next_id += 1;
    }
    if enum_ids.is_empty() {
        return Err("PlayerStat enum is empty".into());
    }

    let map_body = delimited_body(source, "PlayerStatMap", b'[', b']')?;
    let mut names_by_id = BTreeMap::new();
    let mut aliases = BTreeSet::new();
    let mut rest = map_body;
    while let Some(open) = rest.find('[') {
        let after_open = &rest[open + 1..];
        let close = after_open
            .find(']')
            .ok_or_else(|| "PlayerStatMap contains an unterminated row".to_string())?;
        let row = after_open[..close].trim();
        let (alias_text, enum_text) = row
            .split_once(',')
            .ok_or_else(|| "PlayerStatMap row is missing its enum value".to_string())?;
        let raw_alias = parse_quoted(alias_text.trim())?;
        if raw_alias.bytes().any(|byte| byte.is_ascii_lowercase()) {
            return Err("PlayerStatMap alias cannot be found by uppercase command lookup".into());
        }
        let alias = raw_alias.to_ascii_lowercase();
        validate_name(&alias, "skill alias")?;
        let enum_name = enum_text
            .trim()
            .strip_prefix("PlayerStat.")
            .ok_or_else(|| "PlayerStatMap row does not reference PlayerStat".to_string())?;
        let id = *enum_ids
            .get(enum_name)
            .ok_or_else(|| "PlayerStatMap references an unknown skill".to_string())?;
        if !aliases.insert(alias.clone()) || names_by_id.insert(id, alias).is_some() {
            return Err("PlayerStatMap contains a duplicate skill alias or id".into());
        }
        rest = &after_open[close + 1..];
    }

    let enabled_body = delimited_body(source, "PlayerStatEnabled", b'[', b']')?;
    let enabled_source = strip_comments(enabled_body);
    let mut enabled = Vec::new();
    for raw in enabled_source.split(',') {
        match raw.trim() {
            "" => {}
            "true" => enabled.push(true),
            "false" => enabled.push(false),
            _ => return Err("PlayerStatEnabled contains a non-boolean entry".into()),
        }
    }
    if enabled.len() != enum_ids.len() {
        return Err("PlayerStatEnabled length does not match PlayerStat".into());
    }

    let mut skills = BTreeMap::new();
    for id in enum_ids.values() {
        if !enabled[*id] {
            continue;
        }
        let alias = names_by_id
            .get(id)
            .ok_or_else(|| "enabled PlayerStat has no PlayerStatMap name".to_string())?;
        skills.insert(
            alias.clone(),
            SkillDefinition {
                id: i32::try_from(*id).map_err(|_| "skill id is out of range".to_string())?,
                name: alias.clone(),
            },
        );
    }
    if skills.is_empty() {
        return Err("selected engine has no enabled PlayerStat entries".into());
    }
    Ok((skills, enabled))
}

/// The client table must publish every engine-enabled skill a seed can
/// target. A skill the engine enables but the client marks unused (R377
/// `PlayerStatEnabled` enables SLAYER and FARMING, `PlayerStat.ts:51`) has no
/// client stat receipt, so it is removed from the seedable set rather than
/// confirmed by guess. A client-used slot the engine disables still fails.
fn validate_client_skills(
    skills: &mut BTreeMap<String, SkillDefinition>,
    enabled: &[bool],
) -> Result<(), String> {
    let client_count = snapshot::stat_count();
    if enabled.len() > client_count {
        return Err("selected engine skill table exceeds the client skill capacity".into());
    }
    for index in 0..client_count {
        // The engine omits unused trailing client slots.
        if snapshot::stat_used(index) && !enabled.get(index).copied().unwrap_or(false) {
            return Err("selected engine and client enabled skill slots do not match".into());
        }
    }
    skills.retain(|_, skill| usize::try_from(skill.id).is_ok_and(snapshot::stat_used));
    for (name, skill) in skills.iter() {
        let index = usize::try_from(skill.id).map_err(|_| "invalid engine skill id".to_string())?;
        if snapshot::stat_name(index) != name || skill.name != *name {
            return Err("selected engine skill name does not match the client table".into());
        }
    }
    Ok(())
}

fn parse_engine_xp_table(source: &str) -> Result<Vec<i32>, String> {
    let compact: String = source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if !compact.contains("newInt32Array(99)")
        || !compact.contains("Math.floor(level+Math.pow(2.0,level/7.0)*300.0)")
        || !compact.contains("levelExperience[i]=Math.floor(acc/4)*10")
        || !compact.contains("levelExperience[level-2]")
    {
        return Err(
            "selected engine Player.ts XP table does not match the supported formula".into(),
        );
    }
    let mut table = Vec::with_capacity(99);
    let mut accumulated = 0_i64;
    for index in 0..99 {
        let level = index + 1;
        let delta = (level as f64 + 300.0 * 2.0_f64.powf(level as f64 / 7.0)).floor();
        if !delta.is_finite() || delta < 0.0 || delta > i64::MAX as f64 {
            return Err("selected engine XP table formula is out of range".into());
        }
        accumulated = accumulated
            .checked_add(delta as i64)
            .ok_or_else(|| "selected engine XP table overflowed".to_string())?;
        let xp_tenths = (accumulated / 4)
            .checked_mul(10)
            .ok_or_else(|| "selected engine XP table overflowed".to_string())?;
        table.push(
            i32::try_from(xp_tenths)
                .map_err(|_| "selected engine XP table overflowed".to_string())?,
        );
    }
    Ok(table)
}

fn engine_xp_tenths(table: &[i32], level: i32) -> Result<i32, String> {
    if level == 1 {
        return Ok(0);
    }
    let index = usize::try_from(level - 2).map_err(|_| "invalid target skill level".to_string())?;
    table
        .get(index)
        .copied()
        .ok_or_else(|| "target level is outside the selected engine XP table".into())
}

struct SeedSources<'a> {
    cheat_handler: &'a str,
    stat_encoder: &'a str,
    network_player: &'a str,
    controls_varp: &'a str,
    controls_interface: &'a str,
    cheat_other: &'a str,
    cheat_clear_inventory: &'a str,
    cheat_bank: &'a str,
    cheat_debug: &'a str,
}

fn verify_seed_sources(sources: SeedSources<'_>) -> Result<(), String> {
    let SeedSources {
        cheat_handler,
        stat_encoder,
        network_player,
        controls_varp,
        controls_interface,
        cheat_other,
        cheat_clear_inventory,
        cheat_bank,
        cheat_debug,
    } = sources;
    let handler = cheat_handler.to_ascii_lowercase();
    for command in [
        "tele",
        "give",
        "givebank",
        "setstat",
        "advancestat",
        "getvar",
        "setvar",
    ] {
        if !handler.contains(&format!("cmd === '{command}'")) {
            return Err(format!(
                "selected engine cheat handler is missing {command}"
            ));
        }
    }
    if !cheat_handler.contains("PlayerStatMap.get(args[0].toUpperCase())") {
        return Err("selected engine has no uppercase stat command lookup".into());
    }
    if !stat_encoder.contains("xp")
        || !stat_encoder.contains("10")
        || !network_player.contains("stat")
    {
        return Err("selected engine stat receipt sources are incomplete".into());
    }
    if !controls_varp.contains("option_run") || controls_interface.matches("option_run").count() < 2
    {
        return Err("selected content does not bind both run controls to option_run".into());
    }
    let other = compact_source(cheat_other);
    if !other.contains("healenergy(10000)") || !other.contains("p_run(^player_run_on)") {
        return Err("selected energy cheat does not restore energy and enable run".into());
    }
    if !cheat_clear_inventory.contains(INVENTORY_WIPED) {
        return Err("selected clear-inventory script is missing its direct reply".into());
    }
    let bank = cheat_bank.to_ascii_lowercase();
    if !bank.contains("[debugproc,clearbank]") {
        return Err("selected bank script is missing its bank clear command".into());
    }
    if !cheat_debug.contains(ACCESS_REFUSAL) {
        return Err("selected engine does not expose the protected-action refusal".into());
    }
    Ok(())
}

fn compact_source(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn parse_named_constant(source: &str, name: &str) -> Result<i32, String> {
    let mut found = None;
    for raw_line in source.lines() {
        let line = raw_line.split("//").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        if found.is_some() {
            return Err("selected content constant is duplicated".into());
        }
        found = Some(
            value
                .trim()
                .parse::<i32>()
                .map_err(|_| "selected content constant is not an integer".to_string())?,
        );
    }
    found.ok_or_else(|| "selected content constant is missing".into())
}

fn delimited_body<'a>(
    source: &'a str,
    marker: &str,
    open: u8,
    close: u8,
) -> Result<&'a str, String> {
    let marker_start = source
        .find(marker)
        .ok_or_else(|| format!("selected engine source is missing {marker}"))?;
    let bytes = source.as_bytes();
    let mut open_at = marker_start;
    while open_at < bytes.len() && bytes[open_at] != open {
        open_at += 1;
    }
    if open_at == bytes.len() {
        return Err(format!("selected engine source has no block for {marker}"));
    }
    let body_start = open_at + 1;
    let mut depth = 1_usize;
    let mut index = body_start;
    let mut quote = 0_u8;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
            index += 1;
            continue;
        }
        if block_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if quote != 0 {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                quote = 0;
            }
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            line_comment = true;
            index += 2;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            block_comment = true;
            index += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = byte;
            index += 1;
            continue;
        }
        if byte == open {
            depth += 1;
        } else if byte == close {
            depth -= 1;
            if depth == 0 {
                return std::str::from_utf8(&bytes[body_start..index])
                    .map_err(|_| "selected engine source is not valid UTF-8".into());
            }
        }
        index += 1;
    }
    Err(format!(
        "selected engine source has an unterminated block for {marker}"
    ))
}

fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = String::with_capacity(source.len());
    let mut index = 0;
    let mut line_comment = false;
    let mut block_comment = false;
    while index < bytes.len() {
        if line_comment {
            if bytes[index] == b'\n' {
                line_comment = false;
                result.push('\n');
            }
            index += 1;
            continue;
        }
        if block_comment {
            if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                block_comment = false;
                index += 2;
            } else {
                if bytes[index] == b'\n' {
                    result.push('\n');
                }
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            line_comment = true;
            index += 2;
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            block_comment = true;
            index += 2;
            continue;
        }
        result.push(bytes[index] as char);
        index += 1;
    }
    result
}

fn parse_quoted(source: &str) -> Result<&str, String> {
    let bytes = source.as_bytes();
    if bytes.len() < 2 || !matches!(bytes[0], b'\'' | b'"') || bytes[bytes.len() - 1] != bytes[0] {
        return Err("PlayerStatMap contains an invalid string alias".into());
    }
    if bytes[1..bytes.len() - 1].contains(&b'\\') {
        return Err("PlayerStatMap aliases cannot contain escapes".into());
    }
    std::str::from_utf8(&bytes[1..bytes.len() - 1])
        .map_err(|_| "PlayerStatMap alias is not valid UTF-8".into())
}
