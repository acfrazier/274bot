export function stepOffCandidates(from, awayFrom) {
    return globalThis.__rs2b0t_step_off_candidates(
        Number(from.x),
        Number(from.z),
        Number(from.level ?? 0),
        Number(awayFrom.x),
        Number(awayFrom.z),
    );
}
