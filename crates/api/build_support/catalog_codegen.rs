use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use std::fmt::Write as _;

const BANK_CATALOG_NAME: &str = "bank-catalog.json";
const COOK_CATALOG_NAME: &str = "cook-catalog.json";

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Deserialize)]
struct BankCatalog {
    schema: i32,
    facts: Vec<BankFact>,
}

#[derive(Deserialize)]
struct BankFact {
    name: String,
    tile: Tile,
    #[serde(deserialize_with = "required_nullable")]
    approach: Option<Tile>,
    #[serde(deserialize_with = "required_nullable")]
    skill: Option<[i32; 2]>,
    #[serde(deserialize_with = "required_nullable")]
    quest: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    setting: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    object: Option<Operation>,
    #[serde(deserialize_with = "required_nullable")]
    open_first: Option<Operation>,
    #[serde(deserialize_with = "required_nullable")]
    npc: Option<Operation>,
    #[serde(deserialize_with = "required_nullable")]
    choose: Option<String>,
}

#[derive(Deserialize)]
struct Operation {
    name: String,
    op: String,
}

#[derive(Deserialize)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Deserialize)]
struct CookCatalog {
    schema: i32,
    facts: CookFacts,
}

#[derive(Deserialize)]
struct CookFacts {
    max_surface_cheb: i32,
    derived_arrive_radius: i32,
    default_obstacles: Vec<String>,
    camps: Vec<CookCampFact>,
}

#[derive(Deserialize)]
struct CookCampFact {
    bank: String,
    stand: Tile,
    #[serde(deserialize_with = "required_nullable")]
    approach: Option<Tile>,
    loc: Tile,
    loc_name: String,
    kind: CookKind,
    label: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum CookKind {
    Oven,
    Fire,
}

fn parse_catalog<T: DeserializeOwned>(json: &str, catalog_name: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|error| format!("{catalog_name}: {error}"))
}

fn check_schema(catalog_name: &str, schema: i32) -> Result<(), String> {
    if schema == 1 {
        Ok(())
    } else {
        Err(format!(
            "{catalog_name}: unsupported schema {schema}; expected 1"
        ))
    }
}

fn write_tile(output: &mut String, tile: &Tile) {
    write!(
        output,
        "WorldTile {{ x: {}, z: {}, level: {} }}",
        tile.x, tile.z, tile.level
    )
    .expect("formatting into String cannot fail");
}

fn write_string(output: &mut String, value: &str) {
    write!(output, "{value:?}").expect("formatting into String cannot fail");
}

fn write_operation(output: &mut String, operation: &Operation) {
    output.push_str("Some(BankOperation { name: ");
    write_string(output, &operation.name);
    output.push_str(", op: ");
    write_string(output, &operation.op);
    output.push_str(" })");
}

pub fn bank_catalog_rust(json: &str) -> Result<String, String> {
    let catalog: BankCatalog = parse_catalog(json, BANK_CATALOG_NAME)?;
    check_schema(BANK_CATALOG_NAME, catalog.schema)?;

    let mut output = String::from("pub const BANK_CATALOG: &[BankDefinition] = &[\n");
    for bank in catalog.facts {
        output.push_str("    BankDefinition { name: ");
        write_string(&mut output, &bank.name);
        output.push_str(", tile: ");
        write_tile(&mut output, &bank.tile);
        output.push_str(", approach: ");
        match bank.approach {
            Some(tile) => {
                output.push_str("Some(");
                write_tile(&mut output, &tile);
                output.push(')');
            }
            None => output.push_str("None"),
        }
        output.push_str(", skill: ");
        match bank.skill {
            Some([skill, minimum]) => write!(output, "Some(({skill}, {minimum}))")
                .expect("formatting into String cannot fail"),
            None => output.push_str("None"),
        }
        output.push_str(", quest: ");
        match bank.quest {
            Some(value) => {
                output.push_str("Some(");
                write_string(&mut output, &value);
                output.push(')');
            }
            None => output.push_str("None"),
        }
        output.push_str(", setting: ");
        match bank.setting {
            Some(value) => {
                output.push_str("Some(");
                write_string(&mut output, &value);
                output.push(')');
            }
            None => output.push_str("None"),
        }
        output.push_str(", object: ");
        match bank.object {
            Some(operation) => write_operation(&mut output, &operation),
            None => output.push_str("None"),
        }
        output.push_str(", open_first: ");
        match bank.open_first {
            Some(operation) => write_operation(&mut output, &operation),
            None => output.push_str("None"),
        }
        output.push_str(", npc: ");
        match bank.npc {
            Some(operation) => write_operation(&mut output, &operation),
            None => output.push_str("None"),
        }
        output.push_str(", choose: ");
        match bank.choose {
            Some(value) => {
                output.push_str("Some(");
                write_string(&mut output, &value);
                output.push(')');
            }
            None => output.push_str("None"),
        }
        output.push_str(" },\n");
    }
    output.push_str("];\n");
    Ok(output)
}

pub fn cook_catalog_rust(json: &str) -> Result<String, String> {
    let catalog: CookCatalog = parse_catalog(json, COOK_CATALOG_NAME)?;
    check_schema(COOK_CATALOG_NAME, catalog.schema)?;

    let mut output = String::new();
    writeln!(
        output,
        "pub const MAX_SURFACE_CHEB: i32 = {};",
        catalog.facts.max_surface_cheb
    )
    .expect("formatting into String cannot fail");
    writeln!(
        output,
        "pub const DERIVED_ARRIVE_RADIUS: i32 = {};",
        catalog.facts.derived_arrive_radius
    )
    .expect("formatting into String cannot fail");
    output.push_str("pub const DEFAULT_OBSTACLES: &[&str] = &[\n");
    for obstacle in &catalog.facts.default_obstacles {
        output.push_str("    ");
        write_string(&mut output, obstacle);
        output.push_str(",\n");
    }
    output.push_str("];\n");
    output.push_str("pub const COOK_CAMPS: &[CookCamp] = &[\n");
    for camp in catalog.facts.camps {
        output.push_str("    CookCamp { bank: ");
        write_string(&mut output, &camp.bank);
        output.push_str(", stand: ");
        write_tile(&mut output, &camp.stand);
        output.push_str(", approach: ");
        match camp.approach {
            Some(tile) => {
                output.push_str("Some(");
                write_tile(&mut output, &tile);
                output.push(')');
            }
            None => output.push_str("None"),
        }
        output.push_str(", loc: ");
        write_tile(&mut output, &camp.loc);
        output.push_str(", loc_name: ");
        write_string(&mut output, &camp.loc_name);
        output.push_str(", kind: CookSurfaceKind::");
        output.push_str(match camp.kind {
            CookKind::Oven => "Oven",
            CookKind::Fire => "Fire",
        });
        output.push_str(", label: ");
        write_string(&mut output, &camp.label);
        output.push_str(" },\n");
    }
    output.push_str("];\n");
    Ok(output)
}

const GATHER_CATALOG_NAME: &str = "gather-catalog.json";

#[derive(Deserialize)]
struct GatherCatalog {
    schema: i32,
    facts: GatherFacts,
}

#[derive(Deserialize)]
struct GatherFacts {
    woodcutting_camps: Vec<GatherCampFact>,
    mining_camps: Vec<GatherCampFact>,
    fishing_camps: Vec<GatherCampFact>,
    shilo_water_vendor: WaterVendorFact,
    fishing: FishingFact,
    cook: FishCookFact,
    quest_junk: Vec<QuestJunkFact>,
}

#[derive(Deserialize)]
struct GatherCampFact {
    name: String,
    spot: Tile,
    bank_stand: Tile,
    #[serde(deserialize_with = "required_nullable")]
    booth_name: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    booth_op: Option<String>,
    verified: bool,
    #[serde(deserialize_with = "required_nullable")]
    obstacles: Option<Vec<String>>,
    #[serde(deserialize_with = "required_nullable")]
    camp_radius: Option<i32>,
    #[serde(deserialize_with = "required_nullable")]
    chase_radius: Option<i32>,
    #[serde(deserialize_with = "required_nullable")]
    resources: Option<Vec<String>>,
    #[serde(deserialize_with = "required_nullable")]
    avoid_spots: Option<Vec<Tile>>,
    #[serde(deserialize_with = "required_nullable")]
    sweep: Option<Vec<Tile>>,
    #[serde(deserialize_with = "required_nullable")]
    bait_vendor: Option<BaitVendorFact>,
    #[serde(deserialize_with = "required_nullable")]
    notes: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    recommended_combat: Option<i32>,
    keys: Vec<CampKeyFact>,
}

#[derive(Deserialize)]
struct BaitVendorFact {
    keeper: String,
    stand: Tile,
    price: i32,
    item: String,
}

#[derive(Deserialize)]
struct WaterVendorFact {
    keeper: String,
    stand: Tile,
    item: String,
}

/// The camp object's own properties the generator reads from the upstream literal.
#[derive(Deserialize, Clone, Copy)]
enum CampKeyFact {
    Name,
    Spot,
    BankStand,
    BoothName,
    BoothOp,
    Verified,
    Obstacles,
    CampRadius,
    ChaseRadius,
    Resources,
    AvoidSpots,
    Sweep,
    BaitVendor,
    Notes,
    RecommendedCombat,
}

impl CampKeyFact {
    fn variant(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Spot => "Spot",
            Self::BankStand => "BankStand",
            Self::BoothName => "BoothName",
            Self::BoothOp => "BoothOp",
            Self::Verified => "Verified",
            Self::Obstacles => "Obstacles",
            Self::CampRadius => "CampRadius",
            Self::ChaseRadius => "ChaseRadius",
            Self::Resources => "Resources",
            Self::AvoidSpots => "AvoidSpots",
            Self::Sweep => "Sweep",
            Self::BaitVendor => "BaitVendor",
            Self::Notes => "Notes",
            Self::RecommendedCombat => "RecommendedCombat",
        }
    }
}

#[derive(Deserialize)]
struct FishingFact {
    methods: Vec<FishingMethodFact>,
    whirlpool_ids: Vec<i32>,
    shop_costs: Vec<(String, i32)>,
    gerrant_only: Vec<String>,
    gerrant: FishingVendorFact,
    harry: FishingVendorFact,
}

#[derive(Deserialize)]
struct FishingMethodFact {
    name: String,
    op: String,
    pair: String,
    gear: Vec<GearPieceFact>,
}

#[derive(Deserialize)]
struct GearPieceFact {
    name: String,
    min: i32,
    restock: i32,
}

#[derive(Deserialize)]
struct FishingVendorFact {
    keeper: String,
    stand: Tile,
    bank_stand: Tile,
}

#[derive(Deserialize)]
struct FishCookFact {
    surfaces: Vec<FishCookSurfaceFact>,
    plans: Vec<FishCookPlanFact>,
}

#[derive(Deserialize)]
struct FishCookSurfaceFact {
    approach_first: bool,
    #[serde(deserialize_with = "required_nullable")]
    approach: Option<Tile>,
    stand: Tile,
    #[serde(deserialize_with = "required_nullable")]
    loc: Option<Tile>,
    loc_name: String,
    kind: FishKindFact,
    #[serde(deserialize_with = "required_nullable")]
    label: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    notes: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum FishKindFact {
    Range,
    Fire,
    Fireplace,
}

#[derive(Deserialize)]
struct FishCookPlanFact {
    camp: String,
    #[serde(deserialize_with = "required_nullable")]
    pier: Option<usize>,
    #[serde(deserialize_with = "required_nullable")]
    bank: Option<usize>,
}

#[derive(Deserialize)]
struct QuestJunkFact {
    id: i32,
    name: String,
    quest: String,
}

/// The generated Rust text of each gather module; every field is one `include!` body.
pub struct GatherCatalogRust {
    pub camps: String,
    pub fishing: String,
    pub cook: String,
    pub quest_junk: String,
}

fn write_option<T>(output: &mut String, value: Option<T>, emit: impl FnOnce(&mut String, T)) {
    match value {
        Some(value) => {
            output.push_str("Some(");
            emit(output, value);
            output.push(')');
        }
        None => output.push_str("None"),
    }
}

fn write_slice<T>(output: &mut String, values: &[T], emit: impl Fn(&mut String, &T)) {
    output.push_str("&[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        emit(output, value);
    }
    output.push(']');
}

fn write_number(output: &mut String, value: impl std::fmt::Display) {
    write!(output, "{value}").expect("formatting into String cannot fail");
}

fn write_camp_table(output: &mut String, constant: &str, camps: &[GatherCampFact]) {
    writeln!(output, "pub const {constant}: &[GatherCamp] = &[")
        .expect("formatting into String cannot fail");
    for camp in camps {
        output.push_str("    GatherCamp { name: ");
        write_string(output, &camp.name);
        output.push_str(", spot: ");
        write_tile(output, &camp.spot);
        output.push_str(", bank_stand: ");
        write_tile(output, &camp.bank_stand);
        output.push_str(", booth_name: ");
        write_option(output, camp.booth_name.as_deref(), write_string);
        output.push_str(", booth_op: ");
        write_option(output, camp.booth_op.as_deref(), write_string);
        write!(output, ", verified: {}, obstacles: ", camp.verified)
            .expect("formatting into String cannot fail");
        write_option(output, camp.obstacles.as_deref(), |output, values| {
            write_slice(output, values, |output, value| write_string(output, value));
        });
        output.push_str(", camp_radius: ");
        write_option(output, camp.camp_radius, write_number);
        output.push_str(", chase_radius: ");
        write_option(output, camp.chase_radius, write_number);
        output.push_str(", resources: ");
        write_option(output, camp.resources.as_deref(), |output, values| {
            write_slice(output, values, |output, value| write_string(output, value));
        });
        output.push_str(", avoid_spots: ");
        write_option(output, camp.avoid_spots.as_deref(), |output, values| {
            write_slice(output, values, write_tile);
        });
        output.push_str(", sweep: ");
        write_option(output, camp.sweep.as_deref(), |output, values| {
            write_slice(output, values, write_tile);
        });
        output.push_str(", bait_vendor: ");
        write_option(output, camp.bait_vendor.as_ref(), |output, vendor| {
            output.push_str("BaitVendor { keeper: ");
            write_string(output, &vendor.keeper);
            output.push_str(", stand: ");
            write_tile(output, &vendor.stand);
            write!(output, ", price: {}, item: ", vendor.price)
                .expect("formatting into String cannot fail");
            write_string(output, &vendor.item);
            output.push_str(" }");
        });
        output.push_str(", notes: ");
        write_option(output, camp.notes.as_deref(), write_string);
        output.push_str(", recommended_combat: ");
        write_option(output, camp.recommended_combat, write_number);
        output.push_str(", keys: ");
        write_slice(output, &camp.keys, |output, key| {
            output.push_str("CampKey::");
            output.push_str(key.variant());
        });
        output.push_str(" },\n");
    }
    output.push_str("];\n");
}

fn write_fishing_vendor(output: &mut String, vendor: &FishingVendorFact) {
    output.push_str("FishingVendor { keeper: ");
    write_string(output, &vendor.keeper);
    output.push_str(", stand: ");
    write_tile(output, &vendor.stand);
    output.push_str(", bank_stand: ");
    write_tile(output, &vendor.bank_stand);
    output.push_str(" }");
}

pub fn gather_catalog_rust(json: &str) -> Result<GatherCatalogRust, String> {
    let catalog: GatherCatalog = parse_catalog(json, GATHER_CATALOG_NAME)?;
    check_schema(GATHER_CATALOG_NAME, catalog.schema)?;
    let facts = catalog.facts;

    let mut camps = String::new();
    write_camp_table(&mut camps, "WOODCUTTING_CAMPS", &facts.woodcutting_camps);
    write_camp_table(&mut camps, "MINING_CAMPS", &facts.mining_camps);
    write_camp_table(&mut camps, "FISHING_CAMPS", &facts.fishing_camps);
    camps.push_str("pub const SHILO_WATER_VENDOR: WaterVendor = WaterVendor { keeper: ");
    write_string(&mut camps, &facts.shilo_water_vendor.keeper);
    camps.push_str(", stand: ");
    write_tile(&mut camps, &facts.shilo_water_vendor.stand);
    camps.push_str(", item: ");
    write_string(&mut camps, &facts.shilo_water_vendor.item);
    camps.push_str(" };\n");

    let fishing_facts = facts.fishing;
    let mut fishing = String::new();
    fishing.push_str("pub const WHIRLPOOL_IDS: &[i32] = ");
    write_slice(&mut fishing, &fishing_facts.whirlpool_ids, |output, id| {
        write_number(output, id);
    });
    fishing.push_str(";\npub const FISHING_METHODS: &[FishingMethod] = &[\n");
    for method in &fishing_facts.methods {
        fishing.push_str("    FishingMethod { name: ");
        write_string(&mut fishing, &method.name);
        fishing.push_str(", op: ");
        write_string(&mut fishing, &method.op);
        fishing.push_str(", pair: ");
        write_string(&mut fishing, &method.pair);
        fishing.push_str(", gear: ");
        write_slice(&mut fishing, &method.gear, |output, piece| {
            output.push_str("GearPiece { name: ");
            write_string(output, &piece.name);
            write!(
                output,
                ", min: {}, restock: {} }}",
                piece.min, piece.restock
            )
            .expect("formatting into String cannot fail");
        });
        fishing.push_str(" },\n");
    }
    fishing.push_str("];\npub const FISHING_SHOP_COSTS: &[(&str, i32)] = ");
    write_slice(
        &mut fishing,
        &fishing_facts.shop_costs,
        |output, (name, cost)| {
            output.push('(');
            write_string(output, name);
            write!(output, ", {cost})").expect("formatting into String cannot fail");
        },
    );
    fishing.push_str(";\npub const GERRANT_ONLY_FISHING: &[&str] = ");
    write_slice(&mut fishing, &fishing_facts.gerrant_only, |output, name| {
        write_string(output, name);
    });
    fishing.push_str(";\npub const GERRANT_VENDOR: FishingVendor = ");
    write_fishing_vendor(&mut fishing, &fishing_facts.gerrant);
    fishing.push_str(";\npub const HARRY_VENDOR: FishingVendor = ");
    write_fishing_vendor(&mut fishing, &fishing_facts.harry);
    fishing.push_str(";\n");

    let mut cook = String::from("pub const FISH_CAMP_COOK_SURFACES: &[FishCookSurface] = &[\n");
    for surface in &facts.cook.surfaces {
        write!(
            cook,
            "    FishCookSurface {{ approach_first: {}, approach: ",
            surface.approach_first
        )
        .expect("formatting into String cannot fail");
        write_option(&mut cook, surface.approach.as_ref(), write_tile);
        cook.push_str(", stand: ");
        write_tile(&mut cook, &surface.stand);
        cook.push_str(", loc: ");
        write_option(&mut cook, surface.loc.as_ref(), write_tile);
        cook.push_str(", loc_name: ");
        write_string(&mut cook, &surface.loc_name);
        cook.push_str(", kind: FishCookKind::");
        cook.push_str(match surface.kind {
            FishKindFact::Range => "Range",
            FishKindFact::Fire => "Fire",
            FishKindFact::Fireplace => "Fireplace",
        });
        cook.push_str(", label: ");
        write_option(&mut cook, surface.label.as_deref(), write_string);
        cook.push_str(", notes: ");
        write_option(&mut cook, surface.notes.as_deref(), write_string);
        cook.push_str(" },\n");
    }
    cook.push_str("];\npub const FISH_CAMP_COOK_PLANS: &[FishCampCookPlan] = &[\n");
    for plan in &facts.cook.plans {
        cook.push_str("    FishCampCookPlan { camp: ");
        write_string(&mut cook, &plan.camp);
        cook.push_str(", pier: ");
        write_option(&mut cook, plan.pier, write_number);
        cook.push_str(", bank: ");
        write_option(&mut cook, plan.bank, write_number);
        cook.push_str(" },\n");
    }
    cook.push_str("];\n");

    let mut quest_junk = String::from("pub const QUEST_JUNK: &[QuestJunkEntry] = &[\n");
    for row in &facts.quest_junk {
        write!(quest_junk, "    QuestJunkEntry {{ id: {}, name: ", row.id)
            .expect("formatting into String cannot fail");
        write_string(&mut quest_junk, &row.name);
        quest_junk.push_str(", quest: ");
        write_string(&mut quest_junk, &row.quest);
        quest_junk.push_str(" },\n");
    }
    quest_junk.push_str("];\n");

    Ok(GatherCatalogRust {
        camps,
        fishing,
        cook,
        quest_junk,
    })
}
