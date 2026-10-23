# Manage your accounts and log in

> **Who this is for:** you run one or more game accounts in 274bot. When you
> finish, your accounts are saved in the vault, you can log them in and out, and
> you can read every message the app shows while a bot waits to log in. You can
> also bring over your accounts from rs2b0t.

## How 274bot stores your accounts

Each account you save is a **profile**: a username, a game password, a world and
some settings. Profiles live in the **vault**, an encrypted file. Each server has
its own vault file, so an account saved for rs2b2t is not in the vault for a
local engine. The file names are in
[Servers and worlds](servers-and-worlds.md#the-built-in-servers).

The **passphrase** opens the vault. It is not your **game password**, which the
vault keeps for you.

## Create, unlock or reset the vault

Panel:

1. Look at the **profile** area of the panel. A new vault shows the field
   **new vault passphrase** and the button **Create vault**. An existing vault
   shows **vault passphrase**, **Unlock vault** and **Reset vault**.
2. Type the passphrase and click the button. The passphrase counts exactly as you
   type it, spaces included. A new passphrase needs at least one character that
   is not a space.
3. The panel then shows the **Profiles** button. The **Profiles** window shows
   the same prompt while the vault is locked.

If you type the wrong passphrase, a red message says `wrong passphrase`. Nothing
is deleted.

If you forget the passphrase, click **Reset vault**. The window **Reset vault?**
says it deletes the vault file and every saved profile. Tick **I understand**,
click **Reset**, then create a new vault.

Terminal app: `tui-play` asks `Vault passphrase:` for an existing vault. For a
new vault it asks `New vault passphrase:` and then `Repeat the passphrase:`, and
gives you three tries. What you type is not shown. Esc or Ctrl-C cancels. A wrong
passphrase ends the run with `vault <path>: wrong passphrase`.

To start `tui-play` from a launcher that has no terminal, send the passphrase on
the first line of standard input and add `--vault-pass-stdin`:

```
printf '%s\n' "$PASS" | ./tui-play --profile rs2b2t --vault-pass-stdin
```

Without a terminal and without that flag, `tui-play` stops and prints
`no terminal to ask for the vault passphrase on; pipe it in and pass --vault-pass-stdin`.

## Add, change or remove a profile

You do this in the panel. The terminal app has no profile editor.

1. Click **Profiles**. The window lists your profiles under **Vault profiles**.
2. To add a profile, click **New profile**. To change one, click **Edit** next to
   its name. The form opens in the same window.
3. Fill in **user** and **pass**. Set the options you want (table below).
4. Click **Save**. The window shows `Saved <name>.` and selects the profile. If the
   username is empty, the form shows `credentials: username required` and
   `nothing was saved.`
5. To rename a profile, change **user** in its form and click **Save**.
6. To remove a profile, click the red **✕** next to its name. The window
   **Delete profile?** asks you to tick **I understand** and click **Delete**. A bot
   that is already running stays running.

If you leave a form with unsaved changes, the window **Unsaved profile edits?**
offers **Discard** and **Keep editing**.

| Option | What it does | Default |
| --- | --- | --- |
| **Log in automatically** | Logs the bot in as soon as it starts, and again after a disconnect. | off |
| **world (next slot start)** | The world this profile uses. Shown only for a server with a world list, such as rs2b2t. | auto |
| **random events** | On: 274bot handles random events. Off: it shows their status and does not handle or hold them. | on |
| **Policies** | How the bot answers each hostile random event. Only Flee works. Saved Fight or Ignore choices act as Flee. | Flee |
| **lamp auto** | Claims a genie lamp reward without asking you. | on |
| lamp skill drop-down | The skill a genie lamp trains. | strength |
| **clue duel partner** | The other account's name, for the Duel Arena crossing in clue 3554. | empty |

On a saved profile, **Log in automatically**, **random events**, **Policies**,
**lamp auto** and the lamp skill take effect as soon as you change them.
**user**, **pass**, **world** and **clue duel partner** change when you click
**Save**. A saved password is used at the bot's next login attempt.

## Log in and log out

- **Log in** logs in the selected bot. It is dimmed until the vault is unlocked
  and a profile is selected.
- **Logout** sits next to it. It works while the bot is logged in, waiting in the
  login queue, or stopped by the repeat guard (below). After **Logout**, the bot
  stays logged out until you click **Log in**.
- For many bots, use the **Log in** and **Log out** buttons of the **Fleet**
  toolbar. They act on the marked bots.
- In the terminal app, press `i` (Log in) or `u` (Log out) on the Overview
  screen. In the Fleet pane, `m` loads and logs in the marked bots and `U` logs
  them out.

With **Log in automatically** off, a bot waits until you click **Log in**. A
bot that runs a script still logs in by itself.

Bots log in one at a time, at least 1.2 seconds apart. They go in the order they
asked, except that the bot you have selected goes first. 274bot also allows at
most 29 logins from your address in any 60 seconds. That keeps it under the
public server's limit.

If the game drops a bot that has **Log in automatically** on, or that runs a
script, 274bot logs it in again. After three unexpected drops within ten minutes,
the bot stops trying. Its state reads `logged out (repeat guard: 3 unexpected
logouts in 600s; Log in to retry)`. Click **Log in** to clear it.

## Read the login messages

The banner under the panel title shows the bot's state with a capital first
letter and a timer. The **state** line in the **status** section and the terminal
app show the same text in lower case.

| What you see | What it means | What to do |
| --- | --- | --- |
| `waiting in login queue (2/3)` | The bot is second of three waiting to log in. | Wait. |
| `…: next in line, another bot is logging in` | The bot is first, and one other login is still under way. | Wait. |
| `…: next in line, previous login is finishing` | The last login timed out and is closing. | Wait. |
| `…: next in line, pacing logins` | The bot waits out the pause between logins. | Wait. |
| `…: next in line, login rate cooldown (29 attempts), next try in 41 s` | 274bot counted 29 login attempts from your address in the last minute. | Wait. The text counts down. |
| `…: next in line, device login limit` | The same profile logged in several times in a few seconds. | Wait. |
| `…: next in line, server: too many login attempts` | The server refused a login for going too fast. | Wait. The wait grows after each refusal. |
| `logging in…`, `loading scene…` | The login is under way. | Wait. |
| `login code 3: Invalid username or password.` | The game refused the **user** or **pass**. | Fix them in **Profiles** and click **Save**. The next attempt uses the saved password. |
| `login code 5: Try again in 60 secs...` | The game says the account is already logged in. | Close the other session. The bot tries again after 60 seconds. |
| `login code 7: Please use a different world.` | The world is full. | See the next row. |
| `login code 16: Please wait 1 minute and try again.` | The game refused too many attempts. | Wait. The bot waits longer after each refusal. |
| `login Error connecting to server. (connect: …)` | The bot could not reach the server. | Check your internet connection, then wait. The bot waits longer after each failure, up to a minute. |
| `logged out (Log in to connect)` | The bot is logged out. | Click **Log in**. |

### A full world

A full world also writes a line to the log. For example:
`world full (code 7) on w2 w2.rs2b2t.com:443 (auto world); next attempt w3 w3.rs2b2t.com:443 now, through the login queue`.
A profile set to **auto** moves to the next world at once. It waits only after
every world was full in one round: `(auto world; every world was full this
round)`. A profile with a pinned world stays there and waits: `(pinned world)`.
To change the world of a profile, see
[Choose a world](servers-and-worlds.md#choose-a-world).

## Import accounts from rs2b0t

Use this to copy your accounts, and each account's script and script settings,
from rs2b0t into the vault. The import writes nothing until you confirm.

1. In rs2b0t, open **Settings** and export your profiles. The file is called
   `rs2b0t-profiles.json`. It holds your passwords in plain text.
2. In 274bot, unlock the vault. Click **Browse…** once. If a window asks for your
   rs2b0t folder, choose it. The import needs it to carry over scripts and their
   settings. If you clicked **Not now** earlier, click **Import catalog…** in the
   **Scripts** window.
3. Click **Profiles**, then **Import from rs2b0t…**.
4. Type the path of the file and click **Preview**. The preview lists every
   account, what it carries over, and what it leaves out, each with a reason.
5. For accounts that are already in the vault, choose **Skip (keep them)** or
   **Overwrite**.
6. Click **Import**. A report lists every account. It says `not confirmed` for a
   write that was not saved.
7. When every account you chose is saved, **Delete export file…** appears. It asks
   you to tick **I understand** before it deletes the file. Click **Done**.

In the terminal app, press Ctrl-P or `:` and choose **Import rs2b0t profiles…**.
Type the path and press Enter. In the preview, `o` switches between Skip and
Overwrite, `u` switches importing unusable accounts, Enter or `y` imports, and
`b` or Esc goes back. In the report, `d` offers to delete the file and `y`
confirms.

### What the import carries over

| In rs2b0t | In 274bot |
| --- | --- |
| Username and password | A new profile. The password goes only into the vault. |
| World 1 to 3 | The profile's world, used when the server's world list has it. |
| Selected script | The profile's script, if your rs2b0t folder has a script with that name that runs in 274bot. |
| Script settings | That script's settings for the profile. Numbers are read the way rs2b0t reads them and limited to the setting's range. |
| Lamp skill, clue duel partner | The same options on the profile. |
| Renderer off or on | The profile's display off, or back on. |
| Tabs and groups | Not carried over. 274bot has none. |
| Account-wide defaults for the bank, run energy and route drawing | Not carried over. Set the bank choices in each script's settings. Route drawing and teleports are app-wide options in **Nav config**. |
| Loadouts and price books | Not carried over. 274bot has one **Loadouts** list for all accounts. |

An account that the game's login cannot carry is `unusable`: a username longer
than 12 characters, an empty password or one longer than 20 characters, or a line
break or a character beyond Latin-1 in either. The import leaves these out unless
you tick **Import unusable accounts**. It never shortens a username or password.

**Overwrite** writes the password, the world and the settings the file carries.
It keeps every other setting of the profile. For script settings, the preview
shows `[new]`, `[unchanged]` or `[was …]` after each value. Importing the same
file twice writes nothing the second time. Bots that are running use an imported
password and world at their next login, and imported script settings at their
next Start.

If the preview says the file is not an rs2b0t profile export, you picked the
wrong file. Choose the file rs2b0t exported.
