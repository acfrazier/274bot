# Change the settings

> **Who this is for:** you want to change how the app acts. When you finish,
> you can set the app options, the walking options and one script's
> settings. For accounts, see [Manage your accounts and log in](profiles-and-accounts.md).
> For running scripts, see [Run scripts on your bots](running-scripts.md).

## General config

The **General config** button sits under **WalkTo**. It holds the options
for the whole app.

**Server** names the server you bound when the app started. While a session
is bound, the row reads `revision fixed for this session`. Before a
session, **Revision before session** offers 274 and 289. A change needs a
restart, and a flag on the command line still wins.

**Slot** names the focused bot, or `—` when none is focused.

**capture input** sends your mouse and keys to the focused game view. While
it is on, at most one keyboard feeds the game, and the frame rate does not
rise. While it is off, the game view only watches.

**session log file** writes the session's log under `~/.274bot/logs/`. The
files rotate, and the option starts off. While it is on, the row reads
`writing <path>`.

The render boxes set the frame rate. **sidecar 50 fps** paints every shown
bot at 50 frames per second. **only render focused** paints only the
focused bot. **focused 50 fps** paints the game view at 50 frames per
second. **full rate (this run)** appears during a live run and paints every
drawing slot. It is never saved.

The **Panel** group ticks which strip sections stay visible.

## Render and memory mode

The render row offers **none**, **GPU** and **CPU**. The
**lowmem**/**highmem** button queues the memory mode for the next login
after a clean logout. Textures, tabs and sound stay unchanged until then.

## Nav config

The **Nav config** button opens the walking options.

The **Routing** group holds the walk permissions: **allow teleports**,
**allow wilderness** and **allow bank fetch**. Bank fetch applies to manual
**WalkTo** only. The settings are global except where stated.
rs2b0t-compatible scripts always allow wilderness and bank fetch.

**Danger routing** decides if a walk may pass dangerous monsters. Its levels
are **Never**, **When survivable** and **Always**. While the middle level is
held, a note under the control reads `Not available yet: acts as Never.`
Until it works, it refuses like **Never**. With **Always**, the window warns
`Global danger-zone override is enabled.`

Two notices explain the scope. Read them once, then click **Dismiss walk
permissions notice** and **Dismiss danger-routing notice**.

**Pause script on manual movement** holds scripts still while you move the
bot yourself. It starts on. See
[Pause, resume and stop](running-scripts.md#pause-resume-and-stop).

The **Display** group holds **show nav path**. The **WalkTo map** group
holds **ask before baking terrain**. Unticked, the map bakes without
asking.

## Script prefs

The **Script prefs** button opens the settings of the picked script. Each
profile keeps its own settings. A tick box or a choice saves at once. A
text or number field saves when you press Enter or leave the field.

Edits you have not saved block **Start**. The window says
`Unsaved settings prevent Start. In Script prefs, press Enter to save a field or press Escape to discard it.`
Hidden edits are named with **Discard unsaved settings**.

Some settings apply at once. Others apply at the next start. The window
then says `Restart required to apply saved settings.` A number field shows
its valid range, and a bad value shows an error before the form closes. A
script with no settings shows `(no parameters)`, and its button stays
dimmed. For the full rules, see
[Change a script's settings](running-scripts.md#change-a-scripts-settings).

## Random events and loadouts

Random-event handling is a profile option, not an app option. Set
**random events** and **Policies** in the **Profiles** window, as
[Manage your accounts and log in](profiles-and-accounts.md#add-change-or-remove-a-profile)
describes.

The **Loadouts** button opens the window of saved gear lists, which all
profiles share.

## In the terminal app

The terminal app has no config windows. Press `o` on the Overview screen to
open its settings.
