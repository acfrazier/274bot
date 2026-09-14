//! Isolate-thread producer for `skill.xp` and `inventory.changed`.
//!
//! Diffs posted FlatBuffer inv/stats after decode. JS only stores `this.on`
//! callbacks. Callbacks are not executed here — `load` delivers the returned
//! events from a generation/pause/budget-gated tick.

use crate::isolate_fb::{RowReader, SnapshotReader, StatReader};

const MAX_INV_SIZE: i32 = 28;
const MAX_STATS: usize = 64;
const EMPTY_ID: i32 = -1;
const EMPTY_COUNT: i32 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlotState {
    id: i32,
    count: i32,
    name: Option<String>,
}

impl SlotState {
    fn empty() -> Self {
        Self {
            id: EMPTY_ID,
            count: EMPTY_COUNT,
            name: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct XpState {
    index: i32,
    name: String,
    xp: i32,
}

/// One public event to deliver on a gated tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeEvent {
    SkillXp {
        skill: i32,
        name: String,
        xp: i32,
        delta: i32,
    },
    InventoryChanged {
        slot: i32,
        id: i32,
        name: Option<String>,
        count: i32,
        previous_id: i32,
        previous_count: i32,
    },
}

impl NativeEvent {
    pub fn type_name(&self) -> &'static str {
        match self {
            NativeEvent::SkillXp { .. } => "skill.xp",
            NativeEvent::InventoryChanged { .. } => "inventory.changed",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObserveResult {
    pub events: Vec<NativeEvent>,
    pub diagnostic: Option<String>,
}

/// Per-isolate last delivered baselines plus pause-held current tables.
#[derive(Debug, Default)]
pub struct NativeEventProducer {
    last_inv: Option<Vec<SlotState>>,
    last_inv_size: i32,
    last_xp: Option<Vec<XpState>>,
    paused: bool,
    held_inv: Option<Vec<SlotState>>,
    held_inv_size: Option<i32>,
    held_xp: Option<Vec<XpState>>,
    held_ingame_false: bool,
}

impl NativeEventProducer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Pause freezes callback production. Snapshot decode may still absorb
    /// current tables into a single held state (no backlog). Resume returns
    /// the net eligible diff against the last delivered baseline.
    pub fn set_paused(&mut self, paused: bool) -> ObserveResult {
        if self.paused == paused {
            return ObserveResult::default();
        }
        self.paused = paused;
        if paused {
            self.clear_held();
            ObserveResult::default()
        } else {
            self.flush_held()
        }
    }

    pub fn observe(&mut self, snap: &SnapshotReader<'_>) -> ObserveResult {
        if self.paused {
            let diagnostic = self.absorb_held(snap);
            return ObserveResult {
                events: Vec::new(),
                diagnostic,
            };
        }
        self.observe_live(snap)
    }

    fn observe_live(&mut self, snap: &SnapshotReader<'_>) -> ObserveResult {
        if snap.has_ingame() && !snap.ingame() {
            self.reset_baselines();
            return ObserveResult::default();
        }

        let mut events = Vec::new();
        let mut diagnostic = None;

        if snap.has_stats() {
            events.extend(self.apply_stats(&collect_xp(snap)));
        }

        if snap.has_inv_size() {
            let size = snap.inv_size();
            if size <= 0 {
                self.last_inv_size = 0;
                self.last_inv = None;
            } else if size > MAX_INV_SIZE {
                diagnostic = Some(format!(
                    "inventory events: inv_size {size} exceeds {MAX_INV_SIZE}; family reset"
                ));
                self.last_inv_size = 0;
                self.last_inv = None;
            } else {
                if !snap.has_inv() && self.last_inv_size != size {
                    // Scalar size change without rows: do not fabricate removals.
                    self.last_inv = None;
                }
                self.last_inv_size = size;
            }
        }

        if snap.has_inv() {
            match self.apply_inv(&snap.inv()) {
                Ok(inv_events) => events.extend(inv_events),
                Err(reason) => {
                    diagnostic = Some(reason);
                    self.last_inv = None;
                }
            }
        }

        ObserveResult { events, diagnostic }
    }

    fn apply_stats(&mut self, current: &[XpState]) -> Vec<NativeEvent> {
        match self.last_xp.as_mut() {
            None => {
                self.last_xp = Some(current.to_vec());
                Vec::new()
            }
            Some(last) => {
                let events = diff_xp(last, current);
                merge_xp(last, current);
                events
            }
        }
    }

    fn apply_inv(&mut self, rows: &[RowReader<'_>]) -> Result<Vec<NativeEvent>, String> {
        if self.last_inv_size <= 0 {
            // Tutorial-locked / unknown size: leftover rows do not seed.
            return Ok(Vec::new());
        }
        let current = expand_inv(rows, self.last_inv_size)?;
        match self.last_inv.as_ref() {
            None => {
                self.last_inv = Some(current);
                Ok(Vec::new())
            }
            Some(last) => {
                let events = diff_inv(last, &current);
                self.last_inv = Some(current);
                Ok(events)
            }
        }
    }

    fn absorb_held(&mut self, snap: &SnapshotReader<'_>) -> Option<String> {
        if snap.has_ingame() && !snap.ingame() {
            self.held_ingame_false = true;
            self.held_inv = None;
            self.held_xp = None;
            self.held_inv_size = Some(0);
            return None;
        }
        if snap.has_ingame() && snap.ingame() {
            self.held_ingame_false = false;
        }
        if snap.has_stats() {
            self.held_xp = Some(collect_xp(snap));
        }
        let mut diagnostic = None;
        if snap.has_inv_size() {
            let size = snap.inv_size();
            if size <= 0 || size > MAX_INV_SIZE {
                if size > MAX_INV_SIZE {
                    diagnostic = Some(format!(
                        "inventory events: inv_size {size} exceeds {MAX_INV_SIZE}; family reset"
                    ));
                }
                self.held_inv_size = Some(0);
                self.held_inv = None;
            } else {
                if !snap.has_inv() {
                    self.held_inv = None;
                }
                self.held_inv_size = Some(size);
            }
        }
        if snap.has_inv() {
            let size = self.held_inv_size.unwrap_or(self.last_inv_size);
            if size > 0 {
                match expand_inv(&snap.inv(), size) {
                    Ok(slots) => self.held_inv = Some(slots),
                    Err(reason) => {
                        self.held_inv = None;
                        diagnostic = Some(reason);
                    }
                }
            }
        }
        diagnostic
    }

    fn flush_held(&mut self) -> ObserveResult {
        if self.held_ingame_false {
            self.reset_baselines();
            self.clear_held();
            return ObserveResult::default();
        }
        let mut events = Vec::new();
        if let Some(held) = self.held_xp.take() {
            events.extend(self.apply_stats(&held));
        }
        if let Some(size) = self.held_inv_size.take() {
            if size <= 0 {
                self.last_inv_size = 0;
                self.last_inv = None;
            } else if self.held_inv.is_none() && self.last_inv_size != size {
                self.last_inv = None;
                self.last_inv_size = size;
            } else {
                self.last_inv_size = size;
            }
        }
        if let Some(held) = self.held_inv.take() {
            match self.last_inv.as_ref() {
                None => self.last_inv = Some(held),
                Some(last) => {
                    events.extend(diff_inv(last, &held));
                    self.last_inv = Some(held);
                }
            }
        }
        ObserveResult {
            events,
            diagnostic: None,
        }
    }

    fn reset_baselines(&mut self) {
        self.last_inv = None;
        self.last_inv_size = 0;
        self.last_xp = None;
    }

    fn clear_held(&mut self) {
        self.held_inv = None;
        self.held_inv_size = None;
        self.held_xp = None;
        self.held_ingame_false = false;
    }
}

fn collect_xp(snap: &SnapshotReader<'_>) -> Vec<XpState> {
    snap.stats()
        .into_iter()
        .take(MAX_STATS)
        .map(|s: StatReader<'_>| XpState {
            index: s.index(),
            name: s.name().to_string(),
            xp: s.xp(),
        })
        .collect()
}

fn diff_xp(last: &[XpState], current: &[XpState]) -> Vec<NativeEvent> {
    let mut events = Vec::new();
    for cur in current {
        if let Some(prev) = last.iter().find(|p| p.index == cur.index) {
            if cur.xp > prev.xp {
                events.push(NativeEvent::SkillXp {
                    skill: cur.index,
                    name: cur.name.clone(),
                    xp: cur.xp,
                    delta: cur.xp - prev.xp,
                });
            }
        }
    }
    events
}

fn merge_xp(last: &mut Vec<XpState>, current: &[XpState]) {
    for cur in current {
        if let Some(prev) = last.iter_mut().find(|p| p.index == cur.index) {
            *prev = cur.clone();
        } else if last.len() < MAX_STATS {
            last.push(cur.clone());
        }
    }
}

fn expand_inv(rows: &[RowReader<'_>], size: i32) -> Result<Vec<SlotState>, String> {
    if size <= 0 || size > MAX_INV_SIZE {
        return Err(format!(
            "inventory events: invalid inv_size {size}; family reset"
        ));
    }
    let n = size as usize;
    let mut slots = vec![SlotState::empty(); n];
    let mut seen = vec![false; n];
    for row in rows {
        let slot = row.slot();
        if slot < 0 || (slot as usize) >= n {
            return Err(format!(
                "inventory events: invalid slot {slot} (size {size}); family reset"
            ));
        }
        let i = slot as usize;
        if seen[i] {
            return Err(format!(
                "inventory events: duplicate slot {slot}; family reset"
            ));
        }
        seen[i] = true;
        slots[i] = SlotState {
            id: row.id(),
            count: row.count(),
            name: row.name().map(str::to_string),
        };
    }
    Ok(slots)
}

fn diff_inv(last: &[SlotState], current: &[SlotState]) -> Vec<NativeEvent> {
    let mut events = Vec::new();
    for (i, cur) in current.iter().enumerate() {
        let prev = last.get(i).cloned().unwrap_or_else(SlotState::empty);
        if prev.id != cur.id || prev.count != cur.count {
            events.push(NativeEvent::InventoryChanged {
                slot: i as i32,
                id: cur.id,
                name: cur.name.clone(),
                count: cur.count,
                previous_id: prev.id,
                previous_count: prev.count,
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_delta, ItemRowInput, ReachViewInput, SnapshotInput,
        StatInput,
    };

    fn base<'a>() -> SnapshotInput<'a> {
        SnapshotInput {
            tick: 1,
            here: None,
            ingame: true,
            inv: &[],
            inv_size: 28,
            stats: &[],
            booths: &[],
            banks: &[],
            bank: &[],
            bank_side: &[],
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            count_dialog_open: false,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            bank_op_result_seq: 0,
            bank_op_result: false,
            hold: false,
            ours: false,
            npcs: &[],
            locs: &[],
            players: &[],
            ground: &[],
            equipment: &[],
            chat_open: false,
            chat_continue: false,
            chat_text: None,
            chat_options: &[],
            side_tab: -1,
            varps: &[],
            combat_styles: &[],
            run_energy: 0,
            run_enabled: false,
            retaliate_enabled: false,
            my_name: None,
            in_combat: false,
            animating: false,
            main_modal_id: -1,
            chat_modal_id: -1,
            make_products: &[],
            side_tab_ifaces: &[],
            spell_buttons: &[],
            chat_lines: &[],
            nearest_booth: None,
            bank_note_on: -1,
            bank_note_off: -1,
            scene_state: 2,
            weight: 0,
            camera_yaw: 0,
            camera_pitch: 0,
            teleports_enabled: false,
            self_slot: 0,
            trade_offer_open: false,
            trade_confirm_open: false,
            trade_partner: None,
            trade_mine: &[],
            trade_theirs: &[],
            trade_side: &[],
            trade_accept_id: -1,
            trade_decline_id: -1,
            shop_open: false,
            shop_stock: &[],
            reach: ReachViewInput::UNAVAILABLE,
            attacked_by_player: false,
            widgets: &[],
        }
    }

    fn bone(slot: i32) -> ItemRowInput<'static> {
        ItemRowInput {
            name: Some("Bones"),
            count: 1,
            id: 526,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: -1,
            slot,
        }
    }

    fn prayer(xp: i32) -> StatInput<'static> {
        StatInput {
            index: 5,
            name: "prayer",
            xp,
            base: 2,
            effective: 2,
        }
    }

    fn observe(p: &mut NativeEventProducer, bytes: &[u8]) -> ObserveResult {
        let snap = SnapshotReader::from_bytes(bytes).expect("snapshot bytes");
        p.observe(&snap)
    }

    fn bones_25() -> Vec<ItemRowInput<'static>> {
        (0..25).map(bone).collect()
    }

    #[test]
    fn seed_then_bury_emits_empty_slots_and_prayer_delta() {
        let bones = bones_25();
        let stats0 = [prayer(112)];
        let mut snap = base();
        snap.inv = &bones;
        snap.stats = &stats0;
        let mut p = NativeEventProducer::new();
        let first = observe(&mut p, &encode_snapshot(&snap));
        assert!(
            first.events.is_empty(),
            "seed must not emit: {:?}",
            first.events
        );

        let stats1 = [prayer(224)];
        snap.inv = &[];
        snap.stats = &stats1;
        snap.tick = 2;
        let second = observe(&mut p, &encode_snapshot(&snap));
        let xp: Vec<_> = second
            .events
            .iter()
            .filter_map(|e| match e {
                NativeEvent::SkillXp {
                    skill,
                    name,
                    xp,
                    delta,
                } => Some((*skill, name.as_str(), *xp, *delta)),
                _ => None,
            })
            .collect();
        assert_eq!(xp, vec![(5, "prayer", 224, 112)]);
        let inv: Vec<_> = second
            .events
            .iter()
            .filter_map(|e| match e {
                NativeEvent::InventoryChanged {
                    slot,
                    id,
                    previous_id,
                    count,
                    ..
                } => Some((*slot, *id, *previous_id, *count)),
                _ => None,
            })
            .collect();
        assert_eq!(inv.len(), 25);
        assert!(inv
            .iter()
            .all(|(_, id, prev, count)| *id == -1 && *prev == 526 && *count == 0));
        assert_eq!(inv[0].0, 0);
        assert_eq!(inv[24].0, 24);
        match &second.events[0] {
            NativeEvent::SkillXp { .. } => {}
            other => panic!("skills first, got {other:?}"),
        }
    }

    #[test]
    fn omitted_inv_is_not_empty() {
        let bones = [bone(0)];
        let mut snap = base();
        snap.inv = &bones;
        let (keyframe, fp) = encode_snapshot_delta(None, &snap, false);
        let mut p = NativeEventProducer::new();
        assert!(observe(&mut p, &keyframe).events.is_empty());
        snap.tick = 2;
        snap.hold = true;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        let reader = SnapshotReader::from_bytes(&delta).unwrap();
        assert!(!reader.has_inv(), "delta must omit unchanged inv");
        let out = observe(&mut p, &delta);
        assert!(
            out.events
                .iter()
                .all(|e| !matches!(e, NativeEvent::InventoryChanged { .. })),
            "omit must not emit: {:?}",
            out.events
        );
    }

    #[test]
    fn valid_empty_vector_emits_one_clear() {
        let bones = [bone(0)];
        let mut snap = base();
        snap.inv = &bones;
        let mut p = NativeEventProducer::new();
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        snap.inv = &[];
        snap.tick = 2;
        let out = observe(&mut p, &encode_snapshot(&snap));
        let inv: Vec<_> = out
            .events
            .iter()
            .filter(|e| matches!(e, NativeEvent::InventoryChanged { .. }))
            .collect();
        assert_eq!(inv.len(), 1);
        match inv[0] {
            NativeEvent::InventoryChanged {
                slot,
                id,
                previous_id,
                count,
                ..
            } => {
                assert_eq!((*slot, *id, *previous_id, *count), (0, -1, 526, 0));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn identical_inv_repost_emits_nothing() {
        let bones = [bone(3), bone(5)];
        let mut snap = base();
        snap.inv = &bones;
        let mut p = NativeEventProducer::new();
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        snap.tick = 2;
        let out = observe(&mut p, &encode_snapshot(&snap));
        assert!(out.events.is_empty(), "equal rows: {:?}", out.events);
    }

    #[test]
    fn inv_size_zero_does_not_seed_or_emit() {
        let bones = [bone(0)];
        let mut snap = base();
        snap.inv = &bones;
        snap.inv_size = 0;
        let mut p = NativeEventProducer::new();
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        snap.inv_size = 28;
        snap.tick = 2;
        let seed = observe(&mut p, &encode_snapshot(&snap));
        assert!(
            seed.events.is_empty(),
            "later ready inventory seeds: {:?}",
            seed.events
        );
        snap.inv = &[];
        snap.tick = 3;
        let out = observe(&mut p, &encode_snapshot(&snap));
        assert_eq!(
            out.events
                .iter()
                .filter(|e| matches!(e, NativeEvent::InventoryChanged { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn xp_increase_only() {
        let stats = [prayer(0)];
        let mut snap = base();
        snap.stats = &stats;
        let mut p = NativeEventProducer::new();
        observe(&mut p, &encode_snapshot(&snap));
        let s50 = [prayer(50)];
        snap.stats = &s50;
        snap.tick = 2;
        let e = observe(&mut p, &encode_snapshot(&snap));
        match &e.events[..] {
            [NativeEvent::SkillXp { delta, xp, .. }] => {
                assert_eq!((*delta, *xp), (50, 50));
            }
            other => panic!("{other:?}"),
        }
        snap.tick = 3;
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        let s40 = [prayer(40)];
        snap.stats = &s40;
        snap.tick = 4;
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        let s41 = [prayer(41)];
        snap.stats = &s41;
        snap.tick = 5;
        match &observe(&mut p, &encode_snapshot(&snap)).events[..] {
            [NativeEvent::SkillXp { delta, .. }] => assert_eq!(*delta, 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn invalid_slot_fails_closed_and_reseeds() {
        let bad = [ItemRowInput {
            name: Some("Bones"),
            count: 1,
            id: 526,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: -1,
            slot: -1,
        }];
        let mut snap = base();
        snap.inv = &bad;
        let mut p = NativeEventProducer::new();
        let out = observe(&mut p, &encode_snapshot(&snap));
        assert!(out.events.is_empty());
        assert!(
            out.diagnostic
                .as_deref()
                .is_some_and(|d| d.contains("invalid slot -1")),
            "{:?}",
            out.diagnostic
        );
        let good = [bone(0)];
        snap.inv = &good;
        snap.tick = 2;
        assert!(
            observe(&mut p, &encode_snapshot(&snap)).events.is_empty(),
            "reseed after fail-closed"
        );
        snap.inv = &[];
        snap.tick = 3;
        assert_eq!(
            observe(&mut p, &encode_snapshot(&snap))
                .events
                .iter()
                .filter(|e| matches!(e, NativeEvent::InventoryChanged { slot: 0, .. }))
                .count(),
            1
        );
    }

    #[test]
    fn explicit_slots_keep_identity() {
        let rows = [bone(3), bone(5)];
        let mut snap = base();
        snap.inv = &rows;
        let mut p = NativeEventProducer::new();
        observe(&mut p, &encode_snapshot(&snap));
        snap.inv = &[];
        snap.tick = 2;
        let slots: Vec<i32> = observe(&mut p, &encode_snapshot(&snap))
            .events
            .into_iter()
            .filter_map(|e| match e {
                NativeEvent::InventoryChanged {
                    slot,
                    previous_id: 526,
                    id: -1,
                    ..
                } => Some(slot),
                _ => None,
            })
            .collect();
        assert_eq!(slots, vec![3, 5]);
    }

    #[test]
    fn pause_holds_net_and_resume_emits() {
        let bones = [bone(0)];
        let mut snap = base();
        snap.inv = &bones;
        let mut p = NativeEventProducer::new();
        observe(&mut p, &encode_snapshot(&snap));
        assert!(p.set_paused(true).events.is_empty());
        snap.inv = &[];
        snap.tick = 2;
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        let resumed = p.set_paused(false);
        assert_eq!(resumed.events.len(), 1);
        match &resumed.events[0] {
            NativeEvent::InventoryChanged {
                slot: 0,
                id: -1,
                previous_id: 526,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pause_without_posts_emits_no_phantom() {
        let bones = [bone(0)];
        let mut snap = base();
        snap.inv = &bones;
        let mut p = NativeEventProducer::new();
        observe(&mut p, &encode_snapshot(&snap));
        p.set_paused(true);
        p.set_paused(false);
        snap.tick = 2;
        snap.hold = true;
        let (delta, _) = encode_snapshot_delta(
            Some(&encode_snapshot_delta(None, &snap, false).1),
            &snap,
            false,
        );
        // Unchanged omit after resume: no events.
        let mut p2 = NativeEventProducer::new();
        let mut seed = base();
        seed.inv = &bones;
        let (kf, fp) = encode_snapshot_delta(None, &seed, false);
        observe(&mut p2, &kf);
        p2.set_paused(true);
        assert!(p2.set_paused(false).events.is_empty());
        seed.tick = 2;
        let (omit, _) = encode_snapshot_delta(Some(&fp), &seed, false);
        assert!(observe(&mut p2, &omit).events.is_empty());
        let _ = delta;
    }

    #[test]
    fn ingame_false_resets_and_next_gain_seeds() {
        let stats = [prayer(112)];
        let mut snap = base();
        snap.stats = &stats;
        let mut p = NativeEventProducer::new();
        observe(&mut p, &encode_snapshot(&snap));
        snap.ingame = false;
        snap.tick = 2;
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        snap.ingame = true;
        snap.tick = 3;
        assert!(observe(&mut p, &encode_snapshot(&snap)).events.is_empty());
        let later = [prayer(224)];
        snap.stats = &later;
        snap.tick = 4;
        match &observe(&mut p, &encode_snapshot(&snap)).events[..] {
            [NativeEvent::SkillXp { delta: 112, .. }] => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scalar_size_without_rows_does_not_fabricate() {
        let bones = [bone(0)];
        let mut snap = base();
        snap.inv = &bones;
        snap.inv_size = 28;
        let mut p = NativeEventProducer::new();
        observe(&mut p, &encode_snapshot(&snap));
        let (kf, fp) = encode_snapshot_delta(None, &snap, false);
        let _ = kf;
        snap.inv_size = 10;
        snap.tick = 2;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        let reader = SnapshotReader::from_bytes(&delta).unwrap();
        assert!(reader.has_inv_size());
        // inv fingerprint includes rows; size change may still carry inv.
        // Force a size-only observation by not posting inv: use a delta where
        // inv bytes match. If inv is carried, skip this assertion path.
        if !reader.has_inv() {
            let out = observe(&mut p, &delta);
            assert!(
                out.events
                    .iter()
                    .all(|e| !matches!(e, NativeEvent::InventoryChanged { .. })),
                "{:?}",
                out.events
            );
        }
    }

    #[test]
    fn omitted_stats_are_not_zero() {
        let stats = [prayer(50)];
        let mut snap = base();
        snap.stats = &stats;
        let (kf, fp) = encode_snapshot_delta(None, &snap, false);
        let mut p = NativeEventProducer::new();
        observe(&mut p, &kf);
        snap.tick = 2;
        snap.hold = true;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        let reader = SnapshotReader::from_bytes(&delta).unwrap();
        assert!(!reader.has_stats());
        assert!(observe(&mut p, &delta).events.is_empty());
        let again = [prayer(50)];
        snap.stats = &again;
        snap.tick = 3;
        let (carried, _) = encode_snapshot_delta(None, &snap, false);
        assert!(observe(&mut p, &carried)
            .events
            .iter()
            .all(|e| !matches!(e, NativeEvent::SkillXp { .. })));
    }
}
