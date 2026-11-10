use super::*;
use crate::combat::{CastNpc, CastNpcRequest};
use crate::native::request::NativeKind;
use crate::native::{ActionError, ActionHandle, NativeActions};
use api::selected::{ClientRevision, RunKey};
use std::task::Poll;

struct CastNpcRun {
    id: RequestId,
    run: RunKey,
    selected: Arc<api::game_data::SelectedGameData>,
    revision: ClientRevision,
    request: CastNpcRequest,
    handle: Option<ActionHandle<CastNpc>>,
    needs_resume: bool,
}

#[derive(Default)]
pub(super) struct CastNpcSeat {
    // Keep this family's large live state out of every otherwise idle API seat.
    active: Option<Box<CastNpcRun>>,
    values: Vec<(RequestId, Vec<u8>)>,
}

impl CastNpcSeat {
    pub(super) fn token(&self) -> Option<u64> {
        self.active.as_ref().map(|run| run.id.get())
    }

    pub(super) fn selected(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.active.as_ref().map(|run| Arc::clone(&run.selected))
    }

    fn has_request(&self, id: RequestId) -> bool {
        self.active.as_ref().is_some_and(|run| run.id == id)
            || self.values.iter().any(|(value, _)| *value == id)
    }

    fn start(&mut self, run: CastNpcRun) {
        self.active = Some(Box::new(run));
    }

    pub(super) fn cancel(&mut self, token: u64) {
        if self.token() == Some(token) {
            self.active = None;
        }
    }

    pub(super) fn reset(&mut self) {
        self.active = None;
        self.values.clear();
    }

    pub(super) fn rekey(&mut self, session: u64) {
        let Some(run) = self.active.as_mut() else {
            return;
        };
        if run.run.session != session {
            run.run.session = session;
            run.needs_resume = run.handle.is_some();
        }
    }

    pub(super) fn retain_results(&mut self, results: &[crate::native::seat::SeatResult]) {
        self.values.retain(|(id, _)| {
            results.iter().any(|result| {
                result.id == *id
                    && matches!(result.outcome, crate::native::seat::SeatTerminal::Done)
            })
        });
    }

    pub(super) fn result_values<'a>(
        &'a self,
        results: &[crate::native::seat::SeatResult],
        output: &mut [crate::native::seat_rows::SeatResultValue<'a>],
    ) -> usize {
        let mut written = 0;
        for (id, value) in &self.values {
            if !results.iter().any(|result| {
                result.id == *id
                    && matches!(result.outcome, crate::native::seat::SeatTerminal::Done)
            }) {
                continue;
            }
            let Some(out) = output.get_mut(written) else {
                break;
            };
            *out = crate::native::seat_rows::SeatResultValue { id: *id, value };
            written += 1;
        }
        written
    }
}

pub(super) fn complete_api_cast_npc(
    authority: &mut GameplaySeat<SeatEffect>,
    seat: &mut ApiSeat,
    pending_logs: &mut Vec<String>,
    id: RequestId,
    value: bool,
) {
    match crate::channel::encode(&serde_json::Value::Bool(value)) {
        Ok(bytes) => match authority.complete(id, false) {
            Ok(()) => seat.cast_npc.values.push((id, bytes)),
            Err(error) => pending_logs.push(format!("castOnNpc terminal refused: {error:?}")),
        },
        Err(error) => {
            pending_logs.push(format!("castOnNpc terminal encoding failed: {error}"));
            let _ = authority.complete(id, true);
        }
    }
}
impl SlotScript {
    pub(in crate::slot) fn start_api_cast_npc(&mut self, id: RequestId, request: CastNpcRequest) {
        if !self.surface_active() {
            return;
        }
        let Some(selected) = self
            .surface_start
            .as_ref()
            .and_then(|identity| identity.game_data.clone())
        else {
            self.finish_api_cast_npc(id, false);
            return;
        };
        let Ok(revision) = compiled::selected_revision(&selected) else {
            self.finish_api_cast_npc(id, false);
            return;
        };
        let Some(generation) = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
        else {
            self.finish_api_cast_npc(id, false);
            return;
        };
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self
                .seat
                .as_ref()
                .map_or(self.work_epoch, |seat| seat.run().session),
        };
        let busy = {
            let seat = self.api.get_or_insert_with(|| Box::new(ApiSeat::default()));
            seat.reap();
            if seat.cast_npc.has_request(id) {
                return;
            }
            seat.busy()
        };
        if busy {
            self.finish_api_cast_npc(id, false);
            return;
        }
        self.control_generation = generation;
        self.api
            .as_mut()
            .expect("API seat was created above")
            .cast_npc
            .start(CastNpcRun {
                id,
                run,
                selected,
                revision,
                request,
                handle: None,
                needs_resume: false,
            });
    }

    fn finish_api_cast_npc(&mut self, id: RequestId, value: bool) {
        let seat = self.api.get_or_insert_with(|| Box::new(ApiSeat::default()));
        complete_api_cast_npc(
            self.seat
                .as_deref_mut()
                .expect("CastNpc request is admitted"),
            seat,
            &mut self.pending_logs,
            id,
            value,
        );
    }

    pub(super) fn tick_api_cast_npc(&mut self, seat: &mut ApiSeat, ctx: &mut ScriptCtx<'_>) {
        let Some(mut run) = seat.cast_npc.active.take() else {
            return;
        };
        if ctx.compiled.hold {
            seat.cast_npc.active = Some(run);
            return;
        }
        let id = run.id;
        let result = {
            let retained = self
                .retained
                .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default())));
            let mut retained = retained
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut cx = compiled::frame_context(
                ctx,
                run.run,
                run.revision,
                &mut retained,
                &mut self.native_runtime,
            );
            let mut actions = NativeActions { _private: () };
            let mut completed = None;
            let mut should_poll = true;
            if run.needs_resume {
                match run.handle.as_mut() {
                    Some(handle) => match actions.resume_kept(NativeKind::CastNpc, handle, &mut cx)
                    {
                        Ok(()) => run.needs_resume = false,
                        Err(
                            ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted,
                        ) => should_poll = false,
                        Err(_) => {
                            completed = Some(false);
                            should_poll = false;
                        }
                    },
                    None => run.needs_resume = false,
                }
            }
            if should_poll && run.handle.is_none() {
                match actions.begin::<CastNpc>((run.request, Arc::clone(&run.selected)), &mut cx) {
                    Ok(handle) => run.handle = Some(handle),
                    Err(ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted) => {
                        should_poll = false;
                    }
                    Err(_) => {
                        completed = Some(false);
                        should_poll = false;
                    }
                }
            }
            if should_poll {
                if let Some(handle) = run.handle.as_ref() {
                    match actions.poll(handle, &mut cx) {
                        Poll::Pending => {}
                        Poll::Ready(Ok(value)) => completed = Some(value),
                        Poll::Ready(Err(_)) => completed = Some(false),
                    }
                }
            }
            completed
        };
        if let Some(value) = result {
            let Some(authority) = self.seat.as_deref_mut() else {
                seat.cast_npc.active = Some(run);
                return;
            };
            complete_api_cast_npc(authority, seat, &mut self.pending_logs, id, value);
        } else {
            seat.cast_npc.active = Some(run);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::seat::Admission;
    use crate::native::seat_rows::SeatResultValue;
    use api::identity::{ExecutionId, RequestId};
    use api::script_contract::RequestStamp;

    #[test]
    fn api_cast_npc_publishes_native_boolean_as_a_done_value() {
        let run = RunKey {
            slot: 77,
            run: 4,
            session: 0,
        };
        for (raw_id, expected) in [(1, false), (2, true)] {
            let id = RequestId::isolate(raw_id).expect("isolate request id");
            let mut authority = crate::native::seat::GameplaySeat::new(run);
            let stamp = RequestStamp::new(id, None, None, ExecutionId::first(run))
                .expect("seat execution stamp");
            assert_eq!(
                authority
                    .start(stamp, NativeKind::CastNpc, false)
                    .expect("start cast seat"),
                Admission::Admitted
            );
            let mut seat = ApiSeat::default();
            let mut logs = Vec::new();
            complete_api_cast_npc(&mut authority, &mut seat, &mut logs, id, expected);

            assert!(logs.is_empty());
            assert_eq!(authority.results().len(), 1);
            assert_eq!(
                authority.results()[0].outcome,
                crate::native::seat::SeatTerminal::Done,
                "false is an observed API value, not a failed transport"
            );
            let mut values = [SeatResultValue { id, value: &[] }];
            assert_eq!(seat.result_values(authority.results(), &mut values), 1);
            assert_eq!(
                crate::channel::decode(values[0].value).expect("native bool value"),
                serde_json::Value::Bool(expected)
            );
        }
    }
}
