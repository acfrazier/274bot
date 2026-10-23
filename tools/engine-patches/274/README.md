# Engine test patches (274)

The revision-274 nightly engine carries two patches. The
`givebank` patch is the 274 twin of the 289 test cheat. The
loopback patch binds the engine listeners to `127.0.0.1`.
The current 274 cell set does not use the R289-only random-event
oracle, so it does not need a skill-event test cheat.

Apply the engine patch files to a clean engine tree at pinned commit
`4c95f87e` (LostCityRS/Engine-TS, branch `274`). The pristine content
tree is pinned at `000c19997` (LostCityRS/Content, branch `274`) for
building the engine cache; it has no nightly patch. Both engine patches
apply cleanly to that engine commit.

| Patch | Repository | What it does | Needed by |
|---|---|---|---|
| `0001-engine-givebank-cheat.patch` | engine | `::givebank <item> (amount)` in `src/network/game/client/handler/ClientCheatHandler.ts`. It seeds the calling player's bank without a booth deposit. It only works when `node.production` is false and for staff level 4 and up. Byte-for-byte the 289 patch 0001 change. | The scenario cells that seed a bank (same list as 289, `docs/dev/harness.md` "Engine test patches"). |
| `0003-engine-loopback-listeners.patch` | engine | Binds the game listener (`src/server/tcp/TcpServer.ts`), the web and management listeners (`src/web.ts`) and the setup management listener (`src/setup.ts`) to `127.0.0.1`. The 274 branch binds `0.0.0.0`. | The nightly guest. `status-274.sh` refuses an engine with any other listener. |

## Applying

From the directory that holds `engine/` and `content/`:

```sh
git -C engine apply /path/to/274bot/tools/engine-patches/274/0001-engine-givebank-cheat.patch
git -C engine apply /path/to/274bot/tools/engine-patches/274/0003-engine-loopback-listeners.patch
```

`git apply --check <patch>` tests a patch without changing anything.
`git apply -R --check <patch>` tells you whether it is already
applied.

Then restart the engine. Both patches change TypeScript source, so
no content pack rebuild is required. Run `status-274.sh` to check
that the expected listeners bind to loopback.
