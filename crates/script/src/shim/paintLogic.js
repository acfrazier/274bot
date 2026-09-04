// paintLogic helpers the listed scripts import from
// `../../paint/paintLogic.js`. Only the shapes the live catalog uses are
// here; a missing named export fails the module load honestly.
export function fmtDuration(minutes) {
    if (!(minutes >= 0)) {
        return '0s';
    }
    if (minutes < 1) {
        return Math.round(minutes * 60) + 's';
    }
    const h = Math.floor(minutes / 60);
    const m = Math.round(minutes % 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

export function fmtXpHr(gained, mins) {
    if (!(mins > 0) || !(gained >= 0)) {
        return '0';
    }
    return String(Math.round((Number(gained) / Number(mins)) * 60));
}

export function paintSkillShort(skill) {
    switch (String(skill)) {
        case 'woodcutting':
            return 'WC';
        case 'firemaking':
            return 'FM';
        case 'fishing':
            return 'Fish';
        case 'cooking':
            return 'Cook';
        case 'mining':
            return 'Mine';
        default:
            return String(skill);
    }
}
