# Review: native extended teleport / bank / ship differential

2026-09-09. Independent round-1 **artifact** review of root commit
`a6bb0e8e54b37989ad68569ede6934ce5e0fb4cd` on branch `codex/memory-diagnostics`.

- Reviewer profile: `reviewer` (configured default `grok-4.5` / `xai-oauth`).
- Scope: `docs/memory/nav-corpus-native-extension-report.md` and local evidence under
  `diagnostics/native-nav-differential-preparation/results-ccff4bb/` only.
- No production/tool edits, builds, SSH, native re-runs, pack probes, remotes, or
  Stage A files were modified by this review.
- Parent tool `ccff4bbe22d63cdb8b11612aaf97bbbadcb256dc` previously approved
  (`t_2aabed22`).

## Verdict

**APPROVE** bounded native real-pack correctness evidence for the 19-case
extension. Claims that matter for correctness reproduce from preserved archive
bytes and extracted metadata. Missing Stage A/B / RSS / latency evidence is
correctly scoped out and is not treated as a correctness failure.

## Independent checks (reproduced)

### Commit and pins

- `a6bb0e8` touches only `docs/memory/nav-corpus-native-extension-report.md` and
  `docs/memory/STATE.md` (docs/state boundary note).
- Frozen pins present in admissions/audit sources and Git metadata packs:
  dense `29b7aea779322c8611f83dc193e939ca7d756f75`, tiled
  `8385babb23fd15b876506d4a3f6154984a6b2df1`, client
  `3456edc8dabf7b25ada78110ffa56327af9f67a4` (host pack + client pack;
  client tip also resolves in `vendor/fr-client-rust`).
- Tool pin `ccff4bb…` is the approved parent.

### Archives (own hash, not trust of root JSON alone)

| Artifact | Bytes | SHA256 | Regular files | Uncompressed |
| --- | ---: | --- | ---: | ---: |
| `native-run.tar.gz` | 33,126,868 | `9672e28c111abf4ac6c80c2a146946e9a54ddfbb0cadf08b5b7b91a2a3e3c493` | 13,443 | 4,992,792,839 |
| `native-extension-metadata.tar.gz` | 3,019,060 | `6619102ac2f6c91a3a5ea185e838c4f3a6c91fe53edba31082a3afc90f1622db` | 474 | 13,795,799 |

- Full membership streamed and hashed: **13,917** files total; matches
  `root-archive-verification.json` and the report.
- Full generated and real probe outputs live in `native-run.tar.gz` (not only
  extracted metadata).

### Generated qualification (native)

- Streamed full dense/tiled `*.out` equality (not checksum-only):
  **328,120,731** bytes, SHA256
  `464fe7676fcd6edbc5b2e06ed24bda6a32137d37a5cb99932bb13b700eda73f1`.
- Protocol outs equal **80,987** bytes, SHA256
  `e605a7623ad8de3420eabf84a0f447bfc456ed63afeec1222c8f5fb0fc93c686`.
- `metadata/native-run/audit.json`: `verified: true`; extension counts on both
  arms × generated+protocol:
  Teleport **12**, BankSession **8**, Boat **8**, NoPath **52**.
- Sources record dense/tiled commits and executable SHAs matching admissions.

### Native guards and real-wrapper stand-in

- `native-guards-01/guards.json`: `generated_guards_passed: true`,
  `native_real_qualified: true`, Linux glibc platform.
- `guard-tests.err`: **Ran 18 tests … OK** (stderr carries unittest `-v` output;
  `.out` empty). Includes `test_address_limit` (hard AS) and cleanup/timeouts.
  Receipt `returncode: 0`, `address_guard_active: true`.
- Generated-fixture real-wrapper admission: outs equal 80,987 bytes with same
  extension gate counts; `qualified: true`; scope text states generated fixture
  only, not the real pack.

### Real 19-case release

- Authorization file SHA256
  `e2e5f9c16230575dde48269e099e5408ae629d65f65b42977f50bf8953997f32`
  (`released: true`; wall 900 / CPU 800 / RSS 1GiB / AS 4GiB / out 4GiB).
- Routes TSV SHA256
  `34a7d8ef78c8755dbe2ffc775977bdb4b8fc2ec8fd114fac3412b0a936d5edbc`
  (19 rows; byte-identical to approved
  `proposed-native-extension-01/routes.tsv`).
- Cases JSON SHA256
  `3255cd30f3ae6003a786394ed5a379cb5444ee2d5535f178fad3bbd3db458bcf`
  (matches proposed cases; HOST notes document fixed bank
  `[(995,10),(1712,1)]`).
- Pack input SHA256
  `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`,
  73,438,581 bytes.
- `real-release/result.json` SHA256
  `3ef53fbbbf2d39c7a0fe93b59334924cbee851ae183427d1c46710e1621d0f59`:
  both arms `returncode: 0`, `address_guard_active: true`,
  `comparison.equal: true`, `output_bytes: 2119093915`, `qualified: true`.
- Streamed full dense/tiled real outs from the run archive:
  **2,119,093,915** bytes equal; SHA256
  `39e5becd9e82ff0565e032346ad447a0047c6657e594bec48e48a7f732d5f271`.
- `root-real-extension-audit.json`: `verified: true`; same output SHA on both
  arms; geometry origin `(1856,1280)`, `1792×9088`, **65,142,784** cells;
  `binary_frame_counts` cells=2 and blocked-words=2 per arm.

### Outcomes, legs, bank plan (no double-count)

Per arm, identical 19 cases:

| Outcome | Count |
| --- | ---: |
| Route | 12 |
| BankSession | 2 |
| NoPath | 5 |

Leg totals on successful Route+BankSession cases (once each; not final+route):

| Leg | Count |
| --- | ---: |
| Teleport | 5 |
| Door | 4 |
| Stairs | 2 |
| Boat | 2 |

Frame inventory supports the accounting: `host-route` 14, `host-bank` 2,
`host-final` 2, `host-outcome` 5 (NoPath). For both BankSession cases,
`host-final` payload SHA == `host-route` payload SHA.

Bank sessions (`HOST-glory-karamja-fetch-missing` and
`…-fetch-worn-only`) plan exactly:

`Walk {3268,3169,0}, Open, DepositAll, Withdraw {id:1712,count:1}, Close`
→ destination `WorldTile {2918,3176,0}` with
`allow_bank_fetch: false` and no Wear step; final leg Teleport.
This is planning under fixed bank supply, not live withdraw/teleport execution
(explicit in audit `limitations` and report).

Semantic spot-checks matched the report table, including:

- Varrock enabled → Teleport; disabled/missing-runes → ordinary Route.
- Glory carried+enabled → Teleport; worn-only
  `([(995, 10)], [1712], …)` (preset-2 style coins+worn) does **not** Teleport;
  missing/disabled controls behave as claimed.
- Ships: coins5000 → Boat; coins10 / missing → NoPath (both Sarim and Ardougne).
- Host Karamja: missing and worn-only → BankSession+Teleport plan; bank-disabled
  → NoPath; already-carried → direct Teleport Route.
- Mainland stairs: three Door + two Stairs legs.

Dense and tiled case records (id/outcome/legs/row/state/bank/payload hashes)
are identical.

Lane hashes distinguish fixed-model vs fixed-tele-model vs host-route where
teleports apply (e.g. enabled Varrock/glory/ship/already-carried), without
collapsing host API lanes into the fixed models.

### First auditor failure (Git metadata)

- `native-audit-attempt-01.json` classifies failure as missing Git metadata on
  source-only stage (**not** a navigation mismatch).
- Supplied packs: host 110 objects / SHA
  `b9d5e5c4875b4408db6dfad3214944d2dc4d7f2ebe469228fd28a314fe087bfb`
  (commits dense+tiled); client 26 objects / SHA
  `7689a46053e4fa6d2e64a2a3ca61e7bfd096c8cd44f5895c372f65f7432847ee`
  (commit client pin). Independent `git index-pack` on both packs surfaces those
  exact commit tips. Unchanged auditor then `verified: true` with no probe
  retry claim in the preserved chain.

### Performance non-claims

- Real `result.json` wall ~112s/122s and sampled RSS peaks are diagnostic
  I/O-heavy receipts only. Report correctly refuses Stage A/B, resident savings,
  and accepted latency/CPU. Review does **not** promote them.

### Explicit non-coverage (accepted bounds)

- Essence entry-derived full roundtrip still unproven; preseeded exit evidence
  not erased; stated outside this extension.
- No foreign membership / freeSlots / distanceBeforeTeleport / policy import.
- No live client/server/account teleport or bank execution.
- Prior 40-case real proof remains complementary (not re-litigated here).

## Unsupported or out-of-scope (non-blocking)

- None material for the bounded correctness claim.
- Commit also updates `STATE.md` (orchestrator boundary); outside pure report
  file scope but consistent with root ownership and not a correctness defect.

## Conclusion

Root report claims for native generated qualification, guards, real-wrapper
stand-in, and the 19-case real dense≡tiled comparison are reproduced from
primary archive bytes and metadata. **Approve** this correctness evidence
package. Root retains performance release, Stage A reconciliation, and final
whole-branch `branchreviewer` (Grok 4.6).
