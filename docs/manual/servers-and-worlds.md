# Choose a game server and a world

> **Who this is for:** you want to start 274bot on another server, pick the world
> each account uses, or run against a game server on your own computer. When you
> finish, you know which server the app is on and which world each bot logs in
> to.

## See which server you are on

Open **General config** in the panel. The line **Server:** names it, for example
`rs2b2t · w1.rs2b2t.com:443 · revision 289`. Under it, `revision fixed for this
session` tells you the server cannot change until you restart. The title of the
panel shows the revision too, for example `289bot`.

## Start on a server

Name the server with `--profile`:

```
./panel-play --profile rs2b2t
./panel-play --profile local-289
./tui-play --profile rs2b2t
```

On Windows, write `.\panel-play.exe`. The macOS app starts on rs2b2t by itself.

| Way to name a server | Example |
| --- | --- |
| `--profile NAME` | `--profile local-289` |
| `--rs2b2t`, the short form for `--profile rs2b2t` | `./panel-play --rs2b2t` |
| The environment variable `BOT_SERVER_PROFILE` | `BOT_SERVER_PROFILE=rs2b2t` |

`public-289` is a second name for `rs2b2t`. You cannot give `--rs2b2t` and
`--profile` together. 274bot reads the names without regard to capital letters.

If you name no server, 274bot uses `local-274` or `local-289`. It takes the
revision you gave with `--revision 274` or `--revision 289` (or `BOT_REVISION`).
Without one, the panel uses the revision you last chose in **General config**,
and otherwise 274. Both servers need a local engine (see
[Use a local engine](#use-a-local-engine)). With a named server, `--revision`
must match the server's revision.

## The built-in servers

| Server | Revision | Connects to | Vault file |
| --- | --- | --- | --- |
| `rs2b2t` | 289 | Worlds w1, w2 and w3 at `w1.rs2b2t.com`, `w2.rs2b2t.com` and `w3.rs2b2t.com`, port 443, encrypted | `vault-prod` |
| `local-289` | 289 | `127.0.0.1:44594`, game files from port 1080 | `vault-289` |
| `local-274` | 274 | `127.0.0.1:43594`, game files from port 80 | `vault` |

All vault files are in the `.274bot` folder in your home folder.

Revision 289 is the tested version of the game. Revision 274 may run but is not
tested. 274bot ships no game files. It downloads them from the server you choose
and reuses them at later starts.

## Choose a world

The server rs2b2t has three worlds. Each profile uses **auto** or a pinned world.

- **auto:** the bot logs in to w1. When a world is full, it tries the next world
  at once. It waits only after every world was full in one round.
- **pinned world** (w1, w2 or w3): the bot always uses it. If the world is full,
  the bot waits and tries that world again.

To set the world of a profile:

1. Click **Profiles**, then **Edit** next to the profile.
2. Open the list **world (next slot start)** and pick **auto**, **w1**, **w2** or
   **w3**.
3. Click **Save**. The bot uses the world the next time it logs in.

The list appears only for a server with a world list, such as rs2b2t. The
servers `local-274` and `local-289` have one world and no list.

To move a bot that is logged in, click **Switch world** in the **Fleet** toolbar,
or click the world marker on its tile. A list of `World 1`, `World 2` and
`World 3` opens. After you pick one, the Fleet window shows a prompt such as
`Move 1 bot to world 2: alice. Each logs out and back in through the login queue;
a running script continues after it reconnects.` Click **Confirm** to run it.
A bot that is queued for login, still logging in, not logged in or already on
that world is skipped, and the prompt says why.

In the terminal app, press `W` in the Fleet pane to open the same list.
`tui-play --world 2` sets the starting world for profiles that are set to
**auto**. Pinned worlds stay as they are. The panel has no such option.

The first start downloads the game files from the first world that answers. If a
world cannot be reached, 274bot tries the next one.

## Members worlds

A members world is a world that allows members areas. On a free world, 274bot
does not route through members areas. If you pick such a destination in
**WalkTo**, it says `This route requires a members' world`.

- On rs2b2t, every world counts as a members world. This holds only while the
  world list holds just rs2b2t worlds on port 443.
- On a server that uses a local engine, 274bot reads the engine's
  `data/config/world.json`. It uses the file only when it names the same
  revision. Without a usable file, the world counts as a free world.
- Any other server, and a list that mixes rs2b2t worlds with others, counts as a
  free world.

To say what the world is yourself, start with `--world-members true` or
`--world-members false`. Your choice wins over everything else.

## Use a local engine

The servers `local-289` and `local-274` connect to a Lost City game server on
your own computer, the local engine. 274bot needs the folder of the local
engine. It takes the first of these that you give:

1. `--engine FOLDER` on the command line.
2. The environment variable `ENGINE_DIR`.
3. The `engine_dir` entry of the server in `servers.json`.

274bot never guesses the folder from your home folder. Without one, it stops with
`local profile "local-289" needs an engine directory; set ENGINE_DIR, pass --engine, or configure login_key.engine_dir in servers.json`.

The panel treats a server as local only when it connects without encryption and
every game host is on your own computer. The **Debug** section of the panel, with
its cheat commands, appears only for such a server. rs2b2t has no **Debug**
section. How to set up a local engine and where the game files are stored is in
[Cache and local engine](../dev/cache-and-local-engine.md).

## Edit the server list

The list of servers is the file `servers.json` in the `.274bot` folder. 274bot
creates it the first time it starts, with the three servers above, and reads it
at every start. The file has `"schema_version": 1` and a list of servers. Each
server has a `name`, a `revision`, a `transport`, `worlds`, a `login_key`, a
`vault` file name and an optional `members` entry. To point a server at your game
server's folder, add it to the server's `login_key`:

```json
"login_key": { "engine_dir": "/path/to/engine" }
```

Two servers cannot share a name or a vault file. Every world needs a `number`, a
`host`, a `port`, a `node_id`, an `asset_host` and an `asset_port`. A server that
connects without encryption to a host that is not on your own computer needs
`"allow_plaintext_offhost": true`.

## When something goes wrong

| What you see | Why | What to do |
| --- | --- | --- |
| `unknown server profile "rs2b2" in servers.json` | The name is not in `servers.json`. | Check the spelling against the table above. |
| `--rs2b2t conflicts with --profile` | You gave both. | Give only one. |
| `--revision conflicts with the selected --profile` | The revision does not match the server. | Drop `--revision`, or pick the server for that revision. |
| `local profile "local-289" needs an engine directory; …` | The server has no local engine folder. | Give `--engine`, set `ENGINE_DIR`, or use `--profile rs2b2t`. |
| `--world requires the rs2b2t profile` | `tui-play --world` works only with rs2b2t. | Drop `--world`, or use `--profile rs2b2t`. |
| `world 4 is not in the configured public worlds` | The world number is not in the server's world list. | Use a number from the list: 1, 2 or 3. |
| `non-loopback tcp requires allow_plaintext_offhost in the selected profile` | A server that is not encrypted points at another computer. | Add `"allow_plaintext_offhost": true` to that server, or use an encrypted server. |
| `This route requires a members' world` | The route needs members areas and the world is free. | Use a members world, or pick another route. |
