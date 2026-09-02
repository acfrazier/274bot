const MAX_LEVEL = 99;

const XP_AT_LEVEL = (() => {
    const out = [0, 0];
    let points = 0;
    for (let level = 1; level < MAX_LEVEL; level++) {
        points += Math.floor(level + 300 * Math.pow(2, level / 7));
        out[level + 1] = Math.floor(points / 4);
    }
    return out;
})();

export function xpAtLevel(level) {
    return XP_AT_LEVEL[Math.min(MAX_LEVEL, Math.max(1, Math.floor(level)))] ?? 0;
}

export function levelProgress(level, xp) {
    const cur = Math.min(MAX_LEVEL, Math.max(1, Math.floor(level)));
    const next = Math.min(MAX_LEVEL, cur + 1);
    const base = xpAtLevel(cur);
    const top = xpAtLevel(next);
    const span = Math.max(1, top - base);
    const fraction = cur >= MAX_LEVEL ? 1 : Math.max(0, Math.min(1, (xp - base) / span));
    return {
        level: cur,
        fraction,
        remaining: cur >= MAX_LEVEL ? 0 : Math.max(0, top - xp),
    };
}

export function etaHours(level, xp, xpPerHour) {
    if (xpPerHour <= 0) {
        return null;
    }
    const prog = levelProgress(level, xp);
    const hours = prog.remaining / xpPerHour;
    return Number.isFinite(hours) ? hours : null;
}
