use crate::native::{ConfigError, SettingsBag, StartError};
use crate::SettingDef;
use api::game_data::SelectedGameData;
use api::gather_methods::{first_gap, GatherCatalog, GatherMethod, GatherSkill, GatherTarget};
use api::named_banks::{BankPreferences, BANK_CATALOG};
use api::selected::{Knowledge, RequirementKind};
use api::snapshot::WorldTile;
use serde::Deserialize;
use std::sync::{Arc, LazyLock};

const ACCEPTED_PRODUCT_GAPS: &[&str] = &["incidental-gem-roll"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skill {
    Woodcutting,
    Mining,
    Fishing,
}

impl Skill {
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("woodcutting") {
            Some(Self::Woodcutting)
        } else if value.eq_ignore_ascii_case("mining") {
            Some(Self::Mining)
        } else if value.eq_ignore_ascii_case("fishing") {
            Some(Self::Fishing)
        } else {
            None
        }
    }

    pub const fn catalog(self) -> GatherSkill {
        match self {
            Self::Woodcutting => GatherSkill::Woodcutting,
            Self::Mining => GatherSkill::Mining,
            Self::Fishing => GatherSkill::Fishing,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Woodcutting => "Woodcutting",
            Self::Mining => "Mining",
            Self::Fishing => "Fishing",
        }
    }

    pub const fn stat_name(self) -> &'static str {
        match self {
            Self::Woodcutting => "woodcutting",
            Self::Mining => "mining",
            Self::Fishing => "fishing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Start,
    Custom,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPreference {
    BestTier,
    Nearest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GathererSettings {
    #[serde(default = "default_skill")]
    pub skill: String,
    #[serde(default = "default_wc_resources")]
    pub woodcutting_resources: Vec<String>,
    #[serde(default = "default_mining_resources")]
    pub mining_resources: Vec<String>,
    #[serde(default = "default_fishing_method")]
    pub fishing_method: String,
    #[serde(default = "default_target_preference")]
    pub target_preference: String,
    #[serde(default = "default_location")]
    pub location: String,
    #[serde(default)]
    pub custom_tile: Option<WorldTile>,
    #[serde(default = "default_radius")]
    pub radius: u16,
    #[serde(default = "default_disposition")]
    pub disposition: String,
    #[serde(default = "default_bank")]
    pub bank: String,
    #[serde(default)]
    pub use_mage_bank: bool,
    #[serde(default)]
    pub use_zanaris_bank: bool,
    #[serde(default = "default_bait_target")]
    pub bait_target: i32,
    #[serde(default)]
    pub food: String,
    #[serde(default)]
    pub food_target: i32,
    #[serde(default)]
    pub eat_below: i32,
    #[serde(default)]
    pub coin_target: i32,
    #[serde(default = "default_reserve_teleport")]
    pub reserve_teleport: String,
    #[serde(default)]
    pub reserve_casts: i32,
    #[serde(default)]
    pub allow_teleports: bool,
    #[serde(default)]
    pub allow_wilderness: bool,
    #[serde(default = "default_death_policy")]
    pub death_policy: String,
    #[serde(default = "default_max_deaths")]
    pub max_deaths: u8,
}

fn default_skill() -> String {
    "Woodcutting".into()
}
fn default_wc_resources() -> Vec<String> {
    vec!["normal".into()]
}
fn default_mining_resources() -> Vec<String> {
    vec!["copper".into(), "tin".into()]
}
fn default_fishing_method() -> String {
    "fishing.saltfish.op1".into()
}
fn default_target_preference() -> String {
    "Best tier".into()
}
fn default_location() -> String {
    "Start".into()
}
fn default_radius() -> u16 {
    12
}
fn default_disposition() -> String {
    "Bank".into()
}
fn default_bank() -> String {
    "Nearest".into()
}
fn default_bait_target() -> i32 {
    100
}
fn default_reserve_teleport() -> String {
    "Off".into()
}
fn default_death_policy() -> String {
    "Stop".into()
}
fn default_max_deaths() -> u8 {
    2
}

impl Default for GathererSettings {
    fn default() -> Self {
        Self {
            skill: default_skill(),
            woodcutting_resources: default_wc_resources(),
            mining_resources: default_mining_resources(),
            fishing_method: default_fishing_method(),
            target_preference: default_target_preference(),
            location: default_location(),
            custom_tile: None,
            radius: default_radius(),
            disposition: default_disposition(),
            bank: default_bank(),
            use_mage_bank: false,
            use_zanaris_bank: false,
            bait_target: default_bait_target(),
            food: String::new(),
            food_target: 0,
            eat_below: 0,
            coin_target: 0,
            reserve_teleport: default_reserve_teleport(),
            reserve_casts: 0,
            allow_teleports: false,
            allow_wilderness: false,
            death_policy: default_death_policy(),
            max_deaths: default_max_deaths(),
        }
    }
}

impl GathererSettings {
    pub fn from_bag(bag: &SettingsBag) -> Result<Self, ConfigError> {
        Self::deserialize(serde::de::value::MapDeserializer::new(
            bag.iter().map(|(key, value)| (key.as_str(), value)),
        ))
        .map_err(|error| ConfigError::new("", "invalid-settings", error.to_string()))
        .and_then(|settings| settings.validate())
    }

    pub fn validate(mut self) -> Result<Self, ConfigError> {
        let skill = Skill::parse(&self.skill).ok_or_else(|| {
            ConfigError::new(
                "skill",
                "unsupported-skill",
                "supports Woodcutting, Mining and Fishing",
            )
        })?;
        if !(2..=64).contains(&self.radius) {
            return Err(ConfigError::new(
                "radius",
                "invalid-radius",
                "radius must be between 2 and 64",
            ));
        }
        if TargetPreference::parse(&self.target_preference).is_none() {
            return Err(ConfigError::new(
                "targetPreference",
                "invalid-option",
                "targetPreference must be Best tier or Nearest",
            ));
        }
        if Location::parse(&self.location).is_none() {
            return Err(ConfigError::new(
                "location",
                "invalid-option",
                "offers Start, Custom and Auto locations",
            ));
        }
        if self.location.eq_ignore_ascii_case("custom") && self.custom_tile.is_none() {
            return Err(ConfigError::new(
                "customTile",
                "required",
                "Custom location needs a tile",
            ));
        }
        if self.disposition.eq_ignore_ascii_case("power") {
            self.disposition = "Power".into();
        } else if self.disposition.eq_ignore_ascii_case("bank") {
            self.disposition = "Bank".into();
        } else {
            return Err(ConfigError::new(
                "disposition",
                "invalid-option",
                "disposition must be Power or Bank",
            ));
        }
        if self.bank.eq_ignore_ascii_case("nearest") {
            self.bank = "Nearest".into();
        } else if let Some(bank) = BANK_CATALOG
            .iter()
            .find(|bank| bank.name.eq_ignore_ascii_case(self.bank.trim()))
        {
            self.bank = bank.name.to_string();
        } else {
            return Err(ConfigError::new(
                "bank",
                "invalid-option",
                "bank must be Nearest or a named bank from the catalog",
            ));
        }
        if !(1..=10_000).contains(&self.bait_target) {
            return Err(ConfigError::new(
                "baitTarget",
                "invalid-range",
                "baitTarget must be between 1 and 10000",
            ));
        }
        if !(0..=28).contains(&self.food_target) {
            return Err(ConfigError::new(
                "foodTarget",
                "invalid-range",
                "foodTarget must be between 0 and 28",
            ));
        }
        if !(0..=99).contains(&self.eat_below) {
            return Err(ConfigError::new(
                "eatBelow",
                "invalid-range",
                "eatBelow must be between 0 and 99",
            ));
        }
        if !(0..=2_000_000_000).contains(&self.coin_target) {
            return Err(ConfigError::new(
                "coinTarget",
                "invalid-range",
                "coinTarget must be between 0 and 2000000000",
            ));
        }
        if !(0..=1_000).contains(&self.reserve_casts) {
            return Err(ConfigError::new(
                "reserveCasts",
                "invalid-range",
                "reserveCasts must be between 0 and 1000",
            ));
        }
        if self.food_target > 0 && self.food.trim().is_empty() {
            return Err(ConfigError::new(
                "food",
                "required",
                "foodTarget requires a food item name",
            ));
        }
        if self.reserve_casts > 0 && self.reserve_teleport.eq_ignore_ascii_case("off") {
            return Err(ConfigError::new(
                "reserveTeleport",
                "required",
                "reserveCasts requires a selected teleport spell",
            ));
        }
        if self.reserve_casts == 0
            && !self.reserve_teleport.trim().is_empty()
            && !self.reserve_teleport.eq_ignore_ascii_case("off")
        {
            return Err(ConfigError::new(
                "reserveCasts",
                "required",
                "an enabled reserve teleport requires at least one cast",
            ));
        }
        if !self.death_policy.eq_ignore_ascii_case("stop") {
            return Err(ConfigError::new(
                "deathPolicy",
                "staged-option",
                "death recovery is introduced in G4a",
            ));
        }
        let resources = self.resources_for(skill);
        if resources.is_empty() || resources.iter().any(|resource| resource.trim().is_empty()) {
            return Err(ConfigError::new(
                match skill {
                    Skill::Woodcutting => "woodcuttingResources",
                    Skill::Mining => "miningResources",
                    Skill::Fishing => "fishingMethod",
                },
                "required",
                "select at least one resource",
            ));
        }
        Ok(self)
    }

    pub fn skill_kind(&self) -> Skill {
        Skill::parse(&self.skill).expect("validated GathererSettings skill")
    }

    pub fn resources_for(&self, skill: Skill) -> &[String] {
        match skill {
            Skill::Woodcutting => &self.woodcutting_resources,
            Skill::Mining => &self.mining_resources,
            Skill::Fishing => std::slice::from_ref(&self.fishing_method),
        }
    }

    pub fn location_mode(&self) -> Result<Location, crate::gatherer::area::AreaError> {
        Location::parse(&self.location).ok_or(crate::gatherer::area::AreaError::Invalid)
    }

    pub fn target_preference_kind(&self) -> TargetPreference {
        TargetPreference::parse(&self.target_preference).expect("validated targetPreference")
    }

    pub fn restart_required_changed(&self, other: &Self) -> bool {
        self.skill != other.skill
            || self.woodcutting_resources != other.woodcutting_resources
            || self.mining_resources != other.mining_resources
            || self.fishing_method != other.fishing_method
            || self.location != other.location
            || self.custom_tile != other.custom_tile
    }

    pub fn boundary_changed(&self, other: &Self) -> bool {
        self.target_preference != other.target_preference
            || self.radius != other.radius
            || self.disposition != other.disposition
            || self.allow_teleports != other.allow_teleports
            || self.bank != other.bank
            || self.use_mage_bank != other.use_mage_bank
            || self.use_zanaris_bank != other.use_zanaris_bank
            || self.bait_target != other.bait_target
            || self.food != other.food
            || self.food_target != other.food_target
            || self.eat_below != other.eat_below
            || self.coin_target != other.coin_target
            || self.reserve_teleport != other.reserve_teleport
            || self.reserve_casts != other.reserve_casts
            || self.allow_wilderness != other.allow_wilderness
    }

    pub fn bank_preferences(&self) -> BankPreferences {
        BankPreferences {
            use_mage_bank: self.use_mage_bank,
            use_zanaris_bank: self.use_zanaris_bank,
        }
    }
    pub fn resources_field(&self) -> &'static str {
        match self.skill_kind() {
            Skill::Woodcutting => "woodcuttingResources",
            Skill::Mining => "miningResources",
            Skill::Fishing => "fishingMethod",
        }
    }
}

impl Location {
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("start") {
            Some(Self::Start)
        } else if value.eq_ignore_ascii_case("custom") {
            Some(Self::Custom)
        } else if value.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else {
            None
        }
    }
}

impl TargetPreference {
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("best tier") || value.eq_ignore_ascii_case("best-tier") {
            Some(Self::BestTier)
        } else if value.eq_ignore_ascii_case("nearest") {
            Some(Self::Nearest)
        } else {
            None
        }
    }
}

/// Resolve selected settings to catalog method indices, applying the one
/// admission rule from the G1 contract. The returned bitset is safe for the
/// 40-method 289 catalog; an unexpectedly larger catalog refuses explicitly.
pub fn resolve_methods(
    settings: &GathererSettings,
    selected: &SelectedGameData,
    catalog: &GatherCatalog,
) -> Result<(u64, Arc<[usize]>), StartError> {
    let skill = settings.skill_kind().catalog();
    let skill_name = settings.skill_kind().stat_name();
    let resources = settings.resources_for(settings.skill_kind());
    let mut indices = Vec::new();
    for resource in resources {
        let option = selected
            .gather_option(skill_name, resource)
            .ok_or_else(|| {
                StartError::Config(ConfigError::new(
                    settings.resources_field(),
                    "unknown-resource",
                    format!("resource {resource} is not in the selected gathering family"),
                ))
            })?;
        if !option.selectable {
            return Err(StartError::Config(ConfigError::new(
                settings.resources_field(),
                "method-incomplete",
                option.gap.as_deref().unwrap_or("resource-not-selectable"),
            )));
        }
        let method = catalog.method(&option.method).map_err(StartError::Facts)?;
        if method.skill != skill {
            return Err(StartError::Config(ConfigError::new(
                settings.resources_field(),
                "unknown-resource",
                format!("resource {resource} resolved to another skill"),
            )));
        }
        admit_method(settings.resources_field(), method)?;
        let index = catalog
            .methods()
            .iter()
            .position(|candidate| std::ptr::eq(candidate, method))
            .ok_or_else(|| StartError::Unavailable("gather method index unavailable".into()))?;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    if indices.is_empty() {
        return Err(StartError::Config(ConfigError::new(
            settings.resources_field(),
            "required",
            "no method selected",
        )));
    }
    if indices.iter().any(|index| *index >= u64::BITS as usize) {
        return Err(StartError::Unavailable(
            "gather catalog has more than 64 methods".into(),
        ));
    }
    let bits = indices
        .iter()
        .fold(0_u64, |bits, index| bits | (1_u64 << index));
    Ok((bits, Arc::from(indices)))
}

fn admit_method(field: &str, method: &GatherMethod) -> Result<(), StartError> {
    if !target_admits_method(field, method)? {
        let reason = first_gap(&method.targets).map_or_else(
            || "no-resource-target".to_owned(),
            |gap| gap.code.to_string(),
        );
        return Err(StartError::Config(ConfigError::new(
            field,
            "method-incomplete",
            reason,
        )));
    }
    let missing = first_gap(&method.tools)
        .or_else(|| first_gap(&method.consumes))
        .or_else(|| first_gap(&method.requirements))
        .or_else(|| first_gap(&method.spots));
    if let Some(gap) = missing {
        return Err(StartError::Config(ConfigError::new(
            field,
            "method-incomplete",
            gap.code.to_string(),
        )));
    }
    match &method.products {
        Knowledge::Known(_) => Ok(()),
        Knowledge::Partial { gaps, .. }
            if gaps
                .iter()
                .all(|gap| ACCEPTED_PRODUCT_GAPS.contains(&gap.code.as_ref())) =>
        {
            Ok(())
        }
        Knowledge::Partial { gaps, .. } => Err(StartError::Config(ConfigError::new(
            field,
            "method-incomplete",
            gaps.first()
                .map_or_else(|| "products-partial".into(), |gap| gap.code.to_string()),
        ))),
        Knowledge::Unknown(gap) => Err(StartError::Config(ConfigError::new(
            field,
            "method-incomplete",
            gap.code.to_string(),
        ))),
    }
}

/// Partial target coverage is useful when at least one target row has a
/// complete per-target fact. Incomplete rows are filtered during selection.
fn target_admits_method(field: &str, method: &GatherMethod) -> Result<bool, StartError> {
    let rows = match &method.targets {
        Knowledge::Known(rows) | Knowledge::Partial { known: rows, .. } => rows,
        Knowledge::Unknown(gap) => {
            return Err(StartError::Config(ConfigError::new(
                field,
                "method-incomplete",
                gap.code.to_string(),
            )))
        }
    };
    Ok(rows
        .iter()
        .any(|target: &GatherTarget| matches!(target.respawn, Knowledge::Known(_))))
}

/// A level requirement carried by the method's selected skill rows.
pub fn method_level(method: &GatherMethod, stat_index: i32) -> i32 {
    match &method.requirements {
        Knowledge::Known(requirements) => requirements
            .iter()
            .filter_map(|requirement| match requirement.kind {
                RequirementKind::Skill(minimum) if i32::from(minimum.skill) == stat_index => {
                    Some(i32::from(minimum.level))
                }
                _ => None,
            })
            .max()
            .unwrap_or(1),
        _ => i32::MAX,
    }
}

pub fn has_members_requirement(method: &GatherMethod) -> bool {
    matches!(&method.requirements, Knowledge::Known(rows) if rows.iter().any(|row| matches!(row.kind, RequirementKind::MembersWorld)))
}

pub fn schema() -> &'static [SettingDef] {
    SCHEMA.as_slice()
}

static SCHEMA: LazyLock<Vec<SettingDef>> = LazyLock::new(|| {
    vec![
        setting(
            "skill",
            "string",
            Some("Woodcutting"),
            &["Woodcutting", "Mining", "Fishing"],
            None,
            None,
        ),
        setting_with(
            "woodcuttingResources",
            "list",
            Some("normal"),
            &[],
            Some("{ key: 'skill', anyOf: ['Woodcutting'] }"),
            Some("gather:woodcutting"),
        ),
        setting_with(
            "miningResources",
            "list",
            Some("[\"copper\",\"tin\"]"),
            &[],
            Some("{ key: 'skill', anyOf: ['Mining'] }"),
            Some("gather:mining"),
        ),
        setting(
            "fishingMethod",
            "string",
            Some("fishing.saltfish.op1"),
            &[],
            Some("{ key: 'skill', anyOf: ['Fishing'] }"),
            Some("gather:fishing"),
        ),
        setting(
            "targetPreference",
            "string",
            Some("Best tier"),
            &["Best tier", "Nearest"],
            Some("{ key: 'skill', anyOf: ['Woodcutting', 'Mining'] }"),
            None,
        ),
        setting(
            "location",
            "string",
            Some("Start"),
            &["Start", "Custom", "Auto"],
            None,
            None,
        ),
        setting(
            "customTile",
            "tile",
            None,
            &[],
            Some("{ key: 'location', anyOf: ['Custom'] }"),
            None,
        ),
        number_setting("radius", "12", "2", "64"),
        setting_with(
            "disposition",
            "string",
            Some("Bank"),
            &["Power", "Bank"],
            None,
            None,
        ),
        setting_with(
            "bank",
            "string",
            Some("Nearest"),
            &["Nearest"],
            Some("{ key: 'disposition', anyOf: ['Bank'] }"),
            Some("named-banks"),
        ),
        boolean_setting_show_if(
            "useMageBank",
            false,
            "{ key: 'disposition', anyOf: ['Bank'] }",
        ),
        boolean_setting_show_if(
            "useZanarisBank",
            false,
            "{ key: 'disposition', anyOf: ['Bank'] }",
        ),
        number_setting_show_if(
            "baitTarget",
            "100",
            "1",
            "10000",
            "{ key: 'skill', anyOf: ['Fishing'] }",
        ),
        setting_group(
            setting("food", "string", Some(""), &[], None, None),
            "Safety",
        ),
        setting_group(number_setting("foodTarget", "0", "0", "28"), "Safety"),
        setting_group(number_setting("eatBelow", "0", "0", "99"), "Safety"),
        number_setting("coinTarget", "0", "0", "2000000000"),
        setting(
            "reserveTeleport",
            "string",
            Some("Off"),
            &["Off"],
            None,
            Some("gatherer-teleports"),
        ),
        number_setting("reserveCasts", "0", "0", "1000"),
        setting("deathPolicy", "string", Some("Stop"), &["Stop"], None, None),
        number_setting("maxDeaths", "2", "0", "255"),
    ]
});

fn setting(
    id: &str,
    ty: &str,
    default: Option<&str>,
    options: &[&str],
    show_if: Option<&str>,
    options_from: Option<&str>,
) -> SettingDef {
    SettingDef {
        id: id.into(),
        ty: ty.into(),
        default: default.map(str::to_string),
        label: None,
        min: None,
        max: None,
        step: None,
        options: options.iter().map(|value| (*value).into()).collect(),
        option_labels: Vec::new(),
        group: None,
        show_if: show_if.map(str::to_string),
        options_from: options_from.map(str::to_string),
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    }
}

fn setting_with(
    id: &str,
    ty: &str,
    default: Option<&str>,
    options: &[&str],
    show_if: Option<&str>,
    options_from: Option<&str>,
) -> SettingDef {
    setting(id, ty, default, options, show_if, options_from)
}

fn number_setting(id: &str, default: &str, min: &str, max: &str) -> SettingDef {
    let mut def = setting(id, "number", Some(default), &[], None, None);
    def.min = Some(min.into());
    def.max = Some(max.into());
    def.step = Some("1".into());
    def
}

fn boolean_setting(id: &str, default: bool) -> SettingDef {
    setting(
        id,
        "boolean",
        Some(if default { "true" } else { "false" }),
        &[],
        None,
        None,
    )
}
fn setting_group(mut def: SettingDef, group: &str) -> SettingDef {
    def.group = Some(group.into());
    def
}

fn number_setting_show_if(
    id: &str,
    default: &str,
    min: &str,
    max: &str,
    show_if: &str,
) -> SettingDef {
    let mut def = number_setting(id, default, min, max);
    def.show_if = Some(show_if.into());
    def
}

fn boolean_setting_show_if(id: &str, default: bool, show_if: &str) -> SettingDef {
    let mut def = boolean_setting(id, default);
    def.show_if = Some(show_if.into());
    def
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_accept_g3_defaults_and_reject_unsupported_modes() {
        let defaults = GathererSettings::from_bag(&SettingsBag::new()).unwrap();
        assert_eq!(defaults.disposition, "Bank");
        assert_eq!(defaults.bank, "Nearest");
        assert_eq!(defaults.bank_preferences(), BankPreferences::default());
        assert_eq!(defaults.death_policy, "Stop");

        let mut bag = SettingsBag::new();
        bag.insert("skill".into(), json!("Mining"));
        bag.insert("miningResources".into(), json!(["copper"]));
        let parsed = GathererSettings::from_bag(&bag).unwrap();
        assert_eq!(parsed.skill_kind(), Skill::Mining);
        assert_eq!(parsed.mining_resources, ["copper"]);
        assert_eq!(parsed.disposition, "Bank");

        bag.insert("disposition".into(), json!("Power"));
        bag.insert("bank".into(), json!("Varrock East"));
        let parsed = GathererSettings::from_bag(&bag).unwrap();
        assert_eq!(parsed.disposition, "Power");
        assert_eq!(parsed.bank, "Varrock East");

        bag.insert("location".into(), json!("Closest"));
        assert_eq!(
            GathererSettings::from_bag(&bag).unwrap_err().code.as_ref(),
            "invalid-option"
        );
    }

    #[test]
    fn supply_settings_validate_bounds_and_required_pairs() {
        for (field, value) in [
            ("baitTarget", json!(0)),
            ("foodTarget", json!(29)),
            ("eatBelow", json!(100)),
            ("coinTarget", json!(-1)),
            ("reserveCasts", json!(1001)),
        ] {
            let mut bag = SettingsBag::new();
            bag.insert(field.into(), value);
            assert_eq!(
                GathererSettings::from_bag(&bag).unwrap_err().code.as_ref(),
                "invalid-range",
                "{field}"
            );
        }

        let mut bag = SettingsBag::new();
        bag.insert("foodTarget".into(), json!(1));
        assert_eq!(
            GathererSettings::from_bag(&bag).unwrap_err().field.as_ref(),
            "food"
        );
        bag.insert("food".into(), json!("Trout"));
        bag.insert("reserveCasts".into(), json!(1));
        assert_eq!(
            GathererSettings::from_bag(&bag).unwrap_err().field.as_ref(),
            "reserveTeleport"
        );
        bag.insert("reserveTeleport".into(), json!("Trollheim"));
        bag.insert("reserveCasts".into(), json!(0));
        assert_eq!(
            GathererSettings::from_bag(&bag).unwrap_err().field.as_ref(),
            "reserveCasts"
        );

        bag.insert("bank".into(), json!("Closest"));
        bag.insert("reserveCasts".into(), json!(0));
        assert_eq!(
            GathererSettings::from_bag(&bag).unwrap_err().field.as_ref(),
            "bank"
        );
    }

    #[test]
    fn partial_target_coverage_admits_complete_rows() {
        use api::gather_methods::{GatherTarget, TargetClass};
        use api::selected::{EntityId, FactKey};

        let complete = GatherTarget {
            entity: EntityId::Loc(1),
            op: 1,
            class: TargetClass::Resource,
            respawn: Knowledge::Known(None),
        };
        let incomplete = GatherTarget {
            entity: EntityId::Loc(2),
            op: 1,
            class: TargetClass::Resource,
            respawn: Knowledge::Unknown(api::selected::Gap {
                code: Arc::from("custom-handler-target"),
                sources: Arc::from([]),
            }),
        };
        let mut method = GatherMethod {
            id: FactKey::new("mining.copper"),
            skill: GatherSkill::Mining,
            resources: Arc::from([]),
            targets: Knowledge::Known(Arc::from([])),
            products: Knowledge::Known(Arc::from([])),
            tools: Knowledge::Known(Arc::from([])),
            consumes: Knowledge::Known(Arc::from([])),
            requirements: Knowledge::Known(Arc::from([])),
            spots: Knowledge::Known(Arc::from([])),
        };
        method.targets = Knowledge::Partial {
            known: Arc::from([complete, incomplete]),
            gaps: Arc::from([]),
        };
        assert!(target_admits_method("miningResources", &method).unwrap());
    }
}
