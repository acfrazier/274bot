# rs2b0t navigation corpus mapping

Status: static, read-only source mapping for `t_3cd7b8af`. This is corpus evidence and proposed fixed selectors, not Rust/live acceptance, a golden-route oracle, or a performance result. No Bun/Node/Playwright command was run.

## Provenance and scope

The source checkout was clean at inspection time (`git status --short --untracked-files=all` produced zero rows), with HEAD `100adccc037d9f6898080e1cad58fcfc43364775`, matching the operator-supplied reference. The seven navigation harnesses use `e2e/lib/navLiveHarness.ts` for pacing, tele-arrival, inventory/energy setup, and `Traversal.walkTo`; they are live reachability/assertion harnesses, not Rust route-byte fixtures.

Corpus builders and generated files are pack-derived or source-scraped. `script-route-corpus.ts:170-313` combines mainland routes, `WALK_DESTINATIONS`, bank pairs, NAV target→bank commutes, and per-script target chains, then endpoint-dedupes. Its pack probe uses teleport-enabled state with maxed skills, Rune Mysteries/transport quest state, runes and charged jewellery (`script-route-corpus.ts:389-451`). `script-travel-corpus.ts:122-334` scrapes clue, gathering, firemaking, cooking and quest-area endpoints, snapping endpoints to standable tiles (`:69-107`). `transport-heavy-routes.ts:57-186` declares hand-picked transport OD pairs; its rich probe state, exact coin/item quantities, and quest seeds are at `:188-218`, and essence round trips explicitly preserve entry session state at `:244-294`.

## Harness-to-case map

| Source (anchors) | Concrete fixed cases and intent | Assertion type / Rust mapping |
|---|---|---|
| `e2e/nav-tele-smoke.ts:11-14,49-54,101-205` | Lumbridge `(3222,3218,0)` → Varrock landing `(3213,3424,0)`; magic >=25, Law/Air/Fire runes; teleport catalog and `distanceBeforeTeleport=50`. | Requires spell-teleport success plus Chebyshev arrival <=8 and log evidence. No golden route bytes. Rust candidate: `option_bits=1`, a state family extended with carried runes/magic; current four families cannot express these prerequisites. |
| `e2e/nav-stress-live.ts:237-247,323-505,517-617` | Pure walk Lumbridge→chicken `(3232,3298,0)`; Lumbridge→Draynor `(3092,3243,0)`; stress `walk-varrock-edge` `(3185,3436,0)`→`(3093,3491,0)`; spell Varrock/Falador; jewellery Duel `(3315,3235,0)` and glory→Edgeville `(3087,3496,0)`; trapdoor Edgeville `(3096,9868,0)`. | Reachability/radius plus optional transport/teleport log and path-paint samples. No exact route oracle. Walk cases are representable by current fixed rows; spell and jewellery cases need explicit item/skill state. Trapdoor is a crossing intent, not a fixed hop sequence. |
| `e2e/nav-script-routes-live.ts:47-70,122-231,213-233,274-343,350-377` | Generated hardest routes (10 rows; the generated corpus has 69 unique route ids, including Seers→Rock Crabs `(2725,3491)`→`(2710,3720)` and Rellekka→Yanille `(2668,3660)`→`(2612,3092)`); transport-heavy rows; issue #352 Ardougne `(2668,3285,0)` ↔ Brimhaven `(2779,3218,0)`. | Arrival <=8, optional jewellery log, and special essence/ship execution. Generated `cost`, `hops`, and `hopKinds` are provenance/ranking, not live golden expectations. Ship cases require the exact coin/quest facts listed below; essence requires a session from entry, not a synthetic exit override. |
| `e2e/nav-script-travel-live.ts:49-78,86-112,144-170,175-228` | 1,594 directed legs from clues/quests/gathering/fishing/mining/woodcutting/firemaking/cooking. Fixed examples include `(3208,3217,1)`→`(3247,3245,0)` and segment stats: clues 366, quests 996, gathering-all 232, fishing 40, mining 90, woodcutting 54, firemaking 12, cooking 10. | Arrival <=8 after tele-arrival and optional tele kit; no route or hop equality. Rust fixed selectors can use representative walk legs, but the full scraped corpus is not a compact frozen route oracle. Quest segment seeds transport quest varps and coins (`:144-164`). |
| `e2e/nav-path-paint-live.ts:96-123,163-180,285-307` | Fixed walk cases: Lumbridge `(3222,3218)`→chicken `(3232,3298)`; Lumbridge→Draynor `(3093,3243)`; Varrock→Edgeville `(3094,3493)`; Falador `(2965,3378,0)`→Taverley `(2895,3435,0)`. | PathPublish sample count/max tile/client-segment assertions plus arrival. Client rendering/path-segment evidence is outside frozen Rust router guarantees; only the OD legs are reusable. |
| `e2e/nav-two-route-smoke-live.ts:48-147,276-319` | Yanille bank `(2612,3092,0)`→warrior field `(2580,9501,0)`, radius 3, knife, web/stairs/ledge; TGV center `(2542,3169)`→maze outside `(2493,3187)`, no spirit/Elkoy; Elkoy `(2504,3192)`→TGV center, Tree Gnome Village started, knife. The Elkoy row sets `treequest=9`, seeds journal `Tree Gnome Village`, and gives `knife ×1` (`obj` name `knife`; numeric id MISSING) (`:117-131`). | Arrival plus required hop/zone assertions. Stairs/web/ledge/Elkoy are transport/crossing intentions; no fixed hop sequence. Quest varp is set then relogged. |
| `e2e/clues/hardclue-nav-live.ts:40-106,131-147,239-293` | Dynamic hard-clue destinations from `CLUE_DB`, talk anchors and kill anchors; origin Varrock east bank `(3253,3420,0)`. Kit: coins 100,000, shantay passes, rope, spade, machette, gas mask, climbing boots; gas mask and boots are worn. | Dynamic source rows, default-known pack gaps skipped, radius 2 and arrival/refused-crossing/repath assertions. Not directly representable as a fixed route list without freezing selected IDs and prerequisite state. |

The shared harness defaults are deliberately slow (`navLiveHarness.ts:8-23`): walk poll 2s, sustain 5s, arrival poll 400ms, stop poll 500ms. Those pacing values are not router semantics and should not enter Rust differential expected output.

## Transport-heavy fixed candidates

These rows are the best additions for transport coverage because the generated metadata names the intended family and hop kind. They remain expected intents until a Rust dense-vs-tiled run records actual route legs:

- `TH-ship-sarim-musa`: Port Sarim `(3027,3218,0)` → Musa deck `(2956,3143,1)`, intended `ship`.
- `TH-ship-ardy-brim`: Ardougne `(2683,3272,0)` → Brimhaven deck `(2775,3234,1)`, intended `ship`.
- `TH-entrana-out`: Port Sarim monk `(3048,3236,0)` → Entrana `(2834,3331,1)`, intended `ship`.
- `TH-cart-shilo-brim`: Shilo `(2834,2954,0)` → Brimhaven `(2776,3214,0)`, intended cart/transport family.
- `TH-glider-gandius-hub`: Karamja pad `(2971,2969,0)` → Grand Tree `(2465,3501,3)`, intended gnome glider.
- `TH-combo-lumby-entrana`: Lumbridge `(3222,3218,0)` → Entrana `(2834,3331,1)`, generated hop intent `teleport + ship`.
- `TH-combo-varrock-shilo`: Varrock `(3213,3424,0)` → Shilo `(2834,2951,0)`, generated hop intent `teleport + ship + gangplank + ship`.
- `TH-ess-round-aubury`, `TH-ess-round-sedridor`, `TH-ess-round-brimstail`: wizard entry → essence mine pad → portal return; the source explicitly requires `EssenceSession` created by the entry hop and forbids a setvar override (`transport-heavy-routes.ts:115-143,244-294`). Exact packed entry/return coordinates are `ESSENCE_RETURN.aubury`, `.sedridor`, and `.brimstail`, and the mine endpoint is `ESSENCE_MINE_PAD` (`travelCatalog.ts` packed `parseLcCoord` forms); numeric tile values are MISSING in this mapping and must be frozen from those named definitions before a case is added.

The generated transport file says its rich assumptions are members, completed transport quests, full runes and coins (`transport-heavy.routes.json:2-84`). The pack-probe rich state explicitly has Coins `5000` and (among other state) Law rune `200`, Air rune `500`, Fire rune `200` (`transport-heavy-routes.ts:188-218`). Quest seeds include Rune Mysteries (`runemysteries=6`), Grand Tree (`grandtree=160`), Tree Gnome Village (`treequest=9`), and Shilo (`zombiequeen=15`, journal `Shilo Village`), plus Plague City, Watch Tower, Eadgar's Ruse, Waterfall Quest and Dragon Slayer (`:5-82`). Live tele-smoke instead seeds Law `50`, Air `150`, Fire `50` (`nav-tele-smoke.ts:49-54`); shared harness defaults are Law `80`, Air `200`, Fire `80` (`navLiveHarness.ts:540-542`). These facts are not silently transferable to the current Rust fixed facts.

## Rust representability and explicit gaps

The current fixed-row contract is ten integers `from_x from_z from_level to_x to_z to_level option_bits state_family radius model`; option bits are teleport, wilderness, bank-fetch and Aubury essence (`docs/memory/nav-tiled-differential/README.md:98-115`). State families are only: empty; skills `(6,25),(2,3)` plus Rune Mysteries and varps; carried `(995,10)` plus worn item 1712; or their union. Bank supply is exactly coins `(995,10)` and item 1712 `(1)` (`README.md:110-116`).

Representable now, without changing prerequisites:

- ordinary mainland walk legs, including F2P-01..06 and the fixed chicken/Draynor/Edgeville/Falador/Taverley rows;
- radius/NoPath and sealed/crossing cases when the selected OD is actually in the frozen pack;
- teleport-disabled walks (`option_bits=0`) and model/radius variation;
- bank-fetch as a declared option with an explicitly frozen OD, bank supply, and missing-item/worn diagnosis that invokes host `plan_bank_fetch`; an ordinary walk to a bank is not BankSession success.

Capability gaps that must remain explicit:

- spell teleports need carried runes and the relevant magic/quest prerequisites. Magic skill id 6 level 25 is already present in family 1. The live seed counts are Law/Air/Fire `50/150/50` (`nav-tele-smoke.ts:49-54`), shared defaults `80/200/80` (`navLiveHarness.ts:540-542`), and rich probe `200/500/200` (`transport-heavy-routes.ts:188-218`). Numeric edge requirements are independently anchored by the Rust synthetic fixture: `554=firerune`, `556=airrune`, `563=lawrune` (`crates/nav/src/transport.rs:5711`), with Varrock `(554,1)+(556,3)+(563,1)` and magic `(6,25)` (`:5757-5760`). The real-pack selector inventory corroborates the same requirements on edge index 2143 to `(3213,3424,0)` (`diagnostics/native-nav-differential-preparation/results-760d3ac/metadata/root-real-selector-inventory.json:55516-55555`). These are edge minimums, not a claim that the current four diagnostic families carry them; any spell quest requirement remains MISSING;
- jewellery teleport rows need carried charged jewellery. Live stress seeds carried `ring_of_dueling_8` and `amulet_of_glory_4` (`nav-stress-live.ts:462,492`); the glory OD is Al Kharid `(3293,3174,0)`→Edgeville `(3087,3496,0)`. The Rust synthetic fixture maps `1712=amulet_of_glory_4` and `2552=ring_of_dueling_8` (`crates/nav/src/transport.rs:5781-5785`), and asserts glory item requirement `(1712,1)` (`:5823-5836`). Real-pack edge index 2166 independently records `loc=1712`, item `(1712,1)`, and destination Edgeville (`diagnostics/native-nav-differential-preparation/results-760d3ac/metadata/root-real-selector-inventory.json:56258-56284`). Current facts have item 1712 worn, not carried, while bank supply has item 1712 ×1; a successful glory edge therefore needs carried item 1712, not merely worn equipment;
- ship/cart rows need explicit transport state: rich probe Coins `5000`; Shilo cart needs `zombiequeen=15` and journal `Shilo Village`; Grand Tree glider needs `grandtree=160` (`transport-heavy-routes.ts:188-218`). Fare, membership, and any additional edge-specific fields are MISSING. Stairs/glider are not generically unrepresentable: native comparison already exercised successful Stairs and Glider under current facts (with varps distinguishing glider). Web/ledge, ship/cart, and wilderness crossings remain transport-specific intents until their exact facts are frozen;
- essence exit needs the entry-created session. `option_bits` bit 3 names Aubury essence intent, but current fixed facts do not prove a successful session round trip; the named entry/mine/return tile values remain MISSING until extracted from `travelCatalog.ts`;
- hard-clue kit/equipment and per-quest varps are not available in the current fixed state schema;
- current native comparison coverage therefore has Door/Ladder/Stairs/Glider/EssenceExit/radius/NoPath but no successful Teleport or BankSession. The mainland teleport/bank selectors taking ordinary routes are limitations of chosen facts, not router defects. `BankSession` is host session orchestration through `plan_bank_fetch` after missing-item/worn diagnosis (`crates/nav/src/bank_fetch.rs`), not a graph bank-fetch edge.

Do not turn a missing requirement into a successful transport result, add runes/items to an existing family without a reviewed schema extension, or treat generated hop metadata as a golden route.

## Proposed next frozen corpus (root decision required)

Keep the existing 40 selectors unchanged and add a separately reviewed extension only if root authorizes state representation. Suggested bounded rows, with exact prerequisites frozen beside each row:

1. `walk-f2p-01`: `(3222,3218,0)`→`(3208,3220,2)`, `option_bits=0`, `state_family=3`, `radius=0`, `model=0`; source `nav-tiled-differential` fixed family/row, reachability plus stairs/level intent. Stairs are already representable.
2. `walk-lumb-dray`: `(3222,3218,0)`→`(3092,3243,0)`, `option_bits=0`, `state_family=3`, `radius=4`, `model=1`; source `nav-stress-live.ts:323-505`, arrival/reachability, no golden route.
3. `spell-varrock`: `(3222,3218,0)`→`(3213,3424,0)`, `option_bits=1`, `state_family=needs schema extension`, `radius=4`, `model=0`; source `nav-tele-smoke.ts:101-120` (`walkTo` radius 4; the later `ARRIVAL=8` assertion is not the walk radius). Freeze magic `(skill id 6, level 25)`, carried Law/Air/Fire rune IDs `(563,1),(556,3),(554,1)` from the Rust fixture and corroborating real-pack edge index 2143, plus live seed counts `50/150/50` at `nav-tele-smoke.ts:49-54`; any quest requirement remains MISSING. Assertion is teleport log plus arrival, not golden bytes.
4. `jewellery-glory-edge`: `(3293,3174,0)`→`(3087,3496,0)`, `option_bits=1`, `state_family=needs schema extension`, `radius=4`, `model=0`; source `nav-stress-live.ts:462,492` and OD in `nav-stress-live.ts` (the stress walk radius is 4). Freeze carried charged `amulet_of_glory_4`, numeric ID `1712` from the Rust fixture (`crates/nav/src/transport.rs:5781-5836`) and real-pack edge index 2166, not current worn-only 1712; assertion is jewellery log plus arrival.
5. `ship-sarim-musa`: `(3027,3218,0)`→`(2956,3143,1)`, `option_bits=0`, `state_family=needs schema extension`, `radius=0`, `model=1`; source id `TH-ship-sarim-musa` in `transport-heavy.routes.json:2-84`. Freeze Coins `5000` (rich probe `transport-heavy-routes.ts:188-218`), members/fare/ship edge fields MISSING, and crossing state; assertion is intended ship reachability/hop log, not a graph-edge or golden route.
6. `ship-ardy-brim`: `(2683,3272,0)`→`(2775,3234,1)`, `option_bits=0`, `state_family=needs schema extension`, `radius=0`, `model=1`; source id `TH-ship-ardy-brim` in `transport-heavy.routes.json:2-84`. Freeze Coins `5000`; fare, membership, and edge/quest fields MISSING. Pair with issue #352 shore OD only if disembark/gangplank state is represented; assertion is intended ship reachability/hop log.
7. `essence-aubury-roundtrip`: `ESSENCE_RETURN.aubury` entry → `ESSENCE_MINE_PAD` → `ESSENCE_RETURN.aubury` portal, `option_bits=8`, `state_family=1` only if Rune Mysteries is retained, `radius=0`, `model=1`; source `transport-heavy-routes.ts:115-143,244-294` and packed definitions in `travelCatalog.ts` (numeric coordinates MISSING). Freeze `runemysteries=6`, entry-created `EssenceSession`, and no setvar exit override; assertion is session-backed reachability/exit, not golden bytes.
8. `bank-fetch-nearest`: freeze one source-provenanced OD and its exact radius/model, `option_bits=4`, `state_family=needs schema extension`; source contract `docs/memory/nav-tiled-differential/README.md:98-116` plus host orchestration `crates/nav/src/bank_fetch.rs`. Freeze bank supply and missing carried/worn item (current diagnostic supply is Coins `995×10`, item `1712×1`), `allow_bank_fetch`, and the exact diagnosis that forces `plan_bank_fetch`; assertion is `BankSession` plan/result. This is host session orchestration, not a graph bank-fetch edge: an ordinary walk to a bank is not BankSession proof.

For every added row, freeze the dense baseline and candidate, compare complete ordered route legs/hops/costs and error/NoPath output, and label client/live-only observations separately. This corpus mapping does not authorize tool, production, live, real-pack, or measurement changes.

## Source hash manifest

SHA-256 hashes are of the inspected current files; lengths are omitted to keep the mapping readable. The rs2b0t worktree had no dirty or untracked paths at inspection.

```
56215ff792f2571a99c67671da8d1c71901df161715ef8b12b305e59be33217b  e2e/nav-script-travel-live.ts
98cbb06e2ba1ed6bcd4e3d698982171123e13a67563baa840d186b16cfaabcbe  e2e/nav-path-paint-live.ts
206ef0d21d6ed546808d2993f0051c5d9186eb3600f5ef06243fbbfca1ab3eb8  e2e/nav-script-routes-live.ts
b3fbe043e5354929a43b675244de9aade975262e3df12e4135f16c64360c2382  e2e/nav-stress-live.ts
1799607d03151bed7cbc956051bc490f19f203a0cbf333641c55f383fa47b0ca  e2e/nav-two-route-smoke-live.ts
1aebcdabc3d71ece90860a56c9254920a89ed4613b475cba1143ab05a21ca641  e2e/nav-tele-smoke.ts
dcc3a6c481dd3cb7a059cea6e6a6025006633deb1c37fff51cddadad7b9c37cb  e2e/clues/hardclue-nav-live.ts
7406c8d4a6965f54879ac9cc2b6eed7233b08553781c161ef82d01f5230316cc  e2e/lib/navLiveHarness.ts
1488c22332c2dab756a753f6767a4520277f94922b25950ab51a014172e67762  tools/nav/script-route-corpus.ts
1e45585657f60e30bea264859dad8d4dcfe37a5cb355bfbe63efe79f25d7754a  tools/nav/script-travel-corpus.ts
363424826117967bb4b0e7ac6480b12606ba0f7292539fa446bf176cb30ebe1a  tools/nav/mainland-corpus.ts
085316415c11eefad6d6605eb04d5029b3544e798cfc1d50748fc4daf6b1fe35  tools/nav/transport-heavy-routes.ts
510813f9b7563cdc8c587db1c539d2666d4fd3320d7572a0df33b7aa44d7e938  tools/nav/script-routes.generated.json
88a60326891002c8b796fa686a2517ad2d1b28c0162f6197e2864feab4aa7ebd  tools/nav/script-routes.hardest.json
5bc1790704d9e0d256c24bca07f8f09dd367ab941db37b580ce712e635564aec  tools/nav/script-travel.generated.json
857c2d9430a85a2f81fb75c6471c69003b5fcda248e575feba7bdece459f2782  tools/nav/transport-heavy.routes.json
a63752fe5713a7fe602e7f544d9eff5ff58e3ac9888e0e1cd07d0dc2f54c6c95  tools/nav/mainland-routes.json
```
