# Paired NatureCrafter and Duel Arena observations

2026-09-11. Exact7c2eef6e/client9d090ed source verified. Actual Grok4.5 review
1296 approved the fixture artifact; root19 focused tests and strict Clippy pass.
Root built and copied the exact paired test binary using its completed28
functional compiler cache, with a new exact7c source export. This was isolated
from worker WIP; the7c cache was reused, not independently empty-started.

Four old-catalog headless cells FAIL: Air and Duel on both274/289. Newer
families are gated. No paired partial or full core is accepted.

Air fails preparation180s: MasterReady/started while RunnerWaitAck/unstarted.
Root inspected the exact fixture: runner teleports to Air ruins before
open_nearest_booth for givebank200 acknowledgement. That location has no
nearby booth; the stock is never observed. Master starts alone before the
otheractor is ready. This is a fixture error, not proof about the two-script
core. Task103 owns real bank acknowledgement before final ruins setup and a
shared barrier requiring both current prepared actors before eitherStart.

Duel prepares two actual actors with equipped bronze scimitars at the arena
and starts both scripts, then fails on BotHost.addTickListener. Root104 designs
the required callback mapping to existing native observed ticks, including
parkedwaits/pause/stop/generation behavior. No imported runtime/packetlistener
or synthetic tick opcode is authorized. No duel/combat is accepted.

Reports05r and review1296 remain dated source/build evidence. Their partial
first-transfer/craft or first-combat outcomes would not satisfy full-cycle
acceptance; neither partial outcome occurred here. Raw witness/state events
are retained in evidence/paired-catalog-fixtures/live. Root harvester
harvest_7c2eef6e.py verifies exact source/binary/log hashes and the actual Start/
baseline counts, appends fourFAILs to history and updates the four matching
matrix rows. Summary: evidence/paired-catalog-fixtures/summary-7c2eef6e.json.
