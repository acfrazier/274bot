# JS API v2 (native host)

JS API **v2** is the native host authoring contract. It is independent of
application release 0.1.8 and of client 274/289. **v1** remains the rs2b0t
compatibility load path.

Declare both:

```ts
export const apiVersion = 2;
export function tick(api: NativeApi): void;
// or: export async function tick(api: NativeApi): Promise<void>;
```

`export const apiVersion = 2.0` is the same version as `2`. Comments and
string mentions of `apiVersion` are ignored. An explicit version that does
not match the file's bot shape is rejected (`api-version-conflict`,
`api-version-malformed`, `api-version-unsupported`,
`api-version-missing-tick`) and never silently falls back.

Unversioned `export function tick` files keep today's loader: `api.tick` is
the only member.

## NativeApi

Generated types live in `crates/script/host-js/index.d.ts` (`NativeApi`,
`NativeOp`, `NativeSnapshot`). The public handle is:

- `tick` — current game tick
- `snapshot` — read-only view of the host snapshot (do not mutate; copy if
  retaining past this tick)
- `settings.str` / `num` / `bool` — per-identity bag (`export const SETTINGS`
  is still the schema)
- `log(message)`
- `stop(reason?)`
- `paint.begin().title().row().gap().end()`
- `request(op)` — enqueue one supported op; **returns void**

Do not import rs2b0t modules or touch `__rs2b0t_host`. Unsupported
`request` ops throw `not impl: request.<op>`.

### Supported requests

`held`, `open-booth`, `open-stand`, `close`, `set-note-mode`, `withdraw`,
`withdraw-load`, `withdraw-x`, `walk-nearest-bank`.

Completion is the next snapshots' seq/result fields, not a Promise.

- Pass `snapshot.bank_generation` into `withdraw-load` / `withdraw-x`. A
  changed generation is stale, not bank exhaustion.
- `!bank_loaded` is unavailable contents, not exhaustion.
- `walk-nearest-bank` uses host packed nav. Watch `walk_outcome_seq` /
  `walk_outcome_failed` and `here` vs `banks`. No packed stand fails closed
  in Rust.

User script owns bury/restock business logic. Navigation, action sequencing,
random handling and recovery stay in Rust.

## Sync and async tick

A v2 `tick` may be async. The isolate will not re-enter `tick` while that
Promise is pending. Snapshot posts still merge. Prefer a **sync** tick that
polls `api.snapshot` across game ticks.

Pause / Stop / disconnect / reload drop queued actions of the old
generation. Settings updates replace the bag for later ticks. Per-profile
settings stay isolated by card identity.
