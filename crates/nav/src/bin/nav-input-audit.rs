//! Offline audit of the cache/nav/content inputs used by one selected world.

use std::collections::BTreeMap;
use std::path::Path;

use client::config::if_type::{IfType, IfTypeMut};
use client::config::Cache;
use client::io::JagFile;
use nav::manifest::{nav_manifest_path, CacheManifest, NavManifest};
use nav::world::NavWorld;
use serde_json::{json, Value};

const CONTROLS_ROOT: usize = 147;
const BANK_ROOT: usize = 5292;
const MIME_ROOT: usize = 6543;
const MIME_BUTTONS: [usize; 8] = [6546, 6547, 6548, 6549, 6550, 6551, 6552, 6553];
const LAMP_ROOT: usize = 2808;
const LAMP_FIRST: usize = 2812;
const LAMP_CONFIRM: usize = 2831;
const CUBE_ROOT: usize = 6554;
const CUBE_MODELS: [usize; 3] = [6555, 6557, 6559];
const CUBE_QUESTION: usize = 6561;
const CUBE_BUTTONS: [usize; 3] = [6562, 6563, 6564];
const TRADE_SIDE_INV: usize = 3322;
const TRADE_MAIN: usize = 3323;
const TRADE_MAIN_INV: usize = 3415;
const TRADE_OTHER_INV: usize = 3416;
const TRADE_OTHER_PLAYER: usize = 3417;
const TRADE_CONFIRM: usize = 3443;
const SHOP_ROOT: usize = 3824;
const SHOP_INV: usize = 3900;

fn main() {
    if let Err(error) = run() {
        eprintln!("nav-input-audit: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err(
            "usage: nav-input-audit 274|289 CACHE_DIR CACHE_MANIFEST NAV_PACK CONTENT_DIR".into(),
        );
    }
    let revision = args[0]
        .parse::<u16>()
        .map_err(|_| "revision must be 274 or 289".to_string())?;
    let cache_dir = Path::new(&args[1]);
    let cache_manifest_path = Path::new(&args[2]);
    let nav_path = Path::new(&args[3]);
    let content_dir = Path::new(&args[4]);

    let cache_manifest: CacheManifest = read_json(cache_manifest_path)?;
    cache_manifest.verify(revision, cache_dir)?;
    let client_revision = match revision {
        274 => client::io::ClientRevision::R274,
        289 => client::io::ClientRevision::R289,
        _ => return Err("revision must be 274 or 289".into()),
    };
    let game_data = api::game_data::for_profile(client_revision, &cache_manifest.identity())?;
    let nav_bytes =
        std::fs::read(nav_path).map_err(|e| format!("navigation {}: {e}", nav_path.display()))?;
    let flags_path = nav_path.with_extension("navflags");
    let flags_bytes = std::fs::read(&flags_path)
        .map_err(|e| format!("navigation flags {}: {e}", flags_path.display()))?;
    let nav_manifest: NavManifest = read_json(&nav_manifest_path(nav_path))?;
    nav_manifest.verify(revision, &cache_manifest, &nav_bytes, Some(&flags_bytes))?;

    let config_bytes =
        std::fs::read(cache_dir.join("config")).map_err(|e| format!("cache config: {e}"))?;
    let interface_bytes =
        std::fs::read(cache_dir.join("interface")).map_err(|e| format!("cache interface: {e}"))?;
    let cache = Cache::unpack(&JagFile::new(config_bytes));
    let (ifaces, ifaces_mut) = IfType::unpack(&JagFile::new(interface_bytes));
    let world = NavWorld::load_pack(nav_path).map_err(|e| format!("navigation decode: {e}"))?;

    let bank_inventory = find_inventory(&ifaces, BANK_ROOT, "withdraw");
    let controls = component(&ifaces, &ifaces_mut, CONTROLS_ROOT);
    let controls_children = ifaces
        .get(CONTROLS_ROOT)
        .and_then(Option::as_deref)
        .and_then(|root| root.children.as_ref())
        .cloned()
        .unwrap_or_default();
    let control_rows: Vec<Value> = controls_children
        .iter()
        .map(|id| component(&ifaces, &ifaces_mut, *id as usize))
        .collect();

    let guardian_npc_names = [
        "genie",
        "drunken dwarf",
        "mysterious old man",
        "sandwich lady",
        "frog",
        "strange plant",
        "river troll",
        "swarm",
        "rock golem",
        "zombie",
        "shade",
        "watchman",
        "tree spirit",
    ];
    let guardian_npcs: Vec<Value> = cache
        .npcs
        .iter()
        .filter(|npc| guardian_npc_names.contains(&npc.name.to_ascii_lowercase().as_str()))
        .map(|npc| json!({"id": npc.id, "name": npc.name, "ops": npc.op}))
        .collect();
    let guardian_locs: Vec<Value> = cache
        .locs
        .iter()
        .filter(|loc| {
            matches!(
                loc.name.to_ascii_lowercase().as_str(),
                "whirlpool" | "gas chest" | "smoking rock" | "strange plant"
            )
        })
        .map(|loc| json!({"id": loc.id, "name": loc.name, "ops": loc.op}))
        .collect();
    let food: Vec<Value> = game_data
        .fixed_food_heals()
        .map(|(name, heal)| {
            let ids: Vec<i32> = cache
                .objs
                .iter()
                .filter(|obj| obj.name.eq_ignore_ascii_case(name))
                .map(|obj| obj.id)
                .collect();
            json!({"name": name, "heal": heal, "cache_ids": ids})
        })
        .collect();

    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for edge in &world.graph.edges {
        *kinds.entry(format!("{:?}", edge.kind)).or_default() += 1;
    }
    let teleports: Vec<Value> = world
        .graph
        .teleports
        .iter()
        .map(|edge| {
            json!({
                "to": [edge.to.x, edge.to.z, edge.to.level],
                "loc_id": edge.loc_id,
                "option": edge.option,
                "ticks": edge.ticks,
                "skill_req": edge.skill_req,
                "item_req": edge.item_req,
                "quest_req": edge.quest_req,
                "varp_req": edge.varp_req,
                "worn_req": edge.worn_req,
            })
        })
        .collect();
    let banks: Vec<Value> = world
        .banks()
        .iter()
        .map(|bank| {
            json!({
                "name": bank.name,
                "tile": [bank.tile.x, bank.tile.z, bank.tile.level],
                "access": format!("{:?}", bank.access),
            })
        })
        .collect();

    let output = json!({
        "revision": revision,
        "cache_dir": cache_dir,
        "content_dir": content_dir,
        "cache_manifest": cache_manifest,
        "nav_manifest": nav_manifest,
        "cache_counts": {
            "objects": cache.objs.len(),
            "npcs": cache.npcs.len(),
            "locs": cache.locs.len(),
            "spotanims": cache.spots.len(),
            "interfaces": ifaces.iter().filter(|row| row.is_some()).count(),
        },
        "bank": {
            "root": component(&ifaces, &ifaces_mut, BANK_ROOT),
            "inventory": bank_inventory.map(|id| component(&ifaces, &ifaces_mut, id)),
            "capacity": bank_inventory.and_then(|id| ifaces.get(id).and_then(Option::as_deref)).map(|com| com.width * com.height),
            "baked_stands": banks,
        },
        "controls": {
            "root": controls,
            "children": control_rows,
            "snapshot_indices": {
                "retaliate_on": controls_children.get(2),
                "retaliate_off": controls_children.get(3),
                "run_off": controls_children.get(4),
                "run_on": controls_children.get(5),
            },
        },
        "selected_interfaces": {
            "trade_side_inventory": component(&ifaces, &ifaces_mut, TRADE_SIDE_INV),
            "trade_main": component(&ifaces, &ifaces_mut, TRADE_MAIN),
            "trade_main_inventory": component(&ifaces, &ifaces_mut, TRADE_MAIN_INV),
            "trade_other_inventory": component(&ifaces, &ifaces_mut, TRADE_OTHER_INV),
            "trade_other_player": component(&ifaces, &ifaces_mut, TRADE_OTHER_PLAYER),
            "trade_confirm": component(&ifaces, &ifaces_mut, TRADE_CONFIRM),
            "shop": component(&ifaces, &ifaces_mut, SHOP_ROOT),
            "shop_inventory": component(&ifaces, &ifaces_mut, SHOP_INV),
        },
        "content": {
            "food": food,
            "selected_objects": [object(&cache, 3062), object(&cache, 2528), object(&cache, 1113)],
            "stunned_thieving_spotanim": spot(&cache, 245),
            "maze_door_loc": location(&cache, 3628),
        },
        "guardian": {
            "npc_name_inputs": guardian_npcs,
            "loc_name_inputs": guardian_locs,
            "mime": interface_set(&ifaces, &ifaces_mut, MIME_ROOT, &MIME_BUTTONS),
            "lamp": interface_set(&ifaces, &ifaces_mut, LAMP_ROOT, &[LAMP_FIRST, LAMP_CONFIRM]),
            "box": interface_set(
                &ifaces,
                &ifaces_mut,
                CUBE_ROOT,
                &[CUBE_MODELS[0], CUBE_MODELS[1], CUBE_MODELS[2], CUBE_QUESTION, CUBE_BUTTONS[0], CUBE_BUTTONS[1], CUBE_BUTTONS[2]],
            ),
        },
        "navigation": {
            "format_magic": String::from_utf8_lossy(&nav_bytes[..4]),
            "format_version": nav_bytes[4],
            "origin": [world.collision.origin.x, world.collision.origin.z, world.collision.origin.level],
            "width": world.collision.width,
            "height": world.collision.height,
            "ordinary_edges": world.graph.edges.len(),
            "edge_kinds": kinds,
            "teleports": teleports,
        },
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn component(
    ifaces: &[Option<Box<IfType>>],
    mutable: &[Option<Box<IfTypeMut>>],
    id: usize,
) -> Value {
    match ifaces.get(id).and_then(Option::as_deref) {
        Some(com) => {
            let overlay = mutable.get(id).and_then(Option::as_deref);
            json!({
                "id": com.id,
                "layer_id": com.layer_id,
                "type": com.r#type,
                "width": com.width,
                "height": com.height,
                "children": com.children,
                "text": overlay.map(|row| row.text.as_str()).unwrap_or(""),
                "button_type": overlay.map(|row| row.button_type).unwrap_or(0),
                "button_text": com.button_text,
                "iop": com.iop,
            })
        }
        None => Value::Null,
    }
}

fn find_inventory(ifaces: &[Option<Box<IfType>>], root: usize, op: &str) -> Option<usize> {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let Some(com) = ifaces.get(id).and_then(Option::as_deref) else {
            continue;
        };
        if com.r#type == 2
            && com.iop.iter().flatten().any(|value| {
                value
                    .to_ascii_lowercase()
                    .contains(&op.to_ascii_lowercase())
            })
        {
            return Some(id);
        }
        pending.extend(
            com.children
                .iter()
                .flatten()
                .filter_map(|id| usize::try_from(*id).ok()),
        );
    }
    None
}

fn interface_set(
    ifaces: &[Option<Box<IfType>>],
    mutable: &[Option<Box<IfTypeMut>>],
    root: usize,
    selected: &[usize],
) -> Value {
    json!({
        "root": component(ifaces, mutable, root),
        "selected": selected.iter().map(|id| component(ifaces, mutable, *id)).collect::<Vec<_>>(),
    })
}

fn object(cache: &Cache, id: usize) -> Value {
    cache.objs.get(id).map_or(Value::Null, |obj| {
        json!({"id": obj.id, "name": obj.name, "cost": obj.cost, "ops": obj.op, "inventory_ops": obj.iop})
    })
}

fn spot(cache: &Cache, id: usize) -> Value {
    cache.spots.get(id).map_or(
        Value::Null,
        |spot| json!({"id": spot.id, "model": spot.model, "anim": spot.anim}),
    )
}

fn location(cache: &Cache, id: usize) -> Value {
    cache.locs.get(id).map_or(Value::Null, |loc| {
        json!({
            "id": loc.id,
            "name": loc.name,
            "ops": loc.op,
            "blockwalk": loc.blockwalk,
            "width": loc.width,
            "length": loc.length,
        })
    })
}
