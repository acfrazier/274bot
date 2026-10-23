# Import your accounts from rs2b0t

> **Who this is for:** you used rs2b0t and want your accounts in 274bot. When you
> finish, each account is a profile in the vault, with its script and script
> settings. The import writes nothing until you confirm.

You need an unlocked vault. To create or unlock one, see
[Create, unlock or reset the vault](profiles-and-accounts.md#create-unlock-or-reset-the-vault).

## Steps

1. In rs2b0t, open **Settings** and export your profiles. The file is called
   `rs2b0t-profiles.json`. It holds your passwords in plain text.
2. In 274bot, unlock the vault. Open the **script** heading of the panel and
   click **Browse…** once. If a window asks for your rs2b0t folder, choose it.
   The import needs the folder to carry over scripts and their settings. If you
   clicked **Not now** earlier, click **Import catalog…** in the **Scripts**
   window.
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
Overwrite, and `u` switches importing unusable accounts. Enter or `y` imports, and
`b` or Esc goes back. In the report, `d` offers to delete the file and `y`
confirms.

If the preview says the file is not an rs2b0t profile export, you picked the
wrong file. Choose the file rs2b0t exported.

## What the import carries over

| In rs2b0t | In 274bot |
| --- | --- |
| Username and password | A new profile. The password goes only into the vault. |
| World 1 to 3 | The profile's world, used when the server's world list has it. |
| Selected script | The profile's script, if your rs2b0t folder has a script with that name that runs in 274bot. |
| Script settings | That script's settings for the profile. Numbers are read the way rs2b0t reads them and limited to the setting's range. |
| Lamp skill, clue duel partner | The same options on the profile. |
| Renderer off or on | The profile's display off, or back on. |
| Tabs and groups | Not carried over. 274bot has none. |
| Account-wide defaults for the bank, run energy and route drawing | Not carried over. Set the bank choices in each script's settings. |
| Loadouts and price books | Not carried over. 274bot has one **Loadouts** list for all accounts. |

Route drawing and teleports are app-wide options in **Nav config**.

## Accounts the game cannot log in

An account that the game's login cannot carry is `unusable`. These are a
username longer than 12 characters, an empty password or one longer than 20
characters, and a line break or a character beyond Latin-1 in either. The import
leaves these out unless you tick **Import unusable accounts**. It never shortens a
username or password.

## Accounts that are already in the vault

**Overwrite** writes the password, the world and the settings the file carries.
It keeps every other setting of the profile. For script settings, the preview
shows `[new]`, `[unchanged]` or `[was …]` after each value.

Importing the same file twice writes nothing the second time. Bots that are
running use an imported password and world at their next login. They use
imported script settings at their next Start.
