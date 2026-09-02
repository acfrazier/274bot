export function openOp(actions) {
    return (actions || []).find((a) => /^open/i.test(String(a))) ?? null;
}

export function towardDest(from, here, toward) {
    const dx = toward.x - here.x;
    const dz = toward.z - here.z;
    const dot = dx * (from.x - here.x) + dz * (from.z - here.z);
    return dot > 0;
}
