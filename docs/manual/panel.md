# Use the panel window

> **Who this is for:** you run bots in the windowed app. When you finish,
> you can find every control, focus one bot, send it on foot and read the
> log. For many bots at once, see [Work with many bots](fleet-and-groups.md).
> For the app options, see [Change the settings](settings.md).

## What the window holds

The game view shows the bot you focused. The right strip holds the
controls in collapsible sections: **status**, **fleet**, **resource**,
**profile**, **script**, **debug** and **log**. The **script** section
starts closed. Click its heading to open it.

The top rows hold the buttons you use most. **Log in** connects the focused
bot, and **Logout** disconnects it. **WalkTo** toggles the map: an open map
closes. **Fleet** opens the wide view of all your bots. Below them sit
**General config**, **Nav config**, **Loadouts** and **Script prefs**, then
the **Profiles** button. **MultiBox** and **Grid** toggle the many-bots views.
+
Each of **Fleet**, **Log**, **General config**, **Nav config**,
**Loadouts**, **Script prefs** and **Profiles** opens its window. If the
window is already open, the button brings it to the front.

The **debug** section appears only when you run against your own game
server. It never appears on the public server.

## Focus one bot

Click a bot's name in the **Fleet** window to focus it. With one bot
loaded, it is already focused.

## Send one bot on foot
+
1. If the map is closed, click **WalkTo** to open it. Pick a tile.
2. Click **Send** to choose who walks. The Send window offers **Focused bot**
   and **Group**; leave it on **Focused bot** for one bot.
3. Click **Walk** to walk the focused bot there. The map closes and the bot
   walks.
+
The footer also holds **Route through danger zones**. Tick it to allow one
walk past dangerous monsters. It starts unticked each time **WalkTo**
opens. While **Danger routing** is **Always**, the tick is replaced by a
warning and changes nothing.
The walk needs a target you can stand on. A blocked tile refuses at once.

On your own server, the footer adds **Teleport**. It moves the focused bot
to the picked tile, even a blocked one.

The first time the map opens, it may ask to bake terrain with **Bake now**,
**Always bake** and **Not now**. **Bake terrain** starts the bake later.

## Read the log

The **log** section of the strip shows the messages of the focused bot.
The **Log** button beside the title row opens the same view as a dockable
tab. The tab and the section share filters, follow state and history.

To keep the log of a whole session in a file, tick **session log file** in
**General config**. While it is on, the row reads `writing <path>` under
the checkbox. The file lives under `~/.274bot/logs/`.

## When something goes wrong

| You see | Cause | What to do |
| --- | --- | --- |
| **Log in** or **Logout** stays dimmed | The vault is locked or no profile is focused. **Logout** also needs the focused bot connected, queued or guard-parked. | Unlock the vault and focus a profile. |
| The map reads `terrain not baked · catalogue only` | The ground pictures are not baked yet. | Click **Bake terrain**. |
| The panel closes at once on Linux with "importing the supplied dmabufs failed" | The computer has two graphics chips. | Start it with `WGPU_POWER_PREF=high ./panel-play`. See the [known issues](../known-issues.toml). |
