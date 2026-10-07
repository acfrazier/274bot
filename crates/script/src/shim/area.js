import Tile from './Tile.js';

// Keep the native representation opaque; all Area behavior runs in Rust.
const nativeAreas = new WeakMap();

function fromNativeArea(nativeArea, center) {
    const area = new Area();
    nativeAreas.set(area, { nativeArea, center });
    return area;
}

export class Area {
    static rectangular(a, b) {
        return fromNativeArea(globalThis.__rs2b0t_area(0, a, b));
    }

    static circular(center, radius) {
        return fromNativeArea(globalThis.__rs2b0t_area(1, center, radius), center);
    }

    contains(tile) {
        const area = nativeAreas.get(this);
        return globalThis.__rs2b0t_area(2, area?.nativeArea, tile, area?.center);
    }

    getRandomTile() {
        const area = nativeAreas.get(this);
        return Tile.from(globalThis.__rs2b0t_area(3, area?.nativeArea, area?.center));
    }
}
