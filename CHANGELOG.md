# Changelog

All notable public changes to 274bot. Host workspace crate versions are `0.2.0` and
`publish = false` (not on crates.io). Git tags are `0.1.0`, `0.1.1`, …

## [Unreleased] — 0.2.0.1

### Panel

- Turning MultiBox on before unlocking the vault no longer leaves the bot
  that logs in at unlock off the rail: it joins the rail like any profile
  you pick, so selecting another profile keeps both on the rail and the
  first one stays visible and controllable instead of running unseen.
- Picking a profile in MultiBox now shows that profile's own script card.
  Before, the previous bot's card stayed on screen, so Start or a parameter
  edit could act on the wrong card for the newly focused bot.
- Window buttons (Profiles, General config, Nav config, Loadouts, Script
  prefs, Log, Fleet, Debug, Browse, Load, Import catalog and the Fleet
  window's Walk to) now bring their window to the front, selecting its
  tab, when it is already open, instead of doing nothing. An open Loadouts
  keeps its selection and unsaved edits, and an open file browser keeps
  its folder and search.
- The rail's **+ add bot** button is now **Profiles…**; selecting a profile
  never starts its script.
- The profile setting **auto-login on title** is now called **Log in
  automatically**, with a tooltip saying what it does. Saved profiles keep
  their value.
- Nav config no longer scrolls sideways: the routing rows don't repeat
  "Global — applies to every walk." on each line, and one note above them
  says settings are global unless stated and that rs2b0t-compatible scripts
  always allow wilderness and bank fetch. Danger routing is now a full-width
  drop-down, with "When survivable"'s "not available yet" explanation on its
  own line.
- The Quest Paths folder, its load switch and Reload Paths moved from Nav
  config to Script prefs, under the Quester card. They still apply to every
  bot, and the heading says so.

### TUI

- Settings: the routing rows no longer repeat "· Global — applies to every
  walk." on each line; one note above them says the same as the panel's.
  The Quest Paths rows sit under a "Quester: Quest Paths (all bots)"
  heading.

### Navigation

- Fleet walks no longer leave eligible group members stuck at doors that
  close after each crossing, such as the Fishing Guild door; anyone held
  back retries automatically once the door is available again.

### Scripts

- Load scripts using `api.gather.run` now honor the card's
  `allowTeleports`, `allowWilderness` and `allowDangerZones` options for
  Gatherer routes only; the Load script's own rows keep their options, and
  session permissions remain off by default.

## [0.2.0] — 2026-10-08 — Beta 1

### Upgrading from 0.1.9.1

- **Server profiles.** `--profile NAME` now picks an entry from
  `~/.274bot/servers.json`, created on first use with `local-274`,
  `local-289` and `rs2b2t`. `--rs2b2t` selects the public one
  (`public-289` still works as a name). `--prod` and `BOT_TARGET` were
  removed and now error — use `--profile` (or `BOT_SERVER_PROFILE`, or
  `--rs2b2t`) instead. A valid `~/.274bot/worlds.json` is imported once
  into the `rs2b2t` roster; after that, edit `servers.json` (later changes
  to `worlds.json` are ignored). `--user` creates missing accounts with a
  fresh random game password.
- **Vault passphrase.** `BOT_VAULT_PASS` and `--vault-pass` were removed
  (anything on the command line or in the environment is visible to other
  users). `host-play` and `tui-play` ask at a hidden terminal prompt (twice
  for a new vault); pipe it with `--vault-pass-stdin`; the panel asks in
  its unlock window. Unlocking uses the passphrase exactly as typed, and
  vaults made by the 0.1.9.1 panel still open in the panel (at the
  `host-play`/`tui-play` prompt, type the passphrase without surrounding
  spaces). A new vault needs a passphrase that is not blank after trimming;
  strength is your choice. A first run of `tui-play` with no `--user` now
  stops with "vault has no profiles" instead of seeding a `test` profile;
  pass `--user NAME` to create the first one.
- **Navigation packs.** The pack format is now v16; packs you baked yourself
  with 0.1.9.1 or earlier are refused with a message to rebake them with
  `nav-pack`. The bundled pack is rebuilt for you.
- **Local engines.** The bot no longer guesses where a local game engine is.
  Give it with `--engine`, the `ENGINE_DIR` environment variable, or the
  profile's `login_key.engine_dir` in `servers.json`; without one, a local
  profile stops with a message saying how to set it. Public (rs2b2t)
  profiles need nothing.
- **Graphics.** On Windows the panel tries Vulkan first and falls back to
  Direct3D 12. On every platform the window now prefers the power-saving GPU.
  `WGPU_BACKEND` and `WGPU_POWER_PREF` override this (see FIRST-START);
  `WGPU_POWER_PREF=high` can make startup stall for minutes on some laptops.

### Quester

New in 0.2.0, for revision 289: a script that runs quests from built-in quest
guides.

- Built-in guides: Cook's Assistant, Sheep Shearer, Rune Mysteries,
  Romeo & Juliet, Imp Catcher, Vampire Slayer, Doric's Quest, Goblin
  Diplomacy, Witch's Potion, Prince Ali Rescue, Pirate's Treasure, Demon
  Slayer, The Knight's Sword, Death Plateau, The Tourist Trap, Priest in
  Peril, Clock Tower, Monk's Friend, Hazeel Cult and Plague City. Members
  quests run only on members worlds. A quest the server can't run (Haunted
  Mine) is listed with the reason instead of being offered.
- Five more guides ship as drafts that haven't been run yet: Druidic Ritual,
  Gertrude's Cat, Jungle Potion, Sea Slug and Tribal Totem. They're marked
  `[draft]` in the quest lists, and the script's status shows "Untested draft
  Path" while one runs. An empty list leaves them out unless "Include draft
  guides when no quests are picked" is on; picking one by name always runs it.
- Pick quests, skips and priorities from lists. An empty list runs every
  non-draft quest in the built-in order; priorities run first. Finished quests are
  skipped, and a quest that can't run on this server stays visible with its
  reason but can't be picked.
- Before a quest starts, the Quester checks its requirements (members world,
  quest points, levels, items and earlier quests) and names the one that
  blocks it.
- Missing supplies are fetched from the bank as part of the quest: it checks
  what you carry first, keeps unrelated items and tools, and banks only what
  it must to make room. Leftovers carry into the next quest. It plans from
  what the account's bank held last time it was open, so it usually goes
  straight to the withdraw; if the bank has already been checked this session
  and something required isn't there, the quest stops at once and says what's
  short (for example "need 300 Coins; held 0, banked 0").
- Quests gather, make items, fight and pickpocket where the quest needs it,
  using the same gathering, making and combat handling as the other scripts.
- Progress is always read from the game's quest journal, so Stop and Start,
  a dropped connection or a death pick up where the quest really is. After a
  death the bot walks back where it can; how many deaths a run tolerates is a
  setting (normally 2), counted across the whole queue.
- The panel and the TUI say why a quest is waiting or stopped and what would
  unblock it. A quest that stops making progress warns first, then stops
  with the step it was on.
- Per-account choices (for example the Family Crest gauntlet reward) stay
  with each account. The Quester's own settings can allow teleports, the
  Wilderness or danger zones for its walks when the global switches are off,
  and a guide can let a single walk cross the danger zones it needs.
- Custom quest guides can be loaded from a folder (Nav config), without
  rebuilding. A guide that replaces a built-in one overrides it, new ones
  appear as drafts, and a guide with a problem names the file, step and
  reason. See `docs/quester-paths.md`.

### Gatherer

New in 0.2.0: woodcutting, mining and fishing as a native script.

- Choose where to gather: a named place from the game map (for example
  Catherby fishing or Varrock East mine), a start tile, a custom area, or
  Auto, which searches outward for the nearest usable spots. Resources,
  fishing methods (grouped by spot and tool) and food are picked from lists.
- Bank mode banks the haul at the nearest bank it can walk to, keeping the
  tools and food for the next trip, then walks back; you can also name a bank
  to always use. Power mode drops the haul. Gems and other bonus finds are
  banked or dropped with it.
- If the bank is short on bait, food or runes, it takes what is there and
  keeps working, and stops only when something essential is missing, saying
  what. It plans trips from what the bank held last time it was open, so a
  partly stocked bank costs one trip, and once the bank has been checked a
  missing essential stops the Gatherer where it is instead of sending it on
  another trip. A tool you're wielding counts as carried.
- When a rock, tree or fishing spot runs out it moves straight to the next
  one, and waits for respawns when everything nearby is used up. It keeps
  clear of hazards such as gas, ents and whirlpools.
- A flown-off axe or pickaxe head is picked up and refitted; if it can't be
  recovered, the bot fetches a replacement tool from the bank.
- It recovers from deaths (normally up to two per run): it restocks, checks
  its kit and walks back before carrying on. It also resumes cleanly after a
  random event.

### Combat

- The Quester fights with melee, ranged weapons or magic. Magic picks the
  strongest spell you can cast, or a fixed order you choose, and reuses an
  autocast that is already set.
- In native fights, prayers you turned on yourself stay on. When a fight ends
  or you stop the script, only the prayers the bot switched on are turned off.
  Protection prayers follow each attacker's actual attack style, and potion
  sips and eating follow one shared rule.
- Hunts run from compatible scripts use the same protection prayers, food and
  potion rules as native fights, including dragonfire shields with a
  confirmed antifire sip.
- Loot that can't be reached is skipped instead of getting the bot stuck, and
  a target that turns out not to be attackable makes the bot walk out and say
  why.

### Clues

- Clue guardian and key-keeper fights use the full combat rules: protection
  matched to the attacker, food and potion sips, and the kill confirmed
  before moving on.
- Dying mid-clue ends that attempt cleanly, and the death is remembered across
  Stop and Start.
- Before an Entrana clue, the solver walks to a side of the bank booth it can
  actually use to bank the gear Entrana forbids, and stops with a clear error
  if the bank won't open. It no longer clicks an unreachable booth every tick.
- Clue searches walk beside the target and approach a side that can be used,
  including the Entrana drawers reached by boat, instead of trying to stand
  inside blocked scenery. Clue walks are sent once while in flight and stop
  with a clear error if arrival takes too long.

### Sessions

- After you log out, the game view's login screen keeps animating instead of
  freezing, and its Log In button (or Enter) logs that bot back in. While a
  login is waiting (for example the "already logged in" wait) the screen
  keeps animating and says what it is waiting for.
- Switching the memory mode (low/high) in the panel or the TUI (the TUI
  settings gain a memory row) queues the whole mode for the next login; the
  game view, tabs and sound stay as they are until then. While a logged-in
  bot has a different mode queued, the panel and TUI say so and offer
  **Relog now**, which logs out and back in through the normal login queue
  (with a warning if a script is running). If the connection drops, the next
  Log in keeps the memory mode already in use, and the notice says so instead
  of promising that login will apply a queued change. A change you make after
  logging out yourself still applies on the next Log in.
- On local-engine profiles the panel's tutorial check no longer leaves a
  "Click to continue" box over the chat: it is clicked away once per box, and
  only while the box is showing, so chat options are never mis-clicked.
- A genie-lamp random event whose skill menu does not open is now rubbed
  again (up to three tries) instead of being given up on after one attempt,
  which left the lamp unredeemed.
- When a random event drops a bot into the Maze, it gives up after twice the
  event's own time limit instead of waiting there indefinitely. A script run
  with random events turned off that finds itself trapped in the Maze or on
  the Mime's stage stops and says so, instead of standing there showing
  Working.

### Navigation

- Walking out of the desert through the Shantay Pass gate on a members world
  no longer stalls for about 36 seconds and then fails when the walk must end
  on an exact tile (for example from Irena to the Shantay bank chest).
- Script and map walks start moving about half a second sooner: the first
  step no longer waits for the next game tick after the route is ready.
- Walks between Lumbridge and Al Kharid now pay the 10-coin toll at the border
  gate instead of stopping there. Without 10 coins the walk stops and says
  the coins are missing.
- Boat, cart and glider walks retry a brief reach failure within the same
  walk instead of aborting, and say plainly when a ride cannot be completed.
  Ride dialogue is now answered by the text of the options rather than by
  position, which fixes sailors whose menu gains a Crandor option during
  Dragon Slayer and Captain Shanks' two-destination menu.
- Map walks can start from enclosed spots such as the Ardougne market shops:
  door crossings now land where the game puts you, so the pocket has an exit
  (fixes the 0.1.9.1 known issue). Starting from a tile that cannot be walked
  on (for example dense bush) now says the player's tile is not standable
  instead of the misleading "no observed player".
- On free-to-play or unrecognised worlds, map walks no longer plan through
  more members-only crossings (the Duel Arena gates, the Shantay pass
  doorway, the Entrana boat, the Falador wall shortcut, Zanaris and the
  Camelot, Ardougne, Watchtower and Trollheim teleports); the walk is refused
  with "This route requires a members' world".
- Map-walk bank fetch (the "allow bank fetch" setting, off by default) now
  also works at banks served by a teller or opened by using an object,
  checks that each bank step actually landed (stopping with a logged reason
  if it never does), and no longer mistakes items that are only in your bank
  for items you carry. Bots remember what each account's bank held the last
  time it was open (saved per account), so a walk can plan its bank trip
  without the bank being open; if that memory turns out to be out of date,
  the walk makes one trip and then plans from what the bank really holds.
- Custom navigation packs (`--nav-pack`) now support Firemaking like bundled
  navigation. A custom pack whose reach or Firemaking data doesn't match its
  manifest is refused when the profile loads instead of being silently
  ignored.
- Walks now route around dangerous monsters and the Temple of Ikov lava
  bridge by default. **Danger routing** in Nav config has three levels:
  Never, When survivable (the default) and Always. Until the bot can track
  poison, When survivable behaves like Never, so a low-level account can be
  refused routes that cross aggressive monsters. The refusal names the
  monsters in the way and says how to allow it. Set Danger routing to Always
  to cross danger zones on every walk; WalkTo in the panel and the TUI can
  also allow it for a single walk, and scripts that have the option (the
  Quester and the Gatherer) can allow it in their own settings.
- Bots approach stairs, ladders, large trees and other big objects (for
  example the Lumbridge Castle staircase) from a side they can actually use,
  instead of trying through a wall.
- Doors: reachable doors open without unnecessary walking, recovery no longer
  closes a door that is already open, and a bot no longer waits for a tree or
  rock that disappears while it walks there.
- Walks can now use ladders and stairs whose landing the game works out from
  where you are standing, which navigation previously didn't know about. One
  example is the ladder down to the Mage Arena bank in the deep Wilderness.
- Walks through the Wilderness webs on the way to the Mage Arena now cut them,
  with a slashing weapon you are wielding or else a knife, and try again when
  a cut fails.
- Walks to "within N tiles" of a place now finish at a spot the destination
  can really be reached from, which fixes walks that failed near walls, for
  example when returning from a bank.
- Moving by hand (a click in the game view, the minimap or a walk option in a
  menu, or a TUI manual step) now stops the bot's automatic walk at once and,
  by default, pauses the script that owns it; the cancelled walk is not
  replayed when you resume. Turn this off with "Pause script on manual
  movement" in Nav config (shared by the panel and the TUI). Pausing a script
  yourself, walking by hand and resuming still keeps the script's walk.
- Boat and cart rides pay their real fares (including the Shilo Village cart).
  A walk that can't afford a ride refuses before boarding and says how many
  coins are missing; with bank fetch on it withdraws only what's missing.
- Bots bank at the nearest bank they can actually walk to, and walk to its
  public counter. This fixes bots at Edgeville, Seers' Village, Ardougne and
  the Fishing Guild that aimed for the staff side and skipped the local bank.
- A walk whose destination is inside a monster area enters that area only to
  arrive, and a walk that starts inside one leaves it without coming back
  through it.
- Walks can use doors that open only for a full disguise, and hidden
  push-walls, such as the Black Knights' Fortress guard doors and secret wall.
  The walk goes through only while the disguise is actually worn.
- Walks follow climbs and other forced moves read from the game content, for
  example the climbing rocks up to Death Plateau while climbing boots are
  worn.
- Boarding a boat steps on from the tile you're actually standing on, which
  fixes walks that stalled at the dock, including the Entrana boats.
- Doors are handled more patiently: re-opens are paced instead of repeated,
  a swing door someone else closes mid-walk is reopened, and walks never step
  into a walled-up doorway.
- Baking a custom navigation pack fails with a reason (for example an
  unrecognised door) instead of silently leaving content out, and the
  command-line bake now matches the bundled pack exactly.

### Profiles and fleet

- The panel gains a Fleet window (button beside WalkTo) for the bots you mark:
  Start, Stop, Assign a script without starting, Assign & restart, Log in/out,
  Walk N to… (one destination, each bot routed from where it stands) and
  Apply the focused bot's settings to the marks. Columns show runtime, time
  since last progress and levels gained. Every marked-rows command, in the
  panel and the TUI, reports each bot once with a short reason (no path, not
  logged in, running a script…), and marks are shared between the two.
- Start all and Start on marked bots (panel and TUI) now start one bot per frame
  instead of all at once; waiting bots show `queued k/n`, and one running
  report counts each bot once as started, queued, skipped or failed.
- If saving a profile fails after you press Save (for example a read-only disk)
  the error now shows in the form you were editing, in the panel and the TUI,
  with "nothing was saved" and your edits kept. While a save is still being
  written the panel also asks before you leave the form (switching profile,
  Cancel, Close, deleting that profile or turning MultiBox off) and brings
  the Profiles tab forward so the prompt is visible.
- In the TUI, starting the marked bots runs the script currently picked in
  Browse, and the confirmation names it.

### WalkTo map

- A map walk that banks on the way spends less time at the bank: opening it,
  withdrawing and closing follow each other without waiting a tick between
  steps.
- Map markers that said "Rare Trees" now show the nearby tree's own name (for
  example Yew, Willow, Maple tree, Magic tree).
- Quest-start markers show the quest's name where the game content identifies
  the start (a bit over half of them on the 289 map); the others stay generic
  rather than risk a wrong name.
- The WalkTo map and the TUI map can shade the Wilderness, with a persisted
  on/off toggle and edges that follow the navigation content tile-for-tile.
- The TUI map's reach layer (`r`) now works. The panel and TUI load reach data
  only when the layer is first shown.
- Opening WalkTo before the vault is unlocked says the map loads once the
  vault is unlocked, instead of saying the navigation pack is missing.
- On Windows the WalkTo map no longer fails now and then while antivirus is
  scanning its files.

### Panel

- On macOS and Windows the panel window fits inside the usable screen area
  (below the menu bar, clear of the Dock/taskbar) at launch and when MultiBox
  widens it; you can still drag it anywhere afterwards.
- Tidier panel: thinner scrollbars, a centred game view, and a Profiles window
  that fits narrow screens without scrolling. Popups no longer balloon or
  clip "Keep editing" when opened from a far-right Edit button. The Log can
  also open as its own dockable tab (the **Log** button), sharing its filters
  with the inline log, the session-log setting shows the file being written,
  and Status hides empty rows. Script status changes, including why a script
  stopped, are written to the log.
- The panel no longer freezes ("Not Responding") at startup on Windows
  laptops with two graphics chips (see Upgrading).
- On Linux (X11) the panel exits cleanly instead of crashing when its window
  is destroyed from outside.
- A local-engine **Debug** tab replaces the old stub: searchable server
  commands, name pickers, favourites and recent commands; send to the focused
  bot or the marked bots, confirm destructive commands, and see skipped bots
  in the tab and nearby server replies in the log. Hidden on public (rs2b2t)
  profiles.
- Panel text now uses the 3270 typeface, and local account status markers show
  as five crisp squares.
- At fractional display scales (for example 125%, 150% or 175% on Windows) the
  panel lays text and controls out on whole pixels, so text is sharper and
  rows line up evenly.
- Input-method (IME) candidate windows now follow the text caret at fractional
  and Retina display scales, for typing in languages such as Chinese, Japanese
  or Korean.
- A click in the game view made while a panel field has the keyboard is kept
  when focus returns to the game. Leaving a panel field or the window still
  releases keys and mouse buttons you were holding, and no longer clears keys
  the bot is holding on its own.

### TUI

- In the TUI browser, a card whose import cannot be resolved now shows the
  failing import instead of only dimming. `tui-play --help` prints usage and
  exits 0.
- Option settings in the TUI: Space steps through the choices and Enter opens a
  searchable list.

### Scripts

- Bank and shop actions in scripts finish as soon as the game confirms them,
  instead of waiting for the next game tick.
- Food choices in script settings now list every food the game heals with,
  best heal first, instead of a fixed list of 25.
- Starting a script waits if a settings change is still being saved, including
  when you edit those settings again before the first save finishes. The start
  is not reported as a failure while it waits, and it does not run on the
  older settings.
- Starting a script whose file has been deleted now says the file is missing,
  instead of running an older cached copy (seen on Linux).
- A native script that can't continue (for example, out of a required supply
  or unable to walk back to its spot) now stops and keeps the reason, shown as
  "stopped (blocked)" in the panel and the TUI.
- Catalog scripts in JavaScript or TypeScript get checked type declarations
  for the rs2b0t script API, so editors and the compiler catch wrong
  arguments.
- Scripts can use `Area` from the rs2b0t API (rectangular and circular
  areas with `contains` and `getRandomTile`), whether imported from
  `@rs2b0t/api` or from the geometry module. It used to stop the script with
  a `not impl:` error.
- Load scripts can start, watch and stop the Gatherer, and read quest
  progress (which quests exist, their current stage and whether they're
  done). A bundled example gathers and then checks progress.
- Load scripts can run the bot's own fighter with `api.combat.fight(...)`:
  melee, ranged or magic against chosen monsters or whatever is attacking
  you, with the bot's eating, potion and prayer rules. `api.snapshot.combat`
  shows the fight and `api.combat.stop()` ends it. Pausing, reconnecting or
  moving the player yourself ends the fight, even before it has started. The
  bot turns off only prayers it turned on; turning on a protection prayer can
  switch off a different protection prayer you had on, and that one isn't
  restored. A bundled example shows every fight type.
- In a Load script, moving the player yourself now also ends a Gatherer
  session that's gathering in place (not only while it walks), with a
  `manual-movement` result. "Pause script on manual movement" applies to
  Gatherer and combat sessions too; turning it off skips the pause, not the
  stop.
- Script gathering lookups page through long lists and report game data that
  is incomplete, instead of returning partial results silently.
- A script setting that names a saved loadout refuses an unknown or ambiguous
  name with a clear error, instead of starting the script without the gear.

### Audio

- Fixed a brief audio stutter when the first background song starts.

### Performance

- Navigation overlays (collision, path, trail) are much cheaper to leave
  switched on with the GPU renderer, and with the software renderer, where
  they are not drawn, turning them on no longer costs extra memory or CPU.
- Bots use less memory: logged-in bots free their login-screen images (the
  animated login screen still returns after logout), and the built-in game data
  is stored compressed and unpacked once when the bot host starts, which adds
  about 20 ms to launch.
- Faster startup: after the first launch, the client reuses the game assets
  it already downloaded and checked, so later launches reach the game in
  seconds instead of downloading everything again.

### Not in this release

Things that were planned for 0.2.0, or that you might reasonably expect, but
that aren't here yet. Where we know the target, it's noted.

**Quester**

- Guides ship for 25 of the game's 69 quests. The other 44 don't have one
  yet: guides for them are planned for 0.2.0.1. Haunted Mine can't be run on
  this server and stays listed as unavailable.
- Apart from the five drafts, the guides are still beta. Most have been run
  at least part of the way, and a few all the way through, but any of them
  may stop partway. When that happens, the panel shows the step it was on.
- A quest that's waiting or stopped can look idle in the Status card: the
  reason is shown in the script section, not in Status yet.

**Combat**

- Fights hold protection prayers; prayer flicking isn't supported yet.
- Fights are fought in the open: safespots, luring into corners and
  face-tanking fallbacks aren't supported yet, and the bot doesn't pick a
  standing spot by your weapon's attack range (planned for 0.2.1, with an
  attack-range debug overlay).
- The combat machine doesn't attack players. It does defend itself against
  players who attack it.
- Hunt fights give up after two minutes, and a retreat can end next to a
  different monster that attacks.

**Gatherer**

- There's no "closest location" option: choose a named place, your start
  tile, a custom area or Auto.
- Tools aren't bought from shops. If neither you nor your bank has a usable
  tool, gathering stops and names the tool it needs.
- Some spots behind area unlocks (for example Miscellania) aren't offered.

**Clues**

- Some trail clue types don't complete yet, including key-keeper clues
  without a key drop, locked chests, and clues in Kharazi and Tirannwn.

**Navigation**

- Accounts below combat level 51 have no land route past White Wolf
  Mountain; take a boat instead.
- WalkTo's list of town destinations is a fixed list.

**Scripts**

- Some rs2b0t script API members still stop the script with a `not impl:`
  error, among them `Game.castOnNpc`, `Shop.buyById`,
  `ChatDialog.makeOne`, `Traversal.remaining`/`requestRepath`,
  `EntityQuery.inside`/`nearestPreferLocal`, script events
  (`events.on`/`off`, `registerScript`), the `InvItem` class and task-tree
  classes, and several gathering, fishing and banking data helpers. Closing
  these, checked against rs2b0t's own tests, is planned for 0.2.0.1 and 0.2.1.
- Script errors don't yet tell "not possible right now" apart from "not
  implemented".
- Equipping an item while the bank is open isn't handled reliably; close
  the bank first.
- Some catalog scripts stay greyed out until they've been qualified,
  including FlourCollector, the Zanaris cook and most Jive scripts other
  than King Black Dragon trips.

**Panel and TUI**

- The script window doesn't resize with its contents.
- Profiles can't be put in a custom order.
- The TUI shows the map and fleet, not actual game pixels (planned for
  0.2.2).

**Game versions**

- Revision 289 is the supported version. Revision 274 may work but isn't
  tested.

### Fixed during development

Fixes to problems that appeared and were fixed during 0.2.0 development.
They are listed here for completeness and are left out of the release notes.

- Quester: shorter pauses between actions, after opening a door, while
  reading the quest journal and when collecting supplies from the bank.
- Gatherer: work resumes sooner after a bank trip and after walking back to
  the resources, and an empty withdrawal no longer costs a pause.
- Quester and Gatherer: walks finish as soon as the server says the player has
  arrived, instead of waiting for the character on screen to catch up.
- Quester: a quest's carried supplies keep their exact item when two items
  share a name (Gertrude's Cat could try to take the wrong kind of coins).
- Combat: the per-attacker protection prayers treated several monsters as
  switching attack style when you prayed against them (for example the
  Zamorak wizard, which now counts as magic).
- Dialogue: scripts using the classic dialogue helpers could press Continue
  twice on the same page.
- Navigation: a refusal that names the danger zones in the way could name
  the wrong zones when only one destination was blocked.
- Panel: the Debug tab's name pickers could spill outside their window, and
  the tab marked some harmless commands as destructive.
- Panel: a script-setting list with two options of the same name (for
  example two fishing methods for one fish) selected the wrong one.
- TUI: marks could follow a bot's name instead of the bot.
- Panel: the Log tab could open once on its own after an upgrade between
  development builds.
- Gatherer: a saved site from an older setup could be picked even though it
  no longer fit the game data; it now stays visible with its reason and
  can't be started.
- Combat: after a death had already cleared every prayer, the bot could try
  to turn off prayers it had raised before the death.
- Quester: walking to doors and gates from the wrong side, which stalled
  Death Plateau at Tenzing's door and Priest in Peril at the crypt gate, and
  made the temple door bounce the player in and out.
- Quester: pauses of up to 20 seconds after opening a trapdoor, climbing
  down, or fighting, and an extra tick between every step.
- Quester: a quest journal that opened slowly, or a "click to continue" left
  on screen by a cutscene or area trigger, could stop a quest (Death Plateau
  at Dunstan and the secret way). The Quester now clicks through such a page
  itself.
- Quester: boat and customs trips counted as failed conversations (Pirate's
  Treasure); The Tourist Trap could loop before it started; Priest in Peril
  tried the wrong monuments first; The Knight's Sword could run Wydin's shop
  out of redberries after burnt pies.
- Quester: steps that use a banked item if there is one, or else get it, and
  steps allowed to take only some of their items (Imp Catcher's beads), stopped
  once the bank was known not to hold everything.
- Quester: a burnt pie stopped The Knight's Sword instead of baking another;
  quest steps that make an item now try again (a few times at most) when a
  chance step fails. Priest in Peril needed coins it never fetched, waited in
  the crypt after the coffin, couldn't get back through the first gate, and
  could try to douse the coffin through the cell bars; The Tourist Trap was
  refused on the walk to the mining camp captain.
- Gatherer: after banking at Draynor it could pick a spot inside the bank to
  walk back to, lose sight of the fishing spot and give up; at Catherby and
  some other fishing spots it could pick a spot it can't stand on.
- Navigation: the White Wolf Mountain danger zone was larger than the
  wolves' real reach, so Lumbridge to Ardougne by land was refused even for
  high-level characters; it now routes for combat 51 and above. Ghasts in
  Mort Myre are tracked by how they really move.
- Vault: pasting a passphrase of about 1,000 characters or more at the Linux
  terminal prompt waited for another key press; saving the vault on Windows
  could fail while antivirus scanned the file.
- Startup: on Windows a reused copy of the downloaded game files could be
  cleaned up first because its last-used time didn't update.
- Quester: a guide step allowed to cross a guarded area could still be
  refused at the end of its walk inside that area, which stopped The Knight's
  Sword in the Asgarnian Ice Dungeon.
- Quester: The Knight's Sword searched Sir Vyvin's cupboard while he stood
  next to the player, which the game refuses, and kept retrying until it
  stopped. It now waits for him to step away first.

## [0.1.9.1] — 2026-09-28 — Alpha 4 patch

### Sessions

- Hosted sessions no longer trigger the embedded client's inactivity logout.
  The client sends a Java-style keepalive after about a second without an
  outbound packet, without flooding fast-pumped slots.
- A running or paused script keeps its in-flight work across unexpected
  disconnects (server logout, EOF, WebSocket/TLS/protocol errors, dead server)
  and relogs through the normal login queue. Three unexpected exits within ten
  awake minutes park relogging until an explicit Log in.
- A deliberate Logout, Stop or Remove stays final, including a Logout while
  already parked. Cancelling a Logout that never took effect in game no longer
  discards the work at the next unexpected drop.
- Exit, relog, queue, backoff and session-boundary decisions are logged with
  typed reasons.

### Navigation

- WalkTo and script walks now use the members gates on rs2b2t's public worlds.
  With the default public profile, long routes such as Lumbridge to Ardougne
  were refused with "No path to the selected destination" because membership
  resolved as unknown. An explicit `--world-members` setting still wins, and
  local profiles keep both members and free-to-play operation.
- Every panel and TUI map walk request logs one line with its origin,
  destination, routing options, world membership and the outcome (route
  summary or refusal reason).
- Boats, cart drivers and other NPC transports: the bot walks up to the NPC
  where it actually is before talking to it. A wandering sailor or customs
  officer no longer causes "I can't reach that!" and an aborted walk from
  a stale position.
- Every panel and TUI map walk also logs how it ended: arrived, aborted
  (with the reason and the transport it stopped at) or cancelled.
- A map walk that fetches items from the bank now continues to its
  destination after banking, instead of stopping at the bank.

### Profiles and fleet

- The Profiles editor can no longer overwrite another profile. An open edit
  form stays on the profile it was opened for, shows which one ("Editing
  …"), and asks before discarding unsaved changes when you open another
  profile. Renaming onto an existing username, or creating a profile with
  one, is refused inside the Profiles window with "nothing was saved", and a
  successful save shows "Saved …" there. The TUI settings popup behaves
  the same way.
- In single-bot mode, picking a profile in the Profiles window keeps the
  window open (close it with the tab's ×), as MultiBox already did.
- Login all right after Load all reaches a member whose previous worker was
  still stopping, instead of leaving it parked logged out.
- A Remove followed quickly by a re-add now settles, instead of leaving a
  pending operation that never completes.

### Scripts

- Browse lists the compiled Sherlock script again, and selecting it starts it.
- A restored script assignment opens Script prefs with its parameters, not
  "(no parameters)" when the catalog was not yet read. A card whose catalog
  is unavailable is reported as such.
- Browser timer APIs (`setTimeout` and similar) now fail with a catchable
  error in isolate scripts instead of crashing the isolate. Use
  `Execution.delayTicks` for game-time waits.

### WalkTo map

- Place names stored with `/` as a line break (for example `Port/Sarim`)
  display with a space in the picker, canvas labels and TUI search. Search
  still treats `/` as a word break.
- Canvas labels are ranked by importance, then kind, then distance from the
  view centre. Labels whose text would overlap an already placed label are
  omitted (markers still draw).

### Audio and title (client)

- Switching from lowmem to highmem in game resumes the current zone song,
  only when music is on and no jingle is playing, instead of always starting
  the title music. Switching to lowmem stops music and sound effects cleanly.
- Sound effects follow the 289 client: a new effect replaces the current one
  only if it would end later; otherwise it is dropped. They no longer queue
  behind each other, so a hit sound no longer waits for a stun sound to
  finish.
- The sound-effect volume setting is now applied (0, −4, −8, −12 dB).
- Title-screen flames animate at the 289 client's steady-state cadence
  (about 37 ms per frame).

### Known issues

- After an in-game logout, the title screen in the panel stays frozen and
  ignores input. Log in from the panel as usual. A live title with a
  single "Log In" button is planned for 0.2.0.
- A map walk started from a few spots in Ardougne market can fail with
  "No path to the selected destination", because a door there that starts
  open isn't known to navigation yet. Step a tile or two away and try
  again.

## [0.1.9] — 2026-09-27 — Alpha 4

### Release engineering

- Promote the reproducible macOS/Linux/Windows build, finalize, and verification
  workflow into `tools/release/release.py`: exact host/client source exports,
  digest-pinned navigation inputs, clean-checkout gates, per-commit native
  caches, resumable preparation/platform builds and macOS notarization,
  archive/manifest/checksum validation, and explicit post-release cleanup.
  Linux uses the parameterized builder SSH transport, Windows uses the
  checked-in `windows-ssh.py`, and no operator filesystem paths or credentials
  are tracked.

### Memory

- Raw collision/NSEW navflags load only when a drawing surface needs them, stream
  into one resident `Arc<Vec<u32>>` (no full-file byte buffer retained beside the
  decoded words; peak owned payload ≈ one copy of the sidecar), skip the 260 MB
  content hash for trusted bundled flags, and release when the last drawer stops
  or the only focused slot is removed. Bundled reach binding clears on
  session/pack detach; flood cache keys use Weak/`Arc` world identity and empty
  demand releases ownership (WalkTo close still uses `release_map_leases`).

### Scenario qualification

- Cumulative catalog XP proofs now arm immediately before script Start,
  excluding seeded XP without losing real gains that arrive before a later
  sequential watch. Fresh bank-return XP proofs remain step-local.
- ArdyCakes FightBack waits for an idle nearby Guard with line of sight and
  prepares melee stats at 70 before Start. It tolerates one catch-less
  stall session through a bank return and renewed Guard readiness, within
  a 360-second deadline; a second unqualified bank visit fails. Real
  combat XP, Thieving XP and Cake requirements remain mandatory.
- The Taverley jail v2 scenarios wait for the acquired jail/dusty key before
  starting the clean-stop grace. Headless qualification no longer times out
  during the dungeon walk; navigation and the scenario budgets are unchanged.
- FlaxAIO's spin seed lands cardinally beside an open Seers booth, not on
  the catalog's diagonal bank anchor. Exact-booth seed opens now send once
  and await the bank acknowledgement, preventing queued reopens from
  invalidating Herblore and shop seed deposits.
- `tui-play --live script_<name> --catalog-core` (or `--pair-core`;
  `BOT_LIVE_CORE=catalog|pair` is the environment form) qualifies a run
  under the same shared core witness as the panel's `catalog_watch` /
  `pair_watch`: the witness is armed before the slots publish, frozen at the
  actual isolate Start, and a scenario PASS is held while it is Pending,
  before the unchanged 45-second clean-stop grace starts. The gate lives in
  `host_play::live_gate` / `live_start`, and both front ends call it.
  `CATALOG_CORE:` / `PAIRED_CORE:` receipts ride with the terminal line.
- The paired proofs (`nature_crafter_air`, `mule_crafter_air`,
  `flax_runner`, `duel_arena`) refuse to run without the pair gate instead
  of reporting their Start snapshot as PASS, in panel-play and tui-play.
- Duel trainer pair preparation accepts the Train schema's inert empty helper
  partner field while retaining native counterpart identity; helper mode and
  populated partner settings still fail closed. Duel Arena Combat Trainer
  ships qualified in 0.1.9: its `Duel.*` API is native, and two consecutive
  pair-gated `duel_arena` runs on one build passed the full cycle. In each,
  both actors challenged, accepted the offer and confirm screens, fought in
  a pen for script-caused melee XP, finished the duel and went into a second
  one. The support matrix records the trainer as QUALIFIED with those
  receipts.
- FlaxRunner's refrozen full-cycle row is a catalog-defect disposition: meet
  arrival radius 2 can leave both actors arrived but outside trade range 2.
  The support matrix preserves the first-transfer/spin/bank evidence without
  claiming a second delivery. No qualification-only catalog patch is shipped;
  users continue to select their own `rs2b0t-path` source.
- Eleven v2 File-card cells no longer pass on their Start snapshot. Each
  watch and terminal proof is now the card's own outcome, which the
  pre-Start seed cannot satisfy, so the clean-stop grace starts after the
  card's work, not at Start. Hold/retreat/walk-spot and enter/leave-lair
  wait for the card's walk to its distance band from the mainland landing
  (new `arrived_ring` proof). `bank_v2_ts` waits for arrival within 3 of
  the Falador bank. The prayer v1/v2 cells wait for Protect from Melee on
  (varp 97), not for the seeded prayer 43. The observing line-of-sight,
  actor-observation and fight-field cells wait for the receipt row the card
  paints (new `script_receipt` proof, fed from the driven slot's published
  paint in both front ends and recorded in the evidence `receipt` field).

### Distribution

- The repository now carries one parameterized release controller for pinned
  host/client exports, native macOS/Linux/Windows builds, package finalization,
  notarization, extracted-package verification, and cross-platform navigation
  identity. Builder overrides, paths, SSH material, signing identity, and the
  Rusty V8 archive remain operator inputs rather than tracked machine values.

### Performance measurement

- The opt-in `memory-profile` harness now accepts 50-slot runs, representative
  catalog scenarios, custom teardown windows, panel-frame histograms, startup
  readiness, process CPU, and feature-gated host loop/observe/raster counters.
  A find-only 289 navigation corpus reports cold and warm searches plus peak
  scratch capacity. Normal builds do not compile the host timing registry.
- Multi-bot MossGiant baselines replace only the contended post-return fresh-XP
  checkpoint with each local player's fail-closed named-NPC engagement. Every
  fleet member must still pass the scenario's final Strength-XP-since-Start
  proof; the one-bot path retains its ordinary fresh-XP checkpoint as well.

### Navigation

- A bank walk after smithing can recover when the final crafting delay drops
  its first click and a queued level-up blocks the retry. The follower reissues
  its current aim after five stationary game ticks even if the map flag remains,
  without extending the original hop budget or changing the route.
- Radius walks to a solid in-scene target now route to one of its wall-valid
  cardinal / interaction stands with one first-goal baked search, so doors
  and transports remain usable. The same search serves the old radius goal
  set as a fallback: a stand wins whenever one routes, and a radius tile it
  passes on the way needs no second search. A backward search from the
  stands runs beside it: stands sealed in a small region (a dead-end pocket,
  a gated room) are proven unreachable after about twice that region's size.
  A search that neither reaches a stand nor proves one reachable stops after
  524,288 nodes instead of flooding the map; a proven-reachable stand keeps
  the full 4,000,000-node search. The radius tiles keep that budget rule
  for themselves inside the shared search, so it answers for them exactly
  as a search over the tiles alone would; where the stands stop it before
  that answer, the tiles get a search of their own. BankBudget then searches
  once more with every obj the bank and backpack hold treated as carried and
  wearable (their counts combined, capped at a full stack), so a stand
  behind an obj the bank lacks no longer hides one the bank can open. A walk
  spends at most two node-capped searches, plus one each time a planned bank
  trip would deposit an item the route still needs.
  A low-level nearest-route terminal still settles its owned wait, but
  `walkResilient` continues its frozen scene-recovery ladder until the reach
  probe confirms arrival. Every scene/direct `walk-to` request uses the
  client's nearest-tile packet; resilient recovery also keeps the frozen
  48-tile clamp and stalled/periodic re-click. After that scene step,
  `walkResilient` now runs the frozen unstick (`tryNearbyDoor` within
  Chebyshev 3, then one stalled/periodically re-clicked `pickUnstickStep`)
  so a closed live door that the pack never encoded (shape-9 `loc_1530` at
  2669,3316) can still be opened. The one-tile step always runs after the
  door attempt, even when no door helps; it returns on arrival, and progress
  resets the pass before rebaking. Desert Mining Camp's scripted doors stay
  excluded as in frozen.
  `walkOpening` is a Rust machine (eight segments, openable obstacle within
  14, 4000 ms wait) instead of a `walkResilient` reduction. Unstick and
  `walkOpening` clear each candidate's real wall edges on both tiles and open
  only a door whose counterfactual makes the destination reachable under the
  walk arrival rule (or strictly closer) on the posted collision flood. Ties
  use route length, then proximity, rather than BFS enqueue order. Frozen
  path-scoped hints apply only when a published route exists. The native wait
  intentionally does not dismiss the
  frozen quest-lock mesbox, so a locked door costs the full five-second bound.
- The 289 nav bake now has an edge for every engine-openable generic door,
  not only straight level-0 ones (274V10 unchanged). A closed diagonal
  (shape-9) `door_closed` door crosses its own tile to the free cardinal on
  each side, never onto the tile its opened leaf swings to; this is the
  East Ardougne house door (2669,3316) that sealed the 0.1.8.1 Thiever in
  (`NoPath` to the south bank before, a door crossing now). Doors on upper
  floors cross on their plane, and every loc placement (ladders included)
  now sits on the engine's bridge-corrected game plane. Closed doors of a
  generic door category declared outside the door configs (West Ardougne
  `loc_2997`, Rellekka, Troll Stronghold, games room), the Al Kharid
  curtains and Rellekka fur doors (in-place `loc_change` to a non-blocking
  loc), the Paterdomus members fence gate (`members_req`), the Cooking,
  Crafting and Fishing guild doors (their own skill-level and worn
  hat/apron checks on the way in, free on the way out), the West Ardougne
  fence climb and the Miscellania castle stairs are now edges. 2,240 →
  2,758 edges (+510 door, +8 stairs), pack +34,776 bytes; bakes stay
  byte-identical.
- Quest-stage and guild doors and guarded ladders are now 289 edges (274V10
  unchanged). A loc no other producer covers whose own `[oploc1]` opener (or
  its category's) walks the player through one of the engine's
  open-and-close door procs, or climbs `~climb_ladder`, gets an edge per
  crossing the opener provably makes: it is evaluated per placement and
  side with the player past every threshold, and each check on that path
  becomes the minimum the edge requires (quest progress as the completed
  journal row, or the raw varp when 289 transmits it; skill levels; carried
  or worn items; members). Talk, NPC lookups, key use, state writes and
  movement after the crossing keep an opener out. New: the Heroes' and
  Legends' Guild doors, the Mining Guild door and surface ladder (mining
  60), Witch's House, Priest in Peril temple, Nature Spirit, Mourner HQ,
  Troll Stronghold arena and Tai Bwo Wannai doors, one-way exits (Khazard's
  stronghold, jail and cell doors) and quest ladders. An opener is only
  trusted while the engine door procs it calls match their pinned 289
  bodies and every call after the crossing (in arguments, arithmetic,
  string interpolations or called procs) is a command certified not to
  move the player. 2,758 → 2,941 edges (+152 door, +31 ladder), pack
  +13,210 bytes, placements with an edge 1,744 → 1,882; bakes stay
  byte-identical. Openers gated on an in-progress
  quest window, shared or untransmitted flags, NPC dialogs, keys used on the
  loc and player-relative or randomised landings are still not edges.
- Added shared native-map data contracts: independently keyed image/POI caches,
  checked manifests and data-only service records, resumable partial-entry
  validation, bounded map-record reads and 24-texture LOD selection. Raw visual
  bridge-plane conversion now shares collision's implementation; server NPC
  coordinates are not shifted. Frozen catalog-facing bank selection is unchanged.
- Nav bake now emits a data-only `274bot.navpois` sidecar (`274P`/1) with
  revision NPC placements, bounded `@openbank` / `loc_change` bank evidence,
  and place labels. Missing navpois invalidates a warm stamp once. Frozen
  `BANK_CATALOG` / named-bank APIs are unchanged; 274V10 routing bytes are
  unchanged.
- Replaced the WalkTo per-tile collision-dot mesh with an application-owned
  native map renderer: at most 24 terrain textures (none until the local map
  image cache is bound — the map shows a grid and `map imagery unavailable —
  cache not bound`, with no POIs), one viewport overlay for optional map-owned
  grid/collision/NSEW/reach/flood toggles, vector route and destination
  markers in the default view, radius-16 snap, wheel-zoom toward the cursor,
  and GPU/CPU pixel release on close. Reach uses a bound `.navreach` sidecar
  or shows `reach unavailable`; the map never runs a whole-world BFS.
  Basemap defaults on; grid dots default off. `BOT_CPU=1` uses the same map
  path. Header plane/zoom/search and layer toggles wrap inside the Game pane
  at the default window and at narrower widths; the footer reserves the one
  status/action row that is drawn.
- Added a shared host map catalogue and search API that merges revision-bound
  client POIs, authenticated `navpois` service/place facts and borrowed navigation
  transports. Access anchors, annotations, adjacent walk stands and teleport
  **landings** remain different facts. Missing client data or `navpois` is
  explicit; tellers and place labels are unavailable without the supplement.
- The shared selection model provides a radius-16 query (at most 33×33 cells),
  ordered by Chebyshev distance, Manhattan distance, x, then z. A miss has no
  walk target; debug Teleport still uses the requested tile. Place-label
  anchors remain view jumps unless they have a proven stand.
- Shared map confirmations consume the selection once. The destination binds
  tile/POI, plane and nav identity, not a bot. Walk options are taken at
  confirmation. Panel adapters walk through `Play::map_walk`, refuse a missing
  observed player, and send debug Teleport only through `Play::map_teleport`.
  Walk needs the snapped target. Debug Teleport is a cheat: it uses the
  requested tile and does not require walkability or radius-16 snap. It is
  authorized only for a Local target on a loopback host; the panel and host
  share that predicate, and an unsnapped selection is labelled teleport-only
  only when Teleport is available. Walk and Teleport act on the bot focused
  at confirm. A different nav pack still invalidates the pending destination.
- WalkTo Send walks the focused bot (default) or a Group checklist of wall
  bots (All eligible / None). Ineligible bots stay listed and greyed with a
  host reason: not logged in, no position yet, or running a script (stop the
  script to include it). Confirm reads Walk N bots; each eligible bot gets
  its own command, origin and routing options, then a Start-all-style
  summary (`4 walking, 1 no path: bot3`). Debug Teleport stays focused-only.
- TUI Map (F4) uses that shared host model: destination-only selection,
  radius-16 Walk snap, debug Teleport on any selected tile (Local+loopback
  only), fleet group Walk with eligibility reasons and a Walk N bots
  summary, and focused-bot observed NPC services in the POI list. Enter
  confirms only while Map is focused; plane change and recenter clear the
  pending destination. Catalogue demand is catalogue-only (no PNGs) and is
  released on close.
- The shared route projection borrows actual routes with driven-live, script,
  then manual precedence, matching the in-game overlay. Its generation changes
  across arm replacement, including a new path to the same destination.
- Map catalogue entries expose **Map-discovered bank** provenance, distinct from
  the **Frozen bank API roster**. Canifis can be discovered at normalized bridge
  plane 0 while `nearestBank` still omits it; frozen bank APIs are unchanged.
  A geometric, collision-valid adjacent stand proves a walking destination,
  not banking eligibility or the permitted interaction side (v1 POIs omit
  `forceapproach`). Live NPC service extraction borrows the focused snapshot,
  validates slot/world/nav context and returns at most 128 observed records.
- WalkTo now opens a process-wide images demand (catalogue first) against
  the bound profile's map cache. Real bake progress is shown; ReadyImages
  terrain and `Catalogue::from_ready` (navpois + game-data names) replace
  the fixture-only map. Observed services follow the focused bot. Close
  drops the demand and GPU/CPU pixels. A mid-bake close keeps the tiles already
  written; reopen fully decodes each of them (chunk CRCs, zlib, size), adopts
  the intact ones and bakes the rest, so a damaged tile is never published.
  Route tiles are cached by `(source, generation)` and trimmed by a monotonic
  start index (`remaining_path_tiles` leg semantics; an off-route `here` does
  not re-show walked legs). Pending selection and the armed dest draw as
  distinct markers; decode staging reports its real byte count; LOD selection
  uses the bake/manifest `max_lod`.
- A cold WalkTo terrain bake no longer writes and fsyncs every tile a second
  time when it publishes: the finished tiles' receipts are checkpointed as
  written, so the 289 cold bake takes about 15 s instead of about 24 s, and a
  resumed bake no longer spends ~9 s re-publishing. The panel no longer
  creates the map-cache manager at boot; the first WalkTo open does, and a
  closed map's finished job is dropped on reap. The terrain bake policy
  changed, so an existing terrain cache is baked again once.
- WalkTo map-symbol places (anvils, furnaces, shops, water sources, …) are
  named by their world map Key entry, as in rs2b0t's world map and picker,
  instead of `Location ####`. Minigame symbols read "Minigames". A nameless
  place that has no Key entry is left out: the 289 agility-training (4) and
  vegetable-store (2) symbols, which rs2b0t's picker does not show either.
  On 289 the catalogue drops from 745 to 739 places, and none is labelled
  `Location`. The catalogue policy changed, so an existing catalogue is
  baked again once.
- Before a local WalkTo terrain bake starts (no ready terrain for the bound
  client cache's image identity), the panel's WalkTo map warns what it costs
  (CPU for about 15 s, up to ~15 MiB once) and offers **Bake now**, **Always
  bake** or **Not now**. Not now keeps the map catalogue-only (POIs, search,
  grid) with a **Bake terrain** control for later. A ready terrain cache,
  whether baked earlier or installed, opens without asking (a missing or
  stale catalogue is derived silently); catalogue-only
  demand (the TUI map) never asks. The remembered choice is the `map_bake`
  key of `panel-ui.json` (`ask` when absent, as in 0.1.8.1 files), edited in
  the panel's Nav config (**ask before baking terrain**) and the TUI settings
  popup (**map bake**). The decision is `frontend_core::MapBakeGate`, shared
  by both front ends.
- Release packages (macOS, Windows and Linux) ship the 289 WalkTo map
  terrain, as rs2b0t ships its map images (operator decision 2026-09-26).
  `tools/release/package.py` bakes it
  with the staged `tui-play --map-bundle` from the pinned client cache the nav
  bundle was built from, through the same map-cache producer and publication
  as a local bake, and ships it as `map/289/` beside `nav/289/` (and in the
  macOS bundle's `Contents/Resources`), with `274bot.mapimages.json` recording
  the image identity (revision, decoded client content, bake policy), the
  image manifest's receipt and the tile totals; `package.py` refuses terrain
  baked from another cache than the nav bundle, re-verifies every tile and
  records it in `release-manifest.json` (`--check` stages, verifies and
  discards a package). The first WalkTo open installs the shipped terrain
  into `~/.274bot/map-cache` without a bake question, but only for exactly
  the bound client cache's image identity: each tile's length and SHA-256 is
  verified against the pinned manifest as it is copied, and a published
  terrain is never replaced. Shipped terrain for another client cache is
  left unused, and damaged or partial shipped files are never installed:
  the local bake stays behind the existing warning and, once accepted,
  adopts the tiles already verified.

### Paired catalog compatibility

- ClueSolver and Duel Arena Combat Trainer run through the native host, and
  the trainer is qualified in 0.1.9 (see Scenario qualification).
  `fightArenaAt` answers with the frozen `DUEL_FIGHT_ARENAS` entry, so the
  trainer's pen identity checks hold and it fights once inside a pen.
- JiveKQ four-player is present but not enabled or qualified in 0.1.9: the
  catalog lists it dim ("four-player qualification incomplete in 0.1.9") and
  refuses its Start. Its BroadcastChannel broker, max-stat fixture and
  `script_jive_kq_four` witness stay in place for Beta 1. The broker fences
  each four-account roster to one world and one script run. A connection
  boundary the card's isolate survives (a drop, a relog or a logout)
  suspends the member instead of revoking it; the next session re-admits it
  only under the same run and world, and a Stop, a new run, a world change or
  a slot that stops revokes it. Deliveries the broker already made reach the
  script after a relog. A stale sender hears each refusal once per run, a
  slot with no channel open reads one flag, never the broker lock, and the
  broker keeps nothing for a profile once it leaves.
  KQ witness 14's leader had no ropes left for a third descent after its
  second group retreated early, so the fixture now banks eight for it.
- Clue 3554 crosses the Duel Arena through a configured, different helper
  account before digging. Each profile now stores that account-wide partner in
  the profile editor, separately from per-card parameters. Start, a parameter
  edit, Apply to all and a profile save each post the running script its own
  profile's partner with the card bag, so it is read live and copying
  parameters from another member never clears it. As in the frozen client,
  the partner is needed only to enter a pen from outside (a missing or self
  partner logs the frozen operator action and refuses); the lobby and
  destination legs are resilient walks; a wrong obstacle arena is forfeited
  and left before one retry, and before failing; an open duel screen is
  cancelled on the 60 s handshake timeout or an event signal; and the
  frozen log lines reach the card's log. `Duel.partner()` keeps the display
  name, and `Duel.cancel()` awaits the closed screen.
- Ordinary ClueSolver bank returns keep the six-attempt, five-minute
  resilient walk instead of entering the duel travel machine, with the
  teleport catalog unless the card's `ClueExecutor.setTeleports(false)` turned
  it off.
- One Play now brokers bounded same-world `BroadcastChannel` traffic between
  catalog isolates. The `clue_duel_3554` two-account and `jive_kq_four`
  four-account live cells mint disposable local profiles, launch their
  heterogeneous/shared catalog fleets through the one
  `host_play::live_start` fleet launch panel-play and tui-play both call,
  and collect per-slot evidence. Clue acceptance covers both names/screens,
  exact obstacle rules, pen entry, casket/forfeit/helper reset, unchanged HP
  and a new reward in a fresh bank.
  KQ max-stat fixture acceptance delays the roster leader while the other
  three members prove their missing-peer bank hold, then covers both pass
  withdrawal and purchase paths, exact role packs, scene-backed rope use,
  every member's protected cardinal two-form combat/XP and post-food recovery,
  a continuous flying-form death, selected natural non-arrow
  ground-to-inventory-to-bank loot, overlapping personal restock, Camelot and
  dueling-ring escapes, a second four-member descent and a second safe bank
  return.
- Chat-option snapshots retain each `BUTTON_OK` component identity across the
  host FlatBuffer, so direct catalog clicks such as a dueling-ring destination
  no longer degrade to component `-1`.
- Selected 274/289 game data now includes the explicit
  `ai_queue3:kalphite_flyingqueen` death-drop block. It is valid without a
  fabricated `death_drop` parameter; generated `DROP_DB["Kalphite Queen"]`
  therefore drives the unchanged catalog collector.
- `Shop.open` now keeps its native await pending across the modal/stock-page
  publication boundary, so an immediate `Shop.buy` cannot mistake a transient
  empty page for zero purchases and dissolve a JiveKQ party during restock.

### Rendering and client

- Logged-out title-screen brazier flames animate again on CPU and GPU. Full-rate
  views follow the 35 ms flame clock, 1 fps rail tiles catch up when painted,
  and draw-off bot slots remain raster-free.
- Region changes reuse one identity-checked local map store instead of fetching
  every square over ondemand; completed maps are kept under that content
  identity. The 289 ondemand worker no longer sends the legacy Java keepalive
  (`00 00 00 0a`) that this engine family closes on; 274 still does. A closed
  update socket reconnects and resends immediately after a network completion,
  while accept-then-close reconnects back off instead of spinning. A closed
  game socket is noticed on the next frame via EOF peek instead of waiting the
  15 s silence watchdog.

### Startup

- Faster startup: fewer redundant cache and content re-checks. Local binds no
  longer re-hash the whole content tree, login CRCs share the cache capture
  pass, and Play construction does not recapture archives already checked at
  template load.


### Panel and TUI

- **Shared structured log.** Panel and TUI read one log from `frontend-core`:
  each line carries wall time (`HH:MM:SS.mmm`), game tick, slot, source
  (script/login/nav/bank/watchdog/host) and level. Host code logs through one
  `host_log!` facade: login phases, handshakes, script lifecycle, watchdog,
  random events, route-level nav events and bank operations always reach the
  slot log, also in release builds; per-frame and per-tick traces stay on
  stderr under `BOT_DEBUG=1` / `--debug` as before. Each bot keeps its newest
  500 lines plus a 500-line process ring (measured ≈ 61 KiB per bot for
  typical lines, ≤ 282 KiB at the 512-byte line cap).
  Every account password is redacted in the facade itself, before a line
  reaches stderr, the log, Copy, Save or the session file. Lines recorded
  off the slot thread (status transitions, script lines) carry the slot's
  current game tick.
  The panel log section fills the leftover side-panel height (or is a
  resizable box when other sections follow it) with a timestamp column, level
  colours, level/source/scope filters, search, follow, **Copy** and **Save
  log…**; the TUI shows the same view on its Logs tab (**F7**, 80×24 included) and its newest rows in the log drawer. Status
  transitions, script lines, audio and vault/profile errors moved onto it, and
  the TUI now drains script lines too instead of letting them pile up.
- **Session log file** (off by default): the panel's General config
  checkbox or `F` in the TUI log pane sets `session_log_file` in
  `~/.274bot/panel-ui.json` (absent = off). While on, a background thread
  writes the session to `~/.274bot/logs/session-<time>-<pid>.log`, rotating at
  4 MiB (3 old segments) and keeping the newest 10 sessions.
- The main panel shows a collapsible **resource** section whenever MultiBox is
  off, and the TUI status pane shows the same rows: bots N (M running), cpu,
  ram, traffic, plus a background-bot count. One sampler in the operator
  session feeds both (1 Hz, on its own thread; never per bot or per frame, and
  a slow probe never stalls a frame). RAM is the whole process, current
  resident size and lifetime peak (`812.3 MB process, peak 900.1 MB`; before,
  Linux showed the peak without saying so; the peak never reads below the
  current size), never a per-bot figure. A value still being measured reads
  `measuring…`, one the platform cannot measure says so, traffic with no live
  worker reads `no live slots` (not 0 B/s), a replaced or restarted worker
  re-measures instead of showing a false rate, and a failed sample reads as an
  error. The TUI header also carries the meter on one line at every size,
  80×24 included (`cpu 12% ram 263.9 MB net 1.2 KB/s`; `…` while measuring,
  `-` with nothing to measure, `err` on failure).
- **One status per bot in both front ends.** The operator session derives
  each bot's status once (offline, preparing, waiting, queued k/n, logging in,
  loading, idle/running, logged out, login error, failed; a bot walking a
  panel WalkTo or TUI map route reads running) and the panel (rail and grid
  caps, status section, banner, queue cards) and the TUI (fleet table, BOT
  line, header counts, status pane) show it. A bot waiting to reconnect no
  longer reads "logged out" on the rail; a bot parked with auto-login off no
  longer reads "waiting" in the TUI or "Waiting to connect" in the panel; an
  impossible queue place (3 of 2) is not shown. A login error stays visible
  as **last error** while the bot retries, until it is in game. In the TUI
  header a bot waiting out a failed login's retry backoff counts as queued
  (and failed); one whose error holds the login for the operator counts only
  as failed. Each bot shows its newest operation (`op#12 Start failed: …`),
  kept per bot however many other operations follow, and the bot's log
  records each operation's acceptance and outcome with its id (`op#12 Start
  accepted`, `op#12 Start completed`).
- Switching profile in single-bot mode, or turning MultiBox off, still leaves
  other bots running. A one-time acknowledgement names how many live workers
  remain and the live meter cost, points at the resource section, and **Got it,
  don't show again** sets `background_bots_ack` in `~/.274bot/panel-ui.json`
  (the TUI uses the same key). A failed write keeps the notice visible and
  reports the error; a later successful write clears only that error. The TUI
  shows the same sentence with a **Got it** button (`n` on the Overview). Live/harness
  boots do not show or persist the notice. The TUI pump reaps finished workers
  before counting background bots, matching the panel.
- Interactive panel-play and tui-play take an OS advisory lock on
  `~/.274bot/instance.lock` before prefs load, vault unlock, or slot spawn.
  The holder (`panel|tui` + pid) is published to `~/.274bot/instance.holder`
  (tmp + fsync + rename) after the lock is taken so a second instance can
  read it on Windows, where `LockFileEx` is mandatory, and is removed on a
  clean lock drop. A second instance warns and offers Exit (default) or
  Continue anyway, naming the real bot directory (the parent of
  `instance.lock`) in the warning; it accepts only a complete
  newline-terminated marker whose pid is still alive, otherwise retries
  briefly and warns (`pid unknown`) instead of failing startup. `--live` / memory / stress /
  harness boots skip the lock. `File::try_lock` is cfg-free
  (POSIX `flock` / Windows `LockFileEx`).
- Start (and Start all) after a fresh launch now runs the rs2b0t catalog script
  saved on the profile. Before, Start failed with `unavailable: <name>` until
  Browse or Load had filled the catalog, although the script section already
  showed the saved name.
- The panel and the TUI now share one operator session (`frontend-core`):
  vault, fleet membership and the logout latch, the selected bot, Load, Log
  in / Log out (single and all), removal, script Start/Pause/Stop and Start
  settlement run through the same code in both front ends. In the TUI,
  Load+login all loads every profile and logs every member in (a loaded,
  logged-out member is logged back in and a crashed worker recreated), and
  the selected bot has Log in, Log out and Remove (clean logout, then the
  worker stops; the neighbour becomes selected).
- Script coordination is shared too (`frontend-core` `Scripts`): per-profile
  assignment and parameters, Start all / Stop all, Reload and catalog Refresh
  run through the same code in the panel and the TUI. The TUI now edits the
  focused profile's own parameters (not the global `script-settings.json`),
  keeps a per-profile Browse selection, saves the assignment once a Start is
  Ready, and has Reload (warning, then Confirm or Cancel), Start all and Stop
  all buttons under the script buttons. Every script command also has a key,
  shown in its button (`b` Browse, `t` Start, `P` Pause/Resume, `e` Stop, `f`
  Load, `v` Parameters, `R` Reload/Confirm, `C` Cancel, `T` Start all, `E` Stop
  all), and the script rows stay on screen at 80×24. A parameter edit reaches the running
  script only after it is saved; a failed save is never pushed.
- Apply to all: from a profile's script parameters (panel Script prefs, TUI
  parameters `a` then `y`), copy that card's parameters to every wall member
  assigned the same card, after a confirmation naming the members. Each
  member's profile is saved, and a member running that card receives the
  parameters once the save succeeded (only the run seen at Apply; a restarted
  run already started with them). Members on another card are skipped; the
  report counts saved, failed and skipped separately from live delivery.
- Profile edits (auto-login, random events and lamp, credentials, render
  prefs, script assignment, tutorial flag, profile delete) no longer encrypt
  and write the vault on the UI thread. The edit shows at once; one writer
  thread saves it in order, and a running bot picks up the change only after
  the save succeeded. A failed save is shown on the error line and the
  profile goes back to its saved value. A new or renamed profile is selected
  (and its bot started) only after its save succeeded, and a rename saves
  the new name and removes the old one in one write. Edits made while a
  save runs are saved together in one write, and a running bot still picks
  up each of them (of the same setting, only the newest).
- A saved password change now applies to a bot that is already running
  (for example parked on the login screen): its next login uses the new
  password. Before (also in 0.1.8.1), a bot kept logging in with the
  password it was started with until it was removed and loaded again.
- Stop all now also stops a bot whose script is still shutting down for a
  Reload; before, the reloaded script started again after Stop all.
- In the TUI, Remove drops the bot from the fleet table at once and it can
  no longer be selected while it logs out.
- The TUI options popup now saves only the random-event and lamp fields it
  edits, and updates a running bot only after the vault write succeeded.
  Before, it replaced the whole profile settings from its draft, which could
  reset the profile's world pin or auto-login.
- **Responsive TUI shell.** `tui-play` lays out for 80×24 (two header rows,
  one main pane, a three-row message/log drawer, a footer), 120×40 (fleet
  table beside the selected bot's tab, a log drawer) and large terminals
  (queue and script columns, status and chat beside the tab). The fleet table
  has a cursor that never changes the selected bot (Enter or a click does),
  a `[x]` row selection with "selected N of M" (Space; the map's group walk
  uses it), and a filter by name, `wN` or status (`/`). The selected bot's
  tabs are Overview, Map, Script, Chat and Logs (`F3`–`F7`, `F2` the
  fleet); the footer always names where keys go, and the focused pane is
  labelled `[keys]`. `F1`/`?` opens searchable help for the focused pane;
  `Ctrl-P`/`:` opens a command palette listing every command with its
  target and why it is unavailable (it also reaches every screen when an
  SSH terminal swallows function keys). One routing model replaces the old
  global letters: an open popup or overlay takes every key (the Script tab's Browse, Load and catalog prompts too), typing wins in
  text fields, and each pane's letters act only in that pane — so `q` no
  longer quits from a popup and `x` no longer removes a bot from the
  settings popup. `Tab` moves keyboard focus (it no longer cycles the bot);
  WASD walks only in the armed Manual walk box; a dialogue is answered from
  the Chat tab and flagged elsewhere (`Chat!`, `DIALOGUE`). Remove, Load+
  login all, Log out all, Start all, Stop all and quitting with bots loaded
  confirm first, naming their frozen target or members; a fleet that changed
  since is shown again instead of run. Mouse: left click focuses and
  selects or presses, right click opens a context menu (never a left
  click), the wheel scrolls the list, log or map under the pointer, a map
  click selects a tile (walking it is a second action), and the palette
  turns mouse capture off for terminal copy. A resize re-lays out at once
  and click targets always come from the current layout.

### Slot lifecycle

- Forwarded local engine endpoints retain verified game data when their connect
  ports differ from the engine's `world.json` ports; missing verified content
  reports the profile/engine settings remedy.
- Slot startup, stop and restart now have one owned lifetime: per-slot
  registries are published before the worker starts, stopped and crashed
  workers release their queue place, and Log in or Login all recreates a
  terminal worker. Panel rail removal waits and reaps asynchronously instead
  of joining a worker on the UI thread; re-adding, re-seeding or logging in
  during that window cancels only the matching lifetime's removal and restores
  its IO.
- Login and logout commands are serialized by generation, so completion of an
  old logout cannot erase a newer Login all while a successful compatible
  handshake still consumes its one-shot intent. Retry notifications serialize
  with the wait predicate, and Logout latches at the command boundary,
  including while a slot is preparing or parked.
- Connected session state is distinct from scene/player readiness. Connection
  lights, loading labels, Logout and clean removal remain correct while a
  scene loads, while game actions and routing still require a current player
  and valid tile.
- Disconnect clears session byte/run counters but retains paused script paint;
  Stop and unload clear published paint even offline. Status-lock poison from
  a panicked worker is recovered at every access, while poisoned script slots
  fail UI reads, starts and controls closed until terminal retirement reaps
  them.
- Welcome dismissal keeps its spaced attempt bound and reports a visible
  failure after 10 eligible seconds; the window restarts while the scene
  cannot accept a close.

### Random-event guardian

- The strange box now solves "What colour is the Halfmoon?" on the public
  server as well as the local engine's "Half Moon"; shape and colour names
  compare ignoring case and spaces. Before, the guardian held the open cube
  forever.

### Script host
- `Bank.snapshotGeneration()` now follows acknowledged bank item packets,
  independently of the bank modal's session identity. `waitSnapshotAfter`
  can complete after a deposit without closing and reopening the bank, so
  PotionMaker restocks in the same session as its deposit. Withdraw and
  bank-machine requests still use the original open/close session fence.
- The Moss Giant dart qualification opens its seeded bank once and waits
  for that opening's item acknowledgement. Start now retains the loaded
  bank instead of racing a repeated OpenBooth against the acknowledgement.
- Oak Firemaker qualification uses the same measured 300-second full-cycle
  budget as Logs: burn the initial pack, reopen the bank, restock, and light
  again. Its core witnesses are unchanged.
- Catalog walking and recovery follow frozen rs2b0t more closely
  (`docs/api/script.md`, "Catalog walking and recovery"): `walkResilient`
  verifies with a route probe before giving up and honours `sceneRadius`,
  the teleport toggles and `distanceBeforeTeleport`; `avoidZones`
  rectangles are routed around (catalog zone ids fail loudly), and
  `maxBudget` is accepted (the host router never searches less). `createReturnToAnchorTask` keys off the leash and walks the
  frozen legs (long-range leg, `obstacles`, 90 s). `DirectNavigator` clamps
  and re-clicks. `Reach.npcDialog` walks with the resilient ladder, clears
  doors in front of the NPC over up to eight rounds, and stops its walk when
  a random event interrupts it. `RecoveryHints.takeAnchor()` returns the
  anchor across a watchdog restart.
- The traveller ends a follow as `EndBlocked` when the route's last tile,
  one step away, refuses every click, instead of waiting out the caller's
  timeout; script walks treat it as arrival as frozen `'blocked'` does. A
  transport approach refused right after a region rebuild is retried while
  the scene settles instead of failing the walk.
- The FlourCollector catalog card now loads its four Murder Mystery area facts
  through the shim; EssMiner stays visibly dimmed until native Gatherer support
  replaces its pickaxe-acquisition dependency.
- Refrozen mining cards receive `GAS_ROCK_IDS` as a real JavaScript `Set`
  derived from the selected cache's mining rows. CoalTrucks now ignores
  gas-event coal variants instead of failing on `.has` every tick.

- Alcher and LeatherCrafter can load their reachable-bank selector again.
  Selection runs one bounded native multi-target search off the slot pump,
  keeps the same-plane radius-four shortcut, and falls back to air-nearest
  on an unavailable route or a five-second completion timeout. A typed v2
  `bankNearestReachable` helper exposes the same select-only capability.
  The complete 20-bank catalog now preserves stable order, base-skill/quest
  gates, default-off Mage Arena/Zanaris preferences, and object/NPC access.
  Selected-world placements resolve safe walk stands; unavailable Canifis
  content stays a gated air fallback rather than an invented reachable bank.
  Stand resolution retains usable counter-side floor tiles with directional
  wall faces (including Varrock West), while still excluding blocked footprints.
  The selection waiter also bounds requests dropped before host admission;
  explicit v2 opt-ins override settings for both routing and air fallback.
  Native `walk-nearest-bank` reuses the winning route instead of flooding again;
  Stop/reload invalidates pending picks without replacing an existing follow.
  World bank facts now require explicit binding, so an early read cannot freeze
  an empty placement roster.
- Native `walk-nearest-bank` now follows a resolved bank stand to exact arrival, preserving the final booth-approach step before opening. An accepted `open-stand` now cancels the slot's armed scripted walk, as `open-booth` already did, so the bot is not walked off an open bank.
- `Banking.open`, periodic banking and world bank opens now share a Rust-owned
  select/walk/access continuation. Nearby banks still beat distant presets;
  explicit destinations remain the no-scene fallback. NPC and object access
  metadata survive selection, and deposit callbacks wait for loaded bank stock.
  An NPC bank without a dialogue choice never selects an unrelated first option.
  A focused `alcher_dwarven_mine` scenario observes dungeon exit, the selected
  Falador East bank, real withdrawal and fresh High Alchemy XP.

- Baker-stall carried-food counting matches frozen substring patterns, so
  partial cakes (`2/3 cake`, `Slice of cake`) satisfy restock and eat gates
  the same way whole `Cake` does.

- Baker-stall restocking now awaits one Rust step machine; Rust owns selected
  stall facts, callback polling, waits, steal verbs, stand swaps and lockout
  sequencing while callbacks retain the options object as their receiver.
- The v2 clue session is begin plus one awaited run; Rust owns its continuation
  table, optional callbacks, waits and typed verb emission.
- The v2 quest journal is begin plus one awaited run; Rust owns its row click,
  modal acquisition, retry window, exact-pair close and timeout.
- The obsolete clue-verb JSON adapter is removed; the clue machine emits typed
  interact requests directly.
- Script startup, tick interruption and `onStop` now share one serialized
  lifetime: Stop cannot leak a tick interrupt into the hook, Pause blocks new
  entries while legitimate slow work finishes, and Pause shares the watchdog's
  one-game-tick (600 ms) runaway horizon instead of cutting work after 50 ms.
  The horizon follows the active execution's own start rather than later tick
  dispatches. When a watchdog, Pause deadline or session-reset terminate
  actually cuts JavaScript, the host logs the runaway and recreates the script
  runtime instead of guessing which continuation owned the cut; an
  operator-paused script remains paused after recreation. Three cuts within
  five minutes stop the script with a visible error. Disconnect carries an
  active execution's same deadline across the session reset, even though no
  logged-out ticks remain to drive the watchdog. A final queued Pause remains
  authoritative over older Pause/Resume commands. Hostile startup/validation
  is bounded and reclaimed.
- Reload and catalog validation now run off the panel UI thread. A Pause that
  wins the final host dispatch fence preserves the drained script actions for
  Resume instead of silently losing them.
- Active tick errors clear only after a completed successful loop; held frames
  and cancelled ticks no longer manufacture recovery, while a clean loop after
  reconnect and explicit Stop clear the active status and retain diagnostic
  history.
- `buryOneInFight` follows frozen `fightUpkeep`: it skips only the tick the
  swing began instead of every animating tick, so bones are buried during
  attack cooldowns, and it answers true only once the backpack drops a slot
  within three ticks (a queued Bury is no longer a burial). One Rust
  `fight-bury` machine owns the gate, the click and the confirmation.
- `swingStartedThisTick`, `buryOneInFight` and `AttackClock` follow the
  frozen clock: the snapshot now carries the local player's animation id,
  and a swing starts on the first animation seen and on every change to
  another non-idle animation. A script started or reconnected mid-swing no
  longer buries or eats on that tick, and a new attack animation that
  follows another without an idle tick counts as a new swing. Each
  `new AttackClock()` keeps its own state and observes only when called.
- `reader.selfAnim()` returns the local player's animation id and
  `Game.animating()` is true only while an animation plays, as in frozen
  rs2b0t; walking alone no longer counts as animating (it did through the
  host's walking-or-animating flag). Catalog scripts that wait on
  `Game.animating()` (GnomeMagicChopper, AgilityBot and others) and the
  `lightFire` start check follow.
- v1 `foodHealAmount` answers every name like frozen `food.ts`: the exact
  selected heal, else the first selected food whose name contains the given
  name or is contained in it, else 8; an empty name is 8. Unknown, partial
  and ambiguous names (a configured `Cabbage`, `Swordf`) no longer throw
  `not impl` mid-fight. Only missing game data still throws its explicit
  "game data unavailable" error. `shouldEatToUseFood` now holds a heal of
  zero or less above the eat floor, as frozen does, and runs in Rust.
- `ChatDialog.continue()` resolves like frozen: true once the chat modal
  changes or the next page offers Continue again, false with no Continue
  posted or after 3 s. It waited on the inverted condition, so a multi-page
  dialog reported false to callers that branch on it (GatheringBot's desert
  camp route) and a vanished Continue on the same page reported true. The
  press and wait now run in the Rust `chat-dialog` machine.
- `resolveCookLocation(setting, from, unlocked?)` follows frozen
  `CookLocations`: `Auto` takes the host cook location whose bank is
  nearest `from` among those the account can open (it was always `null`,
  so CookBot stopped with "no bank called 'Auto'"), a named location is
  returned only when its bank is unlocked, and a caller's `unlocked`
  predicate replaces the bank requirement.
- `withdrawOp(ops, amount)` reads the label off the bank row's own ops for
  all six frozen amounts (`all`, `10`, `5`, `x`, `1`, `any`) instead of
  assuming `Withdraw All/10/1`; a row without the op answers `null`, so a
  catalog fallback (`?? withdrawOp(ops, 'any')`) takes the op the row
  actually has.
- `describeCombatStyle` names the style the weapon actually trains, as
  frozen does (`strength (training Strength)`, or `defence (training
  Defence; controlled unavailable)` after a fallback), instead of echoing
  the requested style.
- `parseCombatStyle` and `parseRangeStyle` answer frozen's defaults for an
  unknown setting (`strength`, rapid mode 1) instead of throwing `not impl`,
  and `parseRangeStyle` accepts `long range` / `long-range`. The style tables
  are Rust's.
- `eatAtHpThreshold(maxHp, heal, minHp)` is implemented as frozen: the HP
  at which a full heal fits, kept below max HP and at least the floor
  (default 5); it threw `not impl`.
- `foodForms` / `foodCount` / the combat keep list treat the frozen partial
  forms as the whole food: a chocolate cake's `Chocolate slice`, a pizza's
  `1/2 … pizza` and a pie's `Half a/an … pie` (read from the selected item
  aliases; the cache spells the pineapple half `1/2pineapple pizza`).
- CookBot's location list is frozen's: every bank in the roster, each paired
  with its hand-walked camp surface (Catherby, Seers, Draynor) or the
  nearest placed Range/Fireplace within 20 tiles of the bank, ovens first
  (fire mode only when none is). The surfaces are selected content: the
  game-data generator now publishes every cook surface in the 274 and 289
  map packs (`cook_surfaces`, matching rs2b0t's generated table; 289 has one
  more fireplace), and generation fails if a curated surface moves. Without
  game data the list is empty and `resolveCookLocation` reports "game data
  unavailable".
- A dropped connection now pauses the whole Load script and resumes it after
  the relog, as rs2b0t's AutoRelogin does. An offline slot logs in while
  auto-login is on or a script is running or paused on it, as rs2b0t does,
  and stops trying once neither holds. Every await, step machine and task
  runtime in flight (a walk, a bank open, a death-recovery walk-back and the
  walk inside it) stays as one chain; the slot re-sends the script walk it
  was following and the walk requests the dropped connection never sent
  (up to eight), under the same request ids, so the walk completes instead
  of returning `false` and nothing else starts beside it. An operator or
  idle logout, Stop and slot removal still end the work as before. While a
  script is paused (operator Pause or a reconnect), `Execution.delay` and
  `delayUntil` timeouts no longer run down: a wait resumes with the time it
  had left.
- `Traversal.walkTo` is one Rust `walk-to` machine, and like frozen it earns a
  missing Karamja boat fare: when a walk off the island fails with the 30-coin
  fare as its only missing gate item, the bot takes Luthas's banana-plantation
  job (employment, picks, fills the crate to ten, collects the 30 coins) and
  walks again. `walkResilient`'s baked leg does the same. A pack with no free
  slot and no banana skips the job, as frozen does. Qualification cell:
  `script_boat_fare_v1_ts`.
- `Traversal.walkTo` keeps frozen `WalkOptions`: radius 2 and a 300 s bound
  by default, an explicit teleport opt-out wins, `distanceBeforeTeleport`
  gates teleports, `avoidZones` rectangles are routed around (Death
  Plateau's secret path), and the card's Sustain hook runs every tick of the
  walk. Options the host cannot honour (catalog zone ids, teleport id lists,
  ship or shortcut exclusion, `pathFollow`, `forceRepath`) now fail loudly
  instead of being dropped.
- Stopping a script, or a script stopping itself, now stops its walk. An
  operator Pause stops the walk and the host carries it: Resume sends it
  once more (any script walk, bank-open and raw v2 walks included) unless
  it was cancelled, the player already arrived, or a newer walk replaced
  it; a watchdog recovery walk resumes too. A walk interrupted by a random
  event, or whose Sustain hook throws, also stops its route.

### Upgrade compatibility

- Added a real encrypted 0.1.8.1 home fixture and a permanent behavioral
  upgrade test covering complete vault profiles/settings/assignments, panel
  preferences, loadouts and legacy script overrides through a 0.1.9
  save/reopen. The release audit inventories every operator and derived
  on-disk artifact, includes the in-flight frontend vault writer, and defines
  the packaged macOS/Windows/Linux in-place upgrade gate.
- An external navigation pack older than the current `274V10` format,
  including retained `274V8` and `274V9` files, now leaves navigation
  unavailable with a `rebuild it with nav-pack` diagnostic instead of aborting
  panel/TUI startup; the old file is never deleted. Startup logs whether a
  loaded pack came from the packaged `274V10` bundle or an external path so
  release gates can prove package provenance.

### Packaging

- The Linux package ships `panel-play` beside `tui-play`, like macOS and
  Windows, with the adjacent `nav/289/` and `map/289/` resources (its fonts
  are compiled in). It needs an X11 or Wayland session and a Vulkan driver
  (a GPU driver, or Mesa lavapipe for software rendering); `BOT_CPU=1` draws
  the game view with the CPU rasterizer. The system libraries it links and
  loads are listed in `tools/release/README.md` and `FIRST-START.md`.

## [0.1.8.1] — 2026-09-24 — Alpha 3 patch

A patch on 0.1.8: a public-289 crash, both public worlds, the login queue and
three rendering bugs. Crate versions stay `0.1.8`; the tag and packages are
`0.1.8.1`.

### Crash

- Fixed an abort on public-289 when a queued account retried after its login
  cooldown had already expired (a negative duration in the login queue).

### Public worlds

- Public 289 world endpoints come from editable `~/.274bot/worlds.json`
  (w1/w2 defaults); account settings can pin a world or choose auto, and
  panel edits apply to an already-running slot at its next login handshake.
- Auto accounts move to the next world on a full-world login response, with
  a pause after all worlds report full. Panel and TUI show the world used by
  the current connection; `tui-play --world N` sets the default for auto
  accounts. The panel rail marks each member with its world number.
- Public login fetches the world's RSA modulus at runtime with a baked-key
  fallback and refreshes after a wrong-key response. The shared cache tries
  the next listed asset world if the first cannot be reached.
- Login response 21 (just left another world) shows the server's transfer
  countdown and then logs in again on the same world, instead of failing as
  an unexpected response.

### Login queue

- The queue is first come, first served in the order bots are ready to log
  in, with the focused bot moved to the front. Only a bot that is actually
  waiting holds a place, so an online or still-loading bot can no longer
  block everyone behind it (Login all could previously stall with
  "1 of n, 0 in front"). A bot that stops or crashes gives up its place.
- Each bot draws its own queue card ("k of n") on its own view; it appears
  only while that bot is held in the queue and clears on login.
- Every login attempt counts toward the server's per-address and per-account
  limits, a server "too many attempts" reply pauses all bots together, and
  retry waits are not cut short by clicks, focus changes or Login all.
- Turning auto-login off and on keeps an explicit Log in.

### Panel

- Rail and grid labels name the login step (starting, queued k/n, logging in,
  loading) instead of "logged out" until the bot is in game.
- The game view is centred horizontally in its pane.
- The script load-failure list is collapsed by default.

### Rendering

- Players and NPCs behind a wall are no longer drawn through it (Fishing
  Guild bank parapet, CPU and GPU): the scene painter completes each tile's
  back pass like the Java client.
- Walls sharing lighting with their neighbours no longer turn black when a
  ground item or a wall/floor decoration on the same tile changes or animates.

## [0.1.8] — 2026-09-24 — Alpha 3

JS API v1 compatibility with the frozen rs2b0t catalog on revision 289, and the
JS shim pulled back toward name maps: most loops, tables, decisions, retries and
sequencing now live in Rust step machines that read the snapshot Rust already
holds (residuals are listed under Known limits). Revision 274 remains
best-effort and untested in this release.

### Script runtime (fence)

- One decoded scene per isolate; Rust helpers read it instead of JS-posted
  pages. The runner, park list and a single paint pass per tick are Rust-owned.
- A Rust step-machine host (`runMachine`): each driver is one native start and
  one await, with exclusive supersede, reset/pause/hold semantics, join and
  watchdog claims, and one Rust→JS callback path (synchronous hooks stay
  synchronous, as in frozen). Teleport, prayer, autocast, special, modals, bank
  open/access/deposit/withdraw/close, PeriodicBank, DeathRecovery, bankNearest,
  light fire, shop, chat dialog (make/makeX/makeFromPanel/chooseOption), dialog,
  reach (npcDialog/entityOp), walkWithHops, walkResilient, trade, partner trade,
  clue/Sherlock and every hunt family run on it.
- The v2 hunt family is begin plus one awaited run with typed `.d.ts` inputs;
  the nine v2 hunt examples are rewritten to that contract.
- Paint: canvas ops record on a tape and flush once per pass; paint frames are
  shared by `Arc` with sender-side caps; the FlatBuffer paint codec is removed.
  `fmtDuration`/`fmtXpHr`/`etaHours`/`levelProgress`/`paintSkillShort` emit the
  rs2b0t strings.
- Isolate Start/Stop no longer block the UI thread; the isolate command backlog
  is bounded; no JSON text is evaluated as source on the tick path.

### Compatibility and behavior

- v1 `onPaint` runs only after `onStart` succeeds and while the reader is ready
  (in game, scene 2, a tile, stats loaded), as frozen `ScriptRunner.paintBot`.
- Walk arrival is one Rust rule equal to frozen `isArrived` (reach probes), used
  by the isolate wait, the host follow and every shim pre-check; a route that
  ends on its approach tile settles true (frozen "closest").
- `Tile.distanceTo` runs in Rust (NaN propagates as in `Math.max`).
  `Quests.points()` reads varp 101 and every nonzero varp is posted.
  `Equipment.unequip` and the `Equip` op use real host verbs.
- `liveCatalog()` items, `tradeable`, `displayName`/`clientName` and note links
  come from selected game data (game data gains `tradeable` and
  `stack_variant`). `ChatDialog.makeFromPanel`, `Shop.sell` with a pick
  predicate, `SolveClue.walkToBank`, `requiredThieving`, non-booth
  `Bank.openNearestAccess` and `Bank.openNpcAccess` are implemented.
- Declared-surface stubs throw `not impl` on use instead of returning fake
  values.
- A superseded `Game.teleport` resolves `false` in the tick it is superseded.
  v2 `api.tick` advances on every eligible tick, including while an async tick
  is pending. In the ResetSession window the v2 quest journal/status return
  `snapshot-unavailable`.
- `RunManager.override({ runAuto?, energyMin? })` crosses the existing
  isolate-to-host FlatBuffer batch into the matching host slot's run-scoped
  auto-run overlay. Missing fields use host defaults (`runAuto: true`,
  `energyMin: 20`); Start/Stop and runtime replacement clear it, while the
  active run preserves and re-publishes it across relogs.

### Gameplay fixes (live 289)

- Sherlock continues a giver dialog left open when a casket lands.
- Ardougne stalls: the frozen stealCakes loop runs in Rust; the combat lockout
  is waited out instead of counted as a refusal.
- Nav steps through `open_and_close_door2` doors (Tenzing's hut) instead of
  stalling the door hop; the host follow ends at the requested radius.
- A walk wait settles on a posted outcome that later snapshot deltas omit
  (wolf-pit recovery); `walkResilient` retries as frozen.
- DeathRecovery recovers only near its anchor (frozen `near`), and PeriodicBank
  returns with the bank open, as frozen does.

### Rendering and client

- Client `bc9b7f7`: side-step walk sequence names, zero-delay frames play for
  one cycle as in Java, GPU texture coordinates keep their sign across zero,
  GPU tests skip cleanly without an adapter, and clippy 1.98 cleanup.

### Navigation and packaging

- Navigation bakes are reproducible: the transport edges are put in a canonical
  order, so the same revision inputs produce byte-identical `.navpack`,
  `.navreach` and `.navcanlight` files on every run and platform. Previously
  door, ladder, stair, agility and teleport edges came out in hash-set or
  directory-listing order. The navigation source digest labels paths with `/`
  on every platform, so Windows records the same provenance.
- Release packages carry the local 289 navigation build; CI builds without
  navigation inputs (`BOT_NAV_BUILD=skip`). Internal campaign notes under
  `docs/compat` and `docs/harness-integration` are no longer tracked.

### Known limits

- Defence-1 `moss_giant`/`green_dragon`/`fire_giant` catalog cells cannot win
  their fights on 289 (each eat clears the attack); the prepared cells are the
  representatives. `moss_giant_bank`, `ardy_fighter_bank` and the earned-loot
  step of `rock_crab_bank` depend on the frozen Fight hold or drop RNG.
- Fence residuals for 0.1.9: `cake_stall.js` still pumps a Rust begin/next
  driver instead of one machine await; the v2 clue and quest-journal APIs are
  still begin/next; several in-isolate helpers still take untyped JSON
  arguments (no additional host wire).
- Lumbridge fountain banding on the GPU path needs a vertex-format change
  (0.1.9). Measurement-only performance claims are deferred to 0.1.9.

## [0.1.7] — 2026-09-16 — Alpha 2

### Server profiles and operator script controls

- Immutable per-process server profiles: `local-274`, `local-289`,
  `public-289` (`--profile` / `BOT_SERVER_PROFILE`) with revision-specific
  default engine roots, game/asset ports, vault paths and unpack dirs.
  `public-289` uses WSS/HTTPS on `w1.rs2b2t.com:443` with the baked public
  RSA. Conflicting profile/revision/prod combinations fail closed.
- Build-time nav bake/stage defaults to revision **289** next to the
  binary (`BOT_NAV_REVISION`, `BOT_NAV_BUILD=skip` opt-out); `nav-pack`
  remains for custom-input bakes.
- Content-addressed JS/TS transpile cache under `~/.274bot/js-cache`
  (raw-origin SHA-256). Per-profile script assignment and settings bags
  persist in the vault on successful Start.
- Native panel: manual **Reload**, catalog **Refresh catalog**, MultiBox
  **Start all / Stop all** (separate from Login all / Logout all). Reload
  confirm restarts matching running bots and Stops matching paused bots;
  unchanged origins skip transpile.
- Native ordered suite runner `e2e-suite` (profile/catalog selection,
  content-bound identity, process-tree ownership, capture contracts).
  Runner completion is not script qualification; visual captures remain
  `pending_visual_review` until readback. See [docs/e2e-suite.md](docs/e2e-suite.md).

### Release startup and panel fixes

- Drain buffered WSS messages during readiness checks and preserve payload reads
  across WebSocket control frames.
- Enable native clipboard paste into masked profile inputs.
- Fix Loadouts equipment-grid widget IDs and quantity editing; enable item
  search for the audited public-289 cache while rejecting unknown identities.

### Selected allocation, gameplay and portability corrections

- Reuse completed dynamic sprite slots; read animation delay without cloning
  transform data; box sparse appearance packets; share private animation bases
  while keeping public lookups independently owned.
- Release script snapshot storage on explicit Stop, preserving Pause and fresh
  restart keyframes. Release completed scenario snapshots after terminal evidence.
- Share the Play navigation world with harness seeds, including failed-load
  behavior. Allocate panel CPU upload storage only when a CPU frame needs it;
  retain pixels through GPU/CPU switching.
- Preserve routing worker ownership and stale-result rejection, retry failed
  radius requests without losing the prior route, and keep bank approaches
  radius-aware. Withdraw X now waits for published inventory; loadouts and food
  resolve through Rust helpers. Existing unsupported helpers remain errors.
- Correct adjacent door crossing and baked door edges; rebake existing v8 packs.
  Retain the bounded revision-274 stun-recovery inference and focused login
  priority across reconnects.
- Restore overlay, minimap-freeze and main-modal uploads. Add native Windows
  socket wakes, HOME/USERPROFILE selection and real process resource metrics.
- Retain the opt-in fleet harness for preservation work, with explicit fixtures,
  terminal evidence, TUI/panel integration and portable basic accounting. See
  [harness usage and limits](docs/harness.md).

These are selected engineering improvements, not a combined RSS or capacity
claim. Experimental snapshot sharing, borrowed fingerprints, tiled navigation,
and advanced campaign capture/controllers are excluded.

### TS shim (`crates/script`)

- `$RS2B0T` registry parse (static scan of `src/bot/scripts/index.ts`, no
  V8): listed scripts become Browse cards; the root persists to
  `~/.274bot/rs2b0t-path` after the first successful parse.
- TS transpile at Load (CompatClass shape), rs2b0t import remap, and a
  throw-on-missing Proxy for the listed API.
- Shim Game / Inventory / EventSignal from the posted snapshot;
  `Execution.delayUntil` parks the isolate on PLAYER_INFO; Banking.open
  and bank deposit / withdraw on the BankSide container.
- Live-script gaps closed: `Inventory.first` + held-item `interact`
  (`{op:'held', name, action}`), `reader.inventorySize()` mirroring the
  posted inv-tab slot count, `LoopingBot.log` / `settings`, the
  `paintLogic` `fmtDuration` module, and `Bank.setNoteMode` /
  `withdrawOp`. The live shim inventory read now targets the side-tab-3
  backpack exactly (a first-TYPE_INV scan grabbed a bank/trade widget).
- FlatBuffers isolate IPC: delta snapshots omit unchanged tables, and a
  hold tick re-posts so `EventSignal.pending` sees the held state.
- `EventSignal.pending` and `ignoredRandoms` surface from the bot
  instance.

### Guardian solvers (complete)

- The 0.1.2 stubs are gone: evade flee, plant pick, maze / mime /
  strange-box, hazard, lamp rub, and lost-gear / lost-tool solvers land
  as the complete act set.

### Nav and banking

- Pack `274V` version **8**: content-derived bank-stand table baked by
  `nav-pack` over the content tree. v7 files are `BadVersion` — rebake.
- Banking.open and the BankBudget session: deposit-withdraw-wear with
  **any-of** `worn_req`.

### TUI and panel

- `tui-play` script chrome is wired (Browse / Start / Pause / Stop /
  Load over the same JS library); a recording script's paint shows in
  the chat pane (`p` toggles back to the game ring).
- Panel paints ScriptPaint as ImGui over the Game chatbox — never on the
  client framebuffer.

### Live BoneBurier

- `script_bone_burier` live gold: the **real rs2b0t TS BoneBurier** runs
  through the shim on the driven slot. The host starts the `$RS2B0T`
  catalog card at live boot (`scenario.start_script`); the runner seeds
  the account (tutorial skip, five Bones given before the clean relog so
  the script's `onStart` gate opens once the inv tab binds) and then only
  watches for the server's "You bury the bones." chat line. PASS with a
  unique minted account; headed `panel-play --live script_bone_burier`
  and headless `tui-play --live script_bone_burier` pass the same
  runner.

### Public world docs

- `BOT_TARGET=prod` (alias `live`) / `host-play --prod` → `w1.rs2b2t.com:43594`
  with the baked public RSA; the local engine stays the default. Cargo
  `TARGET` is the rustc triple, not a world switch. Not Jagex, not a
  hosted wall, no w1 CI.

## [0.1.2] — 2026-09-01

- Rust **1.98.0** is pinned (`rust-toolchain.toml` + CI, host and client).

### Random-event guardian

- Detect-all + owner: every random kind this rev spawns is named in
  `RandomStatus`; `ours` is a hard NPC target (`target == self`) or an
  overhead-name match, no distance-grab.
- Dialog act: Talk-to + chat continue for genie, drunken dwarf,
  mysterious old man, sandwich lady, frog — no WalkTo, no fail-teleport.
- Hold while a dialog is in flight or the slot is trapped (maze / mime /
  box): script ticks **and** the nav route freeze, then resume latched;
  `on_random` knock (`RandomClaim::Host` default, no in-tree `Handle`);
  45 s wrong-talk cooldown per NPC slot.
- `ProfileSettings.random_events` (default on; off still detects and
  publishes), `lamp_skill`, `lamp_auto` — the lamp stays inert this tag.
- Panel status row binds the same `RandomStatus`.

### TUI operator panel

- New `crates/tui`: `tui-play` (ratatui + crossterm) is the headless
  second view of `host_play::Play` — same slots, raster Off, no GPU.
- Classic collision-dot map (basemap off) with town pins, You Are Here,
  and the remaining-walk polyline; Walk-confirm routes via `arm_walk_on`
  (lifted to host-play), WASD one-tile walks, chat / NPC dialogue
  Continue/Answer, status + `RandomStatus`, inv / stats / nearby locs,
  settings popup; script chrome is shape-only.
- `tui-play --live script_*` runs the same scenarios as `panel-play`;
  e2e unchanged. TestBackend tests in CI, no GitHub TTY.
- Map pane paints the packed collision (`Play`’s nav world), not the
  live loc list — a missing copy left `map (no nav pack)` while routes
  still ran.

### After 0.1.2 (v0.1.5)

- Evade flee, plant pick, maze / mime / strange-box solvers, lamp rub
  (`lamp_auto`).
- WalkTo a live NPC tile when Talk-to is out of range.
- Hitsplat window (`combat_cycle > loop_cycle`) as an extra ours signal.
- JS / TS shim `on_random` / `EventSignal.pending()`.
- Per-name ignore list (rs2b0t `setIgnoredRandoms`).

## [0.1.1] — 2026-09-01

Nav **execute**. Headed gold: `panel-play --live script_nav_routes`
`PASS arrived(2817,3443,0)`. Honest bot scripts are still not this tag.

### Nav

- `WorldState` from the live snapshot fail-closes `find` (skills, items,
  quests, transmit-yes varps, worn). `worn_req` is **any-of**.
- `FindOptions.allow_bank_fetch` is named (checkbox + flag); fetch is not
  implemented.
- `Traveller::follow` executes packed OP_NPC (cart, essence wizard, Elkoy,
  sailors, glider pilots), EssenceSession return, Shantay both ways, and
  packed teles. WalkTo **Teleport** stays `::tele` on loopback.
- Boats: Talk-to the sailor onto the **deck**, then loc Cross the
  `_gangplank_disembark`.
- Gliders: take-off if varp 150 ≥ 160 **or** the Grand Tree journal is
  green; landings settle Chebyshev 1 (`map_findsquare` scatter).
- Slashable webs (loc 733): `oplocu` knife (`option` 0, `item_req` 946)
  and `oploc1` Slash (`option` 1) when any `slashattack_anim` blade is
  worn. Traveller trolls the 50% slash fail like a door.
- Agility shortcuts wait the packed `edge.ticks` after land.
- NPC-backed hops search radius 8 and walk to the **live** NPC tile
  (packed `at` is spawn; officers wander).
- Pack `274V` version **7**: 9-bit walk (`u8` face + `SQ_BLOCKED`). v6 is
  `BadVersion`. Rebake.
- Path-facing orbit yaw (rs2b0t `navCameraFollow` shape; host writes
  `orbit_camera_yaw`).
- Remaining transport hops get a short caption on the Game overlay.

### Scenario / live

- Unique live account per invocation; `--live` uses an ephemeral vault and
  does not write operator settings.
- FAIL dumps include chat (newest first) and `tile` `[x,z,level]`.
- `script_nav_routes` is the headed corpus; `nav_door` stays the Catherby
  door-troll gold fixture.

### Host and panel

- Auto-run (bothost IF_BUTTON) only after `ingame && scene_state == 2`.
- Headed live paints the scenario Follow route, not only WalkTo.
- Scenario nav overlay (paints, camera, find flags) is session-only.

### Client (`FR-client-bothost` `r274-bh-modular`)

- GPU keeps the last 3D texture while `scene_state == 1` (Java freeze;
  overlays such as `ship_journey` still draw).
- Orbit camera chases during that freeze; `LinkBelow` lifts pitch-clamp
  samples so bridges do not slam top-down.
- Logout clears the tutorial overlay (`tut_com` / flash / modals).
- Nav debug hop labels.
- Client CI: fmt + clippy `-D warnings` + test (same bar as the host).

### Git

- `origin/main` is this checkout’s commit history. The `0.1.0` tag remains
  the squash that first went public. Later tags are ordinary annotated
  tags on `main`. Do not squash-publish.
- Rust **1.97.1** is pinned (`rust-toolchain.toml` + CI). Bump in the
  0.1.2 session.

### After 0.1.1 (planned, not this tag)

- **v0.1.2:** headless TUI (ledger only).
- **v0.1.5:** TS rs2b0t compatibility shim; listed scripts, not all-ports.
- **v0.2.0:** hot-load `.ts` as one-session tasks.
- **v0.2.5 beta:** listed scripts against **our** API or the **rs2b0t**
  API.

Alpha gaps (honest, not this tag): DebugPanel v2, Loadouts / parameter
Edit, crates.io publish, 3-platform bins. Random-event handler is 0.1.2+.
Zone-reuse / extra map chunks are not this tag.

## [0.1.0] — 2026-08-31

Alpha of the **bot host + API + nav**. Honest bot scripts are not part of
this release. The script *kernel* (Browse / Start / Pause / Stop, JS Load)
and WalkTo are in-tree.

### Host and panel

- One OS thread per client, 20 ms loop, login FIFO, AES-256-GCM vault.
  Login throttle matches Lost City production (`30` / 60 s per address,
  `4` then remaining of 15 s per device). Local engines default
  `production: false` and do not apply those counters; the host still
  stays under them.
- Native `panel-play` (ImGui, panel-owned winit + wgpu loop): profile picker, Log in / Logout,
  WalkTo, MultiBox rail/grid, click-through capture. Panel/rail widths
  stay 330/264; host-window resize is grid-only. Non-grid Game blit is
  native 765×503. Opening MultiBox grows the OS window if the blit would
  be covered. DPI is OS/winit.
- Per-slot none / GPU / CPU. GPU↔CPU or lowmem/highmem drops and
  reattaches the **renderer head** on the same `Client` — never a logout.
  Draw-off detaches GPU textures, chrome, and decoded overlay mesh; the
  socket and sim keep running. The next paint reattaches from parked
  stamps. Sidecar click is focus, not a restart.
- Game pane follows the **focused** slot. With only-render-selected (the
  stress50 default), unfocused rail members stay raster Off and cannot
  grow extra GPU heads. The focused member can take the GPU seat even if
  its rail tile is Off.
- `--live stress50` is the release 50-head RAM watch (cap-only rail, one
  GPU seat, Game 50 fps). `--live stress50_full` paints every member at
  50 fps. Neither fails on RSS size; PASS prints `rss=… up=50/50`.
- One process copy of IfType decode, fonts/media, and GPU pipelines.
  One OnDemand worker (and one update socket) per `(host, port)`. Occupied
  scene tiles are created on place; empty squares are holes. Loc geometry
  is a process LRU (`SceneModel::Shared`). Unheaded `map_build` stamps
  typecodes/heights/collision; the first headed paint materializes overlay
  and the minimap. Process-wide loc models are not unloaded per 104.
- Headed default is the client submodule's **wgpu** renderer
  (`BOT_CPU=1` is CpuPix3D).
- WalkTo picker in the Game pane (north-up, click-to-pick, Recentre /
  Walk; **Teleport** cheat on loopback engines).
- Local-engine debug heading: TutSkip, Lumbridge (`~home`), maxme,
  Teles. DebugPanel v2 is a stub.
- Script chrome Browse / Start / Pause / Stop + JS Load. Loadouts and
  parameter Edit stay mocked until the TS shim.

### Nav

- Whole-world collision + transport pack (`274V`, version byte **6**):
  compact `u16` walk words per tile. Optional `274F` flags sidecar for
  collision paint. Rebake; v5 streams are `BadVersion`, not silently
  loaded as v1.
- Dijkstra `find` / `find_with` (`FindOptions`: wilderness and teleport
  opt-in, both default off) and pollable `Traveller::follow`.
- Transport coverage includes doors, ladders/stairs, agility, gates,
  spirit trees, cart NPC hops, wilderness levers, Al Kharid toll / Shantay
  northbound, essence-mine **entry**, Elkoy escorts, Zanaris shed + worn
  Dramen. Cow-pen → Varrock uses the south gate (`INTERACT_RADIUS` 1).
- Reach overlay is **paint-only** (not `find`). Traveller does **not**
  yet execute OP_NPC hops (cart / essence / Elkoy) or EssenceSession
  return / Shantay free-exit / tele **execution**.

### API

- Snapshot → query → interact → settle. No tick-end opcode; compiled
  scripts wake on the `PLAYER_INFO` gen edge.

### Client (`FR-client-bothost` `r274-bh-modular`)

- Bot-host fork: gens, skip-paint, shared cache, wgpu GPU 3D (CpuPix3D
  via `BOT_CPU=1`). No bot action API inside `client`.
- Login RSA is **runtime**: stock LC Java pair for local-dev; optional
  `$ENGINE_DIR/data/config/private.pem` or `LOGIN_RSAN` if you rotated
  keys.

### Contributor fence (alpha)

You bring a local engine and pack cache. Live cache fetch / turnkey
public-world login is a **beta** goal, not this tag.

Shipped as the public squash tag `0.1.0`. Later history is on `0.1.1`.
