# Bounded provisioning and recovery capability design

Use configured `grok46` defaults. This is design/source audit only for plan
step 6, while root runs step-4/5 live checks and Sol implements the first bank
family on t_e52e0a03. Read applicable host instructions, fail-closed-dispatch,
plan steps 6–7, support-matrix.json, 04-banking-design.md and brief 18. Frozen
catalog roots are the full paths in support-matrix.json; do not modify them.

Audit required enabled-card behavior for common-loot matching, loadout
gear/supplies/weapon selection and provisioning, PeriodicBank, and DeathRecovery.
Read the actual callers, supported settings and meaningful branch combinations
in both frozen catalogs. Identify existing Rust operations and the smallest
missing Rust services, distinguish harmless disabled construction from enabled
options that require action, and propose coherent implementer task boundaries.
Ordinary banking/restock/food/combat loadouts and their return paths are already
authorized; excluded quest/clue/gatherer/market-maker rewrites remain deferred.
Do not shrink the enabled set or silently mark required options unsupported.
Do not clone foreign JS banking/loadout/recovery planners or introduce a
universal job framework. Reuse Rust navigation and the pending banking service.
Scripts load/start regardless of revision; unavailable operations refuse at
host/client boundaries. Source-specific content exclusions need direct evidence.

Inspect the committed source plus clearly labeled current bank WIP read-only.
Root will integrate its final reviewed commit separately. Specify exact API
shape, Rust ownership, lifecycle/settlement semantics, evidence predicates and
bounded live fixture settings for each proposed task. Existing timeouts,
Pause/Stop, reconnect generations, route ownership and guardian holds persist.
The test design must traverse script -> wire -> host -> posted observation;
queued requests, seed effects and initial state do not prove behavior.

Write only docs/compat/04-provisioning-recovery-design.md with a concise caller /
capability / proof table and recommended task order. Do not edit product files,
STATE, the ledger, other docs or external trees; no live actions, process
restarts, remotes, index or git operations, nor additional delegation. Root owns
committing your report. Record actual model/provider in the report, complete
this design card when delivered, and report remaining product decisions only
if they are genuinely outside the authorized scope.
