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
        const xz = Math.max(Math.abs(this.x - other.x), Math.abs(this.z - other.z));
        if ((this.level ?? 0) !== (other.level ?? 0)) {
            return 1_000_000 + xz;
        }
        return xz;
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
