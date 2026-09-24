// Dragon sites: a name map over the host's site table (`crate::hunt_catalog`),
// shaped into the frozen `DragonSite` rows (Tile instances, RegExp lines,
// an `inArea` over the table's boxes).
import Tile from '../../../geometry/Tile.js';

const logic = (name, ...args) => globalThis.__rs2b0t_hunt_logic(name, ...args);

export const inBox = (b) => (t) =>
    t != null && t.level === b.level && t.x >= b.minX && t.x <= b.maxX && t.z >= b.minZ && t.z <= b.maxZ;

const tile = (t) => (t ? new Tile(t.x, t.z, t.level ?? 0) : t);
const tiles = (rows) => (rows || []).map(tile);
const stand = (s) => ({ label: s.label, tiles: tiles(s.tiles), anchor: tile(s.anchor) });

function site(row) {
    const area = (row.boxes || []).map(inBox);
    const out = {
        ...row,
        approach: tiles(row.approach),
        safespots: tiles(row.safespots),
        meleeAnchor: tile(row.meleeAnchor),
        bank: tile(row.bank),
        walkOut: tile(row.walkOut),
        inArea: (t) => area.some((inside) => inside(t)),
    };
    delete out.boxes;
    if (row.gate) out.gate = { ...row.gate, outside: tile(row.gate.outside), inside: tile(row.gate.inside) };
    if (row.talkGate) out.talkGate = { ...row.talkGate, stand: tile(row.talkGate.stand) };
    if (row.exit) out.exit = { ...row.exit, stand: tile(row.exit.stand) };
    if (row.feeGate) {
        out.feeGate = {
            ...row.feeGate,
            stand: tile(row.feeGate.stand),
            paidLine: new RegExp(row.feeGate.paidLine, 'i'),
            prepaidLine: new RegExp(row.feeGate.prepaidLine, 'i'),
        };
    }
    if (row.stands) out.stands = row.stands.map(stand);
    return out;
}

const ROWS = logic('sites').map(site);

export const DRAGON_SITES = Object.fromEntries(ROWS.map((s) => [s.key, s]));
export const TAVERLEY_BLUE = DRAGON_SITES['taverley-blue'];
export const TAVERLEY_BLACK = DRAGON_SITES['taverley-black'];
export const HEROES_BLUE = DRAGON_SITES['heroes-blue'];
export const GUTANOTH_BLUE = DRAGON_SITES['gutanoth-blue'];
export const BRIMHAVEN_IRON = DRAGON_SITES['brimhaven-iron'];
export const BRIMHAVEN_STEEL = DRAGON_SITES['brimhaven-steel'];
export const STAND_SITE_KEYS = ROWS.filter((s) => (s.stands?.length ?? 0) > 1).map((s) => s.key);
export const MAX_STANDS = Math.max(...ROWS.map((s) => s.stands?.length ?? 1));
export const SITE_OPTIONS = ROWS.map((s) => s.key);

export function needsShield(s, style) {
    return style === 'melee' || s.fireAtRange === true;
}

export function huntNames(s) {
    return [s.target, ...(s.alsoHunt ?? [])];
}

export function standFor(s, n) {
    const stands = s.stands;
    if (!stands || stands.length === 0) {
        return { label: s.label, tiles: [...s.safespots], anchor: s.meleeAnchor };
    }
    return stands[Math.min(Math.max(1, Math.trunc(n) || 1), stands.length) - 1];
}

export function siteFor(key) {
    return DRAGON_SITES[key] ?? TAVERLEY_BLUE;
}
