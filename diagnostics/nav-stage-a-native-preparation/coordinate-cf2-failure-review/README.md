# coordinate-cf2-failure-review

Independent reviewer probes for task `t_7ef3743e` against frozen failed CF2
evidence at commit `a1b2d329448c01df05c6343f2864650e01c4e263`.

| File | Role |
| --- | --- |
| `independent_cf2_failure_review.py` | Stream-hash all 316 archive members; revalidate failure/claim/checkpoint/auth, 61 children, selectors, budget, accounting, protocol next boundary |
| `verdict.json` | Structured verdict + accounting + classification + invariants |
| `checks.json` | Flat boolean check map |

Run (from worktree root):

```
python3 diagnostics/nav-stage-a-native-preparation/coordinate-cf2-failure-review/independent_cf2_failure_review.py
```

Report: `docs/memory/nav-coordinate-cf2-failure-review.md`.
Raw archive stays local/untracked under `coordinate-cf2-result-01/`.
