type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Qualification-only NativeTick example; not a catalog card. */
export const apiVersion = 2;

const FIELD = { x: 2698, z: 3206, level: 0 };
const PIER = { x: 2683, z: 3272, level: 0 };
const BANK = { x: 2655, z: 3283, level: 0 };
const WRONG_BOAT = { minX: 2780, maxX: 3040, minZ: 3130, maxZ: 3330 };

const STOP_OK = 'route inspect brimhaven qualification complete';
const WALK_TICK_LIMIT = 120;
const INSPECT_TICK_LIMIT = 80;
const READY_TICK_LIMIT = 80;

type Phase =
    | 'inspect-begin'
    | 'inspect-wait'
    | 'snapshot0-send'
    | 'snapshot0-wait'
    | 'walk-send'
    | 'walk-wait'
    | 'done'
    | 'fail';

let phase: Phase = 'inspect-begin';
let token = 0;
let phaseSince = 0;
let readySince = 0;
let clockArmed = false;
let inspectOk = false;
let inspectBarnaby = false;
let snap0Ok = false;
let snap0Barnaby = false;
let snapSeqBefore = 0;
let walkSeq = 0;

function hopHasBarnaby(
    hops: Array<{ locName?: string }> | undefined,
): boolean {
    return (hops ?? []).some((h) =>
        (h.locName ?? '').toLowerCase().includes('barnaby')
    );
}

function ready(api: NativeApi): boolean {
    // NativeSnapshot publishes `ingame` and `here`. `scene_state` exists on
    // the raw ScriptSnapshot / host Core gate, not the v2 native projection.
    return api.snapshot.ingame === true && api.snapshot.here != null;
}

function fail(api: NativeApi, reason: string): void {
    phase = 'fail';
    api.log(`route_inspect_brimhaven_v2: ${reason}`);
    api.stop(reason);
}

function paint(api: NativeApi): void {
    api.paint
        .begin({ accent: '#5b9bd5' })
        .title('Route inspect Brimhaven v2')
        .row('phase', phase)
        .row('token', token)
        .row('inspect ok', inspectOk ? 'yes' : 'no')
        .row('inspect barnaby', inspectBarnaby ? 'yes' : 'no')
        .row('snap0 ok', snap0Ok ? 'yes' : 'no')
        .row('snap0 barnaby', snap0Barnaby ? 'yes' : 'no')
        .row('tick', api.tick)
        .end();
}

function chebyshev(
    here: { x: number; z: number; level: number } | null | undefined,
    dest: { x: number; z: number; level: number },
): number {
    if (!here || here.level !== dest.level) {
        return Number.POSITIVE_INFINITY;
    }
    return Math.max(Math.abs(here.x - dest.x), Math.abs(here.z - dest.z));
}

export function tick(api: NativeApi): void {
    if (!ready(api)) {
        if (!readySince) {
            readySince = api.tick;
        }
        if (api.tick - readySince > READY_TICK_LIMIT) {
            fail(api, 'not ready: ingame && here never arrived');
        }
        return;
    }
    // New unreadiness episode after this ready; do not keep the old stamp
    // (that false-fails later) and do not drop the bound after work starts.
    readySince = 0;
    if (!clockArmed) {
        phaseSince = api.tick;
        clockArmed = true;
    }
    paint(api);
    if (phase === 'done' || phase === 'fail') {
        return;
    }
    if (api.tick - phaseSince > INSPECT_TICK_LIMIT &&
        phase !== 'walk-wait' &&
        phase !== 'walk-send') {
        fail(api, `timed out in phase ${phase}`);
        return;
    }
    if (phase === 'walk-wait' && api.tick - phaseSince > WALK_TICK_LIMIT) {
        fail(api, 'walk after inspect timed out');
        return;
    }

    switch (phase) {
        case 'inspect-begin': {
            token = api.inspectBegin({
                from: PIER,
                to: FIELD,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: false,
                avoid: [WRONG_BOAT],
                timeout_ms: 8000,
            });
            if (!token) {
                fail(api, 'inspectBegin returned token 0');
                return;
            }
            phase = 'inspect-wait';
            phaseSince = api.tick;
            return;
        }
        case 'inspect-wait': {
            if (!api.inspectSettled(token)) {
                return;
            }
            const value = api.inspectValue(token);
            if (!value) {
                fail(api, 'inspect settled but inspectValue returned null');
                return;
            }
            inspectOk = value.ok;
            inspectBarnaby = hopHasBarnaby(value.hops);
            if (!inspectOk || !inspectBarnaby) {
                fail(
                    api,
                    `inspect token result rejected (ok=${inspectOk}, barnaby=${inspectBarnaby}, reason=${value.reason})`,
                );
                return;
            }
            phase = 'snapshot0-send';
            return;
        }
        case 'snapshot0-send': {
            snapSeqBefore = api.snapshot.route_inspect_seq;
            api.request({
                op: 'inspect-route',
                from: PIER,
                to: FIELD,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: false,
                avoid: [WRONG_BOAT],
                request_id: 0,
            });
            phase = 'snapshot0-wait';
            phaseSince = api.tick;
            return;
        }
        case 'snapshot0-wait': {
            const snap = api.snapshot;
            if (snap.route_inspect_seq <= snapSeqBefore) {
                return;
            }
            if (snap.route_inspect_request_id !== 0) {
                return;
            }
            snap0Ok = snap.route_inspect_ok;
            snap0Barnaby = hopHasBarnaby(snap.route_inspect_hops);
            if (!snap0Ok || !snap0Barnaby) {
                fail(
                    api,
                    `snapshot request_id 0 rejected (ok=${snap0Ok}, barnaby=${snap0Barnaby}, reason=${snap.route_inspect_reason})`,
                );
                return;
            }
            if (api.inspectSettled(0)) {
                fail(api, 'inspectSettled(0) must stay false for snapshot-only');
                return;
            }
            phase = 'walk-send';
            return;
        }
        case 'walk-send': {
            walkSeq = api.snapshot.walk_outcome_seq;
            api.request({
                op: 'walk-near',
                x: BANK.x,
                z: BANK.z,
                level: BANK.level,
                radius: 4,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
            });
            phase = 'walk-wait';
            phaseSince = api.tick;
            return;
        }
        case 'walk-wait': {
            // Host `note_failure` bumps walk_outcome_seq. Successful
            // request_id 0 follow does not — isolate walk_wait settles on
            // actual arrival (Chebyshev ≤ the armed radius). Seq increment
            // is the failure seam, not completion.
            if (api.snapshot.walk_outcome_seq > walkSeq &&
                api.snapshot.walk_outcome_failed) {
                fail(api, 'ordinary walk after inspect failed');
                return;
            }
            if (chebyshev(api.snapshot.here, BANK) > 4) {
                return;
            }
            phase = 'done';
            api.log(`route_inspect_brimhaven_v2: ${STOP_OK}`);
            api.stop(STOP_OK);
            return;
        }
        default:
            fail(api, `unknown phase ${phase}`);
    }
}
