use super::*;
pub fn duel_operation_gates() -> &'static [OperationGate] {
    &[
        OperationGate {
            source: "DuelInterface.ts Duel.challenge/fight",
            call: "Input.interactPlayer(index, 1|2)",
            host_shape: "{ op: 'player', name, action } from posted player ops[op-1]; false if missing",
            kind: GateKind::Mapped,
            owner: "this fixture; queued Challenge without combat is not a duel",
        },
        OperationGate {
            source: "DuelInterface.ts Duel.accept/partner/waitingForOther",
            call: "actions.ifButton(6674|6520) / reader.ifText(6671|6684|6571) / reader.modals().main",
            host_shape: "generated duel controls + posted widget text; absent id is null, never stale IfType",
            kind: GateKind::CatalogLiteralMatchesGenerated,
            owner: "catalog still hardcodes IDs; both caches match 6575/6412/6733/6674/6520/6671/6684/6571",
        },
        OperationGate {
            source: "DuelArena.ts FightOpponent / observeFightState",
            call: "Game.inCombat / reader.selfChat 3/2/1/FIGHT / fightArenaAt pens",
            host_shape: "local actor in_combat + overhead; seeded modal is not combat",
            kind: GateKind::Mapped,
            owner: "this fixture",
        },
        OperationGate {
            source: "DuelArena.ts SetTrainingStyle",
            call: "Game.combatStyleResolution / Game.setCombatStyle / Game.combatMode",
            host_shape: "existing combat-style mapping; requires exact Attack+Strength (Defence only if target>1)",
            kind: GateKind::Mapped,
            owner: "this fixture seeds a 1-handed melee weapon",
        },
        OperationGate {
            source: "DuelArena.ts CenterLobby / seekOpponent",
            call: "DirectNavigator.walkTo",
            host_shape: "existing direct walk",
            kind: GateKind::Mapped,
            owner: "this fixture",
        },
        OperationGate {
            source: "DuelArena.ts",
            call: "Game.teleport / special / Shop",
            host_shape: "n/a",
            kind: GateKind::UnusedByCase,
            owner: "teleport t_1bf9a22e, special t_68de6f48, shop t_1591d140",
        },
    ]
}

pub fn duel_controls_match(controls: &DuelControls) -> bool {
    controls.select_modal == EXPECTED_DUEL_CONTROLS.select_modal
        && controls.confirm_modal == EXPECTED_DUEL_CONTROLS.confirm_modal
        && controls.win_modal == EXPECTED_DUEL_CONTROLS.win_modal
        && controls.select_accept == EXPECTED_DUEL_CONTROLS.select_accept
        && controls.confirm_accept == EXPECTED_DUEL_CONTROLS.confirm_accept
        && controls.select_partner == EXPECTED_DUEL_CONTROLS.select_partner
        && controls.select_status == EXPECTED_DUEL_CONTROLS.select_status
        && controls.confirm_status == EXPECTED_DUEL_CONTROLS.confirm_status
        && controls.available()
}

pub fn verify_generated_duel_controls(data: &SelectedGameData) -> Result<(), String> {
    let Some(controls) = data.duel_controls() else {
        return Err(format!(
            "selected game data revision {} has no available duel controls",
            data.revision()
        ));
    };
    if !duel_controls_match(controls) {
        return Err(format!(
            "generated duel controls mismatch on revision {}: {controls:?}",
            data.revision()
        ));
    }
    Ok(())
}
pub fn duel_prepared_current(
    expected_player: &str,
    observation: &DuelObservation,
) -> Result<(), String> {
    if !observation.ingame || observation.scene_state != 2 {
        return Err(format!(
            "Start baseline is not attached ingame scene2: {observation:?}"
        ));
    }
    if !observation.inventory_tab_available {
        return Err("Start baseline inventory tab is not bound after relog".into());
    }
    let player = observation
        .player
        .as_deref()
        .ok_or_else(|| "Start baseline has no local player".to_string())?;
    if !account_identity_eq(player, expected_player) {
        return Err(format!(
            "Start baseline player {player:?} is not fresh account {expected_player:?}"
        ));
    }
    if !observation.in_challenge_area {
        return Err(format!(
            "Start baseline is not in the Duel Arena challenge area: {:?}",
            observation.tile
        ));
    }
    if observation.duel_active() || observation.duel_win_open {
        return Err("seeded modal is not a duel: baseline already has a duel interface".into());
    }
    if !observation.weapon_equipped {
        return Err("Start baseline has no 1-handed melee weapon equipped".into());
    }
    Ok(())
}
pub fn in_duel_challenge_area(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == 0
            && tile.0 >= 3328
            && tile.0 <= 3393
            && tile.1 >= 3203
            && tile.1 <= 3325
            && fight_pen(Some(tile)).is_none()
    })
}

pub fn fight_pen(tile: Option<(i32, i32, i32)>) -> Option<(i32, i32, i32, i32)> {
    const PENS: [(i32, i32, i32, i32); 6] = [
        (3333, 3357, 3244, 3258),
        (3364, 3388, 3225, 3239),
        (3333, 3357, 3206, 3220),
        (3364, 3388, 3244, 3258),
        (3333, 3357, 3225, 3239),
        (3364, 3388, 3206, 3220),
    ];
    let tile = tile?;
    if tile.2 != 0 {
        return None;
    }
    PENS.iter().copied().find(|(min_x, max_x, min_z, max_z)| {
        tile.0 >= *min_x && tile.0 <= *max_x && tile.1 >= *min_z && tile.1 <= *max_z
    })
}

fn widget_text(widgets: &[WidgetView], component_id: i32) -> Option<String> {
    widgets.iter().find_map(|widget| {
        (widget.component_id == component_id)
            .then(|| widget.text.clone())
            .flatten()
            .filter(|text| !text.is_empty())
    })
}

fn parse_duel_partner_header(header: Option<&str>) -> Option<String> {
    let header = header?.trim();
    let stripped = header
        .strip_prefix("Dueling with:")
        .or_else(|| header.strip_prefix("dueling with:"))
        .unwrap_or(header)
        .trim()
        .trim_start_matches(':')
        .trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuelObservation {
    pub ingame: bool,
    pub scene_state: i32,
    pub inventory_tab_available: bool,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub tick: u32,
    pub attack_xp: i32,
    pub strength_xp: i32,
    pub defence_xp: i32,
    pub hitpoints_xp: i32,
    pub in_combat: bool,
    pub in_challenge_area: bool,
    pub in_fight_pen: bool,
    pub main_modal: i32,
    pub duel_offer_open: bool,
    pub duel_confirm_open: bool,
    pub duel_win_open: bool,
    pub duel_partner: Option<String>,
    pub waiting_for_other: bool,
    pub weapon_equipped: bool,
    pub peer_visible: bool,
}

impl DuelObservation {
    pub fn from_snapshot(snapshot: &GameSnapshot, peer: &str, weapon_id: i32) -> Self {
        let tile = snapshot.tile();
        let main = snapshot.modals().main;
        let partner = parse_duel_partner_header(
            widget_text(snapshot.widgets(), DUEL_SELECT_PARTNER).as_deref(),
        );
        let status = if main == DUEL_SELECT_MODAL {
            widget_text(snapshot.widgets(), DUEL_SELECT_STATUS)
        } else if main == DUEL_CONFIRM_MODAL {
            widget_text(snapshot.widgets(), DUEL_CONFIRM_STATUS)
        } else {
            None
        };
        let waiting = status.as_deref().is_some_and(|text| {
            text.trim()
                .to_ascii_lowercase()
                .starts_with("waiting for other")
        });
        let peer_visible = snapshot.players().iter().any(|player| {
            player
                .actor
                .name
                .as_deref()
                .is_some_and(|name| account_identity_eq(name, peer))
        });
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            inventory_tab_available: snapshot
                .side_tabs()
                .iter()
                .any(|tab| tab.index == 3 && tab.available),
            player: snapshot
                .local_player()
                .and_then(|local| local.player.actor.name.clone()),
            tile,
            tick: snapshot.tick(),
            attack_xp: stat(snapshot, "attack").map(|row| row.xp).unwrap_or(0),
            strength_xp: stat(snapshot, "strength").map(|row| row.xp).unwrap_or(0),
            defence_xp: stat(snapshot, "defence").map(|row| row.xp).unwrap_or(0),
            hitpoints_xp: stat(snapshot, "hitpoints").map(|row| row.xp).unwrap_or(0),
            in_combat: snapshot
                .local_player()
                .is_some_and(|local| local.player.actor.in_combat),
            in_challenge_area: in_duel_challenge_area(tile),
            in_fight_pen: fight_pen(tile).is_some(),
            main_modal: main,
            duel_offer_open: main == DUEL_SELECT_MODAL,
            duel_confirm_open: main == DUEL_CONFIRM_MODAL,
            duel_win_open: main == DUEL_WIN_MODAL,
            duel_partner: partner,
            waiting_for_other: waiting,
            weapon_equipped: weapon_id > 0 && count_id(snapshot.equipment(), weapon_id) > 0,
            peer_visible,
        }
    }

    pub fn melee_xp(&self) -> i32 {
        self.attack_xp + self.strength_xp + self.defence_xp
    }

    pub fn duel_active(&self) -> bool {
        self.duel_offer_open || self.duel_confirm_open
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DuelClaim {
    FirstCombat,
    ResetAndFurther,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuelSlotRecord {
    pub account: String,
    pub expected_player: String,
    pub partner: String,
    pub settings: Map<String, Value>,
    pub baseline: DuelObservation,
    pub latest: Option<DuelObservation>,
    pub post_start: u32,
    pub mixed_identity: bool,
    pub saw_offer: bool,
    pub saw_confirm: bool,
    pub saw_wrong_partner: bool,
    pub saw_pen: bool,
    pub saw_combat: bool,
    pub saw_win_or_lobby_return: bool,
    pub melee_xp_from_script: i32,
    pub further_combat: bool,
}

impl DuelSlotRecord {
    pub fn new(
        account: String,
        expected_player: String,
        partner: String,
        settings: Map<String, Value>,
        baseline: DuelObservation,
    ) -> Self {
        Self {
            account,
            expected_player,
            partner,
            settings,
            baseline,
            latest: None,
            post_start: 0,
            mixed_identity: false,
            saw_offer: false,
            saw_confirm: false,
            saw_wrong_partner: false,
            saw_pen: false,
            saw_combat: false,
            saw_win_or_lobby_return: false,
            melee_xp_from_script: 0,
            further_combat: false,
        }
    }

    pub fn observe(&mut self, observation: DuelObservation) {
        self.post_start = self.post_start.saturating_add(1);
        if observation
            .player
            .as_deref()
            .is_some_and(|player| !account_identity_eq(player, &self.expected_player))
        {
            self.mixed_identity = true;
        }
        if observation.duel_offer_open {
            self.saw_offer = true;
        }
        if observation.duel_confirm_open {
            self.saw_confirm = true;
        }
        if let Some(partner) = observation.duel_partner.as_deref() {
            if !account_identity_eq(partner, &self.partner) {
                self.saw_wrong_partner = true;
            }
        }
        if observation.in_fight_pen {
            self.saw_pen = true;
        }
        if observation.in_combat && observation.in_fight_pen {
            if self.saw_combat && self.saw_win_or_lobby_return {
                self.further_combat = true;
            }
            self.saw_combat = true;
        }
        if self.saw_combat
            && (observation.duel_win_open || observation.in_challenge_area)
            && !observation.in_fight_pen
        {
            self.saw_win_or_lobby_return = true;
        }
        self.melee_xp_from_script = (observation.melee_xp() - self.baseline.melee_xp()).max(0);
        self.latest = Some(observation);
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuelPairWitness {
    pub a: DuelSlotRecord,
    pub b: DuelSlotRecord,
}

impl DuelPairWitness {
    pub fn qualify_supported(&self) -> Result<DuelClaim, String> {
        self.qualify_common()?;
        if self.a.baseline.duel_active() || self.b.baseline.duel_active() {
            return Err("seeded modal is not a duel: baseline already had a duel interface".into());
        }
        if !self.a.saw_offer || !self.b.saw_offer {
            return Err(
                "one-sided confirmation: both scripts never observed the select/offer duel modal"
                    .into(),
            );
        }
        if !self.a.saw_confirm || !self.b.saw_confirm {
            return Err(
                "one-sided confirmation: both scripts never observed the confirm duel modal".into(),
            );
        }
        if !self.a.saw_pen || !self.b.saw_pen {
            return Err("no real duel combat: neither actor entered a fight pen".into());
        }
        if !self.a.saw_combat || !self.b.saw_combat {
            return Err("no real duel combat: in-combat was never observed inside a pen".into());
        }
        if self.a.melee_xp_from_script <= 0 && self.b.melee_xp_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: no Attack/Strength/Defence XP caused by hits".into(),
            );
        }
        Ok(DuelClaim::FirstCombat)
    }

    pub fn qualify_full_cycle(&self) -> Result<DuelClaim, String> {
        self.qualify_supported()?;
        if !self.a.saw_win_or_lobby_return && !self.b.saw_win_or_lobby_return {
            return Err(
                "no further work for full-cycle claims: duel did not end/reset to the lobby".into(),
            );
        }
        if !self.a.further_combat && !self.b.further_combat {
            return Err("no further work for full-cycle claims: no further script-caused challenge/combat after reset".into());
        }
        Ok(DuelClaim::ResetAndFurther)
    }

    fn qualify_common(&self) -> Result<(), String> {
        if account_identity_eq(&self.a.account, &self.b.account) {
            return Err("mixed identities: both duel slots share one account".into());
        }
        if self.a.mixed_identity || self.b.mixed_identity {
            return Err("mixed identities: published player did not match the minted slot".into());
        }
        if self.a.post_start == 0 || self.b.post_start == 0 {
            return Err("queued-only success: no post-Start observations".into());
        }
        if self.a.saw_wrong_partner || self.b.saw_wrong_partner {
            return Err("wrong partner: duel header was not the minted counterpart".into());
        }
        if !account_identity_eq(&self.a.partner, &self.b.expected_player)
            || !account_identity_eq(&self.b.partner, &self.a.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
        }
        Ok(())
    }
}
pub fn duel_settings(schema: &[script::SettingDef]) -> Map<String, Value> {
    script::merge_bag(schema, &Map::new(), None)
}
