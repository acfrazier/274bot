//! Map-canvas label ranking and overlap. Markers are independent of this
//! filter; only the text rectangles compete.

use nav::map::poi::PoiKind;

/// Axis-aligned label box in canvas points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl LabelRect {
    pub fn overlaps(self, other: Self) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// Canvas offset from the POI point to the label's top-left.
pub const LABEL_OFFSET: [f32; 2] = [5.0, -6.0];

/// One visible named POI after ImGui measurement.
#[derive(Clone, Copy, Debug)]
pub struct MapLabelCandidate {
    pub id: usize,
    pub name_off: u32,
    pub name_len: u32,
    pub px: f32,
    pub py: f32,
    pub content_priority: u8,
    pub kind_rank: u8,
    pub dist2: u32,
    pub rect: LabelRect,
}

/// Content `maps/labels.txt` priority first (lower is more important), then
/// kind, then distance to the view centre. Non-labels sort after every
/// content label whose priority is below 255.
pub fn content_priority(kind: PoiKind) -> u8 {
    match kind {
        PoiKind::Label { priority } => priority,
        _ => 255,
    }
}

pub fn kind_rank(kind: PoiKind) -> u8 {
    match kind {
        PoiKind::Bank => 0,
        PoiKind::Shop => 1,
        PoiKind::Altar => 2,
        PoiKind::Label { .. } => 3,
        PoiKind::MapSymbol { .. } => 4,
        PoiKind::Transport => 5,
        PoiKind::Teleport => 6,
    }
}

pub fn dist2_to_centre(dx: f64, dz: f64) -> u32 {
    let d = dx.mul_add(dx, dz * dz);
    if !d.is_finite() || d >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        d as u32
    }
}

/// Rank `candidates` in place and fill `out` with indices (into the ranked
/// slice) of labels that do not overlap an already placed rectangle, up to
/// `cap`.
pub fn place_map_labels(candidates: &mut [MapLabelCandidate], cap: usize, out: &mut Vec<usize>) {
    out.clear();
    // Unique `id` is a total order, so this is frame-stable without merge-sort scratch.
    candidates.sort_unstable_by(|a, b| {
        a.content_priority
            .cmp(&b.content_priority)
            .then(a.kind_rank.cmp(&b.kind_rank))
            .then(a.dist2.cmp(&b.dist2))
            .then(a.id.cmp(&b.id))
    });
    for (i, candidate) in candidates.iter().enumerate() {
        if out.len() >= cap {
            break;
        }
        if out
            .iter()
            .any(|&j| candidate.rect.overlaps(candidates[j].rect))
        {
            continue;
        }
        out.push(i);
    }
}
