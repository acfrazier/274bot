# Generated navigation corpus extension

Task `t_2aabed22`, branch `codex/memory-diagnostics`. Generated correctness
qualification PASS; same-card review is requested, not yet approved. No new
actual navpack read/execution, native run, live account, foreign runtime, server,
SSH/builder work, performance measurement, production edit, STATE edit, merge,
or push occurred. The previous real comparison and its coverage limits stand.

## Frozen production and bounded tool change

- Dense: `29b7aea779322c8611f83dc193e939ca7d756f75`.
- Tiled: `8385babb23fd15b876506d4a3f6154984a6b2df1`.
- Client, both arms: `3456edc8dabf7b25ada78110ffa56327af9f67a4`.
- Starting reviewed tool: `760d3acbc429931447454af41902e43de8ac5b2e`.
- New evidence: `nav-tiled-differential/corpus-extension-01/`.

Only the task-owned diagnostic tool, generated fixtures/tests, static proposed
selectors and this report changed. Original nav/router/host/client source is
materialized separately per arm and verified against the frozen Git objects.
The existing storage adapters and exact six host helper spans are unchanged.
Both probes built offline into new independent targets, exit 0. Neither an old
admitted run nor its binary was overwritten or recompiled.

Named presets append to, rather than reinterpret, the old four facts:

| Preset | Carried inventory | Skills | All other facts |
| --- | --- | --- | --- |
| 4 `varrock-smoke-min25` | Law 563×50, Air 556×150, Fire 554×50 | magic `(6,25)` | empty |
| 5 `carried-glory4` | charged glory 1712×1 | empty | empty, including worn |
| 6 `ship-coins5000` | coins 995×5000 | empty | empty |

Preset 4 deliberately uses the **minimum level 25**, not the source's `maxme`
account setup. Preset 6 copies only the rich probe's coins, not its full rich
state. Presets 0..3 retain their exact facts: in particular 2 still carries
coins10 and has **worn**, not carried, 1712. Bank supply remains exactly
`[(995,10),(1712,1)]`. These are not reproduced live accounts.

`fixed_selectors` now admits only families 0..6, with unchanged ten-integer,
coordinate, option, radius, model and row-count bounds. No arbitrary state
parser or new selector kind was added. Every fixed request now emits its
sorted complete state for independent exact-fact assertions. The explicit
`fixed-tele-model` API always tests teleports; option-bit contrasts are checked
against the original host calculation, not that unconditional API.

## Source requirements, not missing guesses

The saved inventory, not an actual pack, was read:
`diagnostics/native-nav-differential-preparation/results-760d3ac/metadata/root-real-selector-inventory.json`,
SHA256 `7a0345a26fe3644d05f4b91d48d6b0713a1340605fc15c853bd3a587e3557edc`.
The case JSON records each complete selected edge, inventory hash and index-line
anchor, plus every foreign origin source's full hash and line range.

| Saved edge | Requirements and relevant fields |
| --- | --- |
| 2143, Varrock `(3213,3424,0)` | Teleport; magic `(6,25)`; Fire `(554,1)`, Air `(556,3)`, Law `(563,1)`; ticks3, loc0, option0; **quests/varps/worn all empty** |
| 2166, Edgeville `(3087,3496,0)` | Teleport; carried `(1712,1)`; ticks2, loc1712, option4; skills/quests/varps/worn all empty |
| 2167, Karamja `(2918,3176,0)` | Same carried-glory requirements as 2166, different destination |
| 2089, Musa deck `(2956,3143,1)` | Boat, NPC378 at `(3026,3217,0)`, option1, ticks7, coins `(995,30)`; skills/quests/varps/worn all empty |
| 2095, Brimhaven deck `(2775,3234,1)` | Boat, NPC381 at `(2679,3275,0)`, option1, ticks7, coins `(995,30)`; skills/quests/varps/worn all empty |

Varrock has **no packed quest requirement**. This resolves the older mapping's
MISSING placeholder without editing that approved historical mapping. Ship
fare and packed requirement absence are also resolved, not left generic.

Independent Rust source anchors in `crates/nav/src/transport.rs`:
rune IDs `5711`, Varrock minima `5757–5760`, jewellery IDs `5781–5785`, glory
requirements `5823–5838`; Sarim boat `1864–1895`, Ardougne boat `1961–1991`,
and fare assertion `5162`. These anchors apply to the frozen source spans;
the complete dense transport file hashes to
`45bfa7ea4a9d6cc6c43d936635b35a71b238d4eff27c68eba15adcdc3cef58d8`, tiled to
`e7857c5743676b82eb847178f1ea1617a878203a1e7c4df3ad8d7e4568e69812`.
Their diff contains storage-access/fixture-construction adaptations, not new
spell, jewellery or boat requirements. Frozen source manifests are archived.

Foreign sources are static-only, pinned to the approved hashes at rs2b0t commit
`100adccc037d9f6898080e1cad58fcfc43364775`. Tele-smoke `49–54` supplies rune counts,
`86–95` maxme/minimum, and `115–122` requested radius4. Stress `486–504` supplies
carried glory and the Al Kharid→Edgeville OD. Transport-heavy `158–171` supplies
the two ship ODs, and `188–218` its rich coins5000. No Bun, Node, Playwright or
foreign source was executed.

## Generated proof and independent assertions

One new **337-byte generated** pack contains three gates on disconnected tiny
planes: rune+magic Teleport, carried-glory Teleport, and coins30 Boat. Its
requirements are authored explicitly in the fixture, independently of `facts()`.
The coordinates/planes are synthetic and deliberately isolate gates; this is
not reconstructed real-world geometry or a claim about native route selection.

All 12,953 old corpus entries retain their exact ordered metadata/hash/bytes as
a prefix. The old 90 fixed rows remain a byte-identical prefix. The extension
adds 80 fixed rows: enabled/disabled/missing-item combinations, radius0/4 and
both explicit cost models. All six tiny all-pairs maps use the same 170 rows;
the new graph is also exercised through the existing one-input executable
protocol (argument `real`, **generated bytes only**, unchanged GENERATED caps).
The prior run's `routes-open` protocol artifacts remain untouched.

| Result | Observed |
| --- | --- |
| Generated input count | 12,954 |
| Total input bytes / largest input | 10,601,143 / 19,053 |
| Dense/tiled full output equality | 328,120,731 bytes, direct byte comparison PASS |
| Full output SHA256 | `464fe7676fcd6edbc5b2e06ed24bda6a32137d37a5cb99932bb13b700eda73f1` |
| One-input protocol equality | 80,987 bytes, direct byte comparison PASS |
| Protocol output SHA256 | `e605a7623ad8de3420eabf84a0f447bfc456ed63afeec1222c8f5fb0fc93c686` |
| New generated pack SHA256 | `6b324fdd44c40cca6ea7920ce868d54c47d40e49d11d301c152f2565403523a4` |
| Fixed selectors SHA256 | `9d7a44db107ec10d55f11267a19062b67c2ec149e31a7cfe050253e2ec2fd337` |
| Corpus SHA256 | `42b62bb013818aa3e88ad5a3164e624bbac61a2f10ac97e72ddafd91f3729d79` |

For **each arm and each protocol**, the independent 80-row assertions require:
12 routed Teleport outcomes, 8 actual BankSession outcomes, 8 Boat outcomes,
and 52 NoPath controls. These are checked results, not intent-label counts.
BankSession assertions require Walk/Open/DepositAll/Withdraw1712×1/Close,
no Wear, bank-fetch disabled for pending re-find, and exact equality of the
final route and reported host route containing Teleport. Worn-only glory cannot
substitute for carried glory. Missing spell requirements cannot be supplied by
the fixed bank. Coins10 plus bank coins10 cannot pay the generated coins30 fare.

Full ordered route Debug payloads, f64 tick bits, graph/requirements/banks,
canonical and decoded roundtrip wire, cells, blocked words, bounds, sidecars,
errors and complete-input framing are compared, not merely leg names or hashes.
The audit verifies every original source object, exact host source/spans,
executable/source admissions before and after, every input, full output bytes,
frame inventory, noncanonical ordered/trailing inputs, and both independent
extension assertions. `audit.json` has `verified:true`; `result.json`'s
`qualified:true` alone denotes process/wire equality, not the stronger audit.

Selected frame counts: 12,954 input; 7,776 ordered pairs; 497,664 each options
and walking-options; 1,020 fixed-selector/model/tele-model; 655,360 local step;
2,856 cell and blocked-word; 571 each canonical/round-canonical; exactly one
completion frame. Full counts, errors and source admissions are in `audit.json`.

Tests: **26 run, 25 pass, one Darwin hard-AS skip**, no unexpected failure.
The dedicated unchanged guard suite separately ran 18 tests with the same one
hard-AS skip. `native_real_qualified:false`. CPU, RSS, output, wall, owned-child
cleanup, EOF handling, nonzero receipts, symlink/hash/source/executable mutation,
unsafe dimensions/strings, and malformed/out-of-range selectors were exercised.
No guard, cap, authorization or cleanup relaxation was made. GENERATED remains
240s wall/180s CPU/1GiB RSS/64GiB configured AS/512MiB output per arm; Darwin AS
is not effective or accepted. REAL limits and root-release gating are unchanged.
Diagnostic supervisor timings/peaks are not performance measurements or savings.

## Proposed native cases — no execution or prediction

`nav-tiled-differential/proposed-native-extension-01/cases.json` freezes
**19** rows and source/adaptation/requirement provenance; `routes.tsv` has only
the ten integers per row, ready for a future separately authorized wrapper.

- Varrock `(3222,3218,0)`→`(3213,3424,0)`, radius4/model0:
  spell enabled preset4, disabled preset4, missing-runes preset1.
- Glory stress `(3293,3174,0)`→`(3087,3496,0)`, radius4/model0:
  carried preset5, disabled preset5, worn-only preset2, missing preset0.
- Mainland F2P-01 stairs `(3222,3218,0)`→`(3208,3220,2)`, radius0/model0,
  options0/preset3; F2P-06 `(3213,3424,0)`→`(3222,3218,0)`, radius4/model1,
  options0/preset3. Mainland origins/destinations are at source JSON lines4/9;
  model/radius adaptations are labeled, not attributed to live golden routes.
- Source ship ODs `(3027,3218,0)`→`(2956,3143,1)` and
  `(2683,3272,0)`→`(2775,3234,1)`, radius0/model1/options0:
  coins5000 preset6, coins10 preset2, missing coins preset0 for each.
- **HOST-specific** Glory Karamja proposal `(3222,3218,0)`→`(2918,3176,0)`,
  radius0/model0: tele+bank options5 with missing preset0, worn-only preset2,
  already-carried preset5; bank-disabled options1/preset0 contrast.
  The origin is source Lumbridge; the destination is saved packed edge2167.
  This is a constructed host test, not an rs2b0t bank case or predicted session.

BANK is host `plan_bank_fetch` orchestration after a missing-item diagnosis,
not a graph edge or an ordinary walk to a bank. Native selection could still
find another route; **successful real Teleport/BankSession remain open**.

Requested radius4 remains distinct from tele-smoke post-hoc arrival8. The actual
stress glory source checks post-hoc distance≤12; it must not be mislabeled as8.
All post-hoc live arrival, logs, paint, pacing and execution assertions are
excluded from Rust router acceptance. Membership, freeSlots,
distanceBeforeTeleport, allowTeleportIds and foreign router policy are excluded,
not silently implemented. Empty Rust quest requirements do not establish live
membership or policy eligibility.

Existing essence exit evidence is retained. No entry/return rows were added;
preseeded opts8 does **not** prove an entry-created full roundtrip or session
derivation. The original essence-session limitation remains explicit.

Proposed TSV SHA256:
`34a7d8ef78c8755dbe2ffc775977bdb4b8fc2ec8fd114fac3412b0a936d5edbc`.
Proposed JSON SHA256:
`3255cd30f3ae6003a786394ed5a379cb5444ee2d5535f178fad3bbd3db458bcf`.

## Immutable evidence, hashes, failures and reproduction

New archive `nav-tiled-differential/evidence/corpus-extension-01.tar.gz`:
18,777,315 bytes, **13,445 regular files**, SHA256
`2813bcaf24f344b928c21d45206c6c0fb589e072af9b0626c8e127ae8ea3b373`.
Every archived file was streamed back and SHA-verified against the companion
`corpus-extension-01-archive.json` manifest. This includes full output/input,
frozen source, recipes, admissions, tests, audit, preservation receipts,
Darwin guard receipts and proposed cases. Targets/executables remain local;
executable hashes and toolchain identities are retained in admissions.
Old corpus, archives, guard receipts and failure receipts remain unchanged.

Changed/added executable tool SHA256 values:

| Tool | SHA256 |
| --- | --- |
| harness.py | `099c4bf8e25bf09b6181e5d51575872340f3b27fc286341315fc6fa799969ba6` |
| probe.rs | `ee91e2370d04de8a1c9b9d55eaf9cbb3c8bbd9d0153b4748208fc50656e40cd8` |
| generate.py | `f51c88b73186b9e7a50ea45494cc160523339d2be9c6e10f8134d7de05bdd784` |
| audit.py | `cdda66e4c3add644341337cd61aafa49cd8d333ff35f7df2b619460f903c2892` |
| test_admission.py | `2ccaaf3feaaa7f506f7f2405a2df94702b71aa19f339c73209f3fadfa0fd8d04` |
| test_extension.py | `de4b74c4084ef3a645e7027e89effa3fe8c4b0565ceb48d0940f49eca24faf85` |
| propose_extension.py | `baf4b24dfc3eb8d8c5d0c6bac7855338d5abcfd2e3c8577977aa6fa9bb7c7fc6` |

Failures disclosed: test-first admission rejected presets4/5/6 before the
bounded parser extension, then passed. The independent integration assertions
failed against the unmodified old run's streams (missing new coverage/state),
then passed on the new run; a retained expected-failure replay is
`red-against-original.out`. There was no failed new build/probe or audit, no
recompilation of an admitted run, and no retry with weaker limits. Two initial
context-inspection calls (execute_code and inline Python `-c`) were denied by
headless tool policy; saved metadata was inspected with normal read/JQ tools.
These were not execution proofs or native failures.

Reproduce using a **new** run directory and the README's
`prepare → build → generated → audit` commands, followed by the full suite with
`NAV_EXTENSION_RUN` set, a new guard directory, and `audit --archive-only`.
Static proposals reproduce with `propose_extension.py --source <approved rs2b0t>
--out <new-owned-proposal-dir>`; it verifies approved foreign source hashes and
saved numeric requirements and never opens a pack. Archive restore likewise
uses a new directory; do not replace accepted sources or rebuild in place.

After same-card reviewer approval, only root may release fresh native hard-AS
qualification and a bounded additional real corpus. No Stage A/B, residency
saving, successful native transport or live behavior acceptance is claimed.
