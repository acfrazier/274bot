# Native canStep focused evidence

Raw command outputs in this directory were generated from the exact source
export at `.superpowers/task-exports/t_1a213450-5612b265` (host
`5612b2652968ebcfa6577bf4439f183de1fd5e21`, client git archive
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`, tarball sha256
`ffe863519e87cc44e56823bab93ab275f42408a9f0ccbf8fcbc070cf7c881a0f`) using a
new empty `CARGO_TARGET_DIR=.../target-t_1a213450`. Concurrent worktree files
were not restored or copied over. Client identity is that archive hash, not
`git rev-parse` inside the gitless export. `verification.json` records
cwd, `CARGO_TARGET_DIR`, commands and outcomes. This is not LIVE acceptance.

`diagnostic-lock-wait/` notes the timed-out first script-lib compile and the
package-cache lock wait on `load_isolate`. Operator clarification: a package
cache lock does not invalidate an otherwise verified empty-target run.
