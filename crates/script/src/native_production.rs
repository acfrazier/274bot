//! Snapshot-driven production adapter shared with compat Make-X sequencing.
//! The caller triggers production first; this machine selects and settles its output.
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::production::{
    MakeFixedAction, MakeFixedSelection, MakeXCore, MakeXPhase, MakeXProduct, MakeXSelection,
    MakeXStep,
};
use crate::shim::InteractReq;
use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, MakeProductView};
use std::sync::Arc;
use std::task::Poll;

// These menus are opened directly by `skill_multi2` in
// `content/scripts/skill_fletching/scripts/cut_logs.rs2:38-47` and
// `content/scripts/skill_crafting/scripts/pottery/pottery.rs2:62-66`.
// Neither waits before publishing its menu.
const MENU_OBSERVATION_TICKS: u64 = 1;
// P_COUNTDIALOG writes and suspends in one engine operation; its response
// resumes the script synchronously:
// `engine/src/engine/script/handlers/PlayerOps.ts:369-371` and
// `engine/src/network/game/client/handler/ResumePCountDialogHandler.ts:7-16`.
const MAKE_X_SIGNAL_TICKS: u64 = 1;
// The slowest output/repeat bound is three ticks in
// `content/scripts/skill_crafting/scripts/spinning/spinning.rs2:83-99` and
// `content/scripts/skill_smithing/scripts/smithing/smithing.rs2:269-286`.
// Fletching and pottery use two-tick output queues:
// `content/scripts/skill_fletching/scripts/cut_logs.rs2:83-103` and
// `content/scripts/skill_crafting/scripts/pottery/pottery.rs2:80-101`.
const PRODUCT_PROGRESS_TICKS: u64 = 3;
// Smithing posts the product after its delay, then delays three ticks before
// it returns or starts its next item:
// `content/scripts/skill_smithing/scripts/smithing/smithing.rs2:269-286`.
const MAIN_MAKE_RETURN_TICKS: u64 = 3;

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

impl MakeXProduct for ItemView {
    fn object_id(&self) -> i32 {
        self.def.id
    }

    fn make_x_component_id(&self) -> Option<i32> {
        None
    }

    fn fixed_quantity_action(&self, quantity: i32) -> Option<MakeFixedAction> {
        main_make_action(self, quantity)
    }

    fn fixed_quantity_at_most_action(&self, maximum: i32) -> Option<MakeFixedAction> {
        if self.container != ItemContainer::MainMake
            || self.action_family != ItemActionFamily::Component
        {
            return None;
        }
        let mut selected: Option<(i32, i32)> = None;
        for (index, action) in self.actions.iter().enumerate() {
            let Some(quantity) = action.as_deref().and_then(main_make_quantity) else {
                continue;
            };
            if quantity > maximum || selected.is_some_and(|(best, _)| quantity <= best) {
                continue;
            }
            let Ok(operation) = i32::try_from(index + 1) else {
                continue;
            };
            selected = Some((quantity, operation));
        }
        let (quantity, operation) = selected?;
        Some(MakeFixedAction::MainMake {
            item_id: self.def.id,
            slot: self.slot,
            component_id: self.component_id,
            operation,
            quantity,
        })
    }
}

fn main_make_action(item: &ItemView, quantity: i32) -> Option<MakeFixedAction> {
    if item.container != ItemContainer::MainMake
        || item.action_family != ItemActionFamily::Component
    {
        return None;
    }
    item.actions.iter().enumerate().find_map(|(index, action)| {
        if action.as_deref().and_then(main_make_quantity) != Some(quantity) {
            return None;
        }
        let operation = i32::try_from(index + 1).ok()?;
        Some(MakeFixedAction::MainMake {
            item_id: item.def.id,
            slot: item.slot,
            component_id: item.component_id,
            operation,
            quantity,
        })
    })
}

fn main_make_quantity(action: &str) -> Option<i32> {
    let prefix = action.get(..4)?;
    if !prefix.eq_ignore_ascii_case("make") {
        return None;
    }
    let quantity = match action.get(4..)?.trim() {
        "" => 1,
        value => value.parse().ok()?,
    };
    (quantity > 0).then_some(quantity)
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Chat,
    MainMake,
}

pub struct MakeMachine {
    request: MakeRequest,
    phase: Phase,
    make_x: MakeXCore,
    deadline_tick: u64,
    last_held: i32,
    active_target: i32,
    main_make_ready_tick: u64,
    surface: Option<Surface>,
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
        let tick = cx.evidence().tick;
        let last_held = held(cx, request.product_id).unwrap_or(0);
        let make_x = MakeXCore::new();
        let phase = Phase::FixedMenu;
        Ok(Self {
            request,
            phase,
            make_x,
            deadline_tick: tick.saturating_add(MENU_OBSERVATION_TICKS),
            last_held,
            active_target: 0,
            main_make_ready_tick: tick,
            surface: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        loop {
            let tick = cx.evidence().tick;
            let now_held = held(cx, self.request.product_id);
            let current = now_held.unwrap_or(0);
            if now_held.is_some_and(|count| count >= self.request.qty) {
                let make_x_pending =
                    self.phase == Phase::MakeX && self.make_x.phase() != MakeXPhase::Select;
                let main_make_pending = self.phase == Phase::Product
                    && self.surface == Some(Surface::MainMake)
                    && tick < self.main_make_ready_tick;
                if !make_x_pending && !main_make_pending {
                    return Poll::Ready(Ok(MakeReceipt { held: current }));
                }
            }

            match self.phase {
                Phase::MakeX => {
                    let snapshot = cx.snapshot();
                    let products = snapshot.make_products();
                    let selection = if self.make_x.phase() == MakeXPhase::Select {
                        let mut next_core = self.make_x;
                        let selection = next_core.select(
                            products.as_ref().map(|rows| rows.value),
                            self.request.menu_id,
                            Some(self.request.qty - current),
                        );
                        Some((selection, next_core))
                    } else {
                        None
                    };
                    let count_dialog_open =
                        snapshot.count_dialog_open().is_some_and(|open| open.value);
                    let make_menu_open =
                        products.as_ref().is_some_and(|rows| !rows.value.is_empty());
                    if let Some((selection, next_core)) = selection {
                        match selection {
                            MakeXSelection::MenuMissing => {}
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
                                self.surface = Some(Surface::Chat);
                                self.active_target = self.request.qty;
                                self.last_held = current;
                                self.deadline_tick = tick.saturating_add(MAKE_X_SIGNAL_TICKS);
                                return Poll::Pending;
                            }
                        }
                    }
                    let mut next_core = self.make_x;
                    match next_core.step(
                        count_dialog_open,
                        make_menu_open,
                        tick >= self.deadline_tick,
                    ) {
                        MakeXStep::Wait => return Poll::Pending,
                        MakeXStep::AnswerCount { value } => {
                            std::task::ready!(crate::native::defer_budget(
                                cx.emit(InteractReq::AnswerCount { value })
                            ))?;
                            self.make_x = next_core;
                            self.deadline_tick = tick.saturating_add(MAKE_X_SIGNAL_TICKS);
                            return Poll::Pending;
                        }
                        MakeXStep::WaitMenuClose => {
                            self.make_x = next_core;
                            self.deadline_tick = tick.saturating_add(MAKE_X_SIGNAL_TICKS);
                            return Poll::Pending;
                        }
                        MakeXStep::Complete => {
                            self.make_x = MakeXCore::new();
                            self.phase = Phase::Product;
                            self.surface = Some(Surface::Chat);
                            self.active_target = self.request.qty;
                            self.last_held = current;
                            self.deadline_tick = tick.saturating_add(PRODUCT_PROGRESS_TICKS);
                            return Poll::Pending;
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
                    let chat = snapshot.make_products();
                    let main_make = snapshot.main_make();
                    let chat_rows = chat.as_ref().map(|rows| rows.value).unwrap_or(&[]);
                    let main_make_rows = main_make.as_ref().map(|rows| rows.value).unwrap_or(&[]);
                    if chat.is_none() && main_make.is_none() {
                        if tick >= self.deadline_tick {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "production menu did not expose the product",
                            ))));
                        }
                        return Poll::Pending;
                    }
                    let remaining = self.request.qty - current;
                    let chat_has_row = chat_rows
                        .iter()
                        .any(|row| row.object_id == self.request.menu_id);
                    let main_make_has_row = main_make_rows
                        .iter()
                        .any(|row| row.def.id == self.request.menu_id);
                    if chat_has_row && main_make_has_row {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "production product is ambiguous across open menus",
                        ))));
                    }
                    if chat_has_row {
                        match self.make_x.select_fixed(
                            Some(chat_rows),
                            self.request.menu_id,
                            remaining,
                        ) {
                            MakeFixedSelection::Select {
                                action:
                                    MakeFixedAction::ChatButton {
                                        component_id,
                                        quantity,
                                    },
                            } => {
                                std::task::ready!(crate::native::defer_budget(
                                    cx.emit(InteractReq::IfButton { component_id })
                                ))?;
                                self.phase = Phase::Product;
                                self.surface = Some(Surface::Chat);
                                self.active_target = current.saturating_add(quantity);
                                self.last_held = current;
                                self.deadline_tick = tick.saturating_add(PRODUCT_PROGRESS_TICKS);
                                return Poll::Pending;
                            }
                            MakeFixedSelection::Select {
                                action: MakeFixedAction::MainMake { .. },
                            } => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "chat make row exposed a main-make operation",
                                ))));
                            }
                            MakeFixedSelection::MenuMissing => {}
                            MakeFixedSelection::InvalidCount => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make quantity must be positive",
                                ))));
                            }
                            MakeFixedSelection::MissingButton => {
                                if !self.request.make_x {
                                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                        "make menu has no fixed-quantity button for remaining target",
                                    ))));
                                }
                                let mut next_core = MakeXCore::new();
                                match next_core.select(
                                    Some(chat_rows),
                                    self.request.menu_id,
                                    Some(remaining),
                                ) {
                                    MakeXSelection::Click { component_id } => {
                                        std::task::ready!(crate::native::defer_budget(
                                            cx.emit(InteractReq::IfButton { component_id })
                                        ))?;
                                        self.phase = Phase::MakeX;
                                        self.make_x = next_core;
                                        self.surface = Some(Surface::Chat);
                                        self.active_target = self.request.qty;
                                        self.last_held = current;
                                        self.deadline_tick =
                                            tick.saturating_add(MAKE_X_SIGNAL_TICKS);
                                        return Poll::Pending;
                                    }
                                    MakeXSelection::MenuMissing
                                    | MakeXSelection::MissingButton
                                    | MakeXSelection::InvalidCount => {
                                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                            "make menu has no exact fixed-quantity or Make-X action",
                                        ))));
                                    }
                                }
                            }
                        }
                    } else if main_make_has_row {
                        match self.make_x.select_fixed_at_most(
                            Some(main_make_rows),
                            self.request.menu_id,
                            remaining,
                        ) {
                            MakeFixedSelection::Select {
                                action:
                                    MakeFixedAction::MainMake {
                                        item_id,
                                        slot,
                                        component_id,
                                        operation,
                                        quantity,
                                    },
                            } => {
                                std::task::ready!(crate::native::defer_budget(cx.emit(
                                    InteractReq::MakePanel {
                                        id: item_id,
                                        slot,
                                        component: component_id,
                                        operation,
                                    }
                                )))?;
                                self.phase = Phase::Product;
                                self.surface = Some(Surface::MainMake);
                                self.active_target =
                                    current.saturating_add(quantity).min(self.request.qty);
                                self.last_held = current;
                                self.deadline_tick = tick.saturating_add(PRODUCT_PROGRESS_TICKS);
                                return Poll::Pending;
                            }
                            MakeFixedSelection::Select {
                                action: MakeFixedAction::ChatButton { .. },
                            } => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "main-make row exposed a chat button",
                                ))));
                            }
                            MakeFixedSelection::MenuMissing => {}
                            MakeFixedSelection::MissingButton => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "main-make row has no fixed operation at or below remaining target",
                                ))));
                            }
                            MakeFixedSelection::InvalidCount => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "make quantity must be positive",
                                ))));
                            }
                        }
                    }
                    if tick >= self.deadline_tick {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "production menu did not expose the product",
                        ))));
                    }
                    return Poll::Pending;
                }
                Phase::Product => {
                    if current > self.last_held {
                        self.last_held = current;
                        self.deadline_tick = tick.saturating_add(PRODUCT_PROGRESS_TICKS);
                        if self.surface == Some(Surface::MainMake) {
                            self.main_make_ready_tick = tick.saturating_add(MAIN_MAKE_RETURN_TICKS);
                        }
                    }
                    if now_held.is_some_and(|count| count >= self.request.qty) {
                        if self.surface == Some(Surface::MainMake)
                            && tick < self.main_make_ready_tick
                        {
                            return Poll::Pending;
                        }
                        return Poll::Ready(Ok(MakeReceipt { held: current }));
                    }
                    if self.surface == Some(Surface::MainMake)
                        && current >= self.active_target
                        && tick >= self.main_make_ready_tick
                    {
                        self.phase = Phase::FixedMenu;
                        self.make_x = MakeXCore::new();
                        self.surface = None;
                        self.deadline_tick = tick.saturating_add(MENU_OBSERVATION_TICKS);
                        continue;
                    }
                    if tick >= self.deadline_tick {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "production did not reach the held target",
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
        snapshot.seed_inventory(vec![held_product(count)], 28);
        snapshot
    }

    fn held_product(count: i32) -> api::snapshot::ItemView {
        api::snapshot::ItemView {
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
        }
    }

    fn main_make_client(item_id: i32, item_name: &str, actions: &[&str]) -> Client {
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp/274bot-no-production-cache".into(),
            members: true,
            lowmem: false,
        });
        let cache = Arc::get_mut(&mut client.cache).unwrap();
        cache
            .objs
            .resize(item_id as usize + 1, client::config::ObjType::default());
        cache.objs[item_id as usize] = client::config::ObjType {
            id: item_id,
            name: item_name.into(),
            ..Default::default()
        };
        client.set_iface(
            2200,
            IfType {
                id: 2200,
                layer_id: 2200,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![2210]),
                ..Default::default()
            },
        );
        let mut operations: [Option<String>; 5] = Default::default();
        for (operation, text) in operations.iter_mut().zip(actions) {
            *operation = Some((*text).into());
        }
        client.set_iface(
            2210,
            IfType {
                id: 2210,
                layer_id: 2200,
                r#type: ComponentType::TYPE_INV,
                iop: operations,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            2210,
            IfTypeMut {
                link_obj_type: Some(vec![item_id + 1]),
                link_obj_number: Some(vec![1]),
                ..Default::default()
            },
        );
        client.main_modal_id = 2200;
        client.bump_gens(ServerProt::IF_OPENMAIN);
        client
    }

    fn main_make_snapshot(client: &Client) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.rebuild_family(client, api::snapshot::Family::MainMake);
        snapshot.seed_inventory(Vec::new(), 28);
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
        let menu = menu_snapshot_with_held(&client, 5);
        client.dialog_input_open = true;
        client.bump_gens(ServerProt::IF_OPENCHAT);
        let mut count_page = menu_snapshot_with_held(&client, 5);
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
            HostEffect::Interaction(InteractReq::AnswerCount { value: 15 })
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
    fn exact_fixed_chat_action_precedes_make_x() {
        let client = make_menu_client_with_buttons(&["Make 10", "Make X"]);
        let snapshot = menu_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(&snapshot, &mut ledger, request_with_qty(true, 10));
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
    fn same_tick_menu_polls_do_not_consume_observed_tick_deadline() {
        let client = make_menu_client("Make 1");
        let snapshot = menu_snapshot(&client);
        let mut ledger = None;
        let mut selected = request(false);
        selected.menu_id = 999;
        let handle = start_machine(&snapshot, &mut ledger, selected);
        for _ in 0..3 {
            assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending());
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "production menu did not expose the product"
        ));
    }

    #[test]
    fn main_make_repeats_bounded_fixed_actions_until_held_target() {
        let menu_id = 1737;
        let client = main_make_client(menu_id, "Bronze bar", &["Make 1", "Make 5", "Make 10"]);
        let mut snapshot = main_make_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(
            &snapshot,
            &mut ledger,
            MakeRequest {
                product_id: 1759,
                menu_id,
                qty: 7,
                make_x: false,
            },
        );

        let first = with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            acknowledge_current(&mut tick.cx)
        });
        assert!(matches!(
            first,
            HostEffect::Interaction(InteractReq::MakePanel {
                id: 1737,
                slot: 0,
                component: 2210,
                operation: 2,
            })
        ));

        snapshot.seed_inventory(vec![held_product(5)], 28);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let second = with_tick(&snapshot, &mut ledger, 6, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            acknowledge_current(&mut tick.cx)
        });
        assert!(matches!(
            second,
            HostEffect::Interaction(InteractReq::MakePanel {
                id: 1737,
                slot: 0,
                component: 2210,
                operation: 1,
            })
        ));

        snapshot.seed_inventory(vec![held_product(7)], 28);
        assert!(with_tick(&snapshot, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 12, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(MakeReceipt { held: 7 }))
        ));
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
