# Local fixture inputs

Captured 2026-09-10 for step 1. Exact file sizes, SHA-256 hashes, source trees,
dirty tracked files, public-key fingerprints and selected connection settings
are in `fixture-inputs.json`. These identities are discovery inputs; each live
run must recheck them and establish its own preparation and proof baseline.

| Identity | 274 | 289 |
|---|---|---|
| Fixture ID | `local-274` | `local-289` |
| Engine root | `/Users/acfrazier/experiments/Server/engine` | `/Users/acfrazier/experiments/lostcity-289/engine` |
| Engine commit | `4c95f87efe00b068cadbd229d94736626907bd1a` plus recorded local changes | `a275ea812f34342cbaf7faf2e3558dcd6ca187d0`, clean |
| Content commit | `000c19997e07206131bcb3c884265840efce416d` plus a recorded local item cheat | `92649430fcbc83538d8c4367ecb96cee1a67a944`, clean |
| Game / asset / management ports | `43594` / `80` / `8898` | `44594` / `1080` / `9898` |
| Cache directory under engine | `data/pack/client` | `data/pack/client` |
| Readiness observed | Existing PID 1852, cwd and command checked; game listener and HTTP 200 observed | `status-289.sh` exits 1: stale saved PID 49444; no game listener |
| Navigation | Existing legacy v8 pack and flags hashed; rebake before campaign acceptance | No qualified pack yet; step 5 must bake and bind it |

The 289 engine advances the setup report's upstream `0c7cf655` through three
local commits: loopback listener binding, local RSA key handling, and the RSA
round-trip fixture cursor fix. Those changes are present in the clean current
engine commit. The runtime status above supersedes the setup report's old PIDs.

The 274 handler has the local `givebank` addition used by sustained Thiever
preparation. The 289 handler lacks it. Record this as a missing local fixture
capability before banking/restocking proofs, and resolve it within the fixture
work in step 5. Do not count item or XP seeding as script progress. The 274
engine also has existing web-client changes; they remain external inputs and
are not campaign product edits. Do not rebuild or restart that shared process
casually. Runtime config hashes do not contain configuration secrets.

The local 274 cheat handler and item-cheat source were also copied read-only to
`.superpowers/inputs/fixture-274-local-additions/`, so the setup additions can be
reproduced even if the external checkout advances. Their source/snapshot hashes
are recorded in `evidence/input-preflight/274-local-additions.json`.

Reproduce the read-only identity inventory from the campaign checkout:

```sh
python3 docs/compat/inventory_fixtures.py \
  --world-274 /Users/acfrazier/experiments/Server \
  --world-289 /Users/acfrazier/experiments/lostcity-289 \
  --nav-274 /Users/acfrazier/.274bot/274bot.navpack \
  --output /absolute/new-run/fixture-inputs.json
```

Known setup commands, for the named local fixtures when preparation is due:

```sh
# 274: from /Users/acfrazier/experiments/Server/engine
npm run build
node --import tsx src/app.ts

# 289: from /Users/acfrazier/experiments/lostcity-289
./setup-289.sh
./start-289.sh
./status-289.sh
```

The existing 289 setup command writes local configuration, installs dependencies,
builds the pinned content and migrates its isolated SQLite database. Valid local
RSA keys are retained. Its start/status/stop scripts check complete process
identity and occupied ports; use them when starting that fixture. These commands
were inspected, not executed as part of input inventory. Disposable campaign
accounts must be newly generated; earlier native-proof accounts are not inputs.

For 274 navigation, set `ENGINE_DIR`, `NAV_PACK` and `NAV_FLAGS` to the named
fixture and fresh campaign outputs, then run
`cargo run --locked --release -p nav --bin nav-pack`. The current CLI's
`config_jag` default uses `data/pack/config`. The 289 fixture provides the config
at `data/pack/client/config`; step 5 must use the correct explicit input and add
revision/content validation before accepting the resulting pack.

Native platform discovery: this session has macOS arm64 and the native display.
The configured `274bot-builder` SSH probe timed out during banner exchange;
Linux availability is unverified. No Windows runner has been established by
this preflight. Both remain required platform checks under step 8 when their
paths are affected. Linux CI exists with `SKIP_GPU=1`; that is not native GPU
evidence. Existing documented baseline limits are Windows/Linux GPU shade
boundary (about 223 expected, 255 observed) and Windows unreachable-CRC timing
over five seconds. These are historical limits, not freshly reproduced results.

Raw runtime observations are under `evidence/input-preflight/`. No client login,
script action, rendering or performance acceptance is claimed by this inventory.
