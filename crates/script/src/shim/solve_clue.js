// SolveClue Task: marshal only. The isolate clue machine (`__rs2b0t_clue`
// `begin` / `next`) owns the token, the identify, the phases and the clocks;
// this file coerces the caller's host, marshals the call-time pages onto the
// machine payload and dispatches the kinds the machine posts. No sequencing
// policy lives here: no death latch, no kit / bank-prep / restore, and no
// solved-clue text this file invents — the completion kinds are the machine's
// and `'clue solved'` is its own `callback.setStatus` message, forwarded
// untouched.
import { Execution } from '../../execution/Execution.js';
import { host, notImpl, queue } from '../../../shim/_kernel.js';

const throwUse = (name) => {
    throw notImpl(name);
};

// The machine call. `begin` / `next` answer the raw envelope
// `{ kind, token, reason? }` — never a helper result. A missing helper is no
// step at all: the caller refuses the begin / ends the session instead of
// re-asking it.
function clueCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_clue
        : undefined;
    if (typeof fn !== 'function') return null;
    return fn(payload);
}

// The page is the already-posted `snapshot.inv` `(id, count)` sequence and
// nothing else: a missing or empty page is an empty held list, never a second
// snapshot error token, and a row that is not an i32 pair cannot be held. The
// pair is marshalled as `[id, count]`.
function cluePageI32(value) {
    return typeof value === 'number' && Number.isInteger(value)
        && value >= -2147483648 && value <= 2147483647;
}

function clueHeldPage() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
    const page = snapshot.inv;
    if (!Array.isArray(page)) return [];
    const held = [];
    for (const row of page) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
        if (!cluePageI32(row.id) || !cluePageI32(row.count)) continue;
        held.push([row.id, row.count]);
    }
    return held;
}

// The posted cooperative interrupt, re-read every call: `EventSignal.pending()`
// reads the same `hold || ours` pair. Nothing about it is captured at begin.
function cluePending() {
    const h = host();
    return h.hold === true || h.ours === true;
}

// The posted player tile, read at call time like the pack page. A missing,
// null, or malformed `here` is not sent, and the machine then has no arrival
// claim to make.
function clueHereTile() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    const here = snapshot.here;
    if (!here || typeof here !== 'object' || Array.isArray(here)) return null;
    if (!cluePageI32(here.x) || !cluePageI32(here.z) || !cluePageI32(here.level)) return null;
    return { x: here.x, z: here.z, level: here.level };
}

// One posted scene page (`locs` / `ground`), the same call-time class of page.
// A row that is not the posted `(id, x, z, level, actions)` shape cannot be
// picked and is dropped here, never a snapshot error. `name` rides along only
// for the page whose verb resolves identity by name, and it is never invented.
function clueScenePage(key, withName) {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
    const page = snapshot[key];
    if (!Array.isArray(page)) return [];
    const rows = [];
    for (const row of page) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
        if (!cluePageI32(row.id) || !cluePageI32(row.x) || !cluePageI32(row.z)
            || !cluePageI32(row.level)) continue;
        if (!Array.isArray(row.actions)) continue;
        const actions = [];
        let ok = true;
        for (const action of row.actions) {
            if (typeof action !== 'string') { ok = false; break; }
            actions.push(action);
        }
        if (!ok) continue;
        const out = { id: row.id, x: row.x, z: row.z, level: row.level, actions: actions };
        if (withName) out.name = typeof row.name === 'string' ? row.name : null;
        rows.push(out);
    }
    return rows;
}

function clueLocPage() {
    return clueScenePage('locs', false);
}

// The posted ground page the collect arm Takes from: the loc shape plus the
// display name the host resolves.
function clueGroundPage() {
    return clueScenePage('ground', true);
}

// The posted pack page the collect arm reads: the display name the Drop
// resolves and the positive count that occupies a slot. This is not a second
// inventory read and not a change to `clueHeldPage` — the identify page stays
// the `(id, count)` pair, and both come from the one posted `snapshot.inv`.
function clueInvPage() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
    const page = snapshot.inv;
    if (!Array.isArray(page)) return [];
    const rows = [];
    for (const row of page) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
        if (!cluePageI32(row.id) || !cluePageI32(row.count)) continue;
        rows.push({
            id: row.id,
            name: typeof row.name === 'string' ? row.name : null,
            count: row.count,
        });
    }
    return rows;
}

// The posted main modal id, and only when the page posted it: an omitted slot
// is not the closed `-1` and not a second definition of it, so the machine is
// handed no `main_modal_id` at all.
function clueMainModalId() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    return cluePageI32(snapshot.main_modal_id) ? snapshot.main_modal_id : null;
}

// The posted chat slots the talk arm reads: the open chat modal id — `-1` is
// the closed one the page posts itself — and the posted `chat_continue`. Both
// are posted only when the page carried them, so an unobserved slot is not an
// open chat and not a close.
function clueChatModalId() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    return cluePageI32(snapshot.chat_modal_id) ? snapshot.chat_modal_id : null;
}

function clueChatContinue() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    return typeof snapshot.chat_continue === 'boolean' ? snapshot.chat_continue : null;
}

// The posted count dialog, and only when the page posted the boolean: the talk
// arm answers a count only behind the posted open fact, and an omitted slot is
// unobserved rather than closed.
function clueCountDialogOpen() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    return typeof snapshot.count_dialog_open === 'boolean' ? snapshot.count_dialog_open : null;
}

// The posted inv tab slot count. A page that did not post it hands the machine
// nothing: 28 is the client default, not this adapter's to invent.
function clueInvSize() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    return cluePageI32(snapshot.inv_size) ? snapshot.inv_size : null;
}

// The posted npc page the guarded encounter observes after its spawn. A row
// that did not post an index cannot be Attacked and is dropped here; every
// other absent field is posted as null and matches nothing.
function clueNpcPage() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return [];
    const page = snapshot.npcs;
    if (!Array.isArray(page)) return [];
    const rows = [];
    for (const row of page) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
        if (!cluePageI32(row.index)) continue;
        rows.push({
            index: row.index,
            id: cluePageI32(row.id) ? row.id : null,
            name: typeof row.name === 'string' ? row.name : null,
            x: cluePageI32(row.x) ? row.x : null,
            z: cluePageI32(row.z) ? row.z : null,
            level: cluePageI32(row.level) ? row.level : null,
            distance: cluePageI32(row.distance) ? row.distance : null,
            health: cluePageI32(row.health) ? row.health : null,
            max_health: cluePageI32(row.max_health) ? row.max_health : null,
            in_combat: typeof row.in_combat === 'boolean' ? row.in_combat : null,
            actions: Array.isArray(row.actions)
                ? row.actions.filter((action) => typeof action === 'string')
                : [],
            target_kind: cluePageI32(row.target_kind) ? row.target_kind : null,
            target_index: cluePageI32(row.target_index) ? row.target_index : null,
        });
    }
    return rows;
}

// The posted local-player table slot: the encounter's `targetsMe` read.
// Posted only when the page carried it, so an absent slot is never read as
// zero.
function clueSelfSlot() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    return cluePageI32(snapshot.self_slot) ? snapshot.self_slot : null;
}

// The local player's own posted target pair — `target_kind` `1` is an npc —
// which the health-0 kill read compares with the wizard this token owns. Both
// halves are needed, so a page that posted only one posts no target at all.
function clueSelfTarget() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    if (!cluePageI32(snapshot.self_target_kind) || !cluePageI32(snapshot.self_target_index)) {
        return null;
    }
    return { kind: snapshot.self_target_kind, index: snapshot.self_target_index };
}

// The posted Protect from Magic overlay: the selected prayer row's own varp,
// index 95 on both pins. Posted only when the varp page carried it, so the
// machine reads a missing overlay as unobserved rather than as off.
function clueVarp95() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    const page = snapshot.varps;
    if (!Array.isArray(page)) return null;
    const row = page.find((v) => v && typeof v === 'object' && v.index === 95);
    return row && cluePageI32(row.value) ? row.value : null;
}

// The posted effective hitpoints of the local player, off the posted
// `snapshot.stats` page. A page that did not post the row is not a zero.
function clueHitpoints() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    const page = snapshot.stats;
    if (!Array.isArray(page)) return null;
    const row = page.find((s) => s && typeof s === 'object' && s.name === 'hitpoints');
    return row && cluePageI32(row.effective) ? row.effective : null;
}

// The posted puzzle board the plan reads: the identified component, its
// observed slot count and its sparse rows. A row that did not post an i32 slot
// and id is dropped here; the machine's own read rejects a board that is not
// 24 pieces around one gap.
function cluePuzzleBoard() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    const board = snapshot.puzzle_board;
    if (!board || typeof board !== 'object' || Array.isArray(board)) return null;
    if (!cluePageI32(board.component_id) || !cluePageI32(board.size)) return null;
    if (!Array.isArray(board.items)) return null;
    const items = [];
    for (const row of board.items) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
        if (!cluePageI32(row.slot) || !cluePageI32(row.id)) continue;
        items.push({ slot: row.slot, id: row.id });
    }
    return { component_id: board.component_id, size: board.size, items: items };
}

// The posted board session generation: a non-negative safe integer is the
// whole of it a double can carry. A page that did not post one hands the
// machine nothing, and no click is sent on an invented session.
function cluePuzzleGeneration() {
    const snapshot = host().snapshot;
    if (!snapshot || typeof snapshot !== 'object' || Array.isArray(snapshot)) return null;
    const generation = snapshot.puzzle_board_generation;
    return typeof generation === 'number' && Number.isSafeInteger(generation) && generation >= 0
        ? generation
        : null;
}

// The machine's own walk, held, npc and if-button steps go onto the shared
// interact drain the way the journal enqueues `if-button` / `close-modal` and
// the landed npc consumers enqueue `npc`: the landed field names, one explicit
// arm per kind, and `loc` last. An unknown kind is never enqueued as a loc.
function enqueueClueVerb(step) {
    if (step.kind === 'walk') {
        queue({ op: 'walk', x: step.x, z: step.z, level: step.level });
        return true;
    }
    if (step.kind === 'if-button') {
        queue({ op: 'if-button', component_id: step.component_id });
        return true;
    }
    if (step.kind === 'npc') {
        // The posted identity the Attack rides: the posted name, the action
        // the machine dispatched and the posted scene index.
        queue({ op: 'npc', name: step.name, action: step.action, index: step.index });
        return true;
    }
    if (step.kind === 'answer-count') {
        // The open count dialog the talk arm's challenge step answers: the
        // selected answer the machine parsed, and nothing else. Its own arm,
        // never the loc fall-through.
        queue({ op: 'answer-count', value: step.value });
        return true;
    }
    if (step.kind === 'close-modal') {
        queue({ op: 'close-modal' });
        return true;
    }
    if (step.kind === 'obj') {
        // The casket overflow on the posted tile, by the posted name and
        // action. The host matches that identity and refuses a stale row.
        queue({
            op: 'obj',
            x: step.x,
            z: step.z,
            level: step.level,
            name: step.name,
            action: step.action,
        });
        return true;
    }
    if (step.kind === 'puzzle-move') {
        // The exact posted board row: its own id, the slot it sits in, the
        // posted component and this call's board generation.
        queue({
            op: 'puzzle-move',
            id: step.id,
            slot: step.slot,
            component: step.component,
            generation: step.generation,
        });
        return true;
    }
    if (step.kind === 'held') {
        // The selected item display name; the host resolves the first
        // inventory row with that name. No row id and no tile rides along.
        queue({ op: 'held', name: step.name, action: step.action });
        return true;
    }
    if (step.kind === 'loc') {
        queue({
            op: 'loc',
            x: step.x,
            z: step.z,
            level: step.level,
            action: step.action,
            id: step.id,
        });
        return true;
    }
    return false;
}

// The kinds the machine enqueues. Anything else is a callback, a wait, a
// yield, the `grind-ready` continue, the `supplies-needed` wait-class or a
// terminal — and an unknown kind is none of them.
const ENQUEUED_KINDS = [
    'walk', 'held', 'loc', 'npc', 'answer-count', 'if-button', 'close-modal', 'obj',
    'puzzle-move',
];

// The call-time pages every `next` posts, in the machine's own field names.
// The required page keys are always present (`null`/`[]` when the page posted
// nothing); a slot the page did not carry is omitted rather than defaulted.
function clueNextPayload(token, resume) {
    const payload = {
        op: 'next',
        token: token,
        generation: 0,
        held: clueHeldPage(),
        hold: cluePending(),
        locs: clueLocPage(),
        ground: clueGroundPage(),
        inv: clueInvPage(),
        npcs: clueNpcPage(),
    };
    const here = clueHereTile();
    if (here !== null) payload.here = here;
    const main = clueMainModalId();
    if (main !== null) payload.main_modal_id = main;
    // The talk arm's own call-time facts: posted-only, so an omitted slot stays
    // unobserved on the machine rather than becoming a closed chat or a closed
    // count dialog.
    const chatModal = clueChatModalId();
    if (chatModal !== null) payload.chat_modal_id = chatModal;
    const chatContinue = clueChatContinue();
    if (chatContinue !== null) payload.chat_continue = chatContinue;
    const countOpen = clueCountDialogOpen();
    if (countOpen !== null) payload.count_dialog_open = countOpen;
    const invSize = clueInvSize();
    if (invSize !== null) payload.inv_size = invSize;
    const selfSlot = clueSelfSlot();
    if (selfSlot !== null) payload.self_slot = selfSlot;
    const selfTarget = clueSelfTarget();
    if (selfTarget !== null) {
        payload.self_target_kind = selfTarget.kind;
        payload.self_target_index = selfTarget.index;
    }
    const hitpoints = clueHitpoints();
    if (hitpoints !== null) payload.hitpoints = hitpoints;
    const varp95 = clueVarp95();
    if (varp95 !== null) payload.varp95 = varp95;
    const puzzleBoard = cluePuzzleBoard();
    if (puzzleBoard !== null) payload.puzzle_board = puzzleBoard;
    const puzzleGeneration = cluePuzzleGeneration();
    if (puzzleGeneration !== null) payload.puzzle_board_generation = puzzleGeneration;
    // `resume` is the only answer slot, and it is only ever a boolean: the
    // key is omitted on every call that is not answering `callback.enabled`.
    if (typeof resume === 'boolean') payload.resume = resume;
    return payload;
}

export class SolveClue {
    constructor(hostArg) {
        // Coerce and store only: the seven embeds construct this in onStart,
        // where a begin with no held membership would be refused anyway. The
        // constructor never throws and never begins.
        this.host = hostArg && typeof hostArg === 'object' ? hostArg : {};
        this.token = null;
        this.status = 'idle';
    }

    clueStatus() {
        // The machine's own `callback.setStatus` line, and only that: 'idle'
        // until one is dispatched.
        return this.status;
    }

    noteDeath() {
        // Callbacks stay script owned; the death latch is not this file's.
    }

    ownsEquipment() {
        // The strip / hard-trail banked-this-solve flag is not this file's.
        return false;
    }

    // The embed's own gate, coerced the way the callers coerce it: absent is
    // enabled, and a false answer only means "do not run this session".
    enabled() {
        const fn = this.host.enabled;
        const value = typeof fn === 'function' ? fn() : undefined;
        return value === undefined || value === null ? true : !!value;
    }

    validate() {
        // Disabled first: it must not begin, and it must not abort a live
        // token either — the same session resumes when the flag comes back.
        if (!this.enabled()) return false;
        // A live token is the machine's to keep: a second begin aborts it.
        if (this.token !== null) return true;
        const begin = clueCall({ op: 'begin', generation: 0, held: clueHeldPage() });
        if (!begin || begin.kind !== 'token') return false;
        this.token = begin.token;
        return true;
    }

    async execute() {
        if (this.token === null) {
            // Only reached when a caller runs execute without validate: the
            // one begin is the same begin validate makes.
            const begin = clueCall({ op: 'begin', generation: 0, held: clueHeldPage() });
            if (!begin || begin.kind !== 'token') return;
            this.token = begin.token;
        }
        let resume = undefined;
        for (;;) {
            const step = clueCall(clueNextPayload(this.token, resume));
            resume = undefined;
            if (!step || typeof step !== 'object') {
                // No envelope is no session: end it rather than re-asking.
                this.token = null;
                return;
            }
            if (step.kind === 'aborted' || step.kind === 'done' || step.kind === 'dead'
                || step.kind === 'abandon' || step.kind === 'guardian-lost') {
                // The session's terminals. `aborted` is the identify family's
                // own refusals (no held membership, no selected pin, no trail
                // family), a token that is not the live one, the generation
                // bump or the constrained row; `done` is the finished collect,
                // `dead` the posted hitpoints at or below zero and
                // `guardian-lost` the owned wizard that left outside the
                // grace. All of them kill the token, and nothing is emitted
                // for it. The exact `'clue solved'` status is the machine's
                // own `callback.setStatus` above — this file never invents it.
                this.token = null;
                return;
            }
            if (step.kind === 'yield') {
                // The cooperative interrupt. The token stays live and the
                // session resumes on a later tick.
                return;
            }
            if (step.kind === 'grind-ready') {
                // The collect's own continue: the token is live, no verb rides
                // it and the next call is `done`. No enqueue and no delay.
                continue;
            }
            if (step.kind === 'supplies-needed') {
                // An arrived dig with no `Spade` on the posted pack page: a
                // wait-class and not a terminal, so the token stays live and
                // nothing is fetched.
                await Execution.delayTicks(1);
                continue;
            }
            if (step.kind === 'wait') {
                // The same snapshot must not busy-spin a parked loop.
                await Execution.delayTicks(1);
                continue;
            }
            if (step.kind === 'callback.enabled') {
                // The callback's return is the whole answer, and it is only
                // ever sent as a boolean.
                resume = this.enabled();
                continue;
            }
            if (step.kind === 'callback.log') {
                this.host.log?.(String(step.message));
                continue;
            }
            if (step.kind === 'callback.setStatus') {
                const message = String(step.message);
                this.status = message;
                this.host.setStatus?.(message);
                continue;
            }
            if (ENQUEUED_KINDS.indexOf(step.kind) !== -1) {
                enqueueClueVerb(step);
                // One enqueue per tick: the next call gets a fresh observation
                // instead of replaying this step's own snapshot.
                await Execution.delayTicks(1);
                continue;
            }
            // An unknown kind is not a loc and not a verb: leave it alone.
            return;
        }
    }
}

export function heldClueLikeId() {
    throwUse('SolveClue.heldClueLikeId');
}

export function walkToBank() {
    throwUse('SolveClue.walkToBank');
}
