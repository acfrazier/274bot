# First start

> **Who this is for:** you downloaded a 274bot package and want to see it run.
> At the end you have a bot logged in to the public server, ready for a script.

## What you need

- The package for your system: macOS on Apple Silicon, Windows (64-bit) or
  Linux (64-bit). Get it from the
  [Releases page](https://github.com/acfrazier/274bot/releases/latest).
- Your own account on the public game server, **rs2b2t**.
- An internet connection. The first start downloads the game files.

You do not need Rust or a game server of your own.

### System requirements

**macOS.** `panel-play` and `tui-play` need Apple Silicon running macOS 11.0 or
later.

**Windows.** `panel-play` uses Vulkan and falls back to Direct3D 12 when Vulkan
has no working GPU. It prefers the power-saving GPU, which on a laptop with two
GPUs is the integrated one that drives the screen. To choose otherwise, set
`WGPU_BACKEND=dx12` (or `vulkan`) and `WGPU_POWER_PREF=high` before you start it.
Windows also needs the Microsoft Visual C++ x64 runtime (`VCRUNTIME140.dll` and
`VCRUNTIME140_1.dll`).

**Linux.** You need glibc 2.39 or newer. `panel-play` needs an X11 or Wayland
session and Vulkan: the Vulkan loader plus a GPU driver, or Mesa lavapipe for
software rendering. `BOT_CPU=1` draws the game view on the CPU; the window still
uses Vulkan. On Ubuntu 24.04 the libraries come from `libasound2t64 libssl3t64
libx11-6 libx11-xcb1 libxcursor1 libxi6 libxkbcommon0 libxkbcommon-x11-0
libwayland-client0 libwayland-cursor0 libvulkan1 mesa-vulkan-drivers`.
`tui-play` needs only glibc and OpenSSL 3 (`libssl3t64`). The panel follows the
system's choice of graphics chip; `WGPU_POWER_PREF=low`, `high` or `none`
overrides that.

## Your first run

### macOS

1. Double-click the downloaded `.zip` file to unpack it.
2. Open `274bot.app`. The app starts on the rs2b2t server.
3. Continue with [step 3 below](#in-the-app).

The `.zip` also holds `panel-play` and `tui-play`. They need the `nav` and `map`
folders next to them. The app has its own copies.

### Windows

1. Unpack the downloaded `.zip` file (right-click it, then **Extract All**).
2. In File Explorer, open the unpacked folder, type `powershell` in the address
   bar and press Enter. Run:

   ```
   .\panel-play.exe --profile rs2b2t
   ```

   To start with a double-click later, make a shortcut to `panel-play.exe` and
   add `--profile rs2b2t` after the path in the shortcut's **Target** box.
3. Continue with [step 3 below](#in-the-app).

Keep the `nav` and `map` folders next to `panel-play.exe`.

### Linux

1. Unpack the download: `tar xzf 274bot-<version>-linux-x64.tar.gz`.
2. Open a terminal in the unpacked folder and run:

   ```
   ./panel-play --profile rs2b2t
   ```

3. Continue with [step 3 below](#in-the-app).

Keep the `nav` and `map` folders next to `panel-play`.

### In the app

These steps are the same on every system.

3. **Wait for the loading bar.** A bar and a line such as **Checking game files**
   show what the app is doing. The first start downloads the game files and takes
   longer. Later starts reuse them.
4. **Create your vault.** The vault is the encrypted file that holds your
   accounts. In the panel on the right, type a passphrase into **new vault
   passphrase** and click **Create vault**. The passphrase must have at least one
   character that is not a space. It is not your game password. If you lose it,
   the only way back in is **Reset vault**, which deletes the vault and every
   saved account.
5. **Add your account.** Click **Profiles**, then **New profile**. Type your game
   username and password, then click **Save**. The window shows **Saved** and the
   new profile is selected. The app does not make an account for you.
6. **Log in.** Click **Log in** in the panel on the right. The bot's status shows
   **waiting in login queue** and then **logging in**. When the game appears, you
   are in. To skip this click next time, tick **Log in automatically** in the
   **Profiles** window.
7. **Start a script.** Click **Browse…** and click a script. The first time, a
   window may ask for your rs2b0t folder; click **Not now** if you have none. Then
   click **Start**. **Pause** and **Stop** sit next to it. **Script prefs** opens
   the script's settings.

Next time, open the app, type your passphrase, click **Unlock vault** and click
**Log in**.

## Where 274bot keeps its files

274bot keeps its files in the `.274bot` folder inside your home folder
(`C:\Users\<you>\.274bot` on Windows).

| File or folder | What it is |
| --- | --- |
| `vault-prod` | Your vault, with your accounts |
| `unpack-289` | The game files that the first start downloaded |
| `servers.json` | The list of game servers |
| `panel-ui.json` | Your window layout and choices |

## Troubleshooting

| What you see | Why | What to do |
| --- | --- | --- |
| Windows says `VCRUNTIME140.dll` or `VCRUNTIME140_1.dll` is missing | The Microsoft Visual C++ runtime is not installed. | Install the Microsoft Visual C++ x64 Redistributable, then start the app again. |
| The panel closes right after it starts on Linux with "importing the supplied dmabufs failed" | Your computer has two graphics chips. | Start it with `WGPU_POWER_PREF=high ./panel-play --profile rs2b2t`. |
| The window freezes for minutes at start on a Windows laptop | `WGPU_POWER_PREF=high` is set, and the laptop's dedicated GPU driver is slow to load. | Remove `WGPU_POWER_PREF` or set it to `low`. |
| The app says a local profile "needs an engine directory" | You started `panel-play` without `--profile`. | Start it with `--profile rs2b2t`. |
| An error appears instead of the loading bar, or the bar never finishes | The app cannot download the game files. | Check your internet connection and firewall, then start the app again. |
| "wrong passphrase" | The passphrase differs from the one you created. | Type it exactly as you created it. If you forgot it, click **Reset vault** and confirm. This deletes every saved account. |
| **Profiles** shows "vault is empty — New profile then Save", or `tui-play` stops with "vault has no profiles" | You have not saved an account yet. | In `panel-play`, click **Profiles**, then **New profile**. `tui-play` uses the same vault. |
| The status line stays on "waiting in login queue" | Bots log in one at a time, with a pause between logins. | Wait. A status such as "waiting in login queue (1/3): next in line, login rate cooldown (29 attempts), next try in 41 s" tells you how long. |
| The bot cannot log in and the log says "world full" | The world has no room. | A profile set to **auto** world tries the next world by itself. A profile set to one world stays there; set its world to **auto** in **Profiles**. |
| WalkTo asks "Bake WalkTo map terrain?" | Your game files differ from the ones the package was built with. | Click **Bake now**. It runs once, takes about 15 seconds, and later starts reuse it. **Not now** keeps a map without terrain. |
| WalkTo shows "no nav pack — run nav-pack" | The `nav` folder is missing next to the app. | Unpack the package again and keep its folders together. |
| A bot refuses to walk, or says no bank is reachable, near dangerous monsters | **Danger routing** is on **When survivable**, which can refuse trips that the account could survive. | Open **Nav config** and set **Danger routing** to **Always**. Use this only when the account can handle the monsters. |

More problems are listed under "Known issues" in the
[CHANGELOG](https://github.com/acfrazier/274bot/blob/main/CHANGELOG.md).

## Terminal app

`tui-play` runs the same bots in a terminal, which suits a server with no
screen. Run `./tui-play --profile rs2b2t`. It asks for the vault passphrase in
the terminal. See the
[terminal app guide](https://github.com/acfrazier/274bot/blob/main/docs/api/tui.md).

## Upgrading from 0.1.9.1

Read "Upgrading from 0.1.9.1" in the release notes. The same text is in
`CHANGELOG.md` and in the
[upgrade guide](https://github.com/acfrazier/274bot/blob/main/docs/upgrade-0.2.0.md).
Server profiles, the vault passphrase prompt, local engine paths, self-baked
navigation packs and the graphics defaults changed.

## Build from source

To build 274bot yourself or use your own game server, see
[CONTRIBUTING.md](https://github.com/acfrazier/274bot/blob/main/CONTRIBUTING.md).

## Credits

274bot exists because of [rs2b0t](https://github.com/rs2b2t/rs2b0t). Its
interface is modelled on rs2b0t's (drawn with ImGui instead of Electron), it
keeps rs2b0t's script API so the rs2b0t catalog runs here, our quest Paths
started from its AIOQuester, and parts of our game data come from it. Thank you
to [everyone who has contributed to rs2b0t](https://github.com/rs2b2t/rs2b0t/graphs/contributors).
The Quester also owes a lot to Liza Carvelli's Questionable plugin for
[Dalamud](https://github.com/goatcorp/Dalamud).
It also builds on [Lost City](https://github.com/LostCityRS) and the
[Fairy Ring](https://github.com/Fairy-Ring/FR-client-rust) client. Licenses
are in NOTICE.md.
