use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use client::config::Cache;
use client::dash3d::store::render_test_shared_store_layout;
use client::dash3d::{LocAngle, Model, LOC_SHAPE_TO_LAYER};
use client::graphics::PixFont;
use client::io::JagFile;
use client::map_cache::{self, LocPlacement, MapIndexEntry};
use client::render::backend::gpu_atlas::GpuAssets;
use client::render::budget::RenderResource;
use client::render::nav_debug::{render_test_caption_ink_pixels, render_test_nav_layout};
use client::render::store::render_test_shared_index_layout;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

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

fn decode_record_stream(bytes: &[u8], label: &str) -> Result<Vec<Record<'_>>, String> {
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
        if bytes > self.maximum_bytes {
            self.maximum_bytes = bytes;
            self.trigger = Some(trigger);
        }
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
    let model = match placement.kind {
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
        }
        EntityKind::Npc => {
            let Some(npc) = usize::try_from(placement.id)
                .ok()
                .and_then(|id| cache.npcs.get(id))
            else {
                return Err(format!("{trigger}: NPC config is missing"));
            };
            npc.get_temp_model(cache, -1, -1, None)
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
        }
    };
    match model {
        Some(model) => model_measure(&model, &trigger).map(Some),
        None => Ok(None),
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
    x: u8,
    z: u8,
    world_x: u16,
    world_z: u16,
    shape: u8,
    rotation: u8,
    amount: u32,
    model_bytes: usize,
    faces: usize,
    points: usize,
}

#[derive(Default)]
struct ParsedMap {
    map_cells: usize,
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
                parsed.map_cells = parsed
                    .map_cells
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
                    x: coordinates[1],
                    z: coordinates[2],
                    world_x: u16::from(square_x) * 64 + u16::from(coordinates[1]),
                    world_z: u16::from(square_z) * 64 + u16::from(coordinates[2]),
                    shape,
                    rotation,
                    amount: 1,
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
                    x: coordinates[1],
                    z: coordinates[2],
                    world_x: u16::from(square_x) * 64 + u16::from(coordinates[1]),
                    world_z: u16::from(square_z) * 64 + u16::from(coordinates[2]),
                    shape: 0,
                    rotation: 0,
                    amount,
                    model_bytes: 0,
                    faces: 0,
                    points: 0,
                });
            }
        }
    }
    if parsed.map_cells != seen_map_cells.len() {
        return Err(format!(
            "{label}: MAP has {} unique cells, expected {}",
            parsed.map_cells,
            seen_map_cells.len()
        ));
    }
    if section.is_none() {
        return Err(format!("{label}: no map sections"));
    }
    Ok(parsed)
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

fn maximum_scene_metric(
    entities: &[EntityPlacement],
    scene_size: usize,
    world_size: usize,
    kind: Option<EntityKind>,
    metric: impl Fn(&EntityPlacement) -> usize,
) -> Result<(usize, usize, usize), String> {
    if scene_size == 0 || scene_size > world_size {
        return Err("scene dimensions do not fit the world grid".to_string());
    }
    let origin_count = world_size - scene_size + 1;
    let mut events = (0..origin_count)
        .map(|_| Vec::<(usize, usize, i128)>::new())
        .collect::<Vec<_>>();
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
        let first_x = x.saturating_sub(scene_size - 1);
        let last_x = x.min(origin_count - 1);
        let first_z = z.saturating_sub(scene_size - 1);
        let last_z = z.min(origin_count - 1);
        let delta = i128::try_from(metric(entity))
            .map_err(|_| "scene entity metric exceeds signed accumulator range".to_string())?;
        if delta == 0 {
            continue;
        }
        events[first_x].push((first_z, last_z, delta));
        if let Some(after_last_x) = last_x.checked_add(1).filter(|&next| next < origin_count) {
            events[after_last_x].push((first_z, last_z, -delta));
        }
    }

    let mut tree = RangeAddMax::new(origin_count)?;
    let mut best = (0usize, 0usize, 0usize);
    for (origin_x, updates) in events.into_iter().enumerate() {
        for (first_z, last_z, delta) in updates {
            tree.add(first_z, last_z, delta)?;
        }
        let (value, origin_z) = tree.maximum()?;
        if value > best.0 {
            best = (value, origin_x, origin_z);
        }
    }
    Ok(best)
}

fn maximum_scene_entities(
    entities: &[EntityPlacement],
    scene_size: usize,
    world_size: usize,
    kind: Option<EntityKind>,
) -> Result<SceneMaximum, String> {
    let (count, origin_x, origin_z) =
        maximum_scene_metric(entities, scene_size, world_size, kind, |_| 1)?;
    let (model_bytes, model_origin_x, model_origin_z) =
        maximum_scene_metric(entities, scene_size, world_size, kind, |entity| {
            entity.model_bytes
        })?;
    let (faces, face_origin_x, face_origin_z) =
        maximum_scene_metric(entities, scene_size, world_size, kind, |entity| {
            entity.faces
        })?;
    let (points, point_origin_x, point_origin_z) =
        maximum_scene_metric(entities, scene_size, world_size, kind, |entity| {
            entity.points
        })?;
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

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn add_source_hashes(ledger: &mut InputLedger) -> BTreeMap<&'static str, Vec<u8>> {
    const SOURCES: &[&str] = &[
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
        "vendor/fr-client-rust/crates/client/src/config/spot_type.rs",
        "vendor/fr-client-rust/crates/client/src/config/if_type.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs",
        "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs",
        "vendor/fr-client-rust/crates/client/src/render/media.rs",
        "vendor/fr-client-rust/crates/client/src/render/graphic_sprites.rs",
        "vendor/fr-client-rust/crates/client/src/client/client.rs",
        "vendor/fr-client-rust/crates/client/src/core/world.rs",
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
                | "vendor/fr-client-rust/crates/client/src/dash3d/client_player.rs"
                | "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs"
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
    if route_transitions != nav_planner::router::NODE_BUDGET {
        return Err("source route budget differs from compiled route budget".to_string());
    }
    let max_draw_tiles = source_usize_const(paint_source, "pub const MAX_DRAW_TILES: usize =")?;
    if max_draw_tiles != nav_planner::paint::MAX_DRAW_TILES {
        return Err("source path draw cap differs from compiled path draw cap".to_string());
    }
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
struct SceneCaptureBounds {
    terrain_faces: usize,
    static_entity_faces: usize,
    dynamic_sprites: usize,
    dynamic_faces_per_sprite: usize,
    scene_faces: usize,
    scene_triangles: usize,
    capture_payload_bytes: usize,
    capture_peak_bytes: usize,
}

fn scene_capture_bounds(
    families: &BTreeMap<String, FamilyMaximum>,
    scene_maxima: &BTreeMap<String, SceneMaximum>,
    map_stats: &BTreeMap<String, Value>,
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
        .ok_or_else(|| "missing maximum static scene entity record".to_string())?;
    let terrain_faces = map_stats
        .get("maximum_terrain_faces_per_scene")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "missing maximum terrain faces per scene".to_string())?;
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
    let dynamic_faces_per_sprite = component_faces
        .checked_mul(12)
        .and_then(|faces| faces.checked_add(spot_faces))
        .and_then(|faces| faces.checked_add(loc_faces))
        .ok_or_else(|| "dynamic sprite face bound overflowed".to_string())?;
    let dynamic_faces = layout
        .max_dynamic_sprites
        .checked_mul(dynamic_faces_per_sprite)
        .ok_or_else(|| "dynamic scene face bound overflowed".to_string())?;
    let scene_faces = terrain_faces
        .checked_add(scene.faces)
        .and_then(|faces| faces.checked_add(dynamic_faces))
        .ok_or_else(|| "complete scene face bound overflowed".to_string())?;
    let scene_triangles = scene_faces
        .checked_mul(layout.max_model_triangles_per_face)
        .ok_or_else(|| "scene triangle bound overflowed".to_string())?;
    let capture_vertices = scene_triangles
        .checked_mul(3)
        .ok_or_else(|| "capture vertex count overflowed".to_string())?;
    let capture_payload_bytes = capture_vertices
        .checked_mul(layout.capture_vertex_bytes)
        .ok_or_else(|| "capture payload byte bound overflowed".to_string())?;
    let capture_peak_bytes =
        vector_replacement_peak_bytes(capture_vertices, layout.capture_vertex_bytes)
            .ok_or_else(|| "capture replacement peak overflowed".to_string())?;
    Ok(SceneCaptureBounds {
        terrain_faces,
        static_entity_faces: scene.faces,
        dynamic_sprites: layout.max_dynamic_sprites,
        dynamic_faces_per_sprite,
        scene_faces,
        scene_triangles,
        capture_payload_bytes,
        capture_peak_bytes,
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
                "maximum_static_scene_model_bytes": scene.model_bytes,
                "maximum_static_scene_entity_count": scene.count,
                "max_dynamic_sprite_slots": world.max_dynamic_sprites,
                "max_retained_model_family_bytes": model_max,
                "player_dynamic_model_bytes_upper": player_dynamic_model_bytes,
                "dynamic_model_bytes_per_slot_upper": dynamic_model_max,
                "dynamic_model_payload_bytes": dynamic_model_bytes,
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
                    "dynamic_sprite_slots": world.max_dynamic_sprites,
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
) -> Vec<Value> {
    let model_trigger = families
        .get("base_models")
        .and_then(|family| family.trigger.as_deref());
    let loc_trigger = families
        .get("loc_models")
        .and_then(|family| family.trigger.as_deref());
    let scene_trigger = scene_maxima.get("all").map(|max| {
        format!(
            "scene-origin:{},{};static-loc-count:{}",
            max.origin_x, max.origin_z, max.count
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
            "terrain_faces": capture.terrain_faces,
            "static_entity_faces": capture.static_entity_faces,
            "max_dynamic_sprites": capture.dynamic_sprites,
            "dynamic_faces_per_sprite_upper": capture.dynamic_faces_per_sprite,
            "scene_faces_upper": capture.scene_faces,
            "scene_triangles_upper": capture.scene_triangles,
            "capture_payload_bytes_upper": capture.capture_payload_bytes,
            "capture_replacement_peak_bytes_upper": capture.capture_peak_bytes,
        })
    });
    let vertex_known = capture_bounds.and_then(|capture| {
        let atlas = atlas_layout?;
        let final_capacity = capture.capture_payload_bytes.checked_next_power_of_two()?;
        let previous_capacity = if final_capacity > 1 {
            final_capacity / 2
        } else {
            0
        };
        let replacement_peak = final_capacity.checked_add(previous_capacity)?;
        Some(json!({
            "required_scene_vertex_payload_bytes": capture.capture_payload_bytes,
            "next_power_of_two_storage_capacity_bytes": final_capacity,
            "replacement_peak_bytes": replacement_peak,
            "device_max_storage_buffer_binding_size": atlas.max_storage_buffer_binding_size,
            "fits_requested_device_storage_limit": final_capacity <= atlas.max_storage_buffer_binding_size as usize,
        }))
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
            "all terrain, static map, and capped dynamic model faces × maximum clipped model triangles per face × 3 CaptureVertex values; include old/new Vec capacity at replacement peak",
            scene_trigger.clone(),
            "The pinned map/entity/model inputs or source-defined dynamic sprite and capture topology limits are unavailable.",
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
            "sum of retained SceneModel variants admitted by one head: max legal scene static LOC + dynamic actor/object model families, including every nested Model vector and shared handle",
            loc_trigger.map(str::to_owned),
            "The complete per-head SceneModel composition and old/new replacement overlap are not bounded by current evidence.",
        ),
        (
            RenderResource::HeadMisc,
            "sum of RenderWorld storage, dynamic sprite slots, overlays, chat/text buffers, nested Pix32 data and other owned per-head payloads; use the maximum old-plus-new replacement peak",
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
            "maximum scene vertex payload before checked next-power-of-two GPU storage capacity; report the requested device storage limit and old/new buffer replacement peak",
            loc_trigger.map(str::to_owned),
            "The scene topology, capture element layout, or requested device storage limit is unavailable.",
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
                    capture_bounds.map_or(Value::Null, |capture| json!(capture.capture_peak_bytes)),
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
                    capture_bounds.map_or(Value::Null, |capture| json!(capture.capture_payload_bytes)),
                    vertex_known.clone().unwrap_or(Value::Null),
                    vertex_known.is_some(),
                ),
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
            json!({
                "resource": format!("{resource:?}"),
                "display": resource.to_string(),
                "formula": formula,
                "content_upper_bytes": content_upper,
                "known_measurements": measured,
                "largest_triggering_content_id": trigger,
                "status": if complete { "complete" } else { "incomplete" },
                "blockers": if complete {
                    Vec::<Value>::new()
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
    let mut map_stats = BTreeMap::<String, Value>::new();
    let mut scene_maxima = BTreeMap::<String, SceneMaximum>::new();
    let mut model_record_count = None;
    let mut model_parts = None;
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
        let rotations = [
            LocAngle::WEST,
            LocAngle::NORTH,
            LocAngle::EAST,
            LocAngle::SOUTH,
        ];
        let mut loc_variant_bytes = BTreeMap::<(u32, u8, u8), usize>::new();

        for loc in &cache.locs {
            let shapes = match loc.shape.as_deref() {
                Some(shapes) => shapes.to_vec(),
                None => (0..LOC_SHAPE_TO_LAYER.len())
                    .filter_map(|shape| i32::try_from(shape).ok())
                    .collect(),
            };
            for shape in shapes {
                for rotation in rotations {
                    let Some(model) = loc.get_model_shared(cache, shape, rotation, 0, 0, 0, 0, -1)
                    else {
                        continue;
                    };
                    let trigger = format!("loc:{}:shape:{shape}:rotation:{rotation}", loc.id);
                    if let Err(error) = locs.record(&model, trigger) {
                        inputs.fail("loc_family_measurement_failed", error);
                    }
                }
            }
        }
        for npc in &cache.npcs {
            if let Some(model) = npc.get_temp_model(cache, -1, -1, None) {
                if let Err(error) = npcs.record(&model, format!("npc:{}", npc.id)) {
                    inputs.fail("npc_family_measurement_failed", error);
                }
            }
        }
        for obj in &cache.objs {
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
            for gender in 0..=1 {
                if let Some(model) = obj.get_wear_model_no_check(gender) {
                    if let Err(error) =
                        objs.record(&model, format!("obj:{}:wear-gender:{gender}", obj.id))
                    {
                        inputs.fail("obj_family_measurement_failed", error);
                    }
                }
                if let Some(model) = obj.get_head_model_no_check(gender) {
                    if let Err(error) =
                        objs.record(&model, format!("obj:{}:head-gender:{gender}", obj.id))
                    {
                        inputs.fail("obj_family_measurement_failed", error);
                    }
                }
            }
        }
        for (id, idk) in cache.idks.iter().enumerate() {
            if let Some(model) = idk.get_model_no_check() {
                if let Err(error) = idks.record(&model, format!("idk:{id}:body")) {
                    inputs.fail("idk_family_measurement_failed", error);
                }
            }
            if let Some(model) = idk.get_head_no_check() {
                if let Err(error) = idks.record(&model, format!("idk:{id}:head")) {
                    inputs.fail("idk_family_measurement_failed", error);
                }
            }
        }
        for spot in &cache.spots {
            if let Some(model) = spot.get_temp_model2(cache) {
                if let Err(error) = spots.record(&model, format!("spot:{}", spot.id)) {
                    inputs.fail("spot_family_measurement_failed", error);
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
        match scene_census(&map_entities) {
            Ok(maxima) => {
                map_stats.insert(
                    "maximum_scene_entities".to_string(),
                    Value::Object(
                        maxima
                            .iter()
                            .map(|(name, maximum)| (name.clone(), scene_maximum_json(maximum)))
                            .collect(),
                    ),
                );
                scene_maxima = maxima;
            }
            Err(error) => inputs.fail("scene_census_failed", error),
        }
    } else {
        inputs.fail(
            "config_unavailable",
            "cannot enumerate transformed model families without the pinned config JAG",
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
    let capture_bounds = match scene_capture_bounds(&families, &scene_maxima, &map_stats) {
        Ok(bounds) => Some(bounds),
        Err(error) => {
            inputs.fail("scene_capture_bound_failed", error);
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
    let world_layout = client::render::world::render_test_content_layout();
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
    );
    let complete =
        inputs.failures.is_empty() && resources.iter().all(|row| row["status"] == "complete");
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
        "model_total_retained_bytes": model_total_bytes,
        "map_inventory": map_stats,
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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
            terrain_cells.push(EntityPlacement {
                kind: EntityKind::Loc,
                id: 0,
                plane: cell.plane,
                x: cell.x,
                z: cell.z,
                world_x: u16::from(entry.square_x) * 64 + u16::from(cell.x),
                world_z: u16::from(entry.square_z) * 64 + u16::from(cell.z),
                shape: 0,
                rotation: 0,
                amount: 1,
                model_bytes: 0,
                faces,
                points: 0,
            });
        })
        .map_err(|error| format!("land record {land_id}: {error}"))?;
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
                x: placement.x,
                z: placement.z,
                world_x: u16::from(entry.square_x) * 64 + u16::from(placement.x),
                world_z: u16::from(entry.square_z) * 64 + u16::from(placement.z),
                shape: placement.shape,
                rotation: placement.rotation,
                amount: 1,
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
    let (maximum_terrain_faces, terrain_origin_x, terrain_origin_z) =
        maximum_scene_metric(&terrain_cells, scene_size, MAP_WORLD_TILES, None, |cell| {
            cell.faces
        })?;

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
                "maximum_terrain_faces_per_scene".to_string(),
                json!(maximum_terrain_faces),
            ),
            (
                "maximum_terrain_scene_origin".to_string(),
                json!([terrain_origin_x, terrain_origin_z]),
            ),
        ]),
    })
}

fn scene_census(entities: &[EntityPlacement]) -> Result<BTreeMap<String, SceneMaximum>, String> {
    let scene_size = usize::try_from(client::dash3d::BuildArea::SIZE)
        .map_err(|_| "client build-area size is negative".to_string())?;
    [
        ("all", None),
        ("loc", Some(EntityKind::Loc)),
        ("npc", Some(EntityKind::Npc)),
        ("obj", Some(EntityKind::Obj)),
    ]
    .into_iter()
    .map(|(name, kind)| {
        maximum_scene_entities(entities, scene_size, MAP_WORLD_TILES, kind)
            .map(|maximum| (name.to_string(), maximum))
    })
    .collect()
}

fn scene_maximum_json(maximum: &SceneMaximum) -> Value {
    json!({
        "maximum_entity_rows": maximum.count,
        "maximum_entity_scene_origin": [maximum.origin_x, maximum.origin_z],
        "maximum_model_retained_bytes": maximum.model_bytes,
        "maximum_model_scene_origin": [maximum.model_origin_x, maximum.model_origin_z],
        "maximum_model_faces": maximum.faces,
        "maximum_face_scene_origin": [maximum.face_origin_x, maximum.face_origin_z],
        "maximum_model_points": maximum.points,
        "maximum_point_scene_origin": [maximum.point_origin_x, maximum.point_origin_z],
        "largest_triggering_content_id": maximum.triggering_id,
    })
}
