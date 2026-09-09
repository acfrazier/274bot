# Native real-pack navigation corpus extension

2026-09-09. Root correctness observation after approved tool ccff4bb. Both frozen
arms match all generated and real output bytes. Successful real Teleport and
BankSession route-planning coverage is now present. This is not live transport
execution, performance acceptance, a Stage A/B result or a resident saving.
Independent report review is pending; STATE records later acceptance boundaries.

## Source and release

- Tool: ccff4bbe22d63cdb8b11612aaf97bbbadcb256dc, approved same-card t_2aabed22
  by actual Grok4.5/xai-oauth session20260909_082232_c151b2 (197s).
- Dense production: 29b7aea779322c8611f83dc193e939ca7d756f75.
- Tiled production: 8385babb23fd15b876506d4a3f6154984a6b2df1.
- Both clients: 3456edc8dabf7b25ada78110ffa56327af9f67a4.
- Original revision274 v8 input: 73,438,581 bytes, SHA256
  2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30.
- Approved19-row TSV SHA256:
  34a7d8ef78c8755dbe2ffc775977bdb4b8fc2ec8fd114fac3412b0a936d5edbc.
- Source/requirement/adaptation case JSON SHA256:
  3255cd30f3ae6003a786394ed5a379cb5444ee2d5535f178fad3bbd3db458bcf.
- Root real authorization SHA256:
  e2e5f9c16230575dde48269e099e5408ae629d65f65b42977f50bf8953997f32.

The exact rows and source anchors are described in nav-corpus-extension-report.md
and archived root-real-cases.json. The new presets retain carried/worn distinction,
minimum Varrock magic25, exact rune counts and coins5000. Earlier presets0..3
and their old corpus remain unchanged. Preset2 includes both worn1712 and10coins;
it is not an inventory-identical counterpart to the carried-only preset5.

Root streamed and verified all13,445 files in the approved Mac archive, then
created a source-only native stage with13,418 files/24,209,692 bytes. Payload
SHA256 cfa86be5987716f611d6727ce217c378e350749168517ec5353c3397acc33b02.
Native directory: /home/builder/274bot-campaign/nav-differential-ccff4bb-1228.
This is the separate Hyper-V Linux builder, not the Concord VPS. Boot
 d63ba1f3-9677-4e61-9d4c-f14a8cf502ac, Linux6.8.0-139/glibc2.39; toolchain and
executable hashes are in arm admissions. No old admitted run was rebuilt.
Both new offline builds exited0 with effective Linux address guards.

## Generated native qualification

The generated run passed12,954 inputs, 328,120,731 full output bytes equal;
SHA256464fe7676fcd6edbc5b2e06ed24bda6a32137d37a5cb99932bb13b700eda73f1,
matching the approved Mac stream. The one-input generated protocol passed80,987
bytes equal, SHA256e605a7623ad8de3420eabf84a0f447bfc456ed63afeec1222c8f5fb0fc93c686.

The unchanged auditor checked all input bytes/hashes, original source Git objects,
exact host spans, source/executable admissions, full byte equality, frame counts,
noncanonical wire behaviour and independent extension assertions. Each arm and
each protocol has12Teleport,8BankSession,8Boat and52NoPath required outcomes.
Native18-test guards passed without skips, including actual hard address-space
limit enforcement and owned-process cleanup. No cap or guard was relaxed.

A new copy of the admitted run/executables then exercised the actual real-mode
wrapper on the337-byte generated extension pack, using unchanged REAL limits.
It passed80,987-byte equality and the same positive/negative gate assertions.
This stand-in was generated data, explicitly not the real pack. The original
native run and all original Mac and earlier Linux evidence remain preserved.

## Real results

One root-released attempt ran the19 frozen requests in both arms with unchanged
900s wall,800s CPU,1GiB sampled RSS,4GiB address-space and4GiB output bounds.
Both exited0 without guard failure. Full output equality: **2,119,093,915 bytes**.
SHA25639e5becd9e82ff0565e032346ad447a0047c6657e594bec48e48a7f732d5f271.
Result JSON SHA2563ef53fbbbf2d39c7a0fe93b59334924cbee851ae183427d1c46710e1621d0f59.

Root independently rechecked full byte equality, admitted source/executables,
input and selector hashes, all framing and completion, both65,142,784-cell
passes (15bytes/cell), both8,142,848-byte blocked-word passes, and all19 ordered
selector/state/route outcomes. Geometry is origin1856,1280, dimensions1792×9088,
four complete planes. Complete ordered graph/requirements/indices/banks and
canonical/roundtrip wire payloads are included in the equal stream.

| Case group | Actual host outcome, identical in both arms |
| --- | --- |
| Varrock spell | Enabled+runes selects Teleport; disabled or missing runes selects an ordinary route |
| Carried glory to Edgeville | Carried+enabled selects Teleport; disabled, worn-only or absent does not select Teleport |
| Mainland stairs | Route contains three Door and two Stairs legs |
| Mainland return | Ordinary route succeeds |
| Sarim→Musa and Ardougne→Brimhaven | Each coins5000 case selects Boat; coins10 and absent-coins cases return NoPath |
| Host-specific Karamja bank-fetch | Missing and worn-only cases create BankSession and final Teleport route; bank-disabled returns NoPath; already-carried selects Teleport directly |

Across19 cases:12 direct Route,2 BankSession and5 NoPath outcomes. The14 reported
successful routes contain5Teleport,4Door,2Stairs and2Boat legs. These are observed
leg counts, not counts inferred from test names. Direct and bank-final routes
are accounted once per case, not double-counted from host-final/host-route frames.

Both bank sessions plan exactly Walk(3268,3169,0), Open, DepositAll,
Withdraw1712×1, Close, followed by the Karamja destination(2918,3176,0) with
allow_bank_fetch:false. There is no Wear step. The reported final route is
byte-identical to the reported host route and contains Teleport. This establishes
planning semantics under the fixed bank supply[(995,10),(1712,1)], not a live
withdrawal or execution of that teleport against a server.

## Limits and preserved failure

No whole-client/server/account work occurred. Foreign membership, freeSlots,
distanceBeforeTeleport, allowed-ID policy, live arrival tolerances and pacing
were not imported. Requested radius4 remains distinct from post-hoc source
arrival8/12. Existing preseeded essence-session exit evidence remains; this
extension adds no entry-created full roundtrip or session-derivation proof.
The prior40-case real comparison is preserved and remains complementary evidence.

The first root invocation of the unchanged full auditor failed because the
source-only native stage did not contain Git metadata. It did not report a
navigation mismatch. Root supplied the exact original commit/tree objects and
host/Cargo blobs (110host+26client objects; metadata packs and their hashes are
archived). The unchanged audit then passed. No probe rerun, source/tool edit,
error suppression or guard relaxation occurred. Native-audit-attempt-01.json
preserves the failure and resolution. No new build/probe failure occurred.

Supervisor elapsed/RSS values describe bounded diagnostic processes that write
over2GB per arm. They are not matched cold-load, hot-route, steady-RSS or clean
CPU measurements and must not be applied to Stage A/B thresholds. Stage A tool
qualification/correction and later performance release remain separate gates.

## Durable artifacts and reproduction

Local root: diagnostics/native-nav-differential-preparation/results-ccff4bb/.

- native-run.tar.gz:33,126,868 bytes, SHA256
  9672e28c111abf4ac6c80c2a146946e9a54ddfbb0cadf08b5b7b91a2a3e3c493;
  13,443 regular files,4,992,792,839 uncompressed bytes.
- native-extension-metadata.tar.gz:3,019,060 bytes, SHA256
  6619102ac2f6c91a3a5ea185e838c4f3a6c91fe53edba31082a3afc90f1622db;
  474 regular files,13,795,799 uncompressed bytes.
- Companion manifests enumerate every file size/hash. Root streamed every entry
  of both downloaded archives and verified all13,917 hashes and full membership.
  root-archive-verification.json records this; metadata/ has extracted receipts.
- metadata/root-real-extension-audit.json contains exact cases, state, bank plan,
  payload hashes, counts and framing. metadata/native-run/audit.json contains the
  unchanged generated auditor result. Native/source admissions, guards, root
  authorization/preflight, generated wrapper qualification and original Git
  metadata are preserved alongside them.

Reproduction requires a NEW owned stage and fresh native build/admissions,
generated qualification and guard qualification before an explicit root-bound
real release. Restoring output evidence is not authorization to rerun a pack.
Do not rebuild an admitted directory, overwrite earlier receipts or relabel
these observations as accepted memory savings.
