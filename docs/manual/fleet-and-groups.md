# Work with many bots

> **Who this is for:** you run more than one bot at once. When you finish,
> you can mark bots, start and stop them together, walk them as a group and
> move them to another world. For one bot at a time, see
> [Use the panel window](panel.md).

## Mark bots and filter the list

Every Fleet view shows the same bots in the same order. A tick box beside
each bot is its **mark**. Your next Fleet action takes the marked bots.

The marks row shows a count such as `5 marked, 2 not shown`. A mark stays
when the filter hides its row. The **Marks** menu holds **Mark shown**,
**Unmark shown**, **Invert shown** and **Mark by status**.
**Move marked to top** moves the marked profiles to the top of the saved
Load order.

The filter takes a name, a state or a world, such as `w3`. Every word must
match. The sort offers **Load order**, **Name** and **Status**. **Load
order** is the saved order and the default.

## Pick the fleet card

The **Card** drop-down picks the script that **Start**, **Assign**,
**Assign & restart** and **Copy settings to** use. It starts with
**Each bot's own card** (the default) and **Focused bot's card**, then every
script by category.

Picking a card changes nothing by itself. While the card is **Each bot's
own card**, **Assign**, **Assign & restart** and **Copy settings to** stay
dimmed with the tip `pick a card above`, and **Start** runs each bot's own
saved script.

## The nine buttons

Each label names its scope: `Stop 3` with marks, `Stop all 12` without.

| Button | What it does |
| --- | --- |
| **Start** | Starts the fleet card on the bots. A bot that already runs a script reads `already active`, and a bot with no saved script reads `no assignment`. |
| **Stop** | Ends the running scripts and cancels a start that still waits. |
| **Assign** | Saves the fleet card on each bot without starting it. |
| **Assign & restart** | Saves the fleet card and starts it, stopping a running script first. |
| **Log in** | Logs the bots in. A profile that is not loaded loads first. |
| **Log out** | Logs the bots out and ends their scripts. |
| **Walk** | Opens **WalkTo** in group mode. See below. |
| **Switch world** | Moves the bots to another world. See below. |
| **Copy settings to** | Copies the focused bot's settings for the fleet card to the other bots. |

With marks, an action covers every marked profile, even one the filter
hides. With no marks, it covers the loaded bots the filter shows, and asks
you to click **Confirm** first. **Cancel** writes nothing.

## The Fleet window

The **Fleet** button opens the wide view: the toolbar over a table with one
row per profile. A profile that is not loaded reads `not loaded`. A click
on a loaded bot's name focuses it. A click on its tick box only marks it.

**Load order** is the saved order. Drag a row's name onto another row to
change it. Profiles that are not loaded also sit in the rail, in a folded
**Not loaded** group. Tick them there and press **Log in** to load and log
them in. **Load all** in the **Profiles** window loads every profile, and
only shows while **MultiBox** is on.

## See many bots at once

Click **MultiBox** to raise the rail: a side strip with one tile per shown
bot. **Grid** sits behind it and divides the game view into equal cells,
one per shown bot. It hides the rail and needs **MultiBox** on first.

**only render focused** starts ticked on every launch, so only the focused
bot paints. Unticking it asks for confirm first.

## Walk as a group

1. Mark the bots, or show them with the filter. Click **Walk**. **WalkTo**
   opens in group mode.
2. Pick a tile and click **Send**. Pick **Group**. The confirm names the
   count.
3. Each walking bot gets its own command. A bot that is logged out, has no
   position yet or runs a script is skipped with that reason.

## Move bots to another world

On a server with public worlds, **Switch world** opens the world picker. A
confirmed switch saves the world on each profile, then logs each bot out
and back in through the login queue. A running script carries on after its
bot reconnects. On a server with one world, the button stays dimmed with
the reason `this server has one world`.

## In the terminal app

The terminal app has a Fleet pane with the same marks, filter, sort and
actions. Its keys are `c` for the card, `s` for the sort, `a` to mark shown
rows, `A` to unmark them, `m` to log in the marked bots, `U` to log them
out and `W` to pick a world. On the Overview screen, `i` logs in the
picked bot and `u` logs it out.
