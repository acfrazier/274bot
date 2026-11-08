import fs from 'node:fs';
import path from 'node:path';
import {
    parseJm2NpcPlacements,
    parseMapsquarePath,
    placementMapFiles,
    worldFromMapsquare,
} from './common.ts';

export function extractNpcPlacementsFacts(content: string) {
    const maps = placementMapFiles(content);
    const rows: {
        npc_id: number;
        x: number;
        z: number;
        plane: number;
        mapsquare: string;
    }[] = [];
    for (const map of maps) {
        const { mx, mz } = parseMapsquarePath(map);
        const text = fs.readFileSync(path.join(content, map), 'utf8');
        for (const placement of parseJm2NpcPlacements(text)) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            rows.push({
                npc_id: placement.npc_id,
                x: world.x,
                z: world.z,
                plane: world.plane,
                mapsquare: path.basename(map, '.jm2'),
            });
        }
    }
    if (rows.length === 0) throw new Error('npc_placements: no NPC rows in maps');
    return { rows };
}
