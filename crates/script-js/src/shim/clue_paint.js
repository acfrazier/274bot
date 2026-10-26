// Frozen cluePaint.ts: the live clue's own rows, or the caller's idle line
// when nothing is being solved. The clue machine publishes exactly what it
// knows (`__rs2b0t_clue_paint`, one crossing): its phase line and its last
// walk dest. No invented clue name, leg, attempt, target or distance, and
// never a silent no-op.
function cluePaintRows() {
    const fn = globalThis.rustyscript.functions.__rs2b0t_clue_paint;
    return typeof fn === 'function' ? fn() : [];
}

export function paintClueProgress(p, idle = 'no clue in progress') {
    const rows = cluePaintRows();
    if (!Array.isArray(rows) || rows.length === 0) {
        p.text(String(idle));
        return;
    }
    for (const row of rows) {
        if (row && typeof row.text === 'string') p.text(row.text);
    }
}
