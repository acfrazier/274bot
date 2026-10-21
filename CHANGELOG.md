# Changelog

All notable public changes to 274bot. Host workspace crate versions are `0.2.4` and
`publish = false` (not on crates.io). Git tags are `0.1.0`, `0.1.1`, …

## Unreleased

### Panel

- On Linux, the panel now follows the system's choice of graphics chip
  instead of preferring built-in graphics. This can help on desktops with
  two graphics chips. If "importing the supplied dmabufs failed" still
  appears, start it with `WGPU_POWER_PREF=high ./panel-play`.
- Panel and TUI now share `frontend-core` live scenario preparation, runner
  state and proof gates; screenshot capture and TUI proof output remain
  surface-specific.
- Every panel action that changes state goes through the shared command
  path that the TUI and the headless driver also use. This includes Start,
  the draw switches, the Nav config grants, session log, map bake, Quest
  Paths, marks, the filter, the vault and Quit. Each action returns a typed
  report.
- Fleet Assign & restart reports its paced Starts through completion, after
  the assignment is durable.
- Saved draw preferences (capture input, sidecar and focused 50 fps), the
  render-all and background-bots acknowledgements, the map-bake choice and
  the session log file are written only when you change them. Saving the
  panel's own window state no longer rewrites them.
- A capture frame that fills the input queue is reported, instead of
  dropping events silently.

### TUI

- Marks, the fleet filter, log saving, the session log file, the Nav and
  danger-routing settings, map bake, Quest Paths, vault unlock and Quit go
  through the same command path as the panel.

### Map baking

- Resuming an interrupted local map bake on Windows now repairs a damaged
  tile instead of failing with "Access is denied".

### Scripts

- Ground-item interactions keep the selected item ID through dispatch. Items
  with the same name on one tile, such as Halloween masks, no longer substitute
  for the selected type. Requests without an ID retain name-and-tile lookup.
  Native collection, hunt keys, clue key-keeper drops and trail-end rewards all
  retain the selected row's ID.
- Exact ground Take actions settle from inventory and ground evidence within
  four later observed game ticks after dispatch. Lost races do not penalize
  the item tile. Native scripts can report confirmed progress and wait for
  relevant evidence without disabling scheduler or connection checks.
- Native ground-item selections reject more than 32 IDs, so later items
  cannot bypass the selected-ID exclusion bitset.
- Grinch is a native Gathering card that collects selected visible holiday
  items around its Start tile. All 18 base choices default on. Set a radius
  from 1 through 16 tiles, use the nearest reachable or a named bank, and
  deposit only selected items before returning to the exact Start tile.
  Empty waits stay visible as Waiting; duplicate refusals exclude only that
  item until the next Start. Saved settings apply on the next Start.
- Grinch uses the player's network tile, not the rendered tile, for its Start
  tile and return arrival. A confirmed walking arrival no longer fails while
  the rendered player catches up.
- Native script failures keep their stop reason visible in the TUI status
  pane after the run stops. This includes Grinch stopping with a full pack
  that contains no selected item to deposit.

### Not in this release

- When Grinch banks, it uses the shared bank trip, which still waits up to a
  fixed time for some bank steps. These waits are planned to become game-timed.

### Fixed during development

#### Startup and saved state

- On a slow or busy disk, 274bot no longer reports a preference change as
  failed while the disk still saves it. This applies to a Nav config grant,
  danger routing, a draw switch, the session log file, map bake, Quest Paths
  and a notice's "Don't show again". The Nav config grants, danger routing
  and the session log file change when the disk confirms the save. When two
  changes to one of these settings wait for the disk, the later change
  applies. If you close 274bot before the disk confirms a change, 274bot can
  lose that change or a part of it. Check the setting after the next start.

#### Banking

- When bank selection cannot find a route, the session log now gives the
  routing error, start tile and number of candidate banks and access tiles.

#### Scenario checks

- The Ardougne Thiever fight check now prepares food and attacks a real Guard
  before script Start. It no longer depends on a random stall catch.
- The AutoFighter bank check now waits for public Bones to leave its starting
  field before script Start. The receipt records the wait and observed loot.
- Retired the earned-drop Rock Crab bank check. The separate seeded-cargo
  check keeps its name and still checks deposit, return and further combat.
- Grinch has repeatable live checks for private drops, contested respawns,
  duplicate refusals and selected-item banking. Each check runs on a private
  engine copy with log-only receipts. Respawn checks also add map rows.
  The shared engines and content do not change.

## [0.2.4] — 2026-10-20 — Beta 1 patch

### Toolchain

- Pinned the host and client toolchains and CI to Rust 1.98.1, including the
  trait-object vtable fix.

### Scripts

- Fixed Quester rejecting quests whose game-journal title differs from the
  script's label, including Rune Mysteries.
- Quester now waits for a random-event window to close before it reads the
  quest journal or the Shield of Arrav gang, instead of stopping the quest.
- Compat v1 game-message recording and all readers now use one per-context
  Rust history, bounded to 64 records. Only posted type 0 chat lines enter it.
  `mark`, `since`, `sawSince` and `firstSince` read it. `since` returns original
  message objects oldest first. Session resets clear history without replaying
  omitted pages. Posted rows lack packet provenance, so local type 0 notices
  remain indistinguishable from server game messages.
  See [Game messages](docs/api/script.md#game-messages-compat-v1).
- Gathering scripts can use the declared location options, booth defaults,
  map-square checks and location resolver. A shared Rust preset core serves
  compiled callers and compatibility adapters without a second resource catalog.
- Cooking location options and name lookups now run in Rust over the existing
  bank-bound catalog. Curated surfaces, unlock callbacks and returned object
  identities keep their existing behavior.
- The runecrafting compatibility catalog is materialized directly from the Rust
  route roster. Its names, order, default rune and real Tile values are unchanged;
  the unused posted content-page copy has been removed.
- Cow location, toll, bootstrap and loot-leash helpers are admitted as a complete
  Rust-backed compatibility family. Catalog order, selected-bank bindings,
  object identities and existing behavior are unchanged.
- Skills lookups and HP fraction calculations now run in Rust over the
  already-posted stats rows. Compatibility names, row ordering, missing-skill
  errors and boosted levels keep their existing behavior.
- Inventory, Equipment and the seven pack-rule helpers now run through the
  native inventory family. The backpack helpers select bank-side rows and
  capacity 28 only while the bank is open. Otherwise, they use posted inventory
  rows and capacity. Both helpers refuse an unposted bank-open page instead of
  treating it as a closed pack. Known-empty views stay empty. Existing Inventory
  and Equipment readers keep fresh posted rows, caller-visible getters and
  item-handle identity. Both backpack helpers
  remain on the public Inventory module only.
- World Tile copying, translation, equality, text conversion and posted-coordinate
  validation now run in Rust. Area sampling returns a real Tile in the same native
  call. Existing geometry bounds, plane rules and caller-visible values stay the
  same.
- Bank opening, selection, deposits, withdrawals, note mode and close operations
  now run in the native request seat. Compatibility callbacks keep their
  existing call order, return handling and thrown values. Bank snapshot reads
  and local bank calculations are owned by Rust.
  Paused bank walks revalidate their saved destination on Resume without
  repeating bank callbacks or extending the operation's active-time limit.
  Bank-opening log text is shortened at a UTF-8 boundary to fit the prompt
  byte limit, without changing the operation's result.
- Breaking change: NativeApi v2 no longer accepts `withdraw-load` or exposes
  `withdraw_load_result_seq` and `withdraw_load_result`. Use `withdraw-x` with
  the number of free slots or compatibility `Bank.withdrawLoad(name)`. The
  BoneBurier v2 example uses that method.
- Entity wrappers and queries run on the native boundary, including area and
  player-preferred nearest filters. Production collections select and rank from
  the decoded Rust scene, paired with the original public row objects only after
  successful snapshot publication. Wrapper getters still read their public
  `snap`, while `fromSnapshots` keeps caller-supplied rows and callbacks.
  Public classes, aliases, module helpers and interaction queues are unchanged.
- Scripts no longer have WebCrypto (`crypto.subtle`, `Crypto`, `CryptoKey`,
  `SubtleCrypto`) or the `URL`, `URLSearchParams` and `URLPattern` globals.
  `crypto.randomUUID()` and `crypto.getRandomValues()` still work.
- Script operations now share a bounded request seat across native cards and
  JavaScript. A second top-level Gatherer, combat, quest progress or Hunt
  Session reports **busy** while one Session is live. Capability roots started
  before a Session may coexist with it; a new Capability root while a Session
  is live reports busy, including under Hunt. A child started from a Session
  hook can still run. A fifth concurrent independent operation reports busy
  instead of silently losing its gameplay actions.
  A seat-refused replacement leaves the previous awaited operation running
  until its own terminal result arrives.
- Stops and walk releases keep their place through a random-event hold and
  bypass a session's gameplay foreground. Gameplay emitted while the guardian
  holds the script is discarded, never replayed after the event. Actions
  accepted before an ordinary operation completes retain their authority;
  cancelling the operation still invalidates its queued actions. A non-kept
  Stop or removal drops the old seat before a new run starts.
  A Gatherer or combat Stop also survives a lost Start batch during a kept
  reconnect; an interrupted Hunt safespot walk asks the seat for a fresh walk
  in the new session instead of replaying the old click.
- Walks that arrive normally free their seat release slot, so repeated walks
  do not block later actions. A kept reconnect discards old gameplay walks.
  A wait whose walk was not delivered requests a fresh walk through the new
  session's seat and still settles on arrival. During a guardian hold, all
  walk terminals free their slots after resume. A held seat keeps one
  pending batch and discards only gameplay. Every walk release (up to 16
  handles), Gather Stop and Combat Stop is kept; a repeated release of the
  same target and id is kept once. The run policy, camera yaw, recovery
  anchor reply and progress stamps keep their newest value, so the host
  applies the value the script set last. Wait enqueue and settle facts fold
  into one exact summary that gives the watchdog the same wait count as the
  events in order. BroadcastChannel opens, closes and posts, and route
  inspections, stay in order. When that batch is full, only later
  BroadcastChannel posts are refused: the script's channel gets one
  `messageerror` with the reason, and each refused post is counted. After
  the hold, the host hands the kept batch to the seat in pages of at most
  one take (141 rows besides releases and the policy), so no kept open,
  close, inspection or newest value is lost however long the hold was. The
  newest values arrive after the ordered rows, and actions that the script
  makes after the hold wait behind the remaining pages. Kept releases go to
  the seat with the first page even when the seat queue is full, so a walk
  abort that frees a release slot is never held back. A new hold while
  pages still wait keeps the waiting batch's cancels, starts and other
  controls, which apply after its last page.
- A stop that a script sends while the map loads, while the welcome screen
  is open or while the watchdog recovers a stuck bot is no longer lost. The
  walk aborts reach the game in the first frame after that hold, in the
  order the script sent them. A Gather Stop or Combat Stop takes effect at
  once, so it can take effect before a walk abort that the script sent
  earlier. A run policy that the script sets during such a hold is no
  longer lost either: the last one the script set applies after the hold.
  Other game actions that the script sends during the hold are still
  dropped. A batch that the script sent before a map load started can still
  reach the game after the load, as it did before this change. Only a
  random-event hold now freezes the script's request seat: a map load or
  the welcome screen no longer keeps the script's other actions for later.
- A saturated script tick keeps its release and control rows ahead of gameplay.
  It dispatches the first 141 other rows in their original order, drops the
  excess, and reports one overflow diagnostic instead of failing the batch.
  The guarded append copies nested row values, refuses accessors and Proxies
  without calling them, and counts refused rows. Repeated aborts for one walk
  coalesce; a seventeenth distinct walk release is still refused.
- Compat cards and native V2 scripts now capture the first and subsequent
  JavaScript interactions in one guarded queue. Machine actions keep their
  order relative to queued script actions. The seat still bounds each tick.
- Native V2 tick listeners run before waits resume on each eligible tick.
  Their requests retain the root seat context.
- A quest progress refusal retains its detailed result when its final page
  and failed seat terminal arrive in the same snapshot.
- Gatherer and combat Promises wait for the family's detailed terminal after
  the seat ends their authority, rather than resolving with an empty result.
  A watchdog recovery-anchor reply also arrives while gameplay is held.
- Async walk wait and progress facts admitted by the seat reach the watchdog
  rather than the game-action dispatcher, including after a kept reconnect.
- Per-slot diagnostics expose dropped seat actions and late control copies as
  read-only counters. Duplicate acknowledgements count without noisy error logs.
- Reload and catalog refresh prepare source, siblings and settings off the UI
  thread. Their buttons show **Reloading…** or **Refreshing catalog…** through
  preparation and validation, rather than calling both stages validation.
- Load-script validation now waits for bounded admission without failing
  merely because multiple refreshes submit together. If the independent JS
  deadline service refuses a running execution, its tick error identifies
  the service condition instead of calling the execution runaway.
- A poisoned Load-script isolate can now retire without stopping the shared
  deadline supervisor or refusing new scripts for other accounts.
- Path-folder refreshes and Quester starts now prepare bounded Path batches
  off the UI and game threads. Superseded preparations do not replace the
  active Path registry.

- Card preparation now admits bounded background jobs for Starts, restarts,
  settings saves, gathering facts, catalogue refreshes and Path reloads. When
  the worker is temporarily full, these actions wait for a receipt instead of
  failing. Invalid or oversized inputs still produce a visible refusal.
  The settings editor retains at most 64 pending native saves or deliveries.
  Further requests report a queue-limit refusal until earlier receipts settle.
- A card's source and sibling graph is limited to 8 MiB. The rs2b0t catalog
  accepts at most 4,096 registrations and 8,192 source files. Exceeding
  either limit reports a catalogue error rather than consuming unbounded resources.
- Background preparation coalesces replaceable progress rather than failing
  a valid large catalogue at a progress-count limit. If an arbitrary job
  fills its immutable progress custody, it still completes; later progress
  updates are dropped and reported as diagnostics. A contained job panic is
  reported as **background work job failed internally**, not as a slice
  allowance violation.
- A Quester folder accepts at most 4,096 entries, 1 MiB per Path document and
  16 MiB of retained documents. Larger folders or documents report a typed
  validation error and leave the last successful registry available.
- Fixed: FlourCollector and other scripts that call `Reach.locOp` no longer
  stop with "not impl: Reach.locOp" on every loop. The call now walks to the
  object, opens doors on the way and uses the object, as the script expects.
  A refusal the script names, such as "You need something to put the flour
  in", ends the call at once instead of clicking again, also when older game
  messages are in the chat. An `expectMs` of 0 or less waits without a time
  limit, and a fractional value is accepted.
- Fixed: on a bank trip, the JiveDragons, JiveDemons and JiveKBD hunting
  scripts now deposit noted loot, such as noted Lobsters, instead of keeping
  it as food. When the pack also holds edible food or a kept potion of the
  same name, only the noted stack is deposited and the edible food and the
  potion stay in the pack. The scripts also deposit part-used potions that
  the script keeps only as full doses, and then withdraw fresh full doses.

### Navigation

- Release packages now also ship the 289 WalkTo map catalogue beside the
  shipped map terrain. The first map open installs it like the shipped
  terrain (identity and integrity checked) instead of deriving it from the
  client cache, so it opens faster. A catalogue shipped for another client
  cache, or a damaged one, falls back to the existing local derive.
- Route planning now runs independently for each request, with a 192 MiB
  memory limit for each search. A request waits when search capacity is full.
  Replacing or stopping a walk cancels its old search, so a late result
  cannot replace the new walk. Refused routes no longer appear to wait for
  search capacity.
  Searches still check cancellation and time limits throughout their work.
- Routes share transport details instead of copying them for every hop.
  Plans that fetch banked supplies account for carried and worn requirements
  without reducing the route-search budget.
- Bots that need banked route supplies now walk to the bank access with the
  shortest walking path they can reach, instead of the nearest access in a
  straight line. An unreachable access at the nearest bank no longer stops the
  bank trip.
- Single-tile walks and radius walks keep their existing danger-zone refusal
  rules, even when a radius search has only one candidate.
- Route inspection now runs independently, including banked supply planning
  and danger-zone explanations. Each inspection keeps its 192 MiB memory limit.
  Reported transport names and actions remain unchanged.
  Busy search capacity or result storage delays a valid inspection instead
  of refusing it. Reset cancels the current inspection and prevents stale
  output. Recent results remain available until acknowledged or replaced.
- Fixed: walks through the Ardougne lever to the Wilderness (for example to
  the Mage Arena bank) no longer stop at the lever on an account that has
  not pulled it before. The walk now answers the lever's Wilderness warning
  with "Yes I'm brave." and does not change the account's "don't show this
  message again" choice.
- Fixed: a walk no longer gives up when the game ignores a walk click, for
  example right after the bot pays the Al Kharid toll gate on the way from
  the Shantay Pass to Varrock. The walk now sends the click again once the
  bot has stood on the same tile for five game ticks after the game cancelled
  the click. A walk that is still under way is left alone.
- Fixed: routes no longer go through a door from a tile beside it that a wall
  cuts off from the door. Such a route made the bot walk a long way round,
  and the walk could give up before the bot got there (for example at the
  Varrock house door on the way to Aubury during Rune Mysteries). Walks now
  go to the door's own side before they open it.
- Fixed: Compat v1 scripts that open a bank by walking to its stand, such as
  FlourCollector on its way back from Sinclair Mansion, no longer refuse that
  walk because of danger zones. Their other walks already crossed the same
  zones, so the script now walks out and back the same way.

### Random-event guardian

- Disabling random-event handling no longer leaves Gatherer or other scripts
  waiting forever on a detected event, such as a held Strange box. Events
  remain visible, but do not pause scripts while handling is disabled.

### Gatherer and banking

- Bank selection now runs independently of other searches, with a 192 MiB
  memory limit for each selection. Busy search capacity or result storage
  delays a selection instead of losing it.
  After five seconds, a selection that will walk to the bank switches to a
  fallback route search. If search capacity is still full, it waits for
  capacity before that search starts. Its result can therefore take longer
  than five seconds.
  A selection that does not walk keeps the first bank without a route after
  five seconds, even when search capacity is full.
  An oversized request or a failure to start its search reports a failed
  selection instead of waiting forever. Following a selected bank route and
  displaying it on the map share one route, which avoids extra route copies.

- A Gatherer that stalls on its return from the bank now replans once from
  its current position before stopping. It still requires arrival at the
  gathering site and reports routes that remain blocked.
- Gatherer and Quester banking now check the current inventory and retry once
  after a refused or ineffective deposit. Items already deposited are not
  counted twice, and protected items remain protected. Gatherer keeps unbankable
  items and continues gathering whenever the pack still has space after depositing,
  including tool-fetch and supply-only trips that deposit nothing. A pack filled
  by actual supply withdrawals reports that no gathering space remains. Both
  scripts report a full bank or full item stack. Quester still refuses
  unbankable deposits.
  Unsolved Strange boxes multiply over time and their stacks spread into freed
  inventory slots. With event handling disabled, if no supply withdrawals fill
  the pack first, Gatherer stops with the unbankable-item reason once the boxes
  fill it.

### Saved state

- A vault file that ends right after its header is now reported as a corrupt
  vault file, not a wrong passphrase. A file damaged further in still reports
  a wrong passphrase, because the encryption can't tell the two apart.
- Vault error messages in the panel and the log no longer quote values from
  the decrypted vault.
- A saved bank hint, `loadouts.json` or `panel-ui.json` that 274bot can't
  read (a bank hint from another version or for another account, a file
  larger than its limit or writable by other users, or a damaged bank hint
  or `loadouts.json`) is now left untouched and named in the log with the
  reason. It used to be treated as empty and overwritten at the next save.
  A JavaScript script Start is refused, with the file and the reason, while
  `loadouts.json` can't be read; a Quester stops the Path that needs it.
- A damaged `panel-ui.json` (not JSON, or JSON that isn't an object) is
  still moved aside to `panel-ui.json.corrupt-<time>` by the next
  preference save, which then starts fresh preferences, as before; until
  then, settings read from it fail closed. Now the copy keeps exactly the
  damaged file's bytes and is on disk before the file is replaced, and the
  log shows a warning and an audit line naming the copy. A `panel-ui.json`
  refused for its permissions or size is never moved aside: preference
  saves fail with a message until you repair or remove it.
- A saved file holding something this version can't write back (a field it
  doesn't know in a bank hint, a loadout row it can't read, a setting in
  `panel-ui.json` it can't store as it is, for example one whose name has a
  dot or a space in it) is read but never rewritten, so nothing in it is
  lost. `panel-ui.json` has no version of its own: one written by another
  version is read and saved like any other, and a save keeps every setting
  in it, including ones this version doesn't know.
- A bank hint now records the game content it was seen under, and a hint
  from other content is not used. The log says so; remove the file to start
  a new hint.
- The Loadouts window (panel and TUI) and every preference save now go
  through the same saving service as the bots, so a form save and a bot's
  save can no longer overwrite each other. If the loadouts changed since
  the window loaded them (another 274bot on the same home folder), the save
  says so and keeps your edit for another try. Renaming a loadout to
  another loadout's name is refused with "A loadout named "…" already
  exists." (it used to leave two loadouts with one name).
- If `loadouts.json` already has loadouts that share a name, 274bot renames
  them apart when it reads the file: the second becomes "<name> (2)", the
  third "<name> (3)", and so on, skipping a name another loadout already
  has (a very long name is shortened to make room). Every loadout keeps its
  gear and its place in the list. Before anything changes, the file is
  copied exactly as it was to `loadouts.json.duplicates-<time>`, and the log
  shows a warning and an audit line naming the copy and each renamed
  loadout. Before, editing, renaming or removing one of those loadouts could
  change or remove another one of the same name. If the copy fails, the save
  fails before the file is replaced, or the file also holds a loadout row
  this version can't write back, the file is left as it was and treated as
  one 274bot can't read until it can be renamed. If the file was replaced
  but 274bot couldn't confirm it reached the disk, the log warns that the
  renames aren't saved for certain yet, and the renamed loadouts are used.
- Renaming a loadout saves it in one step, in its place in the list, and
  never loses it. If the save fails before the file is replaced, the
  loadout stays saved under its old name. If the file was replaced but
  274bot couldn't confirm it reached the disk (an error flushing its
  folder), the save is reported as an error, and the loadout may already be
  saved under its new name.
- Two 274bot processes on the same home folder take turns writing each
  saved file, and a save is reported done only when the file still holds it
  after it reached the disk: a change by another program in the meantime is
  reported as a conflict, never as saved.
- No save is reported done before the file and its folder entries are on
  disk, also after a failed attempt. Closing 274bot finishes saves still
  waiting (up to 60 seconds); a save still waiting past that is reported
  cancelled in the log, one line per save, instead of being dropped
  silently.

### Login

- A login that finishes after its lease deadline, Logout, or world switch
  cannot become an active session. An expired handshake keeps its place until
  it actually returns, so another bot cannot overlap it. The queue cancels
  its socket and then admits the next bot in normal order.
- A login refused before it starts no longer uses the next bot's login
  spacing or device/address allowance. Refused and cancelled sockets close
  before the next queued login is admitted.
- Relog now keeps the login request after logging out and clears its pending
  notice when the replacement session starts. Queue cards distinguish a
  server-imposed login hold from spacing and label a retiring handshake
  **previous login is finishing**.
- Editing a running bot's uid applies to its next login without repeatedly
  cycling the login queue. Saving a new password does not count as a clean
  logout: clean-logout release pacing remains in effect. A bot already waiting
  in the login queue refuses its old login and rejoins at the back.
- Unread session-end boundaries are delivered in order up to a bound of 32;
  the consuming lifecycle read reports how many older boundaries were missed
  if the bound fills.

### Panel

- Map baking now runs independently of other background work. Existing map
  caches rebuild once for the updated map preparation.
  Progress shows both catalogue preparation and terrain baking.
  Map baking continues if another background worker stops responding.
- Map preparation no longer locks its output cache while it only reads inputs.
  Oversized caches recover by removing unused files and rebuilding.
  An interrupted bake checks saved batches and later tiles, then replaces
  damaged files before it publishes terrain.
  Opening the map checks existing terrain before starting another bake.
- Closing a map releases any unused work reservation.
  Cache cleanup preserves terrain still in use and removes the least recently
  used entries first. Release packaging waits for a bake to stop before it
  removes temporary files.
- WalkTo terrain tiles decode on a background worker using shared bounded work, rather
  than a separate map-decoding thread.
- Panning the WalkTo map no longer redraws squares that you just scrolled past.

- The Fleet window's filter matches several words and world numbers, the
  same as the TUI's: every word must match a bot's name or status, and
  `w301` or `world:301` matches its world.
- Start on the marked bots uses each bot's saved script when no card is
  selected in Scripts, instead of being unavailable until a card is
  picked. A marked bot with no saved script is named in the report.
- When nothing is marked and no card is selected, the Fleet window's
  Start, Assign and Assign & restart buttons now say "mark at least one
  profile" first, instead of only asking for a card.
- The Assign & restart confirmation names the first six bots it stops
  and the first six it starts, and counts the rest ("+N more") beside
  the total for each, instead of listing every name.
- Render-all draws up to 128 clients at once. A client past that shows
  "renderer refused: render head cap of 128 is full" in its own tile.
- Clients without a drawn view retain less render scratch memory. Hiding a view
  releases that scratch, and showing it again keeps the same game connection.
- On Windows, the instance-lock warning names the live panel or TUI that
  already holds the lock, even when this Windows account cannot open that
  process. It no longer calls that live copy an unknown process.

### TUI

- The Assign & restart confirmation names the bots whose running script
  it stops and the idle bots it starts, as the panel's does: the first
  six of each, with the rest counted ("+N more") beside each total.

### Profiles

- On Windows, giving the rs2b0t import a folder instead of the exported
  file now says it is not a regular file, as on Linux and macOS, instead of
  "cannot read …: Access is denied".
- The first launch downloads the game cache faster. It keeps ten requests
  to the update server in flight for the whole download, and it reads each
  answer as soon as it arrives.
- If 274bot cannot look up the name of the update server, the first
  launch's game cache download now stops at once with a connection error,
  and 274bot tries the next world if there is one. Before, the download
  waited until it timed out.
- When 274bot cannot reach a world's cache server, or cannot look up the
  name of a world's server, while it prepares the game cache, it now moves
  to the next world as a whole: it prepares the cache from that world, and
  bots set to Auto start on that world. After a server name fails to look
  up, 274bot skips each later world that uses that name. There is one
  exception. A cache server name can look up at first and then fail to look
  up during the cache download. Then `host-play` and the TUI (at each
  start), and the panel (only before it selects its server profile), can
  look up that name again for the next world. Each such world costs one
  more wait for the name lookup. Before, 274bot took only the cache from
  the next world and kept the first world's game server.

### Performance

- A bot running a script uses about 8 % less memory than in 0.2.3: about
  49.5 MiB instead of 54.0 MiB per bot with ten Thiever bots, measured on
  Linux. The lists of NPCs, objects, players and ground items that scripts
  read are now built in a more compact form.
- Each running bot copies its route facts (held and worn items, skills,
  settings and quest progress) only when it prepares a walk request, a bank
  selection or a route inspection. Before, its script watchdog also copied them on
  every frame, about 30 times each game tick. In a measured Gatherer bank
  loop, each bot now allocates about 0.25 MB/s less memory for these facts,
  about half of the earlier amount.

### Fixed during development

#### Scripts

- A bot running a script no longer uses about 14 MiB more memory than in
  0.2.3. The NPC, object, player and ground-item lists that scripts read are
  built compactly again. Their values, field order and query results are
  unchanged.
- The compatibility family table now includes magic, prayer, and selected-content
  data modules. Generation rejects a typed module without a family row.
  This corrects coverage reporting and does not activate new behavior.

#### Navigation

- Planning a bank trip for route supplies is fast again from places such as
  the Varrock sewers, where it had slowed to several seconds. If no bank access
  is reachable, danger-zone refusals keep their explanation, and route
  inspection checks each bank access before planning with banked supplies.
- Script routes remain visible in the panel game view. After a bank session,
  the final route is checked against the updated inventory and gear.
  Searches waiting for capacity resume when capacity becomes available.
  Multi-stage routes continue without an extra frame between search stages.
  Routes keep the same map paths and transport labels.
  Route overlays no longer block navigation updates while the panel draws the game view.
- Valid native walks and route assessments no longer fail before their searches
  start. Concurrent requests no longer cause false search-start failures.
- Short route searches use memory only for the parts of the search they visit.
  Their maximum search allowance remains unchanged.
- Walking to an area or to within reach of a tile with a radius of 17 tiles or
  more no longer fails before the route search starts. Such walks are planned
  again within the same memory limit as other routes.
- Route searches avoid duplicate clock and cancellation checks without changing
  cancellation or time limits.
- Large route searches no longer slow sharply as their stored search history grows.
  Their existing search budget and route order stay unchanged.
- Radius walks keep the requested center as their first goal when it is valid.
  Missing-supply checks retain space for the route's transports, not the full
  search budget, so later planning stages do not refuse short routes.
- A card's first walk now starts correctly. Bank-planned walks keep the selected
  approach tile and can equip a carried item without requiring a bank visit.
- Route searches are faster, especially for large searches. They return the
  same routes.
- Repeated route searches on macOS and Linux no longer retain and clear large
  unused search buffers. Synchronous searches avoid extra lookup suspension,
  per-edge call overhead and repeated heap-order checks without changing routes,
  search budgets or cooperative cancellation checks.
- Route searches avoid repeated comparisons when they select the next tile.
  They skip finished reachability proofs and unnecessary zone-check futures.
  Routes, search budgets and cancellation checks stay unchanged.

#### Live scenarios

- Live script scenarios no longer send a world-wide `::speed 600` command by
  default. A scenario sends `::speed` only when it explicitly asks for a
  tick, such as `QUESTER_TICK_MS=300` on Engine Q. Every request is sent and
  then confirmed by a system-chat line newer than that send; a matching
  confirmation already in the ring neither suppresses nor satisfies it.
  Scenario pass criteria are unchanged.

#### Maps

- Map baking saves progress in batches of 64 tiles. Cancelling or finishing a
  bake saves the remaining tiles. Terrain and catalogue saves avoid duplicate
  disk flushes without reducing recovery guarantees.
- Checking for missing terrain no longer creates cache files.
  Catalogue preparation avoids empty progress files and redundant disk flushes.
- A completed catalogue saves its final manifest without first rewriting a
  progress file. Cancelled and incomplete work still save their progress.
- Catalogue preparation groups small operations but still checks cancellation
  and time limits before each placement.
  It avoids a second world scan when all candidates fit its memory limit.
  Larger inputs keep the second scan.

#### Startup and saved state

- The first launch's game cache download no longer switches off background
  work on slower connections, and "Preparing cache snapshot" now shows how
  far the download has got.
- One slow disk write no longer leaves script cards, the script catalogue or
  profiles stopped until 274bot restarts.
- Windows now recovers damaged panel preferences and repairs duplicate loadout
  names after preserving the original file in a flushed copy.
- A loadout list that did not load can no longer save. Add, rename, remove
  and Save in the panel and the TUI refuse and show why the list did not
  load. The saved loadouts stay as they were.

#### Login

- Windows now promptly cancels blocked login transport I/O when a login attempt
  ends.

### Release engineering

- The architecture check refuses candidate Cargo configuration files without
  ASCII case distinctions, including files in vendored trees and symlinks.
  The check also refuses an ASCII case variant of `Cargo.toml`, because it
  replaces the scanned manifest on a case-insensitive file system.
  Every client `Cargo.toml` is now an exact protected input in `274bot-arch`.
  Added, removed, or changed manifests need an operator commit to update the pin.
  This replaces interpretation of vendored workspace membership and inheritance.
  Fork pointer changes with byte-identical manifests need no pin update.
  The check now refuses `git` dependency sources in product manifests.
  Only root patch and replacement entries that protected policy lists may
  appear in the product root. Other entries need an operator commit first.
  The check covers default-feature and optional activation for every protected
  package, even packages outside named configurations.
  Baseline updates use the branch after it merges current `dev`.
  Operators review removed records to retain permissions that other branches still need.
- The architecture checker, its tests, and the crate-edge policy now live
  in the operator-owned `274bot-arch` repo (`main`); this repo keeps the
  local-convenience-only `tools/architecture/run.sh` runner, and CI runs
  the check inline on pushes and PRs — cloning `274bot-arch` `main`,
  verifying the pinned rust-analyzer binary, then running `check.py
  --self-test` and `check.py --root` with no candidate-tree script
  execution and no `ARCH_REPO` override. The fine data (`ownership.json`,
  `inventories.json`, `foreign_types.json`) stays beside the runner and is
  reviewed like code. A new crate edge, edge kind, migration,
  optional-dependency activation, vendored path package, or checker change
  needs an operator commit in `274bot-arch` (`policy.toml` plus a
  regenerated `graph-provenance.json`, operator publishes first); propose
  it as a patch or branch bundle for the operator to push.

### Not in this release

- Native bank sorting (`sortBank`), quest-junk discovery (`findQuestJunk`), pack
  purging at a bank (`purgePackAtBank`) and coin withdrawal (`withdrawCoins`)
  are **Planned**, not included here. These calls still report `not impl`;
  moving existing banking into the native seat does not implement them.
- Caller-supplied bank-cost routers (`nearestWalkableBank`,
  `nearestWalkableBankAsync`, `bankCostForFinder`, `bankCostForNavigator`) and
  random bank pacing (`bankPace`, `bankPaceTicks`) remain deliberately
  unavailable. The host does not recreate the foreign router or random pacing.
- The JiveKBD hunting script (King Black Dragon) does not yet bring back Dragon
  bones reliably. In testing it ran out of food at the dragon and went to the
  bank without the bones. A fix is planned for 0.2.5.
- On Windows, resuming a local map bake that stopped part-way (for example after
  a crash) and left a damaged tile can fail with "Access is denied" instead of
  repairing the tile. Release packages ship the baked map, so this affects only
  self-built copies and local bakes. A fix is planned for 0.2.5.

### Known issues

- **Gatherer:** a Catherby fishing card started from the Fishing Guild can pick
  a spot by the Water Obelisk island, which the bot cannot walk to, and it stops
  at once with a walk failure. Move the bot to the Catherby fishing beach and
  start the card again. A fix is planned for 0.2.5. 0.2.3 has the same problem.
- **Panel (Linux):** on a desktop with two graphics chips (for example a Ryzen
  with built-in graphics plus a separate graphics card), the panel can close
  right after it starts with "importing the supplied dmabufs failed". Start it
  with `WGPU_POWER_PREF=high ./panel-play` until this is fixed. 0.2.0.2 to 0.2.3
  have the same problem.

## [0.2.3] — 2026-10-14 — Beta 1 patch

### Profiles

- **Import your accounts from rs2b0t.** In rs2b0t, export your profiles from
  Settings (this writes `rs2b0t-profiles.json`). Then choose **Import from
  rs2b0t…** in the panel's Profiles window, or **Import rs2b0t profiles…** in
  the TUI command palette, and give the file's path. The vault must be
  unlocked, and nothing is saved until you confirm the preview.
  - **What comes over:** each account's username, password (into the
    encrypted vault) and world; its selected script and saved script
    settings, matched by name in your rs2b0t script catalog (import the
    catalog first); its genie lamp skill and clue duel partner; and
    rs2b0t's renderer-off setting, which turns the profile's display off.
  - **What doesn't:** tabs, rs2b0t's run-energy and route-drawing options,
    loadouts and order books. The preview and the report list each one with
    the reason.
  - **Accounts you already have** are kept unless you choose Overwrite,
    which changes only what the export carries. The preview shows each
    setting's old and new value, and importing the same file again changes
    nothing.
  - **Accounts the server would refuse** (a username over 12 characters, or
    a password that's empty or over 20 characters) are listed as unusable
    and skipped unless you ask for them. Passwords are never shortened.
  - **Your passwords stay private.** The export file holds them in plain
    text, so once every imported account is confirmed saved, 274bot offers
    to delete it. A password never appears in the preview, the report or a
    log message; one that turns up in another field, such as a tab name,
    shows as `***`. The log's account column still shows account names,
    even one that matches a password.

### Quests and dialogues

- A conversation closed by an attack no longer counts as finished. Any hit,
  even a miss, closes the chat; a quest step now stops with "dialogue
  interrupted by combat" and can be retried, and script dialogues
  (`driveDialog` / `talkThrough`) report failure the same way.

### Random-event guardian

- When a random event attacks (a River troll, a Swarm, an angry Strange plant
  and similar), the bot keeps running between hits, misses included, instead
  of walking back to its spot next to the attacker. It runs until the
  attacker stops chasing and returns once the attacker is gone.
  - If it's boxed in and gets no farther away for about 13 seconds while
    still being attacked, it stops and hands back to the script, which can
    then eat or fight.
  - An attacker that's out of sight or has stopped attacking can keep the
    script paused for up to about 10 minutes.
  - Either way, the bot doesn't walk back toward the attacker.
- Panel and TUI have a new per-event Policies control for random events.
  Flee is the default and the only option that works yet.
- When a chopped tree becomes an Ent, the Guardian stops chopping and waits
  for the tree to return before the Gatherer resumes, so the Ent can't break
  the axe.
- Lost-tool recovery no longer gives up on a pickaxe or axe after one
  refused click. It retries on the next game tick, within the existing
  recovery limit.

### Scripts

- **More rs2b0t scripts and helpers work.** This release is checked against
  a newer rs2b0t:
  - JiveDragons, JiveDemons and JiveKBD now start. Their loot checks know
    which held items stack, including noted, poisoned and untradeable items.
  - CowKiller uses its supported locations and bank choices.
  - Banking: `shouldBankNow`, `isDisposableGatherJunk` and `waitBankReady`.
    The junk check covers gems, strange fruit, beer, kebabs, the
    random-event casket, fliers and half-eaten pies. `waitBankReady` returns
    as soon as the bank's item list has arrived, without rs2b0t's extra
    short pause. `parseBankStrategy('Loot count')` now returns the declared
    `items` strategy rather than `loot`.
  - Combat and banking: `facingPlayer`, `resolveCombatStyle` and
    `resolveBankOpenRoute`. Ranged plans honour caller-selected potions.
  - Tools: Attack gates (`toolAttackLevel`, `toolsNeedingEquip`) stay
    separate from held-tool selection (`bestHeldToolNames`,
    `surplusHeldToolNames`), and axe requirements use rs2b0t's level-1
    default.
  - Firemaking: `parseBurnMode`, `findBurnLane`, spot selection and
    tree-log helpers.
  - Hunting: `ENCLAVE_TARGET_OPTIONS`, `enclaveTargets` and `chasesTarget`.
    Paladin and Hero pickpocket leashes cover their 289 roaming areas.
  - Clues and dialogues: `pickByLine`, `isUnderground`, `needsHop`,
    `trailKit` and `nextCoordTool`; cake stalls get `classifySteal`.
  - Food checks, ranged plans, trade and shop batching, clue distance,
    door walking and mining helpers. Quest-only mining rocks stay out of
    the regular ore options.
  - TypeScript declarations cover script completion hooks, task labels,
    current task names and the `InvItem.noted` flag.
- A script that imports an rs2b0t value the bot doesn't support now loads;
  using that value stops the script with a `not impl` message naming it,
  instead of a missing-export error at Start.
- When a catalog script uses an rs2b0t member this version doesn't provide,
  Start names the member, without claiming the catalog is newer than this
  version (some older members are missing too).
- The Gatherer continues message boxes before waiting for its inventory tab,
  so a Mime emote-unlock message after the return teleport no longer stops
  gathering from resuming.

### Panel

- The "only render selected" checkbox is now called "only render focused".
  It always meant the bot shown in the Game pane, not the bots selected in
  the fleet list.
- On Linux Wayland desktops (such as KDE Plasma), Ctrl+V now pastes into the
  panel's text fields, including the vault password and profile fields.
- The Profiles window shows the profile list and the editor side by side
  when there is room, each scrolling on its own, with a divider you can
  drag. In the narrow default dock the editor sits under the list, and
  Close always stays in view.

### Not in this release

Planned work that didn't make this release, plus options and script calls
that visibly refuse. Where a fix is planned, the item says so.

**Quests and combat**

- Quest guides cover 25 of 69 quests. Haunted Mine is unavailable on the
  supported server. The guides that ship are still beta rather than all
  checked end to end. More quest coverage is planned for a later release.
- The Quester doesn't yet pick quest-aware supplies or pick up every quest
  where you left off: earned quest items, some mid-conversation resumes,
  following up on some quest targets, and coin costs in quest requirements
  aren't handled. Quest resume and recipe work is planned for a later
  release.
- Prayer flicking, safespots, luring around corners, attacking other players,
  range-aware standing spots, automatic gear switching and face-tank fallbacks
  aren't here. Combat settings the game doesn't support still refuse to start,
  and fights without progress still use the existing time limits. Combat
  tactics are planned for later combat work.
- Hunt keeps its two-minute fight limit and retreat behaviour; it hasn't moved
  to the full combat system. Hunt parity is planned for later combat work.

**Walking, gathering and clues**

- Walks into quest areas that unlock as a quest progresses, regional
  entrances and transports such as boats and gliders, the land route over
  White Wolf Mountain below level 51, and some traveller and cosmetic walks
  aren't fully handled yet. Fuller routing is planned for a later release.
- Gathering sites the route finder can't reach yet stay hidden: 90 on 289 and
  71 on 274. They're in Morytania, Tirannwn, the islands and boat or glider
  trips, places behind gates and fences, and caves and underground areas. A
  saved Gatherer card that names one refuses to start with an "unknown" site
  error; pick another site. They come back as the route finder learns the
  entrances, which is planned for a later release. A few enclosed spots have
  no known way in yet, so those may stay hidden.
- Apart from random events, the bot doesn't yet react when something attacks
  it during non-combat work: it doesn't run from, for example, an aggressive
  monster near a gathering spot, so a bot on a very long session can
