// World tile representation. Geometry and posted validation run in Rust.
export default class Tile {
    constructor(x, z, level = 0) {
        this.x = x;
        this.z = z;
        this.level = level;
    }

    static from(tile) {
        return globalThis.__rs2b0t_tile(0, Tile, tile);
    }

    distanceTo(other) {
        return globalThis.__rs2b0t_distance(this, other);
    }

    translate(dx, dz) {
        return globalThis.__rs2b0t_tile(1, Tile, this, dx, dz);
    }

    equals(other) {
        return globalThis.__rs2b0t_tile(2, this, other);
    }

    toString() {
        return globalThis.__rs2b0t_tile(3, this);
    }
}

export function tileFromPosted(value) {
    return globalThis.__rs2b0t_tile(4, Tile, value);
}

globalThis.__rs2b0t_tileFromPosted = tileFromPosted;
