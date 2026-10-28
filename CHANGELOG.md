# Changelog

All notable public changes to 274bot. Host workspace crate versions are `0.2.7` and
`publish = false` (not on crates.io). Git tags are `0.1.0`, `0.1.1`, …

## Unreleased

### Panel and terminal app

- Source builds can now omit the JavaScript runtime with
  `--no-default-features`. These builds keep Gatherer, Grinch and Quester,
  their settings, and native Start, Pause, Stop and paint controls. They do
  not offer Load, Reload, catalog or rs2b0t profile import. Default builds
  still include JavaScript. Sherlock needs the JS-runtime build until its
  native clue rewrite, although Sherlock itself creates no V8 isolate.
- The terminal app's script pane now marks the selected compiled (native)
  script card with `>` the same way as a selected JavaScript card.

### Combat

- Bots now drink their boost potion before the first hit even when the target
  starts out of reach.
- Bots now know how hard common monsters such as cows, chickens, rats and
  imps hit. The eat line and the no-food stop line follow each monster's
  real max hit. A cow's max hit is 1, so a bot eats at 3 hitpoints and
  stops for lack of food at 2. Imps cannot damage the bot at all (max hit
  0), so the bot does not raise Protect from Melee against them.
- When a quest fight cannot attack its attacker, the bot now asks the route
  planner for a safe place to walk to. It offers the quest's own stand first,
  then places beyond the attacker's chase range, then places that are only
  farther away. The planner picks the nearest place it can reach. Before, the
  bot could pick a spot it could not reach, stand still, eat all its food and
  die. The static hostile tree near Draynor Manor is one such attacker. If a
  route fails, the bot plans again from where it stands and prefers the next
  kind of place. A route that neither arrives nor fails also ends: the bot
  counts game ticks, and when the walk takes longer than the measured walk
  distance plus a margin of 12 ticks, it gives up on that route and starts
  the next. When the walk distance cannot be measured, the limit is the
  planner's own limit on route length, and the walk machine's ten-minute
  deadline ends the walk first. A pass makes one request for each kind of
  place that has candidates, so up to three requests (two when the quest
  names no stand). The places of different kinds can overlap. A second pass
  can follow the first when the food runs out, so the bot makes at most six
  requests. While food remains, the first pass keeps eating as it walks; the
  second pass, after the food is gone, does not eat. The quest's own walk
  starts only after the bot sees that the attacker is gone. If every route
  fails, the quest stops with the last route failure. This does not promise
  that an escape exists.

### Navigation

- Bots now wait and try again when another player is using a shortcut,
  instead of giving up. The Yanille dungeon ledge ("The ledge is being
  used") stays held for at most 9 game ticks after a refusal, so the bot
  waits that long and crosses again. A busy rope swing works the same way
  with a wait of 5 game ticks. The bot stops after 60 game ticks in total
  for one crossing (refused tries, waits, the crossing itself and the pause
  after landing) and never tries again after that limit. The failure then
  reports how many times it tried and the last refusal.
  Transport time limits now count game ticks instead of polls, so a burst
  of polls inside one tick no longer ends a crossing early, and a slow
  poll that misses several ticks no longer extends it.

### Scripting

- The script API documentation now states that moving the bot by hand during a
  fight ends `api.combat.fight` as `aborted` with reason `user-input`. The host
  has settled it that way since 0.2.4; the declarations and pages still
  described an `interrupted` result.

- A saved setting now takes effect only after its save has reached the disk.
  Before, the panel and the terminal app applied a checkbox at once and kept it
  even when the save then failed. This covers capture input and the 50 fps
  switches, the walk and danger permissions, the manual-walk pause, the map-bake
  choice, the Quest Paths folder and switch, the session log file, and
  "Don't show again" on the notices. In the panel it also covers the remembered
  server revision, the Game pane's none/GPU/CPU raster and its highmem/lowmem
  mode: the revision a session binds with, the saved raster and memory mode and
  the slot's GPU/CPU latch change only when the save lands, and the control
  shows your choice meanwhile. While a save is pending the control shows your
  choice and "Saving…". A save that fails puts the control back to the saved
  value and shows the reason beside it, for example "Not saved: …". The reason
  stays until you try again, or for 15 seconds. A failed "Don't show again" no
  longer disappears silently: the dialog stays open and shows why. Closing and
  reopening a settings window, or the dialog, shows the same state. A change
  saved by another running copy of the program reaches every control, each time
  it saves. The Game pane's raster and memory controls follow it while no bot
  is focused; with a bot focused they show that bot's own saved modes. When
  focus ends (for example you remove the last focused bot), they show the saved
  value at once. The server revision follows it while no session is bound. A
  control you are typing in, or whose own save is still pending, keeps your
  entry until its save ends. A pending raster or memory choice, with its
  "Saving…" and its reason, belongs to the bot you made it for: when you focus
  another bot, that bot's controls show its own values. Layout, colour, order,
  fold and remembered-folder changes still show at once and go back if the save
  fails; in the panel each shows "Saving…" and the reason at its own control
  (the palette, Fleet columns and toolbar fold, section and category order,
  folds, the rail and Grid preview folds, file dialogs and the Debug tab).
  Saves of one setting never mix up which one failed, also when several end at
  once, and when several raster or memory choices end at once the newest one
  takes effect. The session log file reports whether the file could open or
  keep writing, not only that you turned it on; the terminal app's Logs line
  follows it whoever turned it on, and a failure there leaves after 15 seconds.
  In the terminal app's settings popup, a selected setting whose reason is
  taller than the popup (a long path on an 80x24 terminal) says that more is
  below, and Up and Down scroll through it, also when the notices leave it
  only two lines. Notices under the rows that do not fit say that more are
  above or below, and PgUp and PgDn scroll through them, so a failed dismiss
  can be read to its end on an 80x24 terminal. A setting the save queue
  refuses outright (it is full) is logged as a refused command. A failed
  automatic save of the Load order shows
  in the Fleet window (panel) and on the message line (terminal app), which
  also says "Load order: saving…" while it is written. A failed save is final:
  nothing saves it again by itself, also not after the other program releases
  the preferences file; only a new change of that setting starts a new save.
  Before, the panel's Browse window saved the category order again in each
  frame after a failure, so its category chips and the General config's
  Global section said "Saving…" without end, and the order was written later
  without a new action. "Don't show again" in the panel's render-all warning
  now renders every wall member only when the acknowledgement is saved; if
  the save fails, **only render focused** stays on.

## [0.2.7] — 2026-10-26 — Beta 2

### Panel and terminal app

- **Log out** now stops a bot whose login keeps failing. A bot that cannot log
  in (the server is down or refuses it) retries with a growing wait of up to 60
  seconds. In 0.2.6 the panel's **Log out** button was greyed out for such a bot
  and Fleet **Log out all** said "not logged in", so only quitting stopped it.
  Log out is now offered for a bot that is logging in, waiting to retry or
  queued, in the panel and the terminal app, for one bot and for Fleet. It parks
  the bot logged out: no new login attempt starts after you press it. A bot that
  never logged in still has nothing to log out of, and the terminal app's `u`
  now says so instead of marking it logged out. **Log out marked** now skips a
  bot whose login error is waiting for you to act; in 0.2.6 it logged that bot
  out.
- Saving shared preferences no longer waits for disk on the panel or terminal
  app. Accepted saves continue when you close a settings window. A slow save no
  longer erases an unsaved checkbox or folder entry during refresh. Each saved
  change writes one log line that names the changed settings without private
  text. Quitting during initial preference loading now finishes accepted saves
  within the shutdown limit or reports their cancellation.
- Script paint in the panel now follows rs2b0t's layout. The paint has a title
  bar with the title in the script's accent colour, 16-pixel rows, and rs2b0t's
  colours. Pause and Stop are compact buttons side by side. Only the small
  square at the right end of the title bar collapses the paint. The built-in
  Gatherer and Sherlock scripts now show the same Pause and Stop buttons on
  their paint. With input capture on, a click on the paint no longer also clicks
  the game under it.
- With input capture on, the bot that receives a mouse press in the game view
  also receives its release. This still holds if the pointer leaves the
  **Game Image**. A focus change releases the held button from the bot that
  received the press.
- The terminal app can now add a profile. Start `tui-play` on an empty vault to
  open a **New profile** form. Type the username and the account's own game
  password. Press Enter to save. The app rejects an empty password and saves
  nothing. Press `N` in the Fleet pane to add another profile.
  `tui-play --user NAME` selects only a profile already in the vault. It does
  not create a profile with a random password. An unknown name stops with a
  message that names how to add it. If the vault does not exist,
  `tui-play --user NAME` stops before it asks for a passphrase or creates a
  vault.
- The panel button and the terminal app's Fleet button now read **Log out**, as
  the panel's Fleet toolbar already did.
- The panel's profile form also refuses to create a profile with an empty
  password, with the message `credentials: password required`, and saves
  nothing.
- Stopping scripts now clears the Fleet label of every bot whose script was
  walking. Before, a bot that was walking when its script stopped kept the label
  `running` in the panel and the terminal app. Its Script section and the Fleet
  window's script column said `idle`. This was in 0.2.5 and 0.2.6. A walk you
  ordered yourself with **WalkTo** still shows `running` after a script Stop,
  because Stop does not end it. The Fleet label, the status filters,
  **Mark by status** and the Stop button's list of bots now read the same facts
  about each bot. The resource line now says `9 bots (9 in game)` instead of
  `9 bots (9 running)`: the number counts the bots that are in the game, not
  their scripts.
- When the app refuses a command, the session log now records one info line with
  the command, the bots and the reason. The panel and the terminal app write the
  same line. This covers a refusal by the command itself, for example **Stop**
  on bots that have no script running. It also covers a refusal the app makes
  before it sends the command. This includes **Copy settings** with no bot
  focused and **Refresh catalog** with no catalog configured. It also includes
  **Load** on a path that is not a file and a profile import account that cannot
  be written. In the terminal app it also covers a key, palette entry or menu
  item whose command cannot run now, and a Guardian setting changed while no bot
  is focused. A terminal-app key, palette entry or menu item that only opens a
  list or changes the view shows its message and records nothing. An example is
  **W** with no fleet row under the cursor or on a server with one world.
  Before, the message showed on screen and the log had nothing, or the line had
  no reason or no bots.
- The panel now greys out a button it would refuse and shows the reason in a
  tooltip. A greyed-out button cannot be clicked and records nothing. This
  covers **Create vault** while the passphrase is empty or only spaces, and
  **Unlock vault** while it is empty. It also covers the Local-profile debug
  buttons **TutSkip**, **Lumbridge**, the **Teles** destinations and **maxme**.
  The Debug window's **Send** and **Confirm send** are greyed out while no
  focused bot is in a play session. For a send to marked bots, they are greyed
  out while no bot is marked. The Loadouts item slots and **Add supply** are
  greyed out while there is no loadout. Before, a click on them did nothing
  (**maxme** only wrote a status line inside the Debug window).
- A message on the panel's red line or the terminal app's message line now
  clears by itself after 15 seconds, or sooner when you fix the cause. An action
  that fixes the problem clears it at once. In the terminal app, some messages
  come from a key, palette entry or menu item whose command cannot run now. Such
  a message clears as soon as the reason it names is gone. For example, focusing
  a bot clears "Log in: no bot selected", and picking a script clears "Script
  parameters: pick a script first". The one exception is "no message", which
  only a newer message replaces. The terminal app's "is not loaded" message
  clears when that bot is logged in. Focusing a bot clears "no focused profile"
  and the Guardian settings message. In the panel, focusing a bot clears "focus
  a bot first", and a finished save clears "profile is still saving". A newer
  message replaces it and starts its own 15 seconds. The same text shown again
  without a new refusal keeps its first 15 seconds. Before, a message stayed
  until you did something else that happened to clear it.
- Hovering the Fleet's result line (for example "9 bots stopped") now shows its
  text in a normal tooltip. It was a thin column one letter wide and as tall as
  the window.
- After a failed login, the panel's top line now keeps the login error in red
  for 15 seconds. It stays while the bot tries again, waits in the login queue
  or is logged out in that time. The bot's current state shows under the error.
  Before, the next state replaced the error almost at once, and the error came
  back when the next attempt failed. A successful login clears the error at
  once. A newer failed login replaces it and starts the 15 seconds again, also
  when its words are the same. The status rows of the panel and the terminal app
  show this error in red in place of the `last error` row. After the 15 seconds
  the `last error` row comes back, as before.
- Right after you unlock the vault, and when you load bots one at a time, each
  bot now takes its place in your saved profile order at once. Before, such a
  bot could appear in the wrong place until the next refresh.
- After a bot logs out and in again, the chat lines from its earlier session no
  longer stay in view until a new message arrives.

### Host

- The session log warning for a long background work step
  (`background work slice overrun`) now names the kind of work that ran long,
  for example profile preparation. The warning only names the work. The step
  still takes the same time.

### Random events

- The Guardian now handles a held random event when a running script that asked
  to handle that event itself stops ignoring it, is paused or stopped, or fails.
  A later change by a running script can return the same event to that script.
  Before, a Strange box could remain unhandled and interrupt every walk. This
  also affected 0.2.6.
- Strange box handling now retries a missing Open response or answer result. It
  uses game ticks and the game's messages and inventory changes. Time while a
  script owns the box does not count against the time the Guardian waits. After
  bounded retries, it reports the last failure and releases the script instead
  of waiting forever. A fresh message that the box replicated starts another
  bounded set of attempts. This also affected 0.2.6.

### Scripts

- The FlaxRunner, GnomeMagicChopper and HerbloreSecondaries problems listed in
  0.2.5 and 0.2.6 are fixed in rs2b0t itself. Update your rs2b0t folder to get
  the fixes.

- `Prayer.clear()` no longer repeats a prayer click before the game confirms it.
  Its result now counts the clicks it sends.

### Survival

- Combat and protected walks use a held antipoison dose or Strange fruit before
  a health bite when poison makes a cure necessary. Walks that only eat also
  drink a held cure before their next bite.
- When a bot is poisoned, fighting and walk protection count the next poison hit
  before they decide to eat. A bot at full hitpoints no longer eats food that
  would heal nothing.
- A new profile setting group, **Survival and supplies**, holds **Cure poison**:
  **When needed** (the default), **Always** or **Never**.

### Documentation

- New guides explain the panel window, working with many bots, and the settings.
  A reference page lists every Fleet action.
- The built-in scripts guide now explains how to follow a Quester run in the Log
  window and how to load your own quest guides. Writing a quest guide has its
  own page for script authors.

### Fixed during development

- Startup waits for saved preferences before it chooses and loads a profile.
  Saving a new profile during that wait keeps the terminal app's original boot
  load and login. Renaming the boot profile keeps its pending load attached to
  the same profile.
- `tui-play --user NAME` stops on a missing vault before it prepares the game
  cache or reads the engine login key. The message is the same. A local server
  with no engine directory still stops with the engine-directory error.
- Saving the first profile now waits for saved preferences before it loads or
  logs in. The panel and terminal app use their existing startup gates.
- Selecting a bot before preferences finish loading now keeps that bot focused.
  Startup restoration no longer changes a manual command's target during
  preference adoption. This applies to the panel and terminal app. Fleet
  **Log in** also keeps the bot it selects. Removing or deleting a pending
  restore target prevents startup from restoring it. An early terminal selection
  cancels startup focus, but keeps the boot load and login.
- A terminal-app preference error now clears after 15 seconds without returning
  on the next refresh. Recovery clears that error without erasing a newer
  message.

#### Survival

- Strange fruit waits for the game's food delay after a bite. Eating the fruit
  also delays the next combat attack.

#### Profile settings

- When you save Survival or script settings, the session log shows each
  setting's value before and after the saved change. A failed save adds no line.
  An edit replaced before it is saved also adds no line.
- The Cure poison labels now limit **When needed** to combat and protected walks
  and do not claim a poison status row.

### Not in this release

- Native bank sorting (`sortBank`), quest-junk discovery (`findQuestJunk`), pack
  purging at a bank (`purgePackAtBank`) and coin withdrawal (`withdrawCoins`)
  are **Planned**, not included here. These calls still report `not impl`;
  moving existing banking into the native seat does not implement them. 0.2.4 to
  0.2.6 have the same problem.
- Caller-supplied bank-cost routers (`nearestWalkableBank`,
  `nearestWalkableBankAsync`, `bankCostForFinder`, `bankCostForNavigator`) and
  random bank pacing (`bankPace`, `bankPaceTicks`) remain deliberately
  unavailable. The host does not recreate the foreign router or random pacing.
  0.2.4 to 0.2.6 have the same problem.
- The JiveKBD hunting script (King Black Dragon) does not yet bring back Dragon
  bones reliably. In long fights it can run out of food before the dragon dies
  and return to the bank without the bones. A fix is planned with the native
  hunting rewrite. 0.2.4 to 0.2.6 have the same problem.
- When Grinch banks, it uses the shared bank trip, which still waits up to a
  fixed time for some bank steps. These waits are planned to become game-timed.
  0.2.5 to 0.2.6 have the same problem.
- A bot can't get through a door that another player closes again on every tick,
  when that player's moves are handled before the bot's. The bot tries the door
  until its retries run out and then stops at the door. Finding another way
  around, changing worlds when allowed, and a clear "door held shut" result are
  planned for a later release. 0.2.5 to 0.2.6 have the same problem.
- Sherlock was meant to be a native clue solver. In practice it runs on the clue
  machine ported for rs2b0t clue scripts, so it is available only in builds that
  include the JS script runtime (every release build does). A native clue solver
  is planned. 0.1.8 to 0.2.6 have the same problem.
- The northern Desert Mining Camp mine stays hidden. Its minecart needs dialogue
  and a chance-gated crossing that the bot does not yet support. There are still
  85 hidden gathering sites on revision 289 and 66 on 274. 0.2.5 to 0.2.6 have
  the same problem.
- After a super antipoison the bot counts only the ordinary immunity time (120
  game ticks instead of 570), so it may drink again sooner than needed. It is
  still immune for the real time. Counting the super immunity is planned for a
  later release.
- Cure poison does not interrupt Gatherer or Quester steps to drink a held
  antipoison when poison makes a cure necessary. This behavior is not included.
- The panel and terminal app do not show a status row for a poison-related cure
  need. A poison-need status row is not included.

### Known issues

- **Panel (Linux):** on a desktop with two graphics chips (for example a Ryzen
  with built-in graphics plus a separate graphics card), the panel can close
  right after it starts with "importing the supplied dmabufs failed". Start it
  with `WGPU_POWER_PREF=high ./panel-play` until this is fixed. 0.2.0.2 to 0.2.6
  have the same problem.
- **Navigation:** With the default Danger routing setting, **When survivable**,
  bots can refuse walks and bank trips through dangerous monsters instead of
  checking whether the account can survive the trip. For example, a bot at Rock
  Crabs can report that no bank is reachable instead of walking to Seers'
  Village. **Workaround:** set Nav → Danger routing to **Always** (allow danger
  zones). This permits dangerous routes. Use it only when the account can handle
  the monsters on the route. 0.2.0 to 0.2.6 have the same problem.
- **Quester:** The Quester has routes for 20 of the 68 quests. Routes for the
  other quests are planned in waves. 0.2.0 to 0.2.6 have the same problem.
- **Scripts:** The Brimhaven Agility Arena script can stop at a plank or pillar
  it cannot use. The cause is under investigation. 0.2.0 to 0.2.6 have the same
  problem.
- **Banking:** `Bank.depositAllMatching` checks about one item per game tick, so
  large deposits can be slow. A fix is planned for a later release. 0.2.4 to
  0.2.6 have the same problem.
- **Scripts:** The rs2b0t Thiever script waits about three game ticks longer
  than the game requires after each stun, so it gains Thieving XP more slowly
  than it could. A fix is planned with the native thieving rework. 0.2.4 to
  0.2.6 have the same problem.
- **Gatherer:** The lower Desert Mining Camp mine supports Power mode only and
  requires The Tourist Trap complete. Bank mode refuses this site because the
  bot cannot remove the slave kit to leave the camp. 0.2.5 to 0.2.6 have the
  same problem.
- **Scripts:** With "Pause script on manual movement" off, a script that acts in
  place can continue after you move the bot yourself. This affects rs2b0t
  scripts without a Gather or Combat session and built-in scripts other than
  Quester fights. With the setting on, any running rs2b0t script pauses. A
  built-in script pauses while it walks or acts, but not between actions. 0.2.6
  has the same problem.
- **Scripts:** When a script walks from Karamja to the mainland without the 30
  coins for the boat (`Traversal.walkTo`), the host earns the fare by working
  for Luthas. If the bot cannot reach Luthas once after filling the banana
  crate, the walk returns `false` instead of trying again from where the bot
  stands, so the fare is not paid and the bot does not cross. 0.2.6 has the same
  problem.
- **Panel:** If you use **Copy settings** for a built-in card and rename one of
  the target bots in **Profiles** before you press **Confirm**, the copy to that
  bot fails with `settings copy cancelled: bot or vault unavailable`.
  **Workaround:** rename the bot first, or run **Copy settings** again after the
  rename. Found by code reading; not yet seen in a running app. 0.2.0 to 0.2.6
  have the same problem.
- **Scripts:** The rs2b0t FireGiant script can keep fighting after it eats its
  last piece of food instead of leaving to bank, and it can attack a giant other
  than the one hitting it. The bot can die in the fire giant room with an empty
  pack. Watch the bot's food and stop the script when it runs low until the
  script is fixed upstream. 0.2.6 has the same problem.

## [0.2.6] — 2026-10-23 — Beta 2

### Panel and terminal app

- Switching the log view from **all bots** to **bot** now shows only the focused
  bot's lines in the panel and terminal app.
- After you use **Assign** in the Fleet toolbar, **Start** runs the card shown
  in that bot's **Script** section.
- Animations keep moving while you view another bot or after a slow frame. This
  includes player and NPC movement, action poses, map effects, projectiles, and
  cutscenes.
- Script settings no longer leave **Start** blocked after a failed or cancelled
  save. Failed saves restore the saved value and show the error by the field.
  You can retry the save. The panel names the card and field of hidden unsaved
  edits and offers **Discard unsaved settings**. **Script prefs** labels wrap.
  Number fields show their valid ranges. The form shows an error before it
  closes when a value is invalid. The panel and terminal app wait for settings
  saves before **Start**. Each successful save adds a log entry with the card,
  field, old and new values, and action.
- Loading a bot profile no longer stalls while 274bot prepares its quest data.

### Scripts

- With **Pause script on manual movement** on, moving the bot yourself pauses
  running rs2b0t scripts. It also pauses a built-in script while it acts in
  place, such as fighting or gathering. **Resume** continues the script.
- **Examine** and **Cancel** from a game-world menu no longer count as manual
  movement.
- Grinch makes one return attempt if its walk back to the Start tile stalls.
  A second stall stops the script and keeps the failure detail.
- Catalog scripts with many imports, including JiveKQ, can finish loading when
  the host is busy. **Start** still stops with a timeout if a script never
  finishes loading.

### Documentation

- New guides help you start 274bot, set up an account, and choose a server and
  world. A glossary explains common terms.
- New guides explain how to use scripts and the built-in cards.

### Fixed during development

#### Panel and terminal app

- Apply to all adds one save entry to the log for each bot.
- The panel clears a script-setting error after you discard the edit or fix
  the field.

### Not in this release

- Native bank sorting (`sortBank`), quest-junk discovery (`findQuestJunk`), pack
  purging at a bank (`purgePackAtBank`) and coin withdrawal (`withdrawCoins`)
  are **Planned**, not included here. These calls still report `not impl`;
  moving existing banking into the native seat does not implement them. 0.2.4 to
  0.2.5 have the same problem.
- Caller-supplied bank-cost routers (`nearestWalkableBank`,
  `nearestWalkableBankAsync`, `bankCostForFinder`, `bankCostForNavigator`) and
  random bank pacing (`bankPace`, `bankPaceTicks`) remain deliberately
  unavailable. The host does not recreate the foreign router or random pacing.
  0.2.4 to 0.2.5 have the same problem.
- The JiveKBD hunting script (King Black Dragon) does not yet bring back Dragon
  bones reliably. In long fights it can run out of food before the dragon dies
  and return to the bank without the bones. A fix is planned with the native
  hunting rewrite. 0.2.4 to 0.2.5 have the same problem.
- When Grinch banks, it uses the shared bank trip, which still waits up to a
  fixed time for some bank steps. These waits are planned to become game-timed.
  0.2.5 has the same problem.
- A bot can't get through a door that another player closes again on every tick,
  when that player's moves are handled before the bot's. The bot tries the door
  until its retries run out and then stops at the door. Finding another way
  around, changing worlds when allowed, and a clear "door held shut" result are
  planned for a later release. 0.2.5 has the same problem.
- Sherlock was meant to be a native clue solver. In practice it runs on the clue
  machine ported for rs2b0t clue scripts, so it is available only in builds that
  include the JS script runtime (every release build does). A native clue solver
  is planned. 0.1.8 to 0.2.5 have the same problem.
- The northern Desert Mining Camp mine stays hidden. Its minecart needs dialogue
  and a chance-gated crossing that the bot does not yet support. There are still
  85 hidden gathering sites on revision 289 and 66 on 274. 0.2.5 has the same
  problem.

### Known issues

- **Panel (Linux):** on a desktop with two graphics chips (for example a Ryzen
  with built-in graphics plus a separate graphics card), the panel can close
  right after it starts with "importing the supplied dmabufs failed". Start it
  with `WGPU_POWER_PREF=high ./panel-play` until this is fixed. 0.2.0.2 to 0.2.5
  have the same problem.
- **Navigation:** With the default Danger routing setting, **When survivable**,
  bots can refuse walks and bank trips through dangerous monsters instead of
  checking whether the account can survive the trip. For example, a bot at Rock
  Crabs can report that no bank is reachable instead of walking to Seers'
  Village. **Workaround:** set Nav → Danger routing to **Always** (allow danger
  zones). This permits dangerous routes. Use it only when the account can handle
  the monsters on the route. 0.2.0 to 0.2.5 have the same problem.
- **Quester:** The Quester has routes for 20 of the 68 quests. Routes for the
  other quests are planned in waves. 0.2.0 to 0.2.5 have the same problem.
- **Scripts:** The Brimhaven Agility Arena script can stop at a plank or pillar
  it cannot use. The cause is under investigation. 0.2.0 to 0.2.5 have the same
  problem.
- **Banking:** `Bank.depositAllMatching` checks about one item per game tick, so
  large deposits can be slow. A fix is planned for a later release. 0.2.4 to
  0.2.5 have the same problem.
- **Scripts:** The rs2b0t Thiever script waits about three game ticks longer
  than the game requires after each stun, so it gains Thieving XP more slowly
  than it could. A fix is planned with the native thieving rework. 0.2.4 to
  0.2.5 have the same problem.
- **Gatherer:** The lower Desert Mining Camp mine supports Power mode only and
  requires The Tourist Trap complete. Bank mode refuses this site because the
  bot cannot remove the slave kit to leave the camp. 0.2.5 has the same problem.
- **Scripts:** With "Pause script on manual movement" off, a script that acts in
  place can continue after you move the bot yourself. This affects rs2b0t
  scripts without a Gather or Combat session and built-in scripts other than
  Quester fights. With the setting on, any running rs2b0t script pauses. A
  built-in script pauses while it walks or acts, but not between actions.
- **Scripts:** In the FlaxRunner script, the Spinner banking step does not check
  whether the walk to the Seers' Village bank or the bank opening succeeded
  before it tries to deposit. If either step fails once, the deposit does
  nothing for that trip and the trip counter still increases. The script retries
  on the next loop, so work continues, but a trip can be wasted. The fix belongs
  to the upstream script. 0.2.5 has the same problem.
- **Scripts:** The rs2b0t HerbloreSecondaries script does not eat while it
  walks, and its eat rule refuses a lobster above 5 hitpoints because the
  12-point heal does not fit a low-Hitpoints account. On a low-level account the
  red spiders' eggs trip through Edgeville dungeon can kill the bot with food
  still in its pack. Run this script on an account with mid-level combat stats
  (the script's own tests use level 40) until the script is fixed upstream.
  0.2.5 has the same problem.
- **Scripts:** The rs2b0t GnomeMagicChopper script looks for the Gnome
  Stronghold bank's north staircase in the wrong place, a few tiles from the
  real stairs. If its first walk to the bank fails and it switches to the north
  staircase, that bank trip can fail. The south staircase works, and the script
  usually recovers on a later trip. If a fletching bank trip gets stuck, move
  the bot beside the south staircase and restart the script. The fix belongs to
  the upstream script. 0.2.5 has the same problem.
- **Panel:** If WalkTo is open while you change Fleet marks, WalkTo keeps the
  earlier selection until you click **Walk N bots**. **Workaround:** after
  changing Fleet marks, click **Walk N bots**, or close and reopen WalkTo.

## [0.2.5] — 2026-10-22 — Beta 2

### Scripts

- Scripts may pass an item ID to `Bank.withdrawLoad`.
- `Bank.withdrawLoad` now returns false when the bank has no matching item,
  even when the pack is full. Release 0.2.4 returned true.
- Script paint: scripts that use the standard paint-controls helper now show
  working Pause/Resume and Stop buttons. Resume remains available while
  paused. The panel and TUI use the same controls.
- Script paint buttons in the panel now use the same text size as the
  paint rows on scaled displays. Release 0.2.4 drew the button labels
  too large at display scaling above 100 %.
- Starting a script now stops at once with a clear message when the saved
  loadout list it needs is empty, or when a script file it depends on is
  missing. Other start problems are still retried.
- Ground-item interactions keep the item type the script selected. Items
  with the same name on one tile, such as Halloween masks, no longer substitute
  for the selected type. Requests without an item type still find the item by
  name and tile. Native collection, hunt keys, clue key-keeper drops and
  trail-end rewards all keep the selected item type.
- A pick-up of an exact ground item is confirmed within four later game ticks,
  from what the pack and the ground show. If the bot loses the race for an
  item, it does not avoid that item's tile afterwards. Built-in scripts can
  report confirmed progress and wait for what they need without switching off
  the checks that keep the script and its connection healthy.
- A built-in script's ground-item selection holds at most 32 item types. A
  longer list is refused as a whole, so no item can skip the script's list of
  excluded items.
- Grinch is a built-in Gathering script that collects selected visible holiday
  items around its Start tile. All 18 base choices default on. Set a radius
  from 1 through 16 tiles, use the nearest reachable or a named bank, and
  deposit only selected items before returning to the exact Start tile.
  Empty waits stay visible as Waiting; duplicate refusals skip only that
  item until the next Start. Saved settings apply on the next Start.
- Grinch uses the bot's real position, not the position drawn on screen, for
  its Start tile and for knowing it has returned. A confirmed walking arrival
  no longer fails while the drawn player catches up.
- When a built-in script stops with an error, the TUI status pane keeps the
  reason visible after the run ends. This includes Grinch stopping with a full
  pack that has no selected item to deposit.

### Banking

- Banking now opens a closed chest before it tries the bank action. This
  lets ClueSolver open the Duel Arena reward bank.

### Quester

- If you move the bot yourself during a Quester fight, the Quester now stops
  that fight step, as it already does for its other steps. With "Pause script
  on manual movement" off, the Quester stops with "cancelled by user input"
  and does not fight on. With the setting on (the default), the script pauses
  instead; when you resume it, the Quester starts the fight step again.

### Panel

- On Linux, the panel now follows the system's choice of graphics chip
  instead of preferring built-in graphics. This can help on desktops with
  two graphics chips. If "importing the supplied dmabufs failed" still
  appears, start it with `WGPU_POWER_PREF=high ./panel-play`.
- The panel and the TUI now prepare, run and judge the built-in live checks
  with the same code. Only screenshot capture and TUI output still differ.
- Every panel action that changes something now runs the same way as in the
  TUI. This includes Start, the draw switches, the Nav config grants, session
  log, map bake, Quest Paths, marks, the filter, the vault and Quit. Each
  action reports its result.
- Fleet Assign & restart reports its spaced-out Starts until they finish,
  after the assignment is saved.
- Saved draw preferences (capture input, sidecar and focused 50 fps), the
  render-all and background-bots acknowledgements, the map-bake choice and
  the session log file are written only when you change them. Saving the
  panel's own window state no longer rewrites them.
- When one frame of mouse and keyboard input for the game view overflows the
  input queue, the panel now reports it instead of dropping events silently.

### Fleet

- The panel's Fleet table, MultiBox rail and Grid, and the TUI fleet table,
  use one filter, sort and set of marks. The sort offers Load order (the
  default), Name and Status. Profiles that are not loaded are listed as "not
  loaded"; the rail lists them in a folded "Not loaded" group where you can
  mark them. A profile keeps its mark when the filter hides it or when it is
  unloaded, until the profile is deleted. The TUI Map's group send uses the
  same marks.
- Load order is saved. Reorder bots by dragging MultiBox tiles or Fleet rows
  in the panel, with `Alt+Up` / `Alt+Down` in the TUI, or with **Move marked
  to top** (the TUI palette also has **Move profile up** and **Move profile
  down**). Name and Status remain temporary view sorts. Load all logs in bots
  in that order, a new profile goes to the end, a renamed profile keeps its
  place and removing a bot from the wall keeps its place. The order is saved
  with your other preferences; a reorder that cannot be saved is reported and
  the last saved order comes back.
- Choose each bot's own card, the focused bot's card, or a pinned card (the
  panel's card picker; `c` in the TUI). Choosing a card does not start a
  script or change a bot's assignment.
- Fleet actions name their scope. With bots marked, an action covers every
  marked profile, hidden marks included; with nothing marked, it covers the
  loaded bots the filter shows and asks first. Interruption and
  settings-copy prompts name their targets, the panel and the TUI show the
  same prompt, and each action keeps one result until it settles. A plan is
  frozen when it is made: a bot marked, loaded or made ready later does not
  join it, and a bot that changed is skipped with its new reason. Stop on a
  bot whose Start is still waiting cancels that Start, and the Start
  reports it cancelled. The panel's mark box and the TUI's `a` / `A` mark
  and unmark the shown rows, and both mark the shown rows by status. In
  the TUI, mark profiles that are not loaded before logging them in.
- The TUI Script pane's bulk buttons are now **Start fleet** (`T`) and
  **Stop fleet** (`E`) and run the fleet action.
- Switch a bot's world: click its world marker on a MultiBox tile, Grid
  cell or Fleet row and pick a public world, or press `W` on a TUI fleet
  row. To move several bots, mark them and press **Switch world** in the
  fleet toolbar or choose **Switch world of marked…** in the TUI
  palette. Fleet asks first and names the bots. Each bot saves its new
  world, logs out and logs back in through the login queue; a running
  script continues after it reconnects, and a paused script stays paused.
  A bot that is queued, logging in, not logged in or already on that world
  is skipped with the reason, and a bot whose world could not be saved
  stays logged in. A server with one world offers no switch. An action
  that a later command takes over (Log in, Log out, Relog now, another
  Switch world, Remove) reads "cancelled" in the result and names that
  command.
- Panel: the MultiBox rail now contains the Fleet card picker, marks,
  filter, sort and all Fleet actions: Start, Stop, Assign, Assign &
  restart, Log in, Log out, Walk, Switch world and Copy settings to. The
  Fleet window keeps its table and shows the same state; clicking a bot's
  name there focuses it. Grid and single-bot layouts show the actions in
  the side panel's **fleet** section. The rail's Login all, Logout all,
  Start all and Stop all buttons are gone: with nothing marked, the fleet
  buttons act on every loaded bot shown, after asking first.
- Panel: fold the Fleet toolbar to leave more room for bot previews.
  Pending questions and action results remain visible, and the bot
  previews scroll under the toolbar.
- TUI: the fleet header shows the active card and order. Use `c` to choose
  a card, `s` to change sort, `a` / `A` to mark or unmark shown rows, and
  `W` to choose a world; a click on the header's card or sort does the
  same. The palette offers the same Fleet actions.
- Walk with nothing marked (the fleet toolbar's Walk, WalkTo's **Group**
  and the TUI Map's `g`) marks only the loaded bots the filter shows, and
  of those only the bots that can walk. A bot the filter hides is not
  added.
- Copy settings to more than 64 bots now saves every bot. The copy saves a
  few dozen bots at a time and the rest wait their turn; Starts wait until
  the last bot is saved. Cancel stops the bots that are still waiting.
- Fixed: with MultiBox off, the bot that unlocking logs in (the last
  focused profile), or a bot picked in Profiles, is now a fleet member.
  The Fleet window lists it as loaded, and a script can be started on
  it. Before, Start refused it with `no slot: <name>`. Starting a script
  on a profile that is not loaded now says `<name> is not loaded`. 0.2.4
  has the same problem.

### Random events

- Random-event handling starts lost-tool recovery after it observes tool removal or a handle in inventory or equipment.
- Shared axe and pick swing messages no longer start recovery.
- The Guardian no longer gives up fleeing a hostile random event while a
  thieving stun stops the bot from moving. Release 0.2.4 counted the
  stunned ticks as failed fleeing and handed control back within the
  event NPC's reach.

### TUI

- Marks, the fleet filter, log saving, the session log file, the Nav and
  danger-routing settings, map bake, Quest Paths, vault unlock and Quit now
  run the same way as in the panel.

### Map baking

- Resuming an interrupted local map bake on Windows now repairs a damaged
  tile instead of failing with "Access is denied".
### Gatherer

- At every fishing site, the Gatherer keeps its preferred standing spot
  when that spot is reachable. Otherwise, it chooses a reachable alternative.
  This applies to the initial approach and the return after banking. It fixes
  the Catherby failure from the Fishing Guild when the preferred spot is on
  the Water Obelisk island. A bot already on that island can still use it.
- Gatherers at Draynor no longer stop Blocked beside the jail, where guards
  killed the idle bots. Resource selection allows danger zones when either
  the card's Allow danger zones setting or the global Nav setting allows them.
  Resources remain eligible only if they have a safe access tile, and walks
  use only safe access tiles. The same rule applies to finite resource steps
  in the Quester. If no safe resource remains, the failure names the danger
  zone. Without navigation data, bots can work or wait at a resource already
  in range, but cannot walk to another resource.

### Navigation

- Walks can now go through content-gated doors and entrances the game locks
  behind an item: carry the needed key (or wear the needed gear) and the
  route goes through; without it the walk stops and names what is missing.
  Some entrances instead need you to travel light — carrying or wearing a
  listed item stops the route until it is left behind. The bot never moves,
  equips or bank-deposits your gear to pass such a gate; the refusal says
  what to change. Quest-locked crossings use verified quest-list completion;
  a missing or stale journal observation does not grant access.
- The Gatherer now offers both Grand Tree mine sites, the Desert Mining
  Camp's surface and lower mine, and Taverley Dungeon's lava eels. Their
  routes check the quest, keys and worn slave kit that the content requires.
- The bundled navigation data has a new format. The app rebuilds it
  automatically. If you use navigation data that you baked yourself, bake it
  again before use with `cargo run -p nav --bin nav-pack`.

- The built-in Elkoy and rune-essence navigation checks start beside the
  transport NPC and seed only their required quest. They no longer start
  transport checks while unrelated quest rewards still run.

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
- The panel no longer stops during startup with "Background work is
  unavailable" after one minute.

#### Profiles

- Deleting a profile no longer leaves the result of the delete waiting
  forever; it now reports when the delete is saved.

#### Navigation

- A long route search no longer uses up to about 1.5 MB more memory than
  in 0.2.4 on some runs.

#### Banking

- When bank selection cannot find a route, the session log now gives the
  routing error, start tile and number of candidate banks and access tiles.

#### Quester

- Fixed five Path errors. Demon Slayer now waits for Silverlight's delayed
  handoff. Pirate's Treasure estimates its rum and apron at their full-stock
  prices of 30 gp and 2 gp; reading its message scroll from the quest journal
  still does not work. Sea Slug no longer reacquires Swamp paste
  after Holgart takes it. Prince Ali Rescue clears only nearby guards before
  talking to Keli. Gertrude's Cat includes the fence landing tile and walks
  its yard waypoints in order. Monk's Friend skips the ring-stone walk near
  the ladder or inside the cave.
- Demon Slayer: the Quester no longer stops at Delrith's incantation menu; it
  chooses the phrase Gypsy Aris teaches.
- Totem of Ikov records the 30 gp Brimhaven passage fare, and quest-guide
  test runs may start at a chosen spot such as a bank instead of the guide's
  own start point.
- When a script's first step toward an object on a wall (Interact or Use on)
  finds no path, it retries once from the wall tile. The retry carries over if
  the bot runs out of actions that tick; other walk failures still end the
  step.

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

- The live checks' wait limits and the tick count in their results now count
  game ticks the bot observed, not every refresh of its game state. Repeated
  steps send at most once per game tick.
- Game ticks are now counted only when the server's per-tick player update
  arrives; other changes to player data no longer inflate the tick counts in
  live-check results.
- The Gatherer lifecycle check accepts planned restarts only at the measured
  ten-bot size. Other sizes fail with a message that the restart limit was not measured for that size.
  An observation that ends during a planned Stop records the qualified
  stopped state instead of an unfinished qualification.

#### Gatherer

- Fixed: the Gatherer searches distance 0 through 7 from two observed tiles.
  The first is the last player tile before the event. The second is the tile
  from the first snapshot that shows the tool change or event message. This
  covers events that run before or after movement in the same tick. Distance
  zero is valid because the tile search returns its input coordinate when no
  ring tile qualifies.
  When adjacent inventory snapshots identify one removed tool, the Gatherer
  selects that tool's exact head. Otherwise, it selects only a new head of
  the same kind. It waits three observed game ticks for a delayed head.
  It still refits the tool after pickup. It returns to the bank if no head
  appears in that time, if one later tick confirms a missing head, or after
  the ground head's 200-tick lifetime. A full pack with nothing to drop
  returns to the bank at once.

- The Eye of newt scenario check now allows the measured banking cycle at
  the normal game speed, with 50% headroom. Its old three-minute deadline
  stopped the check during the return from Draynor.
- The Duel Arena clue check now recognises all obstacle-fight pens and their
  boundary tiles. A valid duel no longer misses the handshake witness when
  its players start at the east edge or in another obstacle pen.
- The FireGiant bank check now prepares a level-40 Rune kit and eight Lobsters,
  with time for the full trip at normal game speed. It still requires earned
  loot in the bank, a return to the dungeon, and further combat.

#### Script paint

- The Pause and Stop buttons stay visible when a script's paint rows fill
  the chat area. Before this fix, a script with many rows could push Stop
  out of view.

### Not in this release

- Native bank sorting (`sortBank`), quest-junk discovery (`findQuestJunk`), pack
  purging at a bank (`purgePackAtBank`) and coin withdrawal (`withdrawCoins`)
  are **Planned**, not included here. These calls still report `not impl`;
  moving existing banking into the native seat does not implement them. 0.2.4
  has the same problem.
- Caller-supplied bank-cost routers (`nearestWalkableBank`,
  `nearestWalkableBankAsync`, `bankCostForFinder`, `bankCostForNavigator`) and
  random bank pacing (`bankPace`, `bankPaceTicks`) remain deliberately
  unavailable. The host does not recreate the foreign router or random pacing.
  0.2.4 has the same problem.
- The JiveKBD hunting script (King Black Dragon) does not yet bring back Dragon
  bones reliably. In long fights it can run out of food before the dragon dies
  and return to the bank without the bones. A fix is planned with the native
  hunting rewrite. 0.2.4 has the same problem.
- When Grinch banks, it uses the shared bank trip, which still waits up to a
  fixed time for some bank steps. These waits are planned to become game-timed.
- A bot can't get through a door that another player closes again on every tick,
  when that player's moves are handled before the bot's. The bot tries the door
  until its retries run out and then stops at the door. Finding another way
  around, changing worlds when allowed, and a clear "door held shut" result are
  planned for 0.2.6.
- Sherlock was meant to be a native clue solver. In practice it runs on the clue
  machine ported for rs2b0t clue scripts, so it is available only in builds that
  include the JS script runtime (every release build does). A native clue solver
  is planned. 0.1.8 to 0.2.4 have the same problem.
- The northern Desert Mining Camp mine stays hidden. Its minecart needs dialogue
  and a chance-gated crossing that the bot does not yet support. There are still
  85 hidden gathering sites on revision 289 and 66 on 274.

### Known issues

- **Panel (Linux):** on a desktop with two graphics chips (for example a Ryzen
  with built-in graphics plus a separate graphics card), the panel can close
  right after it starts with "importing the supplied dmabufs failed". Start it
  with `WGPU_POWER_PREF=high ./panel-play` until this is fixed. 0.2.0.2 to 0.2.4
  have the same problem.
- **Navigation:** With the default Danger routing setting, **When survivable**,
  bots can refuse walks and bank trips through dangerous monsters instead of
  checking whether the account can survive the trip. For example, a bot at Rock
  Crabs can report that no bank is reachable instead of walking to Seers'
  Village. **Workaround:** set Nav → Danger routing to **Always** (allow danger
  zones). This permits dangerous routes. Use it only when the account can handle
  the monsters on the route. 0.2.0 to 0.2.4 have the same problem.
- **Quester:** The Quester has routes for 20 of the 68 quests. Routes for the
  other quests are planned in waves. 0.2.0 to 0.2.4 have the same problem.
- **Scripts:** The Brimhaven Agility Arena script can stop at a plank or pillar
  it cannot use. The cause is under investigation. 0.2.0 to 0.2.4 have the same
  problem.
- **Banking:** `Bank.depositAllMatching` checks about one item per game tick, so
  large deposits can be slow. A fix is planned for 0.2.6. 0.2.4 has the same
  problem.
- **Scripts:** The rs2b0t Thiever script waits about three game ticks longer
  than the game requires after each stun, so it gains Thieving XP more slowly
  than it could. A fix is planned with the native thieving rework. 0.2.4 has the
  same problem.
- **Gatherer:** The lower Desert Mining Camp mine supports Power mode only and
  requires The Tourist Trap complete. Bank mode refuses this site because the
  bot cannot remove the slave kit to leave the camp.
- **Scripts:** A script notices that you moved the bot yourself only while it is
  walking, or while an rs2b0t script runs a Gather or Combat session. If you
  move the bot while it fights or gathers in place, "Pause script on manual
  movement" does not pause the script, the script does not stop, and it carries
  on. A Quester fight notices your movement only when the fight began with a
  walk to its spot. 0.2.0 to 0.2.4 have the same problem.
- **Scripts:** In the FlaxRunner script, the Spinner banking step does not check
  whether the walk to the Seers' Village bank or the bank opening succeeded
  before it tries to deposit. If either step fails once, the deposit does
  nothing for that trip and the trip counter still increases. The script retries
  on the next loop, so work continues, but a trip can be wasted. The fix belongs
  to the upstream script.
- **Scripts:** The rs2b0t HerbloreSecondaries script does not eat while it
  walks, and its eat rule refuses a lobster above 5 hitpoints because the
  12-point heal does not fit a low-Hitpoints account. On a low-level account the
  red spiders' eggs trip through Edgeville dungeon can kill the bot with food
  still in its pack. Run this script on an account with mid-level combat stats
  (the script's own tests use level 40) until the script is fixed upstream.
- **Scripts:** The rs2b0t GnomeMagicChopper script looks for the Gnome
  Stronghold bank's north staircase in the wrong place, a few tiles from the
  real stairs. If its first walk to the bank fails and it switches to the north
  staircase, that bank trip can fail. The south staircase works, and the script
  usually recovers on a later trip. If a fletching bank trip gets stuck, move
  the bot beside the south staircase and restart the script. The fix belongs to
  the upstream script.

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
  occasionally die. Every hit, even a miss, also closes an open shop or bank
  window. Reacting to other attackers is planned for later combat work; if a
  bot dies, its log around the death helps us look into it.
- In the random-event Policies control, Fight and Ignore can be chosen but
  aren't available yet; a saved Fight or Ignore is treated as Flee. Running
  from an angry Strange plant doesn't cure its poison.
- Faster interactions aren't here: acting on the nearest thing first, not
  walking through the thing you're acting on, shorter follow-ups when using an
  item on something next to you, and reaching a clue's usable side in one walk.
- The Gatherer gets no closest-site option, no shop tool buying, and no tool
  provisioning while a quest route is holding the inventory. Finding locked
  areas and matching gather timing are unchanged. More Gatherer work is
  planned for a later release.
- Clues that need a key you don't carry, locked chests, and Kharazi and
  Tirannwn coverage stay incomplete, and clue conversations still go one step
  at a time. The wider clue backlog is planned for a later release.
- The WalkTo town list is unchanged, and map redraw work is planned for a
  later release.

**Scripts and compatibility**

- Some catalog scripts are dimmed in the script picker and can't be started:
  AIOQuester and Barcrawl (the built-in Quester replaces them, and Barcrawl
  is planned for it); GatheringBot's Woodcutter, Miner and Fisher, and
  EssMiner (the built-in Gatherer replaces them); MarketMaker and BankSorter
  (both planned as built-in features); ArravSupplier and RoguesPurse (they use
  rs2b0t quest helpers the bot doesn't provide yet); JiveKQ (its four-player
  setup isn't qualified yet); and one BrimhavenAgility version that retries a
  broken plank instead of taking another route.
- Scripts that use these rs2b0t helpers still refuse to start, or stop with
  a `not impl` message naming the helper when they reach it:
  - Walking and NPCs: `Traversal.remaining`, `requestRepath`, `walkApproach`,
    `gotoNpc`, `talkChoosingBy`, the walk destination list, and the
    nearest-walkable-bank and walk-cost selectors.
  - Banking: `sortBank`, `findQuestJunk`, `coinsToWithdraw`,
    `purgePackAtBank`, `withdrawCoins`, `withdrawTo`, `bankPace` and
    `bankPaceTicks`.
  - Inventory: `backpackSnapshots`, `backpackCapacity` and
    `acquireKeepNames`.
  - Gathering: the woodcutting, mining and fishing location lists and
    pickers, tool and fishing-gear buying and repair plans, and the checks
    for walking back to a gathering spot.
  - Shops and market: `Shop.buyById`, `buildCatalog`, `searchCatalog`,
    `resolveByName` and `resetLiveCatalog`.
  - Combat and spells: `Fight.hasChaseTarget`, `Game.castOnNpc`, and the
    `Input` methods that use an item on something or cast a spell
    (`useItemOnLoc`, `useItemOnNpc`, `useItemOnObj`, `useItemOnItem`,
    `castOnNpc`, `castOnLoc`, `castOnItem`).
  - Chat and objects: `ChatDialog.makeOne`, `Input.continueDialog`,
    `EntityQuery.inside` and `nearestPreferLocal`, and `GameMessages.record`,
    `reset`, `firstSince` and `recent`. `GameMessages.mark`, `since` and
    `sawSince` read the chat the bot has seen, not a message list the script
    can change.
  - Paint: building a `PaintFrame` directly, `PaintState` and `paintState`,
    the frame layout helpers (`cols`, `wrap`, `list`, `fill`, `grid`,
    `stepper`) and the hit-testing helpers.
  - Client access: snapshot invalidation, attach and detach, packet
    listeners, object-catalog reset, object model sizes, readiness checks,
    retaliate and modal-button reads, the combat-showing threshold and
    `WELCOME_SCREEN`.
  - Script runtime: settings catalogs, `settingOptionLabel`, run-policy and
    runner lifecycle helpers, `BotHostImpl`, script events (`events.on`/`off`,
    `registerScript`), the task classes (`AcquireTask`, `BranchTask`,
    `LeafTask`), `resolveLoopCadence` and `stableClosedPoll`. rs2b0t's timing
    isn't carried over.
  - Quests, clues and duels: `Quests.journal`, the quest engine, the murder
    and suspect tables, clue-visit locations, quest coin and bank rules,
    `trailWalkOpts`, `extraItemShop`, `gateItemShop`, `ClueDuelSearch` and
    `resetClueDuelSearch`.

  Some other helpers refuse only a request they can't handle, with the same
  message and the reason.
- Scripts written against rs2b0t releases newer than the one this release
  was checked against can use calls the bot doesn't know yet: they fail to
  start, or stop when they reach the call.
- The clearer Start messages don't yet separate "can't do that right now"
  from "not implemented" everywhere in scripts.
- FlourCollector, the Zanaris cook, JiveDemons and JiveKBD stay unqualified,
  and the stronger pair-trade checks don't qualify every pair setup.
- A Duel script that remembers the wrong opponent can still wait in a Duel
  Arena fight pen until the fight moves on. Correct opponent tracking is
  planned for a later release.

**Panel, TUI, memory and host**

- The rs2b0t import doesn't bring over rs2b0t's profile tabs and groups,
  its account-wide run, bank and walking settings (auto-run, run energy,
  bank junk, the Zanaris and Mage Arena banks, teleports, route drawing),
  loadouts or order books. A selected script that's dimmed in 274bot (such
  as Woodcutter, which the built-in Gatherer replaces) or missing from your
  catalog isn't selected. The preview lists each of these with the reason.
- No quick world switch from the world bubble, no Fleet bulk world switch
  with script pause and resume, no game-window resizing, no script-window
  auto-sizing, and no custom profile ordering. These controls are planned for
  a later release.
- Grid cells don't each paint their script, the terminal shows no game
  pixels, GPU sampling stays approximate, and the pickpocket wall-clip fix
  isn't in this release. Rendering work is planned for a later release.
- The Mac memory readout still shows RSS rather than physical footprint, and
  turning render-all off can leave counted-but-reclaimable memory behind; the
  capacity warning doesn't change that accounting. Readout improvements are
  planned for a later release.
- Waking bots together and using fewer connection resources per bot are
  planned for later host work; this release brings no broader CPU or memory
  optimization.
- Copying settings onto a profile with no bot can still save without
  assigning when a later copy fails or is cancelled, and there is no saved
  quest-route knowledge yet. Saved-state work is planned for a later release.
- Full audit logging of operator actions, frame-stall diagnostics and
  lock-order checking aren't here; the Gatherer and watchdog messages are
  specific fixes, not that project. Region-load lines still appear under the
  login category.

**Testing and supported versions**

- CowKiller, EdgevilleMonkeyBars, JiveMarketDumper and ShopRunner have no
  live test scenario, so their behaviour on a live server is untested.
- 289 is the supported server version; 274 stays best-effort and unqualified.

### Known issues

- **Panel (Linux):** on a desktop with two graphics chips (for example a
  Ryzen with built-in graphics plus a separate graphics card), the panel can
  close right after it starts with "importing the supplied dmabufs failed".
  Start it with `WGPU_POWER_PREF=high ./panel-play` until this is fixed.
  0.2.0.2, 0.2.1, 0.2.2 and 0.2.3 have the same problem.

### Fixed during development

- The random-event Guardian remembers where an attacker first appeared,
  before its delayed attack starts. It keeps fleeing if a Tree spirit still
  targets the bot after the first escape.
- The escape route goes far enough to let an attacker stop chasing and leave.
  A visible attacker normally prevents the bot from returning too soon.
- When an attacker leaves view, the bot continues toward its escape target.
  If the bot stops beyond the attacker's reach, it returns after about
  13 seconds without seeing the attacker, even if the route stopped short.
- A failed escape no longer prevents the script from eating for several
  minutes while an attacker keeps hitting a boxed-in bot. After the bot
  stops trying, the same attacker cannot immediately pause the script again.
- Ordinary monsters with random-event names no longer trigger event handling.
  A former attacker no longer hides a newly detected event.
- The Guardian uses visible player targeting to avoid claiming another
  player's nearby Ent. Before that target is visible, a nearby Ent can still
  be mistaken for this bot's event. All Ent waits end after 60 game ticks.
- Without selected game content, the status still shows an identified event
  attacker, but the Guardian does not guess a flee distance or pause the script.

## [0.2.2] — 2026-10-13 — Beta 1 patch

### Scripts

- The current rs2b0t Thiever loads and banks again. The other bank scripts
  that open a bank the same way (Alcher, GemCutter, HerbCleaner,
  LeatherCrafter, PotionMaker and Superheater) load again too; they failed
  to start with "does not provide an export named 'openBankAccess'".
- Scripts that finish on their own, such as BankFletcher when the bank runs
  out of logs, now end cleanly with their reason in the log, shown as
  information rather than an error, instead of failing when they try to
  finish.
- Catalog combat scripts that group or customize their Loadout setting
  (ArdyFighter, ChaosDruidKiller, FireGiant, GreenDragon, HillGiant,
  MossGiant and RockCrab) now show a working Loadout picker in the panel and
  TUI and check your saved loadout before Start, instead of showing an
  uneditable `unsupported type` row or starting and failing when no loadouts
  exist.
- Opening a shop (rs2b0t `Shop.open`, used for JiveKQ's Shantay pass and by
  other shop scripts) now sends one Trade click and waits for the measured
  walk to the shopkeeper plus a margin before clicking again. Each wait is
  capped at about 70 seconds. Before, it clicked again after 3 seconds, and
  on a longer walk that second click could close the shop as it opened, so
  the purchase failed. When the bot can't measure the walk, it uses the full
  wait. A shopkeeper behind a closed door is walked to first. A shop that
  doesn't open still gets up to three Trade clicks, and "I can't reach
  that!" ends a click at once. Quester shop purchases follow the same rule,
  and their buy steps no longer give up after 10 seconds.

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
- The bot doesn't yet react when something attacks it during non-combat
  work. The Gatherer doesn't flee or eat on its own when, for example, a
  fishing random event attacks it, so a bot on a very long session can
  occasionally die. Every hit, even a miss, also closes an open shop or bank
  window. Reacting to attackers is planned for later combat work; if a bot
  dies, its log around the death helps us look into it.
- Faster interactions aren't here: acting on the nearest thing first, not
  walking through the thing you're acting on, shorter follow-ups when using an
  item on something next to you, and reaching a clue's usable side in one walk.
- The Gatherer gets no closest-site option, no shop tool buying, and no tool
  provisioning while a quest route is holding the inventory. Finding locked
  areas and matching gather timing are unchanged. More Gatherer work is
  planned for a later release.
- Clues that need a key you don't carry, locked chests, and Kharazi and
  Tirannwn coverage stay incomplete, and clue conversations still go one step
  at a time. The wider clue backlog is planned for a later release.
- The WalkTo town list is unchanged, and map redraw work is planned for a
  later release.

**Scripts and compatibility**

- Some catalog scripts are dimmed in the script picker and can't be started:
  AIOQuester and Barcrawl (the built-in Quester replaces them, and Barcrawl
  is planned for it); GatheringBot's Woodcutter, Miner and Fisher, and
  EssMiner (the built-in Gatherer replaces them); MarketMaker and BankSorter
  (both planned as built-in features); ArravSupplier and RoguesPurse (they use
  rs2b0t quest helpers the bot doesn't provide yet); JiveKQ (its four-player
  setup isn't qualified yet); and one BrimhavenAgility version that retries a
  broken plank instead of taking another route.
- JiveDemons, JiveDragons and JiveKBD can't start yet: they use rs2b0t
  helpers the bot doesn't provide yet. They're planned for the next release.
- Scripts that call these helpers the bot doesn't provide yet still refuse to
  start, or stop when they reach the helper:
  - Walking: `Traversal.remaining` and `requestRepath`.
  - Banking: `shouldBankNow`, `sortBank`, `findQuestJunk` and `coinsToWithdraw`.
  - Shops and trading: `Shop.buyById` and `countOfferMatching`.
  - Tools: `toolAttackLevel` and `toolsNeedingEquip`.
  - Chat, game and objects: `ChatDialog.makeOne`, `Game.castOnNpc`,
    `EntityQuery.inside`, `nearestPreferLocal` and `resolveRockIds`.
  - Inventory items: the `InvItem` methods and `acquireKeepNames`.
- Scripts written against rs2b0t releases newer than the one this release
  was checked against can use calls the bot doesn't know yet: they fail to
  start, or stop when they reach the call. Catching up with current rs2b0t is
  planned for the next release.
- The clearer Start messages don't yet separate "can't do that right now"
  from "not implemented" everywhere in scripts.
- FlourCollector, the Zanaris cook and the blocked Jive cards stay
  unqualified, and the stronger pair-trade checks don't qualify every pair
  setup.
- A Duel script that remembers the wrong opponent can still wait in a Duel
  Arena fight pen until the fight moves on. Correct opponent tracking is
  planned for a later release.

**Panel, TUI, memory and host**

- No quick world switch from the world bubble, no Fleet bulk world switch
  with script pause and resume, no game-window resizing, no script-window
  auto-sizing, and no custom profile ordering. These controls are planned for
  a later release.
- Grid cells don't each paint their script, the terminal shows no game
  pixels, GPU sampling stays approximate, and the pickpocket wall-clip fix
  isn't in this release. Rendering work is planned for a later release.
- The Mac memory readout still shows RSS rather than physical footprint, and
  turning render-all off can leave counted-but-reclaimable memory behind; the
  capacity warning doesn't change that accounting. Readout improvements are
  planned for a later release.
- Waking bots together and using fewer connection resources per bot are
  planned for later host work; this release brings no broader CPU or memory
  optimization.
- Copying settings onto a profile with no bot can still save without
  assigning when a later copy fails or is cancelled, and there is no saved
  quest-route knowledge yet. Saved-state work is planned for a later release.
- Full audit logging of operator actions, frame-stall diagnostics and
  lock-order checking aren't here; the Gatherer and watchdog messages are
  specific fixes, not that project. Region-load lines still appear under the
  login category.

**Testing and supported versions**

- 23 catalog scripts have no live test scenario, so their behaviour on a live
  server is untested.
- 289 is the supported server version; 274 stays best-effort and unqualified.

### Known issues

- **Panel (Linux):** on a desktop with two graphics chips (for example a
  Ryzen with built-in graphics plus a separate graphics card), the panel can
  close right after it starts with "importing the supplied dmabufs failed".
  Start it with `WGPU_POWER_PREF=high ./panel-play` until this is fixed.
  0.2.0.2, 0.2.1 and 0.2.2 have the same problem.

## [0.2.1] — 2026-10-13 — Beta 1 hotfix

### Gatherer

- Fishing bots now look for another spot when a spot moves or temporarily
  disappears, including spots outside their current view. A short resource wait
  running out no longer stops fishing with "resource-unavailable". If no spot
  appears for about 8 minutes without a catch or XP, the bot stops with a clear failure.
  While looking for a spot, it reports surveying rather than gathering.
- Bank trips no longer stop the bot with "bank deposit failed: bank closed
  before the transfer settled" or "bank item table unavailable". The bot's
  automatic run toggle could press the run button while the bank window was
  open, and the game's run button closes the open bank or dialogue window,
  so the bank shut under the deposit. The run button now waits until no bank
  or dialogue window is open. If the bank still closes mid-transfer, the bot
  opens it again from the same booth or banker and finishes what it was
  doing, counting what was already deposited, with up to two extra opens for
  each deposit or withdrawal; if the bank keeps closing past that, the
  transfer still stops the bot with the message above.

### Not in this release

- The Gatherer still doesn't react when something attacks it while it
  gathers (for example a fishing random event), and it doesn't flee or eat on
  its own, so a bot on a very long session can occasionally die. We're
  looking into it; if it happens to you, the bot's log around the death helps.

### Known issues

- **Panel (Linux):** on a desktop with two graphics chips (for example a
  Ryzen with built-in graphics plus a separate graphics card), the panel can
  close right after it starts with "importing the supplied dmabufs failed".
  Start it with `WGPU_POWER_PREF=high ./panel-play` until this is fixed.
  0.2.0.2 has the same problem.

## [0.2.0.2] — 2026-10-12 — Beta 1 patch

### Scripts

- When an rs2b0t script walks to a spot "within N tiles", the bot now heads for
  that spot itself, as it does in rs2b0t, and stops once it's within range.
  Before, it stopped at the nearest tile in range, so two bots meeting at the
  same spot could end up on opposite sides, too far apart to trade. If the
  spot itself can't be reached, the bot still walks to the nearest tile in
  range.
- The current rs2b0t Duel Arena script loads again: the duel API offers the
  challenge-area check that newer catalogs use.
- When a catalog script needs a newer rs2b0t API than this version supports,
  Start now says so and names what's missing, instead of showing a raw script
  error.
- When a bot's assigned script isn't in the loaded catalog, Start now says
  which catalog folder was searched.
- When the game server stops sending updates, the watchdog now says "no game
  packets" instead of reporting that the script is hung.
- A script waiting inside a Duel Arena fight pen is no longer restarted over and
  over trying to walk back to the lobby (the pen's walls make that impossible);
  the watchdog waits until the bot leaves the pen.
- Quest location, production and reach checks use the game's reported player
  tile, so a difference in displayed position no longer makes the script miss
  nearby objectives.
- Quester loadouts, BankBudget trips and Entrana clue recovery no longer send
  a separate Close before Wear; on 289 the engine closes the bank and equips
  the item on that same interaction.

### Gatherer

- After a bank trip or a death, a resource cooldown no longer stops the
  bot with "return failed". It waits until the spot can be used again, then
  walks back and keeps gathering.
- When a fishing hazard appears, the bot steps to a nearby tile it can
  actually reach from where it stands. If every nearby tile is blocked
  and the hazard is temporary, it waits instead of stopping with "walk
  failed".
- Blocked, waiting and resource-unavailable log lines name the method, work
  area, the bot's tile when it planned the return, and the rejection reason
  or deadline, so one line is enough to diagnose a report.
- A watchdog restart names what the Gatherer was waiting for, and every
  pending observation wait shows that reason on the status line.
- Bank trips on a slow computer or a laggy server no longer fail with "bank
  transfer did not settle" or "bank unavailable". Opening, depositing,
  withdrawing and closing each wait for a set number of game ticks and at
  least the matching time before giving up, and a bank that doesn't open or
  close is tried again a few times. Teller banks (such as the Mage Arena)
  get time to finish their conversation before the bank opens. The
  Quester's bank visits get the same wait.

### Random-event guardian

- Lamp auto now uses every genie lamp you hold, not just the first: each one
  gives XP in the skill you picked, without turning lamp auto off and on.
- If a lamp's skill window doesn't open, the bot pauses briefly and tries
  again instead of stopping until you toggle lamp auto.

### Host

- When this computer can't keep up with the bots it's running, the panel, the
  TUI and the log say so: how many bots are falling behind, by how much, and
  whether render-all is on, with a suggestion to run fewer bots or turn
  render-all off. The warning clears once the computer has kept up for 30
  seconds. A slow script, a slow network or a quiet game server doesn't
  trigger it. On macOS and Windows a few other stalls (such as memory being
  paged back in) can also count, and a bot that's idle between game updates
  may be missed.

### Login

- Logging back in right after logging out no longer waits a minute when the
  same world is still saving your character. The bot waits 1.2 seconds before
  its first retry, then tries every 5 seconds while the server reports the
  account in use, for at most 30 seconds from the clean exit. This release
  wait keeps its queue place while eligible bots proceed. Its device cooldown
  also does not hold up bots on other device IDs; bots sharing that account's
  device ID still wait it out. Repeating Log in or Load all keeps this release
  wait and the bot's queue place.
- Loading many bots logs them in one at a time, a little over a second
  apart, focused bot first. A login that gets no answer gives up after 30
  seconds instead of holding up the others.
- The first bot in the login queue shows what it is waiting for and for how
  long. A failed connection says where it failed. Its retries start at about
  5 seconds and grow to about a minute, with some variation between bots.
- "World full" names the world and when the bot tries next; on Auto the bot
  moves on to another world. After you log a bot out, its next login is a
  fresh one; a dropped connection still reconnects.
- When the server sends a bot something the client can't read, the bot now
  logs in again from the start instead of reconnecting, so it comes back to
  a complete world. This fixes the 0.2.0.1 known issue where such a bot came
  back to an empty world until you logged it out and in. The server may still
  be holding the old session, so this login can take about a minute. A
  dropped connection, or a server that stops answering for 15 seconds, still
  reconnects into the running session and keeps its world.
- The log line for that exit now records the packet's size, the packets just
  before it and how far into the session it came, to help find the cause.

### Panel

- Load all leaves the bot you're watching in view. If none is selected, it
  shows one of the loaded bots. Only profiles with auto-login log in; the
  others stay at the title screen.
- The Fleet window is tidier. Its button labels no longer end in "…", its
  table columns share the window's width instead of scrolling sideways, its
  buttons and column checkboxes wrap on a narrow window, and "Sort:" is a
  full-width drop-down.
- Grid now shows the resource readout (bots, CPU, RAM, traffic) in the
  right-hand panel, where the rail's card isn't on screen.
- The rail's mark checkbox says "mark for Fleet actions" on servers that don't
  allow Debug (the public worlds), and still mentions Debug actions where it
  exists.
- The "Render all wall members?" warning has a "Don't show again" button,
  enabled once "I understand" is ticked. It does what OK does and remembers
  the choice, so unchecking "only render selected" later applies at once. OK
  alone still asks again next launch, and "only render selected" still starts
  on at every launch.
- "sidecar 50 fps" and "focused 50 fps" are remembered between launches. A
  bot that isn't drawing doesn't run at 50 fps because of them.
- WalkTo's Send window opens on screen and is sized to its list of bots, and
  the "Walk n bots" count follows the marked bots from the rail, Grid and
  Fleet while WalkTo is open.
- The Status card shows what the selected bot's script is doing and why it
  waits: its phase, and the reason when the script is parked. A Quester that
  finds no step for its stage parks and shows `no step for stage`. A bot that
  runs no script shows no script rows; a script refusal still showing after
  its run stopped keeps its rows.

### TUI

- Load+login all with no bot selected now selects one of the loaded bots.
- The status pane shows the script's phase and park reason on the same rows
  as the panel. Those rows appear only while a script runs or a script
  refusal is still showing.

### Walking

- When a one-time walk overrides danger routing and the bot's poison status is
  unknown, the panel, TUI and log now say "Poison status is unknown until the
  bot can track it" instead of a paragraph about internal planning.

### Connection

- A bot on a WebSocket world no longer disconnects and reconnects when its
  connection is briefly congested.

### Not in this release

Planned work that didn't make this release, plus options and script calls
that visibly refuse. Each item says where it's headed.

**Quests and combat**

- Quest guides cover 25 of 69 quests; the other 44 aren't here, and Haunted
  Mine stays unavailable on the target server. The guides that ship are still
  beta rather than all checked end to end. More quest coverage is planned for
  a later release.
- The Quester doesn't yet pick quest-aware supplies or pick up every quest
  where you left off: earned quest items, some mid-conversation resumes,
  following up on some quest targets, and coin costs in quest requirements
  aren't handled. Quest resume and recipe work is planned for a later
  release.
- Prayer flicking, safespots, luring around corners, attacking other players,
  range-aware standing spots, automatic gear switching and face-tank fallbacks
  aren't here; settings the game marks unsupported still refuse to start, and
  fights without progress still use the existing time limits. Combat tactics
  are planned for later combat work.
- Hunt keeps its two-minute fight limit and retreat behavior; it hasn't moved
  to the full combat system. Hunt parity is planned for later combat work.

**Walking, gathering and clues**

- Walks into quest areas that unlock as a quest progresses, regional
  entrances and transports such as boats and gliders, the land route over
  White Wolf Mountain below level 51, and some traveller and cosmetic walks
  aren't fixed by this release's walk-target change. Fuller survivable
  routing is navigation work planned for a later release.
- Gathering sites the route finder can't reach yet stay hidden: 90 on 289 —
  the Morytania, elven, island and boat-or-glider, gated and underground
  groups listed under 0.2.0.1 below. A saved Gatherer card naming one still
  refuses to start; pick another site. They come back as the route finder
  learns the entrances, which is navigation work planned for a later
  release.
- Faster interactions aren't here: acting on the nearest thing first, not
  walking through the thing you're acting on, shorter follow-ups when using
  an item on something next to you, and reaching a clue's usable side in one
  walk.
- The Gatherer gets no closest-site option, no shop tool buying, and no tool
  provisioning while a quest Path owns the inventory; finding locked areas
  and matching gather timing stay as they are. More Gatherer work is planned
  for a later release.
- Clues that need a key you don't carry, locked chests, and Kharazi and
  Tirannwn coverage stay incomplete, and clue conversations still go one step
  at a time. The Wear change doesn't close the wider clue backlog.
- The WalkTo town list is unchanged, and map redraw work is deferred to a
  later release.

**Scripts and compatibility**

- This release doesn't re-check the whole script catalog against newer
  rs2b0t: the Duel challenge-area check and the clearer "needs a newer API"
  message are the two specific changes. Broader compatibility work is planned
  for a later release.
- Scripts using newer rs2b0t calls the bot doesn't implement yet still refuse
  to start, including `Game.castOnNpc`, `Shop.buyById`, `ChatDialog.makeOne`,
  `Traversal.remaining` and `requestRepath`, `EntityQuery.inside` and
  `nearestPreferLocal`, script registration and events, `InvItem`, the
  acquire/branch/leaf task classes, and the gathering, fishing, mining,
  woodcutting, banking and tool data and helper families. The planned fixes
  for `shouldBankNow`, `toolAttackLevel` and `toolsNeedingEquip` haven't
  landed either. Compatibility work continues in a later release.
- The clearer Start messages don't yet separate "can't do that right now"
  from "not implemented" everywhere in scripts.
- FlourCollector, the Zanaris cook and the blocked Jive cards stay
  unqualified, and the stronger pair-trade checks don't qualify every pair
  setup.
- The watchdog no longer restarts a bot waiting in a Duel Arena fight pen,
  but a Duel script that remembered the wrong opponent can still wait there
  until the fight moves on. Correct opponent tracking is catalog work planned
  for a later release.

**Panel, TUI, memory and host**

- No quick world switch from the world bubble, no Fleet bulk world switch
  with script pause and resume, no game-window resizing, no script-window
  auto-sizing, and no custom profile ordering. These controls are planned for
  a later release.
- Grid cells don't each paint their script, the terminal shows no game
  pixels, GPU sampling stays approximate, and the pickpocket wall-clip fix
  stays out. Rendering work is planned for a later release.
- The Mac memory readout still shows RSS rather than physical footprint, and
  turning render-all off can leave counted-but-reclaimable memory behind; the
  new capacity warning doesn't change that accounting. Readout improvements
  are planned for a later release.
- Waking bots together and using fewer connection resources per bot stay
  parked for later host work; this release brings no broader CPU or memory
  optimization batch.
- Copying settings onto a profile with no bot can still save without
  assigning when a later copy fails or is cancelled, and there is no saved
  quest-route knowledge yet. Saved-state work is planned for a later
  release.
- Full operator-action audit logging, frame-stall diagnostics and lock-order
  checking aren't here; the new Gatherer and watchdog messages are specific
  fixes, not that project. Region-load lines still appear under the login
  category.

**Testing and supported versions**

- This release doesn't establish the full nightly live-testing lane (several
  green nights, every quest stage, full tick ledger); the targeted live
  checks behind these fixes aren't that lane.
- 289 is the supported server version; 274 stays best-effort and
  unqualified.

### Known issues

The first item is new in this release. The rest are unchanged since
0.2.0.1: found while testing that release, not changed by this one. Fixes are
planned for a later release. (The 0.2.0.1 empty-world reconnect issue is fixed
instead — see Login above.)

- **Memory (Linux):** an idle bot uses a little more memory than in 0.2.0.1:
  about 3 MB more for the whole program (the Linux crash fix now prepares the
  script engine at startup) and about 0.2 MiB more per bot on large fleets
  (memory the system allocator keeps free after logins). Bots doing work use
  no more than before. A fix is planned for a later release.

- **Panel:** with MultiBox on, saving a new profile that has auto-login
  enabled logs the bot in but doesn't add it to the wall, so it runs without
  a tile. Click the new profile in the Profiles list to bring it onto the
  wall; nothing is lost.
- **Gatherer:** a bank trip keeps every copy of the tool in your inventory,
  not just one, so a second harpoon (or axe, pickaxe or net) stays in your
  inventory and takes a slot. Bank the extra by hand; the bot leaves it in
  the bank.
- **Scripts:** `api.gather` can stop with "walk failed" when the next
  resource it picks has no spot to stand at that it can reach, instead of
  picking another one. Start it again on a different work area.
- **Quester:** Sheep Shearer can stop at a closed door on the way to the
  spinning wheel. In a quest queue, the next quest then starts and Sheep
  Shearer is left unfinished.
- **Quester:** Imp Catcher can hand in the beads and give you the reward, but
  keep waiting on the conversation instead of marking the quest done. Check
  the quest journal; the quest itself is complete.
- **Quester:** if you stop a quest in the middle of one of its conversations
  (for example the bartender's beer in Vampire Slayer) and press Start again
  while the choice is still open, the quest stops at that choice. Close the
  conversation (walk a step) before pressing Start.
- **Combat:** if a fight is started against a target that can't be attacked,
  the bot stops as it should, but on very low HP its walk away can stall next
  to the target.

### Fixed during development

Fixes to problems that appeared and were fixed during 0.2.0.2 development.
They are listed here for completeness and are left out of the release notes.

- Scripts: while open-bank Wear was being built, Wear's own bank close looked
  like a reason to send it again, so a Wear could go out twice. Wear now
  stays latched on the inventory tab and the worn set through the close, and
  completes when the item is worn.
- Scripts: while the new bank waits were being built, a teller bank's
  conversation could be cut short by the booth-open deadline. Teller dialogue
  now has its own deadline.
- Login: while the paced login queue was being built, a bot first in line
  could look stuck with no reason while it waited out its device cooldown and
  another device's bot went first. It now reports that device wait.

## [0.2.0.1.1] — 2026-10-11 — Beta 1 Linux patch

This patch is for Linux only. macOS and Windows aren't affected and stay on
0.2.0.1.

### Scripts

- On Linux with recent processors, running several bots no longer crashes the
  whole program when a second bot starts or restarts a script.

## [0.2.0.1] — 2026-10-10 — Beta 1 patch

### Scripts

- **Apply focused bot's settings to marked** no longer overwrites a script you
  explicitly assign while a native-script settings copy is still being prepared,
  even if you edit that card again before the copy finishes. When you apply two
  copies to an unassigned bot, the one you applied last chooses its script once
  it finishes. If that later copy is cancelled or fails, the bot can be left
  with no assigned script; assign it again.
- Each bot's log now records every saved change to its assigned script,
  including the previous and new script and the action that changed it.
  Parameter edits alone do not produce assignment-change messages.
- Saved settings that need a restart show a badge (`[restart]` in the panel,
  `!` in the TUI) and offer Restart or Later. Restart stops only listed bots
  still running the same assigned script with pending settings. Bots that
  stop before Restart reaches them or are logged out, reassigned or running
  a different script are skipped and named in the status line. Later leaves
  runs active and suppresses the prompt until settings change again.
- Each `api.questPaths` row now has a `draft` field: `true` for a Quest Path
  still marked as an untested draft, `false` for a released one, so a script
  can tell the two apart before it relies on a Path.
- In Load scripts, `api.gather.run`'s `allowTeleports`, `allowWilderness`
  and `allowDangerZones` options now apply to the Gatherer session's own
  walks. Before, they were accepted but ignored, so those walks followed
  the Nav config settings. The script's other walks keep their own options.
- rs2b0t-compatible deposits (`depositInventory`, `depositAllMatching`,
  `depositAllExcept`) only deposit items you actually hold. If the bank's
  inventory panel doesn't show a held item within about a second, they now
  fail with "bank deposit view did not post" instead of reporting success
  without depositing it.

### Panel

- The panel no longer freezes, stopping every bot with it, when the WalkTo
  map is open while the selected bot walks a route you sent from that map.
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

### Login

- Running many bots at once no longer fails with "Too many open files". On
  macOS an app opened from Finder was limited to 256 open files, so beyond
  roughly 70 bots, logins failed with "Error connecting to server" and
  scripts wouldn't start. The panel, TUI and `host-play` now raise that
  limit when they start and note the new value in the log.
- After "You have only just left another world", the game view's login
  screen now counts down "Your profile will be transferred in" each second
  instead of staying at its first number.
- rs2b2t World 3 is supported. New installs list it, and an existing World 1
  and World 2 list is extended once when you first start this version, with a
  copy of the old `servers.json` kept beside it. A list you edited is left
  alone, and so is a list you change afterwards, even if you remove World 3.

### Navigation

- Fleet walks no longer leave eligible group members stuck at doors that
  close after each crossing, such as the Fishing Guild door; anyone held
  back retries automatically once the door is available again.
- The first walk of a native script after Start (Gatherer, Quester) no
  longer ignores the account's skills, quests and membership. That used
  to refuse routes through gated doors (the Fishing Guild door needs
  Fishing 68) and fail with "walk failed".

### Combat

- Ranged fights that can't confirm the weapon's combat tab or the requested
  style now stop with a clear preparation-readiness reason after eight
  observed ticks, instead of waiting out the full fight budget.
- Ranged fights count every shot the bot fires. Before, a fight could report
  zero attacks, while arrows were used and Ranged XP rose, whenever another
  player stood on the bot's tile.
- Quester combat kits now carry exactly the items the kit names. Before, an
  item whose name is shared by other items, such as Coins, could resolve to
  the wrong one, so the bot set off with the wrong item or without its coins.

### Gatherer

- The Gatherer no longer walks back and forth between two trees or rocks
  without gathering, which happened most often after a bank trip or a Maze
  random event; it now works the one it reached.
- If a tree or rock disappears or is used up while the Gatherer walks to it,
  it picks another instead of stopping with "walk failed".
- Starting a fishing Gatherer from outside the fishing area (such as
  Ardougne for the Fishing Guild) no longer fails straight away with
  "walk failed". The first walk now uses the nearest stand it can
  actually arrive at, the same way a return from the bank already did.
- Bank trips no longer fail with "bank transfer did not settle" after the
  first or last item goes in. The bot sometimes clicked an item a second time
  because the bank's inventory panel still showed it for a moment after it
  was deposited, then waited for a count that could never drop. It now
  deposits only what is still in your inventory, so everything it should bank
  (including spare tools and random-event items) goes in. The Quester's bank
  visits get the same fix.
- Bank transfers wait up to eight game ticks to complete instead of four
  seconds of real time, so a laggy server or a busy computer no longer makes a
  deposit or withdrawal give up early.
- Raw shark (Harpoon) and the big-net catch are selectable again. The
  monkey-form guard in the fishing scripts refuses only while a greegree is
  worn, so the Gatherer now refuses to start in that case
  (`forbidden-state:monkey-form`) instead of hiding the method.
- Gather sites: underground mines and caves and far-off areas get a place name
  (for example Mining Guild, Karamja Volcano, Zanaris), and single fishing
  spots are offered. A site that needs a level or a worn item shows it in its
  label ("Mining Guild (Mining 60)", "Zanaris (Dramen staff)"). The Zanaris
  entrance needs the Dramen staff worn and a members world. 36 sites are new
  on both server versions. Sites the route finder can't reach yet are hidden
  (see "Not in this release"), so the Gatherer now offers 289 sites on both
  versions: 22 fewer than 0.2.0 on 289 (311) and 19 fewer on 274 (308). Every
  site on offer has a walking route on the 289 map for an account with every
  quest, item and level the way there asks for. The route finder still asks
  for Lost City to be completed before it walks into Zanaris.
- Every 0.2.0 site keeps its id and region, hidden or not, and no id is
  reused, so saved Gatherer cards that name a site you can still pick keep
  working.

### Known issues

Found while testing 0.2.0.1. Each comes from code this release didn't
change, so 0.2.0 behaves the same way; fixes are planned for a later
release.

- **Panel:** with MultiBox on, saving a new profile that has auto-login
  enabled logs the bot in but doesn't add it to the wall, so it runs without
  a tile. Click the new profile in the Profiles list to bring it onto the
  wall; nothing is lost.
- **Gatherer:** a bank trip keeps every copy of the tool in your inventory,
  not just one, so a second harpoon (or axe, pickaxe or net) stays in your
  inventory and takes a slot. Bank the extra by hand; the bot leaves it in
  the bank.
- **Scripts:** `api.gather` can stop with "walk failed" when the next
  resource it picks has no spot to stand at that it can reach, instead of
  picking another one. Start it again on a different work area.
- **Quester:** Sheep Shearer can stop at a closed door on the way to the
  spinning wheel. In a quest queue, the next quest then starts and Sheep
  Shearer is left unfinished.
- **Quester:** Imp Catcher can hand in the beads and give you the reward, but
  keep waiting on the conversation instead of marking the quest done. Check
  the quest journal; the quest itself is complete.
- **Quester:** if you stop a quest in the middle of one of its conversations
  (for example the bartender's beer in Vampire Slayer) and press Start again
  while the choice is still open, the quest stops at that choice. Close the
  conversation (walk a step) before pressing Start.
- **Combat:** if a fight is started against a target that can't be attacked,
  the bot stops as it should, but on very low HP its walk away can stall next
  to the target.
- **Login:** rarely, the server sends a bot something the client can't read
  (seen once in about 40 logins on rs2b2t). The bot reconnects on its own but
  comes back to an empty world: only players and NPCs are drawn, and its
  walks can stall. Log that bot out and back in to restore it. An ordinary
  dropped connection is not affected.

### Not in this release

**Gatherer**

- Gathering sites in areas the route finder can't reach yet are hidden: 90 on
  289 (58 that 0.2.0 offered and 32 new) and 71 on 274 (55 that 0.2.0 offered
  and 16 new). The areas are:
  - Morytania: Canifis, Mort'ton, the Haunted Woods, the Temple, the River
    Salve and the west bank of Mort Myre (Mort'ton east is 289 only).
  - Tirannwn: the Elf Camp, Isafdar, Poison Waste, Tyras Camp, Arandar and the
    Prifddinas outskirts.
  - Islands and boat or glider trips: Crandor, the Fishing Platform, the
    Lighthouse island, and on 289 only Miscellania, Ape Atoll and Crash
    Island.
  - Beyond gates and fences: Gu'Tanoth and the forest on the south side of its
    ogre gates, the Castle Wars west bank, the stretch west of the Tree Gnome
    Stronghold fence, the Wilderness agility course, the Demonic Ruins
    charcoal trees, the Desert Mining Camp, the Hemenster fishing contest
    enclosure, the Fremennik mining enclosure, the Waterfall ledge, West
    Ardougne, and on 289 only Trollheim's north-west forest.
  - Caves and underground areas: the Brimhaven Dungeon, the Taverley Dungeon,
    the Grand Tree mine, the Varrock sewers, the Digsite caverns, Glarial's
    Tomb, the Desert Mining Camp mine, a separate pocket of Zanaris and the
    cave west of the Necromancer's cellar.

  A saved Gatherer card that names one of these now refuses to start with an
  "unknown" site error; pick another site. They return as the route finder
  learns the entrances; that work is planned for 0.2.1. For a few enclosed
  spots no way in has been found yet, so those may stay hidden.

### Fixed during development

Fixes to problems that appeared and were fixed during 0.2.0.1 development.
They are listed here for completeness and are left out of the release notes.

- Scripts: `depositAllExcept` could bank an item it was told to keep when
  the bank's inventory panel opened before it showed your items. Kept items
  now stay in your inventory however the panel fills in.
- Combat: in rapid ranged fights the bot's attack count could fall behind the
  shots it actually fired, and it then clicked Attack again on the monster it
  was already shooting. Every shot is counted again.
- Gatherer: the Bedabin Camp cave site was hidden by mistake as if it were a
  separate cave; it is the Zanaris Cosmic Temple trees and is offered again
  under its own id.
- Gatherer: Zanaris sites no longer show Lost City as a separate requirement;
  the entrance needs only the Dramen staff worn and a members world.
- Gatherer: Crash Island is no longer described as part of the Ape Atoll
  voyage (Waydar's glider flight, not Lumdo's voyage, goes there).

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
