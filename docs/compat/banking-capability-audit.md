# Banking capability preparation

Source audit at host `b9cacc6e`, both frozen catalogs unchanged. No new support or
live acceptance is claimed. Script loading/starting remains revision agnostic;
these findings define implementation and proof work, not an execution allowlist.

- `Banking.open` in the shim takes no options and calls `Bank.openNearest`.
  The 289-updated catalog supplies stand, boothName and boothOp (BankFletcher),
  while the primary API also defines nearby/destination/obstacle choices. Map
  required options to the Rust host navigation/interaction path; never run or
  clone the foreign Banking router. Existing `OpenStand` handles a generic booth
  or named NPC, but its booth arm currently ignores name/operation fields.
- `Bank.openBooth` currently ignores its supplied stand/name/op. Existing
  `openNearestAccess` refuses non-default access explicitly. Preserve honest
  unsupported-option behavior while completing the authorized access variants.
- The posted `bank_loaded` currently means an open component with a non-empty
  bank list. It cannot distinguish a freshly received empty bank from missing
  or stale contents. `snapshotReady`, `snapshotGeneration`, `waitSnapshotAfter`
  and `withdrawLoad` currently throw. The 289 primary Bank API distinguishes
  non-empty `loaded` from actual snapshot readiness; its empty-bank contract
  must be observed through Rust-owned packet/container publication.
- Withdraw-X currently queues a withdraw, waits one tick, queues answer-count,
  then waits for inventory movement (4000 ms). The host answers only an actually
  open count dialog. Verify delayed count-dialog publication and stale bank
  rows instead of counting the queued requests as success. Preserve established
  timeout values unless a scoped behavioral change is independently justified.
- `PendingBankFetch` currently advances some steps after a send: Open can pop
  before a fresh bank arrives, then Deposit/Withdraw/Wear/Close each use their
  current observations. The capability work must establish observed settlement
  before provisioning/return is qualified.
- Common-loot matching and includeCommon deposit matching currently throw;
  COMMON_BANK_LOOT is empty. ChickenKiller's PeriodicBank and DeathRecovery
  validators currently return false. Those are explicit ledger gaps, not
  working disabled-option branches or evidence that all options are supported.

Relevant source: `crates/script/src/shim/{bank,banking,periodic_bank,death_recovery}.js`,
`crates/host-play/src/lib.rs` (`dispatch_script_interact`, `step_bank_fetch`,
`build_snapshot_input`), `crates/api/src/snapshot.rs`, and the frozen
`src/bot/api/bank/{Bank,Banking}.ts` for both catalog commits. Do not copy their
foreign planner bodies. The required proof traverses actual script call, IPC,
Rust dispatch and posted result, with stale/empty/timeout/refusal cases and
script-caused live bank/provision/return progress after source review.
