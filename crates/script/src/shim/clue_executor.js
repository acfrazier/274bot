// Teleport policy is posted with each Rust-owned clue machine run; the catalog's
// global setter is retained as a thin setting seat for source compatibility.
let teleports = true;

export class ClueExecutor {
    static setTeleports(enabled) {
        teleports = enabled === true;
    }

    static teleportsEnabled() {
        return teleports;
    }
}
