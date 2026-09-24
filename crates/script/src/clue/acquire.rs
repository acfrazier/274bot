use super::*;


/// Whether the identified row needs the coordinate trio: a selected
/// `trail_sextant=yes` param on the row itself.
///
/// The param is the whole of the membership and it is never widened: the
/// unguarded map, vague and riddle rows that do not carry it never enter this
/// arm, and no requirement is ever grown for them. The packed constrained 3554
/// clue carries it too and is refused above, before `Steady` is ever reached.
pub(super) fn needs_trio(row: &TrailMembershipRow) -> bool {
    row.params
        .iter()
        .any(|param| param.key == TRAIL_SEXTANT && param.value == SEXTANT_YES)
}

/// The three selected givers of the published `trio_givers` family, each
/// resolved by its own alias and only when that row publishes the unique jm2
/// spawn this arm walks to.
///
/// A family that is absent, and one that does not publish all three identities
/// with a spawn, is not a giver set: the intercept does not fire and the row
/// keeps the landed guarded or unguarded-dig arm it has, rather than walking to
/// an invented tile or playing half a chain.
pub(super) struct TrioGivers<'a> {
    pub(super) professor: &'a TrioGiverRow,
    pub(super) murphy: &'a TrioGiverRow,
    pub(super) kojo: &'a TrioGiverRow,
}

pub(super) fn trio_givers(selected: &SelectedGameData) -> Option<TrioGivers<'_>> {
    let rows = &selected.trio_givers()?.rows;
    let spawned = |alias: &str| {
        rows.iter()
            .find(|row| row.alias == alias && row.spawn.is_some())
    };
    Some(TrioGivers {
        professor: spawned(OBSERVATORY_PROFESSOR)?,
        murphy: spawned(MURPHY)?,
        kojo: spawned(BROTHER_KOJO)?,
    })
}

/// This row's own trio-acquire entry: the published givers and the first tool
/// this call's posted pack is short of, or `None` when this call is not the
/// intercept's — no selected pin at all, no three unique-spawn givers, or a
/// pack that already holds the whole trio.
///
/// `None` is the fall-through: the landed guarded and unguarded-dig arms run
/// exactly as they did before this arm existed.
pub(super) fn trio_plan<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Option<(TrioGivers<'a>, Tool)> {
    let selected = selected?;
    Some((trio_givers(selected)?, first_missing_tool(selected, input)?))
}

/// The first tool of the frozen order this call's posted pack does not hold: a
/// selected alias joins that item's own selected id — `2574` / `2575` / `2576`
/// on both pins, never a copied number — a missing item row is not held, and
/// only a posted positive count holds. `None` is the posted `hasAllTrio`, the
/// intercept's own completion.
///
/// The page is the already-marshalled `inv` sequence the collect arm reads out
/// of the same posted `snapshot.inv`, read at call time. A page that did not
/// post it holds nothing, so the tool reads as missing and the arm walks rather
/// than Digging without it. The display names are corroboration and never a
/// second identity: the join is the id.
pub(super) fn first_missing_tool(selected: &SelectedGameData, input: &Value) -> Option<Tool> {
    for (tool, alias) in [Tool::Sextant, Tool::Watch, Tool::Chart]
        .into_iter()
        .zip(TRIO_ITEMS)
    {
        let held = selected
            .item_by_alias(alias)
            .is_some_and(|item| pack_holds(input, item.id));
        if !held {
            return Some(tool);
        }
    }
    None
}

/// Whether this call's posted pack page holds `id`: a posted row of the
/// marshalled `inv` page whose own id is this one and whose count is positive.
/// A row that posted no count, a zero count and a page that posted no such id
/// all hold nothing, and the held order is never read — the trio is a set of
/// three identities, not a position in the pack.
pub(super) fn pack_holds(input: &Value, id: i32) -> bool {
    input
        .get("inv")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("id").and_then(i32_of) == Some(id)
                    && row
                        .get("count")
                        .and_then(i32_of)
                        .is_some_and(|count| count > 0)
            })
        })
}

/// One stop of a tool's chain: the published giver row this stop walks to, and
/// whether that giver is the professor — the one giver whose chat this arm
/// answers options in.
pub(super) struct Stop<'a> {
    pub(super) row: &'a TrioGiverRow,
    pub(super) professor: bool,
}

/// The stop a tool's own index works: the sextant is the professor's teach stop
/// and then Murphy's own, the watch is Kojo's alone and the chart is the
/// professor's again. Any index past a tool's own list is its last stop, so an
/// item that never lands keeps that giver working rather than inventing a
/// fourth one.
pub(super) fn stop_of<'a>(givers: &TrioGivers<'a>, tool: Tool, stop: usize) -> Stop<'a> {
    match tool {
        Tool::Sextant if stop == 0 => Stop {
            row: givers.professor,
            professor: true,
        },
        Tool::Sextant => Stop {
            row: givers.murphy,
            professor: false,
        },
        Tool::Watch => Stop {
            row: givers.kojo,
            professor: false,
        },
        Tool::Chart => Stop {
            row: givers.professor,
            professor: true,
        },
    }
}

/// That tool's own last stop index: the sextant's chain is two givers long, and
/// the watch's and the chart's are one each.
pub(super) fn last_stop(tool: Tool) -> usize {
    match tool {
        Tool::Sextant => 1,
        Tool::Watch | Tool::Chart => 0,
    }
}

/// This call's posted professor option: the first row — in posted order — whose
/// text ASCII-folds to one of the two selected closed-handler literals,
/// carrying that row's own posted 1-based slot.
///
/// A row whose text matches but whose slot is not a posted integer is skipped
/// rather than answered with an invented one, and a page that posted no such
/// row answers nothing at all.
pub(super) fn professor_option(input: &Value) -> Option<i32> {
    input
        .get("chat_options")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let text = row.get("text").and_then(Value::as_str)?;
            if !PROFESSOR_OPTIONS
                .iter()
                .any(|known| text.eq_ignore_ascii_case(known))
            {
                return None;
            }
            row.get("option").and_then(i32_of)
        })
}

/// Whether this call's page posted an option list with at least one row: what a
/// chat this arm may not answer waits behind. An unmatched list is never a
/// reason to send the last option, and an omitted list is not an empty one.
pub(super) fn options_posted(input: &Value) -> bool {
    input
        .get("chat_options")
        .and_then(Value::as_array)
        .is_some_and(|rows| !rows.is_empty())
}
impl ClueRuntime {

    /// `Steady` on an identified row that needs the coordinate tools while this
    /// call's posted pack is short of one or more of them: the acquire chain,
    /// one verb per call.
    ///
    /// The chain is the frozen `nextCoordTool` order over the selected
    /// `trio_givers` rows and nothing else. The sextant is the observatory
    /// professor's teach stop and then Murphy's own, the watch is Brother Kojo's
    /// alone, and the chart is the professor's again; a stop walks to its
    /// giver's published `{x, z, plane}` tile with `plane` as the verb's `level`
    /// and never to a frozen coordinate. The Talk-to is the talk arm's
    /// unique-spawn rule: the posted npc whose own packed id is the giver's, or
    /// failing that whose posted display name is, standing on that tile inside
    /// the frozen `ARRIVE_RADIUS` and carrying a posted talk action. A wanderer
    /// is never chased, nothing is Cleared, a lookalike alias is not this giver,
    /// and a page with no posted `here`, no posted npc page or no posted match
    /// waits at the tile with the token live.
    ///
    /// Behind that giver's open chat the arm sends what the stop's own giver
    /// publishes, and only that: at the professor one posted option whose text
    /// ASCII-folds to a selected closed-handler literal, answered with its own
    /// posted 1-based slot, and at Murphy and Kojo a posted `chat_continue`.
    /// A posted option list with no such row waits — never the last option,
    /// never a frozen fragment and never another quest's choice — and a chat
    /// with neither an option nor a continue is waited out rather than
    /// Talked-to again behind it.
    ///
    /// A stop is complete when its own chat has been posted open and then
    /// closed: the chat is the observation, so no deadline, hop or world state
    /// is invented for it. `hasAllTrio` is the intercept's own completion, read
    /// off this call's posted pack — only then do the landed guarded and
    /// unguarded dig arms run. A giver that is not posted, an item that does not
    /// land, a full pack and a locked door all wait with the token live:
    /// nothing is fetched, banked, shopped, Dropped or abandoned here, and no
    /// completion kind is ever emitted from this arm.
    pub(super) fn acquire(&mut self, givers: &TrioGivers<'_>, tool: Tool, input: &Value) -> Value {
        // This tool's own state: a call whose first missing tool is a later one
        // has watched the earlier tool's item land, so the stop index rebases to
        // that tool's first giver rather than carrying the old position over.
        let ready = dialog_ready(input);
        let mut state = match self.acquire {
            Some(state) if state.tool == tool => state,
            _ => Acquire {
                tool,
                stop: 0,
                open: false,
            },
        };
        if state.open && !ready {
            // The stop's own chat closed after it was posted open: this stop is
            // done, so the chain moves to that tool's next giver — its own last
            // one once the list is exhausted.
            state.stop = (state.stop + 1).min(last_stop(tool));
        }
        state.open = ready;
        self.acquire = Some(state);
        let stop = stop_of(givers, tool, state.stop);
        let Some(spawn) = stop.row.spawn.as_ref() else {
            // The entry filter publishes a spawn for every giver this arm walks;
            // a selected family that stops doing so waits rather than walking to
            // an invented tile.
            return self.emit("wait");
        };
        let tile = Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        };
        match arrival(tile, input) {
            // No posted `here`: no arrival claim to make and no walk to measure.
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            // Arrived behind an open chat: the answer, the continue, or nothing
            // this arm may send.
            Arrival::Arrived if ready => self.chat(&stop, input),
            Arrival::Arrived => match input.get("npcs").and_then(Value::as_array) {
                Some(page) => match pick_at_spawn(&NpcIdentity::Giver(stop.row), page, tile) {
                    Some(pick) => self.npc_verb(&pick),
                    // Arrived with no posted row of this giver's identity on the
                    // tile: stay there and wait, never chase a wanderer, never
                    // Clear and never take a second target.
                    None => self.emit("wait"),
                },
                // No posted npc page this call: no giver can be observed, so
                // there is nothing to Talk-to.
                None => self.emit("wait"),
            },
        }
    }

    /// The open giver chat this arm may drive: the professor's one selected
    /// option, then a posted `chat_continue`, then a wait for everything else.
    ///
    /// The rule is the stop's own giver and not the text alone — the professor
    /// is the only giver this chain answers options in, so a posted option list
    /// at Murphy or Kojo waits. A list the professor's two literals do not match
    /// waits as well: the last option, a frozen fragment and another quest's
    /// choice are all nothing this arm may send, and an open chat with neither
    /// an option nor a continue is waited out rather than Talked-to again.
    pub(super) fn chat(&self, stop: &Stop<'_>, input: &Value) -> Value {
        if stop.professor {
            if let Some(option) = professor_option(input) {
                return json!({
                    "kind": ANSWER,
                    "token": self.token,
                    "option": option,
                });
            }
        }
        if options_posted(input) {
            return self.emit("wait");
        }
        if continue_posted(input) {
            return self.emit(CONTINUE);
        }
        self.emit("wait")
    }
}
