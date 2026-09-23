import type { NativeApi } from '../host-js/index.d.ts';

/**
 * Sherlock v2 thin front: a host File card over the landed isolate clue
 * machine. `api.clue.begin` owns the session and `api.clue.next` advances it
 * one step; the v2 wrapper already enqueues every machine verb (walk, held,
 * loc, npc, if-button, close-modal, obj, puzzle-move) onto the interact
 * drain, so this card requests nothing of its own and re-derives nothing:
 * membership, coordinates, answers and the trail's own bank stop are the
 * machine's and later cards'.
 */
export const apiVersion = 2;

/**
 * Operator bag: the frozen ClueSolver ids, schema only. `clue.begin` ignores
 * every key and captures nothing, and this slice does not honor the bag — no
 * loadout provision, food restock, prayer top-up or teleport routing is
 * wired onto the machine's own steps yet.
 */
export const SETTINGS = {
    loadout: {
        type: 'string',
        default: '',
        label: 'Loadout',
        optionsFrom: 'loadouts',
    },
    foodWithdraw: {
        type: 'number',
        default: 8,
        min: 1,
        max: 27,
        label: 'Food to withdraw',
    },
    restorePrayer: {
        type: 'boolean',
        default: true,
        label: 'Top up prayer between trails',
    },
    useTeleports: {
        type: 'boolean',
        default: true,
        label: 'Use teleports',
    },
};

/** The landed idle: no held membership, so a later pickup needs a new begin. */
const WAITING = 'waiting for clue';

let token: number | null = null;
/** A `callback.enabled` was read; the next step carries the boolean answer. */
let enabled = false;
let status = WAITING;
let step = 'idle';
let solved = 0;
let noted = '';

function note(api: NativeApi, line: string): void {
    if (noted === line) return;
    noted = line;
    api.log(`sherlock v2: ${line}`);
}

function paint(api: NativeApi): void {
    const board = api.snapshot.puzzle_board;
    const label = board && board.component_id >= 0 ? `${board.component_id}/${board.size}` : 'closed';
    const pieces = board && Array.isArray(board.items) ? board.items.length : 0;
    api.paint.begin({ accent: '#8a7bff' })
        .title('sherlock v2')
        .row('status', status)
        .row('step', step)
        .row('solved', solved)
        .row('token', token === null ? 'none' : token)
        .row('board', label, 'pieces', pieces, 'gen', api.snapshot.puzzle_board_generation)
        .end();
}

function start(api: NativeApi): void {
    const began = api.clue.begin();
    if (began.ok) {
        token = began.value.token;
        enabled = false;
        status = WAITING;
        step = 'begin';
        note(api, 'clue session open');
        return;
    }
    token = null;
    enabled = false;
    step = `begin: ${began.error}`;
    // A refused begin leaves no live token. `none-held` is the landed idle,
    // not an error: the front waits for a pickup and begins again.
    if (began.error === 'none-held') {
        status = WAITING;
        note(api, WAITING);
        return;
    }
    note(api, `begin refused: ${began.error}`);
}

function end(api: NativeApi, reason: string): void {
    // The machine reports no completion. Its own `none-held` is the
    // trail-over reading — the collect finishes with it, and a row that left
    // the pack ends with it — so it is the local count this card paints, and
    // nothing here claims more than a session that ended.
    if (reason === 'none-held') {
        solved += 1;
        note(api, WAITING);
    } else {
        note(api, `session ended: ${reason}`);
    }
    token = null;
    enabled = false;
    status = WAITING;
    step = `end: ${reason}`;
}

function advance(api: NativeApi): void {
    const live = token;
    if (live === null) return;
    const answered = enabled;
    const read = answered
        ? api.clue.next({ token: live, resume: true })
        : api.clue.next({ token: live });
    if (!read.ok) {
        end(api, read.error);
        return;
    }
    // A frozen (`wait`) or held (`yield`) call burns nothing, so the boolean
    // answer stays armed until the machine acts on it.
    if (answered && read.kind !== 'wait' && read.kind !== 'yield') enabled = false;
    step = read.kind;
    noted = '';
    switch (read.kind) {
        case 'callback.enabled':
            // Start means enabled, so the answer is always the boolean true.
            enabled = true;
            return;
        case 'callback.log':
            api.log(read.message);
            return;
        case 'callback.setStatus':
            status = read.message;
            return;
        default:
            // walk / held / loc / npc / if-button / close-modal / obj /
            // puzzle-move are already enqueued by the wrapper, and wait /
            // yield idle. The enqueued step is not re-sent as a request.
            return;
    }
}

export function tick(api: NativeApi): void {
    const snapshot = api.snapshot;
    if (!snapshot.ingame) {
        // A logout or reset bumps the wrapper generation, so no session
        // survives one: this card forgets its token rather than reviving it.
        token = null;
        enabled = false;
        status = WAITING;
        step = 'out of game';
        paint(api);
        return;
    }
    if (token === null) {
        start(api);
    } else {
        advance(api);
    }
    paint(api);
}
