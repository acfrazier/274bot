# Native loadout editor round trip

Root native inspection of reviewed source a4157243 found clipped search IDs,
a quantity input pushing Remove off the row, and a default window extending
below the viewport. Host `50f2be8a216bf96691d37454c1d70644e9ed835b` makes the
bounded layout correction: initial height/position use the viewport, quantity
width is 72 px, search uses 460 px with IDs first and a full-label tooltip.
No data, migration, save, provisioning, or gameplay behavior changed.

The five existing panel loadout tests passed from the 1,598-file private export
in a new empty target. The only private source addition is the existing scoped
IsolatedEnv interactive entry, preserved in source.json. The built native binary
hash is in binary.json. No bot login was requested.

Root directly read the native UI and four captured frames. The editor fits the
1,120 x 580 content viewport; longer contents scroll within it. Weapon search
shows `#1333 Rune scimitar rune_scimitar` in full. A private preset named
`Native slot proofloadout` saved a Rune scimitar in righthand and Lobster quantity
17. Duplicate, close/reopen, and selecting the copy retained those values.
The actual private 0600 JSON file was copied before scoped cleanup and is
retained with its hash and source path. Both entries contain the same slot and
positive quantity. Copy current equipment is visibly disabled with a needs-
character explanation when no character exists.

The CUA select-all shortcuts and drag did not replace the initial name in this
run; the resulting literal name above is the one actually saved. That automation
limitation does not change the persisted-value witness. Native copying of live
equipment, invalid/save-error paths and completed bank provisioning are not
claimed by this UI-only check. Existing focused tests cover preservation and
save errors; real provisioning remains a separate live acceptance requirement.
