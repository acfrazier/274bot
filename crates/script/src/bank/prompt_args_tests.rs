//! Bank-family prompt-argument census.
//!
//! Census of every bank-family member that prompts, measured with the existing
//! channel encoder (`crate::channel::encode_arguments`, `channel.rs:36-48`).
//! Prompt bytes cross the isolate boundary in the FlatBuffer wire; the decoder
//! keeps its 64 KiB wire cap while producers stay under the new
//! `api::script_contract::PROMPT_ARGS_MAX_BYTES` bound (parent owns
//! `api/src/script_contract.rs`).
//!
//! Prompt shapes (see `machine.rs:162-165` family descriptors):
//! - `bank_deposit` / `match(name, id)` for `depositAllMatching`
//!   (`bank/seat_transfer.rs:450-466`, `slot/bank.rs:158-163`
//!   `matcher_with_id == true` when no trip is present).
//! - `bank_nearest` / `deposit(name)` for `bankNearest`
//!   (same `match_row` site with `matcher_with_id == false` for a trip).
//! - `bank_nearest` / `afterDeposit()` with no args (`slot/bank.rs:308-310`).
//! - `log(message)` for every bank family with a `log` callback
//!   (`machine.rs:274-289`): transfer `WaitReady` / deposit-view logs plus the
//!   opening dynamic name/coord logs and the NPC logs below.
//!
//! Longest real r289 candidates (no invented stress strings):
//! - Item `Black plateskirt (t)` (20 bytes) and its 20-byte ties
//!   `Black plateskirt (g)`, `Black kiteshield (t/g)`, `Agility arena ticket`.
//!   Provenance: `crates/api/data/game-data/289.json` `items` aliases
//!   `black_plateskirt_trim` id 3472 (cert 3668), `black_plateskirt_gold`
//!   id 3473, `black_kiteshield_trim` id 2589, `black_kiteshield_gold`
//!   id 2597, `agilityarena_ticket` id 2996. All five measure 20 bytes.
//! - Bank object `Shantay chest` (13 bytes). Provenance:
//!   `crates/api/data/game-data/bank-catalog.rs:19` (Shantay Pass object) and
//!   `289.json` `loc_names` id 2693 display `Shantay chest`. `Bank booth`
//!   (10 bytes) with op `Use-quickly` (11 bytes) is the longest opener pair;
//!   provenance `bank/seat_open.rs:1788-1790` defaults and fixtures such as
//!   `crates/script/tests/named_bank_approach.rs:303-305`.
//! - Banker `Gnome banker` (12 bytes). Provenance: `289.json` `npc_names`
//!   id 166 display `Gnome banker` config `gnomebanker` ops Talk-to/Bank.
//!   The frozen catalog bankers `Banker`/`Gundai` (`bank-catalog.rs:16-20`)
//!   are shorter (6 bytes) and also covered.
//! - Counter stand `(3308, 3120)`. Provenance: `bank-catalog.rs:19` Shantay
//!   Pass tile `(3308, 3120, 0)`; any real 4-digit bank tile encodes the same.
//!
//! Measured largest: 72 bytes encoded for the stepping-to-counter log with a
//! real 4-digit stand. Recommendation: `2 * 72 = 144`, next power of two
//! `256` (well under 1024). See the recorded evidence file
//! `TRACK-T2/T2.3/prompt-census.txt`.
//!
//! Producer-formatting note: `deposit`/`match` arg construction
//! (`seat_transfer.rs:452-456`), the `VIEW_NOT_READY` const
//! (`seat_transfer.rs:19`), and the inline `seat_open.rs` log `format!`
//! strings are production-private. The smallest visibility change that would
//! let this census call them directly instead of replicating their exact
//! format strings would be `pub(crate)` on `VIEW_NOT_READY` (and on the
//! `WaitReady` static, currently inline at `seat_transfer.rs:1028`) plus a
//! `pub(crate)` helper returning each `seat_open.rs` log line for a given
//! name/op/coords. That would add production surface for test convenience
//! only, so this census replicates the exact strings with file:line
//! provenance and invokes the real encoder. The NPC templates are the
//! exception: `bank::npc::Note::message` is already `pub` under `load`, so
//! the `load`-gated test below calls the producer directly.

#[cfg(feature = "load")]
use serde_json::{json, Value};

#[cfg(feature = "load")]
/// Longest real r289 item display names: every entry is 20 bytes.
/// Provenance: `crates/api/data/game-data/289.json` `items` (see module docs).
const LONGEST_ITEMS: &[&str] = &[
    "Black plateskirt (t)",
    "Black plateskirt (g)",
    "Black kiteshield (t)",
    "Black kiteshield (g)",
    "Agility arena ticket",
];

#[cfg(feature = "load")]
/// Representative item id for the `match(name, id)` shape. The id encodes as
/// one 8-byte number regardless of value, so any real id measures the same.
/// Provenance: `289.json` alias `black_plateskirt_trim` id 3472.
const LONGEST_ITEM_ID: i32 = 3472;

#[cfg(feature = "load")]
/// Longest frozen-catalog bank object name (13 bytes).
/// Provenance: `bank-catalog.rs:19` and `289.json` `loc_names` id 2693.
const LONGEST_BANK_OBJECT: &str = "Shantay chest";

#[cfg(feature = "load")]
/// Default booth name/op pair (10 + 11 bytes); longest opener combination.
/// Provenance: `bank/seat_open.rs:1788-1790`.
const BOOTH_NAME: &str = "Bank booth";
#[cfg(feature = "load")]
const BOOTH_OP: &str = "Use-quickly";

#[cfg(feature = "load")]
/// Longest real banker display (12 bytes).
/// Provenance: `289.json` `npc_names` id 166.
const LONGEST_BANKER: &str = "Gnome banker";

#[cfg(feature = "load")]
/// Real Shantay Pass stand from the frozen catalog.
/// Provenance: `bank-catalog.rs:19` tile `(3308, 3120, 0)`.
const STAND_X: i32 = 3308;
#[cfg(feature = "load")]
const STAND_Z: i32 = 3120;

#[cfg(feature = "load")]
fn encoded_len(args: &[Value]) -> usize {
    crate::channel::encode_arguments(args)
        .expect("bank prompt args stay in the bounded clone subset")
        .len()
}

#[cfg(feature = "load")]
fn assert_fits(label: &str, args: &[Value]) -> usize {
    let len = encoded_len(args);
    assert!(
        len <= api::script_contract::PROMPT_ARGS_MAX_BYTES,
        "{label} encoded {len} bytes exceeds PROMPT_ARGS_MAX_BYTES {}",
        api::script_contract::PROMPT_ARGS_MAX_BYTES
    );
    len
}

#[cfg(feature = "load")]
#[test]
fn bank_prompt_argument_census_fits_contract_bound() {
    let mut max = 0usize;

    // `bank_deposit` / `match(name, id)` for `depositAllMatching`
    // (`bank/seat_transfer.rs:450-466` with `matcher_with_id == true`).
    for name in LONGEST_ITEMS {
        assert_eq!(name.len(), 20, "census longest item is 20 bytes");
        let args = vec![json!(name), json!(LONGEST_ITEM_ID)];
        let len = assert_fits("depositAllMatching match(name, id)", &args);
        assert_eq!(len, 35, "match(name, id) with a 20-byte name is 35 bytes");
        max = max.max(len);
    }

    // `bank_nearest` / `deposit(name)` for `bankNearest`
    // (same site with `matcher_with_id == false` for a trip).
    for name in LONGEST_ITEMS {
        let args = vec![json!(name)];
        let len = assert_fits("bankNearest deposit(name)", &args);
        assert_eq!(len, 26, "deposit(name) with a 20-byte name is 26 bytes");
        max = max.max(len);
    }

    // `bank_nearest` / `afterDeposit()` with no args (`slot/bank.rs:308-310`).
    let len = assert_fits("bankNearest afterDeposit()", &[]);
    assert_eq!(len, 3, "empty args encode as the bare array header");
    max = max.max(len);

    // Transfer deposit-view log (`bank/seat_transfer.rs:19`, queued at
    // `bank/seat_transfer.rs:273-282`).
    let view_not_ready = "deposit view not ready \u{2014} waiting for the side backpack";
    let len = assert_fits("deposit-view log", &[json!(view_not_ready)]);
    assert_eq!(len, 62, "deposit-view log census");
    max = max.max(len);

    // Transfer `WaitReady` log (`bank/seat_transfer.rs:1026-1029`); the same
    // line is reused by `seat_open.rs:1135` and `npc.rs:71`.
    let opened_no_list = "bank: opened but the item list never arrived";
    let len = assert_fits("waitReady/open log", &[json!(opened_no_list)]);
    assert_eq!(len, 50, "opened-but-no-list census");
    max = max.max(len);

    // NPC logs via `bank::npc::Note::message` (`bank/npc.rs:66-72`,
    // queued at `bank/seat_open.rs:488`).
    let npc_lines = [
        format!("no '{LONGEST_BANKER}' in the scene to bank with"),
        format!("'{LONGEST_BANKER}' never opened a dialogue"),
        format!("could not get {LONGEST_BANKER} to open the bank"),
    ];
    let expected_npc = [49, 44, 49];
    for (line, expected) in npc_lines.iter().zip(expected_npc) {
        let len = assert_fits("npc log", &[json!(line)]);
        assert_eq!(len, expected, "npc log census for {line:?}");
        max = max.max(len);
    }
    // Frozen-catalog bankers are shorter but explicitly covered.
    for name in ["Banker", "Gundai"] {
        let line = format!("no '{name}' in the scene to bank with");
        let len = assert_fits("npc log (catalog banker)", &[json!(line)]);
        assert!(
            len < 49,
            "catalog banker {name:?} is shorter than the census"
        );
        max = max.max(len);
    }

    // Opening object logs (`bank/seat_open.rs:765,837,851,894,1011-1014`).
    let object_lines = [
        format!("no usable '{LONGEST_BANK_OBJECT}' in the scene"),
        format!("opening '{LONGEST_BANK_OBJECT}' before banking"),
        format!("'{LONGEST_BANK_OBJECT}' never became usable"),
        format!("no reachable tile beside '{LONGEST_BANK_OBJECT}' yet \u{2014} closing in"),
    ];
    let expected_object = [44, 44, 41, 65];
    for (line, expected) in object_lines.iter().zip(expected_object) {
        let len = assert_fits("opening object log", &[json!(line)]);
        assert_eq!(len, expected, "object log census for {line:?}");
        max = max.max(len);
    }

    // Opener pair logs: longest real combination is the default booth pair.
    let opener_default = format!("no '{BOOTH_NAME}' with '{BOOTH_OP}' in the scene");
    let len = assert_fits("opener log (booth pair)", &[json!(opener_default)]);
    assert_eq!(len, 53, "booth opener census");
    max = max.max(len);
    let opener_shantay = format!("no '{LONGEST_BANK_OBJECT}' with 'Open' in the scene");
    let len = assert_fits("opener log (shantay pair)", &[json!(opener_shantay)]);
    assert_eq!(len, 49, "shantay opener census");
    max = max.max(len);

    // Longest selected-loc bank/booth display still fits (conservative extra).
    // Provenance: `289.json` `loc_names` id 2215 display `Closed bank booth`.
    let closed_booth = "Closed bank booth";
    let line = format!("no reachable tile beside '{closed_booth}' yet \u{2014} closing in");
    let len = assert_fits("opening log (closed booth)", &[json!(line)]);
    assert_eq!(len, 69, "closed-booth reachable-line census");
    max = max.max(len);

    // Counter-step log with a real catalog stand
    // (`bank/seat_open.rs:1004-1007`).
    let stepping = format!(
        "booth didn't open \u{2014} stepping to the bank counter at ({STAND_X}, {STAND_Z})"
    );
    let len = assert_fits("counter-step log", &[json!(stepping)]);
    assert_eq!(len, 72, "counter-step census with (3308, 3120)");
    max = max.max(len);

    // Remaining static object-dialogue logs
    // (`bank/seat_open.rs:1087,1107`).
    let len = assert_fits(
        "continue-failed log",
        &[json!("bank: failed to Continue the object dialogue")],
    );
    assert_eq!(len, 50, "continue-failed census");
    max = max.max(len);
    let len = assert_fits(
        "dialogue-without-open log",
        &[json!(
            "bank: object dialogue completed without opening the bank"
        )],
    );
    assert_eq!(len, 62, "dialogue-without-open census");
    max = max.max(len);

    assert_eq!(
        max, 72,
        "census maximum is the 72-byte counter-step log with (3308, 3120)"
    );
}

/// The replicated NPC lines above match the producer templates exactly.
/// This calls the real `Note::message` formatter instead of re-encoding.
#[cfg(feature = "load")]
#[test]
fn npc_log_lines_match_producer_templates() {
    use super::npc::Note;

    assert_eq!(
        Note::NoBanker.message(LONGEST_BANKER),
        format!("no '{LONGEST_BANKER}' in the scene to bank with"),
    );
    assert_eq!(
        Note::NoDialogue.message(LONGEST_BANKER),
        format!("'{LONGEST_BANKER}' never opened a dialogue"),
    );
    assert_eq!(
        Note::NotOpened.message(LONGEST_BANKER),
        format!("could not get {LONGEST_BANKER} to open the bank"),
    );
    assert_eq!(
        Note::Unloaded.message(LONGEST_BANKER),
        "bank: opened but the item list never arrived",
    );
}
