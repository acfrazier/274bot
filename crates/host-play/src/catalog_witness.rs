use super::*;
#[derive(Debug, Clone, Serialize)]
pub struct CoreWitness {
    pub case: CoreCase,
    pub baseline: Observation,
    pub start_preparation: Option<HerbCleanerStartPreparationReceipt>,
    pub latest: Observation,
    pub max_items: BTreeMap<String, i32>,
    pub max_xp: BTreeMap<String, i32>,
    pub saw_bury_chat: bool,
    pub post_start_observations: u64,
    pub bone_bank_cycle: BoneBankCycle,
    pub bank_fletcher_cycle: BankFletcherCycle,
    pub bank_fletcher_option_cycle: BankFletcherOptionCycle,
    pub bank_fletcher_string_cycle: BankFletcherStringCycle,
    pub bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle,
    pub alcher_defaults_cycle: AlcherGeneratedCustomCycle,
    pub alcher_generated_custom_cycle: AlcherGeneratedCustomCycle,
    pub alcher_spell_cycle: AlcherGeneratedCustomCycle,
    pub alcher_swarm_cycle: AlcherSwarmDrainCycle,
    pub dart_fletcher_cycle: DartFletcherCycle,
    pub herb_cleaner_cycle: HerbCleanerCycle,
    pub herb_cleaner_empty_cycle: HerbCleanerEmptyCycle,
    pub gem_cutter_cycle: GemCutterCycle,
    pub door_opener_cycle: DoorOpenerCycle,
    pub gnome_course_cycle: GnomeCourseCycle,
    pub wildy_agility_cycle: WildyAgilityCycle,
    pub brimhaven_agility_cycle: BrimhavenAgilityCycle,
    pub flax_picker_cycle: FlaxPickerCycle,
    pub superheater_cycle: SuperheaterCycle,
    pub chicken_killer_bank_cycle: ChickenKillerBankCycle,
    pub vial_filler_cycle: VialFillerCycle,
    pub potion_maker_cycle: PotionMakerCycle,
    pub tanner_bot_cycle: TannerBotCycle,
    pub rune_crafter_cycle: RuneCrafterCycle,
    pub ardy_cakes_cycle: ArdyCakesCycle,
    pub ardy_cakes_fight_cycle: ArdyCakesFightCycle,
    pub ardy_thiever_cycle: ArdyThieverCycle,
    pub ardy_thiever_fight_cycle: ArdyThieverFightCycle,
    pub gnome_chop_cycle: GnomeChopCycle,
    pub gnome_fletch_cycle: GnomeFletchCycle,
    pub coal_trucks_cycle: CoalTrucksCycle,
    pub station_production_cycle: StationProductionCycle,
    pub flax_aio_cycle: FlaxAioCycle,
    pub flax_aio_pick_cycle: FlaxAioPickCycle,
    pub herblore_eggs_cycle: HerbloreEggsCycle,
    pub herblore_newt_cycle: HerbloreNewtCycle,
    pub combat_core_cycle: CombatCoreCycle,
    pub combat_dart_branch_cycle: CombatDartBranchCycle,
    pub combat_bank_cycle: CombatBankCycle,
    pub combat_approach_cycle: CombatApproachCycle,
    pub aio_teleport_cycle: AioTeleportCycle,
    pub shop_buyout_cycle: ShopBuyoutCycle,
    pub smithing_bot_cycle: SmithingBotCycle,
    pub leather_crafter_cycle: LeatherCrafterCycle,
    pub firemaker_cycle: FiremakerCycle,
    pub climbing_boots_cycle: ClimbingBootsCycle,
    pub ranging_guild_round_cycle: RangingGuildRoundCycle,
    pub ranging_guild_redeem_cycle: RangingGuildRedeemCycle,
    pub ranging_guild_bank_cycle: RangingGuildBankCycle,
    pub ranging_guild_full_cycle: RangingGuildFullCycle,
    pub brimhaven_moss_inspect_cycle: BrimhavenMossInspectCycle,
    pub route_inspect_brimhaven_v2_cycle: RouteInspectBrimhavenV2Cycle,
    pub prayer_delivery_cycle: PrayerDeliveryCycle,
    pub line_of_sight_cycle: LineOfSightDeliveryCycle,
    pub actor_observation_cycle: ActorObservationDeliveryCycle,
    pub fight_field_cycle: FightFieldDeliveryCycle,
    pub hunt_cycle: HuntDeliveryCycle,
    pub ordered_first_exhausted: bool,
}
impl CoreWitness {
    pub fn new(case: CoreCase, baseline: Observation) -> Self {
        Self::new_with_start_preparation(case, baseline, None)
    }

    pub fn new_with_start_preparation(
        case: CoreCase,
        baseline: Observation,
        start_preparation: Option<HerbCleanerStartPreparationReceipt>,
    ) -> Self {
        Self {
            max_items: baseline.items.clone(),
            max_xp: baseline.xp.clone(),
            latest: baseline.clone(),
            baseline,
            start_preparation,
            case,
            saw_bury_chat: false,
            post_start_observations: 0,
            bone_bank_cycle: BoneBankCycle::default(),
            bank_fletcher_cycle: BankFletcherCycle::default(),
            bank_fletcher_option_cycle: BankFletcherOptionCycle::default(),
            bank_fletcher_string_cycle: BankFletcherStringCycle::default(),
            bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle::default(),
            alcher_defaults_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_generated_custom_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_spell_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_swarm_cycle: AlcherSwarmDrainCycle::default(),
            dart_fletcher_cycle: DartFletcherCycle::default(),
            herb_cleaner_cycle: HerbCleanerCycle::default(),
            herb_cleaner_empty_cycle: HerbCleanerEmptyCycle::default(),
            gem_cutter_cycle: GemCutterCycle::default(),
            door_opener_cycle: DoorOpenerCycle::default(),
            gnome_course_cycle: GnomeCourseCycle::default(),
            wildy_agility_cycle: WildyAgilityCycle::default(),
            brimhaven_agility_cycle: BrimhavenAgilityCycle::default(),
            flax_picker_cycle: FlaxPickerCycle::default(),
            superheater_cycle: SuperheaterCycle::default(),
            chicken_killer_bank_cycle: ChickenKillerBankCycle::default(),
            vial_filler_cycle: VialFillerCycle::default(),
            potion_maker_cycle: PotionMakerCycle::default(),
            tanner_bot_cycle: TannerBotCycle::default(),
            rune_crafter_cycle: RuneCrafterCycle::default(),
            ardy_cakes_cycle: ArdyCakesCycle::default(),
            ardy_cakes_fight_cycle: ArdyCakesFightCycle::default(),
            ardy_thiever_cycle: ArdyThieverCycle::default(),
            ardy_thiever_fight_cycle: ArdyThieverFightCycle::default(),
            gnome_chop_cycle: GnomeChopCycle::default(),
            gnome_fletch_cycle: GnomeFletchCycle::default(),
            coal_trucks_cycle: CoalTrucksCycle::default(),
            station_production_cycle: StationProductionCycle::default(),
            flax_aio_cycle: FlaxAioCycle::default(),
            flax_aio_pick_cycle: FlaxAioPickCycle::default(),
            herblore_eggs_cycle: HerbloreEggsCycle::default(),
            herblore_newt_cycle: HerbloreNewtCycle::default(),
            combat_core_cycle: CombatCoreCycle::default(),
            combat_dart_branch_cycle: CombatDartBranchCycle::default(),
            combat_bank_cycle: CombatBankCycle::default(),
            combat_approach_cycle: CombatApproachCycle::default(),
            aio_teleport_cycle: AioTeleportCycle::default(),
            shop_buyout_cycle: ShopBuyoutCycle::default(),
            smithing_bot_cycle: SmithingBotCycle::default(),
            leather_crafter_cycle: LeatherCrafterCycle::default(),
            firemaker_cycle: FiremakerCycle::default(),
            climbing_boots_cycle: ClimbingBootsCycle::default(),
            ranging_guild_round_cycle: RangingGuildRoundCycle::default(),
            ranging_guild_redeem_cycle: RangingGuildRedeemCycle::default(),
            ranging_guild_bank_cycle: RangingGuildBankCycle::default(),
            ranging_guild_full_cycle: RangingGuildFullCycle::default(),
            brimhaven_moss_inspect_cycle: BrimhavenMossInspectCycle::default(),
            route_inspect_brimhaven_v2_cycle: RouteInspectBrimhavenV2Cycle::default(),
            prayer_delivery_cycle: PrayerDeliveryCycle::default(),
            line_of_sight_cycle: LineOfSightDeliveryCycle::default(),
            actor_observation_cycle: ActorObservationDeliveryCycle::default(),
            fight_field_cycle: FightFieldDeliveryCycle::default(),
            hunt_cycle: HuntDeliveryCycle::default(),
            ordered_first_exhausted: false,
        }
    }

    pub fn observe(&mut self, observation: &Observation) {
        if !observation.ingame
            || observation.scene_state != 2
            || observation.player != self.baseline.player
        {
            return;
        }
        if matches!(self.case, CoreCase::BoneBurier) {
            self.bone_bank_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcher) {
            self.bank_fletcher_cycle
                .observe(&self.baseline, observation);
        }
        if let Some(spec) = bank_fletcher_option_spec(self.case) {
            self.bank_fletcher_option_cycle
                .observe(spec, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcherString) {
            self.bank_fletcher_string_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcherCutString) {
            self.bank_fletcher_cut_string_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(
            self.case,
            CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName
        ) {
            self.alcher_generated_custom_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::AlcherDefaults) {
            self.alcher_defaults_cycle.observe_target(
                &self.baseline,
                observation,
                YEW_LONGBOW_ID,
                CERT_YEW_LONGBOW_ID,
                YEW_LONGBOW_ALCH_COINS,
            );
        }
        if matches!(
            self.case,
            CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff
        ) {
            let expectation = match self.case {
                CoreCase::AlcherLow => ALCHER_LOW_EXPECTATION,
                _ => ALCHER_FIRE_BATTLESTAFF_EXPECTATION,
            };
            self.alcher_spell_cycle.observe_spelled(
                expectation,
                &self.baseline,
                observation,
                RUNE_CHAINBODY_ID,
                CERT_RUNE_CHAINBODY_ID,
            );
        }
        if matches!(self.case, CoreCase::AlcherSwarmDrain) {
            self.alcher_swarm_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::DartFletcher) {
            self.dart_fletcher_cycle.observe(
                BRONZE_DART_TIP_ID,
                FEATHER_ID,
                BRONZE_DART_ID,
                IRON_DART_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DartFletcherIron) {
            self.dart_fletcher_cycle.observe(
                IRON_DART_TIP_ID,
                FEATHER_ID,
                IRON_DART_ID,
                BRONZE_DART_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(
            self.case,
            CoreCase::HerbCleaner | CoreCase::HerbCleanerNamed
        ) {
            self.herb_cleaner_cycle.observe(
                matches!(self.case, CoreCase::HerbCleanerNamed),
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::HerbCleanerEmptyBank) {
            self.herb_cleaner_empty_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GemCutter | CoreCase::GemCutterNamed) {
            self.gem_cutter_cycle.observe(
                matches!(self.case, CoreCase::GemCutterNamed),
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DoorOpener) {
            self.door_opener_cycle.observe(
                false,
                LUMBRIDGE_DOOR,
                WOODEN_DOOR_CLOSED_ID,
                WOODEN_DOOR_OPEN_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DoorOpenerGate) {
            self.door_opener_cycle.observe(
                true,
                LUMBRIDGE_GATE,
                WOODEN_GATE_CLOSED_ID,
                WOODEN_GATE_OPEN_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(
            self.case,
            CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius
        ) {
            self.gnome_course_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::WildyAgility) {
            self.wildy_agility_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BrimhavenAgility) {
            self.brimhaven_agility_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxPicker) {
            self.flax_picker_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxAio) {
            self.flax_aio_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxAioPick) {
            self.flax_aio_pick_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::HerbloreSecondaries) {
            self.herblore_eggs_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::HerbloreSecondariesNewt) {
            self.herblore_newt_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::MossGiantDart) {
            if let Some(spec) = combat_spec(self.case) {
                self.combat_dart_branch_cycle
                    .observe(spec, &self.baseline, observation);
            }
        } else if let Some(spec) = combat_spec(self.case) {
            self.combat_core_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let (Some(spec), Some(bank)) = (combat_spec(self.case), combat_bank_spec(self.case)) {
            self.combat_bank_cycle
                .observe(spec, bank, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FireGiantApproach) {
            if let Some(spec) = combat_spec(self.case) {
                self.combat_approach_cycle
                    .observe(spec, &self.baseline, observation);
            }
        }
        if let Some(spec) = aio_teleport_spec(self.case) {
            self.aio_teleport_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = shop_buyout_spec(self.case) {
            self.shop_buyout_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = smithing_bot_spec(self.case) {
            self.smithing_bot_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = leather_crafter_spec(self.case) {
            self.leather_crafter_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = firemaker_spec(self.case) {
            self.firemaker_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = climbing_boots_spec(self.case) {
            self.climbing_boots_cycle
                .observe(spec, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildRound) {
            self.ranging_guild_round_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildRedeem) {
            self.ranging_guild_redeem_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildBank) {
            self.ranging_guild_bank_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildFull) {
            self.ranging_guild_full_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BrimhavenMossInspectV1) {
            self.brimhaven_moss_inspect_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RouteInspectBrimhavenV2) {
            self.route_inspect_brimhaven_v2_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::PrayerV2 | CoreCase::PrayerV1) {
            self.prayer_delivery_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::LineOfSightV2) {
            self.line_of_sight_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::ActorObservationV2) {
            self.actor_observation_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::FightFieldV2) {
            self.fight_field_cycle.observe(observation);
        }
        if let Some(cell) = self.case.hunt_cell() {
            self.hunt_cycle.observe(cell, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::Superheater) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: BRONZE_BAR_ID,
                    primary: COPPER_ORE_ID,
                    secondary: TIN_ORE_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterSteel) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: STEEL_BAR_ID,
                    primary: IRON_ORE_ID,
                    secondary: COAL_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: true,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterFireBattlestaff) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: BRONZE_BAR_ID,
                    primary: COPPER_ORE_ID,
                    secondary: TIN_ORE_ID,
                    staff: FIRE_BATTLESTAFF_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterSilverLowNatures) {
            // Single-ore: primary == secondary so the pair-ratio check is a no-op.
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: SILVER_BAR_ID,
                    primary: SILVER_ORE_ID,
                    secondary: SILVER_ORE_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::ChickenKillerBank) {
            self.chicken_killer_bank_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::VialFiller | CoreCase::VialFillerEast) {
            self.vial_filler_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::PotionMaker) {
            self.potion_maker_cycle.observe(
                PotionMakerSpec {
                    herb: GUAM_LEAF_ID,
                    unf: GUAM_UNF_ID,
                    secondary: EYE_OF_NEWT_ID,
                    finished: ATTACK_POTION_3_ID,
                    wrong_unf: RANARR_UNF_ID,
                    wrong_finished: PRAYER_POTION_3_ID,
                    named: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::PotionMakerNamed) {
            self.potion_maker_cycle.observe(
                PotionMakerSpec {
                    herb: RANARR_WEED_ID,
                    unf: RANARR_UNF_ID,
                    secondary: SNAPE_GRASS_ID,
                    finished: PRAYER_POTION_3_ID,
                    wrong_unf: GUAM_UNF_ID,
                    wrong_finished: ATTACK_POTION_3_ID,
                    named: true,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::TannerBot) {
            self.tanner_bot_cycle.observe(
                TannerBotSpec {
                    product: SOFT_LEATHER_ID,
                    wrong_product: HARD_LEATHER_ID,
                    tan_all: SOFT_TAN_ALL_COM,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::TannerBotHard) {
            self.tanner_bot_cycle.observe(
                TannerBotSpec {
                    product: HARD_LEATHER_ID,
                    wrong_product: SOFT_LEATHER_ID,
                    tan_all: HARD_TAN_ALL_COM,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::RuneCrafter) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: AIR_RUNE_ID,
                    wrong_rune: EARTH_RUNE_ID,
                    ruins: RUNECRAFTER_AIR_RUINS,
                    bank: FALADOR_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::RuneCrafterEarth) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: EARTH_RUNE_ID,
                    wrong_rune: AIR_RUNE_ID,
                    ruins: RUNECRAFTER_EARTH_RUINS,
                    bank: VARROCK_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::MuleCrafter) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: AIR_RUNE_ID,
                    wrong_rune: EARTH_RUNE_ID,
                    ruins: MULECRAFTER_AIR_RUINS,
                    bank: FALADOR_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::ArdyCakes) {
            self.ardy_cakes_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::ArdyCakesFight) {
            self.ardy_cakes_fight_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(
            self.case,
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight
        ) {
            self.ardy_thiever_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::ArdyThieverFight) {
            self.ardy_thiever_fight_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GnomeChop) {
            self.gnome_chop_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GnomeFletchShort) {
            self.gnome_fletch_cycle.observe(
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                UNSTRUNG_MAGIC_LONGBOW_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::GnomeFletchLong) {
            self.gnome_fletch_cycle.observe(
                UNSTRUNG_MAGIC_LONGBOW_ID,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::CoalTrucks) {
            self.coal_trucks_cycle.observe(&self.baseline, observation);
        }
        if let Some(spec) = station_production_spec(self.case, observation) {
            self.station_production_cycle
                .observe(spec, &self.baseline, observation);
        }
        let baseline_sequence = self
            .baseline
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(i32::MIN);
        self.saw_bury_chat |= observation.chat.iter().any(|(sequence, text)| {
            *sequence > baseline_sequence && text.to_ascii_lowercase().contains("bury the bones")
        });
        for (name, count) in &observation.items {
            let peak = self.max_items.entry(name.clone()).or_insert(*count);
            *peak = (*peak).max(*count);
        }
        for (name, xp) in &observation.xp {
            let peak = self.max_xp.entry(name.clone()).or_insert(*xp);
            *peak = (*peak).max(*xp);
        }
        self.ordered_first_exhausted |= self.max_items.get("Rune platebody").copied().unwrap_or(0)
            > 0
            && observation.item("Rune platebody") == 0
            && observation.item("Rune chainbody") > 0;
        self.latest = observation.clone();
        if let Some(receipt) = observation.script_lifecycle.clone() {
            self.observe_script_lifecycle(receipt);
        }
        self.post_start_observations += 1;
    }

    /// Observe one compact, non-consuming lifecycle value at the same native
    /// publication boundary as the latest snapshot.
    pub fn observe_script_lifecycle(&mut self, receipt: script::ScriptLifecycleReceipt) {
        match self.case {
            CoreCase::HerbCleanerEmptyBank => self
                .herb_cleaner_empty_cycle
                .observe_script_lifecycle(receipt),
            CoreCase::RangingGuildFull => self
                .ranging_guild_full_cycle
                .observe_script_lifecycle(receipt),
            CoreCase::PrayerV2 | CoreCase::PrayerV1 => {
                if let Some(expected) = self.case.prayer_stop_reason() {
                    self.prayer_delivery_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::LineOfSightV2 => {
                if let Some(expected) = self.case.los_stop_reason() {
                    self.line_of_sight_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::ActorObservationV2 => {
                if let Some(expected) = self.case.actor_stop_reason() {
                    self.actor_observation_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::FightFieldV2 => {
                if let Some(expected) = self.case.fight_field_stop_reason() {
                    self.fight_field_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::HoldSpotV2
            | CoreCase::RetreatSpotV2
            | CoreCase::WalkSpotV2
            | CoreCase::EnterLairV2
            | CoreCase::LeaveLairV2
            | CoreCase::AcquireKeyV2
            | CoreCase::CellV2
            | CoreCase::BankV2 => {
                if let Some(cell) = self.case.hunt_cell() {
                    self.hunt_cycle
                        .observe_script_lifecycle(receipt, cell.stop_reason());
                }
            }
            _ => {}
        }
    }

    fn incomplete_delta_error(&self) -> String {
        if self.case == CoreCase::AlcherSwarmDrain {
            format!(
                "{} core post-Start delta incomplete {}",
                self.case.scenario_name(),
                self.alcher_swarm_cycle.phase_status()
            )
        } else {
            format!(
                "{} core post-Start delta incomplete",
                self.case.scenario_name()
            )
        }
    }

    pub(super) fn swarm_stopped_without_recovery(
        &self,
        observation: &Observation,
    ) -> Option<String> {
        if self.case != CoreCase::AlcherSwarmDrain || self.qualified() {
            return None;
        }
        let stopped = observation
            .script_lifecycle
            .as_ref()
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Stopped);
        stopped.then(|| {
            format!(
                "alcher_swarm_drain stopped without ordered recovery {}",
                self.alcher_swarm_cycle.phase_status()
            )
        })
    }

    pub fn peak_item(&self, name: &str) -> i32 {
        self.max_items.get(name).copied().unwrap_or(0)
    }

    pub fn xp_gained(&self, skill: &str) -> bool {
        self.latest.skill_xp(skill) > self.baseline.skill_xp(skill)
    }

    pub fn item_increased(&self, name: &str) -> bool {
        self.peak_item(name) > self.baseline.item(name)
    }

    pub fn item_consumed_from_baseline(&self, name: &str) -> bool {
        self.latest.item(name) < self.baseline.item(name)
    }

    pub fn acquired_then_consumed(&self, name: &str) -> bool {
        self.item_increased(name) && self.latest.item(name) < self.peak_item(name)
    }

    /// Cheap predicate used by headed UI polling. Evidence is serialized only
    /// once after this becomes true.
    pub fn qualified(&self) -> bool {
        if self.post_start_observations == 0 {
            return false;
        }
        match self.case {
            CoreCase::BoneBurier => {
                self.bone_bank_cycle.buried_after_withdrawal && self.saw_bury_chat
            }
            CoreCase::ChickenKiller => {
                self.xp_gained("strength")
                    && self.acquired_then_consumed("Bones")
                    && self.xp_gained("prayer")
                    && self.saw_bury_chat
            }
            CoreCase::ChickenKillerBank => self.chicken_killer_bank_cycle.qualified(),
            CoreCase::Thiever => self.xp_gained("thieving") && self.item_increased("Coins"),
            CoreCase::Alcher
            | CoreCase::AlcherCustom
            | CoreCase::AlcherOrdered
            | CoreCase::AlcherLargeBatch => {
                self.xp_gained("magic")
                    && self.acquired_then_consumed("Nature rune")
                    && self.item_increased("Coins")
                    && match self.case {
                        CoreCase::AlcherOrdered => {
                            self.peak_item("Rune platebody") >= 1
                                && self.peak_item("Rune chainbody") >= 1
                                && self.ordered_first_exhausted
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        CoreCase::AlcherLargeBatch => {
                            self.peak_item("Rune chainbody") >= 1000
                                && self.peak_item("Nature rune") >= 1000
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        _ => self.acquired_then_consumed("Rune chainbody"),
                    }
            }
            CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
                self.alcher_generated_custom_cycle.consumed
            }
            CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff => {
                self.alcher_spell_cycle.qualified_spell()
            }
            CoreCase::AlcherSwarmDrain => self.alcher_swarm_cycle.qualified(),
            CoreCase::AlcherDefaults => self.alcher_defaults_cycle.consumed,
            CoreCase::BankFletcher => {
                self.bank_fletcher_cycle.crafted_after_withdrawal
                    && self.xp_gained("fletching")
                    && self.item_consumed_from_baseline("Willow logs")
                    && self.item_increased("Willow shortbow")
            }
            CoreCase::BankFletcherShafts | CoreCase::BankFletcherHeadless => {
                self.bank_fletcher_option_cycle.qualified()
            }
            CoreCase::BankFletcherString => self.bank_fletcher_string_cycle.strung_after_withdrawal,
            CoreCase::BankFletcherCutString => {
                self.bank_fletcher_cut_string_cycle.string_pair_created
            }
            CoreCase::DartFletcher | CoreCase::DartFletcherIron => {
                self.dart_fletcher_cycle.qualified()
            }
            CoreCase::HerbCleaner | CoreCase::HerbCleanerNamed => {
                self.herb_cleaner_cycle.qualified()
            }
            CoreCase::HerbCleanerEmptyBank => self.herb_cleaner_empty_cycle.qualified(),
            CoreCase::GemCutter | CoreCase::GemCutterNamed => self.gem_cutter_cycle.qualified(),
            CoreCase::DoorOpener | CoreCase::DoorOpenerGate => self.door_opener_cycle.qualified(),
            CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => {
                self.gnome_course_cycle.qualified()
            }
            CoreCase::WildyAgility => self.wildy_agility_cycle.qualified(),
            CoreCase::BrimhavenAgility => self.brimhaven_agility_cycle.qualified(),
            CoreCase::FlaxPicker => self.flax_picker_cycle.qualified(),
            CoreCase::Superheater
            | CoreCase::SuperheaterSteel
            | CoreCase::SuperheaterFireBattlestaff
            | CoreCase::SuperheaterSilverLowNatures => self.superheater_cycle.qualified(),
            CoreCase::VialFiller | CoreCase::VialFillerEast => self.vial_filler_cycle.qualified(),
            CoreCase::PotionMaker | CoreCase::PotionMakerNamed => {
                self.potion_maker_cycle.qualified()
            }
            CoreCase::TannerBot | CoreCase::TannerBotHard => self.tanner_bot_cycle.qualified(),
            CoreCase::RuneCrafter | CoreCase::RuneCrafterEarth | CoreCase::MuleCrafter => {
                self.rune_crafter_cycle.qualified()
            }
            CoreCase::ArdyCakes => self.ardy_cakes_cycle.qualified(),
            CoreCase::ArdyCakesFight => self.ardy_cakes_fight_cycle.qualified(),
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight => {
                self.ardy_thiever_cycle.qualified()
            }
            CoreCase::ArdyThieverFight => self.ardy_thiever_fight_cycle.qualified(),
            CoreCase::GnomeChop => self.gnome_chop_cycle.qualified(),
            CoreCase::GnomeFletchShort | CoreCase::GnomeFletchLong => {
                self.gnome_fletch_cycle.qualified()
            }
            CoreCase::CoalTrucks => self.coal_trucks_cycle.qualified(),
            CoreCase::CookBot
            | CoreCase::CookBotLobster
            | CoreCase::SmelterBot
            | CoreCase::SmelterBotSteel
            | CoreCase::FlaxSpinner
            | CoreCase::FlaxAioSpin => self.station_production_cycle.qualified(),
            CoreCase::FlaxAio => self.flax_aio_cycle.qualified(),
            CoreCase::FlaxAioPick => self.flax_aio_pick_cycle.qualified(),
            CoreCase::HerbloreSecondaries => self.herblore_eggs_cycle.qualified(),
            CoreCase::HerbloreSecondariesNewt => self.herblore_newt_cycle.qualified(),
            CoreCase::AutoFighterBank
            | CoreCase::MossGiantBank
            | CoreCase::HillGiantBank
            | CoreCase::HillGiantBankPrepared
            | CoreCase::ChaosDruidBank
            | CoreCase::ArdyFighterBank
            | CoreCase::RockCrabBank
            | CoreCase::GreenDragonBank
            | CoreCase::GreenDragonBankPrepared
            | CoreCase::GreenDragonBankDefaultPrepared
            | CoreCase::GreenDragonTele
            | CoreCase::GreenDragonTelePrepared
            | CoreCase::FireGiantBank
            | CoreCase::FireGiantBankPrepared
            | CoreCase::FireGiantCamelotPrepared => {
                match (combat_spec(self.case), combat_bank_spec(self.case)) {
                    (Some(spec), Some(bank)) => self.combat_bank_cycle.qualified(spec, bank),
                    _ => false,
                }
            }
            CoreCase::FireGiantApproach => self.combat_approach_cycle.qualified(),
            CoreCase::AioTeleport | CoreCase::AioTeleportFalador | CoreCase::AioTeleportNoStaff => {
                self.aio_teleport_cycle.qualified()
            }
            CoreCase::ShopBuyout
            | CoreCase::ShopBuyoutAubury
            | CoreCase::ShopBuyoutLowe
            | CoreCase::ShopBuyoutHickton
            | CoreCase::ShopBuyoutHarry
            | CoreCase::ShopBuyoutBetty
            | CoreCase::ShopBuyoutGerrant => self.shop_buyout_cycle.qualified(),
            CoreCase::SmithingBot | CoreCase::SmithingBotPlatebody => {
                self.smithing_bot_cycle.qualified()
            }
            CoreCase::LeatherCrafter | CoreCase::LeatherCrafterHardBody => {
                self.leather_crafter_cycle.qualified()
            }
            CoreCase::Firemaker | CoreCase::FiremakerOak => self.firemaker_cycle.qualified(),
            CoreCase::ClimbingBoots | CoreCase::ClimbingBootsTeleport => {
                match climbing_boots_spec(self.case) {
                    Some(spec) => self.climbing_boots_cycle.qualified(spec),
                    None => false,
                }
            }
            CoreCase::RangingGuildRound => self.ranging_guild_round_cycle.qualified(),
            CoreCase::RangingGuildRedeem => self.ranging_guild_redeem_cycle.qualified(),
            CoreCase::RangingGuildBank => self.ranging_guild_bank_cycle.qualified(),
            CoreCase::RangingGuildFull => self.ranging_guild_full_cycle.qualified(),
            CoreCase::BrimhavenMossInspectV1 => self.brimhaven_moss_inspect_cycle.qualified(),
            CoreCase::RouteInspectBrimhavenV2 => self.route_inspect_brimhaven_v2_cycle.qualified(),
            CoreCase::PrayerV2 | CoreCase::PrayerV1 => self.prayer_delivery_cycle.qualified(),
            CoreCase::LineOfSightV2 => self.line_of_sight_cycle.qualified(),
            CoreCase::ActorObservationV2 => self.actor_observation_cycle.qualified(),
            CoreCase::FightFieldV2 => self.fight_field_cycle.qualified(),
            CoreCase::HoldSpotV2
            | CoreCase::RetreatSpotV2
            | CoreCase::WalkSpotV2
            | CoreCase::EnterLairV2
            | CoreCase::LeaveLairV2
            | CoreCase::AcquireKeyV2
            | CoreCase::CellV2
            | CoreCase::BankV2 => self
                .case
                .hunt_cell()
                .is_some_and(|cell| self.hunt_cycle.qualified(cell)),
            CoreCase::ChaosDruid
            | CoreCase::ChaosDruidTower
            | CoreCase::ChaosDruidYanille
            | CoreCase::MossGiant
            | CoreCase::MossGiantPrepared
            | CoreCase::HillGiant
            | CoreCase::AutoFighter
            | CoreCase::AutoFighterMage
            | CoreCase::AutoFighterRange
            | CoreCase::RockCrab
            | CoreCase::RockCrabRange
            | CoreCase::GreenDragon
            | CoreCase::GreenDragonPrepared
            | CoreCase::GreenDragonSpecial
            | CoreCase::GreenDragonSpecialPrepared
            | CoreCase::GreenDragonPotions
            | CoreCase::GreenDragonPotionsPrepared
            | CoreCase::FireGiant
            | CoreCase::FireGiantPrepared
            | CoreCase::ArdyFighter => {
                combat_spec(self.case).is_some_and(|spec| self.combat_core_cycle.qualified(spec))
            }
            CoreCase::MossGiantDart => self.combat_dart_branch_cycle.qualified(),
            CoreCase::GreenDragonMagePrepared => combat_spec(self.case)
                .is_some_and(|spec| qualified_mage_branch(&self.combat_core_cycle, spec)),
        }
    }

    pub fn qualify(&self) -> Result<Value, String> {
        if self.post_start_observations == 0 {
            return Err("no post-Start observations".into());
        }
        let ok = self.qualified();
        if !ok {
            return Err(self.incomplete_delta_error());
        }
        Ok(json!({
            "case": self.case,
            "baseline": self.baseline,
            "latest": self.latest,
            "max_items": self.max_items,
            "max_xp": self.max_xp,
            "saw_bury_chat": self.saw_bury_chat,
            "post_start_observations": self.post_start_observations,
            "bone_bank_cycle": self.bone_bank_cycle,
            "bank_fletcher_cycle": self.bank_fletcher_cycle,
            "bank_fletcher_option_cycle": self.bank_fletcher_option_cycle,
            "bank_fletcher_string_cycle": self.bank_fletcher_string_cycle,
            "bank_fletcher_cut_string_cycle": self.bank_fletcher_cut_string_cycle,
            "alcher_defaults_cycle": self.alcher_defaults_cycle,
            "alcher_generated_custom_cycle": self.alcher_generated_custom_cycle,
            "alcher_spell_cycle": self.alcher_spell_cycle,
            "alcher_swarm_cycle": self.alcher_swarm_cycle,
            "dart_fletcher_cycle": self.dart_fletcher_cycle,
            "herb_cleaner_cycle": self.herb_cleaner_cycle,
            "herb_cleaner_empty_cycle": self.herb_cleaner_empty_cycle,
            "gem_cutter_cycle": self.gem_cutter_cycle,
            "door_opener_cycle": self.door_opener_cycle,
            "gnome_course_cycle": self.gnome_course_cycle,
            "wildy_agility_cycle": self.wildy_agility_cycle,
            "brimhaven_agility_cycle": self.brimhaven_agility_cycle,
            "flax_picker_cycle": self.flax_picker_cycle,
            "superheater_cycle": self.superheater_cycle,
            "chicken_killer_bank_cycle": self.chicken_killer_bank_cycle,
            "vial_filler_cycle": self.vial_filler_cycle,
            "potion_maker_cycle": self.potion_maker_cycle,
            "tanner_bot_cycle": self.tanner_bot_cycle,
            "rune_crafter_cycle": self.rune_crafter_cycle,
            "ardy_cakes_cycle": self.ardy_cakes_cycle,
            "ardy_cakes_fight_cycle": self.ardy_cakes_fight_cycle,
            "ardy_thiever_cycle": self.ardy_thiever_cycle,
            "ardy_thiever_fight_cycle": self.ardy_thiever_fight_cycle,
            "gnome_chop_cycle": self.gnome_chop_cycle,
            "gnome_fletch_cycle": self.gnome_fletch_cycle,
            "coal_trucks_cycle": self.coal_trucks_cycle,
            "station_production_cycle": self.station_production_cycle,
            "flax_aio_cycle": self.flax_aio_cycle,
            "flax_aio_pick_cycle": self.flax_aio_pick_cycle,
            "herblore_eggs_cycle": self.herblore_eggs_cycle,
            "herblore_newt_cycle": self.herblore_newt_cycle,
            "combat_core_cycle": self.combat_core_cycle,
            "combat_dart_branch_cycle": self.combat_dart_branch_cycle,
            "combat_bank_cycle": self.combat_bank_cycle,
            "combat_approach_cycle": self.combat_approach_cycle,
            "aio_teleport_cycle": self.aio_teleport_cycle,
            "shop_buyout_cycle": self.shop_buyout_cycle,
            "smithing_bot_cycle": self.smithing_bot_cycle,
            "leather_crafter_cycle": self.leather_crafter_cycle,
            "firemaker_cycle": self.firemaker_cycle,
            "climbing_boots_cycle": self.climbing_boots_cycle,
            "ranging_guild_round_cycle": self.ranging_guild_round_cycle,
            "ranging_guild_redeem_cycle": self.ranging_guild_redeem_cycle,
            "ranging_guild_bank_cycle": self.ranging_guild_bank_cycle,
            "ranging_guild_full_cycle": self.ranging_guild_full_cycle,
            "brimhaven_moss_inspect_cycle": self.brimhaven_moss_inspect_cycle,
            "route_inspect_brimhaven_v2_cycle": self.route_inspect_brimhaven_v2_cycle,
            "prayer_delivery_cycle": self.prayer_delivery_cycle,
            "line_of_sight_cycle": self.line_of_sight_cycle,
            "fight_field_cycle": self.fight_field_cycle,
            "hunt_cycle": self.hunt_cycle,
            "ordered_first_exhausted": self.ordered_first_exhausted,
        }))
    }
}
