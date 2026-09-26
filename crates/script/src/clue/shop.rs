use super::*;

/// The selected toll keeper's own spawn: the unique jm2 tile the Shantay arm
/// walks to. Selected constants and never the frozen `GATE_ITEM_SHOPS` stand —
/// `(3304, 3122, 0)` is off by one in z, and no frozen tile is ported here.
pub(super) fn shantay_spawn() -> Tile {
    Tile {
        x: SHANTAY_X,
        z: SHANTAY_Z,
        level: SHANTAY_LEVEL,
    }
}

/// Whether this call's page names the short: a posted `walk_missing_carry` row
/// whose own id is the selected item's and whose count is positive. Only that
/// vector nominates a shop — the walk outcome's fail bit and its dest geometry
/// are never read as a shopping list, and a page that posted no vector names
/// nothing.
///
/// The row's posted display name is corroboration and never a second identity:
/// the join is the id, exactly the way the pack's own trio read joins one.
pub(super) fn carry_names(input: &Value, id: i32) -> bool {
    let Some(rows) = input.get("walk_missing_carry").and_then(Value::as_array) else {
        return false;
    };
    rows.iter().any(|row| {
        posted_i32(row, "id") == Some(id)
            && posted_i32(row, "count").is_some_and(|count| count >= 1)
    })
}

/// This call's posted `shop_open`, and only a posted `true`: an omitted slot is
/// unobserved — neither a closed interface nor an open one — and a posted
/// `false` is a closed one.
pub(super) fn shop_open(input: &Value) -> bool {
    input.get("shop_open").and_then(Value::as_bool) == Some(true)
}

/// One posted stock row the buy click can ride: the short's own id, the posted
/// display name the host resolves, and the posted slot and component its
/// presence check matches. No row id and no tile are invented for one.
pub(super) struct BuyRow<'a> {
    pub(super) name: &'a str,
    pub(super) id: i32,
    pub(super) slot: i32,
    pub(super) component: i32,
}

/// This call's posted stock row for the short, on the open interface.
///
/// The join is the row's own id, and the three fields the host re-resolves it by
/// — name, slot, component — must all be posted: a row the page posted without
/// them is not clickable here and is skipped rather than guessed at, and a page
/// with no such row buys nothing. The count is not a second gate — the click is
/// the frozen `Shop.buy(name, 1)`, one chunk of one.
pub(super) fn buy_row<'a>(input: &'a Value, id: i32) -> Option<BuyRow<'a>> {
    let rows = input.get("shop_stock").and_then(Value::as_array)?;
    rows.iter().find_map(|row| {
        if posted_i32(row, "id") != Some(id) {
            return None;
        }
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())?;
        Some(BuyRow {
            name,
            id,
            slot: posted_i32(row, "slot")?,
            component: posted_i32(row, "component")?,
        })
    })
}

/// The landed shop click the buy rides: the posted stock row's own identity and
/// one chunk of one, as `InteractReq::ShopButton` reads it. Its own envelope kind
/// — the adapters enqueue it as the landed `shop-button` op, never as a
/// `kind: "ops"` shopping list, never as a loc and never as a `V2_OPS` verb.
pub(super) fn buy_verb(row: &BuyRow<'_>, token: u64) -> Value {
    json!({
        "kind": "shop-button",
        "token": token,
        "shop": BUY,
        "name": row.name,
        "id": row.id,
        "slot": row.slot,
        "component": row.component,
        "chunk": 1,
    })
}
impl ClueRuntime {
    /// The gate-toll walk intercept: one live walk this token dispatched that
    /// has not arrived, whose page names a `Carry` short for the selected
    /// Shantay pass that the posted pack does not hold. `None` is the
    /// fall-through — the row's own `Steady` arms run exactly as they did
    /// before this intercept existed.
    ///
    /// It is the walk's intercept and not a second identify: the step is the one
    /// the landed identify already returned, and this reads no membership of its
    /// own. It sits in front of every arm a walk can come from — the search
    /// dispatch, both dig arms, the trio acquire chain, the talk step and the
    /// key hunt — so a failed walk is the same failure whichever arm armed it,
    /// and it is never an arm *after* the talk step.
    ///
    /// The short is the navigator's, never this machine's: a posted `Carry` row
    /// of the selected pass' own id is the whole nomination, and a `NoPath`'s
    /// dest geometry is never read as a shopping list. The Al Kharid toll's
    /// coins, the extra-item Rope and every other named short are not this item
    /// and never shop.
    ///
    /// Once per item id per token: the id is latched before the trip's first
    /// verb goes out, so the Shantay walk can never re-enter this intercept and
    /// a second failure of the same walk never starts a third trip.
    pub(super) fn toll(
        &mut self,
        selected: Option<&SelectedGameData>,
        input: &Value,
    ) -> Option<Value> {
        if let Some(shop) = self.shop {
            return self.shop_trip(shop, input);
        }
        // The selected pass, by alias: no selected item is no shop at all, and
        // the id every posted row is joined to is that item's own.
        let item = selected?.item_by_alias(SHANTAY_PASS)?;
        if self.shopped.contains(&item.id) || holds(input, item.id) {
            // Already shopped for this token, or the pass is on this call's
            // posted pack page: there is nothing to buy.
            return None;
        }
        let dest = self.walk_dest?;
        if arrival(dest, input) != Arrival::Walking || !carry_names(input, item.id) {
            // No live walk — an arrived (or unposted) `here` is not the failure
            // this page is naming — and no posted short for the pass.
            return None;
        }
        self.shopped.push(item.id);
        self.shop = Some(Shop {
            id: item.id,
            dest,
            step: ShopStep::Stand,
            failed: false,
            closed: false,
        });
        Some(self.walk(shantay_spawn()))
    }

    /// One call of the live shop trip, one verb per call: the walk to the
    /// selected Shantay spawn, the posted keeper's `Trade`, the posted stock
    /// row's buy, and then the exit.
    ///
    /// Nothing is invented on any step. The keeper is the posted npc of the
    /// selected type — or, failing that, of the selected display name — standing
    /// on the spawn inside the frozen `ARRIVE_RADIUS` and listing a posted
    /// `Trade`; the buy rides the posted stock row's own name, id, slot and
    /// component; and the settle is the posted pack page holding the short's own
    /// id. A step whose own fact the page never posts waits inside the machine's
    /// own `SHOP_WAIT_MS` window and then gives up — with the token live and the
    /// latch kept, so the named `no-shop` is never a second trip.
    pub(super) fn shop_trip(&mut self, mut shop: Shop, input: &Value) -> Option<Value> {
        let spawn = shantay_spawn();
        match shop.step {
            // The trip is over: nothing is observed again, and the exit owes at
            // most one call per step of its own.
            ShopStep::Exit => self.shop_exit(shop, input),
            ShopStep::Stand => match arrival(spawn, input) {
                // No posted `here`: no arrival claim to make and no walk to
                // measure, so the trip waits where it is.
                Arrival::Unknown => self.shop_wait(shop, input),
                Arrival::Walking => {
                    self.shop = Some(shop);
                    Some(self.walk(spawn))
                }
                Arrival::Arrived => {
                    // The first arrived call opens this step's own observation
                    // window; the walk itself is never bounded by it.
                    if self.clock.deadline.is_none() {
                        self.clock.arm(SHOP_WAIT_MS);
                    }
                    // An open chat is not a tick to click the keeper: the same
                    // rule the talk arm reads, and the count dialog with it.
                    let pick = if dialog_ready(input) || count_open(input) {
                        None
                    } else {
                        input
                            .get("npcs")
                            .and_then(Value::as_array)
                            .and_then(|page| pick_at_spawn(&NpcIdentity::Shantay, page, spawn))
                    };
                    match pick {
                        Some(pick) => {
                            shop.step = ShopStep::Trade;
                            self.shop = Some(shop);
                            // The interface's own window starts with the click.
                            self.clock.arm(SHOP_WAIT_MS);
                            Some(self.npc_verb(&pick))
                        }
                        // Arrived with no posted keeper of this identity on the
                        // spawn: stay there and wait it out rather than chasing
                        // a wanderer, and give up when the window ends.
                        None => self.shop_wait(shop, input),
                    }
                }
            },
            ShopStep::Trade => {
                if !shop_open(input) {
                    // No posted open interface: unobserved is not a closed shop,
                    // so this waits rather than clicking blind.
                    return self.shop_wait(shop, input);
                }
                match buy_row(input, shop.id) {
                    Some(row) => {
                        shop.step = ShopStep::Buy;
                        self.shop = Some(shop);
                        // The settle's own window starts with the click.
                        self.clock.arm(SHOP_WAIT_MS);
                        Some(buy_verb(&row, self.token))
                    }
                    // The interface is up and this short's own stock row is not
                    // on it: nothing here may be clicked, so the trip gives up
                    // instead of pressing a row it did not read.
                    None => {
                        shop.failed = true;
                        shop.step = ShopStep::Exit;
                        self.shop_exit(shop, input)
                    }
                }
            }
            ShopStep::Buy => {
                if holds(input, shop.id) {
                    // The short landed on the posted pack page: the trip is
                    // over and its interface is what is left.
                    return self.shop_exit(shop, input);
                }
                self.shop_wait(shop, input)
            }
        }
    }

    /// This step's own wait: inside the machine's `SHOP_WAIT_MS` window the call
    /// is a `wait` with the token live, and past it the short did not land — the
    /// trip gives up, which is the exit with `failed` set.
    pub(super) fn shop_wait(&mut self, mut shop: Shop, input: &Value) -> Option<Value> {
        if !self.clock.bound_reached() {
            self.shop = Some(shop);
            return Some(self.emit("wait"));
        }
        // The step's own posted fact never came: the trip gives up — that is the
        // exit with `failed` set — and this step never waits again on a window
        // it already spent.
        shop.failed = true;
        shop.step = ShopStep::Exit;
        self.shop_exit(shop, input)
    }

    /// The trip's exit, one step per call and every one of them a posted fact or
    /// the trip's own outcome: the posted interface is closed once when it is
    /// up, a short that did not land exits with the named `no-shop`, and then
    /// the walk back to the original dest goes out.
    ///
    /// `None` is the fall-through the row's own arm takes once that walk is out.
    /// The latch still has the short, so the arm's next failed walk is never a
    /// third trip, and the walk back is the same tile that arm was walking to
    /// before the trip — the second walk, made here so it happens on a page that
    /// posts no `here` at all.
    pub(super) fn shop_exit(&mut self, mut shop: Shop, input: &Value) -> Option<Value> {
        // No step is waiting any more: the exit is driven by this call's posted
        // interface, the trip's own outcome and the dest, never by a clock.
        self.clock.deadline = None;
        if !shop.closed && shop_open(input) {
            shop.closed = true;
            self.shop = Some(shop);
            return Some(self.emit("close-modal"));
        }
        if shop.failed {
            // The kind goes out once: the walk back is the next call's.
            shop.failed = false;
            self.shop = Some(shop);
            return Some(self.emit(NO_SHOP));
        }
        self.shop = None;
        Some(self.walk(shop.dest))
    }
}
