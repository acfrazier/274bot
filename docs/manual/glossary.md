# Glossary

> **Audience:** anyone who uses 274bot. Each word below has one meaning in the
> docs. If a page uses a different word for the same thing, the page is wrong.

| Term | What it means | Do not use |
| --- | --- | --- |
| **profile** | One saved account: username, game password, world and settings. You edit profiles in the **Profiles** window. | slot, head, client, user |
| **bot** | A profile that is loaded and running in the app. | slot, head, client |
| **vault** | The encrypted file that holds your profiles. | store, wallet |
| **passphrase** | The text that unlocks the vault. It is not the game password. | master password, vault password |
| **game password** | The password of the game account. It is stored in the vault. | account password |
| **server** | A game server entry from `servers.json` that you pick with `--profile`: `local-274`, `local-289` or `rs2b2t`. | server profile |
| **world** | One game world on a server, such as w1, w2 or w3. A profile uses one world or **auto**. | node |
| **revision** | The version of the game: 289 (tested) or 274 (may work, not tested). | build, protocol |
| **local engine** | A game server that you run on your own computer. | local server |
| **script** | Anything you can **Start** on a bot. | card (unless you quote a label) |
| **built-in script** | A script that comes with 274bot: Quester, Gatherer, Grinch, Sherlock. | compiled card, native script |
| **catalog script** | A script from your rs2b0t folder (`$RS2B0T` or `--catalog`). | compat script |
| **file script** | A `.js` or `.ts` script that you open with **Load**. | file card |
| **quest guide** | The step list for one quest that the Quester follows. Guides that are not fully tested are marked `[draft]`. | Path |
| **Fleet** | The bots you control together, in the **Fleet** window and its toolbar. | wall |
| **mark** | The tick that picks a bot for a Fleet action. The marked bots you send to one place are a **group**. | selection |
| **MultiBox** | The view that shows many bots at once, as a **rail** (a side strip) or a **grid**. | wall, sidecar |
| **panel** | The windowed app, `panel-play`. | GUI, operator window |
| **TUI** | The terminal app, `tui-play`. It suits a server with no screen. | headless panel |
| **WalkTo** | The map where you pick a destination and send a bot or a group. | map picker |

## Words for the same thing

- Say **random-event handling**, not "guardian".
- Say **navigation data**, not "nav pack", unless you are quoting the `nav-pack` tool.
- **Danger routing** has three levels: **Never**, **When survivable** and **Always**. **When survivable** acts as **Never** until the bot can track poison.
- Use the button names as the app shows them: **Log in**, **Start**, **Pause**,
  **Resume**, **Stop**, **Browse…**, **Load**, **Reload**. The panel's login row
  says **Logout**; the Fleet actions say **Log out**.
