# coordinate-cf1-review

Independent reviewer probes for task `t_f7d038ac` against frozen CF1 evidence
at commit `27151fb2bc5356159e51a2ecda3a0228ca0e22ac`.

| File | Role |
| --- | --- |
| `independent_cf1_review.py` | Stream-hash all 82 archive members; recompute selectors, children, budget, accounting, CF2 admissibility |
| `verdict.json` | Structured verdict + accounting + invariants |
| `checks.json` | Flat boolean check map |

Run (from worktree root):

```
python3 diagnostics/nav-stage-a-native-preparation/coordinate-cf1-review/independent_cf1_review.py
```

Report: `docs/memory/nav-coordinate-cf1-review.md`.
Raw archive stays local/untracked under `coordinate-cf1-result-01/`.
