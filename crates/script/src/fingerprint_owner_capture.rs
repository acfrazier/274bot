//! Fingerprint capacity walker for owner capture (feature `memory-owner-capture`).
//! Lives adjacent to isolate_fb fingerprint types; feature-gated census only.

use crate::isolate_fb::{ItemRowFp, SceneEntityFp, SnapshotFingerprint};
use api::owner_capture::{
    opt_string_capacity_bytes, owned_string_capacity_bytes, strings_capacity_bytes_budgeted,
    v_bytes, Budget, FieldRow, OwnerFragment, Reason,
};

fn push_ok(
    frag: &mut OwnerFragment,
    budget: &mut Budget,
    field: &'static str,
    len: u64,
    cap: u64,
    occ: u64,
    occ_bytes: u64,
    cap_bytes: u64,
    nested: u64,
) {
    let _ = budget.push_row(
        &mut frag.rows,
        FieldRow::ok(
            "fingerprint",
            field,
            len,
            cap,
            occ,
            occ_bytes,
            cap_bytes,
            nested,
            0,
            0,
            budget.elapsed_ns(),
        ),
    );
}

fn item_row_nested(r: &ItemRowFp, budget: &mut Budget) -> Result<u64, Reason> {
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut n = opt_string_capacity_bytes(&r.name);
    n = Budget::checked_add(n, strings_nested(&r.ops, budget)?)?;
    Ok(n)
}

fn strings_nested(v: &Vec<String>, budget: &mut Budget) -> Result<u64, Reason> {
    let mut bytes = v_bytes(v)?;
    for value in v {
        if !budget.visit() {
            return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
        }
        bytes = Budget::checked_add(bytes, value.capacity() as u64)?;
    }
    Ok(bytes)
}

fn items_e(v: &Vec<ItemRowFp>, budget: &mut Budget) -> Result<(u64, u64, u64, u64), Reason> {
    let outer = v_bytes(v)?;
    let mut nested = 0u64;
    for r in v {
        nested = Budget::checked_add(nested, item_row_nested(r, budget)?)?;
    }
    Ok((v.len() as u64, v.capacity() as u64, outer, nested))
}

fn scene_nested(e: &SceneEntityFp, budget: &mut Budget) -> Result<u64, Reason> {
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut n = opt_string_capacity_bytes(&e.name);
    n = Budget::checked_add(n, strings_nested(&e.actions, budget)?)?;
    Ok(n)
}

fn scenes_e(v: &Vec<SceneEntityFp>, budget: &mut Budget) -> Result<(u64, u64, u64, u64), Reason> {
    let outer = v_bytes(v)?;
    let mut nested = 0u64;
    for e in v {
        nested = Budget::checked_add(nested, scene_nested(e, budget)?)?;
    }
    Ok((v.len() as u64, v.capacity() as u64, outer, nested))
}

fn account_items(
    frag: &mut OwnerFragment,
    budget: &mut Budget,
    field: &'static str,
    v: &Vec<ItemRowFp>,
) {
    match items_e(v, budget) {
        Ok((len, cap, outer, nested)) => {
            push_ok(frag, budget, field, len, cap, len, outer, outer, nested);
        }
        Err(r) => {
            frag.complete = false;
            frag.reason = r;
        }
    }
}

fn account_scenes(
    frag: &mut OwnerFragment,
    budget: &mut Budget,
    field: &'static str,
    v: &Vec<SceneEntityFp>,
) {
    match scenes_e(v, budget) {
        Ok((len, cap, outer, nested)) => {
            push_ok(frag, budget, field, len, cap, len, outer, outer, nested);
        }
        Err(r) => {
            frag.complete = false;
            frag.reason = r;
        }
    }
}

fn account_plain_vec<T>(
    frag: &mut OwnerFragment,
    budget: &mut Budget,
    field: &'static str,
    v: &Vec<T>,
) {
    match v_bytes(v) {
        Ok(outer) => {
            push_ok(
                frag,
                budget,
                field,
                v.len() as u64,
                v.capacity() as u64,
                v.len() as u64,
                outer,
                outer,
                0,
            );
        }
        Err(r) => {
            frag.complete = false;
            frag.reason = r;
        }
    }
}

fn account_nested_vec<T>(
    frag: &mut OwnerFragment,
    budget: &mut Budget,
    field: &'static str,
    values: &Vec<T>,
    nested_bytes: impl Fn(&T) -> Result<u64, Reason>,
) {
    let result = (|| {
        let mut nested = 0;
        for value in values {
            if !budget.visit() {
                return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
            }
            nested = Budget::checked_add(nested, nested_bytes(value)?)?;
        }
        Ok((
            api::owner_capture::occ_bytes(values)?,
            v_bytes(values)?,
            nested,
        ))
    })();
    match result {
        Ok((occupied, capacity, nested)) => push_ok(
            frag,
            budget,
            field,
            values.len() as u64,
            values.capacity() as u64,
            values.len() as u64,
            occupied,
            capacity,
            nested,
        ),
        Err(reason) => {
            frag.complete = false;
            frag.reason = reason;
        }
    }
}

/// Capacity-aware fingerprint census (plan §3 script fingerprint).
pub fn fingerprint_owner_payload(fp: &SnapshotFingerprint, budget: &mut Budget) -> OwnerFragment {
    let mut frag = OwnerFragment::new("fingerprint");
    let begin = budget.elapsed_ns();

    let header = std::mem::size_of::<SnapshotFingerprint>() as u64;
    push_ok(
        &mut frag,
        budget,
        "struct_header",
        1,
        1,
        1,
        header,
        header,
        0,
    );

    account_items(&mut frag, budget, "inv", &fp.inv);
    account_items(&mut frag, budget, "bank", &fp.bank);
    account_items(&mut frag, budget, "bank_side", &fp.bank_side);
    account_items(&mut frag, budget, "equipment", &fp.equipment);
    account_items(&mut frag, budget, "trade_mine", &fp.trade_mine);
    account_items(&mut frag, budget, "trade_theirs", &fp.trade_theirs);
    account_items(&mut frag, budget, "trade_side", &fp.trade_side);
    account_items(&mut frag, budget, "shop_stock", &fp.shop_stock);

    account_scenes(&mut frag, budget, "npcs", &fp.npcs);
    account_scenes(&mut frag, budget, "locs", &fp.locs);
    account_scenes(&mut frag, budget, "players", &fp.players);
    account_scenes(&mut frag, budget, "ground", &fp.ground);

    account_plain_vec(&mut frag, budget, "booths", &fp.booths);
    account_plain_vec(&mut frag, budget, "varps", &fp.varps);
    account_plain_vec(&mut frag, budget, "side_tab_ifaces", &fp.side_tab_ifaces);
    account_nested_vec(&mut frag, budget, "banks", &fp.banks, |b| {
        Budget::checked_add(
            Budget::checked_add(b.name.capacity() as u64, b.kind.capacity() as u64)?,
            opt_string_capacity_bytes(&b.choose),
        )
    });
    account_nested_vec(&mut frag, budget, "stats", &fp.stats, |s| {
        Ok(s.1.capacity() as u64)
    });
    account_nested_vec(&mut frag, budget, "chat_lines", &fp.chat_lines, |s| {
        Ok(s.1.capacity() as u64)
    });
    account_nested_vec(&mut frag, budget, "spell_buttons", &fp.spell_buttons, |s| {
        Ok(s.label.capacity() as u64)
    });

    // combat_styles: Vec + nested String labels
    {
        match v_bytes(&fp.combat_styles) {
            Ok(outer) => {
                let mut nested = 0u64;
                for cs in &fp.combat_styles {
                    if !budget.visit() {
                        frag.complete = false;
                        frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
                        break;
                    }
                    nested = nested.saturating_add(owned_string_capacity_bytes(&cs.label));
                }
                push_ok(
                    &mut frag,
                    budget,
                    "combat_styles",
                    fp.combat_styles.len() as u64,
                    fp.combat_styles.capacity() as u64,
                    fp.combat_styles.len() as u64,
                    outer,
                    outer,
                    nested,
                );
            }
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
            }
        }
    }

    // make_products
    {
        match v_bytes(&fp.make_products) {
            Ok(outer) => {
                let btn_sz = std::mem::size_of::<crate::isolate_fb::MakeButtonFp>() as u64;
                let mut nested = 0u64;
                for p in &fp.make_products {
                    if !budget.visit() {
                        frag.complete = false;
                        frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
                        break;
                    }
                    nested = nested.saturating_add(owned_string_capacity_bytes(&p.name));
                    nested =
                        nested.saturating_add((p.buttons.capacity() as u64).saturating_mul(btn_sz));
                }
                push_ok(
                    &mut frag,
                    budget,
                    "make_products",
                    fp.make_products.len() as u64,
                    fp.make_products.capacity() as u64,
                    fp.make_products.len() as u64,
                    outer,
                    outer,
                    nested,
                );
            }
            Err(r) => {
                frag.complete = false;
                frag.reason = r;
            }
        }
    }

    // nearest_booth optional
    {
        let nested = match &fp.nearest_booth {
            Some(b) => owned_string_capacity_bytes(&b.name)
                .saturating_add(owned_string_capacity_bytes(&b.op)),
            None => 0,
        };
        push_ok(
            &mut frag,
            budget,
            "nearest_booth",
            if fp.nearest_booth.is_some() { 1 } else { 0 },
            1,
            if fp.nearest_booth.is_some() { 1 } else { 0 },
            nested,
            nested,
            0,
        );
    }

    // chat_options
    match strings_capacity_bytes_budgeted(&fp.chat_options, budget) {
        Ok((_len, _cap, header, nested)) => {
            push_ok(
                &mut frag,
                budget,
                "chat_options",
                fp.chat_options.len() as u64,
                fp.chat_options.capacity() as u64,
                fp.chat_options.len() as u64,
                header,
                header,
                nested,
            );
        }
        Err(r) => {
            frag.complete = false;
            frag.reason = r;
        }
    }

    for (field, s) in [
        ("chat_text", &fp.chat_text),
        ("my_name", &fp.my_name),
        ("trade_partner", &fp.trade_partner),
    ] {
        let nested = opt_string_capacity_bytes(s);
        push_ok(
            &mut frag,
            budget,
            field,
            if s.is_some() { 1 } else { 0 },
            1,
            if s.is_some() { 1 } else { 0 },
            nested,
            nested,
            0,
        );
    }

    frag.begin_ns = begin;
    frag.end_ns = budget.elapsed_ns();
    frag.visits = budget.visits();
    frag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_slot_probe_preserves_next_flatbuffer_and_stop_absence() {
        let mut control = crate::slot::SlotScript::new();
        let mut observed = crate::slot::SlotScript::new();
        let input = crate::isolate_fb::tests::empty_input;
        assert_eq!(
            control.encode_snapshot_delta(&input(1), false),
            observed.encode_snapshot_delta(&input(1), false)
        );
        let frag = observed.owner_payload(&mut Budget::new());
        assert!(frag.complete);
        assert_eq!(frag.fingerprint_present, Some(true));
        assert_eq!(
            control.encode_snapshot_delta(&input(2), false),
            observed.encode_snapshot_delta(&input(2), false)
        );
        observed.stop();
        let stopped = observed.owner_payload(&mut Budget::new());
        assert!(stopped.complete);
        assert_eq!(stopped.fingerprint_present, Some(false));
        assert_eq!(stopped.script_state, Some("Idle"));
    }
    use api::owner_capture::Budget;

    #[test]
    fn empty_fingerprint_emits_header() {
        let fp = SnapshotFingerprint::default();
        let mut budget = Budget::new();
        let frag = fingerprint_owner_payload(&fp, &mut budget);
        assert!(frag.complete, "empty fp should complete");
        assert!(
            frag.rows.iter().any(|r| r.field == "struct_header"),
            "header row required"
        );
    }

    #[test]
    fn every_owned_fingerprint_family_is_measured() {
        let fp = SnapshotFingerprint::default();
        let frag = fingerprint_owner_payload(&fp, &mut Budget::new());
        for field in [
            "trade_mine",
            "trade_theirs",
            "trade_side",
            "shop_stock",
            "spell_buttons",
            "chat_lines",
            "banks",
            "stats",
        ] {
            assert!(
                frag.rows.iter().any(|r| r.field == field),
                "missing {field}"
            );
        }
    }

    #[test]
    fn chat_options_charge_the_callers_visit_budget() {
        let mut fp = SnapshotFingerprint::default();
        fp.chat_options = vec![String::new(); 257];
        let mut budget = Budget::new();
        let frag = fingerprint_owner_payload(&fp, &mut budget);
        assert!(frag.complete);
        assert!(
            budget.visits() >= 257,
            "chat option traversal escaped the fragment budget"
        );
    }

    #[test]
    fn item_ops_capacity_counted() {
        let mut fp = SnapshotFingerprint::default();
        fp.inv.push(ItemRowFp {
            name: Some("Lobster".into()),
            count: 1,
            id: 1,
            ops: {
                let mut o = Vec::with_capacity(8);
                o.push("Eat".into());
                o
            },
            noted: false,
            cert: 0,
            component_id: 0,
        });
        let mut budget = Budget::new();
        let frag = fingerprint_owner_payload(&fp, &mut budget);
        let inv = frag.rows.iter().find(|r| r.field == "inv").expect("inv");
        assert!(
            inv.nested_capacity_bytes.unwrap_or(0)
                == fp.inv[0].name.as_ref().unwrap().capacity() as u64
                    + (fp.inv[0].ops.capacity() * std::mem::size_of::<String>()) as u64
                    + fp.inv[0].ops[0].capacity() as u64,
            "ops capacity nested"
        );
        assert!(frag.complete);
    }
}
