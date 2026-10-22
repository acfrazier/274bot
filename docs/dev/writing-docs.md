# Writing 274bot docs

> **Audience:** anyone who writes or edits a 274bot document, including the
> README, `FIRST-START.md` and the CHANGELOG. When you finish, a page follows
> these rules and uses the words in the [glossary](../manual/glossary.md).

274bot is used by people who run bots. Most of them do not read Rust. Write
for the person who will do the task, not for the person who built the feature.

## The rules

Each rule ends with a test you can apply when you review a page.

1. **Start with who and what.** The title names the task ("Start a script on
   one bot"). The second line says who needs the page and what they have at the
   end. *Test: a reader knows in ten seconds if the page is for them.*
2. **Task first.** Use numbered steps with one action each. After a step, say
   what the reader will see ("The **Profiles** window opens."). *Test: a new
   user can finish the task with no other page open.*
3. **Say "you".** Write "Click **Start**", not "Start is clicked". Write "you",
   not "the user" or "the operator".
4. **Short sentences.** Aim for 15 to 20 words and never more than 30. One idea
   per paragraph, at most five lines. *Test: no paragraph over 80 words and no
   table cell over two sentences.*
5. **Example first.** Show the command, the screen text or the file, then
   explain the options. Put limits and edge cases after the example.
6. **Copy labels exactly.** Write buttons and windows as the app shows them, in
   bold: **Log in**, **Script prefs**. Write keys, flags, commands and file
   names in code font. Quote error messages exactly as shown. *Test: search the
   app's source for the label.*
7. **No internal names on user pages.** Do not write crate names, type names,
   function names, wire or file-format versions, test names or `file:line`
   citations. You may write flags and environment variables a user types, files
   a user edits and messages a user reads. If a user must act on an internal
   fact, say it in user words ("rebuild your navigation data") and link to a
   developer page.
8. **Keep reference out of steps.** Steps link to a table; they do not contain
   one. Lists of options, keys, limits and defaults belong on reference pages.
9. **One fact, one home.** Say it once and link to it from other pages. A removed
   option is described only in the upgrade guide and the CHANGELOG.
10. **Describe the current version.** Do not write "now", "no longer", "was
    removed" or "from 0.2.5" on manual pages. Versions belong in the upgrade
    guide and the CHANGELOG. Say what does not work yet and give the workaround.
    Do not write "fails closed", "honest" or "best-effort".
11. **Say what happens when it goes wrong.** After a task with a common failure,
    give the message and the fix, or link to a row of the troubleshooting table.
12. **Plain tables.** At most six columns, no lists inside cells, no table that
    needs sideways scrolling.
13. **Size.** A how-to page has 800 words or fewer. A page that also holds a
    requirements list or a troubleshooting table may be longer. A reference
    section has 400 words or fewer. Split a file that grows past 1,000 lines.
14. **Use the glossary words.** Do not swap in a synonym. If a word is missing
    from the glossary, add it in the same change.

## CHANGELOG entries

CHANGELOG entries follow the same rules. Write what changed for the person
who runs the app: what they see, and what they do differently. Do not name
crates, types, tests, commit ids or test counts. Say where a known problem has
a workaround. Deferred work and known defects go in `docs/known-issues.toml`,
not in the Unreleased section.

## Open choices

The app and the docs disagree in a few places. Until the choice is made, copy
what the app shows, and do not "fix" it in prose:

- **Log out or Logout.** The panel's login row says **Logout**; the Fleet
  toolbar says **Log out**.
- **Profile.** The **Profiles** window lists saved accounts, but `--profile`
  picks a server entry. The glossary calls the second one a **server**.
- **Spelling.** Keep the spelling of the page you edit. Labels in the app win.

## Before and after

Before: "Non-toggle window buttons open a closed window where it first appears
and bring an already-open one to the front, selecting its dock tab. The focus
is requested once per click, never every frame."
After: "Each of these buttons opens its window. If the window is already open,
the button brings it to the front."

Before: "Refusals distinguish `NoRouteWithinBounds`, `Unsurvivable`,
`FixableWith`, `Unknown` and `EscapeInProgress`."
After: "If 274bot will not walk a route, it says why: no route was found, the
route is too dangerous, or you can fix it (for example, you lack the coins for
a fare)."
