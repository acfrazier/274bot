# Engine test patches (274)

The revision-274 nightly engine carries three patches. Two are the
274 twins of the 289 test cheats. One binds the engine to loopback.
An engine without a cheat patch still runs every other cell. A cell
that needs a missing cheat ends as UNAVAILABLE. It never ends as a
plain FAIL.

Apply each patch to a clean tree at the pinned commit: engine
`4c95f87e` (LostCityRS/Engine-TS, branch `274`) and content
`000c19997` (LostCityRS/Content, branch `274`). Each patch was made
from that commit and `git apply --check` passes on it.

| Patch | Repository | What it does | Needed by |
|---|---|---|---|
| `0001-engine-givebank-cheat.patch` | engine | `::givebank <item> (amount)` in `src/network/game/client/handler/ClientCheatHandler.ts`. It seeds the calling player's bank without a booth deposit. It only works when `node.production` is false and for staff level 4 and up. Byte-for-byte the 289 patch 0001 change. | The scenario cells that seed a bank (same list as 289, `docs/harness.md` "Engine test patches"). |
| `0002-content-skill-macro-event-cheat.patch` | content | `scripts/_test/scripts/cheats/cheat_skill_macro_event.rs2`: `::skill_macro_event <id>`. The same file as 289 patch 0002, with the header naming this patch. It calls the content's own skill random-event spawn proc for the calling player. | The `guardian_skill_*` cells in `tools/nightly/cells-274.conf`. |
| `0003-engine-loopback-listeners.patch` | engine | Binds the game listener (`src/server/tcp/TcpServer.ts`), the web and management listeners (`src/web.ts`) and the setup management listener (`src/setup.ts`) to `127.0.0.1`. The 274 branch binds `0.0.0.0`. The 289 branch already binds loopback. | The nightly guest. `status-274.sh` refuses an engine with any other listener. |

## Applying

From the directory that holds `engine/` and `content/`:

```sh
git -C engine apply /path/to/274bot/tools/engine-patches/274/0001-engine-givebank-cheat.patch
git -C engine apply /path/to/274bot/tools/engine-patches/274/0003-engine-loopback-listeners.patch
git -C content apply /path/to/274bot/tools/engine-patches/274/0002-content-skill-macro-event-cheat.patch
```

`git apply --check <patch>` tests a patch without changing anything.
`git apply -R --check <patch>` tells you whether it is already
applied.

Then rebuild and restart the engine:

- **Engine patches (0001, 0003):** restart the engine. The
  TypeScript runs from source.
- **Content patch (0002):** recompile the scripts first. In
  `engine/`, run `node --import tsx tools/pack/Build.ts`, then
  restart. A plain restart keeps the old scripts. The cheat then
  stays unavailable.

To check the content patch, log in a staff account and type
`::~skill_macro_event 2`. With the patch, a River troll appears and
the game says `skill_macro_event 2: river troll.`. Without it,
nothing happens. The cells use that silence to report UNAVAILABLE.
The message names the 289 patch file. On the 274 lane, read it as
this directory's 0002.
