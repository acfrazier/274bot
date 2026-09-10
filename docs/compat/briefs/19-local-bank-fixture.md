# Local 289 bank fixture support

Use configured `luna` defaults and same-card `reviewer` afterward. This is a
small, authorized local fixture addition for the 274bot compatibility campaign.
Read the applicable lostcity-289 AGENTS and SETUP-TASK once. Root/engine/content
branches must remain `codex/revision-289-engine-env`; stop if they differ.

The host campaign has prepared an exact candidate at
`/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/.superpowers/fixture-preparation/ClientCheatHandler-with-givebank.ts`
and `givebank-289-preparation.json` in that directory. Inspect the patch rather
than trusting its name. Current engine source baseline is `a275ea812f34342cbaf7faf2e3558dcd6ca187d0`;
ClientCheatHandler SHA must be `d206a141b61edb8610f645b7ff6b68e2d29017cdd00c8552e1639b1bdcf00d69`.

Add only the existing 274 local `givebank` fixture behavior to
`engine/src/network/game/client/handler/ClientCheatHandler.ts`: resolve the bank
inventory and item by actual name, clamp the requested count, and add bank
stock under an explicit `!Environment.node.production && player.staffModLevel >= 4`
condition on the added branch. The surrounding block checks only staff >= 3;
do not treat it as a non-production guard. Preserve
every existing handler branch and error behavior. Expected candidate SHA is
`30466707ab67003aa4b54be9e451982ac8b0267bb24839b6eb15971e7c08db33`;
only this one added branch is scoped, with the explicit local fixture guard.
No accounts, databases, private keys or cache blobs may be copied or committed.

Root owns the currently running exact 289 engine and will restart it after
your same-card review. Do not start/stop any process, connect clients, modify
runtime config/content/cache, edit 274/377 trees, touch remote machines,
merge/push/change remotes, or alter any unowned WIP/index. Commit the one scoped
engine source file on its current named branch. Use focused diff/syntax checks
proportional to this local support change, not a new broad suite. Save exact
before/after/commit/command receipts under the ignored lostcity runtime folder
and report their path to root. The reviewer should confirm the guard, count
semantics and narrow diff, request corrections if needed, then complete.

This prepares a fixture; it cannot establish script or banking acceptance.
