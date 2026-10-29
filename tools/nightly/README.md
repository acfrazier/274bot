# Nightly live run

The nightly runs main (non-quest) cells and a separate quest job. Linux
`malbec` runs both jobs on private Engine A and Engine Q copies. Windows runs
both jobs on Engine B without changing its tick rate. Mac `merlot` starts a
fresh Tart clone and runs client work there: main uses the helper's Engine A
forward, while quest uses a private Engine Q copy.
Each job writes its own summary. The archive host collects Linux and Windows
runs. Merlot also stores finished malbec runs in its `NIGHTLY/` mirror.

```
merlot macOS user LaunchAgent (20:00 America/New_York)
  Tart clone from baked base: 8 vCPU, 24 GiB RAM, 160 GB disk
  main Engine A via helper forward (44594/1080); private Engine Q for quest
  /Volumes/dev-scratch/274bot-evidence/NIGHTLY/<run>-mac/
  /Volumes/dev-scratch/274bot-evidence/NIGHTLY/<run>-mac-quest/
  FLOOR status: /Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running

malbec nightly (02:00 America/New_York)
  /srv/nightly/runs/<run>/ and /srv/nightly/runs/<run>-quest/
       | read-only rrsync pull, archive host (hourly)
       v
archive host  tank/274bot-nightly/<run>/ + ZFS snapshot
       | read-only mirror to merlot
       v
merlot /Volumes/dev-scratch/274bot-evidence/NIGHTLY/<malbec-run>/
       (hourly at minute :51; finished runs only, SHA256SUMS verified)

Windows BotTest nightly (23:00 America/New_York)
  C:\274bot-nightly\runs\<run>-windows\ and <run>-windows-quest\
  Engine B 127.0.0.1:47594/3080; no 300 ms tick override
```

`<run>` is `<date>-<branch>-<commit>`, for example
`2026-10-14-release-0.2.3-fb43e9b9a`; Windows runs end in `-windows`, Mac main
runs in `-mac`, and the quest runs use `-quest` (Mac `-mac-quest`). These
suffixes keep the platforms and jobs distinct. `<date>` is the
America/New_York date the run started; `/` in a branch becomes `-`. A second
run of the same branch and commit on the same date gets `-r2`, `-r3`, ….

## What runs

[`run_nightly.py`](run_nightly.py) uses Python 3 standard library, plus `pywinpty`
on Windows. It fetches each branch from `NIGHTLY_REPO`, which defaults to the
public repository. It checks out the branch detached with its submodule. It
builds the frontends and writes a summary for the selected job.
The catalog comes from `crates/scenario/src/catalog.rs`. The main job runs
every catalog cell except names beginning `quester_`. The quest job runs only
those names. `--cells` and `NIGHTLY_CELLS_FILE` intersect with the job's cells.
An explicit list cannot move a quest cell onto Engine A.

Jobs run sequentially and share the runner's lock and build tree:

- **Linux, malbec:** main uses `/srv/nightly/engine-pristine` and starts/stops its
  private Engine A copy on loopback `1080/44594/9898`. `ExecStartPost` invokes a
  second runner process with `NIGHTLY_JOB=quest`, which copies the separate
  `/srv/nightly/engine-q-pristine` to `engine-q-run` and starts/stops it with
  `start-289q.sh` on `1180/44694/9988`. It sets both parallel limits to 1
  because the Q world permits one connected client. Only quest cells get
  `QUESTER_TICK_MS=300` and `local-289q`.
  Non-Windows quest cells write `local-289q` into each isolated HOME, so they
  do not use the operator's `~/.274bot/servers.json`.
- **macOS, merlot:** the `local.274bot-nightly-mac` LaunchAgent starts a fresh
  clone of the local `274bot-mac-base` image at 20:00 America/New_York. The base
  runs macOS 27.0.1. The clone uses 8 vCPU, 24 GiB RAM and a 160 GB disk.
  The host stages the runner, catalog, fixtures and Engine A/Q source data.
  Shared inputs stay read-only. Private guest copies permit owner writes for
  dependency installation, engine state and Git mirrors.
  Engine staging retains `engine/data/pack/.cache`, which contains runtime
  server-map artifacts. It excludes unrelated caches and player state.
  The guest checks out each exact branch commit from `branches.conf` and runs
  main with `lanes,panel,tui`, then quest with `panel,tui`. Each branch uses a
  fresh source tree. Quest cells use one connection because Engine Q sets
  `maxConnected=1`. Main connects to host Engine A through `mac_vm.py`'s
  loopback forward on `44594/1080`; quest starts private Engine Q in the guest.
  The runner and TUI run inside the clone; `panel-play` uses its Aqua session.
  The base includes the needed privacy grants. The host checks the 120 GiB
  free-space floor before and during the run. After stopping the clone, it
  verifies `SHA256SUMS` and publishes completed run folders to `NIGHTLY/`.
  As with the source runner, the root `DONE` marker is outside `SHA256SUMS`.
  All other regular output files must appear in the manifest.
  The controller mirrors `MAC-QUIET.lock` into the guest. The runner stops an
  active live cell at its next boundary. It records that cell as `UNAVAILABLE`
  with a `MAC-QUIET` reason. It does not start another job.
  FLOOR can read `/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running`.
- **Windows, BotTest:** the task runs main with `lanes,panel,tui`, then quest
  with `panel,tui`, both against existing Engine B (`47594/3080`). Engine Q is
  bound to merlot's loopback and is not reachable from Windows without changing
  Engine B, so Windows quest cells retain the existing 600 ms tick and never
  set `QUESTER_TICK_MS`.

Each cell gets an isolated `HOME` and evidence directory, `BOT_CPU=1` unless
its name matches `NIGHTLY_GPU_CELLS`, and its job's distinct name prefix.
The outer cap (`NIGHTLY_CELL_CAP_S`, default 3600 s) stops a process if it does
not reach a terminal state. Two cells keep a 3600 s cap on each host:
`jive_kbd` and `quester_queue`. Their proven walls pass the Windows host's
1200 s cap (2026-10-21: `jive_kbd` 2543.0 s on Linux, `quester_queue`
1987.9 s on Linux). `NIGHTLY_CELL_CAPS_S` (`cell=seconds,...`) overrides
one cell's cap. The summary records each cap in `cell_caps_s`. A paired
proof refuses to start without the pair witness. The runner retries it
with `--pair-core` and records the gate.

The runner writes `SHA256SUMS` from the completed run files before it writes
`DONE`. A read-only malbec mirror checks each transferred file against that
source manifest. It leaves older malbec runs without a source manifest in place.
Each job writes its summary and per-cell receipts. It writes `DONE` last.

Cells listed in [`skip.conf`](skip.conf) still appear as `SKIP`. Quest cells do
not belong in that file; the job filter routes them to Engine Q.

The runner holds `<NIGHTLY_ROOT>/.nightly.lock`, so jobs cannot share the Cargo
target tree concurrently. It removes matching local runs older than
`NIGHTLY_KEEP_DAYS` (14 by default); the archive keeps all Linux and Windows
runs. `NIGHTLY_RUNS_ROOT` can put finished runs outside the runner work root.

Cargo builds use `<NIGHTLY_ROOT>/sccache`, outside their disposable homes, so
cache writes cannot race per-lane cleanup.

## Reading a run

```
<run>/
  summary.md          job, branch/commit, engine, step statuses and cell/lane counts
  summary.json        the same rows plus platform, host, build, engine endpoints,
                      catalog/runner commits, frontend hashes and optional quiet_lock
  build.log           git and cargo output
  engine.log          local engine log for this run (remote Engine A/B/Q is not copied)
  engine-control.log  local engine start/stop output
  cells/<cell>/       one tui-play cell:
    result.json       this cell's summary row
    receipt.json      the scenario's JSON PASS/FAIL record (when it printed one)
    stderr.log        tui-play stderr (the `FAIL: live <cell> {…}` line and `FAIL: <message>`)
    tty.log           terminal transcript without escape codes
    command.txt       the exact command line
    session-logs/     logs tui-play wrote under its throwaway HOME
    evidence/         LIVE_EVIDENCE_DIR captures, when the cell writes any
  panel-cells/<cell>/ one panel-play cell; stdout.log replaces tty.log
  cells-latency/<band>/<cell>/  one latency-lane cell, when the lane is on.
                      The same files as a tui cell, plus latency.json
  latency/<band>/     one band proxy: proxy-stats.jsonl (one record per
                      connection), proxy.log and proxy-ready.txt
  lanes/<lane>/       one CI lane: result.json (exit code, summed test counts,
                      failed test names) and cargo.log
  SHA256SUMS          source hashes for run files, written before DONE
  DONE                finish time; written last
```

The malbec runner writes `SHA256SUMS` before `DONE`. The merlot mirror checks
the copied files against that source manifest before it moves the run into
`NIGHTLY/`. The mirror never rewrites an existing run.

Results:

| Result | Meaning |
|---|---|
| `PASS` / `FAIL` | the scenario printed its JSON record with that outcome; `first_failure` is its `message`. A lane: `cargo` exited 0 / nonzero |
| `TIMEOUT` | no record before the outer cap or the step deadline; the process tree was killed |
| `ERROR` | the frontend exited without a record (startup error, panic); `first_failure` is the first error-looking stderr line |
| `UNAVAILABLE` | the scenario printed a record with that outcome (for example an engine without a test patch), or the cell or lane could not run on this host tonight: `reason` says why (for example Engine A/B/Q not reachable, not enough free disk space, or a MAC-QUIET lock stopped a live cell) |
| `SKIP` | listed in `skip.conf`; `reason` says why |
| `NOT_RUN` | the build failed, a step deadline passed, the run was interrupted, or a non-cell lane stopped on a MAC-QUIET lock; `reason` says which |

Every row has `platform`, `branch`, `commit` and `job` in the summary metadata;
cell rows also have `frontend` (`tui` or `panel`) and `renderer` (`cpu` for
`BOT_CPU=1`, `gpu`). Every summary has `os` and `host` as well. A latency row
adds `latency_band`, `latency_rtt_ms` and `latency_class`. A zero-latency row
never has those keys. See [Latency lane](#latency-lane).

Every summary also carries one `rs2b0t API` line. The runner reads the frozen
revision from the tested tree's `crates/script-js/compat-js/abi-contract.json`,
fetches upstream `main` once into the catalog checkout, and diffs the two
`rs2b0t-api` files. The line reads `unchanged`, `CHANGED` with per-file counts
and up to 10 commits, or `UNAVAILABLE` with the reason. The check reports only.

## Run one night by hand

On the VM, through the same unit the timer starts:

```sh
sudo systemctl start --no-block 274bot-nightly.service
journalctl -u 274bot-nightly.service -f          # progress, one line per cell
```

Only some branches, cells or a different parallelism (as user `bot`, outside the unit):

```sh
cd /srv/nightly
PATH=$HOME/.cargo/bin:$PATH python3 runner/tools/nightly/run_nightly.py \
  --branch release/0.2.6 --cells shop_buyout_aubury,walk --parallel 2
python3 runner/tools/nightly/run_nightly.py --list --tree trees/release-0.2.6   # cell list only
```

Settings can also go in `/etc/274bot-nightly.env` on the VM (read by the unit):
`NIGHTLY_PARALLEL`, `NIGHTLY_CELL_CAP_S`, `NIGHTLY_BRANCHES` (space-separated;
overrides `branches.conf`), `NIGHTLY_KEEP_DAYS`, `NIGHTLY_NAME_PREFIX`,
`NIGHTLY_REVISION` (`289` by default; `274` on the 274 lane's guest) and
`NIGHTLY_CELLS_FILE` (a cell list like [`cells-274.conf`](cells-274.conf);
`--cells` on the command line wins over both).

On the Windows host, as BotTest, through the same task the schedule starts (it runs in
the console session; from an SSH shell, which is session 0, start the task rather than
the script):

```powershell
Start-ScheduledTask -TaskName 274bot-nightly
Get-Content (Get-ChildItem C:\274bot-nightly\logs\*-night.log | Sort-Object LastWriteTime)[-1] -Wait
```

Settings for the Windows host live in `C:\274bot-nightly\nightly.env`
([`windows/nightly.env`](windows/nightly.env) is the deployed copy); the step list
(`NIGHTLY_STEPS`, default `lanes,panel,tui`), each step's deadline
(`NIGHTLY_LANES_DEADLINE_S`, `NIGHTLY_BUILD_DEADLINE_S`, `NIGHTLY_PANEL_DEADLINE_S`,
`NIGHTLY_TUI_DEADLINE_S`), the whole night's (`NIGHTLY_RUN_DEADLINE_S`) and the disk
guard's cap and floor (`NIGHTLY_TARGET_CAP_GB`, `NIGHTLY_MIN_FREE_GB`) can be set
there too.

On merlot, `nightly-mac.sh` reads
`~/.config/274bot-nightly/mac-nightly.env` and starts
`nightly-mac-guest.py`. The controller starts one Tart clone from the baked
`274bot-mac-base` image. Main uses `mac_vm.py`'s host Engine A forward; quest
starts a private Engine Q in the clone. Both jobs' clients run in the guest.
The host fetches each branch in `branches.conf` from `NIGHTLY_MAC_SOURCE_REPO`.
It pins every branch to the fetched commit before it starts the guest jobs.
The status file records the branch commits and the Tart session ID.
Scheduled runs take both branches from that file: the latest release first,
then `dev`. Do not pass `--branch` for the calendar launch.

Run a hand night through the granted launcher:

```sh
test ! -e /Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock
NIGHTLY274_ACTION=nightly-mac \
  "$HOME/.local/libexec/274bot-nightly/nightly274-launch"
```

The guest writes `-mac` and `-mac-quest` run folders. The controller copies
completed folders to `NIGHTLY/` and checks each `SHA256SUMS` file. The host
status file is `/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running`.
If `MAC-QUIET.lock` appears, the controller creates the matching guest lock.
The runner stops an active live cell at its next boundary. The controller
exports completed folders, then stops and deletes the clone.

Pull at once instead of waiting for the hour:
`systemctl start 274bot-nightly-pull.service` (Windows runs:
`274bot-nightly-pull-windows.service`) on the archive host, then
`systemctl --user start 274bot-nightly-mirror.service` on the review machine.
On merlot, `launchctl kickstart gui/$(id -u)/local.274bot-nightly-malbec-pull`
starts the read-only malbec mirror immediately (after its key is installed).

## Stop it

- Stop tonight's run: `sudo systemctl stop 274bot-nightly.service` on the VM. Running
  cells are killed, the engine is stopped, and the partial run is still summarized
  (`"interrupted": true`) and archived.
- Stop the nightly: `sudo systemctl disable --now 274bot-nightly.timer` on the VM.
- Stop archiving: `systemctl disable --now 274bot-nightly-pull.timer` on the archive host.
- Stop mirroring: `systemctl --user disable --now 274bot-nightly-mirror.timer` on the
  review machine.
- macOS on merlot: `cat /Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running`
  shows the active job. Create `/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock`
  to stop the guest at the next cell boundary and prevent the next job. The
  controller copies completed run folders, then stops and deletes the clone.
  To stop it immediately, run
  `launchctl kill SIGTERM gui/$(id -u)/local.274bot-nightly-mac`.
  To unload the schedule, run
  `launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-mac.plist`.
  Unload the malbec mirror similarly with
  `local.274bot-nightly-malbec-pull.plist`.
- Windows: `Stop-ScheduledTask -TaskName 274bot-nightly` stops tonight's run (the task
  ends the runner without a summary; that night has no `DONE` and is never pulled).
  `Disable-ScheduledTask -TaskName 274bot-nightly` (and `274bot-nightly-catchup`) stops
  the Windows nightly; `windows\install-tasks.ps1 -Remove` removes both tasks.
  `systemctl disable --now 274bot-nightly-pull-windows.timer` on the archive host
  stops archiving Windows runs; `systemctl --user disable --now
  274bot-nightly-win-engine.service` on the review machine takes Engine B away
  from the Windows host (its cells then report `UNAVAILABLE`).

## Change the branches

Edit [`branches.conf`](branches.conf) on `dev`, commit, and push. Nothing on
the machines changes. At the start of a night each runner fetches origin and
reads `tools/nightly/branches.conf` from `origin/dev`
(`git show origin/dev:tools/nightly/branches.conf`). The first line is the
fix line: when the fix line rolls over (for example `release/0.2.6` to
`release/0.2.7`), change that line, commit to `dev`, and the next night tests
the new line on every host. Linux reads the list through `/srv/nightly/runner`,
Mac through the stable copy in `~/.local/lib/274bot-nightly/runner`, Windows
through `C:\274bot-nightly\runner`.

Precedence stays: `--branch` on the command line, then `NIGHTLY_BRANCHES`,
then the GitHub list. A `--branch` hand run and a `NIGHTLY_BRANCHES` host
never touch the network for the list. When the fetch fails, the runner uses
its local `branches.conf` and says so in every summary, so a night is never
lost to a fetch failure.

Each run's `summary.json` records `branch_list` (`source` one of
`cli`/`env`/`github`/`local`, the `commit` the list came from, and a `note`),
and `summary.md` has one `Branch list:` line. The Mac status file records
`branch_list_source=`, `branch_list_commit=` and, on fallback,
`branch_list_note=`.

## Runner self-update

At the start of a night, before it reads the branch list, the runner updates
its own checkout of `tools/nightly/` to `origin/dev` and re-executes itself.
Before switching, it extracts the candidate `tools/nightly/` to a temporary
directory and runs a self-check: `python3 -m py_compile` of the runner files
(`run_nightly.py`, `nightly-mac-guest.py`, `latency_proxy.py`) plus the
runner's own unit tests (`test_run_nightly.py` on Linux and Windows,
`test_nightly_mac_guest.py` on the Mac controller), about a minute at most
on Linux and macOS and five minutes on Windows (same suite, slower process
and git spawn).
When the self-check fails, or the fetch fails, the runner keeps running the
old code and says so in the summary. Only the runner checkout is ever touched
(`git -C <runner> ...` plus the temporary candidate); trees, engines,
catalogs and fixtures are never written.

The update is opt-in: `NIGHTLY_SELF_UPDATE=1` (or `true` or `yes`) updates, and
anything else (including unset) keeps the old code and records `disabled`. Set it to `1`
where the night reads its environment: `/etc/274bot-nightly.env` on the
Linux VM (read by the unit), `C:\274bot-nightly\nightly.env` on Windows
(deployed copy [`windows/nightly.env`](windows/nightly.env), read by
`nightly-task.ps1`), and `~/.config/274bot-nightly/mac-nightly.env` on the
Mac controller. Hand runs from a developer checkout stay put by default, so
uncommitted work is never checked out from under you.

On Windows the re-exec waits for the re-executed child and exits with the
child's exit code (`os.execvpe` there would end the parent with 0 at once,
so Task Scheduler would record success whatever the night did).

Each run's `summary.json` records `self_update` (`status` one of
`updated`/`current`/`check-failed`/`fetch-failed`/`extract-failed`/`disabled`,
the `from`/`to` commits, and a `detail`), and `summary.md` has one
`Runner self-update:` line. The Mac status file records `self_update=` and
`self_update_detail=`.

The self-check runs the candidate's full unit-test file with the updater's own
tests skipped (guard `NIGHTLY_SELF_CHECK_ACTIVE`), so a check never re-tests
the updater recursively. It also runs it without the unit's `NIGHTLY_*`
settings (latency bands, cells file, revision and the like): the tests judge
the candidate's code under the runner defaults, not under one host's
configuration. A failed check names the failing tests in its `detail`, not
only the unittest count line.

Manual refresh is only a fallback for a checkout without the opt-in:
`git -C <runner> fetch origin && git -C <runner> checkout --force origin/dev`
(the checkout is detached after a self-update, so the old
`git -C <runner> pull --ff-only` no longer applies).

## Deployment

### Nightly VM (Ubuntu 24.04, 8 cores, 16 GB, user `bot` with the Rust toolchain from `rust-toolchain.toml`)

1. Node 26 from nodejs.org into `/opt` with `node`, `npm`, `npx` linked into
   `/usr/local/bin`. `rsync`, `lsof`, `curl`, `python3` and the client build
   packages come with the lab template.
2. `sudo mkdir /srv/nightly && sudo chown bot:bot /srv/nightly`.
3. Engine: copy the 289 engine checkout into `/srv/nightly/engine-pristine` without
   `runtime/`, `engine/node_modules/`, `engine/data/players/main/*` and
   `engine/db.sqlite*`. Then run `npm ci --ignore-scripts && npm run sqlite:migrate`
   in `engine/` to create an empty account database. Do not copy any account
   database or player saves.
4. Catalog: put an rs2b0t checkout at `/srv/nightly/rs2b0t`, pinned to the commit the
   branches expect.
5. Runner: `git clone https://github.com/acfrazier/274bot /srv/nightly/runner` and check
   out the branch that carries `tools/nightly/`.
6. Update the runner checkout on malbec and zinfandel to this branch. Confirm
   that each checkout contains `systemd/run-quest-job.sh`. Keep malbec's
   `274bot-nightly.timer` unchanged. Keep zinfandel's
   `274bot-nightly-274.timer` unchanged. Both timers start the shared service.
   Set `NIGHTLY_ENABLE_QUEST=1` only in malbec's `/etc/274bot-nightly.env`.
   Leave that variable unset on zinfandel.
7. Pull key: append the archive host's public key to `~bot/.ssh/authorized_keys` as
   `restrict,from="<archive host IP>",command="/usr/bin/rrsync -ro /srv/nightly/runs/" ssh-ed25519 … 274bot-nightly-pull`.
   The VM holds no push credentials and no key for any other machine.
8. Outside an active nightly, update the checkout before copying the quest-engine
   assets; never point the quest runner at `/srv/nightly/engine-pristine` or Engine A's ports:
   ```sh
   set -e
   sudo -u bot git -C /srv/nightly/runner pull --ff-only
   sudo test ! -e /srv/nightly/engine-q-pristine
   sudo install -d -o bot -g bot /srv/nightly/engine-q-pristine
   sudo -u bot rsync -a /srv/nightly/engine-pristine/ /srv/nightly/engine-q-pristine/
   sudo install -o bot -g bot -m 0644 /srv/nightly/runner/tools/nightly/engine-q/world.json \
     /srv/nightly/engine-q-pristine/engine/data/config/world.json
   sudo -u bot sh -c 'cd /srv/nightly/engine-q-pristine/engine &&
     node --import tsx /srv/nightly/runner/tools/nightly/engine-q/check-world-config.mjs \
       /srv/nightly/engine-q-pristine/engine'
   sudo install -o bot -g bot -m 0755 /srv/nightly/runner/tools/nightly/engine-q/start-289q.sh \
     /srv/nightly/engine-q-pristine/start-289q.sh
   sudo install -o bot -g bot -m 0755 /srv/nightly/runner/tools/nightly/engine-q/stop-289q.sh \
     /srv/nightly/engine-q-pristine/stop-289q.sh
   sudo install -o bot -g bot -m 0755 /srv/nightly/runner/tools/nightly/engine-q/status-289q.sh \
     /srv/nightly/engine-q-pristine/status-289q.sh
   sudo -u bot sh -c 'cd /srv/nightly/engine-q-pristine/engine && npm run sqlite:migrate'
   ```
   The Engine Q process works in `/srv/nightly/engine-q-run/engine`. The
   runner copies the full pristine tree to each run directory, so this config
   reaches the run copy at `engine/data/config/world.json`. Its `db.sqlite`
   stays separate from Engine A's database. The engine reads `world.json` for
   database settings, but SQLite always opens `db.sqlite` from the process
   working directory. The engine uses `db.name` only for MySQL. This engine
   does not support a `world.json` SQLite filename selector. Do not use
   `db.name` as that selector.
   The runner sets `QUESTER_TICK_MS=300` only in quest-cell environments, so
   the harness's world-wide `::speed` command cannot affect Engine A.
   Keep global quest ports, profile, run suffix and tick variables out of
   `/etc/274bot-nightly.env`. Set `NIGHTLY_ENABLE_QUEST=1` there on malbec only.
   The shared service helper skips the quest job when the switch is absent. It
   also refuses to run that job when `NIGHTLY_REVISION` is not `289`.
9. Install the updated service and reload systemd on both hosts. Run these
   commands on malbec and zinfandel:
   ```sh
   sudo install -m 0644 /srv/nightly/runner/tools/nightly/systemd/274bot-nightly.service \
     /etc/systemd/system/274bot-nightly.service
   sudo systemctl daemon-reload
   ```
   Do not change either timer. Zinfandel's revision-274 timer still starts the
   shared service, but its unset quest switch makes the helper skip Engine Q.
   Malbec's switch enables the quest run after the main run. Check each
   `<run>/DONE` file after the timer fires.

### Archive host (Proxmox, root)

1. `zfs create -o compression=on tank/274bot-nightly`.
2. `mkdir -m 700 /etc/274bot-nightly`, then
   `ssh-keygen -t ed25519 -N '' -C '274bot-nightly-pull' -f /etc/274bot-nightly/pull_ed25519`.
   Pin the VM's host key as `nightly-vm-<vmid> ssh-ed25519 …` in
   `/etc/274bot-nightly/known_hosts`. The pull finds the VM's DHCP address through the
   guest agent, so the key is bound to the VM, not to an address.
3. Copy `cellar-pull.sh` to `/usr/local/sbin/274bot-nightly-pull`, and copy
   `systemd/274bot-nightly-pull.{service,timer}` to `/etc/systemd/system/`. Then run
   `systemctl daemon-reload && systemctl enable --now 274bot-nightly-pull.timer`.
   Settings: `NIGHTLY_VMID` (202), `NIGHTLY_VM_USER`, `NIGHTLY_DATASET`.
4. `qm set <vmid> --onboot 1` so the VM comes back after a host reboot.
5. Mirror key: one line in root's `authorized_keys`:
   `restrict,from="<review machine IP>",command="/usr/bin/rrsync -ro /tank/274bot-nightly/" ssh-ed25519 … 274bot-nightly-mirror`.
6. Windows pull (needs the Windows host's pull account, below):
   `ssh-keygen -t ed25519 -N '' -C '274bot-nightly-win-pull (cellar)' -f /etc/274bot-nightly/win_pull_ed25519`,
   pin the Windows host's key as `nightly-windows ssh-ed25519 …` in
   `/etc/274bot-nightly/known_hosts`, copy `cellar-pull-windows.sh` to
   `/usr/local/sbin/274bot-nightly-pull-windows` and
   `systemd/274bot-nightly-pull-windows.{service,timer}` to `/etc/systemd/system/`, then
   `systemctl daemon-reload && systemctl enable --now 274bot-nightly-pull-windows.timer`.
   Settings: `NIGHTLY_WIN_HOST` (`DESKTOP-SL99R6C.lan`), `NIGHTLY_WIN_USER`
   (`274bot-pull`), `NIGHTLY_DATASET`.

The pull copies only runs that have `DONE` and are not on the dataset yet. It copies
without symlinks (`rsync -rt`, or SFTP's recursive get for Windows runs) into
`.incoming-<run>`, renames the copy into place, makes it read-only, and takes
`tank/274bot-nightly@<run>`. A run on the dataset is never rewritten.

### Review machine (user units)

1. `ssh-keygen -t ed25519 -N '' -C '274bot-nightly-mirror' -f ~/.ssh/274bot_nightly_mirror_ed25519`
   (the key goes into the archive host's `authorized_keys` as above). Pin the archive
   host's key in `~/.config/274bot-nightly/cellar_known_hosts`.
2. Write `~/.config/274bot-nightly/mirror.env`:
   ```
   NIGHTLY_ARCHIVE_HOST=root@<archive host>
   NIGHTLY_MIRROR_KEY=<home>/.ssh/274bot_nightly_mirror_ed25519
   NIGHTLY_KNOWN_HOSTS=<home>/.config/274bot-nightly/cellar_known_hosts
   NIGHTLY_MIRROR_DIR=<mirror folder>
   NIGHTLY_MIRROR_DAYS=7
   ```
3. Copy `mirror-pull.sh` to `~/.local/lib/274bot-nightly/mirror-pull.sh` and
   `systemd/274bot-nightly-mirror.{service,timer}` to `~/.config/systemd/user/`, then run
   `systemctl --user daemon-reload && systemctl --user enable --now 274bot-nightly-mirror.timer`.

4. Engine B for the Windows host: copy `systemd/274bot-nightly-win-engine.service` to
   `~/.config/systemd/user/`, then
   `systemctl --user daemon-reload && systemctl --user enable --now 274bot-nightly-win-engine.service`.
   It needs the review machine's existing `274bot-engine-tunnel` service and its
   `bottest` SSH host entry.

User units run only while the user has a session, unless lingering is enabled
(`loginctl enable-linger`). `Persistent=true` catches up a missed hour at the next login.

### Revision-274 pull (review machine)

The 274 lane runs on a Mac review machine, so its pull is a launchd agent, not a
user unit. The committed plist uses the `__HOME__` placeholder, and the script
carries no machine-specific path.

1. `ssh-keygen -t ed25519 -N '' -C '274bot-nightly-274-pull' -f ~/.ssh/274bot_nightly274_pull_ed25519`
   and add the public key to the guest user `bot`'s `authorized_keys` (see
   [Keys](#keys)). Pin the guest's key in
   `~/.config/274bot-nightly/zinfandel_known_hosts`.
2. Write `~/.config/274bot-nightly/pull-274.env` (this file stays outside the
   repository):
   ```
   NIGHTLY274_HOST=<274 guest host>
   NIGHTLY274_KEY=<home>/.ssh/274bot_nightly274_pull_ed25519
   NIGHTLY274_KNOWN_HOSTS=<home>/.config/274bot-nightly/zinfandel_known_hosts
   NIGHTLY274_DEST=<mirror folder>
   ```
   `NIGHTLY274_USER` defaults to `bot`. `NIGHTLY274_RSYNC` overrides the rsync
   binary.
3. Build the launcher, install it with the script, and render the plist. launchd
   starts this binary, not `/bin/bash`, so the removable-volume grant at step 4
   applies to it:
   ```sh
   mkdir -p ~/.local/libexec/274bot-nightly ~/.local/lib/274bot-nightly \
     ~/.local/state/274bot-nightly-274
   clang -O2 -Wall -o ~/.local/libexec/274bot-nightly/nightly274-launch \
     tools/nightly/launchd/nightly274-launch.c
   cp tools/nightly/pull-274.sh ~/.local/lib/274bot-nightly/pull-274.sh
   sed "s#__HOME__#$HOME#g" tools/nightly/launchd/local.274bot-nightly-274-pull.plist \
     > ~/Library/LaunchAgents/local.274bot-nightly-274-pull.plist
   ```
   launchd does not expand `$HOME`, and the repository carries no operator path,
   so the plist uses `__HOME__` and the `sed` fills it in. The log folder must
   exist before step 4, because launchd opens
   `~/.local/state/274bot-nightly-274/pull.log` at start.
4. Load the agent, run it once, and allow the access prompt:
   ```sh
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-274-pull.plist
   launchctl kickstart gui/$(id -u)/local.274bot-nightly-274-pull
   ```
   macOS asks for access to removable volumes on the first run. Allow it. If that
   run failed with an access error, run `launchctl kickstart` again and read
   `~/.local/state/274bot-nightly-274/pull.log`. To grant the access without the
   prompt, add `~/.local/libexec/274bot-nightly/nightly274-launch` in System
   Settings → Privacy & Security → Files and Folders (Removable Volumes) or Full
   Disk Access. A rebuilt launcher has a new code hash, so allow it again.

### Merlot macOS nightly and malbec mirror

The Mac user LaunchAgent starts at **20:00 America/New_York**. It starts one
Tart clone from `274bot-mac-base`, runs both Mac jobs, then stops and deletes the
clone before verifying and publishing completed run archives to `NIGHTLY/`.
The clone uses 8 vCPU, 24 GiB RAM and a 160 GB disk. The controller protects
a 120 GiB free-space floor on `/Volumes/dev-scratch`.
`panel-play` runs in the guest's Aqua session. The user must be logged in.
The controller checks
`/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock` immediately before guest
start, during the start wait, before input staging, and immediately before each
cell batch. The runner checks the guest copy
at each cell and lane boundary. While the LaunchAgent runs,
`/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running` contains
the PID, job, state, branch commits and Tart session ID.
The controller records the `mac_vm.py` start process and the clone name before
the helper starts. Teardown stops and deletes the clone after an interruption.
Bootstrap and job logs remain in the helper session's host `share/logs/`
directory after the clone is deleted. The controller also prints the final
log lines when a task finishes, including failures before the first run folder.
Cell environments preserve the configured guest `ISOHOME_ROOT`; they do not
use the host's `/Volumes/dev-scratch/isohome` default inside the guest.
The helper counts every running Tart VM toward Apple's two-guest limit,
including unprefixed operator guests. It never stops or deletes those guests.
If two guests already run, the nightly refuses to clone.

1. Update or check the primary checkout before each refresh. Confirm that it
   contains the merged nightly changes. Run these commands from that checkout:
   ```sh
   cd "$HOME/experiments/274bot"
   git status --short --branch
   git log -1 --oneline --decorate
   ```
   Copy `tools/nightly` from the primary checkout to a stable runner directory.
   Refresh this copy after each merge that changes the nightly runner. Do not
   use a task worktree because cleanup can remove it.
   Stop if the nightly run lock exists. Do not refresh during a nightly run.
   The host stages the runner, catalog, fixtures, engines and isohome helper
   as read-only inputs. Main uses the host Engine A forward; quest runs private
   Engine Q in the clone.
   Keep a dedicated rs2b0t checkout at the catalog path, pinned to the same
   revision as the other nightlies. Do not use the operator's working copy.
   Change the pin between runs, never during a run.
   ```sh
   mkdir -p ~/.local/lib/274bot-nightly/runner/tools/nightly \
     ~/.config/274bot-nightly ~/.local/lib/274bot-nightly \
     ~/.local/state/274bot-nightly-mac ~/.local/state/274bot-nightly-malbec-pull \
     ~/Library/LaunchAgents
   if test -e /Volumes/dev-scratch/274bot-nightly/.mac-nightly.lock; then
     printf '%s\n' 'Mac nightly is running; do not refresh the runner' >&2
     exit 1
   fi
   rsync -a --delete "$HOME/experiments/274bot/tools/nightly/" \
     "$HOME/.local/lib/274bot-nightly/runner/tools/nightly/"
   cat > ~/.config/274bot-nightly/mac-nightly.env <<EOF
   NIGHTLY_MAC_RUNNER=$HOME/.local/lib/274bot-nightly/runner
   NIGHTLY_MAC_ROOT=/Volumes/dev-scratch/274bot-nightly
   NIGHTLY_MAC_RUNS=/Volumes/dev-scratch/274bot-evidence/NIGHTLY
   NIGHTLY_MAC_STATUS=/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running
   NIGHTLY_MAC_CATALOG=/Volumes/dev-scratch/274bot-nightly/rs2b0t
   NIGHTLY_MAC_ENGINE_A=$HOME/experiments/lostcity-289
   NIGHTLY_MAC_ENGINE_Q=$HOME/experiments/lostcity-289-q
   NIGHTLY_MAC_QUIET_LOCK=/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock
   NIGHTLY_MAC_ISOHOME=$HOME/experiments/274bot/.superpowers/bin/isohome
   NIGHTLY_MAC_PYTHON=/opt/homebrew/bin/python3
   NIGHTLY_MAC_VM_TOOL=$HOME/experiments/274bot/.superpowers/release-0.1.9/tools/headed/mac_vm.py
   NIGHTLY_MAC_BASE=274bot-mac-base
   NIGHTLY_MAC_SOURCE_REPO=https://github.com/acfrazier/274bot
   NIGHTLY_MAC_MIN_FREE_GIB=120
   EOF
   chmod 600 ~/.config/274bot-nightly/mac-nightly.env
   ```
   The wrapper reads this environment file at launch. Do not reload the
   LaunchAgent while `MAC-QUIET.lock` exists. Main can reload it after the
   measurement window.
   After each runner refresh, repeat step 3 to reinstall the wrapper scripts
   and render both LaunchAgent plist files from the updated primary checkout.
   Run `plutil -lint` again. Perform the refresh only between nightly runs.
   Do not reload either LaunchAgent while `MAC-QUIET.lock` exists.
2. Create the dedicated read-only key on merlot (keep the private key here):
   ```sh
   ssh-keygen -t ed25519 -N '' -C '274bot-nightly-malbec-mirror' \
     -f ~/.ssh/274bot_nightly_malbec_mirror_ed25519
   cat ~/.ssh/274bot_nightly_malbec_mirror_ed25519.pub
   ```
   Main installs the emitted public-key line on malbec's `bot` account with
   this prefix and the observed merlot source IP:
   `restrict,from="<merlot-source-IP>",command="/usr/bin/rrsync -ro /srv/nightly/runs/"`.
   It permits read-only rsync of `/srv/nightly/runs/` only, no shell, forwarding
   or writes. The pinned malbec host key is in `~/.ssh/known_hosts_lab`; set
   `~/.config/274bot-nightly/malbec-mirror.env`:
   ```sh
   cat > ~/.config/274bot-nightly/malbec-mirror.env <<EOF
   NIGHTLY_MALBEC_HOST=malbec.lan
   NIGHTLY_MALBEC_USER=bot
   NIGHTLY_MALBEC_KEY=$HOME/.ssh/274bot_nightly_malbec_mirror_ed25519
   NIGHTLY_MALBEC_KNOWN_HOSTS=$HOME/.ssh/known_hosts_lab
   NIGHTLY_MALBEC_DEST=/Volumes/dev-scratch/274bot-evidence/NIGHTLY
   NIGHTLY_MALBEC_RSYNC=$HOME/.local/libexec/274bot-standby/rsync
   EOF
   chmod 600 ~/.config/274bot-nightly/malbec-mirror.env
   ```
3. Reuse the already granted
   `~/.local/libexec/274bot-nightly/nightly274-launch` binary. Do **not** rebuild
   it. Install the dispatcher and both scripts, create launchd log folders, and
   render the two templates:
   ```sh
   mkdir -p ~/.local/state/274bot-nightly-274 \
     ~/.local/state/274bot-nightly-malbec-pull
   cp tools/nightly/pull-274.sh ~/.local/lib/274bot-nightly/pull-274.sh
   cp tools/nightly/nightly-mac.sh ~/.local/lib/274bot-nightly/nightly-mac.sh
   cp tools/nightly/malbec-mirror-pull.sh ~/.local/lib/274bot-nightly/malbec-mirror-pull.sh
   sed "s#__HOME__#$HOME#g" tools/nightly/launchd/local.274bot-nightly-mac.plist \
     > ~/Library/LaunchAgents/local.274bot-nightly-mac.plist
   sed "s#__HOME__#$HOME#g" tools/nightly/launchd/local.274bot-nightly-malbec-pull.plist \
     > ~/Library/LaunchAgents/local.274bot-nightly-malbec-pull.plist
   plutil -lint ~/Library/LaunchAgents/local.274bot-nightly-mac.plist \
     ~/Library/LaunchAgents/local.274bot-nightly-malbec-pull.plist
   ```
4. Enable both agents after the hand acceptance run. Confirm that Main installed
   the restricted key on malbec and that both env files are ready.
   ```sh
   launchctl enable gui/$(id -u)/local.274bot-nightly-mac
   launchctl enable gui/$(id -u)/local.274bot-nightly-malbec-pull
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-mac.plist
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-malbec-pull.plist
   launchctl print gui/$(id -u)/local.274bot-nightly-mac
   launchctl print gui/$(id -u)/local.274bot-nightly-malbec-pull
   ```
   Keep both services enabled and loaded in the Aqua user domain. The Mac job
   keeps its 20:00 schedule. `RunAtLoad=false` prevents an extra run when
   launchd loads the agent.

### Windows nightly host (BotTest, no elevation)

Every step runs as BotTest without the admin token, so the nightly keeps working if
BotTest is demoted to a standard user. Only the pull account (step 8) needs an
administrator, once.

1. Prerequisites already on the host: the Rust toolchain from `rust-toolchain.toml` in
   `C:\Users\BotTest\.cargo`/`.rustup` with the MSVC build tools, Git for Windows,
   and CPython 3.14 in `C:\Program Files\Python314`. The host clock is set to
   Eastern time (the runner uses the host clock: Windows CPython has no tz
   database).
2. `C:\274bot-nightly\` with `runs\`, `logs\`, `state\`; the venv:
   `& 'C:\Program Files\Python314\python.exe' -m venv C:\274bot-nightly\venv`, then
   `C:\274bot-nightly\venv\Scripts\python.exe -m pip install --require-hashes --only-binary :all: --no-deps -r <runner>\tools\nightly\windows\requirements.txt`.
3. Read-only inputs (copied in once; refresh them when they change):
   `engine-pristine\{engine,content}` (the 289 engine and content, without
   `node_modules`, account database or player saves: the clients read its pack, config
   and content; it is never started), `rs2b0t\` (the catalog, as on the Linux VM),
   `nav-snapshot\<16-hex>\` (a decoded client-cache snapshot; the runner makes it again
   when it is missing or stale), `fixtures\274bot-289-v17.navpack` with its sidecars
   (`.navpack.json`, `.navflags`, `.navreach`, `.navcanlight`, `.navpois`; each test
   lane's `NAV_PACK`, `WORLD_NAV_PACK` and `NAV_ARRIVAL_PACK`, the v17 reference pack
   named in `docs/api/nav.md`), `fixtures\inspect.fb` (the old inspection capture
   `T5W_INSPECT_CAPTURE_BASELINE` for the host-play route-inspect test) and
   `fixtures\rusty_v8_release_x86_64-pc-windows-msvc.lib.gz` (the platform gate's
   prebuilt V8, for `v8` 137.2.1; replace it when `Cargo.lock` moves `v8`).
4. Runner: a clone of this repository in `C:\274bot-nightly\runner` on the branch that
   carries `tools/nightly/` (`git -C C:\274bot-nightly\runner pull --ff-only` to update).
5. Settings: copy [`windows/nightly.env`](windows/nightly.env) to `C:\274bot-nightly\nightly.env`.
6. Tasks: `powershell -ExecutionPolicy Bypass -File C:\274bot-nightly\runner\tools\nightly\windows\install-tasks.ps1`
   (`-At 23:00` by default).
7. Engine B: the review machine's `274bot-nightly-win-engine` service (above).
8. Pull account, **by an administrator** (elevated PowerShell), with the archive host's
   public key: `C:\274bot-nightly\runner\tools\nightly\windows\pull-access.ps1 -PublicKey '<win_pull_ed25519.pub>'`
   (`-From` defaults to the archive host's 10.0.0.72). It creates `274bot-pull` (no
   groups, random unknown password), makes `C:\274bot-nightly` private to BotTest,
   SYSTEM and Administrators with read-only `runs\` for `274bot-pull`, and adds the
   `Match User 274bot-pull` block (chroot `runs\`, `internal-sftp -R`, no forwarding,
   no TTY) after `sshd -t` accepts it. `-Undo` removes all of it.

For the quest split, BotTest needs only the runner/task update; Engine Q is bound
to merlot's loopback and cannot be reached from Windows without changing Engine B.
The checked-in task script runs the main job and then the quest job on the
existing Engine B endpoint `127.0.0.1:47594`/`3080`. It removes
`QUESTER_TICK_MS` for each process, so Windows quest cells keep Engine B's
current 600 ms setting. Do not copy Engine Q, change Engine B, or add tick/port
overrides to `nightly.env`. After the `dev` update:

```powershell
git -C C:\274bot-nightly\runner pull --ff-only
Start-ScheduledTask -TaskName 274bot-nightly
Get-ChildItem C:\274bot-nightly\logs\*-main-night.log,C:\274bot-nightly\logs\*-quest-night.log |
  Sort-Object LastWriteTime | Select-Object -Last 2
```

## Keys

| Key | Lives on | Accepted by | Can do |
|---|---|---|---|
| `274bot-nightly-pull` | archive host `/etc/274bot-nightly/pull_ed25519` | VM user `bot` | read-only rsync of `/srv/nightly/runs/` from the archive host's address; no shell, no forwarding |
| `274bot-nightly-win-pull` | archive host `/etc/274bot-nightly/win_pull_ed25519` | Windows host user `274bot-pull` (`%ProgramData%\ssh\274bot_pull_authorized_keys`) | read-only SFTP of `C:\274bot-nightly\runs\` (chroot) from the archive host's address; no shell, no forwarding, no TTY |
| `274bot-nightly-mirror` | review machine `~/.ssh/274bot_nightly_mirror_ed25519` | archive host root | read-only rsync of `/tank/274bot-nightly/` from the review machine's address; no shell, no forwarding |
| `274bot-nightly-274-pull` | review machine `~/.ssh/274bot_nightly274_pull_ed25519` | 274 guest user `bot` | read-only rsync of `/srv/nightly/runs/` from the review machine's address; no shell, no forwarding |
| `274bot-nightly-malbec-mirror` | merlot `~/.ssh/274bot_nightly_malbec_mirror_ed25519` | malbec user `bot` | read-only rsync of `/srv/nightly/runs/` with `/usr/bin/rrsync -ro`; no shell, no forwarding |

The VM and the Windows host clone only public repositories, anonymously
(`credential.helper` empty, no terminal prompt), and hold no push credentials. The
Windows host gets Engine B from the review machine's reverse forward; it holds no key
to the builder VM.

## Revision 274 lane

A second Linux guest (zinfandel) runs the same script against a
revision-274 engine every night. The point is narrow: the
multi-revision code (revision selection, the per-revision cache and
nav pack, the client revision features, content-derived data) stays
exercised while nobody checks it by hand. That protects the future
revision-377 work. 289 stays the production revision. This lane
reports only. It never gates a merge or a release, and its failures
sit at the back of the backlog.

What differs from the 289 lane:

- Settings, in the guest's `/etc/274bot-nightly.env`:
  `NIGHTLY_REVISION=274`, `NIGHTLY_BRANCHES=dev` (no release line),
  `NIGHTLY_CELLS_FILE=cells-274.conf`, `NIGHTLY_CELL_CAP_S=900`,
  `NIGHTLY_RUN_DEADLINE_S=9000`, `NIGHTLY_NAME_PREFIX=n4`,
  `NIGHTLY_START=22:30`. The
  revision selects the built-in `local-274` profile (game 43594,
  assets on port 80), the `start-274.sh`/`stop-274.sh` engine
  scripts, the `unpack` decoded-cache folder, the 274 nav bake
  (`BOT_NAV_REVISION=274`) and the `-274` run suffix. Runs are
  `<date>-dev-<commit>-274`.
- Cells: [`cells-274.conf`](cells-274.conf), 104 of 244. Its header
  states the breadth rule, the R289-only quester and random-event
  oracle exclusions, and the known 289 non-PASS exclusions. The runner
  fails if a listed name leaves the catalog. It never drops a name silently.
- Engine: clean clones at engine `4c95f87e` and content `000c19997`
  (branch `274`), with the patches in `tools/engine-patches/274/`
  and the control scripts and `world.json` in
  [`engine-274/`](engine-274/). Listeners stay on loopback. The
  guest's node binary carries `cap_net_bind_service` for port 80.
- Schedule: [`systemd/274bot-nightly-274.timer`](systemd/274bot-nightly-274.timer),
  22:30 America/New_York, starting the same `274bot-nightly.service`
  the 289 guest uses (the guests' unattended-upgrade gate watches
  that unit name). Malbec builds near 02:00 and 06:20. This lane
  builds at 22:30 and ends before malbec starts.
- Results: the review machine pulls finished runs straight from the
  guest into its `NIGHTLY/<run>-274/` mirror folder (launchd agent
  `local.274bot-nightly-274-pull`, hourly, the restricted pull key
  in the table above). The agent takes the guest, key, pinned host
  key and destination from `~/.config/274bot-nightly/pull-274.env`
  (see [Revision-274 pull](#revision-274-pull-review-machine)). There
  is no archive-host copy of a 274 run. The mirror keeps every run it
  pulled. The guest prunes its own runs after 14 days like the 289
  guest.

Stop it: `sudo systemctl disable --now 274bot-nightly-274.timer` on
the guest stops the lane. `sudo systemctl stop
274bot-nightly.service` stops tonight's run. On the review machine,
`launchctl bootout gui/$(id -u)/local.274bot-nightly-274-pull`
stops the pull.

## Latency lane

The latency lane runs 13 cells again behind a fixed network delay. It answers
one question: does a cell that passes on a local engine still pass at a real
round-trip time? The lane reports only. It never gates a merge or a release.

The lane is off by default. A night with no latency settings writes the same
summary files as a runner without the lane. Two switches turn it on:

- `NIGHTLY_LATENCY_BANDS=50,100,150` in the host settings file, or
- `--latency-bands 50,100,150` on a hand run. An empty value turns it off.

Each band is a round-trip time in ms. The proxy delays each direction by one
half of the band: 25, 50 and 75 ms. The lane has no jitter band.

What runs:

- Branch: `NIGHTLY_LATENCY_BRANCH`, default `dev`. Job: main only. Frontend:
  `tui-play` only. The quest job never runs the lane.
- Cells: [`cells-latency.conf`](cells-latency.conf), or the file in
  `NIGHTLY_LATENCY_CELLS_FILE`. A name outside the catalog fails the run. A
  `quester_*` name fails the run. `--cells` narrows the latency set too, so a
  hand run keeps the same cells at zero and at each band.
- Order: after the tui step, with the same build. The step deadline is
  `NIGHTLY_LATENCY_DEADLINE_S`, default 3600 s. The night deadline and
  `MAC-QUIET.lock` bind latency cells exactly as they bind main cells.
- One [`latency_proxy.py`](latency_proxy.py) process per band. The proxy
  binds a free loopback port. It needs no privilege. It changes no host
  network setting. The runner stops it by its recorded PID at the band end.
- Endpoint: the cell gets the proxy port as its game port and the real engine
  HTTP port as its asset port. Remote mode uses the `--host`/`--port` flags.
  Local mode writes the proxy endpoint into the cell HOME `servers.json`.
  The HTTP cache path stays direct in both modes.
- Accounts: one name prefix per band. The job prefix plus the band digits
  fills the mint limit of 4 characters: `nt50` at rtt50, `n100` at rtt100.

The proxy reads the first byte of each connection. Byte 14 is a game login.
It takes the delayed path. Byte 15 is the OnDemand update socket, which
shares the game port. It takes a direct path with no queue delay. Any other
byte takes the delayed path. Each direction has one FIFO queue, so byte order
cannot change. A full queue (4 MiB) closes that one connection. The proxy
never drops bytes and continues.

Fail closed: a band cell is `UNAVAILABLE` when the proxy does not start in
10 s, when the proxy process exits, or when a probe connection through the
proxy cannot reach the engine. After the cell, the runner checks the
measured median queue delay from the proxy stats. A median more than 10 ms
from the configured one-way delay rewrites the row to `UNAVAILABLE`. A PASS
with no delayed connection in its stats gets the same rewrite. The row class
for each such row is `proxy-unavailable`.

Each band row gets a class from a script, never from prose:

| Zero result | Band result | `latency_class` |
|---|---|---|
| `PASS` | `PASS` | `pass` |
| `PASS` | `FAIL`, `TIMEOUT` or `ERROR` | `latency-defect` |
| `FAIL`, `TIMEOUT` or `ERROR` | any | `also-fails-at-0` |
| `UNAVAILABLE`, `NOT_RUN`, `SKIP` or absent | any | `no-baseline` |
| any | proxy health failed | `proxy-unavailable` |
| `PASS` | `NOT_RUN` or `SKIP` | `not-run` |
| `PASS` | `UNAVAILABLE`, not from the proxy | `unavailable` |

A `latency-defect` row goes through the normal classify-first step (product,
oracle, fixture, interference). It gets no automatic rerun.

Output: `summary.md` gains a cell-by-band matrix after the tui table. The
main counts still describe the zero run only. `summary.json` gains
`latency_counts` by class, `latency_cells` rows, and a `latency` object with
the proxy hash, the bands and the per-band measurements. Each cell folder
holds `latency.json`: the band, the configured delays, the proxy identity,
the measured per-direction queue-delay p50 and p95 for that cell window, the
median first-login response time, connection counts by first byte, the
frontend hash and the account prefix. Proxy records that open before the
band cells start (the health probe) never count toward a cell. Cells in one
band share one proxy, so per-cell measurements hold the connections that
opened inside that cell's wall window.

Enable it on malbec (orchestrator only, never during a run):

1. Update the runner checkout to a commit with this lane:
   `git -C /srv/nightly/runner pull --ff-only`.
2. Add `NIGHTLY_LATENCY_BANDS=50,100,150` to `/etc/274bot-nightly.env`. The
   service reads that file through `EnvironmentFile`. Change no timer and no
   service file.
3. Dry-check the cell list first:
   `python3 /srv/nightly/runner/tools/nightly/run_nightly.py --list --tree /srv/nightly/runner --latency-bands 50,100,150`.
   The last lines name the 13 cells and the 3 bands. An unknown name makes
   the command exit 1.
4. Optional hand run of two cells at one band, as user `bot` outside the
   unit (same shape as "Run one night by hand"), from `/srv/nightly`:
   `PATH=$HOME/.cargo/bin:$PATH python3 runner/tools/nightly/run_nightly.py --branch dev --cells walk,bank_v2_ts --latency-bands 150`.
   Read `measured` in each `latency.json`. The p50 values must sit within
   10 ms of 75 ms.

Tests use Python 3.10+ (`/opt/homebrew/bin/python3` on Merlot):
`python3 -B tools/nightly/test_latency_proxy.py` for the proxy,
`python3 -B tools/nightly/test_run_nightly.py` for the runner and launcher,
and `python3 -B tools/nightly/test_nightly_mac_guest.py` for the Tart controller.

## Windows nightly

The Windows host runs `dev` (`NIGHTLY_BRANCHES` in `nightly.env`) as BotTest in
BotTest's console session, which the GPU and the panel windows need. Nothing it does
needs the admin token.

**Tasks.** `274bot-nightly` (daily 23:00, Interactive: runs only while BotTest is
logged on, Limited, no stored password) starts `windows\nightly-launch.vbs`, which runs
`windows\nightly-task.ps1` without a console window; that reads `nightly.env` and
starts `run_nightly.py` with the venv's Python, logging to `logs\<stamp>-night.log`.
The runner then checks the scenario driver's conditions: BotTest is the console user,
`query user` shows BotTest's console session Active, and the runner runs in that
session. If not (for example the session is disconnected), it still writes the whole
run with every lane and cell `UNAVAILABLE` and the reason. An Interactive task cannot
start at all while BotTest is logged off (after a reboot, the host does not log on by
itself), so `274bot-nightly-catchup` runs at BotTest's next logon: if the last 23:00
night left no `state\started-<date>` marker, it writes that night as a run with every
lane and cell `UNAVAILABLE: BotTest console session not active at the scheduled 23:00
start`. Catch-up does not run the disk guard. Its task limit is 30 minutes, and a
large removal killed at that limit would leave the missed night with no summary at
all. The next scheduled night runs the guard again. Catch-up still runs the sources
step (1 below) before it writes the summary, because the summary names the branch's
commit and lists that commit's cells. On this host that step removes `trees\<branch>`
and clones it again from the local mirror. That folder holds only sources, not Cargo
output. If that step fails, catch-up still writes the summary, with the sources
error in it. If the task is stopped during that step, the missed night has no summary.

**One night, in order, each step with its own deadline:**

1. Sources: an anonymous fetch of the branch into the bare mirror
   `mirror\274bot.git`, then a fresh clone of that exact commit into
   `trees\<branch>`, with the pinned client submodule from
   `mirror\vendor-fr-client-rust.git`. The tree path is the same every night.
   Last night's tree is removed first and the clone is made again at that
   path, so the tree holds exactly the new commit and Cargo overwrites its
   old artefacts instead of adding a new set for a new source path. Trees
   that are not a branch's stable path (for example the per-commit trees of
   the old scheme) are removed at the end of the night.
2. `lanes` (3 h): the Windows platform gate's set (`gate.ps1`) with the GPU on
   (`SKIP_GPU` unset): `build`, `build-vendor`, `clippy`, `clippy-vendor` (with
   `-D warnings`), `test-workspace`, `test-vendor`, `test-script-load`,
   `test-script-no-default`, `test-host-play-memory`, `test-host-play-memory-no-alloc`,
   `test-host-performance`, `test-panel-memory` (every target, including the
   desktop-only `ime_frame`), `test-panel-render-diagnostics`, `test-tui-memory`, and
   `cargo fmt --check` per crate (a workspace-root fmt hits OS error 206 on Windows).
   Each test lane gets the Linux CI test inputs: the three nav pack names set to
   the v17 fixture, `ENGINE_DIR` and `WORLD_ENGINE_DIR` set to the pristine engine
   copy, `BOT_NAV_SNAPSHOT_ROOT` set to the decoded snapshot folder, and
   `T5W_INSPECT_CAPTURE_BASELINE` set to the inspection fixture when the host holds
   it. Each lane gets its own throwaway
   `HOME`/`USERPROFILE`/`APPDATA`/`LOCALAPPDATA`/`TEMP` with a copy of the
   decoded snapshot, and `BOT_NAV_BUILD=skip`, `CARGO_INCREMENTAL=0`,
   `CARGO_PROFILE_DEV_DEBUG=0`; the target directory is `target\<branch>-ci`.
3. Build (1.5 h): `cargo build --locked --release -p tui --bin tui-play -p panel --bin panel-play`
   into `target\<branch>`, with the navigation bake as on the Linux VM.
4. `panel` (3.5 h): every cell as `panel-play --profile local-289 --host 127.0.0.1
   --port 47594 --asset-host 127.0.0.1 --http-port 3080 --engine <engine-run>\engine
   --catalog C:\274bot-nightly\rs2b0t --live script_<cell>`, 3 at a time
   (`NIGHTLY_PANEL_PARALLEL`), windows on the console desktop, prefix `np`. Paired
   proofs are `UNAVAILABLE` here (panel-play has no `--pair-core`); the `tui` step runs
   them under the gate.
5. `tui` (6 h): every cell as on the Linux VM, with the same engine flags, on ConPTY
   (pywinpty; a `cmd /c … 2>stderr.log` shim inside the console keeps stderr apart), 4 at
   a time, prefix `nw`.

In both cell steps every cell gets a throwaway `HOME`/`USERPROFILE`/`APPDATA`/
`LOCALAPPDATA`/`TEMP`, and `BOT_CPU=1` unless its name matches `NIGHTLY_GPU_CELLS`
(`^render_`: those draw with the GPU). The outer cap is 1200 s on this host
(`NIGHTLY_CELL_CAP_S`). `jive_kbd` and `quester_queue` keep 3600 s.
No step runs past the night's deadline either (8.5 h, so a 23:00 start ends by
07:30, before Engine B's busy morning and well inside the task's
11 h limit). A cell still running when its deadline passes is stopped as `TIMEOUT`,
and cells not started by then are `NOT_RUN`.

**Engine B.** The cells use Engine B, the shared engine other agents' live cells use,
through the review machine's `274bot-nightly-win-engine` reverse forward on host
loopback 47594 (game) and 3080 (web). The runner never starts, stops or reconfigures
it; before each cell step it checks the 40-byte `/crc` identity and the game port, and
if either fails, that step's cells are `UNAVAILABLE: Engine B not reachable`. Accounts
are minted fresh under the nightly prefixes (`nw…` for tui-play, `np…` for panel-play),
apart from other agents' accounts; the runner itself sends no cheats or world commands.

**Disk guard.** Before the sources, before the `lanes` step and before the
release build, the runner measures the branch's Cargo target folders
(`target\<branch>-ci`, `target\<branch>` and `target\navskip`) and the free
space on their drive. It removes a target folder that is bigger than
`NIGHTLY_TARGET_CAP_GB` (40 on this host). A target folder whose size cannot
be read in full counts as bigger than the cap and is removed too. If the drive
still has less free space than `NIGHTLY_MIN_FREE_GB` (100 on this host), it
removes the other target folders too, and it logs one line per removal with
the sizes. If free space is still below the floor after that, the whole night
is `UNAVAILABLE` with that reason and nothing is built, so the nightly cannot
fill the disk. A removal that fails also makes the night `UNAVAILABLE`.
Measurement and removal use one walk of the folder. That walk never goes into a
symlink or a junction, at the top or inside the folder. A link counts as 0 bytes,
and removal deletes the link itself (a junction with `rmdir`), never what it points
to. Permission fixes go only to real files and folders inside the folder that is
being removed, so no file outside it changes, not even its mode.
The logon catch-up does not run the guard (see Tasks above).
A setting of 0 turns that check off. Both checks are off by default on
Linux, whose disk is 100 GB in total and whose tree path was already stable.
They run when the variables are set there.

**What persists between nights:** BotTest's `CARGO_HOME` and `RUSTUP_HOME`, the build
caches `target\<branch>` and `target\<branch>-ci` (compiled artefacts only, subject
to the disk guard above), the bare mirrors, the read-only inputs (`engine-pristine`,
`rs2b0t`, `nav-snapshot`, `fixtures`), `runs\` (14 days), the branch's tree
`trees\<branch>`, `logs\` (30 days), the `state\` markers, the venv and the runner
checkout. `engine-run\` (a fresh copy of `engine-pristine` per step) and `work\`
(per-cell and per-lane homes, removed after each) carry nothing to the next night.
