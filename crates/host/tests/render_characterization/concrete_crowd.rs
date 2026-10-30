//! Operator-requested concrete equipment arithmetic, not a rendering proof.
use super::{decode_record_stream, InputLedger, R289_VERSION};
use client::config::Cache;
use client::dash3d::Model;
use client::io::JagFile;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const STATIC_FACES: usize = 572_941;
const CROWD: usize = 250;
const BINDING_BYTES: usize = 128 * 1024 * 1024;
const BODY_SLOTS: [usize; 7] = [8, 11, 4, 6, 9, 7, 10];
const EMPTY: u16 = u16::MAX;

#[derive(Clone)]
struct Equipment {
    id: usize,
    symbol: String,
    source: String,
    line: usize,
    slot: usize,
    mask: usize,
}

fn slot(name: &str) -> Option<usize> {
    [
        "hat",
        "back",
        "front",
        "righthand",
        "torso",
        "lefthand",
        "arms",
        "legs",
        "head",
        "hands",
        "feet",
        "jaw",
    ]
    .iter()
    .position(|&value| value == name)
}

fn obj_files(root: &Path, out: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(root)
        .expect("content directory")
        .map(|entry| entry.expect("content entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            obj_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "obj") {
            out.push(path);
        }
    }
}

fn equipment(content: &Path, ledger: &mut InputLedger) -> Vec<Equipment> {
    let pack = ledger
        .read("legal-kit:obj.pack", &content.join("pack/obj.pack"))
        .expect("object identities");
    let ids = std::str::from_utf8(&pack)
        .expect("obj.pack UTF-8")
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(id, name)| (name.to_owned(), id.parse::<usize>().expect("object id")))
        .collect::<BTreeMap<_, _>>();
    let mut files = Vec::new();
    obj_files(&content.join("scripts"), &mut files);
    let mut result = Vec::new();
    for path in files {
        let bytes = ledger
            .read(&format!("legal-kit:{}", path.display()), &path)
            .expect("equipment source");
        let text = std::str::from_utf8(&bytes).expect("equipment source UTF-8");
        let mut section: Option<(&str, usize, BTreeMap<&str, &str>)> = None;
        let mut finish = |section: Option<(&str, usize, BTreeMap<&str, &str>)>| {
            let Some((name, line, fields)) = section else {
                return;
            };
            let Some(&id) = ids.get(name) else {
                return;
            };
            let Some(&primary) = fields.get("wearpos") else {
                return;
            };
            let Some(primary_slot) = slot(primary) else {
                return;
            };
            if !matches!(fields.get("iop2"), Some(&"Wear" | &"Wield" | &"Equip")) {
                return;
            }
            let mut mask = 1usize << primary_slot;
            for field in ["wearpos2", "wearpos3"] {
                if let Some(&name) = fields.get(field) {
                    if let Some(extra) = slot(name) {
                        mask |= 1usize << extra;
                    }
                }
            }
            result.push(Equipment {
                id,
                symbol: name.to_owned(),
                source: path.display().to_string(),
                line,
                slot: primary_slot,
                mask,
            });
        };
        for (index, line) in text.lines().enumerate() {
            let line = line.split("//").next().unwrap_or("").trim();
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                finish(section.take());
                section = Some((name, index + 1, BTreeMap::new()));
            } else if let Some((key, value)) = line.split_once('=') {
                if let Some((_, _, fields)) = section.as_mut() {
                    fields.insert(key.trim(), value.trim());
                }
            }
        }
        finish(section);
    }
    result.sort_by_key(|item| item.id);
    assert!(
        result.windows(2).all(|pair| pair[0].id != pair[1].id),
        "duplicate concrete equipment definition"
    );
    result
}

#[derive(Clone, Copy)]
struct Choice {
    faces: usize,
    objects: [u16; 12],
}

fn body_kit(cache: &Cache, gender: usize, heaviest: bool) -> [Option<(usize, usize)>; 12] {
    let mut result: [Option<(usize, usize)>; 12] = [None; 12];
    for (id, idk) in cache.idks.iter().enumerate() {
        if id >= 256 || idk.disable || !(0..7).contains(&(idk.part - (gender as i32 * 7))) {
            continue;
        }
        let part = (idk.part as usize) - gender * 7;
        let Some(model) = idk.get_model_no_check() else {
            continue;
        };
        let faces = usize::try_from(model.num_faces).expect("body faces");
        let target = &mut result[BODY_SLOTS[part]];
        if target.is_none() || (heaviest && target.is_some_and(|(_, old)| faces > old)) {
            *target = Some((id, faces));
        }
    }
    result
}

fn measured_kit(
    cache: &Cache,
    gear: &[Equipment],
    selected: &Choice,
    body: &[Option<(usize, usize)>; 12],
    gender: usize,
    label: &str,
) -> Value {
    let mut mask = 0usize;
    let mut models = Vec::new();
    let mut parts = Vec::new();
    for (position, &id) in selected.objects.iter().enumerate() {
        if id == EMPTY {
            continue;
        }
        let item = gear
            .iter()
            .find(|item| item.id == usize::from(id))
            .expect("selected item metadata");
        assert_eq!(position, item.slot);
        assert_eq!(mask & item.mask, 0, "equipped slots conflict");
        mask |= item.mask;
        let obj = cache.obj(item.id);
        let model = obj
            .get_wear_model_no_check(gender as i32)
            .expect("concrete equipment model");
        let model_ids = if gender == 0 {
            [obj.manwear, obj.manwear2, obj.manwear3]
        } else {
            [obj.womanwear, obj.womanwear2, obj.womanwear3]
        };
        parts.push(json!({"kind":"object", "id":item.id,"symbol":item.symbol,"appearance_slot":position,"occupied_mask":item.mask,"source":item.source,"line":item.line,"source_model_ids":model_ids.into_iter().filter(|&id| id >= 0).collect::<Vec<_>>(),"faces":model.num_faces}));
        models.push(Some(model));
    }
    for (position, part) in body.iter().enumerate() {
        if mask & (1 << position) != 0 {
            continue;
        }
        let Some((id, _)) = part else {
            continue;
        };
        let model = cache
            .idk(*id)
            .get_model_no_check()
            .expect("selected actual body model");
        parts.push(json!({"kind":"idk","id":id,"appearance_slot":position,"source_model_ids":cache.idk(*id).model,"faces":model.num_faces}));
        models.push(Some(model));
    }
    let expected_faces = models
        .iter()
        .flatten()
        .map(|model| usize::try_from(model.num_faces).expect("part faces"))
        .sum::<usize>();
    let combined = Model::combine_for_anim(&models, models.len());
    assert_eq!(
        usize::try_from(combined.num_faces).unwrap(),
        expected_faces,
        "actual player merge face count"
    );
    let total_faces = STATIC_FACES + CROWD * expected_faces;
    let vertex_bytes = std::mem::size_of::<client::render::world::GpuVertex>();
    assert_eq!(
        vertex_bytes, 16,
        "the production vertex layout is lossless compact16"
    );
    let bytes = total_faces * 2 * 3 * vertex_bytes;
    let legacy_bytes = total_faces * 2 * 3 * 24;
    json!({
        "case": label,
        "gender": if gender == 0 { "male" } else { "female" },
        "parts": parts,
        "assembled_player_faces": combined.num_faces,
        "assembled_player_points": combined.num_points,
        "players": CROWD,
        "static_faces": STATIC_FACES,
        "total_source_faces": total_faces,
        "production_vertex_bytes": vertex_bytes,
        "clipped_vertex_bytes": bytes,
        "legacy24_clipped_vertex_bytes": legacy_bytes,
        "fits_128_mib": bytes <= BINDING_BYTES,
        "margin_to_128_mib_bytes": BINDING_BYTES as i64 - bytes as i64,
        "same_clipping_factor_bytes_per_face": 2 * 3 * vertex_bytes,
        "retained_cap_bytes": BINDING_BYTES,
        "maximum_old_plus_new_growth_peak_bytes": BINDING_BYTES + BINDING_BYTES / 2,
    })
}

#[test]
#[ignore = "operator concrete real-equipment census; requires fixed 289 snapshot and sibling engine content"]
fn legal_armoured_crowd_cases() {
    let mut inputs = InputLedger::default();
    let cache_dir = client::cache_dir();
    let unpack_dir = client::unpack_dir();
    let cache_text = cache_dir.to_str().expect("cache path");
    assert_eq!(
        client::unpack::selected_version(cache_text).unwrap(),
        R289_VERSION
    );
    client::unpack::load_snapshot(cache_text, unpack_dir.to_str().unwrap())
        .expect("actual fixed snapshot");
    let config = inputs
        .read("legal-kit:fixed-config", &cache_dir.join("config"))
        .unwrap();
    let cache = Cache::unpack(&JagFile::new(config));
    let engine = PathBuf::from(std::env::var_os("ENGINE_DIR").expect("ENGINE_DIR"));
    let content = engine
        .parent()
        .expect("engine sibling content")
        .join("content");
    let gear = equipment(&content, &mut inputs);
    let rune = [
        "rune_full_helm",
        "rune_platebody",
        "rune_platelegs",
        "rune_kiteshield",
        "rune_scimitar",
    ];
    let mut cases = Vec::new();
    for gender in 0..2 {
        let default_body = body_kit(&cache, gender, false);
        let heavy_body = body_kit(&cache, gender, true);
        let mut full_rune = Choice {
            faces: 0,
            objects: [EMPTY; 12],
        };
        for name in rune {
            let item = gear
                .iter()
                .find(|item| item.symbol == name)
                .expect("concrete rune equipment source");
            full_rune.objects[item.slot] = u16::try_from(item.id).unwrap();
        }
        cases.push(measured_kit(
            &cache,
            &gear,
            &full_rune,
            &default_body,
            gender,
            "full_rune_five_named_items_default_remaining_body",
        ));
        let mut choices = vec![None::<Choice>; 1 << 12];
        choices[0] = Some(Choice {
            faces: 0,
            objects: [EMPTY; 12],
        });
        for item in &gear {
            if item.id >= cache.objs.len() {
                continue;
            }
            let Some(model) = cache.obj(item.id).get_wear_model_no_check(gender as i32) else {
                continue;
            };
            let faces = usize::try_from(model.num_faces).unwrap();
            for mask in (0..choices.len()).rev() {
                if mask & item.mask != 0 {
                    continue;
                }
                let Some(mut candidate) = choices[mask] else {
                    continue;
                };
                let target = mask | item.mask;
                candidate.faces += faces;
                candidate.objects[item.slot] = u16::try_from(item.id).unwrap();
                if choices[target].is_none_or(|current| candidate.faces > current.faces) {
                    choices[target] = Some(candidate);
                }
            }
        }
        let (_, selected) = choices
            .into_iter()
            .enumerate()
            .filter_map(|(mask, value)| value.map(|choice| (mask, choice)))
            .max_by_key(|(mask, choice)| {
                choice.faces
                    + heavy_body
                        .iter()
                        .enumerate()
                        .filter(|(slot, _)| mask & (1 << slot) == 0)
                        .filter_map(|(_, part)| part.map(|(_, faces)| faces))
                        .sum::<usize>()
            })
            .expect("legal equipment candidate");
        cases.push(measured_kit(
            &cache,
            &gear,
            &selected,
            &heavy_body,
            gender,
            "heaviest_named_compatible_content_wear_models",
        ));
    }
    let model_bytes = inputs
        .read(
            "legal-kit:fixed-model-records",
            &unpack_dir.join(R289_VERSION).join("models.bin"),
        )
        .unwrap();
    let records = decode_record_stream(&model_bytes, "models.bin").unwrap();
    let mut faces = 0usize;
    let mut source_indices = 0usize;
    let mut unique_positions = 0usize;
    for record in &records {
        let model = Model::load(i32::try_from(record.id).unwrap()).expect("actual model record");
        faces += usize::try_from(model.num_faces).unwrap();
        source_indices += usize::try_from(model.num_points).unwrap();
        let points = model
            .point_x
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .copied()
            .zip(model.point_y.as_deref().unwrap_or(&[]).iter().copied())
            .zip(model.point_z.as_deref().unwrap_or(&[]).iter().copied())
            .map(|((x, y), z)| (x, y, z))
            .collect::<BTreeSet<_>>();
        unique_positions += points.len();
    }
    assert!(
        inputs.failures.is_empty(),
        "complete concrete equipment inputs"
    );
    println!(
        "legal_armoured_crowd_census={}",
        serde_json::to_string_pretty(&json!({
            "schema": "T4.4-concrete-armoured-crowd-v2",
            "fixed_cache_version": R289_VERSION,
            "content_equipment_candidates": gear.len(),
            "classification": "concrete compatible equip-slot compositions; source wear actions, all stats/quests assumed satisfied; no transmog, attached spot, or temporary loc; static co-location remains an arithmetic envelope, not visibility/reachability proof",
            "cases": cases,
            "base_model_index_census": {
                "model_records": records.len(),
                "source_faces": faces,
                "source_vertex_indices": source_indices,
                "unique_positions_summed_per_model": unique_positions,
                "source_indices_per_face": source_indices as f64 / faces as f64,
                "unique_positions_per_face": unique_positions as f64 / faces as f64,
                "qualification": "position-only source-model sharing; NOT a measured ratio of complete GPU vertices, which also depend on per-face alpha/shade/UV and triangle-neighbour shader metadata",
            },
            "inputs": inputs.hashes,
            "complete": true,
        }))
        .unwrap()
    );
}
