# World source audit before step 5

This is preparation for navigation/content qualification. Step 4 remains open.
`audit_world_sources.py` reads the two pinned content trees and emits
`evidence/world-capabilities/source-audit.json`; it changes no fixture state.

The selected hardcoded shop, trade, guardian, rune-chainbody and thieving-stun
IDs name the same definitions in both sources. This does not establish packed
cache shape or runtime behavior. The genie lamp, mime, strange-box, shop and
trade interface source files are byte-identical in the pinned revisions.

The bank inventory grows from 8 by 30 to 8 by 36 slots, with a longer scroll
range. The control interface adds a Blow Kiss emote and moves later emote text
definitions; the early run and retaliate controls are unchanged in source.
Both thieving scripts apply `stunned_thieving` (pack ID 245). The script files
otherwise differ, so the whole thieving behavior is not covered by that fact.

Step 5 should verify actual cache controls, bank capacity, guardian detection
and supported solver inputs before enabling 289 production bot operation. IDs
that are identical should retain their behavior; no speculative renumbering is
needed. The current selected-cache fingerprints remain the pairing authority.

The existing v8 navigation format already carries collision, transports and
bank stands. `host-play::NavManifest` binds a pack and optional flags to the
revision and cache hash while allowing the existing unmanifested 274 path.
The bake CLI does not yet emit that manifest or accept a revision selection.
Its 274 default config path is `engine/data/pack/config`; the 289 fixture has
the selected archive at `engine/data/pack/client/config`. A 289 bake must use
its own pinned content tree, not only change the output filename. Preserve the
v8 packed collision representation and shared route ownership.

Remaining proof: bake and inspect both worlds; reject resource mismatches;
exercise doors, region movement, bank return and guardian hold/resume/reset;
then qualify the frozen cards with the selected content. No such proof is
claimed by this source audit.
