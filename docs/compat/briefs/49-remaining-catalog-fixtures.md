# Design the remaining controlled catalog fixtures

This is bounded step-7 preparation alongside runtime implementation, not LIVE,
source implementation or support acceptance. Read AGENTS.md, docs/execution.md,
fail-closed-dispatch, the active plan's steps 6-8, and support-matrix.json.
Verify `codex/rs2b0t-multirevision`. Use current docs/compat/STATE.md only to
identify accepted dependencies. Existing 04-combat-production-design.md and
04-provisioning-recovery-design.md own capability architecture; do not redesign
them. This task fills the concrete per-card fixture/proof gap those reports leave.

Inputs are the two frozen `.superpowers/inputs/rs2b0t-{100adccc037d9f6898080e1cad58fcfc43364775,8e7d965be2071d6ec65c3265e12af797082d720a}`
roots, selected generated game-data JSON, selected nav world manifests, and
the read-only local `/Users/acfrazier/experiments/{Server,lostcity-289}` server
content/engine sources. Do not operate engines or accounts or modify any inputs.
Do not inspect unrelated worktrees/archives. The scenario machinery, baseline
capture and CoreWitness already exist; do not propose another runner/controller.

Produce compact tables suitable for the implementer who must add the next
shared scenario and strict independent witness. For every named card:

- Exact selected-source core behavior and meaningful settings branches. Record
  whether both catalog versions differ; do not invent a mode on the older one.
- Concrete starting tile/level, target NPC/loc identity, skills/quest/gear/items,
  quantities and bank stock needed. Cite the source line/path for obscure data;
  mark anything unresolved rather than guessing from current OSRS knowledge.
- Baseline after all seed acknowledgements, then observable script-caused
  inventory/XP/world changes. Where banking/return is supported, include that
  cycle; seeds or a queued send cannot meet proof. Preserve existing timeouts.
- A feasible case split for the meaningful options, including trade partner
  needs and selected-revision unavailable content. Exercise distinct behavior;
  do not demand a Cartesian product of unrelated settings.
- Which accepted/queued Rust capability gates execution and what first failure
  would distinguish an invalid fixture from a missing product capability.

Names alone cannot distinguish same-name IDs (for example unstrung vs strung
bows and dragonhide variants). Use IDs/aliases from selected generated content.
Do not turn debug/reward inv_add calls into normal acquisition facts. Keep user
gameplay policy in scripts and host operations; no foreign runtime clone.

Section A owns only `docs/compat/fixtures/remaining-production.md` and covers:
AIO Teleport, RuneCrafter, NatureCrafter, CookBot, BankSorter, DartFletcher,
HerbloreSecondaries, HerbCleaner, PotionMaker, SmelterBot, Superheater,
SmithingBot, FlaxPicker, FlaxSpinner, FlaxAIO, GemCutter, MuleCrafter,
ShopBuyout, DoorOpener, TannerBot, VialFiller, LeatherCrafter, Firemaker,
FlaxRunner.

Section B owns only `docs/compat/fixtures/remaining-combat-world.md` and covers:
Duel Arena Combat Trainer, ChaosDruidKiller, RockCrab, MossGiant, GreenDragon,
FireGiant, ArdyFighter, AutoFighter, ArdyThiever, ArdyCakes, GnomeMagicChopper,
CoalTrucks, GnomeCourse, WildyAgility, BrimhavenAgility, HillGiant.

The five first cards already have core fixtures. Their open options are handled
separately; do not re-audit them. Preserve all 45 enabled cards and all 180 rows.
An existing incomplete/deferred native rewrite stays explicit and cannot be
claimed supported by a fixture that skips its required behavior.

Each architect inspects their named section only, writes its one report,
records actual model/provider and source refs, commits that report and completes
the architecture card. No source, fixtures, STATE, support ledger, script stores,
LIVE, tests, merges, remotes, backups or release actions. No sub-delegation is
needed. Root reconciles the reports into implementation order and runs proofs.
