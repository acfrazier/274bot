# Nightly live run

Every night a dedicated VM builds `tui-play` for each branch in
[`branches.conf`](branches.conf) and runs every live cell of the scenario
catalog against its own private 289 engine. The archive host pulls each
finished run into ZFS and snapshots it. A review machine mirrors the last
7 days.

```
nightly VM (runs, writes only its own disk)
   /srv/nightly/runs/<run>/            <- run_nightly.py, 02:00 America/New_York
        |  read-only rsync, pulled by the archive host (hourly, :10)
        v
archive host  tank/274bot-nightly/<run>/   + snapshot tank/274bot-nightly@<run>
        |  read-only rsync, pulled by the review machine (hourly, :25)
        v
review machine  <mirror folder>/<run>/  (last 7 days; older runs pruned there only)
```

`<run>` is `<date>-<branch>-<commit>`, for example
`2026-10-14-release-0.2.3-fb43e9b9a`. `<date>` is the America/New_York date
the run started; `/` in the branch name becomes `-`. A second run of the same
branch and commit on the same date gets `-r2`, `-r3`, ….

## What runs

[`run_nightly.py`](run_nightly.py) (Python 3 standard library only) does this for each branch, in
the order of `branches.conf`:

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
                      build, engine, catalog commit, runner commit, tui-play sha256, counts)
  build.log           git and cargo output
  engine.log          the engine's log for this run
  engine-control.log  engine start/stop output
  cells/<cell>/
    result.json       this cell's summary row
    receipt.json      the scenario's JSON PASS/FAIL record (when it printed one)
    stderr.log        tui-play stderr (the `FAIL: live <cell> {…}` line and `FAIL: <message>`)
    tty.log           terminal transcript without escape codes (the `PASS: live <cell> {…}` line)
    command.txt       the exact command line
    session-logs/     logs tui-play wrote under its throwaway HOME
    evidence/         LIVE_EVIDENCE_DIR captures, when the cell writes any
  DONE                finish time; written last
```

Results:

| Result | Meaning |
|---|---|
| `PASS` / `FAIL` | the scenario printed its JSON record with that outcome; `first_failure` is its `message` |
| `TIMEOUT` | no record before the outer cap; the process group was killed |
| `ERROR` | `tui-play` exited without a record (startup error, panic); `first_failure` is the first error-looking stderr line |
| `SKIP` | listed in `skip.conf`; `reason` says why |
| `NOT_RUN` | the build or the engine failed, or the run was interrupted; `reason` says which |

Every row has `platform` (for example `linux-x86_64`), `branch` and `commit`. Every
summary also has `os` and `host`, so runs from other platforms can share the archive.

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

Pull at once instead of waiting for the hour:
`systemctl start 274bot-nightly-pull.service` on the archive host, then
`systemctl --user start 274bot-nightly-mirror.service` on the review machine.

## Stop it

- Stop tonight's run: `sudo systemctl stop 274bot-nightly.service` on the VM. Running
  cells are killed, the engine is stopped, and the partial run is still summarized
  (`"interrupted": true`) and archived.
- Stop the nightly: `sudo systemctl disable --now 274bot-nightly.timer` on the VM.
- Stop archiving: `systemctl disable --now 274bot-nightly-pull.timer` on the archive host.
- Stop mirroring: `systemctl --user disable --now 274bot-nightly-mirror.timer` on the
  review machine.

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

The pull copies only runs that have `DONE` and are not on the dataset yet. It copies
without symlinks (`rsync -rt`) into `.incoming-<run>`, renames the copy into place,
makes it read-only, and takes `tank/274bot-nightly@<run>`. A run on the dataset is
never rewritten.

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

User timers run only while the user has a session, unless lingering is enabled
(`loginctl enable-linger`). `Persistent=true` catches up a missed hour at the next login.

## Keys

| Key | Lives on | Accepted by | Can do |
|---|---|---|---|
| `274bot-nightly-pull` | archive host `/etc/274bot-nightly/pull_ed25519` | VM user `bot` | read-only rsync of `/srv/nightly/runs/` from the archive host's address; no shell, no forwarding |
| `274bot-nightly-mirror` | review machine `~/.ssh/274bot_nightly_mirror_ed25519` | archive host root | read-only rsync of `/tank/274bot-nightly/` from the review machine's address; no shell, no forwarding |

The VM clones only public repositories, anonymously, and has no push credentials.

## A Windows runner (later phase)

The runner is platform-neutral: `summary.json` and every row carry `platform`, and the
archive and mirror only move directories. A Windows lab VM would need:

- the Rust toolchain from `rust-toolchain.toml`, Git, Python 3, and the build
  prerequisites from `docs/` for Windows (MSVC build tools);
- its own 289 engine (Node 26, the engine and content copy, `npm ci`, empty
  database), started and stopped by a PowerShell equivalent of
  `start-289.sh`/`stop-289.sh`, because those scripts use `lsof`, `ps` and `nohup`;
- a console host for `tui-play` instead of `pty` (ConPTY, for example through
  `pywinpty`), plus `USERPROFILE`/`HOME`/`TEMP` set per cell;
- a Task Scheduler job in place of the systemd timer;
- OpenSSH Server with a restricted pull key. `rrsync` does not exist on Windows,
  so the archive host would pull over SFTP with a chrooted, read-only account,
  or the VM would expose the runs folder read-only some other way;
- a distinct `<run>` name, for example `<date>-<branch>-<commit>-windows`, so runs
  for the same commit on two platforms do not collide on the dataset.
