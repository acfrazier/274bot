# Inactive worktree cleanup

Operator requested unused/merged worktree and local branch cleanup on 2026-09-11.
Root removed 41 inactive worktrees and 43 local branches after verifying backups.
The removed trees accounted for 115.35 GiB allocated before removal; this is not
a net free-space measurement. Only primary main and the active compatibility
campaign worktrees remain. Remote branches were retained.

Memory host `f9975102a079ba4a04ef5c52a30b3d2fd99baec8` is confirmed on
`origin/codex/memory-diagnostics`; its client
`5c73a4a27f3d72834c2c2a071668eb197eb39fd9` is confirmed on
`acfrazier/FR-client-bothost` branch `codex/memory-direct-owner-capture`.

Private backup: `/Users/acfrazier/experiments/274bot-cleanup-backup-20260911`.
It contains verified self-contained host/client Git bundles, binary patches and
a verified compressed archive of 376,923 local files. Archive contents and
vaults are private and must not be published. Rebuildable caches were excluded.
The member list and original file metadata matched before removal. A read-only
artifact directory interrupted removal once; after backup verification its local
directory permissions were made writable and removal completed.

The clean, completed fresh-remote validation clone and temporary client bundle
collector were also removed. Active source, worker caches and primary untracked
files were retained. No aggressive Git garbage collection was run.
See `evidence/worktree-cleanup-20260911/receipt.json` for hashes and exact refs.
