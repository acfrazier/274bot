//! Bounded, role-local File observations and their receipt watermarks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use api::snapshot::{ChatLineView, GameSnapshot, ItemView};

use crate::file_scenario::{Container, Slots};

const MAX_MARKS: usize = 256;
pub(super) const MAX_RECORDS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub session: u64,
    pub tick: u64,
    pub source_serial: u64,
    /// The last journal record admitted before this mark.
    pub serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecordKind {
    Stat,
    Inventory,
    Varp,
    Message,
    Dialogue,
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordSource {
    Value,
    Packet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatValues {
    pub base: Option<i32>,
    pub effective: Option<i32>,
    pub xp: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub id: i32,
    pub count: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogueOption {
    pub component: i32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogueState {
    pub root: i32,
    pub continue_component: i32,
    pub lines: Vec<String>,
    pub options: Vec<DialogueOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalData {
    Stat {
        skill: i32,
        before: Option<StatValues>,
        after: StatValues,
    },
    Inventory {
        container: Container,
        component: i32,
        slot: i32,
        before: Option<ItemStack>,
        after: Option<ItemStack>,
    },
    InventoryReset {
        carried: bool,
        component: i32,
        form: PacketInventoryForm,
    },
    Message {
        sequence: i32,
        type_: i32,
        sender: Option<String>,
        text: String,
    },
    Varp {
        id: i32,
        value: i32,
    },
    Dialogue {
        before: DialogueState,
        after: DialogueState,
    },
    Session {
        previous: u64,
    },
}

impl JournalData {
    pub fn kind(&self) -> RecordKind {
        match self {
            Self::Stat { .. } => RecordKind::Stat,
            Self::Inventory { .. } | Self::InventoryReset { .. } => RecordKind::Inventory,
            Self::Message { .. } => RecordKind::Message,
            Self::Varp { .. } => RecordKind::Varp,
            Self::Dialogue { .. } => RecordKind::Dialogue,
            Self::Session { .. } => RecordKind::Session,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalRecord {
    pub serial: u64,
    pub actor: u64,
    pub run_binding: u64,
    pub session: u64,
    /// Game tick when sampled. Packet events do not invent a game tick.
    pub tick: Option<u64>,
    pub source_serial: Option<u64>,
    pub application_serial: Option<u64>,
    pub source: RecordSource,
    pub data: JournalData,
}

impl JournalRecord {
    pub fn kind(&self) -> RecordKind {
        self.data.kind()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PacketKinds {
    pub stats: bool,
    pub inventories: bool,
    pub vars: bool,
}

impl PacketKinds {
    fn includes(self, kind: RecordKind) -> bool {
        match kind {
            RecordKind::Stat => self.stats,
            RecordKind::Inventory => self.inventories,
            RecordKind::Varp => self.vars,
            RecordKind::Message | RecordKind::Dialogue | RecordKind::Session => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketInventoryForm {
    Full,
    Partial,
    StopTransmit,
}

/// One neutral decoded receipt. The host adapter copies these fields from the
/// selected client/API provider without exposing a decoder to this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketRecord {
    Stat {
        skill: i32,
        level: i32,
        xp: i32,
        source_serial: u64,
        application_serial: u64,
        client_session: u64,
    },
    InventorySlot {
        carried: bool,
        component: i32,
        slot: i32,
        id: Option<i32>,
        count: i32,
        form: PacketInventoryForm,
        source_serial: u64,
        application_serial: u64,
        client_session: u64,
    },
    InventoryReset {
        carried: bool,
        component: i32,
        form: PacketInventoryForm,
        source_serial: u64,
        application_serial: u64,
        client_session: u64,
    },
    Varp {
        id: i32,
        value: i32,
        source_serial: u64,
        application_serial: u64,
        client_session: u64,
    },
}

impl PacketRecord {
    fn kind(self) -> RecordKind {
        match self {
            Self::Stat { .. } => RecordKind::Stat,
            Self::InventorySlot { .. } | Self::InventoryReset { .. } => RecordKind::Inventory,
            Self::Varp { .. } => RecordKind::Varp,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleStat {
    skill: i32,
    values: StatValues,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleItem {
    container: Container,
    component: i32,
    slot: i32,
    value: ItemStack,
}

#[derive(Debug)]
struct Sample {
    session: u64,
    stats: Option<Vec<SampleStat>>,
    carried: Option<Vec<SampleItem>>,
    worn: Option<Vec<SampleItem>>,
    chat_sequence: Option<i32>,
    dialogue: Option<DialogueState>,
}

impl Sample {
    fn capture(snapshot: &GameSnapshot, session: u64) -> Self {
        Self {
            session,
            stats: (!snapshot.stats().is_empty()).then(|| {
                snapshot
                    .stats()
                    .iter()
                    .map(|stat| SampleStat {
                        skill: stat.index,
                        values: StatValues {
                            base: Some(stat.base),
                            effective: Some(stat.effective),
                            xp: Some(stat.xp),
                        },
                    })
                    .collect()
            }),
            carried: carried_items(snapshot).map(|rows| sample_items(rows, Container::Inventory)),
            worn: snapshot
                .equipment_posted()
                .then(|| sample_items(snapshot.equipment(), Container::Equipment)),
            chat_sequence: snapshot
                .ingame()
                .then(|| snapshot.chat_lines().iter().map(|line| line.sequence).max())
                .flatten(),
            dialogue: snapshot.ingame().then(|| dialogue_state(snapshot)),
        }
    }

    fn refresh(&mut self, snapshot: &GameSnapshot, session: u64) {
        let fresh = Self::capture(snapshot, session);
        self.session = fresh.session;
        replace_option_vec(&mut self.stats, fresh.stats);
        replace_option_vec(&mut self.carried, fresh.carried);
        replace_option_vec(&mut self.worn, fresh.worn);
        self.chat_sequence = fresh.chat_sequence;
        self.dialogue = fresh.dialogue;
    }
}

#[derive(Debug)]
pub(super) struct MarkData {
    pub(super) at: Mark,
    pub(super) action_floor: usize,
    pub(super) tile: Option<(i32, i32, i32)>,
    pub(super) xp: Vec<(i32, i32)>,
    pub(super) inventory: Option<Slots>,
    pub(super) equipment: Option<Slots>,
    pub(super) bank: Option<Slots>,
    pub(super) chat_sequence: Option<i32>,
    pub(super) packet_gap_generation: [u64; 3],
}

#[derive(Debug)]
struct Action {
    session: u64,
    id: u64,
    sent: bool,
}

#[derive(Debug)]
struct PendingRecord {
    session: u64,
    tick: Option<u64>,
    source_serial: Option<u64>,
    application_serial: Option<u64>,
    source: RecordSource,
    data: JournalData,
}

fn packet_gap_index(kind: RecordKind) -> Option<usize> {
    match kind {
        RecordKind::Stat => Some(0),
        RecordKind::Inventory => Some(1),
        RecordKind::Varp => Some(2),
        RecordKind::Message | RecordKind::Dialogue | RecordKind::Session => None,
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct PacketGap {
    serial: u64,
    generation: u64,
}

/// One role's bounded journal. Records are retained independently per kind.
#[derive(Debug, Default)]
pub struct FileJournal {
    marks: BTreeMap<u64, MarkData>,
    actions: Vec<Action>,
    records: BTreeMap<RecordKind, VecDeque<JournalRecord>>,
    evicted_through: BTreeMap<RecordKind, u64>,
    sample: Option<Sample>,
    last_serial: u64,
    packet_kinds: PacketKinds,
    packet_gaps: [PacketGap; 3],
    actor: u64,
    run_binding: u64,
}

impl FileJournal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind receipt identity before observations are recorded.
    pub fn set_identity(&mut self, actor: u64, run_binding: u64) {
        self.actor = actor;
        self.run_binding = run_binding;
    }

    pub fn actor_id(&self) -> u64 {
        self.actor
    }

    pub fn set_packet_provider(&mut self, kinds: PacketKinds) {
        self.packet_kinds = kinds;
    }

    pub fn packet_provider(&self) -> PacketKinds {
        self.packet_kinds
    }

    /// Record a decoder discontinuity at the current journal serial. Marks
    /// retain a per-kind generation so a later mark at the same serial does
    /// not inherit an earlier gap.
    pub fn note_packet_gap(&mut self, kinds: PacketKinds) {
        for (index, armed) in [kinds.stats, kinds.inventories, kinds.vars]
            .into_iter()
            .enumerate()
        {
            if armed {
                let gap = &mut self.packet_gaps[index];
                gap.serial = self.last_serial;
                gap.generation = gap.generation.saturating_add(1);
            }
        }
    }

    pub fn has_packet_provider(&self, kind: RecordKind) -> bool {
        self.packet_kinds.includes(kind)
    }

    pub fn mark(
        &mut self,
        handle: u64,
        snapshot: &GameSnapshot,
        session: u64,
        source_serial: u64,
    ) -> Result<Mark, String> {
        if self.marks.len() == MAX_MARKS || self.marks.contains_key(&handle) {
            return Err("scenario mark capacity or duplicate handle".into());
        }
        let at = Mark {
            session,
            tick: u64::from(snapshot.tick()),
            source_serial,
            serial: self.last_serial,
        };
        self.marks.insert(
            handle,
            MarkData {
                at,
                action_floor: self.actions.len(),
                tile: snapshot.tile(),
                xp: snapshot
                    .stats()
                    .iter()
                    .map(|stat| (stat.index, stat.xp))
                    .collect(),
                inventory: Container::Inventory.slots(snapshot),
                equipment: Container::Equipment.slots(snapshot),
                bank: Container::Bank.slots(snapshot),
                chat_sequence: snapshot.chat_lines().iter().map(|line| line.sequence).max(),
                packet_gap_generation: self.packet_gaps.map(|gap| gap.generation),
            },
        );
        Ok(at)
    }

    pub fn mark_at(&self, handle: u64) -> Option<Mark> {
        self.marks.get(&handle).map(|mark| mark.at)
    }
    pub fn last_serial(&self) -> u64 {
        self.last_serial
    }

    pub(super) fn mark_any(&self, handle: u64) -> Result<&MarkData, String> {
        self.marks
            .get(&handle)
            .ok_or_else(|| format!("unknown mark {handle}"))
    }

    pub(super) fn has_packet_gap_since(&self, mark: &MarkData, kind: RecordKind) -> bool {
        packet_gap_index(kind).is_some_and(|index| {
            self.packet_gaps[index].generation > mark.packet_gap_generation[index]
        })
    }

    pub fn has_session_change_after(&self, serial: u64, session: u64) -> bool {
        self.records_after(RecordKind::Session, serial)
            .any(|record| {
                matches!(
                    record.data,
                    JournalData::Session { previous } if previous == session
                )
            })
    }

    pub fn record_action(&mut self, session: u64, id: u64, sent: bool) -> Result<(), String> {
        if self.actions.len() == MAX_RECORDS
            || self
                .actions
                .last()
                .is_some_and(|last| last.id == id && last.session == session)
        {
            return Err("scenario action journal capacity or duplicate root".into());
        }
        self.actions.push(Action { session, id, sent });
        Ok(())
    }

    pub fn note_sent(&mut self, session: u64, id: u64) -> Result<(), String> {
        let action = self
            .actions
            .last_mut()
            .filter(|action| action.session == session && action.id == id && !action.sent)
            .ok_or("action send does not match an admitted root")?;
        action.sent = true;
        Ok(())
    }

    pub(super) fn get_mark(&self, handle: u64, session: u64) -> Result<&MarkData, String> {
        self.marks
            .get(&handle)
            .filter(|mark| mark.at.session == session)
            .ok_or_else(|| format!("unknown or foreign-session mark {handle}"))
    }

    pub fn action_sent(&self, handle: u64, session: u64) -> Result<bool, String> {
        let mark = self.get_mark(handle, session)?;
        match &self.actions[mark.action_floor..] {
            [] => Ok(false),
            [action] if action.session == session => Ok(action.sent),
            _ => Err("action_sent: expected one matching root with a native send".into()),
        }
    }

    /// Add sampled endpoint differences. An unchanged sample does not allocate.
    pub fn observe(&mut self, snapshot: &GameSnapshot, session: u64) -> Result<(), String> {
        let Some(previous) = self.sample.as_ref() else {
            self.sample = Some(Sample::capture(snapshot, session));
            return Ok(());
        };
        if previous.session != session {
            let prior = previous.session;
            self.push(PendingRecord {
                session,
                tick: Some(u64::from(snapshot.tick())),
                source_serial: None,
                application_serial: None,
                source: RecordSource::Value,
                data: JournalData::Session { previous: prior },
            })?;
            self.sample
                .as_mut()
                .expect("sample exists")
                .refresh(snapshot, session);
            return Ok(());
        }

        let stats_missing = previous.stats.is_none();
        let carried_missing = previous.carried.is_none();
        let worn_missing = previous.worn.is_none();
        let mut pending = Vec::new();
        let mut stats_changed = false;
        let mut carried_changed = false;
        let mut worn_changed = false;
        let mut chat_sequence = previous.chat_sequence;
        let mut dialogue_changed = false;

        if !snapshot.stats().is_empty() {
            if let Some(old) = &previous.stats {
                for stat in snapshot.stats() {
                    let after = StatValues {
                        base: Some(stat.base),
                        effective: Some(stat.effective),
                        xp: Some(stat.xp),
                    };
                    let before = old
                        .iter()
                        .find(|row| row.skill == stat.index)
                        .map(|row| row.values);
                    if before != Some(after) {
                        pending.push(PendingRecord {
                            session,
                            tick: Some(u64::from(snapshot.tick())),
                            source_serial: None,
                            application_serial: None,
                            source: RecordSource::Value,
                            data: JournalData::Stat {
                                skill: stat.index,
                                before,
                                after,
                            },
                        });
                        stats_changed = true;
                    }
                }
                stats_changed |= old.len() != snapshot.stats().len();
            }
        }

        if let (Some(old), Some(items)) = (&previous.carried, carried_items(snapshot)) {
            carried_changed = append_item_differences(
                &mut pending,
                old,
                items,
                Container::Inventory,
                session,
                u64::from(snapshot.tick()),
            );
        }
        if let (Some(old), true) = (&previous.worn, snapshot.equipment_posted()) {
            worn_changed = append_item_differences(
                &mut pending,
                old,
                snapshot.equipment(),
                Container::Equipment,
                session,
                u64::from(snapshot.tick()),
            );
        }

        if snapshot.ingame() {
            let start = previous.chat_sequence.unwrap_or(i32::MIN);
            let mut newest = previous.chat_sequence;
            for line in snapshot.chat_lines().iter().rev() {
                if line.sequence > start {
                    pending.push(message_record(line, session, snapshot.tick()));
                    newest = Some(newest.map_or(line.sequence, |value| value.max(line.sequence)));
                }
            }
            chat_sequence = newest;

            if previous
                .dialogue
                .as_ref()
                .is_some_and(|old| !dialogue_matches(snapshot, old))
            {
                let old = previous.dialogue.as_ref().expect("dialogue was compared");
                pending.push(PendingRecord {
                    session,
                    tick: Some(u64::from(snapshot.tick())),
                    source_serial: None,
                    application_serial: None,
                    source: RecordSource::Value,
                    data: JournalData::Dialogue {
                        before: old.clone(),
                        after: dialogue_state(snapshot),
                    },
                });
                dialogue_changed = true;
            }
        }

        let pending_count =
            u64::try_from(pending.len()).map_err(|_| "scenario journal serial exhausted")?;
        self.last_serial
            .checked_add(pending_count)
            .ok_or("scenario journal serial exhausted")?;
        for record in pending {
            self.push(record)?;
        }
        if stats_changed || stats_missing && !snapshot.stats().is_empty() {
            let sample = self.sample.as_mut().expect("sample exists");
            let rows = sample.stats.get_or_insert_with(Vec::new);
            rows.clear();
            rows.extend(snapshot.stats().iter().map(|stat| SampleStat {
                skill: stat.index,
                values: StatValues {
                    base: Some(stat.base),
                    effective: Some(stat.effective),
                    xp: Some(stat.xp),
                },
            }));
        } else if snapshot.stats().is_empty() {
            self.sample.as_mut().expect("sample exists").stats = None;
        }
        if carried_changed || carried_missing && carried_items(snapshot).is_some() {
            let sample = self.sample.as_mut().expect("sample exists");
            let rows = sample.carried.get_or_insert_with(Vec::new);
            rows.clear();
            rows.extend(
                carried_items(snapshot)
                    .unwrap_or_default()
                    .iter()
                    .map(|item| SampleItem {
                        container: Container::Inventory,
                        component: item.component_id,
                        slot: item.slot,
                        value: ItemStack {
                            id: item.def.id,
                            count: item.count,
                        },
                    }),
            );
        } else if carried_items(snapshot).is_none() {
            self.sample.as_mut().expect("sample exists").carried = None;
        }
        if worn_changed || worn_missing && snapshot.equipment_posted() {
            let sample = self.sample.as_mut().expect("sample exists");
            let rows = sample.worn.get_or_insert_with(Vec::new);
            rows.clear();
            rows.extend(snapshot.equipment().iter().map(|item| SampleItem {
                container: Container::Equipment,
                component: item.component_id,
                slot: item.slot,
                value: ItemStack {
                    id: item.def.id,
                    count: item.count,
                },
            }));
        } else if !snapshot.equipment_posted() {
            self.sample.as_mut().expect("sample exists").worn = None;
        }
        if snapshot.ingame() {
            let sample = self.sample.as_mut().expect("sample exists");
            sample.chat_sequence = chat_sequence;
            if dialogue_changed || sample.dialogue.is_none() {
                sample.dialogue = Some(dialogue_state(snapshot));
            }
        } else {
            let sample = self.sample.as_mut().expect("sample exists");
            sample.chat_sequence = None;
            sample.dialogue = None;
        }
        Ok(())
    }

    /// Add one selected decoded event without coalescing repeated values.
    pub fn observe_packet(&mut self, record: PacketRecord) -> Result<(), String> {
        let kind = record.kind();
        if !self.packet_kinds.includes(kind) {
            return Err(format!("packet {kind:?} provider is not armed"));
        }
        let (session, source_serial, application_serial, data) = match record {
            PacketRecord::Stat {
                skill,
                level,
                xp,
                source_serial,
                application_serial,
                client_session,
            } => (
                client_session,
                source_serial,
                application_serial,
                JournalData::Stat {
                    skill,
                    before: None,
                    after: StatValues {
                        base: None,
                        effective: Some(level),
                        xp: Some(xp),
                    },
                },
            ),
            PacketRecord::InventorySlot {
                carried,
                component,
                slot,
                id,
                count,
                form: _,
                source_serial,
                application_serial,
                client_session,
            } => (
                client_session,
                source_serial,
                application_serial,
                JournalData::Inventory {
                    container: if carried {
                        Container::Inventory
                    } else {
                        Container::Equipment
                    },
                    component,
                    slot,
                    before: None,
                    after: id.filter(|_| count > 0).map(|id| ItemStack { id, count }),
                },
            ),
            PacketRecord::InventoryReset {
                carried,
                component,
                form,
                source_serial,
                application_serial,
                client_session,
            } => (
                client_session,
                source_serial,
                application_serial,
                JournalData::InventoryReset {
                    carried,
                    component,
                    form,
                },
            ),
            PacketRecord::Varp {
                id,
                value,
                source_serial,
                application_serial,
                client_session,
            } => (
                client_session,
                source_serial,
                application_serial,
                JournalData::Varp { id, value },
            ),
        };
        self.push(PendingRecord {
            session,
            tick: None,
            source_serial: Some(source_serial),
            application_serial: Some(application_serial),
            source: RecordSource::Packet,
            data,
        })?;
        Ok(())
    }

    pub fn records(&self) -> impl Iterator<Item = &JournalRecord> {
        self.records.values().flat_map(|rows| rows.iter())
    }

    pub fn records_after(
        &self,
        kind: RecordKind,
        serial: u64,
    ) -> impl Iterator<Item = &JournalRecord> {
        self.records
            .get(&kind)
            .into_iter()
            .flat_map(|rows| rows.iter())
            .filter(move |record| record.serial > serial)
    }

    pub fn record_count(&self, kind: RecordKind) -> usize {
        self.records.get(&kind).map_or(0, VecDeque::len)
    }

    /// Retained collection capacity, for idle-allocation conformance checks.
    pub fn capacity(&self) -> usize {
        self.records.values().map(VecDeque::capacity).sum::<usize>() + self.actions.capacity()
    }

    pub fn history_evicted(&self, kind: RecordKind, after_serial: u64) -> bool {
        self.evicted_through
            .get(&kind)
            .is_some_and(|serial| *serial > after_serial)
    }

    fn push(&mut self, pending: PendingRecord) -> Result<u64, String> {
        let serial = self
            .last_serial
            .checked_add(1)
            .ok_or("scenario journal serial exhausted")?;
        let kind = pending.data.kind();
        let rows = self.records.entry(kind).or_default();
        if rows.len() == MAX_RECORDS {
            let evicted = rows.pop_front().expect("full record queue");
            self.evicted_through.insert(kind, evicted.serial);
        }
        rows.push_back(JournalRecord {
            serial,
            actor: self.actor,
            run_binding: self.run_binding,
            session: pending.session,
            tick: pending.tick,
            source_serial: pending.source_serial,
            application_serial: pending.application_serial,
            source: pending.source,
            data: pending.data,
        });
        self.last_serial = serial;
        Ok(serial)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckMatch {
    pub actor: u64,
    pub session: u64,
    pub tick: u64,
    pub serials: Vec<u64>,
}

/// The caller retains labelled held-check matches and signal observations.
#[derive(Clone, Debug, Default)]
pub struct CheckContext {
    labels: BTreeMap<String, CheckMatch>,
    signals: BTreeSet<String>,
}

impl CheckContext {
    pub fn record(
        &mut self,
        label: impl Into<String>,
        actor: u64,
        session: u64,
        tick: u64,
        serials: Vec<u64>,
    ) {
        self.labels.insert(
            label.into(),
            CheckMatch {
                actor,
                session,
                tick,
                serials,
            },
        );
    }

    pub fn label(&self, label: &str) -> Option<&CheckMatch> {
        self.labels.get(label)
    }

    pub fn signal(&mut self, name: impl Into<String>) {
        self.signals.insert(name.into());
    }

    pub fn has_signal(&self, name: &str) -> bool {
        self.signals.contains(name)
    }
}

fn replace_option_vec<T>(target: &mut Option<Vec<T>>, source: Option<Vec<T>>) {
    match source {
        Some(source) => {
            if let Some(target) = target.as_mut() {
                target.clear();
                target.extend(source);
            } else {
                *target = Some(source);
            }
        }
        None => *target = None,
    }
}

fn carried_items(snapshot: &GameSnapshot) -> Option<&[ItemView]> {
    (snapshot.inventory_ready() && snapshot.inventory_size() > 0).then(|| snapshot.inventory())
}

fn sample_items(items: &[ItemView], container: Container) -> Vec<SampleItem> {
    items
        .iter()
        .map(|item| SampleItem {
            container,
            component: item.component_id,
            slot: item.slot,
            value: ItemStack {
                id: item.def.id,
                count: item.count,
            },
        })
        .collect()
}

fn append_item_differences(
    pending: &mut Vec<PendingRecord>,
    before: &[SampleItem],
    after: &[ItemView],
    container: Container,
    session: u64,
    tick: u64,
) -> bool {
    let mut changed = false;
    for old in before {
        let current = after
            .iter()
            .find(|item| item.component_id == old.component && item.slot == old.slot);
        let next = current.map(|item| ItemStack {
            id: item.def.id,
            count: item.count,
        });
        if next != Some(old.value) {
            pending.push(PendingRecord {
                session,
                tick: Some(tick),
                source_serial: None,
                application_serial: None,
                source: RecordSource::Value,
                data: JournalData::Inventory {
                    container,
                    component: old.component,
                    slot: old.slot,
                    before: Some(old.value),
                    after: next,
                },
            });
            changed = true;
        }
    }
    for item in after {
        if !before
            .iter()
            .any(|old| old.component == item.component_id && old.slot == item.slot)
        {
            pending.push(PendingRecord {
                session,
                tick: Some(tick),
                source_serial: None,
                application_serial: None,
                source: RecordSource::Value,
                data: JournalData::Inventory {
                    container,
                    component: item.component_id,
                    slot: item.slot,
                    before: None,
                    after: Some(ItemStack {
                        id: item.def.id,
                        count: item.count,
                    }),
                },
            });
            changed = true;
        }
    }
    changed
}

fn message_record(line: &ChatLineView, session: u64, tick: u32) -> PendingRecord {
    PendingRecord {
        session,
        tick: Some(u64::from(tick)),
        source_serial: None,
        application_serial: None,
        source: RecordSource::Value,
        data: JournalData::Message {
            sequence: line.sequence,
            type_: line.type_,
            sender: line.username.clone(),
            text: line.text.clone(),
        },
    }
}

fn dialogue_state(snapshot: &GameSnapshot) -> DialogueState {
    DialogueState {
        root: snapshot.modals().chat,
        continue_component: snapshot.chat_continue_component_id(),
        lines: snapshot.chat_modal_texts().to_vec(),
        options: snapshot
            .chat_options()
            .iter()
            .map(|option| DialogueOption {
                component: option.component_id,
                text: option.text.clone(),
            })
            .collect(),
    }
}
fn dialogue_matches(snapshot: &GameSnapshot, state: &DialogueState) -> bool {
    state.root == snapshot.modals().chat
        && state.continue_component == snapshot.chat_continue_component_id()
        && state.lines.as_slice() == snapshot.chat_modal_texts()
        && state.options.len() == snapshot.chat_options().len()
        && state
            .options
            .iter()
            .zip(snapshot.chat_options())
            .all(|(old, new)| old.component == new.component_id && old.text == new.text)
}

#[cfg(test)]
#[path = "file_journal_tests.rs"]
mod tests;
