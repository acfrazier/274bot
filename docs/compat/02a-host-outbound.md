# Revision-correct host outbound boundary

Date: 2026-09-10. Scope: implementation task 1 from
`docs/compat/02-host-boundary-design.md`, on
`codex/rs2b0t-multirevision` with client pin
`2be1697060e4d2b8b709ad4d5e54d12513b38333`.

## Source findings

- `api::Driver` now exposes the bound `ClientRevision`; legacy stubs default to
  R274 and the real `Client` returns its session revision.
- Direct host writers select the client-owned named protocol row at the write
  boundary. This covers typed IF_BUTTON/CLOSE_MODAL/RESUME_P_COUNTDIALOG,
  CLIENT_CHEAT, and the cardinal MOVE_GAMECLICK door step without adding a
  second production opcode table.
- The original named packet selects each typed payload shape. Unsupported
  forged `Send` values are rejected before the output cursor or ISAAC state can
  move. Existing `Send::write` and `WalkStep::write` remain the R274-compatible
  public paths.
- `LEGAL_SEND` remains the R274 compatibility constant;
  `legal_sends_for(revision)` selects every named row's id and length through
  the client mapper. R289 expectations are checked against the pinned primary
  fixture by name, including the legitimate numeric collisions OPPLAYER2=51
  and EVENT_MOUSE_CLICK=224.
- The production raw-write audit found only the typed sends, cheat, and
  WalkStep sites above. Ordinary NPC/player/loc/ground/item/widget operations
  continue through `Client::doAction`, while normal walking continues through
  `Client::tryMove`. `nav::traveller` reaches the door writer through
  `Interactions::pending_door_step`, so no nav product edit was required.
- Public-target cheat refusal remains before all output writes for both
  revisions. The existing `Client` `do_action` acceptance semantics are
  unchanged.

## Offline proof

Tests use real R289 `Client` drivers and primary-source byte literals for direct
button/close/count/cheat/door writers and named NPC, player, loc, ground-item,
held-item, and widget operations. Invalid named actions and forged typed sends
prove refusal without output mutation; the forged-send test also proves ISAAC
state is not consumed. Existing R274 tests remain unchanged and green.

Final commands:

- `cargo test -p api --test interact -- --nocapture`: 64 passed.
- `cargo test -p api`: 173 passed across unit/integration tests, 0 failed.
- `cargo clippy -p api --tests -- -D warnings`: passed.
- `git diff --check` on the three scoped Rust files: passed.

Raw red/green and final logs are retained under
`docs/compat/evidence/host-boundary/outbound/`. No live session or public 289
connection was used.
