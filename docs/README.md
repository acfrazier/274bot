# 274bot docs

Find the page for your task. There are three doors: use 274bot, write scripts for
it, or work on its code.

## I want to use 274bot

| I want to… | Read |
| --- | --- |
| Install it and run it for the first time | [Install and first run](manual/install-and-first-run.md) |
| Fix a first-start problem | [Troubleshooting in FIRST-START.md](../FIRST-START.md#troubleshooting) |
| Learn the window and what its buttons do | [Use the panel window](manual/panel.md) |
| Run bots in a terminal, for example on a server | [Terminal app guide](api/tui.md) |
| Add, change or remove accounts | [Manage your accounts and log in](manual/profiles-and-accounts.md) |
| Move accounts over from rs2b0t | [Import your accounts from rs2b0t](manual/importing-from-rs2b0t.md) |
| Understand why a bot waits to log in | [The login messages](manual/profiles-and-accounts.md#read-the-login-messages) |
| Choose a game server, world or revision | [Choose a game server and a world](manual/servers-and-worlds.md) |
| Look up every setting of a server entry | [Server reference](reference/server-profiles.md) |
| Look up the exact login limits or the vault file format | [Login limits](api/login.md) and [Vault](api/vault.md) |
| Start, pause and stop scripts, and set their options | [Run scripts on your bots](manual/running-scripts.md) |
| Learn what the Quester, Gatherer, Grinch and Sherlock do, and their settings | [The built-in scripts](manual/built-in-scripts.md) |
| Control many bots together, walk them as a group, move them to another world | [Work with many bots](manual/fleet-and-groups.md) |
| Look up what each Fleet button does | [Fleet actions](reference/fleet-actions.md) |
| Change the app, walking and script options | [Change the settings](manual/settings.md) |
| Send bots across the map | [Navigation](api/nav.md) |
| Upgrade from 0.1.9.1 | [Upgrade guide](upgrade-0.2.0.md) |
| See what changed in each release | [CHANGELOG](../CHANGELOG.md) |
| Look up a word | [Glossary](manual/glossary.md) |

Some of these pages are long and mix everyday use with developer detail. Use the
headings to jump to the part you need.

## I want to write scripts

| I want to… | Read |
| --- | --- |
| Write a script and look up the script API | [JS API v2](api/js-api-v2.md) |
| See which rs2b0t scripts and calls run | [Scripts](api/script.md) |
| Write or load a quest guide for the Quester | [Quest guides](scripting/quest-guides.md) |
| Copy a working example | [Example scripts](../crates/script-js/examples/) |
| Test a script against a local game server | [Reusable live harness](dev/harness.md) |

## I want to work on 274bot

| I want to… | Read |
| --- | --- |
| Build it, test it and send a change | [CONTRIBUTING.md](../CONTRIBUTING.md) |
| Build from source and run the live tests | [Build from source](dev/build-from-source.md) |
| Run it against my own game server, or learn how the cache works | [Cache and local engine](dev/cache-and-local-engine.md) |
| Reproduce the architecture check, or change a crate dependency | [Architecture check in detail](dev/architecture-check.md) |
| Regenerate the game data the app ships with | [Generate the selected game data](dev/selected-data-generation.md) |
| Learn the crates and who owns what | [Architecture](dev/architecture.md) |
| Change navigation generation or the combined v18 schema | [Baked reachability contract](dev/baked-reachability.md) |
| Change the Quester guide format or step handlers | [Quest paths for contributors](dev/quest-paths.md) |
| Learn the terms the code uses | [CONTEXT.md](../CONTEXT.md) |
| Read the Rust API for reading and acting | [Reference pages in `docs/api`](api/README.md) |
| Run the end-to-end suite | [e2e suite](dev/e2e-suite.md) |
| Check an upgrade from 0.1.8.1 before a release | [Release gate](dev/release-gate-0.1.8.1.md) |
| See the open and deferred issues | [`known-issues.toml`](known-issues.toml) |
| Write or edit a doc | [Writing docs](dev/writing-docs.md) |

Credits and licenses: [NOTICE.md](../NOTICE.md).
