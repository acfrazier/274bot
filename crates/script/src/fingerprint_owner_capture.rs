//! Fingerprint capacity walker for owner capture (feature `memory-owner-capture`).
//! Lives adjacent to isolate_fb fingerprint types; feature-gated census only.

use crate::isolate_fb::{
    ItemRowFp, SceneEntityFp, SnapshotFingerprint,
};
use api::owner_capture::{
    opt_string_capacity_bytes, owned_string_capacity_bytes, strings_capacity_bytes_vec, v_bytes,
    Budget, FieldRow, OwnerFragment, Reason,
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
    let (_l, _c, _h, ops_nested) = strings_capacity_bytes_vec(&r.ops)?;
    n = Budget::checked_add(n, ops_nested)?;
    Ok(n)
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
    let (_l, _c, _h, acts) = strings_capacity_bytes_vec(&e.actions)?;
    n = Budget::checked_add(n, acts)?;
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

/// Capacity-aware fingerprint census (plan §3 script fingerprint).
pub fn fingerprint_owner_payload(fp: &SnapshotFingerprint, budget: &mut Budget) -> OwnerFragment {
    let mut frag = OwnerFragment::new("fingerprint");
    let begin = budget.elapsed_ns();

    let header = std::mem::size_of::<SnapshotFingerprint>() as u64;
    push_ok(
        &mut frag, budget, "struct_header", 1, 1, 1, header, header, 0,
    );

    account_items(&mut frag, budget, "inv", &fp.inv);
    account_items(&mut frag, budget, "bank", &fp.bank);
    account_items(&mut frag, budget, "bank_side", &fp.bank_side);
    account_items(&mut frag, budget, "equipment", &fp.equipment);

    account_scenes(&mut frag, budget, "npcs", &fp.npcs);
    account_scenes(&mut frag, budget, "locs", &fp.locs);
    account_scenes(&mut frag, budget, "players", &fp.players);
    account_scenes(&mut frag, budget, "ground", &fp.ground);

    account_plain_vec(&mut frag, budget, "booths", &fp.booths);
    account_plain_vec(&mut frag, budget, "varps", &fp.varps);
    account_plain_vec(&mut frag, budget, "side_tab_ifaces", &fp.side_tab_ifaces);
    account_plain_vec(&mut frag, budget, "banks", &fp.banks);
    account_plain_vec(&mut frag, budget, "stats", &fp.stats);

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
                    nested = nested
                        .saturating_add((p.buttons.capacity() as u64).saturating_mul(btn_sz));
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
    match strings_capacity_bytes_vec(&fp.chat_options) {
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
            inv.nested_capacity_bytes.unwrap_or(0) > 0,
            "ops capacity nested"
        );
        assert!(frag.complete);
    }
}
