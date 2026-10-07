import Tile from './Tile.js';

// Keep the native representation opaque; all Area behavior runs in Rust.
const nativeAreas = new WeakMap();

function fromNativeArea(nativeArea) {
    const area = new Area();
    nativeAreas.set(area, nativeArea);
    return area;
}

export class Area {
    static rectangular(a, b) {
        return fromNativeArea(globalThis.__rs2b0t_area(0, a, b));
    }

    static circular(center, radius) {
        return fromNativeArea(globalThis.__rs2b0t_area(1, center, radius));
    }

    contains(tile) {
        return globalThis.__rs2b0t_area(2, nativeAreas.get(this), tile);
    }

    getRandomTile() {
        return Tile.from(globalThis.__rs2b0t_area(3, nativeAreas.get(this)));
    }
}
