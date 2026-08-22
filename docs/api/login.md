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

## Mainland hop (tutorial skip)

New accounts spawn on Tutorial Island. The cheap rs2b0t `mainlandAccount` **send** is `api::interact::mainland_hop` after `ingame && scene_state == 2`: `CLIENT_CHEAT` `tele 0,50,50,20,20` then `setvar tutorial 1000`. host-play: `--mainland` or `BOT_MAINLAND=1`.

This does **not** relog. Side icons stay tutorial-locked until a later campaign (clean IF logout + login FIFO). Local engine grants staff cheats when not `production`.

## Wiring

`api::interact::login` routes the handshake through the driver
(`Client::login`), which opens a fresh stream per attempt and blocks until
the server responds. The FIFO sits ahead of that handshake: request a permit,
wait the returned `Duration` when throttled, then send.
