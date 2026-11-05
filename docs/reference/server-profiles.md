# Server profiles (revision + world)

> **Audience:** anyone who chooses a game server or edits `~/.274bot/servers.json`.
> This text moved here from the README unchanged. For a plain-words guide, read
> [Choose a game server and a world](../manual/servers-and-worlds.md). If you
> only want to play, start at [Install and first run](../manual/install-and-first-run.md).

A session binds one entry from `~/.274bot/servers.json` for the whole process.
The file is created on first use with these built-ins:

| Profile | Revision | Transport | Default game | Default assets | Default vault |
| --- | --- | --- | --- | --- | --- |
| `local-274` | 274 | TCP | `127.0.0.1:43594` | HTTP `:80` | `~/.274bot/vault` |
| `local-289` | 289 | TCP | `127.0.0.1:44594` | HTTP `:1080` | `~/.274bot/vault-289` |
| `local-377` | 377 | TCP | `127.0.0.1:43595` | HTTP `:81` | `~/.274bot/vault-377` |
| `rs2b2t` | 289 | WSS | first roster world (default `w1.rs2b2t.com:443`) | HTTPS `:443` | `~/.274bot/vault-prod` |

Select any stored name with `--profile NAME` on `host-play`, `panel-play`, and
`tui-play`, or with `BOT_SERVER_PROFILE`. `--rs2b2t` selects the public built-in;
`public-289` remains a name alias. `--prod` and `BOT_TARGET` were removed.
Connection overrides apply only to the selected profile and fail closed when
they conflict with its revision, roster, or plaintext policy.

The resolved profile is `Local` only when its transport is TCP and every game
host is loopback; only that class admits client cheat packets. Every WSS
profile is `Remote`. A TCP profile may opt in to a non-loopback endpoint with
`allow_plaintext_offhost: true`, but remains `Remote`; the opt-in is rejected
on WSS profiles.

Public play uses the `rs2b2t` roster, with w1, w2 and w3 bundled by default.
The shared cache is prepared from the first world, trying the
next if its asset server is unreachable; `~/.274bot/unpack-289` remains shared.
The exact 0.2.0 `servers.json` roster with w1 and w2 is extended once;
`servers.json.pre-0.2.0.1` is the byte-exact backup and completion marker, so
later edits (including removing w3) stay respected. If the migration cannot be
written or `servers.json` is a symlink, the app warns and keeps the existing
roster. Other world lists are left alone.
Each account can select `auto` or a listed world in the panel Profiles editor.
An auto account moves to the next listed world when login says it is full,
waiting after all worlds report full; pinned accounts stay put. Panel and TUI
show the active world beside each slot. `tui-play --world N` sets a session
default only for accounts stored as auto. Public login fetches the current
RSA modulus from that world's `/client/client.js`, caches successful fetches
per host and refreshes on a wrong-key login response; a failed fetch uses
the baked key without caching it.

`--world-members true|false` is an operator-declared WORLD property, held immutable with the profile; explicit overrides always win. It is not a client/server packet observation, `NODE_MEMBERS`, or account membership. Without an override, a Local profile uses `node.members` from its engine's `data/config/world.json` only when that file declares the selected revision. Otherwise, `rs2b2t` declares members only when every configured world is an rs2b2t world (`w<N>.rs2b2t.com`, ASCII numeric label, case-insensitive, port 443). Other unresolved hosts, ports, and mixed rosters remain unknown and route as F2P. Profile-bound clients also receive `members=false` for unknown or known-free worlds; that prevents members map squares from being prefetched and makes members-object interface counts unavailable, while login remains unaffected.`

On the first launch after upgrading, a valid `~/.274bot/worlds.json` is copied
into the `rs2b2t` roster. The original is untouched; `servers.json` is
authoritative thereafter. Each profile stores its vault filename explicitly,
so existing `vault`, `vault-289`, and `vault-prod` files are not renamed or
rewritten.

Each panel/TUI map Walk confirmation emits one Info-level `WalkTo` outcome per requested slot through the shared host log sink, including refusals. It records origin, the routed destination (including snapping), teleport/wilderness/bank-fetch options, effective WORLD membership and compact provenance (`unknown`, `explicit`, `rs2b2t` or `local-world-json`), then route legs/walk steps/transports or the refusal reason. Local declaration paths and digests are not included. Unfocused refusals appear in the Process log; an empty group keeps its selection and reports “no bots selected.” Availability checks and frame-by-frame following do not emit request lines.

Without an explicit profile, `--revision`, `BOT_REVISION`, or the saved revision
chooses the matching local profile. Otherwise, local 274 remains the default.

Local engine roots are never inferred from `HOME`. A local profile resolves
the root in this order: `--engine`, `ENGINE_DIR`, then `login_key.engine_dir`
in `~/.274bot/servers.json`. New `servers.json` files leave that path unset;
without an explicit value, a local profile fails fast with setup guidance.
The public WSS profile does not need an engine root.

Profiles cache fetched client archives under `~/.274bot/unpack` (274),
`~/.274bot/unpack-289` (289), or `~/.274bot/unpack-377` (377), unless
`--cache`/`--unpack` overrides them. This repo ships no Jagex assets.
The client fetches `/crc` and jags from the selected profile's asset endpoint.

Beta's **tested** path is the **local** engine on revision **289**. Revision 274 is best-effort: it may run, but it is neither tested nor claimed. The public world is built in for login/asset fetch; it is **not** a hosted wall, **not** Jagex, and there is **no** public-world CI or SLA.

Revision 377 is an explicit local development target. `host-play --revision 377
--engine PATH` selects Fairy Ring's local endpoint. It uses the pinned 377
client decoder, including revision-specific config and interface fields.
Low- and high-memory clients keep separate mode-dependent tables.

The selected cache and navigation pack must carry real revision-377 identities.
The driver does not borrow revision-289 data or navigation, relabel a cache, or
skip a resource check. A changed bound archive or an incompatible pack refuses
the affected operation. Generated game facts attach only when their cache
identity and pinned source bytes match; otherwise the runtime logs why it
withheld them.

The bounded `client_377` scenario uses the existing
`panel-play --revision 377 --live script_client_377` harness. After the normal
mainland seed, it captures scene 2, sends a short direct courtyard walk without
the whole-world router, waits for the exact destination, and talks to the live
NPC with selected-cache type 0 (Hans). It requires a closed dialogue before the
send and captures the new dialogue. This client qualification does not qualify
combat, gathering, questing, banking, or other game-data-dependent families.
