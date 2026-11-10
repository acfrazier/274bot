//! Slot-owned Shop.buyById runner. The JS family carries only typed input and
//! receives the native machine's observed purchase count.
use super::*;
use crate::native::request::NativeKind;
use crate::native::{ActionError, ActionHandle, NativeActions};
use crate::native_shop::{BuyMachine, BuyReceipt, BuyRequest, ShopBuyByIdArgs};
use api::script_contract::AuthorityStatus;
use api::selected::{ClientRevision, RunKey};
use std::task::Poll;

struct ShopRun {
    id: RequestId,
    run: RunKey,
    revision: ClientRevision,
    args: ShopBuyByIdArgs,
    handle: Option<ActionHandle<BuyMachine>>,
    needs_resume: bool,
}

#[derive(Default)]
pub(super) struct ShopSeat {
    active: Option<Box<ShopRun>>,
    values: Vec<(RequestId, Vec<u8>)>,
}

impl ShopSeat {
    fn retain_live(&mut self, seat: &GameplaySeat<SeatEffect>) {
        if self
            .active
            .as_ref()
            .is_some_and(|run| seat.lookup(run.id) != AuthorityStatus::Live)
        {
            self.active = None;
        }
        self.values.retain(|(id, _)| {
            seat.results().iter().any(|result| {
                result.id == *id
                    && matches!(result.outcome, crate::native::seat::SeatTerminal::Done)
            })
        });
    }

    fn clear_work(&mut self) {
        self.active = None;
        self.values.clear();
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

impl SlotScript {
    pub(super) fn shop_runtime_boundary(&mut self) {
        if let Some(shop) = self.shop.as_mut() {
            shop.clear_work();
        }
    }

    pub(super) fn shop_session_boundary(&mut self, reconnect: bool) {
        if !reconnect {
            self.shop_runtime_boundary();
        }
    }

    pub(super) fn start_shop(&mut self, id: RequestId, args: ShopBuyByIdArgs) {
        let Some(item_id) = args.exact_item_id() else {
            self.finish_shop(id, 0);
            return;
        };
        if !self.surface_active() {
            self.finish_shop(id, 0);
            return;
        }
        let Some(selected) = self
            .surface_start
            .as_ref()
            .and_then(|identity| identity.game_data.clone())
        else {
            self.finish_shop(id, 0);
            return;
        };
        let Ok(revision) = compiled::selected_revision(&selected) else {
            self.finish_shop(id, 0);
            return;
        };
        let Some(generation) = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
        else {
            self.finish_shop(id, 0);
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
        let active_id = self
            .shop
            .as_ref()
            .and_then(|shop| shop.active.as_ref())
            .map(|active| active.id);
        let active_live = active_id.is_some_and(|active_id| {
            self.seat
                .as_ref()
                .is_some_and(|seat| seat.lookup(active_id) == AuthorityStatus::Live)
        });
        if active_id.is_some() && active_live {
            self.finish_shop(id, 0);
            return;
        }
        if let Some(shop) = self.shop.as_mut() {
            shop.active = None;
        }
        self.control_generation = generation;
        self.shop
            .get_or_insert_with(|| Box::new(ShopSeat::default()))
            .active = Some(Box::new(ShopRun {
            id,
            run,
            revision,
            args: ShopBuyByIdArgs {
                item_id: f64::from(item_id),
                quantity: args.quantity,
            },
            handle: None,
            needs_resume: false,
        }));
    }

    fn finish_shop(&mut self, id: RequestId, bought: i32) {
        let mut shop = self
            .shop
            .take()
            .unwrap_or_else(|| Box::new(ShopSeat::default()));
        self.complete_shop(&mut shop, id, bought);
        self.shop = Some(shop);
    }

    fn complete_shop(&mut self, shop: &mut ShopSeat, id: RequestId, bought: i32) {
        let value = serde_json::Value::from(bought);
        let bytes = match crate::channel::encode(&value) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.pending_logs
                    .push(format!("Shop.buyById result encoding failed: {error}"));
                if let Some(seat) = self.seat.as_deref_mut() {
                    if let Err(error) = seat.complete(id, true) {
                        self.pending_logs
                            .push(format!("Shop.buyById failure terminal failed: {error:?}"));
                    }
                }
                return;
            }
        };
        let completion = self
            .seat
            .as_deref_mut()
            .map(|seat| seat.complete(id, false));
        match completion {
            Some(Ok(())) => shop.values.push((id, bytes)),
            Some(Err(error)) => self
                .pending_logs
                .push(format!("Shop.buyById terminal refused: {error:?}")),
            None => {}
        }
    }

    pub(super) fn tick_shop(&mut self, ctx: &mut ScriptCtx<'_>) {
        let Some(mut shop) = self.shop.take() else {
            return;
        };
        if let Some(seat) = self.seat.as_ref() {
            shop.retain_live(seat);
        } else {
            shop.active = None;
            shop.values.clear();
        }
        let Some(mut run) = shop.active.take() else {
            self.shop = Some(shop);
            return;
        };
        let Some(item_id) = run.args.exact_item_id() else {
            self.complete_shop(&mut shop, run.id, 0);
            self.shop = Some(shop);
            return;
        };
        if !self.dispatch_open() || ctx.compiled.hold {
            shop.active = Some(run);
            self.shop = Some(shop);
            return;
        }
        let Some(seat_run) = self.seat.as_ref().map(|seat| seat.run()) else {
            self.shop = Some(shop);
            return;
        };
        if run.run.session != seat_run.session {
            run.run.session = seat_run.session;
            run.needs_resume = run.handle.is_some();
        }
        let result = {
            let retained = self
                .retained
                .get_or_insert_with(|| Arc::new(std::sync::Mutex::new(RetainedMemory::default())));
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
            let mut should_poll = true;
            let mut completed: Option<Result<BuyReceipt, ActionError>> = None;
            if run.needs_resume {
                match run.handle.as_mut() {
                    Some(handle) => match actions.resume_kept(NativeKind::Shop, handle, &mut cx) {
                        Ok(()) => run.needs_resume = false,
                        Err(
                            ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted,
                        ) => {
                            should_poll = false;
                        }
                        Err(error) => {
                            completed = Some(Err(error));
                            should_poll = false;
                        }
                    },
                    None => run.needs_resume = false,
                }
            }
            if should_poll && run.handle.is_none() {
                let request = BuyRequest {
                    item_id,
                    quantity: run.args.quantity,
                };
                match actions.begin::<BuyMachine>(request, &mut cx) {
                    Ok(handle) => run.handle = Some(handle),
                    Err(ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted) => {
                        should_poll = false;
                    }
                    Err(error) => {
                        completed = Some(Err(error));
                        should_poll = false;
                    }
                }
            }
            if should_poll {
                if let Some(handle) = run.handle.as_ref() {
                    if let Poll::Ready(result) = actions.poll(handle, &mut cx) {
                        completed = Some(result);
                    }
                }
            }
            completed
        };
        if let Some(result) = result {
            self.complete_shop(
                &mut shop,
                run.id,
                result.map_or(0, |receipt| receipt.bought),
            );
        } else {
            shop.active = Some(run);
        }
        self.shop = Some(shop);
    }
}
