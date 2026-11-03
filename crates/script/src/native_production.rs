//! Snapshot-driven production adapter shared with compat Make-X sequencing.
//! The caller triggers production first; this machine selects and settles its output.
use crate::interact::InteractReq;
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::production::{
    MakeFixedAction, MakeFixedSelection, MakeXCore, MakeXPhase, MakeXProduct, MakeXSelection,
    MakeXStep,
};
use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, MakeProductView};
use std::sync::Arc;
use std::task::Poll;

// These menus are opened directly by `skill_multi3` in
// `content/scripts/skill_fletching/scripts/cut_logs.rs2:38-47` and
// `content/scripts/skill_crafting/scripts/pottery/pottery.rs2:62-66`.
// Neither waits before publishing its menu.
const MENU_OBSERVATION_TICKS: u64 = 1;
// The engine reads the click on a later world tick, then sends the interface
// packet after that tick's update:
// `engine/src/engine/World.ts:350-355` and `NetworkPlayer.ts:171-177`.
const MENU_PACKET_ORDER_TICKS: u64 = 1;
// `skill_multi3` calls `p_countdialog` after a Make-X button selection
// (`content/scripts/interface_chat/scripts/chat.rs2:741-743`). The engine
// opens the input dialog and suspends the script (`engine/src/engine/script/handlers/PlayerOps.ts:369-371`).
// Allow two observed game ticks for the button and count-dialog signals.
const MAKE_X_SIGNAL_TICKS: u64 = 2;
// Make-X spinning adds one product every three game ticks, the first one
// included. The content queues the first spin with delay 3 and each later spin
// with delay 2 (`content/scripts/skill_crafting/scripts/spinning/spinning.rs2:87-110`).
// The engine runs a weak-queue entry when its old delay is 0 or less
// (`engine/src/engine/entity/Player.ts:916-925`), and a queue pass does not
// visit an entry added during that pass (`engine/src/datastruct/LinkList.ts:97-110`),
// so the delay-2 entry also takes three ticks. The wait is that interval, plus
// the one-tick packet-order margin, plus one tick because the deadline check
// uses `>=`: 3 + 1 + 1 = 5.
const MAKE_X_PRODUCT_PROGRESS_TICKS: u64 = 5;
// Smithing posts the product after its delay, then waits three ticks before it
// returns or starts its next item. Fletching and pottery use two-tick queues.
// Sources: `content/scripts/skill_smithing/scripts/smithing/smithing.rs2:269-286`,
// `content/scripts/skill_fletching/scripts/cut_logs.rs2:83-103`, and
// `content/scripts/skill_crafting/scripts/pottery/pottery.rs2:80-101`.
const PRODUCT_PROGRESS_TICKS: u64 = 3;
// Main-make waits three ticks before returning or starting its next item:
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
    // Column5's posted set labels still dispatch one/five/ten processes:
    // content/scripts/skill_smithing/interfaces/smithing.if:1245-1254 and
    // content/scripts/skill_smithing/scripts/smithing/smithing.rs2:25-30.
    let mut words = action.get(4..)?.split_whitespace();
    let quantity = match (words.next(), words.next(), words.next()) {
        (None, None, None) => 1,
        (Some(unit), None, None) if unit.eq_ignore_ascii_case("set") => 1,
        (Some(value), None, None) => value.parse().ok()?,
        (Some(value), Some(unit), None) if unit.eq_ignore_ascii_case("sets") => {
            value.parse().ok()?
        }
        _ => return None,
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
    /// Product items yielded by one menu process, as authored in content.
    pub items_per_process: i32,
    /// Whether a production trigger was accepted before this menu opened.
    pub after_trigger: bool,
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
    MakeX,
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
        if request.qty < 1 || request.items_per_process < 1 {
            return Err(ActionError::Unavailable(Arc::from(
                "make quantity and process yield must be positive",
            )));
        }
        let tick = cx.evidence().tick;
        let packet_order_ticks = if request.after_trigger {
            MENU_PACKET_ORDER_TICKS
        } else {
            0
        };
        let last_held = held(cx, request.product_id).unwrap_or(0);
        let make_x = MakeXCore::new();
        let phase = Phase::FixedMenu;
        Ok(Self {
            request,
            phase,
            make_x,
            deadline_tick: tick
                .saturating_add(MENU_OBSERVATION_TICKS)
                .saturating_add(packet_order_ticks),
            last_held,
            active_target: 0,
            main_make_ready_tick: tick,
            surface: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let tick = cx.evidence().tick;
        let now_held = held(cx, self.request.product_id);
        let current = now_held.unwrap_or(0);
        let remaining_processes = process_count_for_items(
            self.request.qty.saturating_sub(current),
            self.request.items_per_process,
        );
        'phases: loop {
            if now_held.is_some_and(|count| count >= self.request.qty) {
                let make_x_pending =
                    self.phase == Phase::MakeX && self.make_x.phase() != MakeXPhase::Select;
                let main_make_pending = self.phase == Phase::Product
                    && self.surface == Some(Surface::MainMake)
                    && (tick < self.main_make_ready_tick
                        || now_held.is_some_and(|count| count > self.last_held));
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
                            Some(remaining_processes),
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
                                self.active_target = item_target(
                                    current,
                                    remaining_processes,
                                    self.request.items_per_process,
                                );
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
                            self.surface = Some(Surface::MakeX);
                            self.active_target = item_target(
                                current,
                                remaining_processes,
                                self.request.items_per_process,
                            );
                            self.last_held = current;
                            if now_held.is_some_and(|count| count >= self.request.qty) {
                                return Poll::Ready(Ok(MakeReceipt { held: current }));
                            }
                            self.deadline_tick = tick.saturating_add(MAKE_X_PRODUCT_PROGRESS_TICKS);
                            continue 'phases;
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
                    let remaining_processes = process_count_for_items(
                        self.request.qty.saturating_sub(current),
                        self.request.items_per_process,
                    );
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
                            remaining_processes,
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
                                self.active_target =
                                    item_target(current, quantity, self.request.items_per_process);
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
                                    Some(remaining_processes),
                                ) {
                                    MakeXSelection::Click { component_id } => {
                                        std::task::ready!(crate::native::defer_budget(
                                            cx.emit(InteractReq::IfButton { component_id })
                                        ))?;
                                        self.phase = Phase::MakeX;
                                        self.make_x = next_core;
                                        self.surface = Some(Surface::Chat);
                                        self.active_target = item_target(
                                            current,
                                            remaining_processes,
                                            self.request.items_per_process,
                                        );
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
                            remaining_processes,
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
                                    item_target(current, quantity, self.request.items_per_process);
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
                                    "main-make row has no fixed operation at or below remaining process target",
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
                        self.deadline_tick =
                            tick.saturating_add(if self.surface == Some(Surface::MakeX) {
                                MAKE_X_PRODUCT_PROGRESS_TICKS
                            } else {
                                PRODUCT_PROGRESS_TICKS
                            });
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
                        return Poll::Ready(Ok(MakeReceipt { held: current }));
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

fn process_count_for_items(remaining_items: i32, items_per_process: i32) -> i32 {
    let remaining_items = remaining_items.max(0);
    if remaining_items == 0 {
        0
    } else {
        1 + (remaining_items - 1) / items_per_process
    }
}

fn item_target(current: i32, processes: i32, items_per_process: i32) -> i32 {
    current.saturating_add(processes.saturating_mul(items_per_process))
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
                component_id as usize,
                IfType {
                    id: component_id,
                    layer_id: 2100,
                    r#type: ComponentType::TYPE_TEXT,
                    button_text: (*button_text).into(),
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                component_id as usize,
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
            def: api::ItemDefView {
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
            items_per_process: 1,
            after_trigger: false,
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
    fn make_x_waits_for_the_first_queued_spinning_product() {
        let mut client = make_menu_client("Make X");
        let menu = menu_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(&menu, &mut ledger, request(true));

        let clicked = with_tick(&menu, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            acknowledge_current(&mut tick.cx)
        });
        assert!(matches!(
            clicked,
            HostEffect::Interaction(InteractReq::IfButton { component_id: 2120 })
        ));

        client.dialog_input_open = true;
        client.bump_gens(ServerProt::IF_OPENCHAT);
        let mut count_open = menu_snapshot_with_held(&client, 0);
        count_open.rebuild_family(&client, api::snapshot::Family::Modals);
        let answered = with_tick(&count_open, &mut ledger, 3, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            acknowledge_current(&mut tick.cx)
        });
        assert!(matches!(
            answered,
            HostEffect::Interaction(InteractReq::AnswerCount { value: 20 })
        ));

        client.dialog_input_open = false;
        client.bump_gens(ServerProt::IF_OPENCHAT);
        let mut count_closed = menu_snapshot_with_held(&client, 0);
        count_closed.rebuild_family(&client, api::snapshot::Family::Modals);
        assert!(with_tick(&count_closed, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());

        client.chat_modal_id = -1;
        client.bump_gens(ServerProt::IF_CLOSE);
        let mut menu_closed = menu_snapshot_with_held(&client, 0);
        menu_closed.rebuild_family(&client, api::snapshot::Family::Modals);
        assert!(with_tick(&menu_closed, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());

        // No product is seen for four ticks. The first one arrives on the fifth
        // tick after the count answer, the latest that the five-tick wait accepts.
        assert!(with_tick(&menu_closed, &mut ledger, 8, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(with_tick(&menu_closed, &mut ledger, 9, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let first_product = menu_snapshot_with_held(&client, 1);
        assert!(with_tick(&first_product, &mut ledger, 10, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        // Later products arrive four ticks apart: the three-tick spin interval
        // plus one packet-order tick.
        assert!(with_tick(&first_product, &mut ledger, 12, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(with_tick(&first_product, &mut ledger, 13, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let second_product = menu_snapshot_with_held(&client, 2);
        assert!(with_tick(&second_product, &mut ledger, 14, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(with_tick(&second_product, &mut ledger, 16, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(with_tick(&second_product, &mut ledger, 17, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let third_product = menu_snapshot_with_held(&client, 3);
        assert!(with_tick(&third_product, &mut ledger, 18, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
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
    fn main_make_accepts_smithing_set_operations() {
        // smithing.if:1245-1254 posts these labels for column5.
        // smithing.rs2:25-30 maps them to one, five and ten processes.
        for (quantity, operation) in [(1, 1), (5, 2), (10, 3)] {
            let client = main_make_client(
                1737,
                "Production row",
                &["Make set", "Make 5 sets", "Make 10 sets"],
            );
            let snapshot = main_make_snapshot(&client);
            let mut ledger = None;
            let handle = start_machine(
                &snapshot,
                &mut ledger,
                MakeRequest {
                    product_id: 1759,
                    menu_id: 1737,
                    qty: quantity,
                    items_per_process: 1,
                    after_trigger: false,
                    make_x: false,
                },
            );
            let effect = with_tick(&snapshot, &mut ledger, 2, |tick| {
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
                acknowledge_current(&mut tick.cx)
            });
            assert!(matches!(
                effect,
                HostEffect::Interaction(InteractReq::MakePanel {
                    id: 1737,
                    slot: 0,
                    component: 2210,
                    operation: selected,
                }) if selected == operation
            ));
        }
    }

    #[test]
    fn fixed_main_make_quantities_reject_unknown_or_unbounded_labels() {
        for (label, quantity) in [
            ("Make", 1),
            ("Make 1", 1),
            ("Make set", 1),
            ("MAKE 5 SETS", 5),
            ("Make 10 sets", 10),
        ] {
            assert_eq!(main_make_quantity(label), Some(quantity), "{label}");
        }
        for label in [
            "Make X",
            "Make all",
            "Make 0 sets",
            "Make -1 sets",
            "Make set sets",
            "Make 5 items",
            "Make 5 sets now",
            "Buy 5",
        ] {
            assert_eq!(main_make_quantity(label), None, "{label}");
        }
    }

    #[test]
    fn main_make_returns_partial_target_when_content_closes_menu() {
        let menu_id = 1737;
        let mut client = main_make_client(menu_id, "Bronze bar", &["Make 1", "Make 5", "Make 10"]);
        let mut snapshot = main_make_snapshot(&client);
        let mut ledger = None;
        let handle = start_machine(
            &snapshot,
            &mut ledger,
            MakeRequest {
                product_id: 1759,
                menu_id,
                qty: 7,
                items_per_process: 1,
                after_trigger: false,
                make_x: false,
            },
        );

        let first = with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            acknowledge_current(&mut tick.cx)
        });
        assert!(matches!(
            first,
            HostEffect::Interaction(InteractReq::MakePanel { operation: 2, .. })
        ));

        client.main_modal_id = -1;
        client.bump_gens(ServerProt::IF_CLOSE);
        snapshot.rebuild_family(&client, api::snapshot::Family::MainMake);
        snapshot.seed_inventory(vec![held_product(5)], 28);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 6, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(MakeReceipt { held: 5 }))
        ));
    }

    #[test]
    fn fixed_main_make_counts_processes_for_item_targets() {
        let menu_id = 1737;
        let client = main_make_client(menu_id, "Logs", &["Make 1", "Make 5", "Make 10"]);
        let snapshot = main_make_snapshot(&client);
        let mut ledger = None;
        let mut request = request_with_qty(false, 15);
        request.product_id = 52;
        request.menu_id = menu_id;
        // C:scripts/skill_fletching/scripts/cut_logs.rs2:44-47 yields 15 shafts per log.
        request.items_per_process = 15;
        let handle = start_machine(&snapshot, &mut ledger, request);
        let effect = with_tick(&snapshot, &mut ledger, 2, |tick| {
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            acknowledge_current(&mut tick.cx)
        });
        assert!(matches!(
            effect,
            HostEffect::Interaction(InteractReq::MakePanel { operation: 1, .. })
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
