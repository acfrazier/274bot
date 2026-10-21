# Nightly live run

The nightly runs the main (non-quest) cells and a separate quest job. The Linux
VM `malbec` runs both jobs on local Engine A and Engine Q copies; Windows runs
both jobs on its existing Engine B (without changing its tick rate); macOS
`merlot` runs the main job against Engine A and quest cells against Engine Q.
Results are summarized per job. The archive host collects the Linux and Windows
runs; merlot also keeps the finished malbec runs in its `NIGHTLY/` mirror.

```
merlot macOS user LaunchAgent (20:00 America/New_York)
  /Volumes/dev-scratch/274bot-evidence/NIGHTLY/<run>-mac/
  /Volumes/dev-scratch/274bot-evidence/NIGHTLY/<run>-mac-quest/
  Engine A 127.0.0.1:44594/1080; Engine Q 127.0.0.1:44694/1180
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

[`run_nightly.py`](run_nightly.py) (Python 3 standard library only; on Windows also
`pywinpty`) fetches each branch from the public repository, checks it out detached
with its submodule, builds the frontends and writes a complete summary for the
selected job. The catalog comes from `crates/scenario/src/catalog.rs`. The main
job runs every catalog cell except names beginning `quester_`; the quest job
runs only those names. `--cells` and `NIGHTLY_CELLS_FILE` are intersected with
the job's cells, so an explicit list cannot move a quest cell onto Engine A.

Jobs run sequentially and share the runner's lock and build tree:

- **Linux, malbec:** main uses `/srv/nightly/engine-pristine` and starts/stops its
  private Engine A copy on loopback `1080/44594/9898`. `ExecStartPost` invokes a
  second runner process with `NIGHTLY_JOB=quest`, which copies the separate
  `/srv/nightly/engine-q-pristine` to `engine-q-run` and starts/stops it with
  `start-289q.sh` on `1180/44694/9988`. It sets both parallel limits to 1
  because the Q world permits one connected client. Only quest cells get
  `QUESTER_TICK_MS=300` and `local-289q`.
- **macOS, merlot:** the `local.274bot-nightly-mac` user LaunchAgent runs main
  with `lanes,panel,tui` on already-running Engine A (`44594/1080`), then quest
  with `panel,tui` on already-running Engine Q (`44694/1180`), serialized to one
  connection because Q's world sets `maxConnected=1`. It never starts, stops or
  changes either engine. `panel-play` is practical because the LaunchAgent runs
  in the logged-in Aqua session and can open its own windows overnight.
  The Mac launcher checks `MAC-QUIET.lock` before starting; the runner records a
  lock that appears while active and starts no later cells. FLOOR can check
  `/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running`.
- **Windows, BotTest:** the task runs main with `lanes,panel,tui`, then quest
  with `panel,tui`, both against existing Engine B (`47594/3080`). Engine Q is
  bound to merlot's loopback and is not reachable from Windows without changing
  Engine B, so Windows quest cells retain the existing 600 ms tick and never
  set `QUESTER_TICK_MS`.

Each cell gets an isolated `HOME` and evidence directory, `BOT_CPU=1` unless
its name matches `NIGHTLY_GPU_CELLS`, and its job's distinct name prefix. The
outer cap (`NIGHTLY_CELL_CAP_S`, default 3600 s) only stops a process that
never reaches a terminal state. A paired proof refuses to start without the
pair witness; the runner retries it with `--pair-core` and records the gate.
Each job writes its own `summary.json`, `summary.md` and per-cell receipts, then
`DONE`.

Cells listed in [`skip.conf`](skip.conf) still appear as `SKIP`. Quest cells do
not belong in that file; the job filter routes them to Engine Q.

The runner holds `<NIGHTLY_ROOT>/.nightly.lock`, so jobs cannot share the Cargo
target tree concurrently. It removes matching local runs older than
`NIGHTLY_KEEP_DAYS` (14 by default); the archive keeps all Linux and Windows
runs. `NIGHTLY_RUNS_ROOT` can put finished runs outside the runner work root.

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
  lanes/<lane>/       one CI lane: result.json (exit code, summed test counts,
                      failed test names) and cargo.log
  DONE                finish time; written last
```

The malbec mirror adds `SHA256SUMS` to each newly copied run and verifies it
before the atomic move into `NIGHTLY/`. It never rewrites an existing run.

Results:

| Result | Meaning |
|---|---|
| `PASS` / `FAIL` | the scenario printed its JSON record with that outcome; `first_failure` is its `message`. A lane: `cargo` exited 0 / nonzero |
| `TIMEOUT` | no record before the outer cap or the step deadline; the process tree was killed |
| `ERROR` | the frontend exited without a record (startup error, panic); `first_failure` is the first error-looking stderr line |
| `UNAVAILABLE` | the cell or lane could not run on this host tonight: `reason` says why (for example Engine A/B/Q not reachable, not enough free disk space, or a paired proof that panel-play has no gate for) |
| `SKIP` | listed in `skip.conf`; `reason` says why |
| `NOT_RUN` | the build failed, a step deadline passed, the run was interrupted, or `MAC-QUIET.lock` stopped later cells; `reason` says which |

Every row has `platform`, `branch`, `commit` and `job` in the summary metadata;
cell rows also have `frontend` (`tui` or `panel`) and `renderer` (`cpu` for
`BOT_CPU=1`, `gpu`). Every summary has `os` and `host` as well.

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
  --branch release/0.2.4 --cells shop_buyout_aubury,walk --parallel 2
python3 runner/tools/nightly/run_nightly.py --list --tree trees/release-0.2.4   # cell list only
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

On merlot, the user LaunchAgent and the hand-run command both execute the main
and quest jobs through `nightly-mac.sh`; it reads
`~/.config/274bot-nightly/mac-nightly.env`, checks `MAC-QUIET.lock`, and updates
the FLOOR status file. To pin a hand run to the just-fetched `origin/dev`:

```sh
runner="/Volumes/dev-scratch/274bot-worktrees/nightly-mac-quest"
git -C "$runner" fetch origin dev
sha=$(git -C "$runner" rev-parse --verify origin/dev)
pinned="/Volumes/dev-scratch/274bot-nightly-mac/origin-dev-$sha.git"
git init --bare "$pinned"
git -C "$runner" push "$pinned" refs/remotes/origin/dev:refs/heads/dev
NIGHTLY_REPO="$pinned" NIGHTLY274_ACTION=nightly-mac \
  "$HOME/.local/libexec/274bot-nightly/nightly274-launch"
```

The two job summaries land directly in `NIGHTLY/` with `-mac` and
`-mac-quest`. `NIGHTLY/.mac-nightly-running` exists while either job is active;
if a measurement creates `MAC-QUIET.lock`, current cells finish, later cells
are `NOT_RUN` with the lock path recorded, and the next job is skipped.

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
  shows the active job while it is running. Create
  `/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock` to stop it from starting
  another cell at the next boundary. Unload the schedule with
  `launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-mac.plist`;
  unload the malbec mirror similarly with
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

Edit [`branches.conf`](branches.conf) (first line = the fix line), commit, and update
the VM's runner checkout (`git -C /srv/nightly/runner pull --ff-only`). The next night
reads the new list. No unit changes are needed.

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
6. Units: copy `systemd/274bot-nightly.service` and `systemd/274bot-nightly.timer` into
   `/etc/systemd/system/`, then run `sudo systemctl daemon-reload && sudo systemctl enable --now 274bot-nightly.timer`.
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
     /srv/nightly/engine-q-pristine/engine/world.json
   sudo install -o bot -g bot -m 0755 /srv/nightly/runner/tools/nightly/engine-q/start-289q.sh \
     /srv/nightly/engine-q-pristine/start-289q.sh
   sudo install -o bot -g bot -m 0755 /srv/nightly/runner/tools/nightly/engine-q/stop-289q.sh \
     /srv/nightly/engine-q-pristine/stop-289q.sh
   sudo install -o bot -g bot -m 0755 /srv/nightly/runner/tools/nightly/engine-q/status-289q.sh \
     /srv/nightly/engine-q-pristine/status-289q.sh
   sudo -u bot sh -c 'cd /srv/nightly/engine-q-pristine/engine && npm run sqlite:migrate'
   ```
   Its independent runtime copy is `/srv/nightly/engine-q-run`; Engine Q listens
   only on `127.0.0.1:44694` (game), `1180` (HTTP), and `9988` (management).
   `world.json` selects the dedicated Q SQLite database; the runner sets
   `QUESTER_TICK_MS=300` only in quest-cell environments so the
   harness's world-wide `::speed` command cannot affect Engine A. Both trees use
   289 content but the quest database and engine process are separate.
   `/etc/274bot-nightly.env` remains main-job-only: **do not add** global
   `NIGHTLY_GAME_PORT`, `NIGHTLY_HTTP_PORT`, `NIGHTLY_PROFILE`, `NIGHTLY_RUN_SUFFIX`, or
   `QUESTER_TICK_MS` entries. The scoped `ExecStartPost` in the updated service
   sets `NIGHTLY_JOB=quest`, `NIGHTLY_ENGINE=local`, `NIGHTLY_PROFILE=local-289q`,
   the Q ports, `-quest` suffix, `nq`/`nqp` name prefixes and parallel limit 1;
   the runner sets `QUESTER_TICK_MS=300` for those quest cells only.
9. Install the updated service (no timer change):
   ```sh
   sudo install -m 0644 /srv/nightly/runner/tools/nightly/systemd/274bot-nightly.service \
     /etc/systemd/system/274bot-nightly.service
   sudo systemctl daemon-reload
   ```
   The service runs main first, then quest as a second `ExecStartPost` invocation.
   Both summaries share the systemd deadline; check each `<run>/DONE` after the
   timer fires. The quest starts/stops its own engine around its cells.

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

The Mac LaunchAgent starts at **20:00 America/New_York**. It is an evening
desktop window, six hours before the existing 02:00 Linux timer and 5½ hours
before the 01:30 standby pull; the malbec mirror itself runs hourly at minute
51, offset from the existing 274 pull at minute 41. The Aqua user agent can run
`panel-play` in its logged-in desktop session; it does not run when no user is
logged in. Before either job it checks
`/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock`, and `run_nightly.py`
checks that lock at every cell/lane boundary. While either job runs,
`/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running` contains
the PID, job and state for FLOOR controllers.

1. Write `~/.config/274bot-nightly/mac-nightly.env` with the paths for this
   merlot. The runner worktree and the two private engine source trees are local
   inputs; neither engine is started or changed by this LaunchAgent:
   ```sh
   mkdir -p ~/.config/274bot-nightly ~/.local/lib/274bot-nightly \
     ~/.local/state/274bot-nightly-mac ~/Library/LaunchAgents
   cat > ~/.config/274bot-nightly/mac-nightly.env <<EOF
   NIGHTLY_MAC_RUNNER=/Volumes/dev-scratch/274bot-worktrees/nightly-mac-quest
   NIGHTLY_MAC_ROOT=/Volumes/dev-scratch/274bot-nightly
   NIGHTLY_MAC_RUNS=/Volumes/dev-scratch/274bot-evidence/NIGHTLY
   NIGHTLY_MAC_STATUS=/Volumes/dev-scratch/274bot-evidence/NIGHTLY/.mac-nightly-running
   NIGHTLY_MAC_CATALOG=$HOME/experiments/rs2b0t
   NIGHTLY_MAC_ENGINE_A=$HOME/experiments/lostcity-289
   NIGHTLY_MAC_ENGINE_Q=$HOME/experiments/lostcity-289-q
   NIGHTLY_MAC_QUIET_LOCK=/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock
   NIGHTLY_MAC_ISOHOME=$HOME/experiments/274bot/.superpowers/bin/isohome
   NIGHTLY_MAC_PYTHON=/opt/homebrew/bin/python3
   EOF
   chmod 600 ~/.config/274bot-nightly/mac-nightly.env
   ```
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
4. After Main has installed the restricted public key on malbec and both env
   files are ready, bootstrap the schedules:
   ```sh
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-mac.plist
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/local.274bot-nightly-malbec-pull.plist
   launchctl print gui/$(id -u)/local.274bot-nightly-mac
   launchctl print gui/$(id -u)/local.274bot-nightly-malbec-pull
   ```
   The existing launcher already has merlot's removable-volume privacy grant,
   so these additions do not create a new launcher binary or prompt. If that
   grant is unexpectedly requested, stop and have the operator inspect it
   before approving anything. `RunAtLoad=false` keeps installation from
   starting an unscheduled nightly or pull.

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
   when it is missing or stale), `fixtures\bake-r2-a.navpack` (the script lanes'
   `NAV_PACK`, sha256 `4270f027…`) and
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
- Cells: [`cells-274.conf`](cells-274.conf), 103 of 244. Its header
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
   `-D warnings`), `test-workspace`, `test-vendor` (`ENGINE_DIR` = the engine copy),
   `test-script-load` and `test-script-no-default` (`NAV_PACK` = the fixture),
   `test-host-play-memory`, `test-host-play-memory-no-alloc`, `test-host-performance`,
   `test-panel-memory` (every target, including the desktop-only `ime_frame`),
   `test-panel-render-diagnostics`, `test-tui-memory`, and `cargo fmt --check` per
   crate (a workspace-root fmt hits OS error 206 on Windows). Each lane gets its own
   throwaway `HOME`/`USERPROFILE`/`APPDATA`/`LOCALAPPDATA`/`TEMP` with a copy of the
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
(`NIGHTLY_CELL_CAP_S`). No step runs past the night's deadline either (8.5 h, so a
23:00 start ends by 07:30, before Engine B's busy morning and well inside the task's
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
