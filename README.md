# 274bot

**Run a whole fleet of game bots from one window.** 274bot is a bot host for a
2005-era online game (revision 289; revision 274 may work but is not tested).
You add your accounts, pick a script, and 274bot plays them for you: gathering,
quests, clue trails and walking across the map. It runs on macOS, Windows and
Linux. It is in **beta**.

It comes with four built-in scripts:

- **Quester** follows step-by-step guides for quests.
- **Gatherer** cuts trees, mines rock and fishes, and can bank what it collects.
- **Grinch** picks up holiday items that lie on the ground near where you start it.
- **Sherlock** solves clue trails.

It also runs scripts written for [rs2b0t](https://github.com/rs2b2t/rs2b0t),
so you can bring the scripts and the accounts you already have.

## Download

Get the package for your computer from the
[Releases page](https://github.com/acfrazier/274bot/releases/latest):

| System | Package |
| --- | --- |
| macOS (Apple Silicon, macOS 11 or newer) | `274bot-<version>-macos-arm64.zip` |
| Windows (64-bit) | `274bot-<version>-windows-x64.zip` |
| Linux (64-bit) | `274bot-<version>-linux-x64.tar.gz` |

You do not need Rust, a game server of your own or any other tool.

## Your first run

These steps use the public server, **rs2b2t**. You need your own account on it.
[FIRST-START.md](FIRST-START.md) has the same steps with notes for each system.

1. **Unpack the package.** Keep its folders together: the app looks for `nav/`
   and `map/` next to it.
2. **Open the app.** On macOS, open `274bot.app`. On Windows and Linux, open a
   terminal in the folder and run `./panel-play --profile rs2b2t`
   (`.\panel-play.exe --profile rs2b2t` on Windows). The first start downloads the
   game files and shows a loading bar. Later starts reuse them.
3. **Create your vault.** The vault is the encrypted file that holds your
   accounts. In the panel on the right, type a passphrase into **new vault passphrase**
   and click **Create vault**. Keep the passphrase safe. If you lose it, the only
   way back in is **Reset vault**, which deletes your saved accounts.
4. **Add an account.** Click **Profiles**, then **New profile**. Type your game
   username and password, then click **Save**.
5. **Log in and start a script.** Click **Log in**. When the bot is in the game,
   click the closed **script** heading to open it, then click **Browse…**. If a
   window asks for your rs2b0t folder, click **Not now**. Then click a script and
   click **Start**. Open **Script prefs** first to change the script's settings.

The panel does not make an account for you. An empty vault stays empty until you
save a profile.

## Where to go next

| I want to… | Read |
| --- | --- |
| Find the page for my task | [docs/README.md](docs/README.md) |
| Fix a first-start problem | [FIRST-START.md](FIRST-START.md) |
| Learn the window and what its buttons do | [Panel guide](docs/api/panel.md) |
| Run bots in a terminal, for example on a server | [Terminal app guide](docs/api/tui.md) |
| Move my accounts over from rs2b0t | [Import your accounts from rs2b0t](docs/manual/importing-from-rs2b0t.md) |
| Use or write quest guides | [Quest guides](docs/quester-paths.md) |
| Choose a game server, world or revision | [Choose a game server and a world](docs/manual/servers-and-worlds.md) |
| Look up a word | [Glossary](docs/manual/glossary.md) |
| See what changed | [CHANGELOG.md](CHANGELOG.md) |
| Upgrade from 0.1.9.1 | [Upgrade guide](docs/upgrade-0.2.0.md) |
| Write a script | [Scripts](docs/api/script.md) and [JS API v2](docs/api/js-api-v2.md) |
| Build 274bot from source | [CONTRIBUTING.md](CONTRIBUTING.md) and [build notes](docs/dev/build-from-source.md) |

## What works and what does not

- Revision 289 is the tested version. Revision 274 may run, but it is not tested.
- 274bot is tested against a game server you run on your own computer. The public
  rs2b2t server has no automated tests.
- Scripts written for rs2b0t run, but not every call is supported yet. A call
  that is not supported stops the script with a `not impl` message.
- The Quester has routes for 20 of the 68 quests. More are planned.

The full list of known problems is in the [CHANGELOG](CHANGELOG.md).

## What 274bot is not

274bot is not Jagex, not the official Lost City project and not the Fairy Ring
project. It does not ship any Jagex game files: it fetches them from the server
you choose.

## Built on rs2b0t

274bot exists because of **[rs2b0t](https://github.com/rs2b2t/rs2b0t)**. If you've botted on rs2b2t, you already know what it gets right: a whole fleet of bots running from one window, a typed script API that makes writing a bot genuinely pleasant, walking that takes your bots across the map on its own, and quest and clue engines that grind through the tedious parts for you. 

274bot is an attempt at the best version of that experience: rebuilt in Rust so every bot is lighter and faster, while keeping rs2b0t's script API so its scripts can run here (see [What works and what does not](#what-works-and-what-does-not)).

A lot of 274bot comes straight from that work. The interface is modelled on rs2b0t's, drawn with ImGui instead of Electron; our quest Paths started from its AIOQuester; and parts of our game data come from it (MIT, © 2026 N64Jive; details in [NOTICE.md](NOTICE.md)). Thank you to **[everyone who has contributed to rs2b0t](https://github.com/rs2b2t/rs2b0t/graphs/contributors)**.

The Quester also owes a lot to Liza Carvelli's Questionable, a quest-automation plugin for [Dalamud](https://github.com/goatcorp/Dalamud): authored quest routes a whole fleet can run start to finish.

It also stands on **[Lost City](https://github.com/LostCityRS)**, the open 2004-era engine, content and client work every server and client here descends from, and on the **[Fairy Ring](https://github.com/Fairy-Ring/FR-client-rust)** Rust client our bot-host client is built from.

## License

274bot is open source under the [MIT license](LICENSE). Credits and third-party
licenses are in [NOTICE.md](NOTICE.md).
