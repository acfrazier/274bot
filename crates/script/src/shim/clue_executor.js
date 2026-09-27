// Frozen `ClueExecutor.setTeleports`: the card's trail teleport choice is
// held by the Rust clue module, where the bank return walk reads it.
export class ClueExecutor {
    static setTeleports(enabled) {
        globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'setTeleports', on: !!enabled });
    }
}
