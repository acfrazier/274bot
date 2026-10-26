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

Without an explicit profile, the saved panel revision chooses `local-274` or
`local-289`; otherwise local 274 is the default.

Local engine roots are never inferred from `HOME`. A local profile resolves
the root in this order: `--engine`, `ENGINE_DIR`, then `login_key.engine_dir`
in `~/.274bot/servers.json`. New `servers.json` files leave that path unset;
without an explicit value, a local profile fails fast with setup guidance.
The public WSS profile does not need an engine root.

All profiles cache fetched client archives under `~/.274bot/unpack` (274) or `~/.274bot/unpack-289` (289) unless `--cache`/`--unpack` overrides them. This repo ships no Jagex assets; the client fetches `/crc` and jags from the selected profile's asset endpoint.

Beta's **tested** path is the **local** engine on revision **289**. Revision 274 is best-effort: it may run, but it is neither tested nor claimed. The public world is built in for login/asset fetch; it is **not** a hosted wall, **not** Jagex, and there is **no** public-world CI or SLA.
