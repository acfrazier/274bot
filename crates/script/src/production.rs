//! Rust-owned `ChatDialog` make and option sequencing, the `chat-dialog`
//! [`crate::machine`] family.
//!
//! Chat `make_products` already posts the Make/Smelt quantity buttons.
//! `ChatDialog.makeX` must click the posted Make-X control, wait the real
//! count-dialog latch, send one Answer-Count, wait the dialog closed, then
//! wait the make-menu to drop. `ChatDialog.make` presses the largest fixed
//! quantity and waits the modal to change; `chooseOption` answers the
//! matching option and waits the page to move. The anvil panel is a
//! distinct main-modal TYPE_INV with Make-N ops: `makeFromPanelMax` presses
//! the largest posted Make-N on the matched row and `makeFromPanel` the named
//! op (else the first posted one). JavaScript starts one
//! machine per call and awaits its boolean — it does not pick a product,
//! wait one tick and guess the count dialog is open.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, ItemRow, MakeProduct, Scene, Text};
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::Value;

/// Frozen family-1 count-dialog open wait.
pub const COUNT_OPEN_MS: u64 = 3_000;
/// Frozen count-dialog close wait after Answer-Count.
pub const COUNT_CLOSE_MS: u64 = 3_000;
/// Frozen chat make-menu close wait after the count dialog drops.
pub const MAKE_MENU_MS: u64 = 5_000;
/// Frozen anvil panel modal-change wait.
pub const PANEL_WAIT_MS: u64 = 5_000;
/// Frozen `ChatDialog.make` / `chooseOption` modal-change wait.
pub const MODAL_WAIT_MS: u64 = 3_000;

/// The posted facts this module decides from, borrowed from the isolate
/// scene. A logout forgets the session: only pages posted since login count.
fn probe(scene: &Scene) -> Probe<'_> {
    let session = scene.since_login();
    Probe {
        ingame: session.ingame().unwrap_or(false),
        count_dialog_open: session.count_dialog_open().unwrap_or(false),
        main_modal_id: session.main_modal_id().unwrap_or(-1),
        chat_modal_id: session.chat_modal_id(),
        chat_continue: session.chat_continue().unwrap_or(false),
        chat_options: session.chat_options().map_or(&[], Vec::as_slice),
        make_products: session.make_products().map_or(&[], Vec::as_slice),
        main_make: session
            .main_make()
            .and_then(Option::as_ref)
            .map(Vec::as_slice),
    }
}

/// The `ChatDialog` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Kind {
    Make,
    MakeX,
    MakeFromPanel,
    MakeFromPanelMax,
    ChooseOption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Make-X button sent; waiting `count_dialog_open`.
    WaitCountOpen,
    /// One Answer-Count sent; waiting the dialog to close.
    WaitCountClose,
    /// Count dialog closed; waiting chat make-products to drop.
    WaitMakeMenu,
    /// Anvil Make-N sent; waiting the main modal identity to leave `before`.
    WaitPanel { before: i32 },
    /// Make-N button sent; waiting the chat (else main) modal to change.
    WaitModal { chat: bool, before: i32 },
    /// Option answered; waiting the chat modal to change or offer Continue.
    WaitChoice { before: Option<i32> },
}

#[derive(Deserialize)]
pub(crate) struct ChatArgs {
    kind: Kind,
    /// The product / option / panel row text; absent picks the first.
    #[serde(default, rename = "match")]
    match_name: Option<String>,
    #[serde(default)]
    count: Value,
    /// `makeFromPanel`'s op label; absent presses the first posted op.
    #[serde(default)]
    op: Option<String>,
}

/// One `ChatDialog` make or option call.
pub(crate) struct ChatDialog {
    phase: Phase,
    /// Requested Make-X count.
    count: i32,
}

impl Family for ChatDialog {
    const NAME: &'static str = "chat-dialog";
    /// One chat dialog: a new call replaces the one in flight.
    const EXCLUSIVE: bool = true;
    type Args = ChatArgs;
    type Output = bool;

    fn begin(args: ChatArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let match_name = args.match_name.unwrap_or_default();
        observed::with(|scene| {
            let probe = probe(scene);
            match args.kind {
                Kind::Make => begin_make(&probe, &match_name, cx),
                Kind::ChooseOption => begin_choose(&probe, &match_name, cx),
                // Make-X and the anvil panel stop at once when not in game.
                _ if !probe.ingame => Begin::Done(false),
                Kind::MakeX => begin_make_x(&probe, match_name.trim(), &args.count, cx),
                Kind::MakeFromPanel => begin_panel(&probe, &match_name, args.op.as_deref(), cx),
                Kind::MakeFromPanelMax => begin_panel_max(&probe, match_name.trim(), cx),
            }
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        observed::with(|scene| self.step_on(&probe(scene), cx))
    }
}

struct Probe<'a> {
    ingame: bool,
    count_dialog_open: bool,
    main_modal_id: i32,
    /// `None` until a chat modal id was posted (the frozen `undefined`).
    chat_modal_id: Option<i32>,
    chat_continue: bool,
    chat_options: &'a [String],
    make_products: &'a [MakeProduct],
    /// `None` = the main skill-multi panel was not decoded this rebuild.
    main_make: Option<&'a [ItemRow]>,
}

impl Probe<'_> {
    /// `reader.modals()`: the chat and main modal ids, `-1` when unposted.
    fn chat_modal(&self) -> i32 {
        self.chat_modal_id.unwrap_or(-1)
    }
}

/// Frozen JS `a.toLowerCase().includes(b)`.
fn contains_ci(hay: &str, want: &str) -> bool {
    hay.to_lowercase().contains(want)
}

/// A whole JS number, which reaches Rust as a float beyond int32.
fn whole(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.fract() == 0.0)
            .map(|n| n as i64)
    })
}

fn run(phase: Phase, window: u64, cx: &mut Cx<'_>) -> Begin<ChatDialog> {
    cx.clock().arm(window);
    Begin::Run(ChatDialog { phase, count: 0 })
}

/// Frozen `ChatDialog.make`: the product containing `match` (else the
/// first), its largest fixed quantity (first of equals), then the modal
/// change.
fn begin_make(probe: &Probe<'_>, match_name: &str, cx: &mut Cx<'_>) -> Begin<ChatDialog> {
    let want = match_name.to_lowercase();
    let product = if want.is_empty() {
        probe.make_products.first()
    } else {
        probe
            .make_products
            .iter()
            .find(|p| contains_ci(&p.name, &want))
    };
    let button = product.and_then(|product| {
        product.buttons.iter().filter(|b| b.qty > 0).fold(
            None,
            |best: Option<&observed::MakeButton>, b| match best {
                Some(best) if best.qty >= b.qty => Some(best),
                _ => Some(b),
            },
        )
    });
    let Some(button) = button else {
        return Begin::Done(false);
    };
    let chat = probe.chat_modal() != -1;
    let before = if chat {
        probe.chat_modal()
    } else {
        probe.main_modal_id
    };
    cx.emit(InteractReq::IfButton {
        component_id: button.com_id,
    });
    run(Phase::WaitModal { chat, before }, MODAL_WAIT_MS, cx)
}

/// Frozen `ChatDialog.chooseOption`: the option containing `match` (else
/// the first), answered by its 1-based position.
fn begin_choose(probe: &Probe<'_>, match_name: &str, cx: &mut Cx<'_>) -> Begin<ChatDialog> {
    if probe.chat_options.is_empty() {
        return Begin::Done(false);
    }
    let want = match_name.to_lowercase();
    let option = if want.is_empty() {
        1
    } else {
        match probe
            .chat_options
            .iter()
            .position(|option| contains_ci(option, &want))
        {
            Some(index) => index as i32 + 1,
            None => return Begin::Done(false),
        }
    };
    cx.emit(InteractReq::Answer { option });
    let before = probe.chat_modal_id;
    run(Phase::WaitChoice { before }, MODAL_WAIT_MS, cx)
}

fn begin_make_x(
    probe: &Probe<'_>,
    match_name: &str,
    count: &Value,
    cx: &mut Cx<'_>,
) -> Begin<ChatDialog> {
    if match_name.is_empty() {
        return Begin::Done(false);
    }
    let want = match_name.to_ascii_lowercase();
    let Some(product) = probe
        .make_products
        .iter()
        .find(|p| p.name.to_ascii_lowercase().contains(&want))
    else {
        return Begin::Done(false);
    };
    let Some(button) = product
        .buttons
        .iter()
        .find(|b| b.qty == -1 && b.com_id >= 0)
    else {
        return Begin::Refuse("missing Make-X button".into());
    };
    let Some(count) = whole(count)
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| *n >= 0)
    else {
        return Begin::Done(false);
    };
    cx.emit(InteractReq::IfButton {
        component_id: button.com_id,
    });
    cx.clock().arm(COUNT_OPEN_MS);
    Begin::Run(ChatDialog {
        phase: Phase::WaitCountOpen,
        count,
    })
}

fn begin_panel_max(probe: &Probe<'_>, match_name: &str, cx: &mut Cx<'_>) -> Begin<ChatDialog> {
    let Some(rows) = probe.main_make else {
        return Begin::Refuse("missing anvil panel".into());
    };
    if match_name.is_empty() {
        return Begin::Done(false);
    }
    let want = match_name.to_ascii_lowercase();
    let Some(row) = rows
        .iter()
        .find(|row| row.name_or_empty().to_ascii_lowercase().contains(&want))
    else {
        return Begin::Done(false);
    };
    let Some((index, _)) = largest_make_op(&row.ops) else {
        return Begin::Done(false);
    };
    cx.emit(InteractReq::MakePanel {
        id: row.id,
        slot: row.slot_or_unset(),
        component: row.component_or_unset(),
        operation: (index + 1) as i32,
    });
    let before = probe.main_modal_id;
    run(Phase::WaitPanel { before }, PANEL_WAIT_MS, cx)
}

/// Frozen `ChatDialog.makeFromPanel(match, op?)`: the first named row whose
/// name contains `match`, the op equal to `op` (case-folded) or else the first
/// posted op, then the main modal change.
fn begin_panel(
    probe: &Probe<'_>,
    match_name: &str,
    op: Option<&str>,
    cx: &mut Cx<'_>,
) -> Begin<ChatDialog> {
    let Some(rows) = probe.main_make else {
        return Begin::Refuse("missing anvil panel".into());
    };
    let want = match_name.to_lowercase();
    let Some(row) = rows.iter().find(|row| {
        row.name
            .as_deref()
            .is_some_and(|name| contains_ci(name, &want))
    }) else {
        return Begin::Done(false);
    };
    let op = op.map(str::to_lowercase);
    let index = row.ops.iter().position(|posted| match &op {
        Some(op) => posted.to_lowercase() == *op,
        None => !posted.is_empty(),
    });
    let Some(index) = index else {
        return Begin::Done(false);
    };
    cx.emit(InteractReq::MakePanel {
        id: row.id,
        slot: row.slot_or_unset(),
        component: row.component_or_unset(),
        operation: (index + 1) as i32,
    });
    let before = probe.main_modal_id;
    run(Phase::WaitPanel { before }, PANEL_WAIT_MS, cx)
}

impl ChatDialog {
    fn step_on(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<bool> {
        let timed_out = cx.clock().bound_reached();
        match self.phase {
            // The frozen modal waits read the modal ids only: a logout that
            // clears them ends the wait as a change.
            Phase::WaitModal { chat, before } => {
                let now = if chat {
                    probe.chat_modal()
                } else {
                    probe.main_modal_id
                };
                done_or_wait(now != before, timed_out)
            }
            Phase::WaitChoice { before } => done_or_wait(
                probe.chat_modal_id != before || probe.chat_continue,
                timed_out,
            ),
            _ if !probe.ingame => Step::Done(false),
            Phase::WaitCountOpen => {
                if probe.count_dialog_open {
                    self.phase = Phase::WaitCountClose;
                    cx.clock().arm(COUNT_CLOSE_MS);
                    cx.emit(InteractReq::AnswerCount { value: self.count });
                    return Step::Wait;
                }
                done_or_wait(false, timed_out)
            }
            Phase::WaitCountClose => {
                if !probe.count_dialog_open {
                    self.phase = Phase::WaitMakeMenu;
                    cx.clock().arm(MAKE_MENU_MS);
                    return Step::Wait;
                }
                done_or_wait(false, timed_out)
            }
            Phase::WaitMakeMenu => done_or_wait(probe.make_products.is_empty(), timed_out),
            Phase::WaitPanel { before } => done_or_wait(probe.main_modal_id != before, timed_out),
        }
    }
}

/// The frozen wait order: the condition, then the timeout.
fn done_or_wait(settled: bool, timed_out: bool) -> Step<bool> {
    if settled {
        Step::Done(true)
    } else if timed_out {
        Step::Done(false)
    } else {
        Step::Wait
    }
}

/// The 0-based op index and parsed quantity of the largest posted Make-N
/// label. A Make op without digits counts as 1. Missing Make controls
/// refuse rather than inventing an index.
pub fn largest_make_op(ops: &[Text]) -> Option<(usize, i32)> {
    let mut best: Option<(usize, i32)> = None;
    for (index, op) in ops.iter().enumerate() {
        if op.is_empty() {
            continue;
        }
        if !op.to_ascii_lowercase().contains("make") {
            continue;
        }
        let qty = op
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<i32>()
            .unwrap_or(1);
        let qty = if qty < 1 { 1 } else { qty };
        match best {
            Some((_, best_qty)) if qty <= best_qty => {}
            _ => best = Some((index, qty)),
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Reply;
    use crate::observed::MakeButton;
    use crate::task_clock::InstantTaskClock;
    use std::time::{Duration, Instant};

    fn product(name: &str, buttons: &[(i32, i32)]) -> MakeProduct {
        MakeProduct {
            name: name.into(),
            buttons: buttons
                .iter()
                .map(|(qty, com_id)| MakeButton {
                    qty: *qty,
                    com_id: *com_id,
                })
                .collect(),
        }
    }

    fn panel(name: &str, id: i32, slot: i32, component: i32, ops: &[&str]) -> ItemRow {
        ItemRow {
            name: Some(name.into()),
            id,
            slot: Some(slot),
            component_id: Some(component),
            ops: ops.iter().map(|op| Text::from(*op)).collect(),
            ..ItemRow::default()
        }
    }

    fn probe<'a>(
        products: &'a [MakeProduct],
        main_make: Option<&'a [ItemRow]>,
        count_open: bool,
        main_modal: i32,
    ) -> Probe<'a> {
        Probe {
            ingame: true,
            count_dialog_open: count_open,
            main_modal_id: main_modal,
            chat_modal_id: Some(-1),
            chat_continue: false,
            chat_options: &[],
            make_products: products,
            main_make,
        }
    }

    fn machine(phase: Phase, count: i32) -> ChatDialog {
        ChatDialog { phase, count }
    }

    fn armed(window: u64) -> InstantTaskClock {
        let mut clock = InstantTaskClock::new();
        clock.arm(window);
        clock
    }

    fn expired() -> InstantTaskClock {
        let mut clock = InstantTaskClock::new();
        clock.deadline = Some(Instant::now() - Duration::from_millis(1));
        clock
    }

    /// One step: `None` while it waits, else the verdict; plus the ops.
    fn step(
        m: &mut ChatDialog,
        clock: &mut InstantTaskClock,
        probe: &Probe<'_>,
    ) -> (Option<bool>, Vec<InteractReq>) {
        let mut ops = Vec::new();
        let out = match m.step_on(probe, &mut Cx::test(&mut ops, clock, None)) {
            Step::Wait => None,
            Step::Done(ok) => Some(ok),
            _ => panic!("chat-dialog neither calls back nor fails"),
        };
        (out, ops)
    }

    /// A begin: the verdict it settled with (`None`: running), plus the ops.
    fn begin(
        f: impl FnOnce(&mut Cx<'_>) -> Begin<ChatDialog>,
    ) -> (Option<bool>, Vec<InteractReq>, Option<ChatDialog>) {
        let mut ops = Vec::new();
        let mut clock = InstantTaskClock::new();
        let reply: Option<Reply> = None;
        match f(&mut Cx::test(&mut ops, &mut clock, reply)) {
            Begin::Run(m) => (None, ops, Some(m)),
            Begin::Done(ok) => (Some(ok), ops, None),
            Begin::Refuse(why) => panic!("refused: {why}"),
        }
    }

    #[test]
    fn largest_make_op_picks_the_posted_make_n_not_a_guessed_first_slot() {
        let ops = [
            "Examine".into(),
            "Make 1".into(),
            "Make 5".into(),
            "Make 10".into(),
        ];
        assert_eq!(largest_make_op(&ops), Some((3, 10)));
        let only_make = ["Make".into()];
        assert_eq!(largest_make_op(&only_make), Some((0, 1)));
        let no_make = ["Examine".into(), "Drop".into()];
        assert!(largest_make_op(&no_make).is_none());
    }

    #[test]
    fn make_presses_the_largest_fixed_quantity_of_the_matched_product() {
        let products = [
            product("Bronze bar", &[(1, 10), (5, 11), (-1, 12)]),
            product("Iron bar", &[(1, 20), (10, 21), (10, 22), (-1, 23)]),
        ];
        let at = probe(&products, None, false, 3000);
        let (out, ops, _) = begin(|cx| begin_make(&at, "IRON", cx));
        assert_eq!(out, None);
        assert_eq!(
            ops,
            vec![InteractReq::IfButton { component_id: 21 }],
            "largest positive qty, first of equals, never Make-X"
        );
        let (_, ops, _) = begin(|cx| begin_make(&at, "", cx));
        assert_eq!(ops, vec![InteractReq::IfButton { component_id: 11 }]);
        let (out, ops, _) = begin(|cx| begin_make(&at, "steel", cx));
        assert_eq!((out, ops), (Some(false), vec![]));
    }

    #[test]
    fn make_settles_on_the_modal_it_pressed_from_changing() {
        let products = [product("Bronze bar", &[(5, 11)])];
        let at = probe(&products, None, false, 3000);
        let (_, _, m) = begin(|cx| begin_make(&at, "bronze", cx));
        let mut m = m.expect("running");
        let mut clock = armed(MODAL_WAIT_MS);
        assert_eq!(step(&mut m, &mut clock, &at), (None, vec![]));
        let moved = probe(&products, None, false, -1);
        assert_eq!(step(&mut m, &mut clock, &moved), (Some(true), vec![]));
        let mut late = expired();
        let mut m = machine(
            Phase::WaitModal {
                chat: false,
                before: 3000,
            },
            0,
        );
        assert_eq!(step(&mut m, &mut late, &at), (Some(false), vec![]));
    }

    #[test]
    fn choose_option_answers_the_matched_position_and_waits_the_page() {
        let options = ["Yes please.".to_string(), "No thanks.".to_string()];
        let at = Probe {
            chat_modal_id: Some(4882),
            chat_options: &options,
            ..probe(&[], None, false, -1)
        };
        let (out, ops, m) = begin(|cx| begin_choose(&at, "NO", cx));
        assert_eq!(out, None);
        assert_eq!(ops, vec![InteractReq::Answer { option: 2 }]);
        let (_, ops, _) = begin(|cx| begin_choose(&at, "", cx));
        assert_eq!(ops, vec![InteractReq::Answer { option: 1 }]);
        let (out, ops, _) = begin(|cx| begin_choose(&at, "maybe", cx));
        assert_eq!((out, ops), (Some(false), vec![]));

        let mut m = m.expect("running");
        let mut clock = armed(MODAL_WAIT_MS);
        assert_eq!(step(&mut m, &mut clock, &at), (None, vec![]));
        let cont = Probe {
            chat_continue: true,
            ..at
        };
        assert_eq!(step(&mut m, &mut clock, &cont), (Some(true), vec![]));
    }

    #[test]
    fn make_x_does_not_answer_count_until_the_dialog_is_posted_open() {
        let products = [product("Bow string", &[(-1, 8875), (10, 8876)])];
        let mut m = machine(Phase::WaitCountOpen, 28);
        let mut clock = armed(COUNT_OPEN_MS);
        let closed = probe(&products, None, false, -1);
        assert_eq!(step(&mut m, &mut clock, &closed), (None, vec![]));

        let open = probe(&products, None, true, -1);
        assert_eq!(
            step(&mut m, &mut clock, &open),
            (None, vec![InteractReq::AnswerCount { value: 28 }])
        );
        assert_eq!(m.phase, Phase::WaitCountClose);
    }

    #[test]
    fn make_x_count_open_timeout_sends_no_answer() {
        let products = [product("Bow string", &[(-1, 8875)])];
        let mut m = machine(Phase::WaitCountOpen, 28);
        let closed = probe(&products, None, false, -1);
        assert_eq!(step(&mut m, &mut expired(), &closed), (Some(false), vec![]));
    }

    #[test]
    fn make_x_waits_count_close_then_the_make_menu() {
        let products = [product("Bow string", &[(-1, 8875)])];
        let mut m = machine(Phase::WaitCountClose, 28);
        let mut clock = armed(COUNT_CLOSE_MS);
        let open = probe(&products, None, true, -1);
        assert_eq!(step(&mut m, &mut clock, &open), (None, vec![]));

        let closed = probe(&products, None, false, -1);
        assert_eq!(step(&mut m, &mut clock, &closed), (None, vec![]));
        assert_eq!(m.phase, Phase::WaitMakeMenu);

        let empty = probe(&[], None, false, -1);
        assert_eq!(step(&mut m, &mut clock, &empty), (Some(true), vec![]));
    }

    #[test]
    fn panel_max_presses_the_largest_make_n_and_succeeds_on_the_modal_change() {
        let rows = [panel(
            "Bronze dagger",
            1205,
            0,
            1119,
            &["Make 1", "Make 5", "Make 10"],
        )];
        let still = probe(&[], Some(rows.as_slice()), false, 3000);
        let (out, ops, m) = begin(|cx| begin_panel_max(&still, "dagger", cx));
        assert_eq!(out, None);
        assert_eq!(
            ops,
            vec![InteractReq::MakePanel {
                id: 1205,
                slot: 0,
                component: 1119,
                operation: 3,
            }]
        );
        let mut m = m.expect("running");
        let mut clock = armed(PANEL_WAIT_MS);
        assert_eq!(step(&mut m, &mut clock, &still), (None, vec![]));
        let closed = probe(&[], Some(rows.as_slice()), false, -1);
        assert_eq!(step(&mut m, &mut clock, &closed), (Some(true), vec![]));
    }

    #[test]
    fn panel_timeout_does_not_invent_a_second_press() {
        let rows = [panel("Bronze dagger", 1205, 0, 1119, &["Make 10"])];
        let mut m = machine(Phase::WaitPanel { before: 3000 }, 0);
        let still = probe(&[], Some(rows.as_slice()), false, 3000);
        assert_eq!(step(&mut m, &mut expired(), &still), (Some(false), vec![]));
    }
}
