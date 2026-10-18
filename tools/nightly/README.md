# Nightly live run

Every night a dedicated Linux VM builds `tui-play` for each branch in
[`branches.conf`](branches.conf) and runs every live cell of the scenario
catalog against its own private 289 engine. A Windows host does the same for
`dev` natively on Windows: the CI test lanes with the GPU on, every cell
through `panel-play` and every cell through `tui-play` on ConPTY, against
Engine B (see [Windows nightly](#windows-nightly)). The archive host pulls each
finished run into ZFS and snapshots it. A review machine mirrors the last
7 days.

```
nightly VM (runs, writes only its own disk)
   /srv/nightly/runs/<run>/            <- run_nightly.py, 02:00 America/New_York
        |  read-only rsync, pulled by the archive host (hourly, :10)
        v
Windows nightly host (BotTest console session, writes only C:\274bot-nightly)
   C:\274bot-nightly\runs\<run>-windows\  <- run_nightly.py, 23:00 America/New_York
        |  read-only chrooted SFTP, pulled by the archive host (hourly, :15)
        v
archive host  tank/274bot-nightly/<run>/   + snapshot tank/274bot-nightly@<run>
        |  read-only rsync, pulled by the review machine (hourly, :25)
        v
review machine  <mirror folder>/<run>/  (last 7 days; older runs pruned there only)
```

`<run>` is `<date>-<branch>-<commit>`, for example
`2026-10-14-release-0.2.3-fb43e9b9a`; Windows runs end in `-windows`
(`2026-10-15-dev-1fc2e3e31-windows`), so one commit's runs on the two platforms
never collide on the dataset. `<date>` is the America/New_York date
the run started; `/` in the branch name becomes `-`. A second run of the same
branch and commit on the same date gets `-r2`, `-r3`, ….

## What runs

[`run_nightly.py`](run_nightly.py) (Python 3 standard library only; on Windows also
`pywinpty`) does this for each branch, in the order of `branches.conf`. This is the
Linux VM; the Windows host runs the same script with three steps instead of one
(see [Windows nightly](#windows-nightly)):

1. Fetches the branch tip anonymously from the public repository into
   `/srv/nightly/trees/<branch>` and checks it out detached with its submodule
   (`git clean -ffdx`; the build cache lives apart in `/srv/nightly/target/<branch>`).
2. Builds `tui-play` with `cargo build --locked --release -p tui --bin tui-play`.
   The navigation bake needs the engine (`ENGINE_DIR`) and a decoded client-cache
   snapshot (`BOT_NAV_SNAPSHOT_ROOT=/srv/nightly/nav-snapshot`). When the snapshot
   is missing, or the bake reports it stale, the runner makes it again: it builds
   a navigation-free `tui-play` (`BOT_NAV_BUILD=skip`, `target/navskip`), lets one
   `walk` cell log in to a fresh engine and keeps the snapshot the client decoded.
3. Reads the cell list from the checked-out tree: every name in the `REGISTRY`
   table of `crates/scenario/src/catalog.rs`. New cells join when they merge. If
   that table's format changes, the run reports the parse error instead of
   running a partial list.
4. Copies `/srv/nightly/engine-pristine` (the 289 engine and content with an empty
   account database and no player saves) over `/srv/nightly/engine-run`, and starts
   it with its own `start-289.sh` (loopback ports 1080/44594/9898).
5. Runs each cell as
   `tui-play --profile local-289 --engine <engine-run>/engine --catalog /srv/nightly/rs2b0t --live script_<cell>`
   on a pseudo-terminal, 4 at a time (`--parallel`, `NIGHTLY_PARALLEL`). Every cell
   gets a throwaway `HOME` and `TMPDIR`, `BOT_CPU=1` and `BOT_LIVE_NAME_PREFIX=nt`
   (accounts are minted as `nt<token>_<i>`). Each cell enforces its own scenario
   deadline inside `tui-play`; the runner only adds an outer cap
   (`NIGHTLY_CELL_CAP_S`, default 3600 s) for a process that never reaches a
   terminal state. A paired proof (two driven slots) refuses to start without
   the pair witness and says so (`… only qualifies under the pair core gate`);
   the runner then starts it again with `--pair-core` and records
   `"gate": "--pair-core"` in its row.
6. Stops the engine, copies its log, and writes `summary.json`, `summary.md` and
   then `DONE`.

Cells listed in [`skip.conf`](skip.conf) are not started. They still appear in the
summary as `SKIP` with the reason given there. Put cells that need a different
engine (for example Engine Q at a 300 ms tick) in that file.

The runner holds `/srv/nightly/.nightly.lock`, so two runs never overlap. It removes
local runs older than 14 days (`NIGHTLY_KEEP_DAYS`). The archive keeps all runs.

## Reading a run

```
<run>/
  summary.md          table: cell, result, ticks, wall time, first failure line, commit
  summary.json        the same rows plus run metadata (platform, host, branch, commit,
                      build, engine, catalog commit, runner commit, tui-play sha256, counts,
                      steps with their status and deadline)
  build.log           git and cargo output
  engine.log          the engine's log for this run (Linux; Engine B's log is not copied)
  engine-control.log  engine start/stop output (Linux)
  cells/<cell>/       one tui-play cell:
    result.json       this cell's summary row
    receipt.json      the scenario's JSON PASS/FAIL record (when it printed one)
    stderr.log        tui-play stderr (the `FAIL: live <cell> {…}` line and `FAIL: <message>`)
    tty.log           terminal transcript without escape codes (the `PASS: live <cell> {…}` line)
    command.txt       the exact command line
    session-logs/     logs tui-play wrote under its throwaway HOME
    evidence/         LIVE_EVIDENCE_DIR captures, when the cell writes any
  panel-cells/<cell>/ Windows: one panel-play cell, the same files with stdout.log for tty.log
  lanes/<lane>/       Windows: one CI lane: result.json (exit code, summed test counts,
                      failed test names) and cargo.log
  DONE                finish time; written last

Results:

| Result | Meaning |
|---|---|
| `PASS` / `FAIL` | the scenario printed its JSON record with that outcome; `first_failure` is its `message`. A lane: `cargo` exited 0 / nonzero |
| `TIMEOUT` | no record before the outer cap or the step deadline; the process tree was killed |
| `ERROR` | the frontend exited without a record (startup error, panic); `first_failure` is the first error-looking stderr line |
| `UNAVAILABLE` | the cell or lane could not run on this host tonight: `reason` says why (Windows: BotTest console session not active, Engine B not reachable, not enough free disk space, or a paired proof that panel-play has no gate for) |
| `SKIP` | listed in `skip.conf`; `reason` says why |
| `NOT_RUN` | the build failed, the step deadline passed before it started, or the run was interrupted; `reason` says which |

Every row has `platform` (`linux-x86_64`, `windows-x86_64`), `branch` and `commit`;
cell rows also have `frontend` (`tui` or `panel`) and `renderer` (`cpu` for `BOT_CPU=1`,
`gpu`). Every summary also has `os` and `host`, so runs from both platforms share the
archive.

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
  --branch release/0.2.3 --cells shop_buyout_aubury,walk --parallel 2
python3 runner/tools/nightly/run_nightly.py --list --tree trees/release-0.2.3   # cell list only
```

Settings can also go in `/etc/274bot-nightly.env` on the VM (read by the unit):
`NIGHTLY_PARALLEL`, `NIGHTLY_CELL_CAP_S`, `NIGHTLY_BRANCHES` (space-separated;
overrides `branches.conf`), `NIGHTLY_KEEP_DAYS`, `NIGHTLY_NAME_PREFIX`.

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

Pull at once instead of waiting for the hour:
`systemctl start 274bot-nightly-pull.service` (Windows runs:
`274bot-nightly-pull-windows.service`) on the archive host, then
`systemctl --user start 274bot-nightly-mirror.service` on the review machine.

## Stop it

- Stop tonight's run: `sudo systemctl stop 274bot-nightly.service` on the VM. Running
  cells are killed, the engine is stopped, and the partial run is still summarized
  (`"interrupted": true`) and archived.
- Stop the nightly: `sudo systemctl disable --now 274bot-nightly.timer` on the VM.
- Stop archiving: `systemctl disable --now 274bot-nightly-pull.timer` on the archive host.
- Stop mirroring: `systemctl --user disable --now 274bot-nightly-mirror.timer` on the
  review machine.
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

### Windows nightly host (BotTest, no elevation)

Every step runs as BotTest without the admin token, so the nightly keeps working if
BotTest is demoted to a standard user. Only the pull account (step 8) needs an
administrator, once.

1. Prerequisites already on the host: the Rust toolchain from `rust-toolchain.toml` in
   `C:\Users\BotTest\.cargo`/`.rustup` with the MSVC build tools, Git for Windows,
   NASM in `C:\Users\BotTest\274bot-tools\nasm-3.02`, CPython 3.14 in
   `C:\Program Files\Python314`. The host clock is set to Eastern time (the runner uses
   the host clock: Windows CPython has no tz database).
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

## Keys

| Key | Lives on | Accepted by | Can do |
|---|---|---|---|
| `274bot-nightly-pull` | archive host `/etc/274bot-nightly/pull_ed25519` | VM user `bot` | read-only rsync of `/srv/nightly/runs/` from the archive host's address; no shell, no forwarding |
| `274bot-nightly-win-pull` | archive host `/etc/274bot-nightly/win_pull_ed25519` | Windows host user `274bot-pull` (`%ProgramData%\ssh\274bot_pull_authorized_keys`) | read-only SFTP of `C:\274bot-nightly\runs\` (chroot) from the archive host's address; no shell, no forwarding, no TTY |
| `274bot-nightly-mirror` | review machine `~/.ssh/274bot_nightly_mirror_ed25519` | archive host root | read-only rsync of `/tank/274bot-nightly/` from the review machine's address; no shell, no forwarding |

The VM and the Windows host clone only public repositories, anonymously
(`credential.helper` empty, no terminal prompt), and hold no push credentials. The
Windows host gets Engine B from the review machine's reverse forward; it holds no key
to the builder VM.

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
start`.

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
`NIGHTLY_TARGET_CAP_GB` (40 on this host). If the drive still has less free
space than `NIGHTLY_MIN_FREE_GB` (100 on this host), it removes the other
target folders too, and it logs one line per removal with the sizes. If free
space is still below the floor after that, the whole night is `UNAVAILABLE`
with that reason and nothing is built, so the nightly cannot fill the disk.
A setting of 0 turns that check off. Both checks are off by default on
Linux, whose disk is 100 GB in total and whose tree path is already stable.
They run when the variables are set there.

**What persists between nights:** BotTest's `CARGO_HOME` and `RUSTUP_HOME`, the build
caches `target\<branch>` and `target\<branch>-ci` (compiled artefacts only, subject
to the disk guard above), the bare mirrors, the read-only inputs (`engine-pristine`,
`rs2b0t`, `nav-snapshot`, `fixtures`), `runs\` (14 days), the branch's tree
`trees\<branch>`, `logs\` (30 days), the `state\` markers, the venv and the runner
checkout. `engine-run\` (a fresh copy of `engine-pristine` per step) and `work\`
(per-cell and per-lane homes, removed after each) carry nothing to the next night.
