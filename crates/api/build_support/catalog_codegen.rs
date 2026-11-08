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
