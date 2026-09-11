# Generated custom-item Alcher fixtures

This bounded extension keeps the frozen `Alcher` card and production script
source unchanged. Two new scenario names map to that same card and apply to
both frozen catalogs on revisions 274 and 289:

- `alcher_custom_alias`: selects `custom`, names `adamant_scimitar`, uses
  `alchs=1`.
- `alcher_custom_name`: selects `custom`, names `Adamant scimitar`, uses
  `alchs=1`.

Both cases seed one unnoted adamant scimitar (id 1331), one Nature rune (id
561) and one staff of fire (id 1387) in bank on a fresh account. Inventory
starts with no coins, no unnoted/noted target and no Nature rune. The seed
bank is opened to acknowledge those exact IDs, then closed before Start.

Generated rows in both revisions: adamant_scimitar is id 1331, display
Adamant scimitar, cost 2560, certificate_link 1332; cert_adamant_scimitar is
id 1332, template 799, link 1331, stackable. High alchemy pays 1536 coins and
65 Magic XP. The noted form shares the unnoted display name, so proofs use
exact IDs.

The independent witness requires a later bank generation than the Start
baseline, the certificate id 1332 plus a Nature rune in pack, then closed-bank
consumption of both into +1536 coins and at least 65 Magic XP. It rejects
seed-only state, a first-item acquisition, display-name-only evidence,
unnoted id 1331, an unrelated chainbody cast, the wrong coin delta, and a
queued send with no new bank session.

Historical `alcher_custom` (Rune chainbody) and the other Alcher option cases
are unchanged. Panel and host-play keep using `scenario::get` / `names()`, so
native catalog_watch can select both new names.

## Verification

Implementation baseline: host `1e17c07a98a41f34f7c167005f1ba6a99e46aba4`,
client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`.

- `cargo test -p scenario` — 83 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 13 passed, 1 ignored (`LIVE` cell).
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.

No LIVE or fixture process was launched. Root owns the eight alias/name ×
catalog × revision cells after review.
