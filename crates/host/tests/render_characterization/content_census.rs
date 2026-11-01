use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};

use client::config::Cache;
use client::dash3d::store::render_test_shared_store_layout;
use client::dash3d::{LocAngle, Model, LOC_SHAPE_TO_LAYER};
use client::graphics::PixFont;
use client::io::JagFile;
use client::map_cache::{self, MapIndexEntry};
use client::render::backend::gpu_atlas::GpuAssets;
use client::render::budget::RenderResource;
use client::render::nav_debug::{render_test_caption_ink_pixels, render_test_nav_layout};
use client::render::store::render_test_shared_index_layout;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "concrete_crowd.rs"]
mod concrete_crowd;
#[path = "legal_population.rs"]
mod legal_population;

const R289_VERSION: &str = "37214163f1e6ceca";
const CLIENT_JAGS: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

#[derive(Default)]
struct InputLedger {
    hashes: Vec<Value>,
    failures: Vec<Value>,
}

impl InputLedger {
    fn read(&mut self, label: &str, path: &Path) -> Option<Vec<u8>> {
        match fs::read(path) {
            Ok(bytes) => {
                let digest = Sha256::digest(&bytes);
                self.hashes.push(json!({
                    "input": label,
                    "path": path.display().to_string(),
                    "bytes": bytes.len(),
                    "sha256": format!("{digest:x}"),
                }));
                Some(bytes)
            }
            Err(error) => {
                self.failures.push(json!({
                    "code": "input_unavailable",
                    "input": label,
                    "path": path.display().to_string(),
                    "error": error.to_string(),
                }));
                None
            }
        }
    }

    fn fail(&mut self, code: &str, detail: impl Into<String>) {
        self.failures.push(json!({
            "code": code,
            "detail": detail.into(),
        }));
    }
}

#[derive(Clone, Copy)]
struct Record<'a> {
    id: u32,
    bytes: &'a [u8],
}

fn decode_record_stream<'a>(bytes: &'a [u8], label: &str) -> Result<Vec<Record<'a>>, String> {
    let mut records = Vec::new();
    let mut ids = BTreeSet::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let header_end = offset
            .checked_add(8)
            .ok_or_else(|| format!("{label}: record header offset overflow"))?;
        if header_end > bytes.len() {
            return Err(format!("{label}: truncated record header at {offset}"));
        }
        let id = u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| format!("{label}: invalid record id at {offset}"))?,
        );
        let len = usize::try_from(u32::from_le_bytes(
            bytes[offset + 4..header_end]
                .try_into()
                .map_err(|_| format!("{label}: invalid record length at {offset}"))?,
        ))
        .map_err(|_| format!("{label}: record length does not fit usize"))?;
        let end = header_end
            .checked_add(len)
            .ok_or_else(|| format!("{label}: record end overflow for id {id}"))?;
        if end > bytes.len() {
            return Err(format!("{label}: truncated record {id} at {offset}"));
        }
        if !ids.insert(id) {
            return Err(format!("{label}: duplicate record id {id}"));
        }
        records.push(Record {
            id,
            bytes: &bytes[header_end..end],
        });
        offset = end;
    }
    Ok(records)
}
#[derive(Clone, Copy)]
struct ModelCacheLayouts {
    loc: client::config::loc_type::RenderTestLocCacheLayout,
    npc: client::config::RenderTestCacheLayout,
    obj: client::config::RenderTestCacheLayout,
    spot: client::config::RenderTestCacheLayout,
    interface: client::config::RenderTestCacheLayout,
    player: client::dash3d::client_player::RenderTestPlayerLayout,
    idk: client::config::idk_type::RenderTestIdkLayout,
}

impl ModelCacheLayouts {
    fn load() -> Option<Self> {
        Some(Self {
            loc: client::config::loc_type::render_test_cache_layout()?,
            npc: client::config::npc_type::render_test_cache_layout()?,
            obj: client::config::obj_type::render_test_cache_layout()?,
            spot: client::config::spot_type::render_test_cache_layout()?,
            interface: client::config::if_type::render_test_cache_layout()?,
            player: client::dash3d::client_player::render_test_layout()?,
            idk: client::config::idk_type::render_test_layout(),
        })
    }

    fn allocation_bytes(self) -> Option<usize> {
        [
            self.loc.raw_cache.allocation_bytes,
            self.loc.transformed_cache.allocation_bytes,
            self.npc.allocation_bytes,
            self.obj.allocation_bytes,
            self.spot.allocation_bytes,
            self.interface.allocation_bytes,
            self.player.model_cache.allocation_bytes,
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
    }

    fn loc_sentinel_count(self) -> Option<usize> {
        self.loc
            .raw_cache
            .sentinel_count
            .checked_add(self.loc.transformed_cache.sentinel_count)
    }

    fn json(self) -> Value {
        let cache = |layout: client::config::RenderTestCacheLayout| {
            json!({
                "capacity": layout.capacity,
                "allocation_bytes": layout.allocation_bytes,
                "node_bytes": layout.node_bytes,
                "sentinel_count": layout.sentinel_count,
            })
        };
        json!({
            "loc_raw": cache(self.loc.raw_cache),
            "loc_transformed": cache(self.loc.transformed_cache),
            "npc": cache(self.npc),
            "obj": cache(self.obj),
            "spot": cache(self.spot),
            "interface": cache(self.interface),
            "player": {
                "cache": cache(self.player.model_cache),
                "appearance_slots": self.player.appearance_slots,
                "loc_model_box_bytes": self.player.loc_model_box_bytes,
            },
            "idk": {
                "type_bytes": self.idk.type_bytes,
                "model_ids_field_bytes": self.idk.model_ids_field_bytes,
                "head_slots": self.idk.head_slots,
                "model_value_bytes": self.idk.model_value_bytes,
            },
            "model_value_bytes": self.loc.model_value_bytes,
            "arc_header_bytes": self.loc.arc_header_bytes,
        })
    }
}
fn insert_graphic_key(keys: &mut BTreeSet<(String, i32)>, value: &str) {
    if let Some((name, index)) = value.rsplit_once(',') {
        if let Ok(index) = index.trim().parse::<i32>() {
            keys.insert((name.to_owned(), index));
        }
    }
}

fn interface_graphic_keys(bytes: &[u8]) -> Result<Vec<(String, i32)>, String> {
    let interface = JagFile::new(bytes.to_vec());
    let (components, mutable_components) = client::config::if_type::IfType::unpack(&interface);
    if components.is_empty() {
        return Err("fixed interface JAG did not decode any components".to_string());
    }
    let mut keys = BTreeSet::new();
    for (index, component) in components.iter().enumerate() {
        let Some(component) = component.as_deref() else {
            continue;
        };
        if component.r#type == client::config::if_type::ComponentType::TYPE_GRAPHIC {
            if let Some(mutable) = mutable_components.get(index).and_then(Option::as_deref) {
                insert_graphic_key(&mut keys, &mutable.graphic_name);
            }
            insert_graphic_key(&mut keys, &component.graphic2_name);
        }
        if let Some(backgrounds) = &component.inv_background_name {
            for name in backgrounds.iter().flatten() {
                insert_graphic_key(&mut keys, name);
            }
        }
    }
    Ok(keys.into_iter().collect())
}

#[derive(Default, Clone)]
struct FamilyMaximum {
    variants: usize,
    total_bytes: usize,
    maximum_bytes: usize,
    trigger: Option<String>,
    maximum_points: usize,
    maximum_faces: usize,
    retained_sizes: Vec<usize>,
}

impl FamilyMaximum {
    fn record(&mut self, model: &Model, trigger: String) -> Result<(), String> {
        let bytes = model.retained_bytes();
        if bytes == usize::MAX {
            return Err(format!("{trigger}: model retained-byte formula overflowed"));
        }
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes)
            .ok_or_else(|| format!("{trigger}: family byte sum overflowed"))?;
        self.variants = self
            .variants
            .checked_add(1)
            .ok_or_else(|| format!("{trigger}: variant count overflowed"))?;
        self.retained_sizes.push(bytes);
        self.maximum_points = self
            .maximum_points
            .max(usize::try_from(model.num_points).map_err(|_| {
                format!(
                    "{trigger}: negative or unrepresentable point count {}",
                    model.num_points
                )
            })?);
        self.maximum_faces = self
            .maximum_faces
            .max(usize::try_from(model.num_faces).map_err(|_| {
                format!(
                    "{trigger}: negative or unrepresentable face count {}",
                    model.num_faces
                )
            })?);
        if bytes > self.maximum_bytes {
            self.maximum_bytes = bytes;
            self.trigger = Some(trigger);
        }
        Ok(())
    }

    fn largest_bytes(&self, capacity: usize) -> Option<usize> {
        let mut sizes = self.retained_sizes.clone();
        sizes.sort_unstable_by(|left, right| right.cmp(left));
        sizes
            .into_iter()
            .take(capacity)
            .try_fold(0usize, usize::checked_add)
    }

    fn json(&self) -> Value {
        json!({
            "variants": self.variants,
            "sum_retained_bytes": self.total_bytes,
            "max_retained_bytes": self.maximum_bytes,
            "largest_triggering_content_id": self.trigger,
            "max_points": self.maximum_points,
            "max_faces": self.maximum_faces,
        })
    }
}
#[derive(Clone, Copy, Default)]
struct ModelMeasure {
    bytes: usize,
    faces: usize,
    points: usize,
}

fn model_measure(model: &Model, trigger: &str) -> Result<ModelMeasure, String> {
    let bytes = model.retained_bytes();
    if bytes == usize::MAX {
        return Err(format!("{trigger}: model retained-byte formula overflowed"));
    }
    let faces = usize::try_from(model.num_faces)
        .map_err(|_| format!("{trigger}: negative face count {}", model.num_faces))?;
    let points = usize::try_from(model.num_points)
        .map_err(|_| format!("{trigger}: negative point count {}", model.num_points))?;
    Ok(ModelMeasure {
        bytes,
        faces,
        points,
    })
}
const PLAYER_APPEARANCE_SLOT_COUNT: usize = 12;
const PLAYER_IDK_BODY_SLOT_BY_PART: [usize; 7] = [8, 11, 4, 6, 9, 7, 10];
const PLAYER_IDK_BODY_PART_COUNT: usize = 7;
const ILLUSTRATIVE_CROWD_PLAYER_COUNT: usize = 250;

#[derive(Clone, Default)]
struct FaceMaximum {
    faces: usize,
    trigger: Option<String>,
}

impl FaceMaximum {
    fn record_model(&mut self, model: &Model, trigger: String) -> Result<(), String> {
        let faces = usize::try_from(model.num_faces).map_err(|_| {
            format!(
                "{trigger}: negative or unrepresentable face count {}",
                model.num_faces
            )
        })?;
        self.record_faces(faces, trigger);
        Ok(())
    }

    fn record_faces(&mut self, faces: usize, trigger: String) {
        let replace = match self.trigger.as_deref() {
            None => true,
            Some(current) => {
                faces > self.faces || (faces == self.faces && trigger.as_str() < current)
            }
        };
        if replace {
            self.faces = faces;
            self.trigger = Some(trigger);
        }
    }

    fn merge(&mut self, other: &Self) {
        if let Some(trigger) = &other.trigger {
            self.record_faces(other.faces, trigger.clone());
        }
    }

    fn json(&self) -> Value {
        json!({
            "max_faces": self.faces,
            "trigger": self.trigger,
        })
    }
}

fn empty_player_slot_maxima() -> [FaceMaximum; PLAYER_APPEARANCE_SLOT_COUNT] {
    std::array::from_fn(|_| FaceMaximum::default())
}

fn checked_face_sum(faces: &[usize], label: &str) -> Result<usize, String> {
    faces.iter().try_fold(0usize, |sum, &value| {
        sum.checked_add(value)
            .ok_or_else(|| format!("{label}: face upper overflowed"))
    })
}

fn checked_slot_face_sum(
    slots: &[FaceMaximum; PLAYER_APPEARANCE_SLOT_COUNT],
    label: &str,
) -> Result<usize, String> {
    slots
        .iter()
        .enumerate()
        .try_fold(0usize, |sum, (slot, maximum)| {
            sum.checked_add(maximum.faces)
                .ok_or_else(|| format!("{label}: slot-face sum overflowed at slot {slot}"))
        })
}

fn transmog_branch_max(appearance_faces: usize, npc_faces: usize) -> (usize, &'static str) {
    if npc_faces > appearance_faces {
        (npc_faces, "npc_transmog")
    } else {
        (appearance_faces, "appearance_slots")
    }
}

struct PlayerGenderFaceUpper {
    slot_maxima: [FaceMaximum; PLAYER_APPEARANCE_SLOT_COUNT],
    appearance_slot_faces: usize,
    transmog_base_faces: usize,
    transmog_base_branch: &'static str,
    face_upper: usize,
}

impl PlayerGenderFaceUpper {
    fn new(
        slot_maxima: [FaceMaximum; PLAYER_APPEARANCE_SLOT_COUNT],
        npc_faces: &FaceMaximum,
        spot_faces: &FaceMaximum,
        loc_faces: &FaceMaximum,
        gender: &str,
    ) -> Result<Self, String> {
        let appearance_slot_faces = checked_slot_face_sum(&slot_maxima, gender)?;
        let (transmog_base_faces, transmog_base_branch) =
            transmog_branch_max(appearance_slot_faces, npc_faces.faces);
        let face_upper = checked_face_sum(
            &[transmog_base_faces, spot_faces.faces, loc_faces.faces],
            &format!("{gender} player face upper"),
        )?;
        Ok(Self {
            slot_maxima,
            appearance_slot_faces,
            transmog_base_faces,
            transmog_base_branch,
            face_upper,
        })
    }

    fn json(&self) -> Value {
        json!({
            "appearance_slot_face_upper": self.appearance_slot_faces,
            "slot_maxima": self.slot_maxima.iter().enumerate().map(|(slot, maximum)| json!({
                "appearance_slot": slot,
                "max_faces": maximum.faces,
                "trigger": maximum.trigger,
            })).collect::<Vec<_>>(),
            "transmog_base_face_upper": self.transmog_base_faces,
            "transmog_base_branch": self.transmog_base_branch,
            "face_upper": self.face_upper,
        })
    }
}

fn loc_declares_model_for_shape(loc: &client::config::LocType, shape: i32) -> bool {
    let Some(models) = loc.model.as_deref() else {
        return false;
    };
    if let Some(shapes) = loc.shape.as_deref() {
        let Some(index) = shapes.iter().position(|&candidate| candidate == shape) else {
            return false;
        };
        return models.get(index).is_some_and(|&model_id| model_id != -1);
    }
    shape == 10 && !models.is_empty()
}

fn configured_model_ids_load(model_ids: &[i32]) -> bool {
    model_ids
        .iter()
        .all(|&model_id| model_id >= 0 && Model::load(model_id).is_some())
}

fn resolve_sequence_replacement_faces(
    cache: &Cache,
    gender: usize,
    appearance_value: i32,
    idk_face_counts: &[Option<usize>],
    idk_model_lists_load: &[bool],
    object_wear_face_counts: &[Vec<Option<usize>>; 2],
    object_wear_model_lists_load: &[Vec<bool>; 2],
) -> Result<(usize, &'static str, Option<usize>), String> {
    if appearance_value < 256 {
        return Ok((0, "empty", None));
    }
    if appearance_value < 512 {
        let id = usize::try_from(appearance_value - 256)
            .map_err(|_| format!("invalid IDK appearance value {appearance_value}"))?;
        let Some(idk) = cache.idks.get(id) else {
            return Err(format!(
                "sequence IDK appearance value {appearance_value} is out of range"
            ));
        };
        if idk.model.is_none() {
            return Ok((0, "idk_without_body_model", Some(id)));
        }
        if !idk_model_lists_load.get(id).copied().unwrap_or(false) {
            return Err(format!("sequence IDK {id} body model list is incomplete"));
        }
        let faces = idk_face_counts
            .get(id)
            .copied()
            .flatten()
            .ok_or_else(|| format!("sequence IDK {id} body composite is unavailable"))?;
        return Ok((faces, "idk_body", Some(id)));
    }

    let id = usize::try_from(appearance_value - 512)
        .map_err(|_| format!("invalid object appearance value {appearance_value}"))?;
    let Some(obj) = cache.objs.get(id) else {
        return Err(format!(
            "sequence object appearance value {appearance_value} is out of range"
        ));
    };
    let primary_wear_model = if gender == 0 {
        obj.manwear
    } else {
        obj.womanwear
    };
    if primary_wear_model == -1 {
        return Ok((0, "object_without_wear_model", Some(id)));
    }
    if !object_wear_model_lists_load
        .get(gender)
        .and_then(|models| models.get(id))
        .copied()
        .unwrap_or(false)
    {
        return Err(format!(
            "sequence object {id} wear model is incomplete for gender {gender}"
        ));
    }
    let faces = object_wear_face_counts
        .get(gender)
        .and_then(|models| models.get(id))
        .copied()
        .flatten()
        .ok_or_else(|| {
            format!("sequence object {id} wear composite is unavailable for gender {gender}")
        })?;
    Ok((faces, "object_wear", Some(id)))
}

fn dynamic_face_uppers_json(
    player_slots: [[FaceMaximum; PLAYER_APPEARANCE_SLOT_COUNT]; 2],
    object_wear_maxima: [FaceMaximum; 2],
    npc_faces: &FaceMaximum,
    spot_faces: &FaceMaximum,
    loc_faces: &FaceMaximum,
    sequence_hand_replacements: Vec<Value>,
    male_body_kit_case: Value,
) -> Result<Value, String> {
    let male = PlayerGenderFaceUpper::new(
        player_slots[0].clone(),
        npc_faces,
        spot_faces,
        loc_faces,
        "male player",
    )?;
    let female = PlayerGenderFaceUpper::new(
        player_slots[1].clone(),
        npc_faces,
        spot_faces,
        loc_faces,
        "female player",
    )?;
    let (maximum_gender, player_face_upper) = if female.face_upper > male.face_upper {
        ("female", female.face_upper)
    } else {
        ("male", male.face_upper)
    };
    let npc_face_upper = checked_face_sum(
        &[npc_faces.faces, spot_faces.faces],
        "NPC plus attached spot face upper",
    )?;
    Ok(json!({
        "classification": "sound_unqualified_per_entity_source_face_upper",
        "face_scope": "composite Model.num_faces before raster clipping and per-face culling",
        "player": {
            "formula": "max(sum(per-slot maxima for appearance slots 0..11), largest NPC transmog composite) + largest spot model + largest temporary loc model",
            "face_upper": player_face_upper,
            "maximum_gender": maximum_gender,
            "male": male.json(),
            "female": female.json(),
            "illustrative_legal_case": male_body_kit_case,
            "global_object_wear_max_faces_by_gender": {
                "male": object_wear_maxima[0].json(),
                "female": object_wear_maxima[1].json(),
            },
            "largest_npc_transmog_candidate": npc_faces.json(),
            "largest_attached_spot_candidate": spot_faces.json(),
            "largest_temporary_loc_candidate": loc_faces.json(),
            "sequence_hand_replacements": sequence_hand_replacements,
            "object_wear_slot_scope": "The largest loaded object WEAR composite for each gender is admitted in every appearance slot as a sound overestimate because the client ObjType table does not retain wearpos/wearpos2/wearpos3 slot metadata.",
        },
        "npc": {
            "formula": "largest NPC composite + largest attached spot model",
            "npc_face_upper": npc_face_upper,
            "largest_npc_composite": npc_faces.json(),
            "largest_attached_spot": spot_faces.json(),
        },
        "projectile": {
            "formula": "largest spot model",
            "projectile_face_upper": spot_faces.faces,
            "largest_spot": spot_faces.json(),
        },
        "map_spot": {
            "formula": "largest spot model",
            "map_spot_face_upper": spot_faces.faces,
            "largest_spot": spot_faces.json(),
        },
        "source_citations": [
            "engine/src/engine/entity/Player.ts:1322-1345 maps the seven gendered body-part values to appearance slots 8, 11, 4, 6, 9, 7, and 10; lines 1370-1413 emit equipped objects as 0x200 + id and use wearpos2/wearpos3 to suppress occupied slots.",
            "vendor/fr-client-rust/crates/client/src/client/client.rs:7159-7173 selects the first non-disabled IDK for each of seven creation-screen part positions; vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs:565-581 decodes appearance values 256..511 as IDK IDs 0..255; vendor/fr-client-rust/crates/client/src/config/idk_type.rs:26-33 and 104-126 define the part/model/disable fields and compose source models.",
            "vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs:494-524 replaces right-hand slot 3 and left-hand slot 5 from SeqType; the same file's lines 557-585 select IDK body composites and ObjType WEAR composites by appearance value.",
            "vendor/fr-client-rust/crates/client/src/config/idk_type.rs:104-126 composes the IDK body model list; vendor/fr-client-rust/crates/client/src/config/obj_type.rs:848-894 composes the gender-specific object WEAR model list.",
            "vendor/fr-client-rust/crates/client/src/config/npc_type.rs:183-267 builds the NPC composite and applies vertex animation; vendor/fr-client-rust/crates/client/src/config/spot_type.rs:96-122 resolves the spot model.",
            "vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs:272-301 and 310-330 append the attached spot and temporary loc; vendor/fr-client-rust/crates/client/src/dash3d/model.rs:1222-1347 and 1351-1477 sum source num_faces and copy each source face once when combining.",
            "vendor/fr-client-rust/crates/client/src/dash3d/model.rs:1507-1538 copies num_faces unchanged; the same file's lines 1889-1921 animate model labels/vertices without changing num_faces, as used by vendor/fr-client-rust/crates/client/src/config/npc_type.rs:183-267.",
            "vendor/fr-client-rust/crates/client/src/dash3d/model.rs:2200-2265 rotate, translate, recolour, mirror, and resize coordinates or face attributes without changing num_faces.",
            "vendor/fr-client-rust/crates/client/src/config/loc_type.rs:451-482 applies hill-skew by copying and adjusting vertex coordinates; vendor/fr-client-rust/crates/client/src/dash3d/model.rs:1541-1548 retains the source num_faces.",
            "vendor/fr-client-rust/crates/client/src/config/loc_type.rs:498-520 returns no model for a matched shape with no paired model ID or the -1 sentinel.",
        ],
    }))
}

fn male_idk_body_kit_case(
    cache: &Cache,
    selected_parts: [Option<(usize, usize)>; PLAYER_IDK_BODY_PART_COUNT],
) -> Result<Value, String> {
    let mut face_counts = Vec::with_capacity(PLAYER_IDK_BODY_PART_COUNT);
    let mut body_kit_parts = Vec::with_capacity(PLAYER_IDK_BODY_PART_COUNT);
    for (body_part, selected) in selected_parts.into_iter().enumerate() {
        let appearance_slot = PLAYER_IDK_BODY_SLOT_BY_PART[body_part];
        if let Some((id, faces)) = selected {
            let idk = cache
                .idks
                .get(id)
                .ok_or_else(|| format!("selected male body IDK {id} is missing"))?;
            let appearance_value = 256usize
                .checked_add(id)
                .filter(|&value| value < 512)
                .ok_or_else(|| format!("male body IDK {id} cannot be encoded"))?;
            body_kit_parts.push(json!({
                "body_part_index": body_part,
                "appearance_slot": appearance_slot,
                "appearance_value": appearance_value,
                "idk_id": id,
                "idk_source_model_ids": idk.model.as_deref().unwrap_or(&[]),
                "faces": faces,
                "trigger": format!("idk:{id}:body:part:{}:gender:0", body_part),
            }));
            face_counts.push(faces);
        } else {
            body_kit_parts.push(json!({
                "body_part_index": body_part,
                "appearance_slot": appearance_slot,
                "appearance_value": 0,
                "idk_id": Value::Null,
                "idk_source_model_ids": [],
                "faces": 0,
                "trigger": Value::Null,
            }));
            face_counts.push(0);
        }
    }
    let per_entity_face_sum = checked_face_sum(&face_counts, "male legal IDK body kit")?;
    let crowd_face_sum = per_entity_face_sum
        .checked_mul(ILLUSTRATIVE_CROWD_PLAYER_COUNT)
        .ok_or_else(|| "illustrative 250-player IDK crowd face sum overflow".to_string())?;
    Ok(json!({
        "case_name": "male_first_non_disabled_creation_screen_idk_kit_no_equipment",
        "gender": "male",
        "description": "One legal male creation-screen body kit: first non-disabled appearance-encodable IDK for each of seven part indices; no equipped objects, spot, or temporary loc.",
        "per_entity_appearance_face_sum": per_entity_face_sum,
        "illustrative_250_player_crowd": {
            "player_count": ILLUSTRATIVE_CROWD_PLAYER_COUNT,
            "composition_case": "male_first_non_disabled_creation_screen_idk_kit_no_equipment",
            "per_player_appearance_face_sum": per_entity_face_sum,
            "aggregate_appearance_face_sum": crowd_face_sum,
            "footprint_scope": "appearance body kit only; no equipped objects, attached spot, or temporary loc",
            "is_reachability_claim": false,
        },
        "equipment": [],
        "body_kit_parts": body_kit_parts,
    }))
}

fn placement_model_measure(
    cache: &Cache,
    placement: &EntityPlacement,
) -> Result<Option<ModelMeasure>, String> {
    let trigger = format!(
        "{}:{}:shape:{}:rotation:{}:amount:{}",
        placement.kind.label(),
        placement.id,
        placement.shape,
        placement.rotation,
        placement.amount
    );
    match placement.kind {
        EntityKind::Loc => {
            let Some(loc) = usize::try_from(placement.id)
                .ok()
                .and_then(|id| cache.locs.get(id))
            else {
                return Err(format!("{trigger}: location config is missing"));
            };
            loc.get_model_shared(
                cache,
                i32::from(placement.shape),
                i32::from(placement.rotation),
                0,
                0,
                0,
                0,
                -1,
            )
            .map(|model| model_measure(&model, &trigger))
            .transpose()
        }
        EntityKind::Npc => {
            let Some(npc) = usize::try_from(placement.id)
                .ok()
                .and_then(|id| cache.npcs.get(id))
            else {
                return Err(format!("{trigger}: NPC config is missing"));
            };
            npc.get_temp_model(cache, -1, -1, None)
                .map(|model| model_measure(&model, &trigger))
                .transpose()
        }
        EntityKind::Obj => {
            let count = i32::try_from(placement.amount)
                .map_err(|_| format!("{trigger}: object amount exceeds i32"))?;
            let Some(obj) = usize::try_from(placement.id)
                .ok()
                .and_then(|id| cache.objs.get(id))
            else {
                return Err(format!("{trigger}: object config is missing"));
            };
            obj.get_model_lit(cache, count)
                .map(|model| model_measure(&model, &trigger))
                .transpose()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntityKind {
    Loc,
    Npc,
    Obj,
}

impl EntityKind {
    fn label(self) -> &'static str {
        match self {
            Self::Loc => "loc",
            Self::Npc => "npc",
            Self::Obj => "obj",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EntityPlacement {
    kind: EntityKind,
    id: u32,
    plane: u8,
    world_x: u16,
    world_z: u16,
    shape: u8,
    rotation: u8,
    amount: u32,
    model_resolved: bool,
    model_bytes: usize,
    faces: usize,
    points: usize,
}

#[derive(Clone, Copy, Debug)]
struct TerrainCell {
    world_x: u16,
    world_z: u16,
    faces: u8,
}

#[derive(Clone, Copy, Default)]
struct StaticGeometryMaximum {
    origin_x: usize,
    origin_z: usize,
    containing_loaded_origin_x: usize,
    containing_loaded_origin_z: usize,
    terrain_cells: usize,
    terrain_faces: usize,
    terrain_unclipped_vertex_payload_bytes: usize,
    terrain_max_clipped_vertex_payload_bytes: usize,
    entity_rows: usize,
    resolved_model_rows: usize,
    unresolved_model_rows: usize,
    loc_rows: usize,
    npc_rows: usize,
    obj_rows: usize,
    entity_model_retained_bytes: usize,
    entity_faces: usize,
    entity_points: usize,
    entity_unclipped_vertex_payload_bytes: usize,
    entity_max_clipped_vertex_payload_bytes: usize,
    total_faces: usize,
    raw_unclipped_vertex_count: usize,
    raw_unclipped_vertex_payload_bytes: usize,
    max_two_triangle_clipped_vertex_count: usize,
    max_two_triangle_clipped_vertex_payload_bytes: usize,
    max_triangles_per_face: usize,
    vertex_bytes: usize,
}

impl StaticGeometryMaximum {
    fn json(self) -> Value {
        json!({
            "scope": "all source-derived static faces in the maximum co-located 50x50 RenderWorld tile-traversal footprint, before visibility and per-face culling",
            "submitted_tiles_per_axis": SUBMITTED_SCENE_TILES,
            "origin_world_tile": [self.origin_x, self.origin_z],
            "containing_loaded_scene_origin_world_tile": [
                self.containing_loaded_origin_x,
                self.containing_loaded_origin_z
            ],
            "terrain": {
                "nonempty_plane_cell_record_count": self.terrain_cells,
                "faces": self.terrain_faces,
                "vertex_bytes_per_vertex": self.vertex_bytes,
                "raw_unclipped_vertex_payload_bytes": self.terrain_unclipped_vertex_payload_bytes,
                "max_two_triangle_clipped_vertex_payload_bytes": self.terrain_max_clipped_vertex_payload_bytes
            },
            "resolved_map_entities": {
                "placement_rows": self.entity_rows,
                "resolved_model_rows": self.resolved_model_rows,
                "unresolved_or_nonrenderable_rows": self.unresolved_model_rows,
                "loc_rows": self.loc_rows,
                "npc_rows": self.npc_rows,
                "obj_rows": self.obj_rows,
                "model_retained_bytes_not_vertex_payload": self.entity_model_retained_bytes,
                "faces": self.entity_faces,
                "points": self.entity_points,
                "vertex_bytes_per_vertex": self.vertex_bytes,
                "raw_unclipped_vertex_payload_bytes": self.entity_unclipped_vertex_payload_bytes,
                "max_two_triangle_clipped_vertex_payload_bytes": self.entity_max_clipped_vertex_payload_bytes
            },
            "co_located_static_total": {
                "faces": self.total_faces,
                "raw_unclipped_triangle_count": self.total_faces,
                "raw_unclipped_vertex_count": self.raw_unclipped_vertex_count,
                "raw_unclipped_vertex_payload_bytes": self.raw_unclipped_vertex_payload_bytes,
                "max_triangles_per_face": self.max_triangles_per_face,
                "max_two_triangle_clipped_vertex_count": self.max_two_triangle_clipped_vertex_count,
                "max_two_triangle_clipped_vertex_payload_bytes": self.max_two_triangle_clipped_vertex_payload_bytes
            },
        })
    }
}

#[derive(Default)]
struct ParsedMap {
    map_cells: usize,
    explicit_map_cells: usize,
    entities: Vec<EntityPlacement>,
}

fn parse_map_source(
    bytes: &[u8],
    label: &str,
    square_x: u8,
    square_z: u8,
) -> Result<ParsedMap, String> {
    #[derive(Clone, Copy)]
    enum Section {
        Map,
        Loc,
        Npc,
        Obj,
    }

    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("{label}: map source is not UTF-8: {error}"))?;
    let mut section = None;
    let mut parsed = ParsedMap::default();
    let mut seen_map_cells = vec![false; 4 * 64 * 64];
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line
            .strip_prefix("==== ")
            .and_then(|s| s.strip_suffix(" ===="))
        {
            section = Some(match name {
                "MAP" => Section::Map,
                "LOC" => Section::Loc,
                "NPC" => Section::Npc,
                "OBJ" => Section::Obj,
                _ => return Err(format!("{label}:{line_number}: unknown section {name:?}")),
            });
            continue;
        }
        let Some(section) = section else {
            return Err(format!("{label}:{line_number}: row precedes a section"));
        };
        let (coordinate_text, value_text) = line
            .split_once(':')
            .ok_or_else(|| format!("{label}:{line_number}: missing coordinate separator"))?;
        let coordinates = coordinate_text
            .split_whitespace()
            .map(str::parse::<u8>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("{label}:{line_number}: invalid coordinates: {error}"))?;
        if coordinates.len() != 3
            || coordinates[0] >= 4
            || coordinates[1] >= 64
            || coordinates[2] >= 64
        {
            return Err(format!("{label}:{line_number}: coordinates out of bounds"));
        }
        match section {
            Section::Map => {
                let cell_index = usize::from(coordinates[0]) * 64 * 64
                    + usize::from(coordinates[1]) * 64
                    + usize::from(coordinates[2]);
                if std::mem::replace(&mut seen_map_cells[cell_index], true) {
                    return Err(format!("{label}:{line_number}: duplicate MAP cell"));
                }
                parsed.explicit_map_cells = parsed
                    .explicit_map_cells
                    .checked_add(1)
                    .ok_or_else(|| format!("{label}: map row count overflowed"))?;
            }
            Section::Loc => {
                let fields = value_text.split_whitespace().collect::<Vec<_>>();
                if !(2..=3).contains(&fields.len()) {
                    return Err(format!(
                        "{label}:{line_number}: LOC row must have id, shape, and optional rotation"
                    ));
                }
                let id = fields[0]
                    .parse::<u32>()
                    .map_err(|error| format!("{label}:{line_number}: invalid LOC id: {error}"))?;
                let shape = fields[1].parse::<u8>().map_err(|error| {
                    format!("{label}:{line_number}: invalid LOC shape: {error}")
                })?;
                let rotation = fields
                    .get(2)
                    .map(|value| value.parse::<u8>())
                    .transpose()
                    .map_err(|error| {
                        format!("{label}:{line_number}: invalid LOC rotation: {error}")
                    })?
                    .unwrap_or(0);
                if shape as usize >= LOC_SHAPE_TO_LAYER.len() || rotation >= 4 {
                    return Err(format!(
                        "{label}:{line_number}: LOC shape or rotation out of bounds"
                    ));
                }
                parsed.entities.push(EntityPlacement {
                    kind: EntityKind::Loc,
                    id,
                    plane: coordinates[0],
                    world_x: u16::from(square_x) * 64 + u16::from(coordinates[1]),
                    world_z: u16::from(square_z) * 64 + u16::from(coordinates[2]),
                    shape,
                    rotation,
                    amount: 1,
                    model_resolved: false,
                    model_bytes: 0,
                    faces: 0,
                    points: 0,
                });
            }
            Section::Npc | Section::Obj => {
                let fields = value_text.split_whitespace().collect::<Vec<_>>();
                let expected = if matches!(section, Section::Npc) {
                    1
                } else {
                    2
                };
                if fields.len() != expected {
                    return Err(format!(
                        "{label}:{line_number}: invalid {} row width",
                        if matches!(section, Section::Npc) {
                            "NPC"
                        } else {
                            "OBJ"
                        }
                    ));
                }
                let id = fields[0].parse::<u32>().map_err(|error| {
                    format!("{label}:{line_number}: invalid entity id: {error}")
                })?;
                let amount = fields
                    .get(1)
                    .map(|value| value.parse::<u32>())
                    .transpose()
                    .map_err(|error| {
                        format!("{label}:{line_number}: invalid object amount: {error}")
                    })?
                    .unwrap_or(1);
                if amount == 0 {
                    return Err(format!("{label}:{line_number}: zero-count entity row"));
                }
                parsed.entities.push(EntityPlacement {
                    kind: if matches!(section, Section::Npc) {
                        EntityKind::Npc
                    } else {
                        EntityKind::Obj
                    },
                    id,
                    plane: coordinates[0],
                    world_x: u16::from(square_x) * 64 + u16::from(coordinates[1]),
                    world_z: u16::from(square_z) * 64 + u16::from(coordinates[2]),
                    shape: 0,
                    rotation: 0,
                    amount,
                    model_resolved: false,
                    model_bytes: 0,
                    faces: 0,
                    points: 0,
                });
            }
        }
    }
    if section.is_none() {
        return Err(format!("{label}: no map sections"));
    }
    // Pack.js:309-340 initializes all 4*64*64 cells and emits opcode 0
    // for absent sparse MAP rows. They are real default terrain, not missing input.
    parsed.map_cells = seen_map_cells.len();
    Ok(parsed)
}

#[test]
fn sparse_map_source_includes_packed_default_terrain() {
    let parsed = parse_map_source(b"==== MAP ====\n0 1 2: u1\n", "m3_4", 3, 4).unwrap();
    assert_eq!(parsed.map_cells, 4 * 64 * 64);
    assert_eq!(parsed.explicit_map_cells, 1);
    assert!(parsed.entities.is_empty());
    let defaults = parse_map_source(b"==== MAP ====\n", "m3_4", 3, 4).unwrap();
    assert_eq!(defaults.map_cells, 4 * 64 * 64);
    assert_eq!(defaults.explicit_map_cells, 0);
    assert!(parse_map_source(b"==== MAP ====\n0 1 2: u1\n0 1 2: u2\n", "duplicate", 3, 4).is_err());
}

#[derive(Default)]
struct RangeAddMax {
    max: Vec<i128>,
    lazy: Vec<i128>,
    best_index: Vec<usize>,
    len: usize,
}

impl RangeAddMax {
    fn new(len: usize) -> Result<Self, String> {
        let slots = len
            .checked_mul(4)
            .ok_or_else(|| "scene-window tree size overflowed".to_string())?;
        let mut tree = Self {
            max: vec![0; slots],
            lazy: vec![0; slots],
            best_index: vec![0; slots],
            len,
        };
        if len == 0 {
            return Err("scene-window tree has no legal origin".to_string());
        }
        tree.build(1, 0, len - 1);
        Ok(tree)
    }

    fn build(&mut self, node: usize, low: usize, high: usize) {
        if low == high {
            self.best_index[node] = low;
            return;
        }
        let middle = low + (high - low) / 2;
        self.build(node * 2, low, middle);
        self.build(node * 2 + 1, middle + 1, high);
        self.pull(node);
    }

    fn pull(&mut self, node: usize) {
        let left = node * 2;
        let right = left + 1;
        if self.max[left] >= self.max[right] {
            self.max[node] = self.max[left];
            self.best_index[node] = self.best_index[left];
        } else {
            self.max[node] = self.max[right];
            self.best_index[node] = self.best_index[right];
        }
    }

    fn add(&mut self, start: usize, end: usize, delta: i128) -> Result<(), String> {
        if start > end || end >= self.len {
            return Err(format!("invalid scene-window update {start}..={end}"));
        }
        self.add_node(1, 0, self.len - 1, start, end, delta)
    }

    fn add_node(
        &mut self,
        node: usize,
        low: usize,
        high: usize,
        start: usize,
        end: usize,
        delta: i128,
    ) -> Result<(), String> {
        if start <= low && high <= end {
            self.max[node] = self.max[node]
                .checked_add(delta)
                .ok_or_else(|| "scene-window maximum overflowed".to_string())?;
            self.lazy[node] = self.lazy[node]
                .checked_add(delta)
                .ok_or_else(|| "scene-window lazy tag overflowed".to_string())?;
            return Ok(());
        }
        let middle = low + (high - low) / 2;
        if start <= middle {
            self.add_node(node * 2, low, middle, start, end.min(middle), delta)?;
        }
        if end > middle {
            self.add_node(
                node * 2 + 1,
                middle + 1,
                high,
                start.max(middle + 1),
                end,
                delta,
            )?;
        }
        let lazy = self.lazy[node];
        for child in [node * 2, node * 2 + 1] {
            self.max[child] = self.max[child]
                .checked_add(lazy)
                .ok_or_else(|| "scene-window propagated maximum overflowed".to_string())?;
            self.lazy[child] = self.lazy[child]
                .checked_add(lazy)
                .ok_or_else(|| "scene-window propagated lazy tag overflowed".to_string())?;
        }
        self.lazy[node] = 0;
        self.pull(node);
        Ok(())
    }

    fn maximum(&self) -> Result<(usize, usize), String> {
        let value = usize::try_from(self.max[1])
            .map_err(|_| "scene-window maximum is negative or does not fit usize".to_string())?;
        Ok((value, self.best_index[1]))
    }
}

#[derive(Default)]
struct SceneMaximum {
    count: usize,
    origin_x: usize,
    origin_z: usize,
    model_bytes: usize,
    model_origin_x: usize,
    model_origin_z: usize,
    faces: usize,
    face_origin_x: usize,
    face_origin_z: usize,
    points: usize,
    point_origin_x: usize,
    point_origin_z: usize,
    triggering_id: Option<String>,
}

fn scene_origin_count(
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
) -> Result<usize, String> {
    if scene_size == 0 || scene_size > world_size || origin_stride == 0 {
        return Err("scene dimensions or origin stride do not fit the world grid".to_string());
    }
    Ok((world_size - scene_size) / origin_stride + 1)
}

fn maximum_scene_metric(
    entities: &[EntityPlacement],
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
    kind: Option<EntityKind>,
    metric: impl Fn(&EntityPlacement) -> usize,
) -> Result<(usize, usize, usize), String> {
    let x_origin_count = scene_origin_count(scene_size, world_size, origin_stride)?;
    let z_origin_count = scene_origin_count(scene_size, world_size, origin_stride)?;
    let mut events = (0..x_origin_count)
        .map(|_| Vec::<(usize, usize, i128)>::new())
        .collect::<Vec<_>>();
    let max_origin = world_size - scene_size;
    for entity in entities
        .iter()
        .filter(|entity| kind.is_none_or(|expected| expected == entity.kind))
    {
        let x = usize::from(entity.world_x);
        let z = usize::from(entity.world_z);
        if x >= world_size || z >= world_size {
            return Err(format!(
                "entity {}:{} coordinate ({x},{z}) exceeds {world_size}x{world_size} world",
                entity.kind.label(),
                entity.id
            ));
        }
        let first_x = x.saturating_sub(scene_size - 1).div_ceil(origin_stride);
        let last_x = x.min(max_origin) / origin_stride;
        let first_z = z.saturating_sub(scene_size - 1).div_ceil(origin_stride);
        let last_z = z.min(max_origin) / origin_stride;
        if first_x > last_x || first_z > last_z {
            continue;
        }
        let delta = i128::try_from(metric(entity))
            .map_err(|_| "scene entity metric exceeds signed accumulator range".to_string())?;
        if delta == 0 {
            continue;
        }
        events[first_x].push((first_z, last_z, delta));
        if let Some(after_last_x) = last_x.checked_add(1).filter(|&next| next < x_origin_count) {
            events[after_last_x].push((first_z, last_z, -delta));
        }
    }

    let mut tree = RangeAddMax::new(z_origin_count)?;
    let mut best = (0usize, 0usize, 0usize);
    for (origin_x_index, updates) in events.into_iter().enumerate() {
        for (first_z, last_z, delta) in updates {
            tree.add(first_z, last_z, delta)?;
        }
        let (value, origin_z_index) = tree.maximum()?;
        if value > best.0 {
            best = (
                value,
                origin_x_index * origin_stride,
                origin_z_index * origin_stride,
            );
        }
    }
    Ok(best)
}

fn maximum_scene_entities(
    entities: &[EntityPlacement],
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
    kind: Option<EntityKind>,
) -> Result<SceneMaximum, String> {
    let (count, origin_x, origin_z) =
        maximum_scene_metric(entities, scene_size, world_size, origin_stride, kind, |_| 1)?;
    let (model_bytes, model_origin_x, model_origin_z) = maximum_scene_metric(
        entities,
        scene_size,
        world_size,
        origin_stride,
        kind,
        |entity| entity.model_bytes,
    )?;
    let (faces, face_origin_x, face_origin_z) = maximum_scene_metric(
        entities,
        scene_size,
        world_size,
        origin_stride,
        kind,
        |entity| entity.faces,
    )?;
    let (points, point_origin_x, point_origin_z) = maximum_scene_metric(
        entities,
        scene_size,
        world_size,
        origin_stride,
        kind,
        |entity| entity.points,
    )?;
    let scene_end_x = model_origin_x + scene_size;
    let scene_end_z = model_origin_z + scene_size;
    let triggering_id = entities
        .iter()
        .filter(|entity| kind.is_none_or(|expected| expected == entity.kind))
        .filter(|entity| {
            let x = usize::from(entity.world_x);
            let z = usize::from(entity.world_z);
            (model_origin_x..scene_end_x).contains(&x) && (model_origin_z..scene_end_z).contains(&z)
        })
        .max_by_key(|entity| (entity.model_bytes, entity.faces, entity.points))
        .map(|entity| {
            format!(
                "{}:{}@{},{},p{}s{}r{}n{}",
                entity.kind.label(),
                entity.id,
                entity.world_x,
                entity.world_z,
                entity.plane,
                entity.shape,
                entity.rotation,
                entity.amount
            )
        });
    Ok(SceneMaximum {
        count,
        origin_x,
        origin_z,
        model_bytes,
        model_origin_x,
        model_origin_z,
        faces,
        face_origin_x,
        face_origin_z,
        points,
        point_origin_x,
        point_origin_z,
        triggering_id,
    })
}

trait ScenePoint {
    fn world_x(&self) -> usize;
    fn world_z(&self) -> usize;
}

impl ScenePoint for TerrainCell {
    fn world_x(&self) -> usize {
        usize::from(self.world_x)
    }

    fn world_z(&self) -> usize {
        usize::from(self.world_z)
    }
}

impl ScenePoint for EntityPlacement {
    fn world_x(&self) -> usize {
        usize::from(self.world_x)
    }

    fn world_z(&self) -> usize {
        usize::from(self.world_z)
    }
}

fn validate_sorted_scene_points<T: ScenePoint>(
    points: &[T],
    world_size: usize,
    label: &str,
) -> Result<(), String> {
    let mut previous_x = None;
    for point in points {
        let x = point.world_x();
        let z = point.world_z();
        if x >= world_size || z >= world_size {
            return Err(format!(
                "{label} coordinate ({x},{z}) exceeds {world_size}x{world_size} world"
            ));
        }
        if previous_x.is_some_and(|previous| x < previous) {
            return Err(format!("{label} points are not sorted by world x"));
        }
        previous_x = Some(x);
    }
    Ok(())
}

fn scene_point_origin_range(
    tile: usize,
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
) -> Option<(usize, usize)> {
    let max_origin = world_size - scene_size;
    let first = tile.saturating_sub(scene_size - 1).div_ceil(origin_stride);
    let last = tile.min(max_origin) / origin_stride;
    (first <= last).then_some((first, last))
}

fn add_scene_point_to_tree(
    tree: &mut RangeAddMax,
    z: usize,
    metric: usize,
    add: bool,
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
) -> Result<(), String> {
    if metric == 0 {
        return Ok(());
    }
    let Some((first_z, last_z)) =
        scene_point_origin_range(z, scene_size, world_size, origin_stride)
    else {
        return Ok(());
    };
    let amount = i128::try_from(metric)
        .map_err(|_| "scene point metric exceeds signed accumulator range".to_string())?;
    tree.add(first_z, last_z, if add { amount } else { -amount })
}

#[expect(
    clippy::too_many_arguments,
    reason = "The sweep updates one caller-owned window using explicit grid and metric inputs"
)]
fn advance_scene_window<T: ScenePoint>(
    tree: &mut RangeAddMax,
    points: &[T],
    active_start: &mut usize,
    next_add: &mut usize,
    origin_x: usize,
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
    metric: impl Fn(&T) -> usize,
) -> Result<(), String> {
    while *active_start < *next_add {
        let point = &points[*active_start];
        if point.world_x() >= origin_x {
            break;
        }
        add_scene_point_to_tree(
            tree,
            point.world_z(),
            metric(point),
            false,
            scene_size,
            world_size,
            origin_stride,
        )?;
        *active_start += 1;
    }
    let window_end_x = origin_x + scene_size;
    while *next_add < points.len() {
        let point = &points[*next_add];
        if point.world_x() >= window_end_x {
            break;
        }
        if point.world_x() < origin_x {
            return Err("scene sweep skipped a point before the active window".to_string());
        }
        add_scene_point_to_tree(
            tree,
            point.world_z(),
            metric(point),
            true,
            scene_size,
            world_size,
            origin_stride,
        )?;
        *next_add += 1;
    }
    Ok(())
}

fn maximum_sorted_scene_metric<T: ScenePoint>(
    points: &[T],
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
    label: &str,
    metric: impl Fn(&T) -> usize,
) -> Result<(usize, usize, usize), String> {
    validate_sorted_scene_points(points, world_size, label)?;
    let x_origin_count = scene_origin_count(scene_size, world_size, origin_stride)?;
    let z_origin_count = scene_origin_count(scene_size, world_size, origin_stride)?;
    let mut tree = RangeAddMax::new(z_origin_count)?;
    let mut active_start = 0;
    let mut next_add = 0;
    let mut best = (0usize, 0usize, 0usize);
    for origin_x_index in 0..x_origin_count {
        let origin_x = origin_x_index * origin_stride;
        advance_scene_window(
            &mut tree,
            points,
            &mut active_start,
            &mut next_add,
            origin_x,
            scene_size,
            world_size,
            origin_stride,
            &metric,
        )?;
        let (value, origin_z_index) = tree.maximum()?;
        if value > best.0 {
            best = (value, origin_x, origin_z_index * origin_stride);
        }
    }
    Ok(best)
}

fn maximum_combined_scene_faces(
    terrain: &[TerrainCell],
    entities: &[EntityPlacement],
    scene_size: usize,
    world_size: usize,
    origin_stride: usize,
) -> Result<(usize, usize, usize), String> {
    validate_sorted_scene_points(terrain, world_size, "terrain")?;
    validate_sorted_scene_points(entities, world_size, "static entity")?;
    let x_origin_count = scene_origin_count(scene_size, world_size, origin_stride)?;
    let z_origin_count = scene_origin_count(scene_size, world_size, origin_stride)?;
    let mut tree = RangeAddMax::new(z_origin_count)?;
    let mut terrain_start = 0;
    let mut terrain_next = 0;
    let mut entity_start = 0;
    let mut entity_next = 0;
    let mut best = (0usize, 0usize, 0usize);
    for origin_x_index in 0..x_origin_count {
        let origin_x = origin_x_index * origin_stride;
        advance_scene_window(
            &mut tree,
            terrain,
            &mut terrain_start,
            &mut terrain_next,
            origin_x,
            scene_size,
            world_size,
            origin_stride,
            |cell| usize::from(cell.faces),
        )?;
        advance_scene_window(
            &mut tree,
            entities,
            &mut entity_start,
            &mut entity_next,
            origin_x,
            scene_size,
            world_size,
            origin_stride,
            |entity| entity.faces,
        )?;
        let (value, origin_z_index) = tree.maximum()?;
        if value > best.0 {
            best = (value, origin_x, origin_z_index * origin_stride);
        }
    }
    Ok(best)
}

fn geometry_vertex_payload(
    faces: usize,
    triangles_per_face: usize,
    vertex_bytes: usize,
) -> Result<(usize, usize, usize), String> {
    let triangles = faces
        .checked_mul(triangles_per_face)
        .ok_or_else(|| "scene triangle count overflowed".to_string())?;
    let vertices = triangles
        .checked_mul(3)
        .ok_or_else(|| "scene vertex count overflowed".to_string())?;
    let bytes = vertices
        .checked_mul(vertex_bytes)
        .ok_or_else(|| "scene vertex payload overflowed".to_string())?;
    Ok((triangles, vertices, bytes))
}

#[expect(
    clippy::too_many_arguments,
    reason = "The summary needs the exact window coordinates and renderer element layout"
)]
fn static_geometry_at_window(
    terrain: &[TerrainCell],
    entities: &[EntityPlacement],
    origin_x: usize,
    origin_z: usize,
    scene_size: usize,
    world_size: usize,
    max_triangles_per_face: usize,
    vertex_bytes: usize,
) -> Result<StaticGeometryMaximum, String> {
    let end_x = origin_x
        .checked_add(scene_size)
        .ok_or_else(|| "static scene x extent overflowed".to_string())?;
    let end_z = origin_z
        .checked_add(scene_size)
        .ok_or_else(|| "static scene z extent overflowed".to_string())?;
    if end_x > world_size || end_z > world_size {
        return Err("static geometry window exceeds the world grid".to_string());
    }
    let mut maximum = StaticGeometryMaximum {
        origin_x,
        origin_z,
        max_triangles_per_face,
        vertex_bytes,
        ..StaticGeometryMaximum::default()
    };
    let terrain_start = terrain.partition_point(|cell| usize::from(cell.world_x) < origin_x);
    let terrain_end = terrain.partition_point(|cell| usize::from(cell.world_x) < end_x);
    for cell in &terrain[terrain_start..terrain_end] {
        let z = usize::from(cell.world_z);
        if (origin_z..end_z).contains(&z) {
            maximum.terrain_cells = maximum
                .terrain_cells
                .checked_add(1)
                .ok_or_else(|| "static scene terrain cell count overflowed".to_string())?;
            maximum.terrain_faces = maximum
                .terrain_faces
                .checked_add(usize::from(cell.faces))
                .ok_or_else(|| "static scene terrain face count overflowed".to_string())?;
        }
    }
    let entity_start = entities.partition_point(|entity| usize::from(entity.world_x) < origin_x);
    let entity_end = entities.partition_point(|entity| usize::from(entity.world_x) < end_x);
    for entity in &entities[entity_start..entity_end] {
        let z = usize::from(entity.world_z);
        if !(origin_z..end_z).contains(&z) {
            continue;
        }
        let resolved_rows = if entity.model_resolved {
            &mut maximum.resolved_model_rows
        } else {
            &mut maximum.unresolved_model_rows
        };
        *resolved_rows = (*resolved_rows)
            .checked_add(1)
            .ok_or_else(|| "static scene resolved model row count overflowed".to_string())?;
        maximum.entity_rows = maximum
            .entity_rows
            .checked_add(1)
            .ok_or_else(|| "static scene entity row count overflowed".to_string())?;
        let row_count = match entity.kind {
            EntityKind::Loc => &mut maximum.loc_rows,
            EntityKind::Npc => &mut maximum.npc_rows,
            EntityKind::Obj => &mut maximum.obj_rows,
        };
        *row_count = (*row_count)
            .checked_add(1)
            .ok_or_else(|| "static scene entity kind row count overflowed".to_string())?;
        maximum.entity_model_retained_bytes = maximum
            .entity_model_retained_bytes
            .checked_add(entity.model_bytes)
            .ok_or_else(|| "static scene model retained-byte count overflowed".to_string())?;
        maximum.entity_faces = maximum
            .entity_faces
            .checked_add(entity.faces)
            .ok_or_else(|| "static scene entity face count overflowed".to_string())?;
        maximum.entity_points = maximum
            .entity_points
            .checked_add(entity.points)
            .ok_or_else(|| "static scene entity point count overflowed".to_string())?;
    }
    maximum.total_faces = maximum
        .terrain_faces
        .checked_add(maximum.entity_faces)
        .ok_or_else(|| "static scene total face count overflowed".to_string())?;
    let (_, _, terrain_unclipped_payload) =
        geometry_vertex_payload(maximum.terrain_faces, 1, vertex_bytes)?;
    maximum.terrain_unclipped_vertex_payload_bytes = terrain_unclipped_payload;
    let (_, _, terrain_clipped_payload) =
        geometry_vertex_payload(maximum.terrain_faces, max_triangles_per_face, vertex_bytes)?;
    maximum.terrain_max_clipped_vertex_payload_bytes = terrain_clipped_payload;
    let (_, _, entity_unclipped_payload) =
        geometry_vertex_payload(maximum.entity_faces, 1, vertex_bytes)?;
    maximum.entity_unclipped_vertex_payload_bytes = entity_unclipped_payload;
    let (_, _, entity_clipped_payload) =
        geometry_vertex_payload(maximum.entity_faces, max_triangles_per_face, vertex_bytes)?;
    maximum.entity_max_clipped_vertex_payload_bytes = entity_clipped_payload;
    let (_, raw_vertices, raw_payload) =
        geometry_vertex_payload(maximum.total_faces, 1, vertex_bytes)?;
    maximum.raw_unclipped_vertex_count = raw_vertices;
    maximum.raw_unclipped_vertex_payload_bytes = raw_payload;
    let (_, clipped_vertices, clipped_payload) =
        geometry_vertex_payload(maximum.total_faces, max_triangles_per_face, vertex_bytes)?;
    maximum.max_two_triangle_clipped_vertex_count = clipped_vertices;
    maximum.max_two_triangle_clipped_vertex_payload_bytes = clipped_payload;
    Ok(maximum)
}

fn maximum_submitted_static_geometry(
    terrain: &[TerrainCell],
    entities: &mut [EntityPlacement],
    world_size: usize,
    loaded_scene_size: usize,
    submitted_scene_size: usize,
    max_triangles_per_face: usize,
    vertex_bytes: usize,
) -> Result<StaticGeometryMaximum, String> {
    if max_triangles_per_face != 2 {
        return Err(format!(
            "expected the renderer's two-triangle clipped-face bound, found {max_triangles_per_face}"
        ));
    }
    scene_origin_count(loaded_scene_size, world_size, MAP_LOAD_ORIGIN_STRIDE)?;
    if submitted_scene_size > loaded_scene_size {
        return Err("submitted geometry window exceeds the loaded scene".to_string());
    }
    entities.sort_unstable_by_key(|entity| (entity.world_x, entity.world_z));
    let (maximum_faces, origin_x, origin_z) =
        maximum_combined_scene_faces(terrain, entities, submitted_scene_size, world_size, 1)?;
    let max_loaded_origin =
        (world_size - loaded_scene_size) / MAP_LOAD_ORIGIN_STRIDE * MAP_LOAD_ORIGIN_STRIDE;
    let containing_loaded_origin_x =
        (origin_x / MAP_LOAD_ORIGIN_STRIDE * MAP_LOAD_ORIGIN_STRIDE).min(max_loaded_origin);
    let containing_loaded_origin_z =
        (origin_z / MAP_LOAD_ORIGIN_STRIDE * MAP_LOAD_ORIGIN_STRIDE).min(max_loaded_origin);
    let loaded_end_x = containing_loaded_origin_x + loaded_scene_size;
    let loaded_end_z = containing_loaded_origin_z + loaded_scene_size;
    if origin_x < containing_loaded_origin_x
        || origin_z < containing_loaded_origin_z
        || origin_x + submitted_scene_size > loaded_end_x
        || origin_z + submitted_scene_size > loaded_end_z
    {
        return Err("submitted footprint is not contained by a legal loaded scene".to_string());
    }
    let mut maximum = static_geometry_at_window(
        terrain,
        entities,
        origin_x,
        origin_z,
        submitted_scene_size,
        world_size,
        max_triangles_per_face,
        vertex_bytes,
    )?;
    maximum.containing_loaded_origin_x = containing_loaded_origin_x;
    maximum.containing_loaded_origin_z = containing_loaded_origin_z;
    if maximum.total_faces != maximum_faces {
        return Err("static geometry summary does not match the maximum face sweep".to_string());
    }
    Ok(maximum)
}

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn legal_population_receipt(
    inputs: &mut InputLedger,
    client_sources: &BTreeMap<&'static str, Vec<u8>>,
) -> Option<Value> {
    let Some(engine_root) = std::env::var_os("ENGINE_DIR").map(PathBuf::from) else {
        inputs.fail(
            "legal_population_engine_unavailable",
            "ENGINE_DIR is required to verify the pinned engine/content population envelope",
        );
        return None;
    };
    let Some(manifest_path) = engine_root
        .parent()
        .map(|parent| parent.join("runtime-input-manifest.json"))
    else {
        inputs.fail(
            "legal_population_manifest_unavailable",
            format!(
                "staged engine root has no sibling runtime-input-manifest.json: {}",
                engine_root.display()
            ),
        );
        return None;
    };
    let result =
        legal_population::verify_staged_engine(&engine_root, &manifest_path).and_then(|verified| {
            legal_population::derive_population(
                &verified,
                client_sources,
                client::dash3d::collision_map::BuildArea::SIZE as usize,
                client::dash3d::collision_map::BuildArea::LEVELS as usize,
                &LOC_SHAPE_TO_LAYER,
            )
        });
    match result {
        Ok(receipt) => Some(receipt),
        Err(error) => {
            inputs.fail("legal_population_census_failed", error);
            None
        }
    }
}

fn add_source_hashes(ledger: &mut InputLedger) -> BTreeMap<&'static str, Vec<u8>> {
    const SOURCES: &[&str] = &[
        "crates/host/tests/render_characterization/content_census.rs",
        "crates/host/tests/render_characterization/concrete_crowd.rs",
        "crates/host/tests/render_characterization/legal_population.rs",
        "vendor/fr-client-rust/crates/client/src/render/budget.rs",
        "vendor/fr-client-rust/crates/client/src/render/nav_debug.rs",
        "vendor/fr-client-rust/crates/client/src/render/backend/gpu_atlas.rs",
        "vendor/fr-client-rust/crates/client/src/render/backend/gpu.rs",
        "vendor/fr-client-rust/crates/client/src/render/world.rs",
        "vendor/fr-client-rust/crates/client/src/render/renderer.rs",
        "vendor/fr-client-rust/crates/client/src/render/draw.rs",
        "vendor/fr-client-rust/crates/client/src/render/store.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/model.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/model_source.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/store.rs",
        "vendor/fr-client-rust/crates/client/src/datastruct/lru_cache.rs",
        "vendor/fr-client-rust/crates/client/src/datastruct/hash_table.rs",
        "vendor/fr-client-rust/crates/client/src/datastruct/linkable.rs",
        "vendor/fr-client-rust/crates/client/src/datastruct/link_list.rs",
        "vendor/fr-client-rust/crates/client/src/graphics/pix3d.rs",
        "vendor/fr-client-rust/crates/client/src/io/packet.rs",
        "vendor/fr-client-rust/crates/client/src/client/title_flames.rs",
        "vendor/fr-client-rust/crates/client/src/graphics/pix32.rs",
        "vendor/fr-client-rust/crates/client/src/graphics/pixfont.rs",
        "vendor/fr-client-rust/crates/client/src/config/mod.rs",
        "vendor/fr-client-rust/crates/client/src/config/loc_type.rs",
        "vendor/fr-client-rust/crates/client/src/config/npc_type.rs",
        "vendor/fr-client-rust/crates/client/src/config/obj_type.rs",
        "vendor/fr-client-rust/crates/client/src/config/idk_type.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/loc_shape.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/map_spot_anim.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/collision_map.rs",
        "vendor/fr-client-rust/crates/client/src/client/zone_289.rs",
        "vendor/fr-client-rust/crates/client/src/config/spot_type.rs",
        "vendor/fr-client-rust/crates/client/src/config/if_type.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs",
        "vendor/fr-client-rust/crates/client/src/render/media.rs",
        "vendor/fr-client-rust/crates/client/src/render/graphic_sprites.rs",
        "vendor/fr-client-rust/crates/client/src/client/client.rs",
        "vendor/fr-client-rust/crates/client/src/core/world.rs",
        "vendor/fr-client-rust/crates/client/src/client/actor_289.rs",
        "vendor/fr-client-rust/crates/client/src/map_cache.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/client_proj.rs",
        "crates/nav-planner/src/router.rs",
        "crates/nav-planner/src/paint.rs",
        "crates/panel/src/session.rs",
    ];
    let root = source_root();
    let mut selected = BTreeMap::new();
    for relative in SOURCES {
        let bytes = ledger.read(&format!("source:{relative}"), &root.join(relative));
        if matches!(
            *relative,
            "vendor/fr-client-rust/crates/client/src/config/loc_type.rs"
                | "vendor/fr-client-rust/crates/client/src/config/npc_type.rs"
                | "vendor/fr-client-rust/crates/client/src/config/obj_type.rs"
                | "vendor/fr-client-rust/crates/client/src/config/idk_type.rs"
                | "vendor/fr-client-rust/crates/client/src/config/spot_type.rs"
                | "vendor/fr-client-rust/crates/client/src/config/if_type.rs"
                | "vendor/fr-client-rust/crates/client/src/dash3d/loc_shape.rs"
                | "vendor/fr-client-rust/crates/client/src/dash3d/map_spot_anim.rs"
                | "vendor/fr-client-rust/crates/client/src/dash3d/collision_map.rs"
                | "vendor/fr-client-rust/crates/client/src/client/zone_289.rs"
                | "vendor/fr-client-rust/crates/client/src/client/actor_289.rs"
                | "vendor/fr-client-rust/crates/client/src/datastruct/link_list.rs"
                | "vendor/fr-client-rust/crates/client/src/datastruct/linkable.rs"
                | "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs"
                | "vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs"
                | "vendor/fr-client-rust/crates/client/src/client/client.rs"
                | "crates/nav-planner/src/router.rs"
                | "crates/nav-planner/src/paint.rs"
                | "crates/panel/src/session.rs"
        ) {
            if let Some(bytes) = bytes {
                selected.insert(*relative, bytes);
            }
        }
    }
    if let Some(engine) = std::env::var_os("ENGINE_DIR").map(PathBuf::from) {
        for (identity, relative) in [
            (
                "engine/src/network/rsbuf/build.ts",
                "src/network/rsbuf/build.ts",
            ),
            (
                "engine/src/network/rsbuf/info.ts",
                "src/network/rsbuf/info.ts",
            ),
            (
                "engine/src/engine/entity/Player.ts",
                "src/engine/entity/Player.ts",
            ),
        ] {
            if let Some(bytes) = ledger.read(&format!("source:{identity}"), &engine.join(relative))
            {
                selected.insert(identity, bytes);
            }
        }
    } else {
        ledger.fail(
            "dynamic_count_source_unavailable",
            "ENGINE_DIR is required for the actor-count and appearance authorities",
        );
    }
    selected
}
fn source_usize_const(source: &[u8], declaration: &str) -> Result<usize, String> {
    let source = std::str::from_utf8(source).map_err(|error| format!("{declaration}: {error}"))?;
    let (_, value) = source
        .split_once(declaration)
        .ok_or_else(|| format!("missing source declaration {declaration:?}"))?;
    let digits = value
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '_')
        .filter(|&character| character != '_')
        .collect::<String>();
    if digits.is_empty() {
        return Err(format!(
            "source declaration {declaration:?} has no decimal value"
        ));
    }
    digits
        .parse()
        .map_err(|error| format!("invalid source value for {declaration:?}: {error}"))
}

fn source_function_string_literals(source: &[u8], signature: &str) -> Result<Vec<String>, String> {
    let source = std::str::from_utf8(source).map_err(|error| format!("{signature}: {error}"))?;
    let (_, function) = source
        .split_once(signature)
        .ok_or_else(|| format!("missing source function {signature:?}"))?;
    let end = function
        .find("\n}\n")
        .ok_or_else(|| format!("unterminated source function {signature:?}"))?;
    let function = &function[..end];
    let mut literals = Vec::new();
    let mut remaining = function;
    while let Some(start) = remaining.find("Some(\"") {
        let value = &remaining[start + 6..];
        let end = value
            .find("\")")
            .ok_or_else(|| format!("unterminated Some string in {signature:?}"))?;
        let literal = &value[..end];
        if literal.contains('\\') {
            return Err(format!(
                "escaped source literal is unsupported in {signature:?}"
            ));
        }
        literals.push(literal.to_string());
        remaining = &value[end + 2..];
    }
    if literals.is_empty() {
        return Err(format!("no Some string literals in {signature:?}"));
    }
    Ok(literals)
}

#[derive(Default)]
struct NavContentBounds {
    collision_cells: usize,
    path_entries: usize,
    trail_entries: usize,
    hulls: usize,
    labels: usize,
    label_bytes: usize,
    nav_input_bytes: usize,
    nav_input_replacement_peak_bytes: usize,
    quads: usize,
    edges: usize,
    vertices: usize,
    nav_vertices_peak_bytes: usize,
    nav_quads_peak_bytes: usize,
    nav_edges_peak_bytes: usize,
    max_caption_bytes: usize,
    max_caption_ink_pixels: usize,
}

fn vector_replacement_peak_bytes(items: usize, item_bytes: usize) -> Option<usize> {
    items.checked_mul(item_bytes)?.checked_mul(3)
}
const VERTEX_BUFFER_MIN_BYTES: usize = 1_048_576;

fn vertex_capacity_bytes(content_upper_vertex_bytes: usize) -> Option<usize> {
    content_upper_vertex_bytes
        .max(VERTEX_BUFFER_MIN_BYTES)
        .checked_next_power_of_two()
}

fn previous_vertex_capacity_bytes(capacity_bytes: usize) -> usize {
    if capacity_bytes > VERTEX_BUFFER_MIN_BYTES {
        capacity_bytes / 2
    } else {
        0
    }
}

fn vertex_growth_peak_bytes(capacity_bytes: usize) -> Option<usize> {
    capacity_bytes.checked_add(previous_vertex_capacity_bytes(capacity_bytes))
}

fn vertex_capacity_supported(
    capacity_bytes: usize,
    max_storage_buffer_binding_size: u64,
) -> Option<bool> {
    u64::try_from(capacity_bytes)
        .ok()
        .map(|required| required <= max_storage_buffer_binding_size)
}

fn nav_content_bounds(
    scene_tiles: usize,
    trail_tiles: usize,
    hull_window: usize,
    route_transitions: usize,
    max_draw_tiles: usize,
    caption_texts: &[String],
    font: &PixFont,
) -> Result<NavContentBounds, String> {
    let layout = render_test_nav_layout();
    let collision_cells = scene_tiles
        .checked_mul(scene_tiles)
        .ok_or_else(|| "collision cell count overflowed".to_string())?;
    let raw_path_entries = route_transitions
        .checked_mul(3)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "raw route path bound overflowed".to_string())?;
    let selected_path_entries = max_draw_tiles
        .checked_add(1)
        .and_then(|value| {
            route_transitions
                .checked_mul(2)
                .and_then(|routes| value.checked_add(routes))
        })
        .ok_or_else(|| "selected route path bound overflowed".to_string())?;
    let path_entries = raw_path_entries.min(selected_path_entries);
    let hulls = route_transitions.min(hull_window / 2 + 2);
    let labels = route_transitions;
    let max_caption_bytes = caption_texts
        .iter()
        .map(String::len)
        .max()
        .unwrap_or_default();
    let max_caption_ink_pixels = caption_texts
        .iter()
        .map(|text| render_test_caption_ink_pixels(font, text))
        .max()
        .unwrap_or_default();
    let label_bytes = labels
        .checked_mul(max_caption_bytes)
        .ok_or_else(|| "label UTF-8 capacity bound overflowed".to_string())?;
    let nav_input_bytes = collision_cells
        .checked_mul(layout.collision_cell_bytes)
        .and_then(|bytes| bytes.checked_add(path_entries.checked_mul(layout.path_entry_bytes)?))
        .and_then(|bytes| bytes.checked_add(trail_tiles.checked_mul(layout.trail_entry_bytes)?))
        .and_then(|bytes| bytes.checked_add(hulls.checked_mul(layout.hull_bytes)?))
        .and_then(|bytes| bytes.checked_add(labels.checked_mul(layout.label_entry_bytes)?))
        .and_then(|bytes| bytes.checked_add(label_bytes))
        .ok_or_else(|| "NavInput content bound overflowed".to_string())?;
    let nav_input_replacement_peak_bytes = nav_input_bytes
        .checked_mul(2)
        .ok_or_else(|| "NavInput old/new replacement peak overflowed".to_string())?;
    let quads = collision_cells
        .checked_add(path_entries)
        .and_then(|count| count.checked_add(trail_tiles))
        .ok_or_else(|| "NavQuads content bound overflowed".to_string())?;
    let edges = collision_cells
        .checked_mul(layout.max_edges_per_collision_cell)
        .ok_or_else(|| "NavEdges content bound overflowed".to_string())?;
    let vertices = quads
        .checked_mul(layout.max_vertices_per_quad)
        .and_then(|count| {
            collision_cells
                .checked_mul(layout.max_corner_lines_per_collision_cell)?
                .checked_mul(layout.max_vertices_per_line)?
                .checked_add(count)
        })
        .and_then(|count| {
            edges
                .checked_mul(layout.max_vertices_per_line)?
                .checked_add(count)
        })
        .and_then(|count| {
            collision_cells
                .checked_mul(layout.nsew_glyph_pixels_per_collision_cell)?
                .checked_mul(layout.max_vertices_per_line)?
                .checked_add(count)
        })
        .and_then(|count| {
            hulls
                .checked_mul(layout.max_hull_lines)?
                .checked_mul(layout.max_vertices_per_line)?
                .checked_add(count)
        })
        .and_then(|count| {
            layout
                .click_lines
                .checked_mul(layout.max_vertices_per_line)?
                .checked_add(count)
        })
        .and_then(|count| {
            labels
                .checked_mul(max_caption_ink_pixels)?
                .checked_mul(2)?
                .checked_mul(layout.max_vertices_per_line)?
                .checked_add(count)
        })
        .and_then(|count| {
            labels
                .checked_mul(quads)?
                .checked_mul(layout.max_caption_fill_vertices_per_quad)?
                .checked_add(count)
        })
        .ok_or_else(|| "NavVertices content bound overflowed".to_string())?;
    let nav_vertices_peak_bytes = vector_replacement_peak_bytes(vertices, layout.vertex_bytes)
        .ok_or_else(|| "NavVertices replacement peak overflowed".to_string())?;
    let nav_quads_peak_bytes = vector_replacement_peak_bytes(quads, layout.quad_bytes)
        .ok_or_else(|| "NavQuads replacement peak overflowed".to_string())?;
    let nav_edges_peak_bytes = vector_replacement_peak_bytes(edges, layout.edge_bytes)
        .ok_or_else(|| "NavEdges replacement peak overflowed".to_string())?;
    Ok(NavContentBounds {
        collision_cells,
        path_entries,
        trail_entries: trail_tiles,
        hulls,
        labels,
        label_bytes,
        nav_input_bytes,
        nav_input_replacement_peak_bytes,
        quads,
        edges,
        vertices,
        nav_vertices_peak_bytes,
        nav_quads_peak_bytes,
        nav_edges_peak_bytes,
        max_caption_bytes,
        max_caption_ink_pixels,
    })
}

fn source_bytes<'a>(
    sources: &'a BTreeMap<&'static str, Vec<u8>>,
    relative: &str,
) -> Result<&'a [u8], String> {
    sources
        .get(relative)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("missing source bytes for {relative:?}"))
}

fn nav_source_bounds(
    sources: &BTreeMap<&'static str, Vec<u8>>,
    font: &PixFont,
) -> Result<NavContentBounds, String> {
    let client_source = source_bytes(
        sources,
        "vendor/fr-client-rust/crates/client/src/client/client.rs",
    )?;
    let panel_source = source_bytes(sources, "crates/panel/src/session.rs")?;
    let router_source = source_bytes(sources, "crates/nav-planner/src/router.rs")?;
    let paint_source = source_bytes(sources, "crates/nav-planner/src/paint.rs")?;
    let client_scene_tiles = source_usize_const(client_source, "const BUILD_AREA_SIZE: i32 =")?;
    let panel_scene_tiles = source_usize_const(panel_source, "const SCENE_TILES: i32 =")?;
    if client_scene_tiles != panel_scene_tiles {
        return Err(format!(
            "client and panel scene dimensions differ: {client_scene_tiles} vs {panel_scene_tiles}"
        ));
    }
    let scene_tile_formula =
        "const BUILD_AREA_TILES: usize = (BUILD_AREA_SIZE * BUILD_AREA_SIZE) as usize;";
    if !std::str::from_utf8(client_source)
        .map_err(|error| format!("client scene source: {error}"))?
        .contains(scene_tile_formula)
    {
        return Err("client trail size is no longer derived from the full build area".to_string());
    }
    let hull_window = source_usize_const(panel_source, "const HULL_WINDOW: usize =")?;
    let route_transitions = source_usize_const(router_source, "pub const NODE_BUDGET: usize =")?;
    let max_draw_tiles = source_usize_const(paint_source, "pub const MAX_DRAW_TILES: usize =")?;
    let caption_texts = source_function_string_literals(panel_source, "fn hop_caption_text(")?;
    let layout = client::render::world::render_test_content_layout();
    if layout.scene_tiles_per_axis != client_scene_tiles {
        return Err("client scene source differs from the render-world scene layout".to_string());
    }
    nav_content_bounds(
        client_scene_tiles,
        client_scene_tiles
            .checked_mul(client_scene_tiles)
            .ok_or_else(|| "client trail size overflowed".to_string())?,
        hull_window,
        route_transitions,
        max_draw_tiles,
        &caption_texts,
        font,
    )
}

#[derive(Default)]
struct AdmittedSceneGeometry {
    shared_dynamic_count_upper: usize,
    dynamic_classes: Vec<Value>,
    dynamic_faces_upper: usize,
    scene_faces_upper: usize,
    vertex_payload_bytes_upper: usize,
}

fn aggregate_admitted_geometry(
    static_faces: usize,
    shared_count: usize,
    mut classes: [(&str, usize, usize); 3],
    triangles_per_face: usize,
    vertex_bytes: usize,
) -> Result<AdmittedSceneGeometry, String> {
    classes.sort_by_key(|&(_, _, faces)| std::cmp::Reverse(faces));
    let bytes_per_face = triangles_per_face
        .checked_mul(3)
        .and_then(|vertices| vertices.checked_mul(vertex_bytes))
        .ok_or_else(|| "admitted geometry bytes per face overflowed".to_string())?;
    let mut remaining = shared_count;
    let mut dynamic_faces_upper = 0usize;
    let mut dynamic_classes = Vec::with_capacity(classes.len());
    for (class, count_upper, faces_per_entity) in classes {
        let count = remaining.min(count_upper);
        remaining -= count;
        let faces = count
            .checked_mul(faces_per_entity)
            .ok_or_else(|| format!("{class}: admitted face product overflowed"))?;
        dynamic_faces_upper = dynamic_faces_upper
            .checked_add(faces)
            .ok_or_else(|| "admitted dynamic face sum overflowed".to_string())?;
        let bytes = faces
            .checked_mul(bytes_per_face)
            .ok_or_else(|| format!("{class}: clipped vertex byte product overflowed"))?;
        dynamic_classes.push(json!({
            "class": class,
            "count_upper": count_upper,
            "count_in_maximizing_shared_allocation": count,
            "faces_per_entity_upper": faces_per_entity,
            "aggregate_faces_upper": faces,
            "clipped_vertex_bytes_upper": bytes,
        }));
    }
    let scene_faces_upper = static_faces
        .checked_add(dynamic_faces_upper)
        .ok_or_else(|| "admitted scene face upper overflowed".to_string())?;
    let vertex_payload_bytes_upper = scene_faces_upper
        .checked_mul(bytes_per_face)
        .ok_or_else(|| "admitted scene vertex byte upper overflowed".to_string())?;
    Ok(AdmittedSceneGeometry {
        shared_dynamic_count_upper: shared_count,
        dynamic_classes,
        dynamic_faces_upper,
        scene_faces_upper,
        vertex_payload_bytes_upper,
    })
}

fn admitted_scene_geometry(
    sources: &BTreeMap<&'static str, Vec<u8>>,
    faces: &Value,
    static_faces: usize,
) -> Result<AdmittedSceneGeometry, String> {
    let build = source_bytes(sources, "engine/src/network/rsbuf/build.ts")?;
    let info = std::str::from_utf8(source_bytes(sources, "engine/src/network/rsbuf/info.ts")?)
        .map_err(|error| format!("actor info source: {error}"))?;
    for guard in [
        "if (player.build.players.len() >= BuildArea.PREFERRED_PLAYERS)",
        "if (player.build.npcs.len() >= BuildArea.PREFERRED_NPCS)",
    ] {
        if !info.contains(guard) {
            return Err(format!("actor info count guard is unavailable: {guard}"));
        }
    }
    let players = source_usize_const(build, "static readonly PREFERRED_PLAYERS =")?
        .checked_add(1)
        .ok_or_else(|| "nearby players plus local player count overflowed".to_string())?;
    let npcs = source_usize_const(build, "static readonly PREFERRED_NPCS =")?;
    let read_faces = |class: &str, field: &str| {
        faces
            .get(class)
            .and_then(|value| value.get(field))
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("missing complete face upper {class}.{field}"))
    };
    let player_faces = read_faces("player", "face_upper")?;
    let npc_faces = read_faces("npc", "npc_face_upper")?;
    let transient_faces = read_faces("projectile", "projectile_face_upper")?
        .max(read_faces("map_spot", "map_spot_face_upper")?);
    let layout = client::render::world::render_test_content_layout();
    aggregate_admitted_geometry(
        static_faces,
        layout.max_dynamic_sprites,
        [
            ("player_including_local", players, player_faces),
            ("npc", npcs, npc_faces),
            (
                "projectile_or_map_spot",
                layout.max_dynamic_sprites,
                transient_faces,
            ),
        ],
        layout.max_model_triangles_per_face,
        layout.capture_vertex_bytes,
    )
}

#[derive(Default)]
struct SceneCaptureBounds {
    submitted_static: StaticGeometryMaximum,
    legal_loaded_terrain_faces: usize,
    legal_loaded_terrain_origin: Value,
    legal_loaded_entity_faces: usize,
    legal_loaded_entity_origin: [usize; 2],
    conservative_window_terrain_faces_upper: usize,
    conservative_window_terrain_origin: Value,
    conservative_window_entity_faces_upper: usize,
    conservative_window_entity_origin: Value,
    dynamic_sprite_storage_capacity: usize,
    dynamic_faces_per_sprite_upper: usize,
    conservative_dynamic_faces_upper: usize,
    conservative_scene_faces_upper: usize,
    conservative_scene_triangles_upper: usize,
    conservative_capture_payload_bytes_upper: usize,
    conservative_capture_peak_bytes_upper: usize,
    admitted: AdmittedSceneGeometry,
}

fn scene_capture_bounds(
    families: &BTreeMap<String, FamilyMaximum>,
    scene_maxima: &BTreeMap<String, SceneMaximum>,
    map_stats: &BTreeMap<String, Value>,
    submitted_static: StaticGeometryMaximum,
    sources: &BTreeMap<&'static str, Vec<u8>>,
    dynamic_faces: &Value,
) -> Result<SceneCaptureBounds, String> {
    let layout = client::render::world::render_test_content_layout();
    let family_faces = |name: &str| {
        families
            .get(name)
            .map(|family| family.maximum_faces)
            .ok_or_else(|| format!("missing model family {name:?}"))
    };
    let base_faces = family_faces("base_models")?;
    let loc_faces = family_faces("loc_models")?;
    let npc_faces = family_faces("npc_models")?;
    let obj_faces = family_faces("obj_models")?;
    let idk_faces = family_faces("idk_models")?;
    let spot_faces = family_faces("spot_models")?;
    let scene = scene_maxima
        .get("all")
        .ok_or_else(|| "maximum static loaded-scene entity record is unavailable".to_string())?;
    let legal_loaded_entity_faces = scene.faces;
    let legal_loaded_terrain_faces = map_stats
        .get("maximum_terrain_faces_per_loaded_scene")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "missing legal loaded-scene terrain face maximum".to_string())?;
    let legal_loaded_terrain_origin = map_stats
        .get("maximum_terrain_loaded_scene_origin")
        .cloned()
        .ok_or_else(|| "missing legal loaded-scene terrain origin".to_string())?;
    let conservative_window_terrain_faces_upper = map_stats
        .get("conservative_any_tile_window_terrain_face_upper")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "missing conservative terrain face upper".to_string())?;
    let conservative_window_entity_faces_upper = map_stats
        .get("conservative_any_tile_window_entity_face_upper")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "missing conservative entity face upper".to_string())?;
    let conservative_window_terrain_origin = map_stats
        .get("conservative_any_tile_window_terrain_origin")
        .cloned()
        .ok_or_else(|| "missing conservative terrain window origin".to_string())?;
    let conservative_window_entity_origin = map_stats
        .get("conservative_any_tile_window_entity_origin")
        .cloned()
        .ok_or_else(|| "missing conservative entity window origin".to_string())?;
    let legal_loaded_entity_origin = [scene.face_origin_x, scene.face_origin_z];
    let raw_wear_model_faces = base_faces
        .checked_mul(3)
        .ok_or_else(|| "raw wear-model face bound overflowed".to_string())?;
    let component_faces = [
        raw_wear_model_faces,
        loc_faces,
        npc_faces,
        obj_faces,
        idk_faces,
        spot_faces,
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    let dynamic_faces_per_sprite_upper = component_faces
        .checked_mul(12)
        .and_then(|faces| faces.checked_add(spot_faces))
        .and_then(|faces| faces.checked_add(loc_faces))
        .ok_or_else(|| "dynamic sprite face upper overflowed".to_string())?;
    let conservative_dynamic_faces_upper = layout
        .max_dynamic_sprites
        .checked_mul(dynamic_faces_per_sprite_upper)
        .ok_or_else(|| "conservative dynamic face upper overflowed".to_string())?;
    let independent_static_faces = conservative_window_terrain_faces_upper
        .checked_add(conservative_window_entity_faces_upper)
        .ok_or_else(|| "independent conservative window face maxima overflowed".to_string())?;
    let conservative_scene_faces_upper = independent_static_faces
        .checked_add(conservative_dynamic_faces_upper)
        .ok_or_else(|| "conservative scene face upper overflowed".to_string())?;
    let conservative_scene_triangles_upper = conservative_scene_faces_upper
        .checked_mul(layout.max_model_triangles_per_face)
        .ok_or_else(|| "conservative scene triangle upper overflowed".to_string())?;
    let capture_vertices = conservative_scene_triangles_upper
        .checked_mul(3)
        .ok_or_else(|| "conservative capture vertex count overflowed".to_string())?;
    let conservative_capture_payload_bytes_upper = capture_vertices
        .checked_mul(layout.capture_vertex_bytes)
        .ok_or_else(|| "conservative capture payload upper overflowed".to_string())?;
    let conservative_capture_peak_bytes_upper =
        vector_replacement_peak_bytes(capture_vertices, layout.capture_vertex_bytes)
            .ok_or_else(|| "conservative capture replacement peak overflowed".to_string())?;
    Ok(SceneCaptureBounds {
        submitted_static,
        legal_loaded_terrain_faces,
        legal_loaded_terrain_origin,
        legal_loaded_entity_faces,
        legal_loaded_entity_origin,
        conservative_window_terrain_faces_upper,
        conservative_window_terrain_origin,
        conservative_window_entity_faces_upper,
        conservative_window_entity_origin,
        dynamic_sprite_storage_capacity: layout.max_dynamic_sprites,
        dynamic_faces_per_sprite_upper,
        conservative_dynamic_faces_upper,
        conservative_scene_faces_upper,
        conservative_scene_triangles_upper,
        conservative_capture_payload_bytes_upper,
        conservative_capture_peak_bytes_upper,
        admitted: admitted_scene_geometry(sources, dynamic_faces, submitted_static.total_faces)?,
    })
}

#[derive(Clone, Copy, Default)]
struct ModelSourceParts {
    loc: usize,
    npc: usize,
    npc_head: usize,
    idk: usize,
    idk_head: usize,
    object_wear: usize,
    player_slots: usize,
    player_components: usize,
    max_source_parts: usize,
}

fn maximum_source_parts(values: impl Iterator<Item = usize>) -> usize {
    values.max().unwrap_or_default()
}

fn model_source_parts(cache: &Cache, player_slots: usize) -> Option<ModelSourceParts> {
    let loc = maximum_source_parts(
        cache
            .locs
            .iter()
            .map(|loc| loc.model.as_ref().map_or(0, Vec::len)),
    );
    let npc = maximum_source_parts(
        cache
            .npcs
            .iter()
            .map(|npc| npc.model.as_ref().map_or(0, Vec::len)),
    );
    let npc_head = maximum_source_parts(
        cache
            .npcs
            .iter()
            .map(|npc| npc.head.as_ref().map_or(0, Vec::len)),
    );
    let idk = maximum_source_parts(
        cache
            .idks
            .iter()
            .map(|idk| idk.model.as_ref().map_or(0, Vec::len)),
    );
    let idk_head = maximum_source_parts(
        cache
            .idks
            .iter()
            .map(|idk| idk.head.iter().filter(|&&id| id >= 0).count()),
    );
    let object_wear = maximum_source_parts(cache.objs.iter().map(|obj| {
        let male = [obj.manwear, obj.manwear2, obj.manwear3]
            .into_iter()
            .filter(|&id| id >= 0)
            .count();
        let female = [obj.womanwear, obj.womanwear2, obj.womanwear3]
            .into_iter()
            .filter(|&id| id >= 0)
            .count();
        male.max(female)
    }));
    let player_components = player_slots.checked_mul(idk.max(object_wear).max(1))?;
    let max_source_parts = [
        loc,
        npc,
        npc_head,
        idk,
        idk_head,
        object_wear,
        1,
        player_components,
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    Some(ModelSourceParts {
        loc,
        npc,
        npc_head,
        idk,
        idk_head,
        object_wear,
        player_slots,
        player_components,
        max_source_parts,
    })
}

#[derive(Clone)]
struct OwnerValue {
    bytes: usize,
    details: Value,
}

#[derive(Clone)]
struct OwnerBounds {
    head_models: OwnerValue,
    head_misc: OwnerValue,
    temp_models: OwnerValue,
    shared_models: OwnerValue,
    shared_sprites: OwnerValue,
    context_cpu: OwnerValue,
}

fn sum_bytes(values: impl IntoIterator<Item = usize>) -> Option<usize> {
    values.into_iter().try_fold(0usize, usize::checked_add)
}

fn largest_family_bytes(
    families: &BTreeMap<String, FamilyMaximum>,
    names: &[&str],
    capacity: usize,
) -> Option<usize> {
    let mut sizes = Vec::new();
    for name in names {
        sizes.extend(families.get(*name)?.retained_sizes.iter().copied());
    }
    sizes.sort_unstable_by(|left, right| right.cmp(left));
    sizes
        .into_iter()
        .take(capacity)
        .try_fold(0usize, usize::checked_add)
}

fn family_total_bytes(families: &BTreeMap<String, FamilyMaximum>, names: &[&str]) -> Option<usize> {
    names.iter().try_fold(0usize, |total, name| {
        total.checked_add(families.get(*name)?.total_bytes)
    })
}

fn family_maximum_bytes(
    families: &BTreeMap<String, FamilyMaximum>,
    names: &[&str],
) -> Option<usize> {
    names
        .iter()
        .map(|name| families.get(*name).map(|family| family.maximum_bytes))
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().max().unwrap_or_default())
}

#[expect(
    clippy::too_many_arguments,
    reason = "The owner census combines distinct measured layouts and source-content populations"
)]
fn content_owner_bounds(
    families: &BTreeMap<String, FamilyMaximum>,
    scene_maxima: &BTreeMap<String, SceneMaximum>,
    model_caches: ModelCacheLayouts,
    model_parts: ModelSourceParts,
    world: client::render::world::RenderTestContentLayout,
    renderer: client::render::renderer::RenderTestRendererLayout,
    graphic_sprites: client::render::renderer::RenderTestGraphicSpriteLayout,
    atlas: client::render::backend::gpu_atlas::RenderTestAtlasLayout,
    shared_store: client::dash3d::store::RenderTestSharedStoreLayout,
    shared_index: client::render::store::RenderTestSharedIndexLayout,
    media_file_bytes: usize,
    process_media_bytes: usize,
    item_sprite_ids: usize,
) -> Result<OwnerBounds, String> {
    const MODEL_FAMILIES: &[&str] = &[
        "base_models",
        "loc_models",
        "npc_models",
        "obj_models",
        "idk_models",
        "spot_models",
    ];
    let raw = families
        .get("base_models")
        .ok_or("raw model family is unavailable")?;
    let loc = families
        .get("loc_models")
        .ok_or("transformed loc family is unavailable")?;
    let npc = families
        .get("npc_models")
        .ok_or("transformed NPC family is unavailable")?;
    let obj = families
        .get("obj_models")
        .ok_or("transformed object family is unavailable")?;
    let idk = families
        .get("idk_models")
        .ok_or("transformed IDK family is unavailable")?;
    let spot = families
        .get("spot_models")
        .ok_or("transformed spot family is unavailable")?;
    let scene = scene_maxima
        .get("all")
        .ok_or("maximum static scene is unavailable")?;
    let raw_max = raw.maximum_bytes;
    let model_max = family_maximum_bytes(families, MODEL_FAMILIES)
        .ok_or("maximum model family is unavailable")?;
    let model_header = model_caches.idk.model_value_bytes;
    let arc_header = model_caches.loc.arc_header_bytes;
    let source_entry_bytes = size_of::<Option<Model>>();
    let source_bytes = |parts: usize| {
        raw_max
            .checked_mul(parts)?
            .checked_add(source_entry_bytes.checked_mul(parts)?)
    };
    let transformed_peak = |parts: usize, family: &FamilyMaximum| {
        source_bytes(parts)?.checked_add(family.maximum_bytes)
    };

    let idk_component_bytes = idk.maximum_bytes.max(obj.maximum_bytes);
    let player_component_bytes = model_parts
        .player_slots
        .checked_mul(idk_component_bytes)
        .ok_or("player component payload bound overflowed")?;
    let player_component_source_bytes = source_bytes(model_parts.player_components)
        .ok_or("player component source bound overflowed")?;
    let player_base_peak = player_component_bytes
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_add(player_component_source_bytes))
        .and_then(|bytes| bytes.checked_add(idk_component_bytes))
        .ok_or("player composition base peak overflowed")?;
    let npc_head_source_bytes =
        source_bytes(model_parts.npc_head).ok_or("NPC head source bound overflowed")?;
    let npc_head_peak = npc_head_source_bytes
        .checked_mul(2)
        .ok_or("NPC head transform peak overflowed")?;
    let npc_body_peak =
        transformed_peak(model_parts.npc, npc).ok_or("NPC body transform peak overflowed")?;
    let npc_peak = npc_head_peak.max(npc_body_peak);
    let loc_peak = transformed_peak(model_parts.loc, loc).ok_or("loc transform peak overflowed")?;
    let object_peak = transformed_peak(model_parts.object_wear.max(1), obj)
        .ok_or("object-wear transform peak overflowed")?;
    let idk_peak = transformed_peak(model_parts.idk.max(model_parts.idk_head), idk)
        .ok_or("IDK transform peak overflowed")?;
    let spot_peak = transformed_peak(1, spot).ok_or("spot transform peak overflowed")?;
    let player_peak = sum_bytes([player_base_peak, loc_peak, spot_peak])
        .ok_or("player attached-model peak overflowed")?;
    let temp_models = [
        loc_peak,
        npc_peak,
        object_peak,
        idk_peak,
        spot_peak,
        player_peak,
        graphic_sprites.temp_models_peak_bytes,
    ]
    .into_iter()
    .max()
    .unwrap_or_default();

    let player_dynamic_model_bytes = sum_bytes([
        player_component_bytes,
        loc.maximum_bytes,
        spot.maximum_bytes,
    ])
    .ok_or("dynamic player model bound overflowed")?;
    let dynamic_model_max = model_max.max(player_dynamic_model_bytes);
    let dynamic_model_bytes = dynamic_model_max
        .checked_mul(world.max_dynamic_sprites)
        .ok_or("dynamic model payload bound overflowed")?;
    let head_model_replacement = dynamic_model_max
        .checked_mul(2)
        .ok_or("old/new model replacement bound overflowed")?;
    let head_models = scene
        .model_bytes
        .checked_add(dynamic_model_bytes)
        .and_then(|bytes| bytes.checked_add(head_model_replacement))
        .ok_or("HeadModels content bound overflowed")?;

    let tile_vector_peak =
        vector_replacement_peak_bytes(world.max_tile_cache_slots, world.tile_cache_slot_bytes)
            .ok_or("tile vector replacement peak overflowed")?;
    let tile_payloads = world
        .max_tile_cache_slots
        .checked_mul(2)
        .and_then(|slots| slots.checked_mul(world.tile_cache_payload_bytes))
        .ok_or("tile cache payload bound overflowed")?;
    let tile_vectors = tile_vector_peak
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(tile_payloads))
        .ok_or("tile cache bound overflowed")?;
    let active_occluders = world
        .max_active_occluders
        .checked_mul(size_of::<Option<usize>>())
        .ok_or("active occluder bound overflowed")?;
    let occlusion_cycles = world
        .occlusion_cycle_entries
        .checked_mul(size_of::<i32>())
        .ok_or("occlusion cycle bound overflowed")?;
    let sprite_buffer = world
        .max_sprite_buffer
        .checked_mul(size_of::<Option<usize>>())
        .ok_or("sprite buffer bound overflowed")?;
    let world_sprite_slots = scene
        .count
        .checked_add(world.max_dynamic_sprites)
        .ok_or("world sprite count overflowed")?;
    let sprite_model_peak =
        vector_replacement_peak_bytes(world_sprite_slots, world.scene_model_entry_bytes)
            .ok_or("sprite model-cache replacement peak overflowed")?;
    let sprite_stamp_peak =
        vector_replacement_peak_bytes(world_sprite_slots, world.sprite_stamp_bytes)
            .ok_or("sprite stamp replacement peak overflowed")?;
    let dynamic_sprite_element_bytes = sum_bytes([
        world.dynamic_sprite_index_bytes,
        world.dynamic_sprite_element_bytes,
        world.dynamic_free_element_bytes,
    ])
    .ok_or("dynamic sprite element layout overflowed")?;
    let dynamic_sprite_arrays = world
        .max_dynamic_sprites
        .checked_mul(dynamic_sprite_element_bytes)
        .ok_or("dynamic sprite array bound overflowed")?;
    let world_sprite_peak =
        vector_replacement_peak_bytes(world_sprite_slots, world.world_sprite_element_bytes)
            .ok_or("world sprite arena replacement peak overflowed")?;
    let max_model_points = MODEL_FAMILIES
        .iter()
        .map(|name| families.get(*name).map(|family| family.maximum_points))
        .collect::<Option<Vec<_>>>()
        .ok_or("model point-count inventory is incomplete")?
        .into_iter()
        .max()
        .unwrap_or_default();
    let share_maps = max_model_points
        .checked_mul(2)
        .and_then(|points| points.checked_mul(size_of::<i32>()))
        .ok_or("model share-map bound overflowed")?;
    let fill_queue =
        vector_replacement_peak_bytes(world.max_tile_cache_slots, world.fill_queue_entry_bytes)
            .ok_or("fill queue replacement peak overflowed")?;
    let ground_layers = world.max_tile_cache_slots;
    let overlay_ground = ground_layers
        .checked_mul(world.ground_payload_bytes)
        .ok_or("overlay ground payload bound overflowed")?;
    let graphics_replacement = graphic_sprites
        .retained_bytes
        .checked_add(graphic_sprites.entry_bytes)
        .ok_or("graphic sprite replacement peak overflowed")?;
    let item_sprite_clone = shared_store
        .pix32_bytes
        .checked_add(
            shared_store
                .item_sprite_pixels
                .checked_mul(shared_store.pix32_pixel_bytes)
                .ok_or("item sprite pixel bound overflowed")?,
        )
        .ok_or("item sprite clone bound overflowed")?;
    let renderer_misc = sum_bytes([
        renderer.pix3d_game_bytes,
        renderer.texture_buffer_bytes,
        renderer.tile_occupancy_bytes,
        renderer.minimap_bytes,
        renderer.minimap_mask_bytes,
        renderer.active_map_function_bytes,
        renderer.active_map_function_payload_bytes,
        renderer.image_runes_bytes,
        renderer.title_icon_bytes,
        renderer.title_flames_bytes,
        renderer.chat_text_capacity_bytes,
        graphics_replacement,
        item_sprite_clone,
    ])
    .ok_or("renderer HeadMisc payload sum overflowed")?;
    let world_misc = sum_bytes([
        world.visibility_backing_bytes,
        world.visibility_build_scratch_bytes,
        active_occluders,
        occlusion_cycles,
        sprite_buffer,
        tile_vectors,
        sprite_model_peak,
        sprite_stamp_peak,
        dynamic_sprite_arrays,
        world_sprite_peak,
        share_maps,
        fill_queue,
        overlay_ground,
    ])
    .ok_or("RenderWorld HeadMisc payload sum overflowed")?;
    let head_misc = renderer_misc
        .checked_add(world_misc)
        .ok_or("HeadMisc content bound overflowed")?;

    let model_cache_storage = model_caches
        .allocation_bytes()
        .ok_or("config model-cache storage overflowed")?;
    let loc_raw_entries = raw
        .largest_bytes(model_caches.loc.raw_cache.capacity)
        .ok_or("raw loc cache top-K payload overflowed")?;
    let loc_transformed_entries = loc
        .largest_bytes(model_caches.loc.transformed_cache.capacity)
        .ok_or("transformed loc cache top-K payload overflowed")?;
    let npc_entries = npc
        .largest_bytes(model_caches.npc.capacity)
        .and_then(|bytes| {
            bytes.checked_sub(
                npc.variants
                    .min(model_caches.npc.capacity)
                    .checked_mul(model_header)?,
            )
        })
        .ok_or("NPC cache top-K payload overflowed")?;
    let obj_entries = obj
        .largest_bytes(model_caches.obj.capacity)
        .and_then(|bytes| {
            bytes.checked_sub(
                obj.variants
                    .min(model_caches.obj.capacity)
                    .checked_mul(model_header)?,
            )
        })
        .ok_or("object cache top-K payload overflowed")?;
    let spot_entries = spot
        .largest_bytes(model_caches.spot.capacity)
        .and_then(|bytes| {
            bytes.checked_sub(
                spot.variants
                    .min(model_caches.spot.capacity)
                    .checked_mul(model_header)?,
            )
        })
        .ok_or("spot cache top-K payload overflowed")?;
    let transformed_model_entries = [
        "loc_models",
        "npc_models",
        "obj_models",
        "idk_models",
        "spot_models",
    ]
    .iter()
    .try_fold(0usize, |count, name| {
        count.checked_add(families.get(*name)?.variants)
    })
    .ok_or("model family variant inventory is incomplete or overflowed")?;
    let model_index_entries = raw
        .variants
        .checked_add(transformed_model_entries)
        .ok_or("model index variant count overflowed")?;
    let interface_entries =
        largest_family_bytes(families, MODEL_FAMILIES, model_caches.interface.capacity)
            .and_then(|bytes| {
                bytes.checked_sub(
                    model_index_entries
                        .min(model_caches.interface.capacity)
                        .checked_mul(model_header)?,
                )
            })
            .ok_or("interface cache top-K payload overflowed")?;
    let loc_cache_arc_headers = sum_bytes([
        raw.variants
            .min(model_caches.loc.raw_cache.capacity)
            .checked_mul(arc_header)
            .ok_or("raw loc cache Arc header bound overflowed")?,
        loc.variants
            .min(model_caches.loc.transformed_cache.capacity)
            .checked_mul(arc_header)
            .ok_or("transformed loc cache Arc header bound overflowed")?,
    ])
    .ok_or("loc cache Arc header sum overflowed")?;
    let player_entries = player_component_bytes
        .checked_sub(model_header)
        .and_then(|bytes| bytes.checked_mul(model_caches.player.model_cache.capacity))
        .ok_or("player cache payload bound overflowed")?;
    let loc_sentinel_payload = model_caches
        .loc_sentinel_count()
        .and_then(|count| count.checked_mul(model_header.checked_add(arc_header)?))
        .ok_or("loc cache sentinel payload bound overflowed")?;
    let inline_cache_sentinels = sum_bytes([
        model_caches.npc.sentinel_count,
        model_caches.obj.sentinel_count,
        model_caches.spot.sentinel_count,
        model_caches.interface.sentinel_count,
        model_caches.player.model_cache.sentinel_count,
    ])
    .ok_or("inline model cache sentinel count overflowed")?;
    let inline_sentinel_payload = inline_cache_sentinels
        .checked_mul(model_header)
        .ok_or("inline model cache sentinel payload overflowed")?;
    let config_cache_payload = sum_bytes([
        loc_raw_entries,
        loc_transformed_entries,
        loc_cache_arc_headers,
        npc_entries,
        obj_entries,
        spot_entries,
        interface_entries,
        player_entries,
        loc_sentinel_payload,
        inline_sentinel_payload,
    ])
    .ok_or("config model-cache payload sum overflowed")?;
    let all_model_payloads = family_total_bytes(families, MODEL_FAMILIES)
        .ok_or("complete model family payload sum is unavailable")?;
    let shared_model_store_payload =
        largest_family_bytes(families, MODEL_FAMILIES, shared_store.model_capacity)
            .and_then(|bytes| {
                bytes.checked_add(
                    model_index_entries
                        .min(shared_store.model_capacity)
                        .checked_mul(shared_store.model_arc_header_bytes)?,
                )
            })
            .ok_or("shared model-store top-K payload overflowed")?;
    let model_store_sentinel_entry_bytes = model_header
        .checked_add(shared_store.model_arc_header_bytes)
        .ok_or("model-store sentinel entry size overflowed")?;
    let model_store_sentinel_payload = shared_store
        .model_sentinel_count
        .checked_mul(model_store_sentinel_entry_bytes)
        .ok_or("model-store sentinel payload overflowed")?;
    let model_index_orphans = all_model_payloads
        .checked_add(
            model_index_entries
                .checked_mul(2 * size_of::<usize>())
                .ok_or("model index Arc-header bound overflowed")?,
        )
        .ok_or("model orphan payload bound overflowed")?;
    let shared_models = sum_bytes([
        shared_store.model_store_metadata_bytes,
        shared_store.model_lru_capacity_bytes,
        shared_model_store_payload,
        model_store_sentinel_payload,
        model_cache_storage,
        config_cache_payload,
        shared_index.model_index_mutex_bytes,
        shared_index.model_index_capacity_bytes,
        model_index_orphans,
    ])
    .ok_or("SharedModels content bound overflowed")?;

    let sprite_payload_per_item = shared_store
        .pix32_bytes
        .checked_add(
            shared_store
                .item_sprite_pixels
                .checked_mul(shared_store.pix32_pixel_bytes)
                .ok_or("item sprite pixel bound overflowed")?,
        )
        .and_then(|bytes| bytes.checked_add(shared_store.sprite_arc_header_bytes))
        .ok_or("item sprite payload bound overflowed")?;
    let sprite_content_payload = item_sprite_ids
        .checked_mul(sprite_payload_per_item)
        .ok_or("item sprite content count overflowed")?;
    let sentinel_sprite_bytes = shared_store
        .pix32_bytes
        .checked_add(shared_store.sprite_arc_header_bytes)
        .ok_or("sprite sentinel entry size overflowed")?;
    let sprite_sentinels = shared_store
        .sprite_sentinel_count
        .checked_mul(sentinel_sprite_bytes)
        .ok_or("sprite sentinel payload bound overflowed")?;
    let shared_sprites = sum_bytes([
        shared_store.sprite_lru_capacity_bytes,
        shared_index.sprite_index_mutex_bytes,
        shared_index.sprite_index_capacity_bytes,
        sprite_content_payload,
        sprite_sentinels,
    ])
    .ok_or("SharedSprites content bound overflowed")?;

    let context_cpu = sum_bytes([
        process_media_bytes,
        atlas.gpu_assets_bytes,
        atlas.font_free_bytes,
        atlas.sprite_atlas_cpu_bytes,
        atlas.region_cache_bytes,
        atlas.model_mip_peak_scratch_bytes,
    ])
    .ok_or("ContextCpu content bound overflowed")?;

    Ok(OwnerBounds {
        head_models: OwnerValue {
            bytes: head_models,
            details: json!({
                "maximum_resident_loaded_scene_model_bytes": scene.model_bytes,
                "maximum_resident_loaded_scene_entity_rows": scene.count,
                "dynamic_sprite_storage_capacity_slots": world.max_dynamic_sprites,
                "dynamic_population_bound_established": false,
                "max_retained_model_family_bytes": model_max,
                "player_dynamic_model_bytes_upper": player_dynamic_model_bytes,
                "dynamic_model_bytes_per_slot_loose_upper": dynamic_model_max,
                "conservative_all_storage_slots_model_payload_bytes_upper": dynamic_model_bytes,
                "single_slot_old_new_model_replacement_bytes": head_model_replacement,
            }),
        },
        head_misc: OwnerValue {
            bytes: head_misc,
            details: json!({
                "renderer_misc_bytes": renderer_misc,
                "render_world_misc_bytes": world_misc,
                "graphic_sprite_cache": {
                    "unique_keys": graphic_sprites.unique_keys,
                    "entry_bytes": graphic_sprites.entry_bytes,
                    "key_bytes": graphic_sprites.key_bytes,
                    "sprite_pixel_bytes": graphic_sprites.sprite_pixel_bytes,
                    "retained_bytes": graphic_sprites.retained_bytes,
                    "old_entry_vector_bound_bytes": graphic_sprites.entry_bytes,
                },
                "world_slots": {
                    "tile_cache_slots_per_vector": world.max_tile_cache_slots,
                    "dynamic_sprite_storage_capacity_slots": world.max_dynamic_sprites,
                    "world_sprite_slots_upper": world_sprite_slots,
                    "overlay_ground_layers": ground_layers,
                    "sprite_model_vector_replacement_peak_bytes": sprite_model_peak,
                    "sprite_stamp_vector_replacement_peak_bytes": sprite_stamp_peak,
                    "dynamic_sprite_arrays_bytes": dynamic_sprite_arrays,
                    "world_sprite_arena_replacement_peak_bytes": world_sprite_peak,
                    "tile_cache_replacement_peak_bytes": tile_vectors,
                    "fill_queue_replacement_peak_bytes": fill_queue,
                    "sprite_buffer_bytes": sprite_buffer,
                    "active_occluder_bytes": active_occluders,
                    "occlusion_cycle_bytes": occlusion_cycles,
                    "share_map_peak_bytes": share_maps,
                    "overlay_ground_payload_bytes": overlay_ground,
                    "visibility_backing_bytes": world.visibility_backing_bytes,
                    "visibility_build_scratch_bytes": world.visibility_build_scratch_bytes,
                },
                "renderer_layout": {
                    "pix3d_game_bytes": renderer.pix3d_game_bytes,
                    "active_map_function_bytes": renderer.active_map_function_bytes,
                    "texture_buffer_bytes": renderer.texture_buffer_bytes,
                    "tile_occupancy_bytes": renderer.tile_occupancy_bytes,
                    "minimap_bytes": renderer.minimap_bytes,
                    "minimap_mask_bytes": renderer.minimap_mask_bytes,
                    "active_map_function_payload_bytes": renderer.active_map_function_payload_bytes,
                    "title_flames_bytes": renderer.title_flames_bytes,
                    "image_runes_bytes": renderer.image_runes_bytes,
                    "title_icon_bytes": renderer.title_icon_bytes,
                    "chat_text_capacity_bytes": renderer.chat_text_capacity_bytes,
                    "max_chat_message_bytes": renderer.max_chat_message_bytes,
                },
                "item_sprite_clone_bytes": item_sprite_clone,
            }),
        },
        temp_models: OwnerValue {
            bytes: temp_models,
            details: json!({
                "source_part_bounds": {
                    "loc": model_parts.loc,
                    "npc": model_parts.npc,
                    "npc_head": model_parts.npc_head,
                    "idk": model_parts.idk,
                    "idk_head": model_parts.idk_head,
                    "object_wear": model_parts.object_wear,
                    "player_appearance_slots": model_parts.player_slots,
                    "player_model_components": model_parts.player_components,
                    "maximum_source_parts": model_parts.max_source_parts,
                },
                "transform_peak_candidates_bytes": {
                    "loc": loc_peak,
                    "npc_body": npc_body_peak,
                    "npc_head": npc_head_peak,
                    "object_wear": object_peak,
                    "idk_body_and_head": idk_peak,
                    "spot": spot_peak,
                    "player": player_peak,
                    "graphic_sprite_decode": graphic_sprites.temp_models_peak_bytes,
                },
                "raw_model_max_bytes": raw_max,
                "maximum_transformed_model_bytes": model_max,
                "player_component_bytes_upper": player_component_bytes,
                "player_composition_peak_bytes": player_peak,
                "graphic_sprite_decode_peak_bytes": graphic_sprites.temp_models_peak_bytes,
                "media_archive_source_bytes": media_file_bytes,
            }),
        },
        shared_models: OwnerValue {
            bytes: shared_models,
            details: json!({
                "model_store_metadata_bytes": shared_store.model_store_metadata_bytes,
                "model_lru_capacity_bytes": shared_store.model_lru_capacity_bytes,
                "top_k_store_payload_bytes": shared_model_store_payload,
                "model_store_sentinel_payload_bytes": model_store_sentinel_payload,
                "config_cache_capacity_bytes": model_cache_storage,
                "config_cache_top_k_payload_bytes": config_cache_payload,
                "config_cache_layouts": model_caches.json(),
                "loc_cache_arc_header_bytes": loc_cache_arc_headers,
                "inline_cache_sentinel_payload_bytes": inline_sentinel_payload,
                "model_index_mutex_bytes": shared_index.model_index_mutex_bytes,
                "model_index_capacity_bytes": shared_index.model_index_capacity_bytes,
                "model_index_orphan_payload_upper_bytes": model_index_orphans,
                "all_model_family_payload_bytes": all_model_payloads,
            }),
        },
        shared_sprites: OwnerValue {
            bytes: shared_sprites,
            details: json!({
                "unique_object_type_ids_upper": item_sprite_ids,
                "sprite_payload_bytes_per_item": sprite_payload_per_item,
                "all_item_sprite_payload_upper_bytes": sprite_content_payload,
                "sprite_lru_capacity_bytes": shared_store.sprite_lru_capacity_bytes,
                "sprite_index_capacity_bytes": shared_index.sprite_index_capacity_bytes,
                "sprite_sentinel_payload_bytes": sprite_sentinels,
            }),
        },
        context_cpu: OwnerValue {
            bytes: context_cpu,
            details: json!({
                "process_media_retained_bytes": process_media_bytes,
                "gpu_assets_inline_bytes": atlas.gpu_assets_bytes,
                "sprite_atlas_cpu_bytes": atlas.sprite_atlas_cpu_bytes,
                "font_free_region_bytes": atlas.font_free_bytes,
                "region_cache_bytes": atlas.region_cache_bytes,
                "model_mip_peak_scratch_bytes": atlas.model_mip_peak_scratch_bytes,
            }),
        },
    })
}

fn resource_alignment(resource: RenderResource) -> usize {
    let nav = render_test_nav_layout();
    match resource {
        RenderResource::Capture => align_of::<client::render::world::GpuVertex>(),
        RenderResource::NavVertices => nav.vertex_alignment,
        RenderResource::NavQuads => nav.quad_alignment,
        RenderResource::NavEdges => nav.edge_alignment,
        RenderResource::NavInput => nav.input_alignment,
        RenderResource::HeadModels | RenderResource::TempModels | RenderResource::SharedModels => {
            align_of::<Model>().max(align_of::<usize>())
        }
        RenderResource::HeadMisc => align_of::<client::render::Renderer>(),
        RenderResource::SharedSprites => align_of::<usize>(),
        RenderResource::ContextCpu => align_of::<GpuAssets>(),
        // Logical GPU bytes use four-byte buffer/texel units. Driver allocation
        // alignment belongs to native memory, not this logical byte ceiling.
        RenderResource::Vertex | RenderResource::ContextGpu => align_of::<u32>(),
        RenderResource::VertexCoordinates => {
            unreachable!("fixed coordinate range has no byte alignment")
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Each resource row preserves independently available census inputs and their blockers"
)]
fn resource_rows(
    families: &BTreeMap<String, FamilyMaximum>,
    scene_maxima: &BTreeMap<String, SceneMaximum>,
    model_total_bytes: Option<usize>,
    raw_model_count: Option<usize>,
    nav_bounds: Option<&NavContentBounds>,
    capture_bounds: Option<&SceneCaptureBounds>,
    atlas_layout: Option<client::render::backend::gpu_atlas::RenderTestAtlasLayout>,
    shared_store_layout: Option<client::dash3d::store::RenderTestSharedStoreLayout>,
    shared_index_layout: Option<client::render::store::RenderTestSharedIndexLayout>,
    item_sprite_ids: Option<usize>,
    owner_bounds: Option<&OwnerBounds>,
    vertex_content_upper_established: bool,
) -> Vec<Value> {
    let legal_vertex_capacity = if vertex_content_upper_established {
        capture_bounds
            .and_then(|capture| vertex_capacity_bytes(capture.admitted.vertex_payload_bytes_upper))
    } else {
        None
    };
    let vertex_device_support =
        legal_vertex_capacity
            .zip(atlas_layout)
            .and_then(|(capacity, atlas)| {
                vertex_capacity_supported(capacity, atlas.max_storage_buffer_binding_size)
            });
    let vertex_device_rejected = vertex_device_support == Some(false);
    let model_trigger = families
        .get("base_models")
        .and_then(|family| family.trigger.as_deref());
    let loc_trigger = families
        .get("loc_models")
        .and_then(|family| family.trigger.as_deref());
    let scene_trigger = scene_maxima.get("all").map(|max| {
        format!(
            "loaded-scene-origin:{},{};entity-rows:{}",
            max.origin_x, max.origin_z, max.count
        )
    });
    let capture_trigger = capture_bounds.map(|capture| {
        format!(
            "submitted-static-origin:{},{};conservative-unqualified-dynamic-upper",
            capture.submitted_static.origin_x, capture.submitted_static.origin_z
        )
    });
    let shared_known = json!({
        "all_serialized_model_retained_bytes": model_total_bytes,
        "raw_model_record_count": raw_model_count,
        "base_model_max_id": model_trigger,
        "model_store_layout": shared_store_layout.map(|layout| json!({
            "model_capacity": layout.model_capacity,
            "model_store_metadata_bytes": layout.model_store_metadata_bytes,
            "model_lru_capacity_bytes": layout.model_lru_capacity_bytes,
            "model_node_bytes": layout.model_node_bytes,
            "model_sentinel_count": layout.model_sentinel_count,
            "model_arc_header_bytes": layout.model_arc_header_bytes,
        })),
        "model_index_layout": shared_index_layout.map(|layout| json!({
            "mutex_bytes": layout.model_index_mutex_bytes,
            "entry_bytes": layout.model_index_entry_bytes,
            "capacity_bytes": layout.model_index_capacity_bytes,
            "orphan_bytes_per_dead_arc": layout.model_orphan_bytes_per_dead_arc,
        })),
        "transformed_model_families": (["loc_models", "npc_models", "obj_models", "idk_models", "spot_models"]
            .into_iter()
            .filter_map(|name| families.get(name).map(|family| (name, family)))
            .map(|(name, family)| (name, json!({
                "variants": family.variants,
                "sum_retained_bytes": family.total_bytes,
            })))
            .collect::<BTreeMap<_, _>>()),
        "source_derived_owner_bound": owner_bounds.map(|bounds| json!({
            "content_upper_bytes": bounds.shared_models.bytes,
            "known_measurements": bounds.shared_models.details,
        })),
    });
    let nav_known = nav_bounds.map(|nav| {
        json!({
            "collision_cells": nav.collision_cells,
            "path_entries": nav.path_entries,
            "trail_entries": nav.trail_entries,
            "hulls": nav.hulls,
            "labels": nav.labels,
            "max_caption_bytes": nav.max_caption_bytes,
            "max_caption_ink_pixels": nav.max_caption_ink_pixels,
            "label_utf8_capacity_bytes": nav.label_bytes,
            "nav_input_capacity_bytes": nav.nav_input_bytes,
            "nav_input_replacement_peak_bytes": nav.nav_input_replacement_peak_bytes,
            "quads": nav.quads,
            "edges": nav.edges,
            "vertices": nav.vertices,
            "nav_vertices_replacement_peak_bytes": nav.nav_vertices_peak_bytes,
            "nav_quads_replacement_peak_bytes": nav.nav_quads_peak_bytes,
            "nav_edges_replacement_peak_bytes": nav.nav_edges_peak_bytes,
        })
    });
    let capture_known = capture_bounds.map(|capture| {
        json!({
            "classification": "conservative_unqualified_upper_not_an_achieved_scene",
            "exact_maximum_submitted_static_geometry": capture.submitted_static.json(),
            "legal_loaded_scene_component_maxima": {
                "loaded_scene_tiles_per_axis": client::dash3d::BuildArea::SIZE,
                "legal_origin_stride_tiles": MAP_LOAD_ORIGIN_STRIDE,
                "terrain_face_maximum": capture.legal_loaded_terrain_faces,
                "terrain_maximum_origin_world_tile": capture.legal_loaded_terrain_origin,
                "static_entity_face_maximum": capture.legal_loaded_entity_faces,
                "static_entity_maximum_origin_world_tile": capture.legal_loaded_entity_origin,
                "components_share_one_origin": false
            },
            "conservative_any_tile_window_components": {
                "window_tiles_per_axis": client::dash3d::BuildArea::SIZE,
                "origin_stride_tiles": 1,
                "terrain_face_upper": capture.conservative_window_terrain_faces_upper,
                "terrain_window_origin_world_tile": capture.conservative_window_terrain_origin,
                "static_entity_face_upper": capture.conservative_window_entity_faces_upper,
                "entity_window_origin_world_tile": capture.conservative_window_entity_origin,
                "components_share_one_window": false,
                "used_only_for_conservative_upper": true
            },
            "loose_storage_only_envelope": {
                "sprite_arena_storage_capacity_slots": capture.dynamic_sprite_storage_capacity,
                "faces_per_sprite_loose_composition_upper": capture.dynamic_faces_per_sprite_upper,
                "all_storage_slots_times_loose_face_upper": capture.conservative_dynamic_faces_upper
            },
            "conservative_scene_faces_upper": capture.conservative_scene_faces_upper,
            "conservative_scene_triangles_upper": capture.conservative_scene_triangles_upper,
            "conservative_capture_payload_bytes_upper": capture.conservative_capture_payload_bytes_upper,
            "conservative_capture_replacement_peak_bytes_upper": capture.conservative_capture_peak_bytes_upper,
            "diagnostic_unqualified_capture_capacity_bytes": vertex_capacity_bytes(capture.conservative_capture_payload_bytes_upper)
        })
    });
    let vertex_known = capture_bounds.map(|capture| {
        let static_payload = capture
            .submitted_static
            .max_two_triangle_clipped_vertex_payload_bytes;
        let final_capacity = vertex_capacity_bytes(capture.admitted.vertex_payload_bytes_upper);
        let previous_capacity = final_capacity.map(previous_vertex_capacity_bytes);
        let replacement_peak = final_capacity.and_then(vertex_growth_peak_bytes);
        let static_capacity = vertex_capacity_bytes(static_payload);
        let device_support_for_unqualified_upper = final_capacity
            .zip(atlas_layout)
            .and_then(|(capacity, atlas)| {
                vertex_capacity_supported(capacity, atlas.max_storage_buffer_binding_size)
            });
        let legal_capacity = if vertex_content_upper_established {
            final_capacity
        } else {
            None
        };
        let device_support_for_legal_upper = legal_capacity
            .zip(atlas_layout)
            .and_then(|(capacity, atlas)| {
                vertex_capacity_supported(capacity, atlas.max_storage_buffer_binding_size)
            });
        json!({
            "classification": "conservative_unqualified_upper_not_an_achieved_scene",
            "capacity_derivation": "B_vertex = checked_next_power_of_two(max(1048576, content_upper_vertex_bytes))",
            "device_requirement": "Reject the device if max_storage_buffer_binding_size is below the full B_vertex capacity. Do not clamp B_vertex.",
            "minimum_capacity_bytes": VERTEX_BUFFER_MIN_BYTES,
            "legal_content_upper_qualified": vertex_content_upper_established,
            "legal_B_vertex_capacity_bytes": legal_capacity,
            "legal_B_vertex_replacement_peak_bytes": legal_capacity.and_then(vertex_growth_peak_bytes),
            "legal_device_supports_full_B_vertex": device_support_for_legal_upper,
            "device_selection_for_legal_B_vertex": match device_support_for_legal_upper {
                Some(true) => "supported",
                Some(false) => "reject_device",
                None => "unqualified_or_device_limit_unavailable",
            },
            "maximum_submitted_static_clipped_vertex_payload_bytes": static_payload,
            "maximum_submitted_static_next_power_of_two_capacity_bytes": static_capacity,
            "maximum_submitted_static_capacity_fits_requested_device_storage_limit": static_capacity
                .zip(atlas_layout)
                .and_then(|(capacity, atlas)| vertex_capacity_supported(capacity, atlas.max_storage_buffer_binding_size)),
            "admitted_geometry": {
                "classification": "sound_count_and_face_upper_not_a_reachability_claim",
                "shared_dynamic_count_upper": capture.admitted.shared_dynamic_count_upper,
                "dynamic_classes": capture.admitted.dynamic_classes,
                "dynamic_faces_upper": capture.admitted.dynamic_faces_upper,
                "static_faces_upper": capture.submitted_static.total_faces,
                "scene_faces_upper": capture.admitted.scene_faces_upper,
                "vertex_payload_bytes_upper": capture.admitted.vertex_payload_bytes_upper,
                "count_source_citations": [
                    "engine/src/network/rsbuf/build.ts:55-56 and info.ts:100-102,340-342 cap nearby players at 250 and NPCs at 255",
                    "render/draw.rs:852-868 submits the local player in addition to nearby players",
                    "core/world.rs:34,982-987 and render/world.rs:692-714 share 5000 admitted dynamic sprites among players, NPCs, projectiles, and map spots",
                ],
            },
            "diagnostic_unqualified_vertex_payload_bytes_upper": capture.admitted.vertex_payload_bytes_upper,
            "diagnostic_unqualified_B_vertex_capacity_bytes": final_capacity,
            "diagnostic_unqualified_previous_capacity_bytes": previous_capacity,
            "diagnostic_unqualified_replacement_peak_bytes": replacement_peak,
            "device_max_storage_buffer_binding_size": atlas_layout.map(|atlas| atlas.max_storage_buffer_binding_size),
            "diagnostic_unqualified_upper_fits_requested_device_storage_limit": device_support_for_unqualified_upper,
        })
    });
    let atlas_known = atlas_layout.map(|layout| {
        json!({
            "max_texture_dimension_2d": layout.max_texture_dimension_2d,
            "max_storage_buffer_binding_size": layout.max_storage_buffer_binding_size,
            "sprite_layer_count": layout.sprite_layer_count,
            "model_gpu_bytes": layout.model_gpu_bytes,
            "sprite_layer_gpu_bytes": layout.sprite_layer_gpu_bytes,
            "font_gpu_bytes": layout.font_gpu_bytes,
            "context_gpu_replacement_peak_bytes": layout.context_gpu_replacement_peak_bytes,
            "gpu_assets_inline_bytes": layout.gpu_assets_bytes,
            "gpu_atlas_inline_bytes": layout.gpu_atlas_bytes,
            "sprite_atlas_cpu_bytes": layout.sprite_atlas_cpu_bytes,
            "font_free_region_bytes": layout.font_free_bytes,
            "region_cache_bytes": layout.region_cache_bytes,
            "model_mip_peak_scratch_bytes": layout.model_mip_peak_scratch_bytes,
        })
    });
    let shared_sprite_upper = shared_store_layout
        .zip(shared_index_layout)
        .zip(item_sprite_ids)
        .and_then(|((store, index), item_count)| {
            let pixel_bytes = store
                .item_sprite_pixels
                .checked_mul(store.pix32_pixel_bytes)?;
            let payload_per_sprite = store
                .pix32_bytes
                .checked_add(pixel_bytes)?
                .checked_add(store.sprite_arc_header_bytes)?;
            let variant_payload = item_count.checked_mul(payload_per_sprite)?;
            let sentinel_payload = store.sprite_sentinel_count.checked_mul(
                store
                    .pix32_bytes
                    .checked_add(store.sprite_arc_header_bytes)?,
            )?;
            store
                .sprite_lru_capacity_bytes
                .checked_add(index.sprite_index_mutex_bytes)?
                .checked_add(index.sprite_index_capacity_bytes)?
                .checked_add(variant_payload)?
                .checked_add(sentinel_payload)
        });
    let shared_sprite_known = json!({
        "unique_object_type_ids_upper": item_sprite_ids,
        "store_layout": shared_store_layout.map(|layout| json!({
            "sprite_capacity": layout.sprite_capacity,
            "sprite_lru_capacity_bytes": layout.sprite_lru_capacity_bytes,
            "sprite_node_bytes": layout.sprite_node_bytes,
            "sprite_sentinel_count": layout.sprite_sentinel_count,
            "sprite_arc_header_bytes": layout.sprite_arc_header_bytes,
            "pix32_bytes": layout.pix32_bytes,
            "pixel_bytes": layout.pix32_pixel_bytes,
            "pixels_per_item_sprite": layout.item_sprite_pixels,
        })),
        "index_layout": shared_index_layout.map(|layout| json!({
            "mutex_bytes": layout.sprite_index_mutex_bytes,
            "entry_bytes": layout.sprite_index_entry_bytes,
            "capacity_bytes": layout.sprite_index_capacity_bytes,
            "orphan_bytes_per_dead_arc": layout.sprite_orphan_bytes_per_dead_arc,
        })),
        "source_derived_owner_bound": owner_bounds.map(|bounds| json!({
            "content_upper_bytes": bounds.shared_sprites.bytes,
            "known_measurements": bounds.shared_sprites.details,
        })),
        "all_item_sprite_payload_upper_bytes": shared_sprite_upper,
    });
    let rows = [
        (
            RenderResource::Capture,
            "Conservative unqualified upper only: independent 104x104 face maxima over every tile origin (not legal 8-aligned load origins) + all 5000 sprite arena storage slots × loose face-composition upper; × two clipped triangles × 3 vertices × CaptureVertex bytes, with old/new Vec replacement peak. Not an achieved scene or legal dynamic population bound.",
            capture_trigger.clone(),
            "A legal dynamic sprite population/topology maximum is not established; only a conservative storage-capacity composition is available.",
        ),
        (
            RenderResource::NavVertices,
            "30 vertices per eligible collision/path/trail quad + 6 per cardinal/corner/hull/click line + exact set-glyph and caption ink/fill vertices; include old/new Vec capacity",
            scene_trigger.clone(),
            "The pinned panel route, caption font, trail source, or private mesh layout is unavailable.",
        ),
        (
            RenderResource::NavQuads,
            "max projected collision + selected path + client trail quads × private Quad layout; include old/new Vec capacity",
            scene_trigger.clone(),
            "The pinned route/trail inputs or private Quad layout is unavailable.",
        ),
        (
            RenderResource::NavEdges,
            "four possible cardinal edge tuples per collision cell × private edge layout; corner strokes are vertex-only; include old/new Vec capacity",
            scene_trigger.clone(),
            "The pinned collision input or private edge layout is unavailable.",
        ),
        (
            RenderResource::NavInput,
            "collision*sizeof(NavDebugCell) + path*sizeof((i32,i32,bool)) + trail*sizeof((i32,i32,bool)) + hulls*sizeof(NavDebugHull) + labels*sizeof((i32,i32,String)) + max UTF-8 label bytes, with old/new retained paint overlap",
            scene_trigger.clone(),
            "The pinned route, caption text, font, or private input layouts are unavailable.",
        ),
        (
            RenderResource::HeadModels,
            "Conservative per-head upper: maximum resident static model bytes in one legal loaded scene + all dynamic sprite storage slots × a loose per-slot model-family upper + single-slot old/new replacement; not proof of a legal dynamic population.",
            loc_trigger.map(str::to_owned),
            "The complete per-head SceneModel composition and old/new replacement overlap are not bounded by current evidence.",
        ),
        (
            RenderResource::HeadMisc,
            "sum of RenderWorld storage, dynamic sprite arena storage capacity, overlays, chat/text buffers, nested Pix32 data and other owned per-head payloads; use the maximum old-plus-new replacement peak",
            scene_trigger.clone(),
            "Per-head overlays, dynamic nested payloads, and replacement overlap are not fully bounded.",
        ),
        (
            RenderResource::TempModels,
            "maximum simultaneously live transform/merge/sprite scratch: every loc/NPC/obj/IDK/spot/player family plus nested model vectors and old/new replacement overlap",
            model_trigger.map(str::to_owned),
            "The complete transform and composition scratch lifetime overlap is not bounded by current evidence.",
        ),
        (
            RenderResource::SharedModels,
            "all serialized raw models and transformed model families, plus model-store and weak-index capacities, Arc headers, sentinel owners, and aliases",
            Some(format!("{};{}", model_trigger.unwrap_or("no-model"), loc_trigger.unwrap_or("no-loc"))),
            "Per-config transform-cache node capacities and every admissible transformed model family are not fully inventoried.",
        ),
        (
            RenderResource::SharedSprites,
            "all admitted item-type-keyed 32×32 sprite payloads plus full shared-sprite LRU/index capacities, Arc headers and sentinel owners",
            model_trigger.map(str::to_owned),
            "The complete unique object-type count or shared store/index layouts are unavailable.",
        ),
        (
            RenderResource::Vertex,
            "B_vertex = checked_next_power_of_two(max(1048576, legal content_upper_vertex_bytes)). Reject a device whose max_storage_buffer_binding_size is below the full B_vertex capacity. Do not clamp B_vertex.",
            capture_trigger.clone(),
            "The legal vertex content upper, checked power-of-two capacity, or requested device storage limit is unavailable.",
        ),
        (
            RenderResource::ContextCpu,
            "maximum CPU-owned GPU-context atlas, font/map staging, mip-generation scratch and retained atlas-copy capacities across all admissible media content",
            None,
            "CPU atlas replacement overlap and all staging-owner capacities are not fully bounded.",
        ),
        (
            RenderResource::ContextGpu,
            "model atlas mip chain + maximum sprite/font atlas allocations at the requested device dimension, including old/new texture replacement overlap",
            None,
            "The backend atlas layout or requested device dimension limit is unavailable.",
        ),
    ];
    rows
        .into_iter()
        .map(|(resource, formula, trigger, blocker)| {
            let (content_upper, measured, complete) = match resource {
                RenderResource::Capture => (
                    capture_bounds.map_or(Value::Null, |capture| json!(capture.conservative_capture_peak_bytes_upper)),
                    capture_known.clone().unwrap_or(Value::Null),
                    capture_bounds.is_some(),
                ),
                RenderResource::NavVertices => (
                    nav_bounds.map_or(Value::Null, |nav| json!(nav.nav_vertices_peak_bytes)),
                    nav_known.clone().unwrap_or(Value::Null),
                    nav_bounds.is_some(),
                ),
                RenderResource::NavQuads => (
                    nav_bounds.map_or(Value::Null, |nav| json!(nav.nav_quads_peak_bytes)),
                    nav_known.clone().unwrap_or(Value::Null),
                    nav_bounds.is_some(),
                ),
                RenderResource::NavEdges => (
                    nav_bounds.map_or(Value::Null, |nav| json!(nav.nav_edges_peak_bytes)),
                    nav_known.clone().unwrap_or(Value::Null),
                    nav_bounds.is_some(),
                ),
                RenderResource::NavInput => (
                    nav_bounds.map_or(Value::Null, |nav| json!(nav.nav_input_replacement_peak_bytes)),
                    nav_known.clone().unwrap_or(Value::Null),
                    nav_bounds.is_some(),
                ),
                RenderResource::HeadModels => (
                    owner_bounds.map_or(Value::Null, |bounds| json!(bounds.head_models.bytes)),
                    owner_bounds.map_or_else(
                        || json!({
                            "largest_available_family_id": trigger.as_deref(),
                            "base_model_max_retained_bytes": families.get("base_models").map(|family| family.maximum_bytes),
                            "loc_family_max_retained_bytes": families.get("loc_models").map(|family| family.maximum_bytes),
                        }),
                        |bounds| bounds.head_models.details.clone(),
                    ),
                    owner_bounds.is_some(),
                ),
                RenderResource::HeadMisc => (
                    owner_bounds.map_or(Value::Null, |bounds| json!(bounds.head_misc.bytes)),
                    owner_bounds.map_or(Value::Null, |bounds| bounds.head_misc.details.clone()),
                    owner_bounds.is_some(),
                ),
                RenderResource::TempModels => (
                    owner_bounds.map_or(Value::Null, |bounds| json!(bounds.temp_models.bytes)),
                    owner_bounds.map_or_else(
                        || json!({
                            "largest_available_family_id": trigger.as_deref(),
                            "base_model_max_retained_bytes": families.get("base_models").map(|family| family.maximum_bytes),
                            "loc_family_max_retained_bytes": families.get("loc_models").map(|family| family.maximum_bytes),
                        }),
                        |bounds| bounds.temp_models.details.clone(),
                    ),
                    owner_bounds.is_some(),
                ),
                RenderResource::SharedModels => (
                    owner_bounds.map_or(Value::Null, |bounds| json!(bounds.shared_models.bytes)),
                    shared_known.clone(),
                    owner_bounds.is_some(),
                ),
                RenderResource::SharedSprites => (
                    owner_bounds.map_or(Value::Null, |bounds| json!(bounds.shared_sprites.bytes)),
                    shared_sprite_known.clone(),
                    owner_bounds.is_some(),
                ),
                RenderResource::Vertex => (
                    if vertex_content_upper_established {
                        capture_bounds.map_or(Value::Null, |capture| {
                            json!(capture.admitted.vertex_payload_bytes_upper)
                        })
                    } else {
                        Value::Null
                    },
                    vertex_known.clone().unwrap_or(Value::Null),
                    vertex_content_upper_established && vertex_known.is_some(),
                ),
                RenderResource::VertexCoordinates => {
                    unreachable!("fixed coordinate range is not a byte-budget census row")
                }
                RenderResource::ContextCpu => (
                    owner_bounds.map_or(Value::Null, |bounds| json!(bounds.context_cpu.bytes)),
                    json!({
                        "owner_bound": owner_bounds.map(|bounds| bounds.context_cpu.details.clone()),
                        "atlas_layout": atlas_known,
                    }),
                    owner_bounds.is_some(),
                ),
                RenderResource::ContextGpu => (
                    atlas_layout.map_or(Value::Null, |atlas| json!(atlas.context_gpu_replacement_peak_bytes)),
                    atlas_known.clone().unwrap_or(Value::Null),
                    atlas_layout.is_some(),
                ),
            };
            let alignment = resource_alignment(resource);
            let budget_bytes = if resource == RenderResource::Vertex {
                legal_vertex_capacity.and_then(|capacity| u64::try_from(capacity).ok())
            } else {
                content_upper
                    .as_u64()
                    .and_then(|bytes| bytes.checked_next_multiple_of(alignment as u64))
            };
            let charge_limit_bytes = if resource == RenderResource::Vertex {
                legal_vertex_capacity
                    .and_then(vertex_growth_peak_bytes)
                    .and_then(|bytes| u64::try_from(bytes).ok())
            } else {
                budget_bytes
            };
            let complete = complete
                && budget_bytes.is_some()
                && (resource != RenderResource::Vertex || vertex_device_support == Some(true));
            json!({
                "resource": format!("{resource:?}"),
                "display": resource.to_string(),
                "formula": formula,
                "content_upper_bytes": content_upper,
                "allocation_or_device_alignment_bytes": alignment,
                "B_bytes": budget_bytes,
                "charge_limit_bytes": charge_limit_bytes,
                "budget_derivation": if resource == RenderResource::Vertex {
                    "B_vertex = checked_next_power_of_two(max(1048576, legal content_upper_vertex_bytes)); reject devices below the full storage-buffer binding size"
                } else {
                    "align_up(content_upper_bytes, allocation_or_device_alignment_bytes)"
                },
                "known_measurements": measured,
                "largest_triggering_content_id": trigger,
                "status": if resource == RenderResource::Vertex && vertex_device_rejected {
                    "unsupported_device"
                } else if complete {
                    "complete"
                } else {
                    "incomplete"
                },
                "blockers": if complete {
                    Vec::<Value>::new()
                } else if resource == RenderResource::Vertex && vertex_device_rejected {
                    vec![json!({
                        "code": "device_cannot_support_full_B_vertex",
                        "required": "Reject this device because max_storage_buffer_binding_size is below the full derived B_vertex capacity.",
                        "required_capacity_bytes": legal_vertex_capacity,
                        "device_max_storage_buffer_binding_size": atlas_layout.map(|atlas| atlas.max_storage_buffer_binding_size),
                    })]
                } else if resource == RenderResource::Vertex && !vertex_content_upper_established {
                    vec![json!({
                        "code": "legal_vertex_content_upper_unqualified",
                        "required": blocker,
                    })]
                } else if resource == RenderResource::Vertex {
                    vec![json!({
                        "code": "vertex_capacity_or_device_limit_unavailable",
                        "required": blocker,
                    })]
                } else {
                    vec![json!({"code": "private_or_unbounded_owner_input", "required": blocker})]
                },
            })
        })
        .collect()
}

#[test]
#[ignore = "manual full-content render budget census; requires the fixed revision-289 cache and engine content tree"]
fn render_budget_content_census() {
    let _serial = super::serial();
    let mut inputs = InputLedger::default();
    let source_inputs = add_source_hashes(&mut inputs);
    let legal_population = legal_population_receipt(&mut inputs, &source_inputs);
    let model_cache_layouts = ModelCacheLayouts::load();
    if model_cache_layouts.is_none() {
        inputs.fail(
            "model_cache_layout_unavailable",
            "one or more production model caches could not be measured",
        );
    }

    let cache_dir = client::cache_dir();
    if super::assets().is_none() {
        inputs.fail(
            "fixed_cache_missing",
            format!("{} has no media jag", cache_dir.display()),
        );
    }
    let cache_dir_text = cache_dir.to_str();
    let unpack_dir = client::unpack_dir();
    let unpack_dir_text = unpack_dir.to_str();
    let mut selected_version = None;
    let mut manifest = None;
    let mut snapshot_dir = None;
    let mut model_total_bytes = None;
    let mut families = BTreeMap::<String, FamilyMaximum>::new();
    let mut map_rows = Vec::<MapIndexEntry>::new();
    let mut map_entities = Vec::<EntityPlacement>::new();
    let mut terrain_cells = Vec::<TerrainCell>::new();
    let mut map_stats = BTreeMap::<String, Value>::new();
    let mut scene_maxima = BTreeMap::<String, SceneMaximum>::new();
    let mut submitted_static_geometry = None;
    let world_layout = client::render::world::render_test_content_layout();
    let mut model_record_count = None;
    let mut model_parts = None;
    let mut dynamic_face_uppers = None;
    let mut item_sprite_ids = None;
    let mut anim_record_count = None;

    let mut title_bytes = None;
    let mut media_bytes = None;
    let mut textures_bytes = None;
    let mut interface_bytes = None;
    for jag in CLIENT_JAGS {
        let bytes = inputs.read(&format!("cache:{jag}"), &cache_dir.join(jag));
        match jag {
            "title" => title_bytes = bytes,
            "media" => media_bytes = bytes,
            "textures" => textures_bytes = bytes,
            "interface" => interface_bytes = bytes,
            _ => {}
        }
    }
    let nav_font =
        title_bytes.and_then(|bytes| PixFont::depack(&JagFile::new(bytes), "b12_full", false));
    if nav_font.is_none() {
        inputs.fail(
            "nav_caption_font_missing",
            "fixed title JAG has no readable b12_full font",
        );
    }
    let interface_graphic_keys =
        interface_bytes
            .as_deref()
            .and_then(|bytes| match interface_graphic_keys(bytes) {
                Ok(keys) => Some(keys),
                Err(error) => {
                    inputs.fail("interface_graphic_inventory_failed", error);
                    None
                }
            });
    let media_file_bytes = media_bytes.as_ref().map(Vec::len);
    if media_file_bytes.is_none() {
        inputs.fail("media_jag_missing", "fixed media JAG bytes are unavailable");
    }
    drop(media_bytes);
    drop(interface_bytes);
    let textures_jag = textures_bytes.map(JagFile::new);
    if textures_jag.is_none() {
        inputs.fail(
            "textures_jag_missing",
            "fixed textures JAG bytes are unavailable",
        );
    }
    let process_media = cache_dir_text.map(client::render::media::Media::process);
    if process_media.is_none() {
        inputs.fail(
            "media_process_path_unavailable",
            "cache directory is not UTF-8",
        );
    }
    let renderer_layout = process_media.as_deref().and_then(|media| {
        client::render::renderer::render_test_content_layout(media, textures_jag.as_ref())
    });
    if renderer_layout.is_none() {
        inputs.fail(
            "renderer_content_layout_unavailable",
            "renderer, title-flame, or texture storage layout is unavailable",
        );
    }
    let graphic_sprite_layout = cache_dir_text
        .zip(interface_graphic_keys.as_deref())
        .and_then(|(cache_dir, keys)| {
            client::render::renderer::render_test_graphic_sprite_layout(cache_dir, keys)
        });
    if graphic_sprite_layout.is_none() {
        inputs.fail(
            "graphic_sprite_layout_unavailable",
            "fixed interface/media content did not produce a complete graphics cache layout",
        );
    }
    let process_media_bytes = process_media
        .as_ref()
        .map(|_| client::render::media::Media::process_storage_bytes());
    let versionlist_bytes =
        inputs.read("cache:versionlist:selected", &cache_dir.join("versionlist"));
    if let Some(cache_dir_text) = cache_dir_text {
        match client::unpack::selected_version(cache_dir_text) {
            Ok(version) => {
                if version != R289_VERSION {
                    inputs.fail(
                        "wrong_fixed_revision",
                        format!(
                            "selected cache version {version} differs from pinned {R289_VERSION}"
                        ),
                    );
                }
                selected_version = Some(version);
            }
            Err(error) => inputs.fail("cache_version_unavailable", error.to_string()),
        }
    } else {
        inputs.fail("non_utf8_cache_path", cache_dir.display().to_string());
    }

    let expected_snapshot = unpack_dir.join(R289_VERSION);
    for name in [
        "manifest",
        "models.bin",
        "anims.bin",
        "midi.bin",
        "maps.bin",
    ]
    .into_iter()
    .chain(CLIENT_JAGS)
    {
        inputs.read(&format!("snapshot:{name}"), &expected_snapshot.join(name));
    }
    // Legacy completed snapshots have no integrity marker. Readiness still
    // validates their manifest; this census hashes every payload independently.
    let integrity = expected_snapshot.join("integrity");
    if integrity.is_file() {
        inputs.read("snapshot:integrity", &integrity);
    }

    if let (Some(cache_dir_text), Some(unpack_dir_text)) = (cache_dir_text, unpack_dir_text) {
        match client::unpack::snapshot_state(cache_dir_text, unpack_dir_text) {
            Ok(client::unpack::SnapshotState::Ready(found)) => {
                if found.version != R289_VERSION || !found.complete {
                    inputs.fail(
                        "snapshot_not_pinned_complete",
                        format!(
                            "snapshot version={} complete={}",
                            found.version, found.complete
                        ),
                    );
                }
                if found.models.total != found.models.unpacked + found.models.skipped
                    || found.maps.total != found.maps.unpacked + found.maps.skipped
                {
                    inputs.fail(
                        "snapshot_census_mismatch",
                        "manifest record totals do not reconcile",
                    );
                }
                manifest = Some(found.clone());
                snapshot_dir = Some(expected_snapshot.clone());
                match client::unpack::load_snapshot(cache_dir_text, unpack_dir_text) {
                    Ok(loaded) => {
                        model_record_count = Some(loaded.models);
                        anim_record_count = Some(loaded.anim_records);
                        if loaded.models != found.models.unpacked as usize {
                            inputs.fail(
                                "model_record_count_mismatch",
                                format!(
                                    "loaded {} model records, manifest declares {}",
                                    loaded.models, found.models.unpacked
                                ),
                            );
                        }
                        if loaded.anim_records != found.anims.unpacked as usize {
                            inputs.fail(
                                "animation_record_count_mismatch",
                                format!(
                                    "loaded {} animation records, manifest declares {}",
                                    loaded.anim_records, found.anims.unpacked
                                ),
                            );
                        }
                    }
                    Err(error) => inputs.fail("snapshot_load_failed", error.to_string()),
                }
            }
            Ok(state) => inputs.fail("snapshot_incomplete", state.describe()),
            Err(error) => inputs.fail("snapshot_state_unavailable", error.to_string()),
        }
    }

    let versionlist_jag = versionlist_bytes.map(JagFile::new);
    let map_index_bytes = versionlist_jag
        .as_ref()
        .and_then(|jag| jag.read("map_index"));
    if map_index_bytes.is_none() {
        inputs.fail(
            "map_index_missing",
            "fixed versionlist has no readable map_index member",
        );
    } else if let Some(bytes) = map_index_bytes.as_deref() {
        match map_cache::decode_map_index(bytes) {
            Ok(entries) => map_rows = entries,
            Err(error) => inputs.fail("map_index_invalid", error.to_string()),
        }
    }

    let mut model_bytes = None;
    let mut maps_bytes = None;
    let mut config_bytes = None;
    if let Some(snapshot_dir) = &snapshot_dir {
        model_bytes = inputs.read(
            "snapshot:models.bin:records",
            &snapshot_dir.join("models.bin"),
        );
        maps_bytes = inputs.read("snapshot:maps.bin:records", &snapshot_dir.join("maps.bin"));
        config_bytes = inputs.read("snapshot:config:jag", &snapshot_dir.join("config"));
    }
    match load_map_census(
        &mut inputs,
        &map_rows,
        maps_bytes.as_deref(),
        manifest.as_ref().map(|found| found.maps.unpacked as usize),
    ) {
        Ok(census) => {
            map_entities = census.entities;
            terrain_cells = census.terrain_cells;
            map_stats = census.stats;
        }
        Err(error) => inputs.fail("map_source_inventory_failed", error),
    }

    if let Some(bytes) = model_bytes.as_deref() {
        match decode_record_stream(bytes, "models.bin") {
            Ok(records) => {
                model_record_count = Some(records.len());
                if let Some(found) = &manifest {
                    if records.len() != found.models.unpacked as usize {
                        inputs.fail(
                            "model_stream_count_mismatch",
                            format!(
                                "models.bin has {} records, manifest declares {}",
                                records.len(),
                                found.models.unpacked
                            ),
                        );
                    }
                }
                let mut total = 0usize;
                let mut base = FamilyMaximum::default();
                for record in records {
                    let id = match i32::try_from(record.id) {
                        Ok(id) => id,
                        Err(_) => {
                            inputs.fail("model_id_out_of_range", record.id.to_string());
                            continue;
                        }
                    };
                    if let Some(model) = Model::load(id) {
                        if let Err(error) = base.record(&model, format!("model:{id}")) {
                            inputs.fail("model_measurement_failed", error);
                        }
                        total = match total.checked_add(model.retained_bytes()) {
                            Some(total) => total,
                            None => {
                                inputs.fail("model_total_overflow", format!("model:{id}"));
                                total
                            }
                        };
                    } else {
                        inputs.fail("serialized_model_not_loadable", format!("model:{id}"));
                    }
                }
                model_total_bytes = Some(total);
                families.insert("base_models".to_string(), base);
            }
            Err(error) => inputs.fail("model_stream_invalid", error),
        }
    }

    let config_cache = config_bytes.map(|bytes| Cache::unpack(&JagFile::new(bytes)));
    if let Some(cache) = &config_cache {
        let required_tables = [
            ("objs", cache.objs.len()),
            ("npcs", cache.npcs.len()),
            ("locs", cache.locs.len()),
            ("idks", cache.idks.len()),
            ("seqs", cache.seqs.len()),
            ("spots", cache.spots.len()),
        ];
        if required_tables.iter().any(|(_, count)| *count == 0) {
            inputs.fail(
                "config_table_missing",
                format!("required model config tables are not all populated: {required_tables:?}"),
            );
        }
        item_sprite_ids = Some(cache.objs.len());
        model_parts = model_cache_layouts
            .and_then(|layouts| model_source_parts(cache, layouts.player.appearance_slots));
        if model_parts.is_none() {
            inputs.fail(
                "model_source_parts_unavailable",
                "complete loc/NPC/obj/IDK/player source-part layouts are unavailable",
            );
        }
        let mut locs = FamilyMaximum::default();
        let mut npcs = FamilyMaximum::default();
        let mut objs = FamilyMaximum::default();
        let mut idks = FamilyMaximum::default();
        let mut spots = FamilyMaximum::default();
        let mut loc_face_max = FaceMaximum::default();
        let mut npc_face_max = FaceMaximum::default();
        let mut spot_face_max = FaceMaximum::default();
        let mut player_slot_maxima = std::array::from_fn(|_| empty_player_slot_maxima());
        let mut male_body_kit_parts = [None; 7];
        let mut object_wear_face_maxima = std::array::from_fn(|_| FaceMaximum::default());
        let mut object_wear_face_counts: [Vec<Option<usize>>; 2] =
            std::array::from_fn(|_| vec![None; cache.objs.len()]);
        let mut object_wear_model_lists_load: [Vec<bool>; 2] =
            std::array::from_fn(|_| vec![true; cache.objs.len()]);
        let mut idk_face_counts = vec![None; cache.idks.len()];
        let mut idk_model_lists_load = vec![true; cache.idks.len()];
        let mut sequence_hand_replacements = Vec::<Value>::new();
        let mut dynamic_face_upper_complete = true;
        let rotations = [
            LocAngle::WEST,
            LocAngle::NORTH,
            LocAngle::EAST,
            LocAngle::SOUTH,
        ];

        for loc in &cache.locs {
            let shapes = match loc.shape.as_deref() {
                Some(shapes) => shapes.to_vec(),
                None => (0..LOC_SHAPE_TO_LAYER.len())
                    .filter_map(|shape| i32::try_from(shape).ok())
                    .collect(),
            };
            for shape in shapes {
                for rotation in rotations {
                    let trigger = format!("loc:{}:shape:{shape}:rotation:{rotation}", loc.id);
                    let declares_model = loc_declares_model_for_shape(loc, shape);
                    let Some(model) = loc.get_model_shared(cache, shape, rotation, 0, 0, 0, 0, -1)
                    else {
                        if declares_model {
                            inputs.fail(
                                "dynamic_temporary_loc_model_unavailable",
                                format!("{trigger}: configured source model could not be composed"),
                            );
                            dynamic_face_upper_complete = false;
                        }
                        continue;
                    };
                    if let Err(error) = locs.record(&model, trigger.clone()) {
                        inputs.fail("loc_family_measurement_failed", error);
                    }
                    if let Err(error) = loc_face_max.record_model(&model, trigger) {
                        inputs.fail("dynamic_temporary_loc_face_measurement_failed", error);
                        dynamic_face_upper_complete = false;
                    }
                }
            }
        }
        for npc in &cache.npcs {
            let model_list_load = npc.model.as_deref().is_none_or(configured_model_ids_load);
            if !model_list_load {
                inputs.fail(
                    "dynamic_npc_model_source_unavailable",
                    format!(
                        "npc:{}: one or more configured composite model parts are unavailable",
                        npc.id
                    ),
                );
                dynamic_face_upper_complete = false;
            }
            if let Some(model) = npc.get_temp_model(cache, -1, -1, None) {
                let trigger = format!("npc:{}", npc.id);
                if let Err(error) = npcs.record(&model, trigger.clone()) {
                    inputs.fail("npc_family_measurement_failed", error);
                }
                if let Err(error) = npc_face_max.record_model(&model, trigger) {
                    inputs.fail("dynamic_npc_face_measurement_failed", error);
                    dynamic_face_upper_complete = false;
                }
            } else if npc.model.is_some() {
                inputs.fail(
                    "dynamic_npc_composite_unavailable",
                    format!(
                        "npc:{}: configured body composite could not be loaded",
                        npc.id
                    ),
                );
                dynamic_face_upper_complete = false;
            }
        }
        for (obj_index, obj) in cache.objs.iter().enumerate() {
            let mut counts = BTreeSet::from([1i32]);
            if let Some(thresholds) = &obj.countco {
                for threshold in thresholds {
                    if *threshold > 1 {
                        counts.insert(i32::from(*threshold));
                    }
                }
            }
            for count in counts {
                if let Some(model) = obj.get_model_unlit(cache, count) {
                    if let Err(error) =
                        objs.record(&model, format!("obj:{}:unlit-count:{count}", obj.id))
                    {
                        inputs.fail("obj_family_measurement_failed", error);
                    }
                }
                if let Some(model) = obj.get_model_lit(cache, count) {
                    if let Err(error) =
                        objs.record(&model, format!("obj:{}:lit-count:{count}", obj.id))
                    {
                        inputs.fail("obj_family_measurement_failed", error);
                    }
                }
            }
            for gender in 0..=1usize {
                let (wear, wear2, wear3) = if gender == 0 {
                    (obj.manwear, obj.manwear2, obj.manwear3)
                } else {
                    (obj.womanwear, obj.womanwear2, obj.womanwear3)
                };
                if wear != -1 {
                    let ids = [wear, wear2, wear3];
                    let count = if wear2 == -1 {
                        1
                    } else if wear3 == -1 {
                        2
                    } else {
                        3
                    };
                    object_wear_model_lists_load[gender][obj_index] =
                        configured_model_ids_load(&ids[..count]);
                    if !object_wear_model_lists_load[gender][obj_index] {
                        inputs.fail(
                            "dynamic_player_object_wear_source_unavailable",
                            format!("obj:{}: one or more gender {gender} WEAR model parts are unavailable", obj.id),
                        );
                        dynamic_face_upper_complete = false;
                    }
                }
                let trigger = format!("obj:{}:wear-gender:{gender}", obj.id);
                if let Some(model) = obj.get_wear_model_no_check(gender as i32) {
                    if let Err(error) = objs.record(&model, trigger.clone()) {
                        inputs.fail("obj_family_measurement_failed", error);
                    }
                    match usize::try_from(model.num_faces) {
                        Ok(faces) => {
                            object_wear_face_counts[gender][obj_index] = Some(faces);
                            object_wear_face_maxima[gender].record_faces(faces, trigger);
                        }
                        Err(_) => {
                            inputs.fail(
                                "dynamic_player_object_wear_face_measurement_failed",
                                format!(
                                    "{trigger}: negative or unrepresentable face count {}",
                                    model.num_faces
                                ),
                            );
                            dynamic_face_upper_complete = false;
                        }
                    }
                } else if wear != -1 {
                    object_wear_model_lists_load[gender][obj_index] = false;
                    inputs.fail(
                        "dynamic_player_object_wear_composite_unavailable",
                        format!("{trigger}: configured WEAR composite could not be loaded"),
                    );
                    dynamic_face_upper_complete = false;
                }
                if let Some(model) = obj.get_head_model_no_check(gender as i32) {
                    if let Err(error) =
                        objs.record(&model, format!("obj:{}:head-gender:{gender}", obj.id))
                    {
                        inputs.fail("obj_family_measurement_failed", error);
                    }
                }
            }
        }
        for (id, idk) in cache.idks.iter().enumerate() {
            let model_list_load = idk.model.as_deref().is_none_or(configured_model_ids_load);
            idk_model_lists_load[id] = model_list_load;
            let body_gender_slot = match idk.part {
                0..=6 => Some((0, idk.part as usize)),
                7..=13 => Some((1, (idk.part - 7) as usize)),
                _ => None,
            }
            .filter(|_| !idk.disable && id < 256);
            if body_gender_slot.is_some() && idk.model.is_some() && !model_list_load {
                inputs.fail(
                    "dynamic_player_idk_body_source_unavailable",
                    format!("idk:{id}: one or more configured BODY model parts are unavailable"),
                );
                dynamic_face_upper_complete = false;
            }
            if let Some(model) = idk.get_model_no_check() {
                if let Err(error) = idks.record(&model, format!("idk:{id}:body")) {
                    inputs.fail("idk_family_measurement_failed", error);
                }
                if model_list_load {
                    match usize::try_from(model.num_faces) {
                        Ok(faces) => idk_face_counts[id] = Some(faces),
                        Err(_) if body_gender_slot.is_some() => {
                            inputs.fail(
                                "dynamic_player_idk_body_face_measurement_failed",
                                format!(
                                    "idk:{id}: negative or unrepresentable face count {}",
                                    model.num_faces
                                ),
                            );
                            dynamic_face_upper_complete = false;
                        }
                        Err(_) => {}
                    }
                }
                if let Some((gender, body_part)) = body_gender_slot {
                    if let Some(faces) = idk_face_counts[id] {
                        let slot = PLAYER_IDK_BODY_SLOT_BY_PART[body_part];
                        player_slot_maxima[gender][slot].record_faces(
                            faces,
                            format!("idk:{id}:body:part:{}:gender:{gender}", idk.part),
                        );
                    } else if idk.model.is_some() {
                        inputs.fail(
                            "dynamic_player_idk_body_composite_unavailable",
                            format!("idk:{id}: configured BODY composite could not be measured"),
                        );
                        dynamic_face_upper_complete = false;
                    }
                }
            } else if body_gender_slot.is_some() && idk.model.is_some() {
                inputs.fail(
                    "dynamic_player_idk_body_composite_unavailable",
                    format!("idk:{id}: configured BODY composite could not be loaded"),
                );
                dynamic_face_upper_complete = false;
            }
            if let Some((0, body_part)) = body_gender_slot {
                if male_body_kit_parts[body_part].is_none() {
                    male_body_kit_parts[body_part] =
                        Some((id, idk_face_counts[id].unwrap_or_default()));
                }
            }
            if let Some(model) = idk.get_head_no_check() {
                if let Err(error) = idks.record(&model, format!("idk:{id}:head")) {
                    inputs.fail("idk_family_measurement_failed", error);
                }
            }
        }
        for spot in &cache.spots {
            let trigger = format!("spot:{}", spot.id);
            if let Some(model) = spot.get_temp_model2(cache) {
                if let Err(error) = spots.record(&model, trigger.clone()) {
                    inputs.fail("spot_family_measurement_failed", error);
                }
                if let Err(error) = spot_face_max.record_model(&model, trigger) {
                    inputs.fail("dynamic_spot_face_measurement_failed", error);
                    dynamic_face_upper_complete = false;
                }
            } else {
                inputs.fail(
                    "dynamic_spot_model_unavailable",
                    format!("{trigger}: configured projectile/map/player spot model could not be loaded"),
                );
                dynamic_face_upper_complete = false;
            }
        }
        for gender in 0..=1 {
            for maximum in &mut player_slot_maxima[gender] {
                maximum.merge(&object_wear_face_maxima[gender]);
            }
        }
        if cache.seqs.is_empty() {
            inputs.fail(
                "dynamic_sequence_table_unavailable",
                "sequence table is empty, so hand replacement candidates cannot be enumerated",
            );
            dynamic_face_upper_complete = false;
        }
        for (sequence_id, sequence) in cache.seqs.iter().enumerate() {
            for (gender, slot_maxima) in player_slot_maxima.iter_mut().enumerate() {
                let gender_name = if gender == 0 { "male" } else { "female" };
                for (hand, slot, appearance_value) in [
                    ("right", 3usize, sequence.replaceheldright),
                    ("left", 5usize, sequence.replaceheldleft),
                ] {
                    if appearance_value < 0 {
                        continue;
                    }
                    match resolve_sequence_replacement_faces(
                        cache,
                        gender,
                        appearance_value,
                        &idk_face_counts,
                        &idk_model_lists_load,
                        &object_wear_face_counts,
                        &object_wear_model_lists_load,
                    ) {
                        Ok((faces, source_kind, source_id)) => {
                            let trigger = match source_id {
                                Some(source_id) => format!(
                                    "seq:{sequence_id}:replaceheld{hand}:value:{appearance_value}:{source_kind}:{source_id}:gender:{gender}"
                                ),
                                None => format!(
                                    "seq:{sequence_id}:replaceheld{hand}:value:{appearance_value}:empty"
                                ),
                            };
                            slot_maxima[slot].record_faces(faces, trigger.clone());
                            sequence_hand_replacements.push(json!({
                                "gender": gender_name,
                                "appearance_slot": slot,
                                "hand": hand,
                                "sequence_id": sequence_id,
                                "appearance_value": appearance_value,
                                "source_kind": source_kind,
                                "source_id": source_id,
                                "max_faces": faces,
                                "trigger": trigger,
                            }));
                        }
                        Err(error) => {
                            inputs.fail(
                                "dynamic_sequence_hand_replacement_unavailable",
                                format!("seq:{sequence_id}:replaceheld{hand}: {error}"),
                            );
                            dynamic_face_upper_complete = false;
                        }
                    }
                }
            }
        }
        for (source, maximum) in [
            ("NPC composite", &npc_face_max),
            ("spot model", &spot_face_max),
            ("temporary loc model", &loc_face_max),
        ] {
            if maximum.trigger.is_none() {
                inputs.fail(
                    "dynamic_face_upper_source_unavailable",
                    format!("no complete {source} face maximum was measured"),
                );
                dynamic_face_upper_complete = false;
            }
        }
        let male_body_kit_case = match male_idk_body_kit_case(cache, male_body_kit_parts) {
            Ok(case) => Some(case),
            Err(error) => {
                inputs.fail("dynamic_player_legal_body_kit_case_unavailable", error);
                dynamic_face_upper_complete = false;
                None
            }
        };
        if dynamic_face_upper_complete {
            match male_body_kit_case {
                Some(male_body_kit_case) => match dynamic_face_uppers_json(
                    player_slot_maxima,
                    object_wear_face_maxima,
                    &npc_face_max,
                    &spot_face_max,
                    &loc_face_max,
                    sequence_hand_replacements,
                    male_body_kit_case,
                ) {
                    Ok(uppers) => dynamic_face_uppers = Some(uppers),
                    Err(error) => inputs.fail("dynamic_face_upper_aggregation_failed", error),
                },
                None => {
                    inputs.fail(
                        "dynamic_player_legal_body_kit_case_unavailable",
                        "no serialized legal IDK body kit was available",
                    );
                }
            }
        }
        families.insert("loc_models".to_string(), locs);
        families.insert("npc_models".to_string(), npcs);
        families.insert("obj_models".to_string(), objs);
        families.insert("idk_models".to_string(), idks);
        families.insert("spot_models".to_string(), spots);

        let mut loc_cache = BTreeMap::<(u32, u8, u8), Option<ModelMeasure>>::new();
        let mut npc_cache = BTreeMap::<u32, Option<ModelMeasure>>::new();
        let mut obj_cache = BTreeMap::<(u32, u32), Option<ModelMeasure>>::new();
        let mut nonrenderable_rows = 0usize;
        for placement in &mut map_entities {
            let measured = match placement.kind {
                EntityKind::Loc => {
                    let key = (placement.id, placement.shape, placement.rotation);
                    if let Some(measured) = loc_cache.get(&key) {
                        Ok(*measured)
                    } else {
                        let measured = placement_model_measure(cache, placement);
                        if let Ok(value) = measured.as_ref() {
                            loc_cache.insert(key, *value);
                        }
                        measured
                    }
                }
                EntityKind::Npc => {
                    if let Some(measured) = npc_cache.get(&placement.id) {
                        Ok(*measured)
                    } else {
                        let measured = placement_model_measure(cache, placement);
                        if let Ok(value) = measured.as_ref() {
                            npc_cache.insert(placement.id, *value);
                        }
                        measured
                    }
                }
                EntityKind::Obj => {
                    let key = (placement.id, placement.amount);
                    if let Some(measured) = obj_cache.get(&key) {
                        Ok(*measured)
                    } else {
                        let measured = placement_model_measure(cache, placement);
                        if let Ok(value) = measured.as_ref() {
                            obj_cache.insert(key, *value);
                        }
                        measured
                    }
                }
            };
            match measured {
                Ok(Some(measure)) => {
                    placement.model_resolved = true;
                    placement.model_bytes = measure.bytes;
                    placement.faces = measure.faces;
                    placement.points = measure.points;
                }
                Ok(None) => {
                    nonrenderable_rows = nonrenderable_rows.saturating_add(1);
                }
                Err(error) => inputs.fail("map_entity_model_resolution_failed", error),
            }
        }
        map_stats.insert(
            "nonrenderable_entity_rows".to_string(),
            json!(nonrenderable_rows),
        );
        match scene_census(&mut map_entities, &mut terrain_cells, world_layout) {
            Ok(SceneCensus {
                maxima,
                submitted_static,
                conservative_scene_faces,
            }) => {
                map_stats.insert(
                    "maximum_scene_entities".to_string(),
                    Value::Object(
                        maxima
                            .iter()
                            .map(|(name, maximum)| (name.clone(), scene_maximum_json(maximum)))
                            .collect(),
                    ),
                );
                map_stats.insert(
                    "conservative_any_tile_window_entity_face_upper".to_string(),
                    json!(conservative_scene_faces.0),
                );
                map_stats.insert(
                    "conservative_any_tile_window_entity_origin".to_string(),
                    json!([conservative_scene_faces.1, conservative_scene_faces.2]),
                );
                map_stats.insert(
                    "maximum_submitted_static_geometry".to_string(),
                    submitted_static.json(),
                );
                scene_maxima = maxima;
                submitted_static_geometry = Some(submitted_static);
            }
            Err(error) => inputs.fail("scene_census_failed", error),
        }
    } else {
        inputs.fail(
            "config_unavailable",
            "cannot enumerate transformed model families without the pinned config JAG",
        );
        inputs.fail(
            "dynamic_face_upper_unavailable",
            "cannot derive dynamic entity face uppers without the pinned config JAG",
        );
    }

    let nav_bounds = match nav_font.as_ref() {
        Some(font) => match nav_source_bounds(&source_inputs, font) {
            Ok(bounds) => Some(bounds),
            Err(error) => {
                inputs.fail("nav_content_bound_failed", error);
                None
            }
        },
        None => None,
    };
    let capture_bounds = match (submitted_static_geometry, dynamic_face_uppers.as_ref()) {
        (Some(submitted_static), Some(dynamic_faces)) => {
            match scene_capture_bounds(
                &families,
                &scene_maxima,
                &map_stats,
                submitted_static,
                &source_inputs,
                dynamic_faces,
            ) {
                Ok(bounds) => Some(bounds),
                Err(error) => {
                    inputs.fail("scene_capture_bound_failed", error);
                    None
                }
            }
        }
        _ => {
            inputs.fail(
                "scene_capture_bound_failed",
                "co-located submitted static geometry or complete dynamic face uppers are unavailable",
            );
            None
        }
    };
    let atlas_layout = GpuAssets::render_test_content_layout();
    if atlas_layout.is_none() {
        inputs.fail(
            "atlas_layout_unavailable",
            "GPU atlas layout overflowed or was unavailable",
        );
    }
    let shared_store_layout = model_record_count.and_then(render_test_shared_store_layout);
    if shared_store_layout.is_none() {
        inputs.fail(
            "shared_store_layout_unavailable",
            "raw model record count or bounded LRU fixture layout is unavailable",
        );
    }
    let transformed_model_entries = [
        "loc_models",
        "npc_models",
        "obj_models",
        "idk_models",
        "spot_models",
    ]
    .into_iter()
    .try_fold(0usize, |count, name| {
        count.checked_add(families.get(name)?.variants)
    });
    let model_index_entries = model_record_count
        .zip(transformed_model_entries)
        .and_then(|(raw, transformed)| raw.checked_add(transformed));
    let sprite_index_entries = item_sprite_ids;
    let shared_index_layout = model_index_entries
        .zip(sprite_index_entries)
        .and_then(|(models, sprites)| render_test_shared_index_layout(models, sprites));
    if shared_index_layout.is_none() {
        inputs.fail(
            "shared_index_layout_unavailable",
            "complete raw/transformed model counts or unique object-type sprite count are unavailable",
        );
    }
    let owner_bounds = match (
        model_cache_layouts,
        model_parts,
        renderer_layout,
        graphic_sprite_layout,
        atlas_layout,
        shared_store_layout,
        shared_index_layout,
        media_file_bytes,
        process_media_bytes,
        item_sprite_ids,
    ) {
        (
            Some(model_caches),
            Some(model_parts),
            Some(renderer),
            Some(graphic_sprites),
            Some(atlas),
            Some(shared_store),
            Some(shared_index),
            Some(media_file_bytes),
            Some(process_media_bytes),
            Some(item_sprite_ids),
        ) => match content_owner_bounds(
            &families,
            &scene_maxima,
            model_caches,
            model_parts,
            world_layout,
            renderer,
            graphic_sprites,
            atlas,
            shared_store,
            shared_index,
            media_file_bytes,
            process_media_bytes,
            item_sprite_ids,
        ) {
            Ok(bounds) => Some(bounds),
            Err(error) => {
                inputs.fail("content_owner_bound_failed", error);
                None
            }
        },
        _ => {
            inputs.fail(
                "content_owner_inputs_unavailable",
                "renderer, model cache, shared cache, media, atlas, or source layouts are incomplete",
            );
            None
        }
    };
    let legal_population_complete = legal_population
        .as_ref()
        .and_then(|population| {
            population.pointer("/qualification/dynamic_population_bound_established")
        })
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let legal_static_map_equivalence_established = legal_population
        .as_ref()
        .and_then(|population| {
            population
                .pointer("/static_map_content/source_map_to_server_pack_equivalence_established")
        })
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let legal_vertex_content_upper_established =
        legal_population_complete && legal_static_map_equivalence_established;
    let resources = resource_rows(
        &families,
        &scene_maxima,
        model_total_bytes,
        model_record_count,
        nav_bounds.as_ref(),
        capture_bounds.as_ref(),
        atlas_layout,
        shared_store_layout,
        shared_index_layout,
        item_sprite_ids,
        owner_bounds.as_ref(),
        legal_vertex_content_upper_established,
    );
    let complete = inputs.failures.is_empty()
        && legal_population_complete
        && legal_vertex_content_upper_established
        && resources.iter().all(|row| row["status"] == "complete");
    let receipt = json!({
        "schema": "T4.4-S2-S4-content-census-v1",
        "revision": 289,
        "fixed_cache_version": R289_VERSION,
        "selected_cache_version": selected_version,
        "snapshot_model_records": model_record_count,
        "snapshot_animation_records": anim_record_count,
        "snapshot_manifest": manifest.map(|found| json!({
            "source": found.source,
            "complete": found.complete,
            "models": {"total": found.models.total, "unpacked": found.models.unpacked, "skipped": found.models.skipped, "bytes": found.models.bytes},
            "anims": {"total": found.anims.total, "unpacked": found.anims.unpacked, "skipped": found.anims.skipped, "bytes": found.anims.bytes},
            "maps": {"total": found.maps.total, "unpacked": found.maps.unpacked, "skipped": found.maps.skipped, "bytes": found.maps.bytes},
        })),
        "inputs": inputs.hashes,
        "failures": inputs.failures,
        "model_families": families.iter().map(|(name, family)| (name.clone(), family.json())).collect::<BTreeMap<_, _>>(),
        "dynamic_face_uppers": dynamic_face_uppers,
        "model_total_retained_bytes": model_total_bytes,
        "map_inventory": map_stats,
        "legal_dynamic_population": legal_population,
        "legal_vertex_content_upper_established": legal_vertex_content_upper_established,
        "resources": resources,
        "complete": complete,
    });
    println!(
        "render_budget_content_census={}",
        serde_json::to_string_pretty(&receipt).expect("receipt serializes")
    );
    assert!(
        receipt["complete"] == true,
        "render budget content census is incomplete; see the JSON receipt above"
    );
}

const MAP_WORLD_TILES: usize = 256 * 64;
const MAP_LOAD_ORIGIN_STRIDE: usize = 8;
const SUBMITTED_SCENE_TILES: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LocIdentity {
    world_x: u16,
    world_z: u16,
    plane: u8,
    id: u32,
    shape: u8,
    rotation: u8,
}

fn loc_identity(entity: &EntityPlacement) -> LocIdentity {
    LocIdentity {
        world_x: entity.world_x,
        world_z: entity.world_z,
        plane: entity.plane,
        id: entity.id,
        shape: entity.shape,
        rotation: entity.rotation,
    }
}

struct MapCensus {
    entities: Vec<EntityPlacement>,
    terrain_cells: Vec<TerrainCell>,
    stats: BTreeMap<String, Value>,
}

fn load_map_census(
    inputs: &mut InputLedger,
    map_index: &[MapIndexEntry],
    map_archive_bytes: Option<&[u8]>,
    expected_archive_records: Option<usize>,
) -> Result<MapCensus, String> {
    let archive_bytes = map_archive_bytes.ok_or("pinned maps.bin is unavailable")?;
    let archive_records = decode_record_stream(archive_bytes, "maps.bin")?;
    if let Some(expected) = expected_archive_records {
        if archive_records.len() != expected {
            inputs.fail(
                "map_archive_record_count_mismatch",
                format!(
                    "maps.bin has {} records, manifest declares {expected}",
                    archive_records.len()
                ),
            );
        }
    }
    let records_by_id = archive_records
        .iter()
        .map(|record| (record.id, record.bytes))
        .collect::<BTreeMap<_, _>>();
    let mut referenced_record_ids = BTreeSet::new();
    let mut cache_land_cells = 0usize;
    let mut max_terrain_faces = 0usize;
    let mut terrain_trigger = None;
    let mut terrain_cells = Vec::new();
    let mut cache_locations = Vec::new();
    let expected_land_cells = usize::from(map_cache::MAP_PLANES)
        * usize::from(map_cache::MAP_SQUARE_SIZE)
        * usize::from(map_cache::MAP_SQUARE_SIZE);

    for entry in map_index {
        let land_id = u32::from(entry.land_file);
        let land = records_by_id
            .get(&land_id)
            .ok_or_else(|| format!("maps.bin has no land record {land_id}"))?;
        referenced_record_ids.insert(land_id);
        let mut cells_in_record = 0usize;
        let mut invalid_shape = None;
        let mut invalid_face_count = None;
        map_cache::visit_land(land, |cell| {
            cells_in_record += 1;
            let shape = if cell.overlay != 0 {
                Some(cell.overlay_shape.saturating_add(1))
            } else if cell.underlay != 0 {
                Some(0)
            } else {
                None
            };
            let Some(shape) = shape else {
                return;
            };
            let Some(faces) = client::dash3d::ground::render_test_face_count(shape) else {
                invalid_shape = Some((
                    entry.square_x,
                    entry.square_z,
                    cell.plane,
                    cell.x,
                    cell.z,
                    shape,
                ));
                return;
            };
            if faces > max_terrain_faces {
                max_terrain_faces = faces;
                terrain_trigger = Some(format!(
                    "terrain:map:{},{}:plane:{}:tile:{},{}:shape:{}",
                    entry.square_x, entry.square_z, cell.plane, cell.x, cell.z, shape
                ));
            }
            let face_count = match u8::try_from(faces) {
                Ok(face_count) => face_count,
                Err(_) => {
                    invalid_face_count = Some((
                        entry.square_x,
                        entry.square_z,
                        cell.plane,
                        cell.x,
                        cell.z,
                        shape,
                        faces,
                    ));
                    return;
                }
            };
            terrain_cells.push(TerrainCell {
                world_x: u16::from(entry.square_x) * 64 + u16::from(cell.x),
                world_z: u16::from(entry.square_z) * 64 + u16::from(cell.z),
                faces: face_count,
            });
        })
        .map_err(|error| format!("land record {land_id}: {error}"))?;
        if let Some((square_x, square_z, plane, x, z, shape, faces)) = invalid_face_count {
            return Err(format!(
                "land record {land_id} at map {square_x},{square_z} has unrepresentable face count {faces} for shape {shape} at {plane}:{x},{z}"
            ));
        }
        if let Some((square_x, square_z, plane, x, z, shape)) = invalid_shape {
            return Err(format!(
                "land record {land_id} at map {square_x},{square_z} has invalid overlay shape {shape} at {plane}:{x},{z}"
            ));
        }
        if cells_in_record != expected_land_cells {
            return Err(format!(
                "land record {land_id} has {cells_in_record} cells, expected {expected_land_cells}"
            ));
        }
        cache_land_cells = cache_land_cells
            .checked_add(cells_in_record)
            .ok_or_else(|| "total cache land cell count overflowed".to_string())?;

        let loc_id = u32::from(entry.loc_file);
        let locations = records_by_id
            .get(&loc_id)
            .ok_or_else(|| format!("maps.bin has no location record {loc_id}"))?;
        referenced_record_ids.insert(loc_id);
        map_cache::visit_locations(locations, |placement| {
            cache_locations.push(EntityPlacement {
                kind: EntityKind::Loc,
                id: placement.id,
                plane: placement.plane,
                world_x: u16::from(entry.square_x) * 64 + u16::from(placement.x),
                world_z: u16::from(entry.square_z) * 64 + u16::from(placement.z),
                shape: placement.shape,
                rotation: placement.rotation,
                amount: 1,
                model_resolved: false,
                model_bytes: 0,
                faces: 0,
                points: 0,
            });
        })
        .map_err(|error| format!("location record {loc_id}: {error}"))?;
    }
    let unreferenced_archive_records = archive_records.len() - referenced_record_ids.len();
    if unreferenced_archive_records != 0 {
        inputs.fail(
            "unreferenced_map_archive_records",
            format!("{unreferenced_archive_records} maps.bin records are not named by map_index"),
        );
    }

    let scene_size = usize::try_from(client::dash3d::BuildArea::SIZE)
        .map_err(|_| "client build-area size is negative".to_string())?;
    if scene_size % MAP_LOAD_ORIGIN_STRIDE != 0 {
        return Err(format!(
            "loaded scene size {scene_size} is not aligned to {MAP_LOAD_ORIGIN_STRIDE}-tile zones"
        ));
    }
    terrain_cells.sort_unstable_by_key(|cell| (cell.world_x, cell.world_z));
    let (maximum_terrain_faces, terrain_origin_x, terrain_origin_z) = maximum_sorted_scene_metric(
        &terrain_cells,
        scene_size,
        MAP_WORLD_TILES,
        MAP_LOAD_ORIGIN_STRIDE,
        "terrain",
        |cell| usize::from(cell.faces),
    )?;
    let (
        conservative_terrain_faces_upper,
        conservative_terrain_origin_x,
        conservative_terrain_origin_z,
    ) = maximum_sorted_scene_metric(
        &terrain_cells,
        scene_size,
        MAP_WORLD_TILES,
        1,
        "terrain",
        |cell| usize::from(cell.faces),
    )?;

    let root = std::env::var_os("ENGINE_DIR")
        .map(PathBuf::from)
        .and_then(|engine| engine.parent().map(|parent| parent.join("content/maps")))
        .ok_or("set ENGINE_DIR to the revision-289 engine root with sibling content/maps")?;
    let entries = fs::read_dir(&root).map_err(|error| {
        format!(
            "cannot enumerate pinned engine map directory {}: {error}",
            root.display()
        )
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "cannot read an engine map directory entry under {}: {error}",
                root.display()
            )
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "jm2") {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(format!("no .jm2 source maps in {}", root.display()));
    }
    let indexed = map_index
        .iter()
        .map(|entry| (entry.square_x, entry.square_z))
        .collect::<BTreeSet<_>>();
    let mut seen_squares = BTreeSet::new();
    let mut source_entities = Vec::new();
    let mut source_loc_identities = Vec::new();
    let mut map_cells = 0usize;
    let mut explicit_map_cells = 0usize;
    let mut entity_counts = BTreeMap::<&'static str, usize>::new();
    for path in &paths {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("map source name is not UTF-8: {}", path.display()))?;
        let (square_x, square_z) = name
            .strip_prefix('m')
            .and_then(|value| value.split_once('_'))
            .ok_or_else(|| format!("map source name is not mX_Z: {name}"))?;
        let square_x = square_x
            .parse::<u8>()
            .map_err(|error| format!("invalid map square x in {name}: {error}"))?;
        let square_z = square_z
            .parse::<u8>()
            .map_err(|error| format!("invalid map square z in {name}: {error}"))?;
        if !seen_squares.insert((square_x, square_z)) {
            inputs.fail("duplicate_content_map", name);
            continue;
        }
        if !indexed.contains(&(square_x, square_z)) {
            inputs.fail("content_map_missing_from_cache_index", name);
        }
        let Some(bytes) = inputs.read(&format!("engine_map:{name}"), path) else {
            continue;
        };
        match parse_map_source(&bytes, name, square_x, square_z) {
            Ok(mut parsed) => {
                map_cells = map_cells
                    .checked_add(parsed.map_cells)
                    .ok_or_else(|| "total terrain cell count overflowed".to_string())?;
                explicit_map_cells = explicit_map_cells
                    .checked_add(parsed.explicit_map_cells)
                    .ok_or_else(|| "total explicit terrain cell count overflowed".to_string())?;
                for entity in &parsed.entities {
                    let count = entity_counts.entry(entity.kind.label()).or_default();
                    *count = count.checked_add(1).ok_or_else(|| {
                        format!("{} entity row count overflowed", entity.kind.label())
                    })?;
                    if entity.kind == EntityKind::Loc {
                        source_loc_identities.push(loc_identity(entity));
                    }
                }
                source_entities.append(&mut parsed.entities);
            }
            Err(error) => inputs.fail("content_map_invalid", error),
        }
    }
    let unrepresented_index_rows = map_index
        .iter()
        .filter(|entry| !seen_squares.contains(&(entry.square_x, entry.square_z)))
        .count();
    if unrepresented_index_rows != 0 {
        inputs.fail(
            "cache_index_without_engine_map_source",
            format!("{unrepresented_index_rows} cache-index squares have no .jm2 source"),
        );
    }
    if map_cells != cache_land_cells {
        inputs.fail(
            "terrain_source_count_mismatch",
            format!("engine sources have {map_cells} cells; maps.bin has {cache_land_cells}"),
        );
    }
    source_loc_identities.sort_unstable();
    let mut cache_loc_identities = cache_locations.iter().map(loc_identity).collect::<Vec<_>>();
    cache_loc_identities.sort_unstable();
    if source_loc_identities != cache_loc_identities {
        let first_difference = source_loc_identities
            .iter()
            .zip(&cache_loc_identities)
            .find(|(source, cache)| source != cache)
            .map(|(source, cache)| format!("source={source:?}, cache={cache:?}"));
        inputs.fail(
            "location_source_mismatch",
            format!(
                "engine .jm2 has {} LOC rows; maps.bin has {}{}",
                source_loc_identities.len(),
                cache_loc_identities.len(),
                first_difference
                    .map(|difference| format!("; first differing row: {difference}"))
                    .unwrap_or_default()
            ),
        );
    }

    let entities = source_entities
        .into_iter()
        .filter(|entity| entity.kind != EntityKind::Loc)
        .chain(cache_locations.iter().copied())
        .collect();
    let cache_location_rows = cache_locations.len();
    Ok(MapCensus {
        entities,
        terrain_cells,
        stats: BTreeMap::from([
            (
                "source_directory".to_string(),
                json!(root.display().to_string()),
            ),
            ("source_map_files".to_string(), json!(paths.len())),
            ("cache_index_rows".to_string(), json!(map_index.len())),
            (
                "cache_index_rows_without_engine_map_source".to_string(),
                json!(unrepresented_index_rows),
            ),
            ("terrain_cells".to_string(), json!(map_cells)),
            (
                "explicit_terrain_cells".to_string(),
                json!(explicit_map_cells),
            ),
            (
                "implicit_default_terrain_cells".to_string(),
                json!(map_cells - explicit_map_cells),
            ),
            ("cache_land_cells".to_string(), json!(cache_land_cells)),
            (
                "map_archive_records".to_string(),
                json!(archive_records.len()),
            ),
            (
                "unreferenced_map_archive_records".to_string(),
                json!(unreferenced_archive_records),
            ),
            ("static_loc_rows".to_string(), json!(cache_location_rows)),
            (
                "engine_static_loc_rows".to_string(),
                json!(entity_counts.get("loc").copied().unwrap_or(0)),
            ),
            (
                "static_npc_rows".to_string(),
                json!(entity_counts.get("npc").copied().unwrap_or(0)),
            ),
            (
                "maximum_terrain_faces_per_cell".to_string(),
                json!(max_terrain_faces),
            ),
            (
                "maximum_terrain_trigger".to_string(),
                json!(terrain_trigger),
            ),
            (
                "maximum_terrain_faces_per_loaded_scene".to_string(),
                json!(maximum_terrain_faces),
            ),
            (
                "maximum_terrain_loaded_scene_origin".to_string(),
                json!([terrain_origin_x, terrain_origin_z]),
            ),
            (
                "conservative_any_tile_window_terrain_face_upper".to_string(),
                json!(conservative_terrain_faces_upper),
            ),
            (
                "conservative_any_tile_window_terrain_origin".to_string(),
                json!([conservative_terrain_origin_x, conservative_terrain_origin_z]),
            ),
        ]),
    })
}

struct SceneCensus {
    maxima: BTreeMap<String, SceneMaximum>,
    submitted_static: StaticGeometryMaximum,
    conservative_scene_faces: (usize, usize, usize),
}

fn scene_census(
    entities: &mut [EntityPlacement],
    terrain: &mut [TerrainCell],
    world_layout: client::render::world::RenderTestContentLayout,
) -> Result<SceneCensus, String> {
    let scene_size = usize::try_from(client::dash3d::BuildArea::SIZE)
        .map_err(|_| "client build-area size is negative".to_string())?;
    if !scene_size.is_multiple_of(MAP_LOAD_ORIGIN_STRIDE) {
        return Err(format!(
            "loaded scene size {scene_size} is not aligned to {MAP_LOAD_ORIGIN_STRIDE}-tile zones"
        ));
    }
    if world_layout.scene_tiles_per_axis != scene_size {
        return Err(format!(
            "render layout reports {} scene tiles but BuildArea is {scene_size}",
            world_layout.scene_tiles_per_axis
        ));
    }
    let conservative_scene_faces =
        maximum_scene_metric(entities, scene_size, MAP_WORLD_TILES, 1, None, |entity| {
            entity.faces
        })?;
    let maxima = [
        ("all", None),
        ("loc", Some(EntityKind::Loc)),
        ("npc", Some(EntityKind::Npc)),
        ("obj", Some(EntityKind::Obj)),
    ]
    .into_iter()
    .map(|(name, kind)| {
        maximum_scene_entities(
            entities,
            scene_size,
            MAP_WORLD_TILES,
            MAP_LOAD_ORIGIN_STRIDE,
            kind,
        )
        .map(|maximum| (name.to_string(), maximum))
    })
    .collect::<Result<BTreeMap<_, _>, _>>()?;
    let submitted_static = maximum_submitted_static_geometry(
        terrain,
        entities,
        MAP_WORLD_TILES,
        scene_size,
        SUBMITTED_SCENE_TILES,
        world_layout.max_model_triangles_per_face,
        world_layout.capture_vertex_bytes,
    )?;
    Ok(SceneCensus {
        maxima,
        submitted_static,
        conservative_scene_faces,
    })
}

fn scene_maximum_json(maximum: &SceneMaximum) -> Value {
    json!({
        "metric_scope": "independent maxima across legal 8-aligned 104x104 loaded-scene origins",
        "loaded_scene_tiles_per_axis": client::dash3d::BuildArea::SIZE,
        "legal_loaded_scene_origin_stride_tiles": MAP_LOAD_ORIGIN_STRIDE,
        "maximum_resident_entity_rows": maximum.count,
        "maximum_resident_entity_rows_origin": [maximum.origin_x, maximum.origin_z],
        "maximum_resident_loaded_model_bytes": maximum.model_bytes,
        "maximum_resident_loaded_model_bytes_origin": [maximum.model_origin_x, maximum.model_origin_z],
        "maximum_resident_loaded_model_faces": maximum.faces,
        "maximum_resident_loaded_model_faces_origin": [maximum.face_origin_x, maximum.face_origin_z],
        "maximum_resident_loaded_model_points": maximum.points,
        "maximum_resident_loaded_model_points_origin": [maximum.point_origin_x, maximum.point_origin_z],
        "largest_triggering_content_id": maximum.triggering_id,
    })
}

fn synthetic_entity(x: u16, z: u16, faces: usize) -> EntityPlacement {
    EntityPlacement {
        kind: EntityKind::Loc,
        id: u32::from(x) * 256 + u32::from(z),
        plane: 0,
        world_x: x,
        world_z: z,
        shape: 10,
        rotation: 0,
        amount: 1,
        model_resolved: true,
        model_bytes: faces * 8,
        faces,
        points: faces * 3,
    }
}

#[test]
fn loaded_scene_metric_uses_eight_tile_aligned_origins() {
    let entities = [synthetic_entity(7, 3, 1), synthetic_entity(8, 3, 1)];
    let maximum = maximum_scene_metric(&entities, 8, 32, 8, None, |_| 1).unwrap();
    assert_eq!(maximum, (1, 0, 0));
    let any_tile_maximum = maximum_scene_metric(&entities, 8, 32, 1, None, |_| 1).unwrap();
    assert_eq!(any_tile_maximum, (2, 1, 0));
    assert!(any_tile_maximum.0 > maximum.0);
    assert_eq!(maximum.1 % MAP_LOAD_ORIGIN_STRIDE, 0);
    assert_eq!(maximum.2 % MAP_LOAD_ORIGIN_STRIDE, 0);
}

#[test]
fn submitted_static_geometry_sums_only_co_located_components() {
    let terrain = vec![TerrainCell {
        world_x: 0,
        world_z: 0,
        faces: 9,
    }];
    let mut entities = vec![synthetic_entity(2, 0, 9)];
    let terrain_maximum = maximum_sorted_scene_metric(&terrain, 2, 8, 1, "terrain test", |cell| {
        usize::from(cell.faces)
    })
    .unwrap()
    .0;
    let entity_maximum = maximum_scene_metric(&entities, 2, 8, 1, None, |entity| entity.faces)
        .unwrap()
        .0;
    let maximum =
        maximum_submitted_static_geometry(&terrain, &mut entities, 8, 4, 2, 2, 24).unwrap();
    assert_eq!(terrain_maximum, 9);
    assert_eq!(entity_maximum, 9);
    assert_eq!(maximum.total_faces, 9);
    assert_eq!(
        maximum.terrain_faces + maximum.entity_faces,
        maximum.total_faces
    );
    assert!(terrain_maximum + entity_maximum > maximum.total_faces);
}

#[test]
fn submitted_fifty_tile_footprint_is_distinct_from_loaded_104_tile_scene() {
    let terrain = Vec::new();
    let mut entities = vec![synthetic_entity(0, 2, 3), synthetic_entity(60, 2, 3)];
    let loaded = maximum_scene_entities(&entities, 104, 120, MAP_LOAD_ORIGIN_STRIDE, None).unwrap();
    let submitted =
        maximum_submitted_static_geometry(&terrain, &mut entities, 120, 104, 50, 2, 24).unwrap();
    assert_eq!(loaded.faces, 6);
    assert_eq!(submitted.total_faces, 3);
    assert!(loaded.faces > submitted.total_faces);
    assert_eq!(
        [
            submitted.containing_loaded_origin_x,
            submitted.containing_loaded_origin_z
        ],
        [0, 0]
    );
}

#[test]
fn dynamic_face_upper_sums_independent_slot_maxima_and_attachments() {
    let mut slots = empty_player_slot_maxima();
    slots[3].record_faces(17, "obj:47:wear-gender:0".to_string());
    slots[3].record_faces(12, "idk:9:body:part:4:gender:0".to_string());
    slots[5].record_faces(
        23,
        "seq:8:replaceheldleft:value:559:object_wear:47".to_string(),
    );
    assert_eq!(checked_slot_face_sum(&slots, "test player").unwrap(), 40);
    assert_eq!(
        checked_face_sum(&[40, 11, 13], "test attachments").unwrap(),
        64
    );
    assert_eq!(slots[3].trigger.as_deref(), Some("obj:47:wear-gender:0"));
}

#[test]
fn dynamic_face_upper_uses_transmog_branch_max_not_sum() {
    let (base_faces, branch) = transmog_branch_max(40, 90);
    assert_eq!(base_faces, 90);
    assert_eq!(branch, "npc_transmog");
    assert_eq!(
        checked_face_sum(&[base_faces, 11, 13], "test player").unwrap(),
        114
    );
    assert_eq!(transmog_branch_max(90, 40), (90, "appearance_slots"));
}

#[test]
fn dynamic_face_upper_checked_aggregation_rejects_overflow() {
    let mut slots = empty_player_slot_maxima();
    slots[0].faces = usize::MAX;
    slots[1].faces = 1;
    assert!(checked_slot_face_sum(&slots, "test slots").is_err());
    assert!(checked_face_sum(&[usize::MAX, 1], "test attachments").is_err());
}

#[test]
fn loc_shape_without_a_paired_model_is_not_declared() {
    let mut loc = client::config::LocType {
        shape: Some(vec![10, 4]),
        model: Some(vec![42]),
        ..Default::default()
    };
    assert!(!loc_declares_model_for_shape(&loc, 4));
    loc.model = Some(vec![42, 43]);
    assert!(loc_declares_model_for_shape(&loc, 4));
    loc.model = Some(vec![42, -1]);
    assert!(!loc_declares_model_for_shape(&loc, 4));
    loc.shape = Some(vec![10]);
    loc.model = Some(vec![42]);
    assert!(loc_declares_model_for_shape(&loc, 10));
    loc.shape = None;
    assert!(loc_declares_model_for_shape(&loc, 10));
    assert!(!loc_declares_model_for_shape(&loc, 4));
}

#[test]
fn admitted_geometry_maximizes_faces_under_the_shared_count() {
    let upper = aggregate_admitted_geometry(
        7,
        10,
        [
            ("player", 2, 8),
            ("npc", 3, 5),
            ("projectile_or_map_spot", 10, 2),
        ],
        2,
        std::mem::size_of::<client::render::world::GpuVertex>(),
    )
    .unwrap();
    assert_eq!(upper.dynamic_faces_upper, 41);
    assert_eq!(upper.scene_faces_upper, 48);
    assert_eq!(upper.vertex_payload_bytes_upper, 48 * 2 * 3 * 16);
    assert_eq!(
        upper.dynamic_classes[0]["count_in_maximizing_shared_allocation"],
        2
    );
    assert_eq!(
        upper.dynamic_classes[1]["count_in_maximizing_shared_allocation"],
        3
    );
    assert_eq!(
        upper.dynamic_classes[2]["count_in_maximizing_shared_allocation"],
        5
    );
}

#[test]
fn admitted_geometry_does_not_assume_player_faces_are_largest() {
    let upper = aggregate_admitted_geometry(
        0,
        10,
        [
            ("player", 2, 1),
            ("npc", 3, 2),
            ("projectile_or_map_spot", 10, 8),
        ],
        2,
        std::mem::size_of::<client::render::world::GpuVertex>(),
    )
    .unwrap();
    assert_eq!(upper.dynamic_faces_upper, 80);
    assert_eq!(upper.dynamic_classes[0]["class"], "projectile_or_map_spot");
}

#[test]
fn admitted_geometry_rejects_face_overflow() {
    assert!(aggregate_admitted_geometry(
        usize::MAX,
        1,
        [
            ("player", 1, 1),
            ("npc", 0, 0),
            ("projectile_or_map_spot", 0, 0)
        ],
        2,
        24,
    )
    .is_err());
}

#[test]
fn vertex_capacity_uses_minimum_power_of_two_without_a_ceiling() {
    assert_eq!(vertex_capacity_bytes(0), Some(1_048_576));
    assert_eq!(vertex_capacity_bytes(1_048_576), Some(1_048_576));
    assert_eq!(vertex_capacity_bytes(1_048_577), Some(2_097_152));
    assert_eq!(
        vertex_capacity_bytes(128 * 1024 * 1024 + 1),
        Some(256 * 1024 * 1024)
    );
    assert_eq!(vertex_capacity_bytes(usize::MAX), None);
}

#[test]
fn vertex_growth_peak_and_device_support_use_the_full_capacity() {
    assert_eq!(vertex_growth_peak_bytes(1_048_576), Some(1_048_576));
    assert_eq!(vertex_growth_peak_bytes(2_097_152), Some(3_145_728));
    assert_eq!(vertex_capacity_supported(2_097_152, 2_097_151), Some(false));
    assert_eq!(vertex_capacity_supported(2_097_152, 2_097_152), Some(true));
}
#[test]
fn census_reports_exact_nav_alignment_and_separate_vertex_replacement_charge() {
    let nav = render_test_nav_layout();
    assert_eq!(
        resource_alignment(RenderResource::NavVertices),
        nav.vertex_alignment
    );
    assert_eq!(
        resource_alignment(RenderResource::NavQuads),
        nav.quad_alignment
    );
    assert_eq!(
        resource_alignment(RenderResource::NavEdges),
        nav.edge_alignment
    );
    assert_eq!(
        resource_alignment(RenderResource::NavInput),
        nav.input_alignment
    );
    for resource in [
        RenderResource::Capture,
        RenderResource::NavVertices,
        RenderResource::NavQuads,
        RenderResource::NavEdges,
        RenderResource::NavInput,
        RenderResource::HeadModels,
        RenderResource::HeadMisc,
        RenderResource::TempModels,
        RenderResource::SharedModels,
        RenderResource::SharedSprites,
        RenderResource::Vertex,
        RenderResource::ContextCpu,
        RenderResource::ContextGpu,
    ] {
        assert!(resource_alignment(resource).is_power_of_two());
    }
    let resources = resource_rows(
        &BTreeMap::new(),
        &BTreeMap::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        false,
    );
    let vertex = resources
        .iter()
        .find(|row| row["resource"] == "Vertex")
        .unwrap();
    assert_eq!(vertex["B_bytes"], Value::Null);
    assert_eq!(vertex["charge_limit_bytes"], Value::Null);
    assert_eq!(vertex["status"], "incomplete");
}
