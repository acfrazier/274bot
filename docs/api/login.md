# Login: FIFO throttle numbers

`crates/host/src/login_queue.rs` serializes login handshakes under the 274
server's default throttle. `LoginQueue::request_permit(uid, now)` returns
`Permit::Grant` or `Permit::Wait(duration)` — retry after `duration`. Only
the FIFO head may be granted.

## Constants (the numbers agents must respect)

| Rule | Value | Meaning |
| --- | --- | --- |
| spacing | **2.5 s** (`Duration::from_millis(2500)`) | between consecutive grants |
| per-IP window | **30 grants / 60 s** | `ip_cap = 30`, `ip_window = 60 s` |
| per-uid cap | **4 grants, then 16 s cooldown** | `UID_GRANT_CAP = 4`, measured from the latest grant |
| backoff (response 16, world full) | **20 s + 45 s per prior hit** | `LoginBackoff::delay()` escalates; `reset()` clears |

Defaults are `LoginQueue::default()`; `new(spacing, ip_cap, ip_window)`
exists for tests. A blocked requester waits the longest unmet constraint:
time since the last grant vs spacing, the per-IP window roll-off, or the
per-uid cooldown.

## Backoff

`LoginBackoff` delays retries after a response-16 (world full) rejection:
first retry 20 s, then 65 s, 110 s, … (`20 + 45·hits`). Call `reset()` on
any successful login.

## Queue position and leaving

While a slot waits it sits on the FIFO. `LoginQueue::status(uid)` returns
its place as `Option<QueuePos { position: u32, total: u32 }>` — the **k of n**
snapshot (1-based; a granted uid is popped and no longer present). host-play
mirrors that onto `SlotStatus.queue_position` / `queue_total` while the slot
waits; the panel renders it as **"k of n"** in the status row and as the
queue card over the focused slot. `LoginQueue::leave(uid)` drops a queued
uid (no-op if absent) — the panel's rail ✕ and `stop_slot` call it so a
removed slot does not sit in the FIFO.

## Mainland hop (tutorial skip)

New accounts spawn on Tutorial Island. The cheap rs2b0t `mainlandAccount` **send** is `api::interact::mainland_hop` after `ingame && scene_state == 2`: `CLIENT_CHEAT` `tele 0,50,50,20,20` then `setvar tutorial 1000`. host-play: `--mainland` or `BOT_MAINLAND=1`.

This does **not** relog. Side icons stay tutorial-locked. A clean logout is already wired: `api::interact::logout` presses the `CC_LOGOUT` iface (client code 205) through the doAction path, so client-code logout vetoes still apply ([interact.md](interact.md)). Local engine grants staff cheats when not `production`.

## RSA bake (`BOT_TARGET`)

Cargo already uses `TARGET` for the rustc triple, so the live/prod switch
on the bothost `client` crate is **`BOT_TARGET`**.

| Bake | Env | Modulus |
| --- | --- | --- |
| local (default) | `LOGIN_RSAN` / `LOGIN_RSAE` | engine `private.pem` / Java default |
| live | `BOT_TARGET=live` **and** `LIVE_RSAN` (abort if empty) | **not** the local pem. Public half scraped from `https://w1.rs2b2t.com/client/client.js` (first 250+ digit run — rs2b0t `tools/b0t.sh`) |
| prod | `BOT_TARGET=prod` **and** `PROD_RSAN` | same scrape of the hosted client |

Live/prod exponent is **65537**. Runtime login **code 6** retries **once**
after GET `/loginkey` (plain decimal) then scrape `{origin}/client/client.js`.
`host-play` `TARGET=live` (process env, not bake) defaults host to
`w1.rs2b2t.com:43594` TCP. No WSS.

## Wiring

`api::interact::login` routes the handshake through the driver
(`Client::login`), which opens a fresh stream per attempt and blocks until
the server responds. The FIFO sits ahead of that handshake: request a permit,
wait the returned `Duration` when throttled, then send.


## Panel: Login all vs auto-login

The panel arms logins through `SlotArm` flags (host-play), not
`api::interact::login` directly. Two intents differ:

- **Login all / Log in** is a **one-shot**: it clears the member's logout
  latch, cancels any pending logout, and arms the handshake. Once the grant
  lands the arm disarms, so an unexpected DC leaves the slot on the title
  until the next explicit arm.
- **Auto-login** (the credentials **auto-login on title** checkbox, backed
  by `ProfileSettings.auto_login`, default **off**) keeps the arm armed
  after a successful handshake, so a DC re-handshakes. An explicit
  **Logout / Logout all** latches the member, which blocks even an
  auto-login slot until the next **Login all** clears the latch.
