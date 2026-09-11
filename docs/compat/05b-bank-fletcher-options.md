# BankFletcher stringing option fixtures

This bounded extension proves two production `BankFletcher` paths without
changing either frozen catalog or the runtime API. Both panel and host-play use
the shared scenario registry, so the new names are available through their
existing scenario selectors and terminal-shot handling.

## Exact item identity

Willow shortbow (u), object id 60, and the strung Willow shortbow, object id
849, have the same display name. The scenarios therefore use new exact-ID
inventory and fresh-bank predicates rather than aggregating this name. The
other fixture IDs are Willow logs 1519 and unstacked Bow string 1777.

`ItemId` and `ItemIdAtMost` count inventory stacks by object ID. `BankItemId`
and `BankItemIdAtMost` count bank rows by object ID and fail closed unless the
snapshot is in-game, at scene state 2, and has an open, loaded bank. Existing
name-based proofs and evidence remain unchanged.

## Scenarios

### `bank_fletcher_string`

This scenario applies to both frozen catalogs on revisions 274 and 289. It
injects `material=Willow logs` and `product=String short bow`; it deliberately
does not inject `mode`, because catalog
`100adccc037d9f6898080e1cad58fcfc43364775` has no such setting and derives
stringing from the product.

Before Start it seeds Fletching 35, two id-60 bows and two id-1777 strings in
inventory, plus 28 of each input in bank. It seeds no id-849 output. The proof
opens the Varrock West booth to acknowledge both exact bank stocks and the
absence of id 849, then closes the bank before Start. It arms Fletching XP
before production, observes both initial pairs becoming id
849, observes those products deposited in a fresh bank, then observes the
production script's 14-string exact withdrawal and withdraw-all bow behavior.
For unstacked inputs this yields an exact 14+14 pack and matching bank decreases
from 28 to 14 in one bank generation. After the bank closes, another id-60 and
id-1777 pair must be consumed into id 849 with further Fletching XP.

### `bank_fletcher_cut_string`

This scenario applies only to catalog
`8e7d965be2071d6ec65c3265e12af797082d720a`, on revisions 274 and 289. It
injects `mode=cut+string`, `material=Willow logs`, and `product=Short bow`.
Selecting it with catalog `100adccc037d9f6898080e1cad58fcfc43364775`
fails before profile/client startup with an explicit unsupported-catalog error.

Before Start it seeds Fletching 35, one Knife and two id-1519 logs in inventory,
plus 28 id-1777 strings in bank. It seeds no logs, id-60 bows, or id-849 bows in
bank and no bow/string outcome in inventory. It opens the seed bank to
acknowledge all of those exact stocks and absences, then closes it before Start.
The witness requires both logs to
be consumed into exact id 60 with cutting XP, those script-created bows to be
deposited in a fresh bank, and the script to switch phases after confirming the
log bin is empty. The same bank generation must then show the Knife deposited,
all two id-60 bows withdrawn, and 14 id-1777 strings withdrawn. After bank close,
both exact input IDs must be consumed into two id-849 bows with further XP.

The frozen new-catalog source confirms this fixture shape: cut+string keeps the
Knife during the cut phase, deposits it when logs are exhausted, requests 14
unstacked string slots, then withdraws all exact-id unstrung bows.

## Independent witness and rejection coverage

The headless catalog boundary observation now retains inventory and bank counts
by ID in addition to the existing display-name maps. Its BankFletcher state
machines require an ordered open/loaded bank observation and require deposit
and withdrawal to share one bank generation. Unit coverage rejects:

- a seeded-only observation and the first crafted pair without a bank cycle;
- same-display-name evidence with the required exact ID removed;
- closed or unloaded bank rows;
- a withdrawal observed in a different bank generation;
- combined-mode observations that omit cutting, deposit/withdrawal, or exact
  id-849 stringing;
- a pre-Start combined fixture containing a seeded id-60 or id-849 outcome.

## Verification

Implementation baseline: host `7852b5a053fd27e292221be78ea0783e805e3454`,
client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`.

- `cargo test -p scenario` — 82 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 12 passed, 1 ignored (`LIVE` cell).

No LIVE or fixture process was launched. Root owns the four stringing cells and
two combined-mode cells after review. These six bounded cells do not qualify
all BankFletcher settings; explicit `mode=string`, aliases, other materials and
products remain follow-up proof scope.
