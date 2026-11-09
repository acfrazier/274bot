//! Fishing gear calculations over caller-owned pieces: which gear is held or missing, the restock top-up, and
//! the shop buy plan with its single-vendor cart (`data/fishingMethods.ts:98-135`,
//! `api/acquisition/ToolAcquire.ts:449-561,624-629`).
//!
//! The method's gear and the caller's count functions stay with the caller. [`GearHost`] is how a
//! calculation reads them, so the V8 adapter calls the script's own functions exactly where the frozen loops
//! would reach them (and a getter or a callback that throws ends the calculation unchanged), while the loops,
//! comparisons and arithmetic run here. The tables are `api::fishing_gear`. Every calculation is pure: it waits
//! for nothing and sends nothing.

use crate::gather_locations::{js_max, js_min};
use api::fishing_gear::{self, FishingShop, FishingVendor};
use api::selected::FactError;

/// Which caller count function a read goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    /// The lone `count(name)` callback of `hasFishingGear` and friends.
    Plain,
    /// `w.heldCount(name)`: inventory plus worn.
    Held,
    /// `w.invCount(name)` / `invCount(name)`.
    Inv,
    /// `w.bankCount(name)` / `bankCount(name)`.
    Bank,
}

/// What a count is asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// The gear piece at this index; the host passes the piece's own `name` value to the callback.
    Piece(usize),
    /// The `Coins` item (`COINS`, `ToolAcquire.ts:13`).
    Coins,
}

/// The caller's gear and world as one calculation sees them. Reads happen in the frozen order.
pub trait GearHost {
    /// A callback or getter that threw, or the refusal of a build without the fishing tables.
    type Error;

    /// Number of gear pieces.
    fn pieces(&self) -> usize;
    /// `g.min`.
    fn min(&mut self, piece: usize) -> Result<f64, Self::Error>;
    /// `g.restock`.
    fn restock(&mut self, piece: usize) -> Result<f64, Self::Error>;
    /// `g.name.toLowerCase()`, which throws for a name that is not a string.
    fn lowered(&mut self, piece: usize) -> Result<String, Self::Error>;
    /// One count callback for an item.
    fn count(&mut self, which: Count, item: Item) -> Result<f64, Self::Error>;
    /// The error for a build without the fishing tables.
    fn facts(&mut self, error: FactError) -> Self::Error;
    /// The distances from `opts.near` to Harry's and Gerrant's stands, or `None` when there is no position.
    fn near(
        &mut self,
        harry: &FishingVendor,
        gerrant: &FishingVendor,
    ) -> Result<Option<(f64, f64)>, Self::Error>;
}

/// JavaScript `a >= b`: false when either side is NaN.
fn at_least(a: f64, b: f64) -> bool {
    matches!(
        a.partial_cmp(&b),
        Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Greater)
    )
}

/// Frozen `hasFishingGear` (`fishingMethods.ts:98-103`): every piece meets its minimum. Stops at the first
/// piece that does not, so later pieces' counts are never asked.
pub fn has_gear<H: GearHost>(host: &mut H) -> Result<bool, H::Error> {
    for piece in 0..host.pieces() {
        let held = host.count(Count::Plain, Item::Piece(piece))?;
        if !at_least(held, host.min(piece)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Frozen `missingFishingGear` (`fishingMethods.ts:105-110`): the pieces below their minimum, in method order.
pub fn missing_gear<H: GearHost>(host: &mut H) -> Result<Vec<usize>, H::Error> {
    let mut missing = Vec::new();
    for piece in 0..host.pieces() {
        let held = host.count(Count::Plain, Item::Piece(piece))?;
        if held < host.min(piece)? {
            missing.push(piece);
        }
    }
    Ok(missing)
}

/// Frozen `shopableMissingFishingGear` (`ToolAcquire.ts:624-629`): the missing pieces that have a shop price.
pub fn shopable_missing_gear<H: GearHost>(host: &mut H) -> Result<Vec<usize>, H::Error> {
    let mut missing = Vec::new();
    for piece in 0..host.pieces() {
        let held = host.count(Count::Plain, Item::Piece(piece))?;
        if held < host.min(piece)?
            && fishing_gear::shop_cost(&host.lowered(piece)?)
                .map_err(|error| host.facts(error))?
                .is_some()
        {
            missing.push(piece);
        }
    }
    Ok(missing)
}

/// Frozen `fishingRestockPlan` (`fishingMethods.ts:116-135`): per piece in order, top up from the bank by
/// `min(restock - inventory, bank)`, skipping a nonpositive need and an empty bank. Returns `(piece, quantity)`.
pub fn restock_plan<H: GearHost>(host: &mut H) -> Result<Vec<(usize, f64)>, H::Error> {
    let mut plan = Vec::new();
    for piece in 0..host.pieces() {
        let have = host.count(Count::Inv, Item::Piece(piece))?;
        let need = host.restock(piece)? - have;
        if need <= 0.0 {
            continue;
        }
        let available = host.count(Count::Bank, Item::Piece(piece))?;
        if available <= 0.0 {
            continue;
        }
        plan.push((piece, js_min(need, available)));
    }
    Ok(plan)
}

/// Frozen `withBaitTarget` (`ToolAcquire.ts:458-466`): the target `max(1, floor(baitQty))` and, per piece,
/// whether it is a bait piece that takes the target as its restock.
pub fn bait_target<H: GearHost>(host: &mut H, bait_qty: f64) -> Result<(f64, Vec<bool>), H::Error> {
    let target = js_max(1.0, bait_qty.floor());
    let mut flags = Vec::with_capacity(host.pieces());
    for piece in 0..host.pieces() {
        let restock = host.restock(piece)?;
        flags.push(fishing_gear::is_bait_piece(restock, || {
            host.lowered(piece)
        })?);
    }
    Ok((target, flags))
}

/// One planned shop purchase (`FishingGearBuyPlan`, `ToolAcquire.ts:475`); the rest of the plan object is fixed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuyPlan {
    /// The gear piece bought.
    pub piece: usize,
    /// Total list price.
    pub cost: f64,
    /// Quantity.
    pub qty: f64,
    /// The shop that sells it.
    pub shop: FishingShop,
}

/// Frozen `planFishingGearBuys` (`ToolAcquire.ts:482-527`): a buy for every missing piece that has a shop
/// price and that the coins left after the earlier lines can afford, with a partial bait stack when only some
/// can be paid. `bait_qty` is the already-resolved `opts.baitQty ?? 1000`.
pub fn buy_plans<H: GearHost>(host: &mut H, bait_qty: f64) -> Result<Vec<BuyPlan>, H::Error> {
    let tables = fishing_gear::fishing_tables().map_err(|error| host.facts(error))?;
    let (target, bait_flags) = bait_target(host, bait_qty)?;
    let mut out = Vec::new();
    let mut coins_left =
        host.count(Count::Inv, Item::Coins)? + host.count(Count::Bank, Item::Coins)?;
    for (piece, &flagged) in bait_flags.iter().enumerate() {
        let have = host.count(Count::Held, Item::Piece(piece))?
            + host.count(Count::Bank, Item::Piece(piece))?;
        // The frozen loop reads the copied piece: a bait piece carries the target as its restock, and
        // `isFishingBaitPiece` is asked again on that copy.
        let restock = if flagged {
            target
        } else {
            host.restock(piece)?
        };
        let is_bait = fishing_gear::is_bait_piece(restock, || host.lowered(piece))?;
        let need_at_least = if is_bait { restock } else { host.min(piece)? };
        if at_least(have, need_at_least) {
            continue;
        }
        let lowered = host.lowered(piece)?;
        let Some(unit) = fishing_gear::shop_cost(&lowered).map_err(|error| host.facts(error))?
        else {
            continue;
        };
        let unit = f64::from(unit);
        let need_qty = js_max(
            1.0,
            need_at_least - host.count(Count::Held, Item::Piece(piece))?,
        );
        let mut qty = if is_bait {
            js_max(host.min(piece)?, need_qty)
        } else {
            1.0
        };
        let mut cost = unit * qty;
        if coins_left < cost {
            if qty > 1.0 && coins_left >= unit {
                qty = (coins_left / unit).floor();
                cost = unit * qty;
            } else {
                continue;
            }
        }
        let lowered = host.lowered(piece)?;
        let shop = tables.vendor_for(&lowered, |harry, gerrant| host.near(harry, gerrant))?;
        out.push(BuyPlan {
            piece,
            cost,
            qty,
            shop,
        });
        coins_left -= cost;
    }
    Ok(out)
}

/// Frozen `fishingGearShopCart` filter (`ToolAcquire.ts:550-561`): keep only the plans for the first plan's
/// shop, in method-gear order, so one vendor trip serves the cart.
pub fn shop_cart(plans: Vec<BuyPlan>) -> Vec<BuyPlan> {
    let Some(first) = plans.first().map(|plan| plan.shop) else {
        return plans;
    };
    plans
        .into_iter()
        .filter(|plan| plan.shop == first)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The refusal of a build without the fishing tables; the tests skip before they can meet it.
    #[derive(Debug)]
    struct Fault(#[allow(dead_code)] FactError);

    impl From<FactError> for Fault {
        fn from(error: FactError) -> Self {
            Self(error)
        }
    }

    struct Gear {
        name: &'static str,
        min: f64,
        restock: f64,
    }

    #[derive(Default)]
    struct Host {
        gear: Vec<Gear>,
        held: HashMap<&'static str, f64>,
        inv: HashMap<&'static str, f64>,
        bank: HashMap<&'static str, f64>,
        near: Option<(f64, f64)>,
        calls: Vec<String>,
    }

    impl Host {
        fn method(pieces: &[(&'static str, f64, f64)]) -> Self {
            Self {
                gear: pieces
                    .iter()
                    .map(|&(name, min, restock)| Gear { name, min, restock })
                    .collect(),
                ..Self::default()
            }
        }
    }

    impl GearHost for Host {
        type Error = Fault;

        fn pieces(&self) -> usize {
            self.gear.len()
        }

        fn min(&mut self, piece: usize) -> Result<f64, Fault> {
            Ok(self.gear[piece].min)
        }

        fn restock(&mut self, piece: usize) -> Result<f64, Fault> {
            Ok(self.gear[piece].restock)
        }

        fn lowered(&mut self, piece: usize) -> Result<String, Fault> {
            Ok(self.gear[piece].name.to_lowercase())
        }

        fn count(&mut self, which: Count, item: Item) -> Result<f64, Fault> {
            let name = match item {
                Item::Piece(piece) => self.gear[piece].name,
                Item::Coins => "Coins",
            };
            self.calls.push(format!("{which:?}:{name}"));
            let table = match which {
                Count::Plain | Count::Held => &self.held,
                Count::Inv => &self.inv,
                Count::Bank => &self.bank,
            };
            Ok(table.get(name).copied().unwrap_or(0.0))
        }

        fn facts(&mut self, error: FactError) -> Fault {
            Fault(error)
        }

        fn near(
            &mut self,
            _: &FishingVendor,
            _: &FishingVendor,
        ) -> Result<Option<(f64, f64)>, Fault> {
            Ok(self.near)
        }
    }

    const FLY: [(&str, f64, f64); 2] = [("Fly fishing rod", 1.0, 1.0), ("Feather", 1.0, 100.0)];

    #[test]
    fn has_gear_stops_at_the_first_missing_piece() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&FLY);
        assert!(!has_gear(&mut host).unwrap());
        assert_eq!(host.calls, ["Plain:Fly fishing rod"]);
        host.held.insert("Fly fishing rod", 1.0);
        host.held.insert("Feather", 1.0);
        host.calls.clear();
        assert!(has_gear(&mut host).unwrap());
        assert_eq!(host.calls, ["Plain:Fly fishing rod", "Plain:Feather"]);
        assert!(
            has_gear(&mut Host::default()).unwrap(),
            "no gear is trivially held"
        );
    }

    #[test]
    fn missing_and_shopable_gear_ask_every_piece_in_order() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host =
            Host::method(&[("Oily fishing rod", 1.0, 1.0), ("Fishing bait", 1.0, 100.0)]);
        assert_eq!(missing_gear(&mut host).unwrap(), [0, 1]);
        assert_eq!(host.calls, ["Plain:Oily fishing rod", "Plain:Fishing bait"]);
        assert_eq!(
            shopable_missing_gear(&mut host).unwrap(),
            [1],
            "the oily rod has no shop price"
        );
        host.held.insert("Fishing bait", 5.0);
        assert_eq!(missing_gear(&mut host).unwrap(), [0]);
    }

    #[test]
    fn restock_tops_up_from_the_bank_by_the_smaller_amount() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&FLY);
        host.inv.insert("Feather", 30.0);
        host.bank.insert("Fly fishing rod", 1.0);
        host.bank.insert("Feather", 50.0);
        assert_eq!(restock_plan(&mut host).unwrap(), [(0, 1.0), (1, 50.0)]);
        host.bank.insert("Feather", 500.0);
        assert_eq!(restock_plan(&mut host).unwrap()[1], (1, 70.0));
        host.inv.insert("Fly fishing rod", 1.0);
        host.calls.clear();
        assert_eq!(restock_plan(&mut host).unwrap(), [(1, 70.0)]);
        assert_eq!(
            host.calls,
            ["Inv:Fly fishing rod", "Inv:Feather", "Bank:Feather"],
            "the bank is asked only for a positive need"
        );
        host.bank.clear();
        assert!(restock_plan(&mut host).unwrap().is_empty());
    }

    #[test]
    fn bait_target_floors_clamps_and_flags_only_bait_pieces() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&FLY);
        assert_eq!(
            bait_target(&mut host, 250.7).unwrap(),
            (250.0, vec![false, true])
        );
        assert_eq!(bait_target(&mut host, -3.0).unwrap().0, 1.0);
        assert!(bait_target(&mut host, f64::NAN).unwrap().0.is_nan());
        let mut single = Host::method(&[("Feather", 1.0, 1.0)]);
        assert_eq!(
            bait_target(&mut single, 9.0).unwrap().1,
            [false],
            "a restock of 1 is not a stack"
        );
    }

    #[test]
    fn buy_plans_follow_the_frozen_cart_rules() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&FLY);
        host.inv.insert("Coins", 100.0);
        let plans = buy_plans(&mut host, 1000.0).unwrap();
        assert_eq!(
            plans,
            [
                BuyPlan { piece: 0, cost: 5.0, qty: 1.0, shop: FishingShop::Gerrant },
                BuyPlan { piece: 1, cost: 94.0, qty: 47.0, shop: FishingShop::Gerrant },
            ],
            "the feather stack is the full shortfall of 1000, cut to what the 95 coins left buy at 2 each"
        );
        let mut rich = Host::method(&FLY);
        rich.bank.insert("Coins", 5000.0);
        let plans = buy_plans(&mut rich, 40.0).unwrap();
        assert_eq!((plans[1].qty, plans[1].cost), (40.0, 80.0));
        let mut broke = Host::method(&FLY);
        broke.inv.insert("Coins", 4.0);
        assert_eq!(
            buy_plans(&mut broke, 1000.0).unwrap(),
            [BuyPlan {
                piece: 1,
                cost: 4.0,
                qty: 2.0,
                shop: FishingShop::Gerrant
            }],
            "the rod costs 5 and is skipped; the stack is cut to the 4 coins"
        );
    }

    #[test]
    fn a_piece_already_held_or_banked_is_not_bought() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&FLY);
        host.inv.insert("Coins", 100.0);
        host.held.insert("Fly fishing rod", 1.0);
        host.bank.insert("Feather", 5000.0);
        assert!(buy_plans(&mut host, 1000.0).unwrap().is_empty());
        host.bank.insert("Feather", 10.0);
        host.held.insert("Feather", 4.0);
        let plans = buy_plans(&mut host, 20.0).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(
            (plans[0].qty, plans[0].cost),
            (16.0, 32.0),
            "20 wanted, 4 held: 16 more"
        );
    }

    #[test]
    fn a_bait_target_of_one_makes_the_stack_a_plain_piece() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&[("Fishing bait", 1.0, 100.0)]);
        host.inv.insert("Coins", 50.0);
        let plans = buy_plans(&mut host, 0.0).unwrap();
        assert_eq!(
            plans,
            [BuyPlan {
                piece: 0,
                cost: 3.0,
                qty: 1.0,
                shop: FishingShop::Harry
            }],
            "target 1 is no stack, so one is bought, and with no position bait goes to Harry"
        );
    }

    #[test]
    fn vendors_follow_position_and_the_cart_keeps_the_first_shop() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&[
            ("Lobster pot", 1.0, 1.0),
            ("Feather", 1.0, 100.0),
            ("Big fishing net", 1.0, 1.0),
        ]);
        host.inv.insert("Coins", 10_000.0);
        host.near = Some((2.0, 9.0));
        let plans = buy_plans(&mut host, 1000.0).unwrap();
        assert_eq!(
            plans.iter().map(|plan| plan.shop).collect::<Vec<_>>(),
            [FishingShop::Harry, FishingShop::Gerrant, FishingShop::Harry]
        );
        let cart = shop_cart(plans);
        assert_eq!(
            cart.iter().map(|plan| plan.piece).collect::<Vec<_>>(),
            [0, 2]
        );
        assert!(shop_cart(Vec::new()).is_empty());
    }

    #[test]
    fn coins_are_asked_once_up_front_inventory_then_bank() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let mut host = Host::method(&[("Harpoon", 1.0, 1.0)]);
        host.inv.insert("Coins", 5.0);
        buy_plans(&mut host, 1000.0).unwrap();
        assert_eq!(
            host.calls,
            [
                "Inv:Coins",
                "Bank:Coins",
                "Held:Harpoon",
                "Bank:Harpoon",
                "Held:Harpoon"
            ]
        );
    }
}
