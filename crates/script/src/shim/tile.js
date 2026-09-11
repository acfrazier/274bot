// World tile primitive (Chebyshev distance). Thin shim over posted coords.
export default class Tile {
    constructor(x, z, level = 0) {
        this.x = x;
        this.z = z;
        this.level = level;
    }

    static from(tile) {
        return new Tile(tile.x, tile.z, tile.level ?? 0);
    }

    distanceTo(other) {
        return globalThis.rustyscript.functions.__rs2b0t_tile_distance(this, other);
    }

    translate(dx, dz) {
        return new Tile(this.x + dx, this.z + dz, this.level);
    }

    equals(other) {
        return this.x === other.x && this.z === other.z && this.level === other.level;
    }

    toString() {
        return `(${this.x}, ${this.z}, ${this.level})`;
    }
}

export function tileFromPosted(value) {
    if (
        !value ||
        typeof value !== 'object' ||
        !Number.isSafeInteger(value.x) || value.x < 0 || value.x > 16_383 ||
        !Number.isSafeInteger(value.z) || value.z < 0 || value.z > 16_383 ||
        (value.level !== undefined && (
            !Number.isSafeInteger(value.level) || value.level < 0 || value.level > 3
        ))
    ) {
        return null;
    }
    return value instanceof Tile ? value : Tile.from(value);
}

globalThis.__rs2b0t_tileFromPosted = tileFromPosted;
