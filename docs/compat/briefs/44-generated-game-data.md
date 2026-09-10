# Generate revisioned game-data assets from the server

Operator direction, 2026-09-10: the bot needs many game-data tables; generate
these programmatically from server code/data and load them with serde instead
of maintaining hand-written Rust tables. This is authorized campaign work.

Use campaign branch codex/rs2b0t-multirevision, AGENTS.md and docs/execution.md.
This task owns ONLY a new generator under tools/game-data/, generated assets
under crates/api/data/game-data/{274,289}/, focused generator verification,
`docs/compat/04-generated-game-data.md` and evidence/generated-game-data/.
Do not edit Rust source, Cargo manifests, client, engines, frontends, or existing
worker files. The bank worker is active. The separate item integration card
will deserialize and consume these assets after its source dependencies finish.

Build a deterministic, rerunnable extraction pipeline. Start with a complete
thin item table: stable obj/debug alias, id, display name, shop cost, stackable,
members, certificate links and wear-position facts where the source has them.
Keep fields useful to current enabled consumers; exclude models/sprites/render
assets. Produce ordinary JSON data, not generated Rust source. Every row must
come from the selected server/content/cache input, not copied foreign ITEM_DB
or newly hand-maintained IDs. Preserve distinct aliases/IDs with identical
client names. Unknown or absent content stays absent.

Known read-only input roots (verify source identities):
- 274 engine /Users/acfrazier/experiments/Server/engine at 4c95f87efe00b068cadbd229d94736626907bd1a, content ../content at 000c19997e07206131bcb3c884265840efce416d.
- 289 engine /Users/acfrazier/experiments/lostcity-289/engine at cc359656b4acd216ca452495874b6beba9a0ac75, content ../content at 92649430fcbc83538d8c4367ecb96cee1a67a944.
- Actual packed client and server metadata lives in each engine/data/pack.
- Engine src/cache/config/{ObjType,ParamType,NpcType,LocType}.ts already decodes
  packed configs with debug names; prefer reusing these offline decoders over
  making a second config/parser interpretation. ObjType.load(dir) reads both
  server/obj.dat and client/config, fixes certificates and can filter members
  facts through Environment. Deliberately handle that filter during generation;
  an operator runtime's membership flag must not silently truncate the pack.
  Content pack/obj.pack is another authoritative alias source if needed.

Do not start/import the game engine application, connect to game accounts,
modify/repack live caches, or mutate server checkouts. Use an isolated generation
process. No server checkout, Node, Python or generator is required when an end
user launches the native app; generated data ships with the Rust application.

Include a versioned schema and manifest binding revision, engine/content
commits, consumed input hashes and client config/cache identity. Runtime matching
can use that identity in the integration task. Avoid machine-absolute paths and
wall-clock timestamps in deterministic payload bytes. Pin the exact relevant
source files when a checkout has unrelated dirty changes; do not pretend a
commit includes those changes. Existing nav cache manifests define the current
selected archive identity in .superpowers/world-capabilities/{274,289}.
No auth, RSA keys, accounts, player databases, logs or runtime world settings
belong in output assets or provenance.

Generate both revisions and compare a second generation byte-for-byte. Validate
schema, unique aliases/IDs as applicable, Rune platebody/chainbody value order,
custom items beyond chainbody, and same-name dragonhide identities against
actual source/cache records. Keep this proof meaningful and bounded; no test
that only restates a generated constant. Document exact regenerate commands.
Record output sizes and record counts without claiming memory savings.

Also inventory the currently hand-coded game facts in api::content (food heal
values, pickpocket requirements, etc.) and pending spell/control/production
families, with exact server source paths for subsequent extraction. Distinguish
server facts from host selection policies such as curated standing spots and
loot keep rules. Do not reimplement those families in this tooling card.

Commit scoped tooling/assets/report. Request review on this SAME card using
profile reviewer, with exact source/output and verification, then STOP. Never
call complete yourself. Actual Grok 4.5 review is required for data fidelity.
