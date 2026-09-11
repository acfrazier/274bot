# Wildy log destination player-plane correction

## Verdict

The Wildy log scenery and the observed player intentionally occupy different
planes. In both selected revisions, map square `m46_61` stores loc 2297 on raw
plane 1 at local `(57,41)`, which is world `(3001,3945)`. That tile has map flag
`f3`, including the client's `LinkBelow` bit. The client therefore exposes the
bridge scenery in the effective level-0 scene while the server keeps the player
on plane 0. The correct post-log player witness is `(2994,3945,0)`, not
`(2994,3945,1)`.

Only the Wildy log destination in the scenario and independent catalog oracle
was changed. The ridge, pipe, ropeswing, stepping-stone, rocks, lap bonus, and
next-pipe continuation witnesses remain ordered and unchanged. Agility 52,
five Lobsters, the 150-tick watches, and the 180-second global deadline also
remain unchanged.

## Source and observation evidence

- Both selected maps are byte-identical at SHA-256
  `09c30dc7d888dfaa918634d3c656e52742ce8f573a0bbfacbe236651ee3bcf21`.
  `m46_61.jm2:5503` places `2297` as `1 57 41`, while
  `m46_61.jm2:4162` marks that plane-1 tile `f3`. With square origin
  `(46*64,61*64)`, local `(57,41)` is `(3001,3945)`.
- The 274 script at
  `/Users/acfrazier/experiments/Server/content/scripts/skill_agility/scripts/wilderness_course.rs2:128-148`
  and the 289 script at
  `/Users/acfrazier/experiments/lostcity-289/content/scripts/skill_agility/scripts/wilderness_course.rs2:129-149`
  both teleport to loc x+1 and force-move x-4, x-3, x-1. The net destination
  is x-7, and neither script changes plane. Their differing SHA-256 hashes are
  `fddf3605c923f61585edf45c2edc0377b13be9e668ce745d648c59a008062572`
  and `db69683bd6b23f73241a4274334e40b3e9ebae954e853869c45b0d6cab5c3fe9`;
  the relevant log sequence is identical.
- Client `MapFlag::LINK_BELOW` is `0x2`
  (`vendor/fr-client-rust/crates/client/src/dash3d/map_flag.rs:4-8`). Static loc
  placement routes a raw plane-1 bridge tile to level-0 collision
  (`vendor/fr-client-rust/crates/client/src/core/build.rs:350-389`), and the
  completed scene pushes `LinkBelow` tiles down
  (`vendor/fr-client-rust/crates/client/src/core/build.rs:1448-1455`).
- Revision 274 updates `minusedlevel` only from the local-player `PLAYER_INFO`
  plane (`vendor/fr-client-rust/crates/client/src/client/client.rs:8489-8543`).
  Revision 289 applies its decoded player plane to the same field
  (`vendor/fr-client-rust/crates/client/src/client/actor_289.rs:434-443`). The
  host snapshot publishes the local player tile with that scene/player plane
  (`crates/api/src/snapshot.rs:1337-1355`), not the raw loc placement plane.
- The retained `wildy15e` runs corroborate this. Three cells clear the log,
  rocks, and lap 1; two continue through lap 2. Their interactions report loc
  2297 on effective level 0, while the runner waits forever for
  `arrived_near(2994,3945,1,3)`. Final evidence remains on player plane 0. The
  fourth cell, r274/newer catalog, fails the ridge witness after the original
  fall/death path and remains a failed cell.

## Correction and regression coverage

`crates/scenario/src/lib.rs` now uses level 0 for `WILDY_LOG_DEST`, so the
scenario's post-log `ArrivedNear` arm observes the actual player plane.
`crates/host-play/tests/catalog_boundary_live.rs` makes the same correction in
the independent Wildy cycle oracle. Its regression executes the complete
ordered ridge-to-next-pipe chain with the observed plane-0 log destination and
also proves that substituting the raw scenery plane 1 does not qualify.

The regression was written first and failed against the old level-1 constants
in both crates. After the two constants changed, the focused tests and full
affected suites passed. No LIVE run was launched; root retains ownership of the
fresh four-cell qualification.

## Retained failures and limits

The original four `wildy15e` JSON/log pairs under
`docs/compat/evidence/catalog-harness/live/` are unchanged. This correction does
not reinterpret them as passing cells, retry the ridge death, weaken the
full-lap requirement, replace world witnesses with script messages, or alter
the script/global timing bounds. Evidence and command receipts are in
`docs/compat/evidence/wildy-log-plane/`.
