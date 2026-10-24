//! JavaScript cards and the run that carries their isolate and wire state.

use super::isolate::{GameplayFactory, LoadIsolate, Ready};
use super::{ApiFamily, LoadShape};
use crate::interact::{InteractReq, QueuedInteract};
use crate::isolate_fb::{IsolateBuf, SnapshotFingerprint};
use crate::native::recovery_hints::RecoveryHintsCell;
use crate::native::seat_rows::QueuedSeatBatch;
use crate::native::snapshot_source::{NativeFactsInput, SnapshotInput};
use crate::native::SettingsBag;
use crate::paint::ScriptPaint;
use crate::surface::{
    PostedSnapshotFacts, RouteInspectIdentities, ScriptStopReceipt, SurfaceCard, SurfaceError,
    SurfaceFactory, SurfaceFamily, SurfaceKind, SurfaceReady, SurfaceRun, SurfaceStart,
    TickOutcome,
};
use api::identity::ExecutionId;
use std::any::Any;
use std::sync::{mpsc::Sender, Arc};
use std::time::Instant;

/// Frozen JavaScript inputs retained by the slot across run restarts.
#[derive(Debug)]
pub struct JsStartCard {
    pub source: Arc<str>,
    pub shape: LoadShape,
    pub siblings: Arc<[(String, String)]>,
    pub family: Option<ApiFamily>,
}

impl SurfaceCard for JsStartCard {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Js
    }

    fn family(&self) -> Option<SurfaceFamily> {
        match self.family {
            Some(ApiFamily::V1) => Some(SurfaceFamily::JsV1),
            Some(ApiFamily::V2) => Some(SurfaceFamily::JsV2),
            Some(ApiFamily::Unversioned) | None => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Creates gameplay isolates for JavaScript cards.
#[derive(Default)]
pub struct JsSurfaceFactory {
    gameplay: GameplayFactory,
}

impl SurfaceFactory for JsSurfaceFactory {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Js
    }

    fn spawn(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
        recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        let card = start
            .card
            .as_any()
            .downcast_ref::<JsStartCard>()
            .ok_or(SurfaceError::ForeignCard(SurfaceKind::Js))?;
        let isolate = self
            .gameplay
            .spawn_with_content(
                card.source.to_string(),
                card.shape,
                card.siblings.iter().cloned().collect(),
                start.game_data.clone(),
                Arc::clone(&start.named_banks),
                Some(execution),
            )
            .map_err(|error| SurfaceError::Spawn(error.to_string()))?;
        isolate.post_loadouts(&start.loadouts);
        isolate.post_recovery_hints(recovery_hints);
        if let Some(bag) = start.settings_bag.as_deref() {
            isolate.post_settings_bag(bag);
        }
        Ok(Box::new(JsSurfaceRun {
            isolate,
            ipc: IsolateBuf::new(),
            last_snapshot: None,
        }))
    }
}

/// One JavaScript run. Its builder and fingerprint never survive retirement.
pub struct JsSurfaceRun {
    isolate: LoadIsolate,
    ipc: IsolateBuf,
    last_snapshot: Option<SnapshotFingerprint>,
}

impl JsSurfaceRun {
    /// Give a completion-path test a wider teardown-hook deadline.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_onstop_timeout_for_test(&mut self, timeout: std::time::Duration) {
        self.isolate.set_onstop_timeout_for_test(timeout);
    }

    fn post_encoded_snapshot(
        &mut self,
        bytes: Vec<u8>,
        fingerprint: SnapshotFingerprint,
        evidence_changed: bool,
    ) -> bool {
        self.last_snapshot = Some(fingerprint);
        self.isolate
            .record_snapshot_evidence_change(evidence_changed);
        let accepted = self.isolate.post_snapshot(bytes);
        if !accepted {
            self.last_snapshot = None;
        }
        accepted
    }
}

/// Post through the real JS wire owner and inspect the emitted gather pages.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn post_snapshot_delta_pages_for_test(
    slot: &mut crate::SlotScript,
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
    force_banks: bool,
) -> (bool, bool) {
    assert!(
        slot.post_snapshot_delta(input, native, force_banks),
        "reconnect keyframe is accepted by the real isolate"
    );
    slot.with_surface_run_for_test(|run| {
        let run = run
            .as_any_mut()
            .downcast_mut::<JsSurfaceRun>()
            .expect("JavaScript surface run");
        let encoded = crate::isolate_fb::Snapshot::from_bytes(
            run.ipc.finished_snapshot_bytes_for_test(),
        )
        .expect("the accepted snapshot remains in its builder");
        (
            encoded
                .api_gather()
                .is_some_and(|page| page.request_id() != 0),
            encoded.api_gather_outcome().is_some(),
        )
    })
    .expect("active surface run")
}

impl SurfaceRun for JsSurfaceRun {
    fn poll_ready(&self) -> SurfaceReady {
        match self.isolate.poll_ready() {
            Ready::Pending => SurfaceReady::Pending,
            Ready::Ready => SurfaceReady::Ready,
            Ready::Failed(message) => SurfaceReady::Failed(message),
        }
    }

    fn generation(&self) -> u64 {
        self.isolate.generation()
    }

    fn execution_active(&self) -> bool {
        self.isolate.execution_active()
    }

    fn execution_sequence(&self) -> (u64, bool) {
        self.isolate.execution_sequence()
    }

    fn align_execution(&self, execution: ExecutionId) {
        self.isolate.align_execution(execution);
    }

    fn reset_session_work(&mut self, execution: Option<ExecutionId>) -> u64 {
        self.last_snapshot = None;
        self.isolate.reset_session_work(execution)
    }

    fn reconnect_session_work(&mut self, execution: Option<ExecutionId>) -> u64 {
        self.last_snapshot = None;
        self.isolate.reconnect_session_work(execution)
    }

    fn pause(&self) {
        self.isolate.pause();
    }

    fn resume(&self) {
        self.isolate.resume();
    }

    fn stop(self: Box<Self>, done: Sender<Vec<String>>) {
        let Self {
            isolate,
            ipc,
            last_snapshot,
        } = *self;
        drop(ipc);
        drop(last_snapshot);
        isolate.join_detached(done);
    }

    fn stopped(&self) -> bool {
        self.isolate.stopped()
    }

    fn script_stop_receipt(&self) -> Option<ScriptStopReceipt> {
        self.isolate.script_stop_receipt()
    }

    fn take_script_cut(&self) -> bool {
        self.isolate.take_script_cut()
    }

    fn post_snapshot_delta(
        &mut self,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        force_banks: bool,
    ) -> bool {
        let (bytes, fingerprint, evidence_changed) =
            self.ipc.encode_snapshot_delta_with_native_and_evidence(
                self.last_snapshot.as_ref(),
                input,
                native,
                force_banks,
            );
        self.post_encoded_snapshot(bytes, fingerprint, evidence_changed)
    }

    fn post_snapshot_wake(
        &mut self,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        force_banks: bool,
        preserve_inv: bool,
    ) -> bool {
        let (bytes, fingerprint, evidence_changed) =
            self.ipc.encode_snapshot_wake_with_native_and_evidence(
                self.last_snapshot.as_mut(),
                input,
                native,
                force_banks,
                preserve_inv,
            );
        self.post_encoded_snapshot(bytes, fingerprint, evidence_changed)
    }

    fn posted_snapshot_facts(&self) -> Option<PostedSnapshotFacts> {
        self.last_snapshot.as_ref().map(|fingerprint| {
            let route = &fingerprint.route_inspect;
            PostedSnapshotFacts {
                withdraw_x_result_seq: fingerprint.withdraw_x_result_seq,
                bank_op_result_seq: fingerprint.bank_op_result_seq,
                walk_outcome_seq: fingerprint.walk_outcome_seq,
                bank_selection: fingerprint.bank_selection,
                route_inspect: RouteInspectIdentities {
                    latest: (
                        route.latest.seq,
                        route.latest.generation,
                        route.latest.request_id,
                    ),
                    prev: (route.prev.seq, route.prev.generation, route.prev.request_id),
                    replaced_id: route.replaced_id,
                    replaced_prev_id: route.replaced_prev_id,
                    refused: [route.refused_id, route.refused_id_2, route.refused_id_3],
                },
            }
        })
    }

    fn post_channel_events(&self, events: &[InteractReq]) -> bool {
        self.isolate
            .post_channel_events(crate::isolate_fb::encode_interact_batch(events))
    }

    fn post_settings_bag(&self, bag: &SettingsBag) {
        self.isolate.post_settings_bag(bag);
    }

    fn post_loadouts(&self, loadouts: &[api::loadout::Loadout]) {
        self.isolate.post_loadouts(loadouts);
    }

    fn on_game_tick_at(&self, snap_tick: u64, input_identity: u64) {
        self.isolate.on_game_tick_at(snap_tick, input_identity);
    }

    fn on_snapshot_change_at(&self, snap_tick: u64, input_identity: u64) {
        self.isolate.on_snapshot_change_at(
            snap_tick,
            input_identity,
            self.isolate.snapshot_evidence_sequence(),
        );
    }

    fn drain_interacts_stamped(&self) -> Vec<QueuedInteract> {
        self.isolate.drain_interacts_stamped()
    }

    fn restore_interacts_stamped(&self, drained: Vec<QueuedInteract>) {
        self.isolate.restore_interacts_stamped(drained);
    }

    fn drain_seat_batches(&self) -> Vec<QueuedSeatBatch> {
        self.isolate.drain_seat_batches()
    }

    fn restore_seat_batches(&self, drained: Vec<QueuedSeatBatch>) {
        self.isolate.restore_seat_batches(drained);
    }

    fn drain_lifecycle(&self) -> Vec<InteractReq> {
        self.isolate.drain_lifecycle()
    }

    fn drain_tick_outcomes(&self) -> Vec<TickOutcome> {
        self.isolate.drain_tick_outcomes()
    }

    fn discard_mouse_interacts(&self) {
        self.isolate.discard_mouse_interacts();
    }

    fn rearm_host_discarded_walk(&self, token: u64) {
        self.isolate.rearm_host_discarded_walk(token);
    }

    fn request_recovery_anchor(&self) {
        self.isolate.request_recovery_anchor();
    }

    fn live_walking_operation(&self) -> bool {
        self.isolate.live_walking_operation()
    }

    fn note_manual_walk_takeover(&self, user_move_intent_seq: u64) {
        self.isolate.note_manual_walk_takeover(user_move_intent_seq);
    }

    fn paint(&self) -> Option<Arc<ScriptPaint>> {
        self.isolate.paint()
    }

    fn paint_click(&self, id: &str) {
        self.isolate.paint_click(id);
    }

    fn paint_select(&self, key: &str, name: &str) {
        self.isolate.paint_select(key, name);
    }

    fn journal_paint_hidden(&self, now: Instant) -> bool {
        self.isolate.compat_journal_paint_hidden(now)
    }

    fn probe(&self, expr: &str) -> Result<serde_json::Value, SurfaceError> {
        self.isolate.probe(expr).map_err(SurfaceError::Probe)
    }

    fn drain_logs(&self) -> Vec<String> {
        self.isolate.drain_logs()
    }

    fn ignored_randoms(&self) -> Vec<String> {
        self.isolate.ignored_randoms()
    }

    fn ignores_random(&self, name: &str) -> bool {
        self.isolate.ignores_random(name)
    }

    fn memory_metrics(&self) -> Option<serde_json::Value> {
        #[cfg(feature = "memory-profile")]
        {
            Some(self.isolate.memory_metrics())
        }
        #[cfg(not(feature = "memory-profile"))]
        {
            None
        }
    }

    fn memory_progress(&self) -> serde_json::Value {
        #[cfg(feature = "memory-profile")]
        {
            let mut value = self.isolate.memory_progress();
            if let Some(fingerprint) = &self.last_snapshot {
                let rows = |rows: &[crate::isolate_fb::ItemRowFp]| {
                    rows.iter()
                        .take(32)
                        .map(|row| serde_json::json!({"name":row.name,"count":row.count,"ops":row.ops}))
                        .collect::<Vec<_>>()
                };
                value["inventory"] = serde_json::json!(rows(&fingerprint.inv));
                value["bank"] = serde_json::json!(rows(&fingerprint.bank));
                value["bank_open"] = serde_json::json!(fingerprint.bank_open);
                value["bank_loaded"] = serde_json::json!(fingerprint.bank_loaded);
            }
            value
        }
        #[cfg(not(feature = "memory-profile"))]
        {
            serde_json::json!({})
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    #[cfg(feature = "test-hooks")]
    fn seat_batch_backlog(&self) -> usize {
        self.isolate.seat_batch_backlog()
    }

    #[cfg(feature = "test-hooks")]
    fn held_fold_len(&self) -> usize {
        self.isolate.held_fold_len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::isolate_fb::{InspectHopFp, RouteInspectFp, RouteInspectTerminalFp};
    use crate::native::snapshot_source::BankSelectionInput;
    use std::time::Duration;

    fn run(source: &str) -> JsSurfaceRun {
        let isolate = GameplayFactory
            .spawn_with_content(
                source.into(),
                LoadShape::NativeTick,
                Vec::new(),
                None,
                Arc::default(),
                None,
            )
            .expect("gameplay isolate");
        isolate.post_loadouts(&[]);
        isolate.post_recovery_hints(Arc::new(RecoveryHintsCell::new()));
        let run = JsSurfaceRun {
            isolate,
            ipc: IsolateBuf::new(),
            last_snapshot: None,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match run.poll_ready() {
                SurfaceReady::Ready => return run,
                SurfaceReady::Failed(message) => panic!("surface setup failed: {message}"),
                SurfaceReady::Pending => {
                    assert!(Instant::now() < deadline, "surface setup stalled");
                    std::thread::yield_now();
                }
            }
        }
    }

    fn retire(run: JsSurfaceRun) {
        let (done, retired) = std::sync::mpsc::channel();
        Box::new(run).stop(done);
        retired
            .recv_timeout(Duration::from_secs(10))
            .expect("surface retirement");
    }

    fn encode(
        run: &mut JsSurfaceRun,
        input: &SnapshotInput<'_>,
    ) -> (Vec<u8>, SnapshotFingerprint, bool) {
        run.ipc.encode_snapshot_delta_with_native_and_evidence(
            run.last_snapshot.as_ref(),
            input,
            NativeFactsInput::default(),
            false,
        )
    }

    fn encode_and_post(run: &mut JsSurfaceRun, input: &SnapshotInput<'_>) -> Vec<u8> {
        let (bytes, fingerprint, evidence_changed) = encode(run, input);
        let owned = bytes.clone();
        assert!(run.post_encoded_snapshot(bytes, fingerprint, evidence_changed));
        owned
    }

    #[test]
    fn run_layout_contains_only_the_three_owned_fields() {
        let fields = std::mem::size_of::<LoadIsolate>()
            + std::mem::size_of::<IsolateBuf>()
            + std::mem::size_of::<Option<SnapshotFingerprint>>();
        let alignment = std::mem::align_of::<JsSurfaceRun>();
        assert_eq!(
            std::mem::size_of::<JsSurfaceRun>(),
            fields.div_ceil(alignment) * alignment
        );
    }

    #[test]
    fn populated_fingerprint_fact_reads_and_host_comparisons_allocate_nothing() {
        let mut run = run("export function tick() {}");
        let hop = |from_x, to_x| InspectHopFp {
            kind: "loc".into(),
            loc_id: 1170,
            loc_name: "Door".into(),
            action: "Open".into(),
            from_x,
            from_z: 3200,
            to_x,
            to_z: 3200,
            ..Default::default()
        };
        let mut fingerprint = SnapshotFingerprint::default();
        fingerprint.withdraw_x_result_seq = 11;
        fingerprint.bank_op_result_seq = 12;
        fingerprint.walk_outcome_seq = 13;
        fingerprint.bank_selection = BankSelectionInput {
            request_id: 14,
            generation: 15,
            bank_index: 2,
            kind: 3,
        };
        fingerprint.route_inspect = RouteInspectFp {
            latest: RouteInspectTerminalFp {
                seq: 21,
                generation: 22,
                request_id: 23,
                ok: true,
                reason: "arrived".into(),
                hops: vec![hop(3200, 3201), hop(3201, 3202)],
                ..Default::default()
            },
            prev: RouteInspectTerminalFp {
                seq: 31,
                generation: 32,
                request_id: 33,
                reason: "blocked".into(),
                hops: vec![hop(3202, 3203), hop(3203, 3204)],
                ..Default::default()
            },
            replaced_id: 41,
            replaced_prev_id: 42,
            refused_id: 51,
            refused_id_2: 52,
            refused_id_3: 53,
            ..Default::default()
        };
        assert_eq!(fingerprint.route_inspect.latest.hops.len(), 2);
        assert_eq!(fingerprint.route_inspect.prev.hops.len(), 2);
        run.last_snapshot = Some(fingerprint);
        let compare_host_facts = |facts: Option<PostedSnapshotFacts>| {
            facts.is_some_and(|last| {
                let route = last.route_inspect;
                last.bank_selection
                    == (BankSelectionInput {
                        request_id: 14,
                        generation: 15,
                        bank_index: 2,
                        kind: 3,
                    })
                    && last.withdraw_x_result_seq == 11
                    && last.bank_op_result_seq == 12
                    && last.walk_outcome_seq == 13
                    && route.latest == (21, 22, 23)
                    && route.prev == (31, 32, 33)
                    && route.replaced_id == 41
                    && route.replaced_prev_id == 42
                    && route.refused == [51, 52, 53]
            })
        };
        let mut compared = [false; 3];
        let allocations = allocation_counter::measure(|| {
            for result in &mut compared {
                *result = compare_host_facts(run.posted_snapshot_facts());
            }
        });
        assert_eq!(allocations.count_total, 0, "fact reads allocate");
        assert_eq!(allocations.bytes_total, 0, "fact reads allocate bytes");
        assert_eq!(compared, [true; 3]);
        retire(run);
    }

    #[test]
    fn session_boundaries_clear_the_fingerprint_and_force_a_keyframe() {
        let mut run = run("export function tick() {}");
        let text = "x".repeat(64 * 1024);
        let mut input = crate::isolate_fb::tests::empty_input(1);
        input.chat_text = Some(&text);
        let first = encode_and_post(&mut run, &input);
        assert!(first.len() > text.len());
        for reconnect in [false, true] {
            let delta = encode_and_post(&mut run, &input);
            assert!(delta.len() < 1024);
            assert!(run.posted_snapshot_facts().is_some());
            if reconnect {
                run.reconnect_session_work(None);
            } else {
                run.reset_session_work(None);
            }
            assert_eq!(run.posted_snapshot_facts(), None);
            assert_eq!(encode_and_post(&mut run, &input), first);
        }
        retire(run);
    }

    pub(crate) fn assert_stop_releases_snapshot_storage_and_restart_emits_keyframe() {
        let mut run = run("export function tick() {}");
        let text = "x".repeat(1024 * 1024);
        let mut input = crate::isolate_fb::tests::empty_input(1);
        input.chat_text = Some(&text);
        let first = encode_and_post(&mut run, &input);
        assert!(first.len() > text.len());
        run.pause();
        assert!(run.posted_snapshot_facts().is_some());
        run.resume();
        let delta = encode_and_post(&mut run, &input);
        assert!(delta.len() < 1024);
        let fingerprint = run.last_snapshot.take().expect("populated fingerprint");
        let released = allocation_counter::measure(|| retire(run));
        assert!(
            released.bytes_current < -(text.len() as i64 / 2),
            "retirement must release the warmed builder: {released:?}"
        );
        drop(fingerprint);
        let mut replacement = self::run("export function tick() {}");
        assert_eq!(
            std::mem::take(&mut replacement.ipc).into_backing_capacity(),
            0
        );
        assert_eq!(replacement.posted_snapshot_facts(), None);
        assert_eq!(encode_and_post(&mut replacement, &input), first);
        assert!(crate::isolate_fb::Snapshot::from_bytes(&first).is_ok());
        retire(replacement);

        let mut fingerprint_only = self::run("export function tick() {}");
        let mut fingerprint = SnapshotFingerprint::default();
        fingerprint.chat_text = Some(text);
        fingerprint_only.last_snapshot = Some(fingerprint);
        let released = allocation_counter::measure(|| retire(fingerprint_only));
        assert!(
            released.bytes_current < -(1024 * 1024 / 2),
            "retirement must release the fingerprint: {released:?}"
        );
    }

    pub(crate) fn assert_refused_snapshot_post_forces_a_keyframe() {
        let mut run =
            run("export function tick() { const t = Date.now(); while (Date.now() - t < 400) {} }");
        let text = "x".repeat(64 * 1024);
        let mut input = crate::isolate_fb::tests::empty_input(1);
        input.chat_text = Some(&text);
        let keyframe = encode_and_post(&mut run, &input);
        assert!(keyframe.len() > text.len());
        run.on_game_tick_at(1, 0);
        let mut refused = false;
        for _ in 0..200 {
            let (delta, fingerprint, evidence_changed) = encode(&mut run, &input);
            assert!(delta.len() < 1024, "unchanged fields stay out of a delta");
            if !run.post_encoded_snapshot(delta, fingerprint, evidence_changed) {
                refused = true;
                break;
            }
        }
        assert!(refused, "the busy isolate's backlog is bounded");
        assert_eq!(run.posted_snapshot_facts(), None);
        assert!(
            encode(&mut run, &input).0.len() > text.len(),
            "the post after a refusal carries every field again"
        );
        retire(run);
    }
}
