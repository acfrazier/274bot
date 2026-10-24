# The built-in scripts

> **Who this is for:** you want to know what the Quester, Gatherer, Grinch and
> Sherlock do and which settings they have. When you finish, you can choose the
> right script and set it up. This page also explains the fighting that these
> scripts share, which 274bot calls Combat.

To start, pause and stop a script, see [Run scripts on your bots](running-scripts.md).
Look up a word in the [glossary](glossary.md).

## Which script do I need?

You pick these scripts in **Browse…**. Each has a category in the **Scripts**
window.

| Script | Category | Use it to |
| --- | --- | --- |
| **Quester** | Quests | Finish quests with step-by-step guides. |
| **Gatherer** | Gathering | Cut trees, mine rock or fish, and bank what you collect. |
| **Grinch** | Gathering | Pick up holiday items that lie on the ground near the start tile. |
| **Sherlock** | Treasure Trails | Solve clue scrolls. |

Combat is not in the list. You do not start it. See [Combat](#combat).

The settings of each script are in **Script prefs**. The terminal app calls that
window **Parameters**. The setting names below are the names that the window
shows.

## Quester

The Quester follows one guide for each quest. A guide tells it where to walk, who
to talk to, what to fight and which items to take from the bank. The Quester
reads your quest list in the game to see how far each quest has gone.

Twenty quests have a released guide:

- Cook's Assistant, Sheep Shearer, Rune Mysteries and Romeo & Juliet
- Imp Catcher, Vampire Slayer, Doric's Quest and Goblin Diplomacy
- Witch's Potion, Prince Ali Rescue, Pirate's Treasure and Demon Slayer
- The Knight's Sword, Death Plateau, The Tourist Trap and Priest in Peril
- Clock Tower, Monk's Friend, Hazeel Cult and Plague City

The other quests have no guide yet. See the [known issues](../known-issues.toml).

Five more guides are drafts: Druidic Ritual, Gertrude's Cat, Jungle Potion, Sea
Slug Quest and Tribal Totem. A draft is marked `[draft]` in the quest list. While
a draft runs, the **Path status** row reads `Untested draft Path`. The list also
holds Haunted Mine. You cannot tick it, because this server's game content has
no Haunted Mine quest.

## Quester: choose the quests

Open **Script prefs** with the Quester picked. The setting **Quests** shows a
checkbox for each quest.

- With no quest ticked, the Quester runs every released quest, in list order.
- With quests ticked, it runs only those quests.
- **Order override** names quests to run first. **Skip** names quests to leave out.

To run one quest, tick it while the script is stopped, then click **Start**.
The Quester reads its settings at each start.

## Quester: settings

| Setting | Default | What it does |
| --- | --- | --- |
| **Quests** | none ticked | The quests to run. None ticked means all released quests. |
| **Include draft guides when no quests are picked** | off | Adds the draft guides to the run when no quest is ticked. |
| **Order override** | none | Quests that run first. A draft listed here also runs when no quest is ticked. |
| **Skip** | none | Released quests that do not run. |
| **Maximum deaths** | 2 | The Quester stops with a reason when the bot dies more often than this. The count covers all quests of one run. |
| **Partner account**, **Gang**, **Family Crest gauntlets** | empty, empty, `chaos` | Choices for two-account quests and for one enchantment. No guide that ships with 274bot uses them. |
| **Allow teleports**, **Allow wilderness**, **Allow danger zones** | off | Walk permissions for this script. See [Run scripts on your bots](running-scripts.md#change-a-scripts-settings). |

## Quester: what it checks before a quest

Before it starts a quest, the Quester checks the quest list and the quest's
requirements: members world, skill levels, quests you must finish first, quest
points and items. A quest that fails a check is blocked, with the reason. For
example, `quest requires a members world` appears on a free world.

The Quester takes the items a quest needs from your bank. It learns what the bank
holds when the bank is open. A quest that has not started is blocked for a
missing item only after the bank has been open since you logged in.

## Quester: read a run

The **script** section shows rows such as `Quest` and `Queue`. `Quest` names the
quest that runs. `Queue` has one letter for each quest: `D` done, `R` running,
`Y` ready, `P` parked, `B` blocked or `?` not known yet. In the panel, the
**status** section shows what the bot does in its `phase` row. It reads
`walking`, `talking`, `fighting`, `banking`, `waiting` or `working`. The same
section also shows the current step comment.

A quest that fails is parked (`P`) with a reason, and the Quester goes on with the
next quest. Two stops need your action:

| Reason | What to do |
| --- | --- |
| `quest list was not observed within 30 seconds: log out and log in normally outside the tutorial (finish it if needed), then Stop/Start Quester` | Do what the message says. |
| `cancelled by user input` | You moved the bot while the Quester walked or acted, so it stopped. Click **Start**; [Pause, resume and stop](running-scripts.md#pause-resume-and-stop) explains. |

The Log window shows the run in more detail. When a guide starts, the Log names
the guide, where it loaded from, a fingerprint of the exact file, and the first
step's comment. Stage changes, step starts, step skips and step finishes follow.
A skip line names the check that passed. Warnings name failed steps and the
reason a run parked. Repeated retries fold into one line with a count. The trace
keeps the last 32 different events.

If a quest needs items first, the Quester collects them before the quest steps
begin. Those lines look the same as the lines of a collect step. A step start
line appears only when that step truly starts. Items that wait for a later stage
stay out until then. Cook's Assistant talks to the cook first and collects later.

## Quester: run guides from a folder

The Quester can also run quest guides from a folder on your computer. With the
Quester picked, **Script prefs** has a section **Quest Paths (all bots)**. Tick
**Load quest Paths from folder**, enter the folder and click **Reload Paths**.
The setting is off at first. The default folder is `~/.274bot/quester/paths/289`.
274bot does not create it for you.

The guides are for revision 289. New guides appear as drafts. In the terminal
app, press `o` on the Overview screen to find the load checkbox, the folder, and
**Reload Paths**.

Put each guide in `<folder>/<id>.json`. The `id` inside the file must match the
file name. An optional `index.json` lists guides in the order they appear:

```json
{"schema":1,"paths":[{"id":"cook","file":"cook.json"},{"id":"my-draft","file":"my-draft.json"}]}
```

A folder guide with the same name as a shipped guide replaces it. Other folder
guides join the list as drafts. Drafts from the index come first, then the rest
in file name order. The picker marks changed guides and drafts. When a guide
starts, the Log names where it loaded from.

Click **Reload Paths** to read the folder again without starting a quest. Each
start reads it too. To run an edit, stop the quest and start it again. A reload
never swaps a guide that already runs. Delete a changed file and reload to get
the shipped guide back. With the setting off, the folder is ignored.

Each file must fit in 1 MiB. The loader reads the first 256 files in file name
order and skips the rest. A bad file reports its name, its step, and the reason.
A bad changed file falls back to the shipped guide. A bad draft stays in the
list but cannot run.

The rest still load. Guides the server cannot run stay unavailable. Linked
files work if their target is a normal file. The report lists up to 31 problems.
When more remain, a last line reads `N more validation diagnostics omitted`. The
terminal app shows the short form only. Full details are in the Logs.

[Quest guides](../scripting/quest-guides.md#quest-guide-files) describes the file
format.
## Gatherer

The Gatherer cuts trees, mines rock or fishes. It needs the tool for the job:
an axe, a pickaxe, or the fishing tool and its bait. If the tool is missing, it
can take one from your bank.

## Gatherer: choose what to gather and where

| Setting | Default | What it does |
| --- | --- | --- |
| `skill` | Woodcutting | Woodcutting, Mining or Fishing. |
| `woodcuttingResources`, `miningResources`, `fishingMethod` | `normal`; `copper` and `tin`; `fishing.saltfish.op1` | What to gather. Only the setting for the chosen skill appears. |
| `targetPreference` | Best tier | **Best tier** or **Nearest**. It applies to Woodcutting and Mining. |
| `location` | Start | Where to work. See the list below. |
| `site` | none | The named place for `location` **Site**. |
| `customTile` | none | The tile for `location` **Custom**. |
| `radius` | 12 | How far from the work place to gather, from 2 to 64 tiles. |

The four places for `location`:

- **Start** works around the tile where the bot stands when you start the script.
- **Site** works at a named camp for the resource you chose.
- **Auto** searches outward from the start tile, up to 128 tiles.
- **Custom** works at a tile you enter.

## Gatherer: choose what to do with the haul

| Setting | Default | What it does |
| --- | --- | --- |
| `disposition` | Bank | **Bank** carries the haul to a bank and deposits it. **Power** drops it. |
| Bank: Nearest (default), or always use the bank you pick | Nearest | **Nearest** uses the nearest bank the bot can reach. A bank you pick is always used. |
| `useMageBank`, `useZanarisBank` | off | Allow those two banks when `disposition` is **Bank**. |
| `baitTarget` | 100 | For Fishing: the bait to carry, from 1 to 10,000. |
| `food`, `foodTarget`, `eatBelow` | none, 0, 0 | The food to carry, how many, and the hit points at which to eat. 0 for `eatBelow` means half your maximum hit points. |
| `coinTarget` | 0 | The coins to top up to during a bank trip. A coin shortage never starts a trip. |
| `reserveTeleport`, `reserveCasts` | `Off`, 0 | A teleport spell and the casts to keep runes for. This does not allow teleports; `allowTeleports` does. |
| `deathPolicy`, `maxDeaths` | **Recover**, 2 | **Recover** walks back to the work place after a death. **Stop** ends the run, and so does the third death. |
| `allowTeleports`, `allowWilderness`, `allowDangerZones` | off | Walk permissions for this script. |

A change of skill, resource or place applies at the next start. Other changes
apply to the running script at a safe moment.

When the Gatherer stops, the reason is on screen. Examples:

- `supply-missing:` and an item name: a tool, bait or food that you set is in
  neither the pack nor the bank.
- `inventory-blocked:` and a number: the pack is full and no product can be dropped.
- `Auto widening limit: tried: 4 (within 128 tiles)`: **Auto** found four used-up
  places.
- `died before a recovered haul completed` or `maximum deaths exceeded;
  Stop/Start required`: the bot died too often.

## Grinch

The Grinch picks up holiday items that lie on the ground near where you start
it, banks them and walks back. It works around one start tile. It never walks
beyond the radius to look for items.

| Setting | Default | What it does |
| --- | --- | --- |
| **Items** | all 18 | The items to pick up. Choose at least one. |
| `radius` | 12 | How far from the start tile to look, from 1 to 16 tiles. |
| **Bank: Nearest reachable, or the bank you pick** | Nearest reachable | The bank for the deposit. |
| `allow_wilderness`, `allow_danger_zones`, `allow_teleports` | off | Walk permissions for this script. |

The 18 items are:

- Easter egg, Disk of returning, Half full wine jug and Pumpkin
- The green, blue and red Halloween masks
- Santa hat and Christmas cracker
- The red, yellow, blue, green, purple and white partyhats
- Scythe, Bunny ears and Yo-yo

The Grinch banks only the items you chose. Other items stay in your pack. Its
status reads `waiting` when no chosen item is on the ground, and `working` while
it collects, banks or walks back. If the pack is full of other items, it stops
with `inventory is full and contains no selected item to deposit`. Saved changes
apply at the next start.

## Sherlock

Sherlock solves clue scrolls. Start it, and it waits until the bot holds a clue.
It then follows the clue: it walks to the place, searches or digs, fights the
wizard or key-keeper that a clue can bring, and opens the casket at the end.

Sherlock has no settings of its own. Its **Script prefs** window shows the three
walk permissions **Allow teleports**, **Allow wilderness** and **Allow danger
zones**, all off at first.

Sherlock has limits:

- It does not fetch a Spade. At a dig place without one, it waits.
- For some clues, it recognises the clue and then does nothing.
- If the bot dies, Sherlock ends that clue attempt.

## Combat

Combat is the fighting that other scripts use. It is not a script that you start,
so **Browse…** does not list it.

The Quester uses it for the fights in a quest. The panel shows `fighting` in the
`phase` row then. Sherlock uses it for the fights of a clue. A script that you
load can ask for a fight too.

Combat eats food, drinks potions and uses protection prayers during a fight. It
keeps the prayers you turned on. It turns off only the prayers that it turned on.
A protection prayer that it turns on can switch off a different protection prayer
that you had on. It does not turn that one back on.

## Where to go next

| I want to… | Read |
| --- | --- |
| Start, pause and stop a script | [Run scripts on your bots](running-scripts.md) |
| Write a quest guide for the Quester | [Quest guides](../scripting/quest-guides.md) |
| See what does not work yet | [Known issues](../known-issues.toml) |
