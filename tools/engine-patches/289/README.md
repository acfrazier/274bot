# Engine test patches (289)

Some live cells need a test-only cheat that the stock 289 engine and content
don't have. The patches here add those cheats. An engine without a patch still
runs every other cell; a cell that needs a missing patch ends as
**UNAVAILABLE** and names the patch file. It never ends as a plain FAIL.

| Patch | Repository | Adds | Needed by |
|---|---|---|---|
| `0001-engine-givebank-cheat.patch` | engine (`lostcity-289/engine`) | `::givebank <item> (amount)` in `src/network/game/client/handler/ClientCheatHandler.ts`. It seeds the calling player's bank without a booth deposit. It only works when `node.production` is false and for staff level 4 and up. | The scenario cells listed in [`docs/dev/harness.md`](../../../docs/dev/harness.md#engine-test-patches) ("Engine test patches"), and the `host-play` live tests that seed a bank. |
| `0002-content-skill-macro-event-cheat.patch` | content (`lostcity-289/content`) | `scripts/_test/scripts/cheats/cheat_skill_macro_event.rs2`: `::skill_macro_event <id>`. It calls the content's own skill random-event spawn proc for the calling player (1 big fish, 2 river troll, 3 whirlpool, 4 gas, 5 lost pickaxe, 6 rock golem, 7 tree spirit, 8 ent, 9 lost axe, 10 shade, 11 zombie, 12 watchman). It checks what the skill scripts check before they roll an event: no event already active, an allowed area, the fishing spot, rock or tree beside the player, and the tool. Every call answers with one `skill_macro_event <id>: ...` game message. | The twelve `guardian_skill_*` cells (same list). |

## Applying

From the directory that holds `engine/` and `content/`:

```sh
git -C engine apply /path/to/274bot/tools/engine-patches/289/0001-engine-givebank-cheat.patch
git -C content apply /path/to/274bot/tools/engine-patches/289/0002-content-skill-macro-event-cheat.patch
```

`git apply --check <patch>` tests a patch without changing anything, and
`git apply -R --check <patch>` tells you whether it is already applied.

Then rebuild and restart the engine:

- **Engine patch (0001):** restart the engine. The TypeScript runs from
  source.
- **Content patch (0002):** recompile the scripts first. In `engine/`, run
  `npm run build` (`tsx tools/pack/Build.ts`), then restart. Starting the
  engine does not recompile changed content unless `data/pack/server/script.dat`
  is missing (`src/app.ts`), and `build.liveReload` is off in the default
  `data/config/world.json`. A plain restart therefore keeps the old scripts,
  and the cheat stays unavailable.

To check the content patch, log in a staff account and type
`::~skill_macro_event 2`. With the patch, a River troll appears and the game
says `skill_macro_event 2: river troll.`. Without it, nothing happens: the
engine ignores an unknown debug cheat silently. The cells use that silence
to report UNAVAILABLE.

## How the cells report a missing patch

The `guardian_skill_*` cells send the cheat once the activity is running.
If neither the cheat's answer nor the content's busy reply ("Please finish
what you are doing first.") arrives within 10 game ticks, the run ends with
outcome `UNAVAILABLE` in its evidence JSON. The message is `UNAVAILABLE:
engine patch tools/engine-patches/289/0002-content-skill-macro-event-cheat.patch
is not installed ...`. The front ends print that next to their usual
`FAIL: live script_<cell>` line and exit non-zero, so a missing patch never
counts as a pass.
