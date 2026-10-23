# Install 274bot and log in for the first time

> **Who this is for:** you downloaded a 274bot package and want one bot in the
> game on the public server. When you finish, the bot is logged in and a script
> is running. You also know how to start it again the next day.

## Before you start

You need three things:

- The package for your system from the
  [Releases page](https://github.com/acfrazier/274bot/releases/latest).
- Your own account on the public game server, **rs2b2t**. 274bot does not
  create game accounts.
- An internet connection. The first start downloads the game files.

| System | Package |
| --- | --- |
| macOS (Apple Silicon, macOS 11 or newer) | `274bot-<version>-macos-arm64.zip` |
| Windows (64-bit) | `274bot-<version>-windows-x64.zip` |
| Linux (64-bit) | `274bot-<version>-linux-x64.tar.gz` |

Each system has a few extra requirements, such as graphics drivers. They are
listed in [FIRST-START.md](../../FIRST-START.md#system-requirements).

## Steps

1. **Unpack the package.** Keep its folders together. The app looks for the
   `nav` and `map` folders next to the program. The macOS app has its own
   copies inside it.
2. **Open the app.**
   - macOS: open `274bot.app`. It starts on rs2b2t.
   - Windows: open PowerShell in the unpacked folder and run
     `.\panel-play.exe --profile rs2b2t`.
   - Linux: open a terminal in the unpacked folder and run
     `./panel-play --profile rs2b2t`.

   A window opens. The panel, with all the buttons, is the strip on the right.
   The title at the top of the panel shows the game revision, for example
   `289bot`.
3. **Wait for the loading bar.** A line such as `Checking game files` and a bar
   such as `[#########-----------]  45%` show what the app is doing. The first
   start downloads the game files and takes longer. Later starts reuse them. If
   an error replaces the bar, see [FIRST-START.md](../../FIRST-START.md#troubleshooting).
4. **Create your vault.** The vault is the encrypted file that holds your
   accounts. Type a passphrase into the field **new vault passphrase** and click
   **Create vault**. The passphrase needs at least one character that is not a
   space. It is not your game password. If you lose it, the only way back in is
   **Reset vault**, which deletes every saved account. The panel then shows a
   **Profiles** button.
5. **Add your account.** Click **Profiles**, then **New profile**. Type your
   game username into **user** and your game password into **pass**. Click
   **Save**. The window shows `Saved <name>.` and selects the new profile.
6. **Log in.** Click **Log in** in the panel. Under the title, a banner shows
   what the bot is doing, with a timer: `Waiting in login queue (1/1) — 2.0s`,
   then `Logging in — 0.4s`, then `Loading scene — 3.1s`. When the banner
   disappears, the bot is in the game. If the banner stays on the queue for a
   while, see [the login messages](profiles-and-accounts.md#read-the-login-messages).
7. **Start a script.** In the panel, click the heading **script** to open it.
   Click **Browse…**. If no rs2b0t folder is set yet, a window asks for it at
   once and takes the focus. Click **Not now** if you do not use rs2b0t. Then
   click a script in the **Scripts** window and click **Start**. The **status**
   line under the buttons changes to `starting` and then to `running` or a phase
   such as `working`. **Pause** stops the script for a while and becomes
   **Resume**. **Stop** ends it. **Script prefs**, below **WalkTo**, opens the
   script's settings. The button is dimmed for a script that has none.

The panel does not make an account for you. An empty vault stays empty until you
save a profile.

## Next time

1. Open the app.
2. Type your passphrase into **vault passphrase** and click **Unlock vault**.
3. Click **Log in**. The panel selects the profile you used last.

To skip step 3, tick **Log in automatically** for the profile. See
[Log in automatically](profiles-and-accounts.md#log-in-and-log-out).

## Use the terminal app instead

`tui-play` runs the same bots in a terminal, which suits a server with no
screen. It logs in the first profile of the vault. You can create profiles
in the panel first, add them in the terminal app with **New profile**
(below), or [import them from rs2b0t](importing-from-rs2b0t.md).

Run `./tui-play --profile rs2b2t`. It asks for the vault passphrase in the
terminal and does not show what you type. It then logs in the first profile
in the vault. With `--user NAME` it logs in that profile instead. The flag
only selects a profile that is already in the vault. It creates none. With
an unknown name, `tui-play` stops and says so.

With a new vault, `tui-play` asks for the passphrase twice and creates the
vault. The vault holds no profile yet, so the app opens its **New profile**
form. Type the username and the account's own game password, then press
Enter to save. The new profile loads, logs in and takes focus. Press Esc
to close the form without saving. To add another profile later, press `N`
in the Fleet pane or run **New profile** from the command palette.

## Where to go next

| I want to… | Read |
| --- | --- |
| Add more accounts or change a password | [Manage your accounts and log in](profiles-and-accounts.md) |
| Move accounts over from rs2b0t | [Import your accounts from rs2b0t](importing-from-rs2b0t.md) |
| Use another server or another world | [Choose a game server and a world](servers-and-worlds.md) |
| Look up a word | [Glossary](glossary.md) |
| Find every other page | [docs/README.md](../README.md) |
