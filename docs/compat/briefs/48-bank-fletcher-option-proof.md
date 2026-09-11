# BankFletcher stringing and combined-mode proof

Implement bounded step-7 proof in the current campaign checkout. Verify branch
`codex/rs2b0t-multirevision`; read AGENTS.md, docs/execution.md and the
fail-closed-dispatch skill. Root owns LIVE and fixture processes.

Own only `crates/scenario/src/{lib,proof}.rs`,
`crates/host-play/tests/catalog_boundary_live.rs`, and
`docs/compat/05b-bank-fletcher-options.md`. Do not edit runtime API/shim, engine,
generated data, support matrix, STATE or other workers' files. Current runtime
metadata work is concurrent. Export exact committed host/client plus your own
changes for checks when needed; never restore or stash shared working files.

Core cut-mode bank cycles now pass both revisions and both frozen catalogs at
6c6bb2d5/client 56d8027. Add strict production-path scenarios and independent
CoreWitness observations for the supported stringing modes. Inspect both frozen
BankFletcher and BankFletcherLogic files under `.superpowers/inputs/`.

Important content fact: unstrung willow shortbow id 60 and strung willow shortbow
id 849 have the SAME display name. Both selected generated data files confirm
these IDs/aliases, willow logs 1519 and bow string 1777 (unstacked). Proof must
preserve IDs and cannot pass by aggregating that name. A small ItemId/BankItemId
predicate addition is appropriate. Bank predicates must require real open,
loaded, scene-2 state. Add inventory/bank ID counts to this harness's Observation
without changing production snapshots. Preserve existing name-based evidence.

Required cases, using unchanged 180-second overall and existing observation
budgets, all seeds acknowledged before Start:

1. `bank_fletcher_string` for both catalogs and revisions. Inject material
   Willow logs/product String short bow, so old catalog's product-based behavior
   and new catalog's auto mode choose stringing. Seed Fletching 35, two unstrung
   bows plus two strings in inventory; bank 28 unstrung bows and 28 strings.
   Prove both initial pairs were consumed into id 849 with XP, deposit those
   two strung bows, observe a fresh bank generation then withdrawal of a new
   14+14 input pack with the matching bank decrease, close bank and string a
   newly withdrawn bow with further XP. Inspect script withdrawal behavior to
   confirm exact fixture expectations before encoding them. No seeded strung bow.
2. `bank_fletcher_cut_string` for new catalog 8e7d965b on both revisions only.
   The old catalog has no `mode` setting; refuse that fixture combination
   explicitly rather than silently treating it as cut. Inject cut+string,
   Willow logs, Short bow. Seed knife + two logs, no unstrung/strung bows in
   inventory or bank, bank 28 strings and no extra logs. Prove logs become id 60
   with cut XP; those exact script-created unstrung bows enter the bank; the
   script switches phases, deposits the knife, withdraws the bows plus strings,
   closes bank and consumes both input IDs into id 849 with further XP. This
   observes the cut-to-string transition after exhaustion, not only one craft.

This is harness work, not a fix to foreign script logic. Use local fixture
aliases `unstrung_willow_shortbow`, `willow_shortbow`, `bow_string`,
`willow_logs`. Selected server raw XP is 333 for cutting and 332 for stringing;
integer snapshot XP must account for tenths. Prefer observed-stage XP comparisons
for the independent witness. Do not weaken existing cut/bone witnesses or add
runtime settings gates. If source inspection contradicts a fixture expectation,
record the concrete finding and use a faithful bounded fixture.

Meaningful checks: same-name wrong ID cannot qualify; seed-only and first pair
cannot qualify a bank cycle; stale/closed bank or a generation change cannot
join a transfer; combined mode requires observed cutting and subsequent
stringing, with no seeded unstrung/strung outcome. Run focused scenario/harness
tests and strict affected harness Clippy. Avoid new tests that only enumerate
registry strings. Ensure both frontends can select the shared scenarios through
the existing registry and existing terminal-shot mechanism. Do not add a camera,
controller, runtime, feature split or scenario engine.

Commit only owned source/report, hand this SAME card to profile `reviewer` with
exact commits/checks using kanban_request_review, then stop. Do not complete it
yourself. Root will qualify six LIVE cells after actual Grok 4.5 approval. The
new explicit `mode=string` alias, material/product choices and other meaningful
branches can remain clearly named follow-up proof scope; do not claim all
BankFletcher settings from these six cells.
