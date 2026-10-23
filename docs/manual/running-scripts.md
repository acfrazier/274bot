# Run scripts on your bots

> **Who this is for:** you use 274bot and have a bot in the game. When you
> finish, you can pick a script, change its settings, start it, pause it and
> stop it. You can also load a script file of your own and use scripts from your
> rs2b0t folder.

The scripts that come with 274bot are described in
[Built-in scripts](built-in-scripts.md).

## Open the script controls

The panel keeps its script buttons in the **script** section on the right. The
section is closed at first. Click the heading **script** to open it.

In the terminal app, press F5 to show the **Script** screen. Its first line reads
`script: idle   sel: —` while the bot has no script and you have picked none.

## Start a script on one bot

1. Click **Browse…**. The **Scripts** window opens. If you have no rs2b0t folder
   yet, another window asks for it. Click **Not now** if you do not use rs2b0t.
2. Click the script you want. Its name appears in the **script** section. If it
   differs from the saved script, `(pending)` appears beside it. The script has
   not started yet.
3. Optional: click **Script prefs** to change the script's settings. See
   [Change a script's settings](#change-a-scripts-settings).
4. Click **Start**. The **status** line shows `starting`. It then shows
   `running` or a phase such as `working`.

In the terminal app, press `b` to open the list. Press Up or Down to pick a
script. Press Enter to close the list; the pick stays. Press `t` to start.

When a start works, 274bot saves the script on the profile. If you pick no other
script, **Start** runs the saved one.

## What the buttons do

| Panel button | Terminal app | What it does |
| --- | --- | --- |
| **Browse…** | `[Browse b]` | Opens the list of scripts so you can pick one. The panel dims it while a script is active. |
| **Load** | `[Load f]` | Opens a file window to load a script file. The panel dims it while a script is active. |
| **Reload** | `[Reload R]` | Reads a changed script file again. It works for file scripts and catalog scripts only. |
| **Start** | `[Start t]` | Starts the picked script on the bot you selected. The panel dims it while a script is active. |
| **Pause**, then **Resume** | `[Pause P]` | Holds the script, then lets it go on. The button reads **Resume** while the script is paused. |
| **Stop** | `[Stop e]` | Ends the script. It also cancels a start that is still waiting. |
| **Script prefs** | `[Params v]` | Opens the settings of the picked script. The terminal app calls this window **Parameters**. |
| Fleet toolbar **Start**, **Stop** | `[Start fleet T]`, `[Stop fleet E]` | Start or stop the scripts of many bots. See [Start and stop many bots](#start-and-stop-many-bots). |

## Change a script's settings

Click **Script prefs**. The **Script prefs** window shows the settings of the
picked script for the selected profile. Each profile keeps its own settings.

A checkbox or a choice list saves as soon as you change it. A text or number
field saves when you press Enter or leave the field. Press Esc to undo an edit
you have not saved.

Some settings apply to a running script at once. Others apply at its next start.
When a restart is needed, the window says `Restart required to apply saved
settings.` In the MultiBox rail, the bot's row shows `[restart]`. A window can
also ask, for example, `Restart Gatherer on alice to apply the new settings?`
Click **Restart** or **Later**.

A script can have a **Walk permissions** group. It holds permissions for
teleports, the wilderness and danger zones on that script's own walks. Some
scripts show them by their ids, such as `allowTeleports`. If a permission is
already on in **Nav config**, the window says it is inherited, and the script
cannot turn it off.

**Apply to same-card members…** copies the settings of this script to the other
bots that use the same script. A confirmation names the bots first.

In the terminal app, press `v` for **Parameters**. Press Up or Down to move and
Space to change a choice or a checkbox. Press Enter to edit a field or to open a
choice list. Press `a` to copy the settings to the other bots, and Esc to close.

If a script has no settings, **Script prefs** is dimmed. Its tooltip reads
`selected script has no parameters`.

## Pause, resume and stop

Click **Pause** to hold the script. Click **Resume** to let it go on. Click
**Stop** to end it. **Pause** works only while the script is starting, running or
paused.

Your own movement can also pause a script. Clicking in the game view or on the
minimap counts as movement. Choosing an entry such as Walk or Talk-to from the
world menu also counts. Choosing **Examine** or **Cancel** from that menu does
not. In the terminal app, a manual step counts.

With **Pause script on manual movement** on (the default), such a click pauses
any running file script or catalog script. It also pauses a built-in script
while that script walks or acts, for example when it fights or gathers. Click
**Resume** to continue. With the setting off, your click still cancels the walk
of the script, but some scripts that act in place carry on. The
[known issues](../known-issues.toml) name the scripts this affects.

Find the setting in the **Nav config** window of the panel. In the terminal app,
press `o` on the Overview screen to open the settings.

## Load a script file of your own

You can load a script that you wrote or downloaded. The file ends in `.ts` or
`.js`.

1. Click **Load**. The **Load script** window opens.
2. Open the folder that holds the file. Click the file, then click **Load**. A
   double click on the file does the same.
3. The window closes and the script is picked. Click **Start**.

The terminal app has `[Load f]`. Pick a file in its list and press Enter.

274bot remembers the files you load. A file that is gone at the next start is
dropped. A file over 8 MiB is refused. In the panel, its entry in **Browse…** reads
`[JS] [File]`.

After you change the file, click **Reload**. If the file is the same,
`Nothing changed; nothing to reload` appears. If bots run the script, a warning
reads `reload will replace running bots` and lists them. Running bots restart.
Paused bots stop. Click **Confirm**, or click **Cancel reload** to keep things as
they are.

## Use scripts from your rs2b0t folder

274bot lists the scripts of your rs2b0t folder in **Browse…**. In the panel, their
entries read `[JS] [Catalog]`. The folder must hold `src/bot/scripts/index.ts`.

274bot finds the folder in this order: the `RS2B0T` environment variable, then
the folder that you chose last time. You can also pass `--catalog` on the command
line. In the folder window, **Use this folder** appears when the folder is right.
Click **Not now** to skip the question.

If you changed the folder, click **Refresh catalog** at the top of the
**Scripts** window. If nothing changed, `Nothing changed.` appears. Some
scripts fail to load. The window then shows a red `N failed` heading. Open it to
see the reasons and the **Copy failures** button.

Scripts written for rs2b0t run, but a call that 274bot does not support stops the
script with `not impl`. To move accounts and script settings from rs2b0t, see
[Import your accounts from rs2b0t](importing-from-rs2b0t.md).

## Start and stop many bots

The Fleet toolbar has **Start** and **Stop** buttons. They act on the marked bots.
With no marks, they act on the loaded bots that the filter shows, after you click
**Confirm**. In the terminal app, press `T` or `E` on the Script screen.

By default, each bot starts its own saved script. A bot that already runs a
script is skipped with `already active`. A bot with no saved script fails with
`no assignment`.
Bots start one after another, so some show `queued 2/5` for a while. The
[panel guide](../api/panel.md#fleet-toolbar) describes the toolbar.

## Read the status

The **status** line of the **script** section shows one of these words. While a
script runs, the **status** section of the panel shows it in its `script` row.

| Word | What it means |
| --- | --- |
| `idle` | No script runs on the bot. |
| `starting` | The script is being prepared. |
| `running` | The script runs. It has no phase to show. |
| `preparing`, `working`, `waiting` | A built-in script reports what it does. `waiting` means it waits for something. |
| `blocked` | The script cannot do its current work. A reason appears under the line. |
| `paused` | You paused the script. |
| `stopping` | The script is ending. |
| `complete` | The script finished its work. |
| `error` | The script failed. The reason follows after a colon. |
| `stopped (blocked)` | The script ended because it could not go on. The reason stays visible. |

A built-in script can add rows below the line, such as `Quest` and `Queue` for the
Quester. The terminal app shows the same words, with the reason on its own line.

## When something goes wrong

| You see | Cause | What to do |
| --- | --- | --- |
| `no assignment` | The bot has no saved script and you picked none. | Click **Browse…** and pick a script. |
| `script already active: stop it first` | A script already runs on the bot. | Click **Stop**, then start again. |
| `<name> is not loaded` | The profile is not loaded as a bot. | Load the profile and log in first. |
| `missing file: <name>` | The file of a file script is gone. | Put the file back, or load another copy. |
| `catalog card <name> is missing; no catalog root was scanned` | 274bot has not read any rs2b0t folder. | Choose your rs2b0t folder in **Browse…**. |
| `settings edit is uncommitted or its save has not settled` | A settings field still holds an edit. | Press Enter in the field, or press Esc to undo. |
| `no script to reload` | The picked script is a built-in script. | Use **Reload** only for file scripts and catalog scripts. |
| `browse to pick a script first` | You pressed `t` in the terminal app with no pick. | Press `b` and pick a script. |
| `stopped (blocked)` | The script could not go on. | Read the reason. Fix it, then click **Start**. |

For scripts that do not work yet, see the
[known issues](../known-issues.toml).
