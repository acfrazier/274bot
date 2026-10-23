# Glossary

> **Audience:** anyone who uses 274bot. Each word below has one meaning in the
> docs. If a page uses a different word for the same thing, the page is wrong.
> Buttons and windows are written as the app shows them, in bold.

## Accounts and the vault

| Term | What it means | Do not use |
| --- | --- | --- |
| **profile** | One saved account: username, game password, world and settings. You edit profiles in the **Profiles** window. | slot, head, client, user |
| **bot** | A profile that is loaded and running in the app. | slot, head, client |
| **vault** | The encrypted file that holds your profiles. Each server has its own vault file. | store, wallet |
| **passphrase** | The text that unlocks the vault. It is not the game password. | master password, vault password |
| **game password** | The password of the game account. It is stored in the vault. | account password |
| **Log in**, **Log out** | The buttons that connect a bot to the game and disconnect it. The panel's login row says **Logout**; the Fleet toolbar and the terminal app say **Log out**. | login (as a verb), logon |
| **Log in automatically** | The profile option that logs a bot in when it starts and again after a disconnect. | auto-login |
| **login queue** | The line that bots wait in. Bots log in one at a time. | FIFO, admission |
| **repeat guard** | The stop that follows three unexpected drops within ten minutes. The bot stays logged out until you click **Log in**. | latch |
| **export file** | The file `rs2b0t-profiles.json` that rs2b0t writes. It holds your passwords in plain text. | profile dump |

## Servers and worlds

| Term | What it means | Do not use |
| --- | --- | --- |
| **server** | A game server entry from `servers.json` that you pick with `--profile`: `local-274`, `local-289` or `rs2b2t`. The flag is named `--profile`, but it picks a server, not a profile. | server profile |
| **rs2b2t** | The public game server. It has the worlds w1, w2 and w3. | prod, production |
| **local engine** | A game server that you run on your own computer. The servers `local-274` and `local-289` connect to it. | local server, LC server |
| **world** | One game world on a server, such as w1, w2 or w3. The profile form shows it as `w1`, and the world list shows it as `World 1`. | node |
| **auto** | The world setting that lets a bot move to the next world when its world is full. | automatic world |
| **pinned world** | A profile set to w1, w2 or w3. The bot stays on that world and waits when it is full. | fixed world |
| **members world** | A world that allows members areas. A world that does not is a **free world**. | P2P, F2P |
| **revision** | The version of the game: 289 (tested) or 274 (may work, not tested). | build, protocol |
| **game files** | The files that 274bot downloads from the server at the first start and reuses afterwards. | cache, assets |
| **navigation data** | The data that 274bot uses to find routes. It comes in the `nav` folder next to the app. | nav pack |
| `servers.json` | The file that lists your servers. It is in the `.274bot` folder in your home folder. | server list file |

## Scripts

| Term | What it means | Do not use |
| --- | --- | --- |
| **script** | Anything you can **Start** on a bot. | card (unless you quote a label) |
| **built-in script** | A script that comes with 274bot: Quester, Gatherer, Grinch and Sherlock. | compiled card, native script |
| **catalog script** | A script from your rs2b0t folder. | compat script |
| **rs2b0t folder** | Your copy of rs2b0t, whose scripts appear in **Browse…**. You set it with `$RS2B0T`, with `--catalog`, or in the window that asks for it. | catalog root, clone root |
| **file script** | A `.js` or `.ts` script that you open with **Load**. | file card |
| **quest guide** | The step list for one quest that the Quester follows. Guides that are not fully tested are marked `[draft]`. | Path |
| **Script prefs** | The window with a script's settings. The terminal app calls the same settings **Parameters**. | Params |
| **random-event handling** | How 274bot answers random events, such as the sandwich lady or the genie. You switch it with the profile option **random events**. | Guardian |
| **loadout** | A saved list of equipment and inventory items, which all profiles share. You edit it in **Loadouts**. | preset |
| **Combat** | The fighting that the Quester, Sherlock and loaded scripts use. You do not start it from **Browse…**. | combat script |

The script buttons are **Browse…**, **Load**, **Reload**, **Start**, **Pause**,
**Resume** and **Stop**.

## Bots together

| Term | What it means | Do not use |
| --- | --- | --- |
| **Fleet** | The bots you control together, in the **Fleet** window and its toolbar. | wall |
| **mark** | The tick that picks a bot for a Fleet action. The marked bots you send to one place are a **group**. | selection |
| **fleet action** | A button of the Fleet toolbar: **Start**, **Stop**, **Assign**, **Assign & restart**, **Log in**, **Log out**, **Walk**, **Switch world** and **Copy settings to**. It acts on the marked bots; with no marks, it acts on the loaded bots that the filter shows, and asks you to click **Confirm** first. | bulk action |
| **Switch world** | The fleet action that moves logged-in bots to another world. It logs each bot out and in again through the login queue. | world hop |
| **MultiBox** | The view that shows many bots at once, as a **rail** (a side strip) or a **grid**. | wall, sidecar |

## The apps and their windows

| Term | What it means | Do not use |
| --- | --- | --- |
| **panel** | The windowed app, `panel-play`. | GUI, operator window |
| **TUI** | The terminal app, `tui-play`. It suits a server with no screen. | headless panel |
| **command palette** | The list of every command in the terminal app. Press Ctrl-P or `:` to open it. | command bar |
| **General config**, **Nav config** | Panel windows for app-wide options and for walking options. **General config** also shows which server you are on. | global settings |
| **Log** | The messages from the app and from each bot. You read them in the **Log** window or in the **log** section of the panel. | console |

## Walking

| Term | What it means | Do not use |
| --- | --- | --- |
| **WalkTo** | The map where you pick a destination and send a bot or a group. | map picker |
| **Danger routing** | The option that decides whether a walk may pass dangerous monsters, with three levels: **Never**, **When survivable** and **Always**. **When survivable** acts as **Never** until the bot can track poison. | danger zones |

## Words for the same thing

- Say **random-event handling**, not "guardian".
- Say **navigation data**, not "nav pack", unless you are quoting the `nav-pack` tool.
- Use the button names as the app shows them: **Log in**, **Start**, **Pause**,
  **Resume**, **Stop**, **Browse…**, **Load**, **Reload**. The panel's login row
  says **Logout**. The Fleet actions say **Log out**.
