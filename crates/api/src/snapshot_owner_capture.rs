//! Capacity-aware GameSnapshot owner census (plan §3 both shells).
//! Child of `snapshot` so private fields are visible. Feature-gated.

#[cfg(test)]
mod owner_regressions {
    use super::*;

    #[test]
    fn independent_shells_keep_distinct_epochs_and_spare_capacity() {
        let host = GameSnapshot::new();
        let mut nav = GameSnapshot::new();
        assert_eq!(
            host.owner_epoch("host_snapshot").tick,
            nav.owner_epoch("nav_snapshot").tick
        );
        nav.tick = 17;
        nav.chat_lines.reserve(19);
        let host_rows = host.owner_payload(&mut Budget::new());
        let nav_rows = nav.owner_payload(&mut Budget::new());
        assert!(host_rows.complete && nav_rows.complete);
        assert_ne!(
            host.owner_epoch("host_snapshot").tick,
            nav.owner_epoch("nav_snapshot").tick
        );
        let bytes = |f: &OwnerFragment| {
            f.rows
                .iter()
                .find(|r| r.field == "chat_lines")
                .unwrap()
                .capacity_bytes
                .unwrap()
        };
        assert!(bytes(&nav_rows) > bytes(&host_rows));
        assert_eq!(nav.chat_lines.len(), 0);
    }

    #[test]
    fn nested_action_storage_charges_spare_option_headers_and_strings() {
        let mut actions = Vec::with_capacity(11);
        actions.push(Some(String::with_capacity(103)));
        actions.push(None);
        let expected = (actions.capacity() * std::mem::size_of::<Option<String>>()) as u64 + 103;
        assert_eq!(
            account_actions_nested(&actions, &mut Budget::new()).unwrap(),
            expected
        );
    }
}

use super::*;
use crate::owner_capture::{
    opt_string_capacity_bytes, owned_string_capacity_bytes, strings_capacity_bytes_budgeted,
    v_bytes, Budget, FieldRow, OwnerFragment, Reason,
};

impl GameSnapshot {
    pub fn owner_epoch(&self, source: &'static str) -> crate::owner_capture::OwnerEpoch {
        crate::owner_capture::OwnerEpoch {
            source,
            tick: self.tick,
            gens: crate::owner_capture::generation_values(&self.gens),
            family_gates: Some([
                self.loc_gen,
                self.loc_model_stamp,
                self.ground_item_gen,
                self.inventory_gate.iface,
                self.inventory_gate.inv,
                self.equipment_gate.iface,
                self.equipment_gate.inv,
                self.bank_gate.iface,
                self.bank_gate.inv,
                self.bank_side_gate.iface,
                self.bank_side_gate.inv,
                self.trade_gate.iface,
                self.trade_gate.inv,
                self.shop_gate.iface,
                self.shop_gate.inv,
                self.widgets_gate.iface,
                self.widgets_gate.inv,
                self.side_tabs_gate.iface,
                self.side_tabs_gate.inv,
                self.chat_options_gate,
                self.make_products_gate,
                self.quest_statuses_gate,
                self.modals_gate,
                self.controls_gate,
                self.menu_gate,
            ]),
            base: self.base,
            tile: self.tile,
            ingame: self.ingame,
            scene_state: self.scene_state,
            draw: None,
            loop_cycle: None,
        }
    }
    /// Borrowed budgeted census of heap-bearing snapshot fields.
    /// Returns a fixed family array of scalar rows; never clones the snapshot.
    pub fn owner_payload(&self, budget: &mut Budget) -> OwnerFragment {
        let begin = budget.elapsed_ns();
        let mut frag = OwnerFragment::new("gamesnapshot");
        frag.source_tick = self.tick;
        frag.begin_ns = begin;

        let header = std::mem::size_of::<GameSnapshot>() as u64;
        let _ = budget.push_row(
            &mut frag.rows,
            FieldRow::ok(
                "gamesnapshot",
                "struct_header",
                1,
                1,
                1,
                header,
                header,
                0,
                0,
                0,
                budget.elapsed_ns().saturating_sub(begin),
            ),
        );

        account_npc_vec(self, budget, &mut frag);
        account_players_shell(self, budget, &mut frag);
        account_stats(self, budget, &mut frag);
        account_inv(self, budget, &mut frag);
        account_chat_opt(self, budget, &mut frag);
        account_varps(self, budget, &mut frag);
        account_scene_collision(self, budget, &mut frag);
        account_locs(self, budget, &mut frag);
        account_ground_items(self, budget, &mut frag);
        account_item_vec("inventory", &self.inventory, budget, &mut frag);
        account_item_vec("equipment", &self.equipment, budget, &mut frag);
        account_item_vec("bank", &self.bank, budget, &mut frag);
        account_item_vec("bank_side", &self.bank_side, budget, &mut frag);
        account_item_vec("trade.my_offer", &self.trade.my_offer, budget, &mut frag);
        account_item_vec(
            "trade.their_offer",
            &self.trade.their_offer,
            budget,
            &mut frag,
        );
        account_item_vec("trade.side_pack", &self.trade.side_pack, budget, &mut frag);
        {
            let nested = opt_string_capacity_bytes(&self.trade.partner);
            let _ = budget.push_row(
                &mut frag.rows,
                FieldRow::ok(
                    "gamesnapshot",
                    "trade.partner",
                    if self.trade.partner.is_some() { 1 } else { 0 },
                    1,
                    if self.trade.partner.is_some() { 1 } else { 0 },
                    nested,
                    nested,
                    0,
                    0,
                    0,
                    budget.elapsed_ns().saturating_sub(begin),
                ),
            );
        }
        account_item_vec("shop.stock", &self.shop.stock, budget, &mut frag);
        account_widgets(self, budget, &mut frag);
        account_side_tabs(self, budget, &mut frag);
        account_chat_lines(self, budget, &mut frag);
        account_chat_options(self, budget, &mut frag);
        account_make_products(self, budget, &mut frag);
        account_quest_statuses(self, budget, &mut frag);
        account_string_vec("menu_entries", &self.menu_entries, budget, &mut frag);
        account_string_vec(
            "main_modal_texts",
            &self.main_modal_texts,
            budget,
            &mut frag,
        );
        account_string_vec(
            "chat_modal_texts",
            &self.chat_modal_texts,
            budget,
            &mut frag,
        );
        {
            let cap = owned_string_capacity_bytes(&self.login_message);
            let _ = budget.push_row(
                &mut frag.rows,
                FieldRow::ok(
                    "gamesnapshot",
                    "login_message",
                    self.login_message.len() as u64,
                    self.login_message.capacity() as u64,
                    1,
                    self.login_message.len() as u64,
                    cap,
                    0,
                    0,
                    0,
                    budget.elapsed_ns().saturating_sub(begin),
                ),
            );
        }

        if let Some(r) = budget.failed() {
            frag.complete = false;
            frag.reason = r;
        }
        frag.visits = budget.visits();
        frag.end_ns = budget.elapsed_ns();
        frag
    }
}

fn push_vec_row(
    frag: &mut OwnerFragment,
    budget: &mut Budget,
    field: &'static str,
    len: u64,
    cap: u64,
    occupied: u64,
    occ_bytes: u64,
    cap_bytes: u64,
    nested: u64,
    boxes: u64,
    box_bytes: u64,
) {
    let elapsed = budget.elapsed_ns();
    let _ = budget.push_row(
        &mut frag.rows,
        FieldRow::ok(
            "gamesnapshot",
            field,
            len,
            cap,
            occupied,
            occ_bytes,
            cap_bytes,
            nested,
            boxes,
            box_bytes,
            elapsed,
        ),
    );
}

fn account_actions_nested(
    actions: &Vec<Option<String>>,
    budget: &mut Budget,
) -> Result<u64, Reason> {
    let mut bytes = v_bytes(actions)?;
    for action in actions {
        if !budget.visit() {
            return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
        }
        bytes = Budget::checked_add(bytes, opt_string_capacity_bytes(action))?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_actions_charge_spare_element_storage() {
        let mut actions = Vec::with_capacity(9);
        actions.push(Some(String::with_capacity(17)));
        let item = ItemView {
            def: ItemDefView {
                id: 0,
                name: None,
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            component_id: 0,
            actions,
        };
        let expected =
            v_bytes(&item.actions).unwrap() + item.actions[0].as_ref().unwrap().capacity() as u64;
        let mut snap = GameSnapshot::new();
        snap.inventory.push(item);
        let frag = snap.owner_payload(&mut Budget::new());
        assert!(frag.complete);
        let row = frag.rows.iter().find(|r| r.field == "inventory").unwrap();
        assert_eq!(row.nested_capacity_bytes, Some(expected));
    }
}

fn account_item_nested(item: &ItemView, budget: &mut Budget) -> Result<(u64, u64), Reason> {
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut nested = opt_string_capacity_bytes(&item.def.name);
    let an = account_actions_nested(&item.actions, budget)?;
    nested = Budget::checked_add(nested, an)?;
    Ok((0, nested))
}

fn account_item_vec(
    field: &'static str,
    items: &Vec<ItemView>,
    budget: &mut Budget,
    frag: &mut OwnerFragment,
) {
    if budget.failed().is_some() {
        return;
    }
    let Ok(cap_bytes) = v_bytes(items) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for it in items {
        match account_item_nested(it, budget) {
            Ok((_h, n)) => match Budget::checked_add(nested, n) {
                Ok(v) => nested = v,
                Err(r) => {
                    frag.complete = false;
                    frag.reason = r;
                    return;
                }
            },
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
                let _ = budget.push_row(
                    &mut frag.rows,
                    FieldRow::incomplete("gamesnapshot", field, r, budget.elapsed_ns()),
                );
                return;
            }
        }
    }
    push_vec_row(
        frag,
        budget,
        field,
        items.len() as u64,
        items.capacity() as u64,
        items.len() as u64,
        (items.len() * std::mem::size_of::<ItemView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_actor_nested(a: &ActorView, budget: &mut Budget) -> Result<u64, Reason> {
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut n = opt_string_capacity_bytes(&a.name);
    n = Budget::checked_add(n, opt_string_capacity_bytes(&a.overhead_text))?;
    let an = account_actions_nested(&a.actions, budget)?;
    Budget::checked_add(n, an)
}

fn account_npc_vec(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let npc = &snap.npc;
    let Ok(cap_bytes) = v_bytes(npc) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for n in npc {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = match (|| {
            let mut t = opt_string_capacity_bytes(&n.name);
            t = Budget::checked_add(t, opt_string_capacity_bytes(&n.overhead_text))?;
            let an = account_actions_nested(&n.actions, budget)?;
            Budget::checked_add(t, an)
        })() {
            Ok(v) => match Budget::checked_add(nested, v) {
                Ok(x) => x,
                Err(r) => {
                    frag.complete = false;
                    frag.reason = r;
                    return;
                }
            },
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
                return;
            }
        };
    }
    push_vec_row(
        frag,
        budget,
        "npc",
        npc.len() as u64,
        npc.capacity() as u64,
        npc.len() as u64,
        (npc.len() * std::mem::size_of::<NpcView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_players_shell(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let players = &snap.players;
    let Ok(cap_bytes) = v_bytes(players) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for p in players {
        match account_actor_nested(&p.actor, budget) {
            Ok(n) => nested = nested.saturating_add(n),
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
                return;
            }
        }
    }
    if let Some(lp) = &snap.player {
        match account_actor_nested(&lp.player.actor, budget) {
            Ok(n) => nested = nested.saturating_add(n),
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
                return;
            }
        }
    }
    push_vec_row(
        frag,
        budget,
        "players",
        players.len() as u64,
        players.capacity() as u64,
        players.len() as u64,
        (players.len() * std::mem::size_of::<PlayerView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_stats(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let stats = &snap.stats;
    let Ok(cap_bytes) = v_bytes(stats) else {
        frag.reason = Reason::Overflow;
        frag.complete = false;
        return;
    };
    let mut nested = 0u64;
    for s in stats {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(owned_string_capacity_bytes(&s.name));
    }
    push_vec_row(
        frag,
        budget,
        "stats",
        stats.len() as u64,
        stats.capacity() as u64,
        stats.len() as u64,
        (stats.len() * std::mem::size_of::<StatView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_inv(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let inv = &snap.inv;
    let Ok(cap_bytes) = v_bytes(inv) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    push_vec_row(
        frag,
        budget,
        "inv",
        inv.len() as u64,
        inv.capacity() as u64,
        inv.len() as u64,
        (inv.len() * std::mem::size_of::<(i32, i32)>()) as u64,
        cap_bytes,
        0,
        0,
        0,
    );
}

fn account_chat_opt(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let nested = opt_string_capacity_bytes(&snap.chat);
    push_vec_row(
        frag,
        budget,
        "chat",
        if snap.chat.is_some() { 1 } else { 0 },
        1,
        if snap.chat.is_some() { 1 } else { 0 },
        nested,
        nested,
        0,
        0,
        0,
    );
}

fn account_varps(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let v = &snap.varps;
    let Ok(cap_bytes) = v_bytes(v) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    push_vec_row(
        frag,
        budget,
        "varps",
        v.len() as u64,
        v.capacity() as u64,
        v.len() as u64,
        (v.len() * std::mem::size_of::<VarpView>()) as u64,
        cap_bytes,
        0,
        0,
        0,
    );
}

fn account_scene_collision(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let flags = &snap.scene.collision_flags;
    let Ok(cap_bytes) = v_bytes(flags) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    push_vec_row(
        frag,
        budget,
        "scene.collision_flags",
        flags.len() as u64,
        flags.capacity() as u64,
        flags.len() as u64,
        (flags.len() * std::mem::size_of::<i32>()) as u64,
        cap_bytes,
        0,
        0,
        0,
    );
}

fn account_locs(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    #[cfg(not(feature = "snapshot-dedup"))]
    let locs: &Vec<LocView> = &snap.loc;
    #[cfg(feature = "snapshot-dedup")]
    let locs: &Vec<LocView> = snap.loc.as_ref();
    let Ok(cap_bytes) = v_bytes(locs) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for loc in locs {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(opt_string_capacity_bytes(&loc.name));
        nested = nested.saturating_add(opt_string_capacity_bytes(&loc.description));
        match account_actions_nested(&loc.actions, budget)
            .and_then(|n| Budget::checked_add(nested, n))
        {
            Ok(n) => nested = n,
            Err(reason) => {
                frag.complete = false;
                frag.reason = reason;
                return;
            }
        }
        // Capacity correction vs legacy actions_bytes (len-only): include spare
        // slot headers via actions_capacity_bytes_vec inside account_actions_nested.
    }
    // Also charge spare capacity on the actions vecs (already in nested via header).
    push_vec_row(
        frag,
        budget,
        "loc",
        locs.len() as u64,
        locs.capacity() as u64,
        locs.len() as u64,
        (locs.len() * std::mem::size_of::<LocView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_ground_items(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let items = &snap.ground_item;
    let Ok(cap_bytes) = v_bytes(items) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for g in items {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(opt_string_capacity_bytes(&g.def.name));
        match account_actions_nested(&g.actions, budget)
            .and_then(|n| Budget::checked_add(nested, n))
        {
            Ok(n) => nested = n,
            Err(reason) => {
                frag.complete = false;
                frag.reason = reason;
                return;
            }
        }
    }
    push_vec_row(
        frag,
        budget,
        "ground_item",
        items.len() as u64,
        items.capacity() as u64,
        items.len() as u64,
        (items.len() * std::mem::size_of::<GroundItemView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn widget_nested(w: &WidgetView, budget: &mut Budget) -> Result<u64, Reason> {
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut n = 0u64;
    n = Budget::checked_add(n, opt_string_capacity_bytes(&w.text))?;
    n = Budget::checked_add(n, opt_string_capacity_bytes(&w.alternate_text))?;
    n = Budget::checked_add(n, opt_string_capacity_bytes(&w.button_text))?;
    n = Budget::checked_add(n, opt_string_capacity_bytes(&w.target_verb))?;
    n = Budget::checked_add(n, opt_string_capacity_bytes(&w.target_base))?;
    if let Some(scripts) = &w.scripts {
        n = Budget::checked_add(
            n,
            Budget::checked_mul(
                scripts.capacity() as u64,
                std::mem::size_of::<Option<Vec<i32>>>() as u64,
            )?,
        )?;
        for s in scripts {
            if !budget.visit() {
                return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
            }
            if let Some(v) = s {
                n = Budget::checked_add(n, v_bytes(v)?)?;
            }
        }
    }
    if let Some(v) = &w.script_comparators {
        n = Budget::checked_add(n, v_bytes(v)?)?;
    }
    if let Some(v) = &w.script_operands {
        n = Budget::checked_add(n, v_bytes(v)?)?;
    }
    n = Budget::checked_add(n, v_bytes(&w.varp_bindings)?)?;
    let an = account_actions_nested(&w.actions, budget)?;
    n = Budget::checked_add(n, an)?;
    n = Budget::checked_add(n, v_bytes(&w.items)?)?;
    for it in &w.items {
        let (_h, inn) = account_item_nested(it, budget)?;
        n = Budget::checked_add(n, inn)?;
    }
    Ok(n)
}

fn account_widgets(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    #[cfg(not(feature = "snapshot-dedup"))]
    let widgets: &Vec<WidgetView> = &snap.widgets;
    #[cfg(feature = "snapshot-dedup")]
    let widgets: &Vec<WidgetView> = snap.widgets.as_ref();
    let Ok(cap_bytes) = v_bytes(widgets) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for w in widgets {
        match widget_nested(w, budget) {
            Ok(n) => nested = nested.saturating_add(n),
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
                return;
            }
        }
    }
    push_vec_row(
        frag,
        budget,
        "widgets",
        widgets.len() as u64,
        widgets.capacity() as u64,
        widgets.len() as u64,
        (widgets.len() * std::mem::size_of::<WidgetView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_side_tabs(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    #[cfg(not(feature = "snapshot-dedup"))]
    let tabs: &Vec<SideTabView> = &snap.side_tabs;
    #[cfg(feature = "snapshot-dedup")]
    let tabs: &Vec<SideTabView> = snap.side_tabs.as_ref();
    let Ok(cap_bytes) = v_bytes(tabs) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for tab in tabs {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(v_bytes(&tab.widgets).unwrap_or(0));
        for w in &tab.widgets {
            match widget_nested(w, budget) {
                Ok(n) => nested = nested.saturating_add(n),
                Err(r) => {
                    frag.complete = false;
                    frag.reason = r;
                    return;
                }
            }
        }
    }
    push_vec_row(
        frag,
        budget,
        "side_tabs",
        tabs.len() as u64,
        tabs.capacity() as u64,
        tabs.len() as u64,
        (tabs.len() * std::mem::size_of::<SideTabView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_chat_lines(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let lines = &snap.chat_lines;
    let Ok(cap_bytes) = v_bytes(lines) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for l in lines {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(opt_string_capacity_bytes(&l.username));
        nested = nested.saturating_add(owned_string_capacity_bytes(&l.text));
    }
    push_vec_row(
        frag,
        budget,
        "chat_lines",
        lines.len() as u64,
        lines.capacity() as u64,
        lines.len() as u64,
        (lines.len() * std::mem::size_of::<ChatLineView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_chat_options(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let opts = &snap.chat_options;
    let Ok(cap_bytes) = v_bytes(opts) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for o in opts {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(owned_string_capacity_bytes(&o.text));
    }
    push_vec_row(
        frag,
        budget,
        "chat_options",
        opts.len() as u64,
        opts.capacity() as u64,
        opts.len() as u64,
        (opts.len() * std::mem::size_of::<ChatOptionView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_make_products(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let prods = &snap.make_products;
    let Ok(cap_bytes) = v_bytes(prods) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for p in prods {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(owned_string_capacity_bytes(&p.name));
        nested = nested.saturating_add(v_bytes(&p.buttons).unwrap_or(0));
    }
    push_vec_row(
        frag,
        budget,
        "make_products",
        prods.len() as u64,
        prods.capacity() as u64,
        prods.len() as u64,
        (prods.len() * std::mem::size_of::<MakeProductView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_quest_statuses(snap: &GameSnapshot, budget: &mut Budget, frag: &mut OwnerFragment) {
    let q = &snap.quest_statuses;
    let Ok(cap_bytes) = v_bytes(q) else {
        frag.complete = false;
        frag.reason = Reason::Overflow;
        return;
    };
    let mut nested = 0u64;
    for e in q {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            return;
        }
        nested = nested.saturating_add(owned_string_capacity_bytes(&e.name));
    }
    push_vec_row(
        frag,
        budget,
        "quest_statuses",
        q.len() as u64,
        q.capacity() as u64,
        q.len() as u64,
        (q.len() * std::mem::size_of::<QuestStatusView>()) as u64,
        cap_bytes,
        nested,
        0,
        0,
    );
}

fn account_string_vec(
    field: &'static str,
    v: &Vec<String>,
    budget: &mut Budget,
    frag: &mut OwnerFragment,
) {
    let Ok((len, cap, header, nested)) = strings_capacity_bytes_budgeted(v, budget) else {
        frag.complete = false;
        frag.reason = budget.failed().unwrap_or(Reason::Overflow);
        return;
    };
    push_vec_row(
        frag,
        budget,
        field,
        len,
        cap,
        len,
        header.min(len.saturating_mul(std::mem::size_of::<String>() as u64)),
        header,
        nested,
        0,
        0,
    );
}
