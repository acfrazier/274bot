# Manage your accounts and log in

> **Who this is for:** you run one or more game accounts in 274bot. When you
> finish, your accounts are saved in the vault and you can log them in and out.
> You can also read the messages the app shows while a bot waits to log in. To
> bring over your accounts from rs2b0t, see the end of this page.

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

A new vault holds no profile. The terminal app then opens its **New profile**
form so that you can add the first one. See
[Use the terminal app instead](install-and-first-run.md#use-the-terminal-app-instead).

## Add, change or remove a profile

You change or remove a profile in the panel. To add one, you can also use
the terminal app: press `N` in the Fleet pane (or run **New profile** from
the command palette), type the username and the account's own password,
then press Enter to save. Its other options take their defaults. Change
them later in the panel.

In the panel:

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
| **Policies** | How the bot answers each hostile random event. Only Flee works, and a saved Fight or Ignore acts as Flee. | Flee |
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
- **Log out** sits next to it. It works while the bot is logged in, waiting in
  the login queue, or stopped by the repeat guard (below). After **Log out**,
  the bot stays logged out until you click **Log in**.
- For many bots, use the **Log in** and **Log out** buttons of the **Fleet**
  toolbar. See [Work with many bots](fleet-and-groups.md). They act on the
  marked bots. With no marks, they act on every loaded bot that the filter
  shows. The toolbar then lists those bots by name and waits for you to click
  **Confirm**.
- In the terminal app, press `i` (Log in) or `u` (Log out) on the Overview
  screen. In the Fleet pane, `m` loads and logs in the marked bots and `U` logs
  them out.

With **Log in automatically** off, a bot waits until you click **Log in**. A
bot that runs a script still logs in by itself.

Bots log in one at a time, at least 1.2 seconds apart. They go in the order they
asked, except that the bot you have selected goes first. 274bot also allows at
most 29 login attempts from your address in a row. The count starts again after
60 seconds with no attempt. That keeps it under the public server's limit.

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
| `…: next in line, login rate cooldown (29 attempts), next try in 41 s` | 274bot counted 29 login attempts from your address with no quiet minute between them. A full minute with no attempt starts the count again. | Wait. The text counts down. |
| `…: next in line, login rate cooldown (29 attempts) after the current login` | The same limit, while another login is still under way. The wait has no end time yet. | Wait. The countdown appears when that login ends. |
| `…: next in line, device login limit` | The same profile logged in several times in a few seconds. | Wait. |
| `…: next in line, server: too many login attempts` | The server refused a login for going too fast. | Wait. The wait grows after each refusal. |
| `logging in…`, `loading scene…` | The login is under way. | Wait. |
| `logged out (Log in to connect)` | The bot is logged out. | Click **Log in**. |

## Read the login codes about your account

When the game refuses a login, the bot's state line shows `login code N:` and the
second line of the game's message. The game screen of that bot shows both lines
while it waits. After a refusal the bot tries again by itself, after 5 seconds
unless the row says otherwise.

| What you see | What it means | What to do |
| --- | --- | --- |
| `login code 3: Invalid username or password.` | The game refused the **user** or **pass**. | Fix them in **Profiles** and click **Save**. The next attempt uses the saved password. |
| `login code 4: Please check your message-centre for details.` | The game disabled the account. The first line, shown on the game screen but not in the state line, is `Your account has been disabled.` | Read the account's message centre for the reason. 274bot cannot lift a disabled account. |
| `login code 5: Try again in 60 secs...` | The game says the account is already logged in. | Close the other session. The bot tries again after 60 seconds. |
| `login code 9: Too many connections from your address.` | The game refused the login for too many connections from your address. | Close other game connections from your address. |
| `login code 12: Please subscribe, or use a different world.` | The world needs a members account, and the account is not one. | Pick a free world in the profile's **world** option. See [Choose a world](servers-and-worlds.md#choose-a-world). |
| `login code 16: Please wait 1 minute and try again.` | The game refused too many attempts. | Wait. The bot waits longer after each refusal. |
| `login code 17: To play on this world move to a free area first` | The account stands in a members-only area, and the world is a free one. | Log in on a members world and walk the character out of that area. |
| `Your profile will be transferred in: 12 seconds` | The game says you only just left another world. | Wait. The bot tries again when the countdown ends. |

## Read the login codes about the server

| What you see | What it means | What to do |
| --- | --- | --- |
| `login code 6: Wrong RSA key; check this world's login key.` | The key the server uses for logins is not the key 274bot has. | A public server: 274bot fetches the key again once. Your own server: check its `login_key` in [Edit the server list](servers-and-worlds.md#edit-the-server-list). |
| `login code 7: Please use a different world.` | The world is full. | See [A full world](#a-full-world). |
| `login code 8: Login server offline.` | The game's login service is not answering. | Wait. |
| `login code 10: Bad session id.` | The server refused the login handshake. | Wait. |
| `login code 11: Please try again.` | The server asks for another attempt. | Wait. |
| `login code 13: Please try using a different world.` | The game could not complete the login. | On a server with a world list, pick another world in the profile's **world** option. |
| `login code 14: Please wait 1 minute and try again.` | The server is being updated. | Wait. The bot tries again after 5 seconds, sooner than the message asks. |
| `login code 20: Please try using a different world.` | The game did not accept the login server. | On a server with a world list, pick another world in the profile's **world** option. |
| `login code N: Please try using a different world.` | The game sent a code 274bot has no text for. The app shows its number. | Wait. If it repeats, pick another world. |
| `login Error connecting to server. (connect: …)` | The bot could not reach the server. | Check your internet connection, then wait. The bot waits longer after each failure, up to a minute. |
| `login Please try using a different world. (…: connection closed before the server answered)` | The server closed the connection without an answer. | Wait. The bot waits longer after each failure, up to a minute. |

## A full world

A full world also writes a line to the log. For example:
`world full (code 7) on w2 w2.rs2b2t.com:443 (auto world); next attempt w3 w3.rs2b2t.com:443 now, through the login queue`.
A profile set to **auto** moves to the next world at once. It waits only after
every world was full in one round: `(auto world; every world was full this
round)`. A profile with a pinned world stays there and waits: `(pinned world)`.
To change the world of a profile, see
[Choose a world](servers-and-worlds.md#choose-a-world).

## Import accounts from rs2b0t

To copy your accounts from rs2b0t into the vault, with each account's script and
script settings, see [Import your accounts from rs2b0t](importing-from-rs2b0t.md).
