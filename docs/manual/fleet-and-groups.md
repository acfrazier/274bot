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

## What each button does
+
For the nine buttons, see [the Fleet actions list](../reference/fleet-actions.md).
+
With marks, an action covers every marked profile, even one the filter hides. With no marks, every action but **Walk** and **Switch world** covers the loaded shown bots, and asks you to click **Confirm** first. **Walk** opens **WalkTo** in group mode instead. **Switch world** opens the world picker first, and **Confirm** comes after you pick a world. **Cancel** writes nothing.

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
bot paints. Unticking it asks for confirm first, unless **Don't show again** is ticked.

## Walk as a group

1. Mark the bots, or show them with the filter. Click **Walk**. **WalkTo**
   opens in group mode. With no marks, it ticks every loaded shown bot that
   can walk.
2. Pick a tile and click **Send**. Pick **Group**. The footer button names
   the count, such as `Walk 3 bots`.
3. Click it to walk them at once. Each walking bot gets its own
   command. A bot that cannot walk is skipped with its reason: `not logged
   in`, `no position yet` or `running a script (stop the script to include it)`.

On a server with public worlds, **Switch world** opens the world picker. A
confirmed switch saves the new world on the moving profiles, then relogs
each bot that is in game on another world through the login queue. A running
script carries on after its bot reconnects. A bot that is `not loaded`,
`already on this world`, `queued for login`, `logging in` or `not logged in`
is skipped. The switch still completes for the other bots. On a server with
one world, the button stays dimmed with the reason `this server has one world`.

## In the terminal app

The terminal app shares the marks, filter and sort. Its Fleet pane holds
**Load + log in fleet** and **Log out fleet**. **Start fleet** and **Stop
fleet** sit on the Script screen, with keys `T` and `E`. The Fleet pane keys
are `c` for the card, `s` for the sort, `a` to mark shown rows and `A` to
unmark them. `m` logs in the marked bots, `U` logs them out and `W` picks a
world. On the Overview screen, `i` logs in the picked bot and `u` logs it out.
