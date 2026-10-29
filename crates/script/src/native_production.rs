//! Snapshot-driven production adapter shared with compat Make-X sequencing.
//! The caller triggers the production loc/use-on first; this machine adapts
//! target-held quantities and observes output settlement.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::production::{MakeXCore, MakeXPhase, MakeXProduct, MakeXSelection, MakeXStep};
use crate::shim::InteractReq;
use api::snapshot::MakeProductView;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const PRODUCT_BOUND: Duration = Duration::from_secs(2 * 60);

impl MakeXProduct for MakeProductView {
    fn object_id(&self) -> i32 {
        self.object_id
    }

    fn make_x_component_id(&self) -> Option<i32> {
        self.buttons
            .iter()
            .find(|button| button.quantity == -1 && button.component_id >= 0)
            .map(|button| button.component_id)
    }

    fn fixed_quantity_component_id(&self, quantity: i32) -> Option<i32> {
        self.buttons
            .iter()
            .find(|button| button.quantity == quantity && button.component_id >= 0)
            .map(|button| button.component_id)
    }
}

#[derive(Debug, Clone)]
pub struct MakeRequest {
    pub product_id: i32,
    /// Object represented by the menu row; it may be the input, not the output.
    pub menu_id: i32,
    /// Desired held count after this run.
    pub qty: i32,
    pub make_x: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakeReceipt {
    pub held: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    MakeX,
    FixedMenu,
    Product,
}

pub struct MakeMachine {
    request: MakeRequest,
    phase: Phase,
    make_x: MakeXCore,
    deadline: Duration,
    before: i32,
}

impl NativeMachine for MakeMachine {
    type Args = MakeRequest;
    type Output = MakeReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if request.qty < 1 {
            return Err(ActionError::Unavailable(Arc::from(
                "make quantity must be positive",
            )));
        }
        let before = held(cx, request.product_id).unwrap_or(0);
        let make_x = MakeXCore::new();
        let phase = if request.make_x {
            Phase::MakeX
        } else {
            Phase::FixedMenu
        };
        let deadline = cx
            .active_now()
            .saturating_add(Duration::from_millis(make_x.timeout_ms()));
        Ok(Self {
            request,
            phase,
            make_x,
            deadline,
            before,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            let now = cx.active_now();
            let now_held = held(cx, self.request.product_id);
            if now_held.is_some_and(|count| count >= self.request.qty)
                && (self.phase != Phase::MakeX || self.make_x.phase() == MakeXPhase::Select)
            {
                return Poll::Ready(Ok(MakeReceipt {
                    held: now_held.unwrap_or_default(),
                }));
            }
            match self.phase {
                Phase::MakeX => {
                    let current = now_held.unwrap_or(0);
                    let answer = (self.request.qty - current).max(1);
                    let (selection, count_dialog_open, make_menu_open) = {
                        let snapshot = cx.snapshot();
                        let products = snapshot.make_products();
                        let selection = if self.make_x.phase() == MakeXPhase::Select {
                            let mut next_core = self.make_x;
                            let selection = next_core.select(
                                products.as_ref().map(|rows| rows.value),
                                self.request.menu_id,
                                Some(answer),
                            );
                            Some((selection, next_core))
                        } else {
                            None
                        };
                        let count_dialog_open =
                            snapshot.count_dialog_open().is_some_and(|open| open.value);
                        let make_menu_open =
                            products.as_ref().is_none_or(|rows| !rows.value.is_empty());
                        (selection, count_dialog_open, make_menu_open)
                    };
                    if let Some((selection, next_core)) = selection {
                        match selection {
                            MakeXSelection::MenuMissing => {
                                if now >= self.deadline {
                                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                        "make menu did not expose the product",
                                    ))));
                                }
                                return Poll::Pending;
                            }
                            MakeXSelection::MissingButton => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make menu has no Make-X button",
                                ))));
                            }
                            MakeXSelection::InvalidCount => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make quantity must be positive",
                                ))));
                            }
                            MakeXSelection::Click { component_id } => {
                                std::task::ready!(crate::native::defer_budget(
                                    cx.emit(InteractReq::IfButton { component_id })
                                ))?;
                                self.make_x = next_core;
                                self.deadline = now.saturating_add(Duration::from_millis(
                                    self.make_x.timeout_ms(),
                                ));
                                return Poll::Pending;
                            }
                        }
                    }
                    let mut next_core = self.make_x;
                    match next_core.step(count_dialog_open, make_menu_open, now >= self.deadline) {
                        MakeXStep::Wait => return Poll::Pending,
                        MakeXStep::AnswerCount { value } => {
                            std::task::ready!(crate::native::defer_budget(
                                cx.emit(InteractReq::AnswerCount { value })
                            ))?;
                            self.make_x = next_core;
                            self.deadline =
                                now.saturating_add(Duration::from_millis(self.make_x.timeout_ms()));
                            return Poll::Pending;
                        }
                        MakeXStep::WaitMenuClose => {
                            self.make_x = next_core;
                            self.deadline =
                                now.saturating_add(Duration::from_millis(self.make_x.timeout_ms()));
                            return Poll::Pending;
                        }
                        MakeXStep::Complete => {
                            self.make_x = next_core;
                            self.phase = Phase::Product;
                            self.deadline = now.saturating_add(PRODUCT_BOUND);
                            continue;
                        }
                        MakeXStep::TimedOut(phase) => {
                            let message = match phase {
                                MakeXPhase::Select => "make menu did not expose the product",
                                MakeXPhase::WaitCountOpen => "Make-X count dialog did not open",
                                MakeXPhase::WaitCountClose => "Make-X count dialog did not close",
                                MakeXPhase::WaitMenuClose => "Make-X menu did not close",
                            };
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(message))));
                        }
                    }
                }
                Phase::FixedMenu => {
                    let snapshot = cx.snapshot();
                    let Some(products) = snapshot.make_products() else {
                        return Poll::Pending;
                    };
                    let remaining = self.request.qty - now_held.unwrap_or(0);
                    let component_id = match self.make_x.select_fixed(
                        Some(products.value),
                        self.request.menu_id,
                        remaining,
                    ) {
                        MakeXSelection::MenuMissing => {
                            if now >= self.deadline {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make menu did not expose the product",
                                ))));
                            }
                            return Poll::Pending;
                        }
                        MakeXSelection::MissingButton => {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "make menu has no fixed-quantity button for remaining target",
                            ))));
                        }
                        MakeXSelection::InvalidCount => {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "make quantity must be positive",
                            ))));
                        }
                        MakeXSelection::Click { component_id } => component_id,
                    };
                    std::task::ready!(crate::native::defer_budget(
                        cx.emit(InteractReq::IfButton { component_id },)
                    ))?;
                    self.phase = Phase::Product;
                    self.deadline = now.saturating_add(PRODUCT_BOUND);
                    return Poll::Pending;
                }
                Phase::Product => {
                    if now_held.is_some_and(|count| count > self.before) {
                        // A fixed button may make fewer than requested. The caller's
                        // authored settle sees the exact held count and re-selects.
                        return Poll::Ready(Ok(MakeReceipt {
                            held: now_held.unwrap_or_default(),
                        }));
                    }
                    if now >= self.deadline {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "production did not yield the product",
                        ))));
                    }
                    return Poll::Pending;
                }
            }
        }
    }

    fn cancel(&mut self) {}
}

fn held(cx: &ActionContext<'_>, id: i32) -> Option<i32> {
    cx.snapshot().stock().held(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ActionContext, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::{with_tick, with_tick_snapshots};
    use api::snapshot::{GameSnapshot, SnapshotView};
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;

    fn make_menu_client(button_text: &str) -> Client {
        make_menu_client_with_buttons(&[button_text])
    }

    fn make_menu_client_with_buttons(button_texts: &[&str]) -> Client {
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp/274bot-no-production-cache".into(),
            members: true,
            lowmem: false,
        });
        let cache = Arc::get_mut(&mut client.cache).unwrap();
        cache.objs.resize(1738, client::config::ObjType::default());
        cache.objs[1737] = client::config::ObjType {
            id: 1737,
            name: "Wool".into(),
            ..Default::default()
        };
        let button_ids = (0..button_texts.len())
            .map(|index| 2120 + index as i32)
            .collect::<Vec<_>>();
        let mut children = Vec::with_capacity(button_ids.len() + 1);
        children.push(2110);
        children.extend(button_ids.iter().copied());
        client.set_iface(
            2100,
            IfType {
                id: 2100,
                layer_id: 2100,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(children),
                ..Default::default()
            },
        );
        client.set_iface(
            2110,
            IfType {
                id: 2110,
                layer_id: 2100,
                r#type: ComponentType::TYPE_MODEL,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            2110,
            IfTypeMut {
                model1_type: 4,
                model1_id: 1737,
                ..Default::default()
            },
        );
        for (component_id, button_text) in button_ids.into_iter().zip(button_texts) {
            client.set_iface(
                component_id,
                IfType {
                    id: component_id,
                    layer_id: 2100,
                    r#type: ComponentType::TYPE_TEXT,
                    button_text: (*button_text).into(),
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                component_id,
                IfTypeMut {
                    button_type: ButtonType::BUTTON_OK,
                    ..Default::default()
                },
            );
        }
        client.chat_modal_id = 2100;
        client.bump_gens(ServerProt::IF_OPENCHAT);
        client
    }

    fn menu_snapshot(client: &Client) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.rebuild_family(client, api::snapshot::Family::MakeProducts);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot
    }

    fn menu_snapshot_with_held(client: &Client, count: i32) -> GameSnapshot {
        let mut snapshot = menu_snapshot(client);
        snapshot.seed_inventory(
            vec![api::snapshot::ItemView {
                def: api::snapshot::ItemDefView {
                    id: 1759,
                    name: Some("Ball of wool".into()),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 0,
                count,
                actions: Vec::new(),
                component_id: 3214,
            }],
            28,
        );
        snapshot
    }

    fn request_with_qty(make_x: bool, qty: i32) -> MakeRequest {
        MakeRequest {
            qty,
            ..request(make_x)
        }
    }

    fn request(make_x: bool) -> MakeRequest {
        MakeRequest {
            product_id: 1759,
            menu_id: 1737,
            qty: 20,
            make_x,
        }
    }

    fn spend_interaction(cx: &mut ActionContext<'_>) {
        cx.action_id = cx.ledger.as_ref().unwrap().owner.as_ref().unwrap().id.get();
        let request_id = cx
            .emit(InteractReq::ContinueDialog { component_id: None })
            .unwrap();
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.pop().expect("budget-filling interaction");
        assert_eq!(action.request_id.get(), request_id);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id,
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn acknowledge_current(cx: &mut ActionContext<'_>) -> HostEffect {
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.remove(0);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id: action.request_id.get(),
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
        action.effect
    }

    fn start_machine(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        request: MakeRequest,
    ) -> crate::native::ActionHandle<MakeMachine> {
        with_tick(snapshot, ledger, 1, |tick| {
            tick.actions
                .begin::<MakeMachine>(request, &mut tick.cx)
                .unwrap()
        })
    }

    #[test]
    fn make_x_select_budget_denial_retries_same_product_next_tick() {
        let client = make_menu_client("Make X");
        let snapshot = menu_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger, request(true));
        let denied = with_tick(&snapshot, &mut ledger, 2, |tick| {
            spend_interaction(&mut tick.cx);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let retry = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::IfButton { component_id: 2120 })
        ));
    }

    #[test]
    fn make_x_same_tick_count_page_budget_denial_answers_next_tick() {
        let mut client = make_menu_client("Make X");
        let menu = menu_snapshot(&client);
        client.dialog_input_open = true;
        client.bump_gens(ServerProt::IF_OPENCHAT);
        let mut count_page = menu_snapshot(&client);
        count_page.rebuild_family(&client, api::snapshot::Family::Modals);

        let mut ledger = None;
        let handle = with_tick_snapshots(&menu, &count_page, &mut ledger, 1, |tick, next| {
            let handle = tick
                .actions
                .begin::<MakeMachine>(request(true), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            assert!(matches!(
                acknowledge_current(&mut tick.cx),
                HostEffect::Interaction(InteractReq::IfButton { component_id: 2120 })
            ));

            let mut evidence = tick.cx.evidence();
            evidence.sequence += 1;
            tick.cx.evidence = evidence;
            tick.cx.snapshot = SnapshotView::new(Some(next), evidence);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            assert!(
                tick.cx.ledger.as_ref().unwrap().outbox.is_empty(),
                "count-page wake cannot answer with this tick's spent event"
            );
            handle
        });

        let retry = with_tick(&count_page, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::AnswerCount { value: 20 })
        ));
    }

    #[test]
    fn fixed_make_button_budget_denial_retries_next_tick() {
        let client = make_menu_client("Make 10");
        let snapshot = menu_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger, request_with_qty(false, 10));
        let denied = with_tick(&snapshot, &mut ledger, 2, |tick| {
            spend_interaction(&mut tick.cx);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(denied.is_pending(), "budget denial must remain Pending");
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        let retry = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(retry.is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::IfButton { component_id: 2120 })
        ));
    }

    #[test]
    fn fixed_make_button_selects_exact_held_target_remainder() {
        let client = make_menu_client_with_buttons(&["Make 5", "Make 10"]);
        let snapshot = menu_snapshot_with_held(&client, 5);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger, request_with_qty(false, 10));
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::IfButton { component_id: 2120 })
        ));
    }

    #[test]
    fn fixed_make_button_requires_an_exact_remaining_quantity() {
        let client = make_menu_client("Make 5");
        let snapshot = menu_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger, request_with_qty(false, 10));
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "make menu has no fixed-quantity button for remaining target"
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
    #[test]
    fn production_adapter_state_sizes_stay_within_explicit_bounds() {
        let request = std::mem::size_of::<MakeRequest>();
        let machine = std::mem::size_of::<MakeMachine>();
        let core = std::mem::size_of::<MakeXCore>();
        assert!(request <= 32, "MakeRequest grew to {request} bytes");
        assert!(machine <= 128, "MakeMachine grew to {machine} bytes");
        assert!(core <= 16, "MakeXCore grew to {core} bytes");
        println!(
            "production adapter state sizes: MakeRequest={request} MakeMachine={machine} MakeXCore={core} bytes"
        );
    }
}
