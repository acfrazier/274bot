import { reader } from '../../adapter/ClientAdapter.js';
import { liveCatalog } from '../../api/market/catalog.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { Paint } from '../../paint/Paint.js';

const RECEIPT = 'w6-compat-object-cache';

function isPlainRecord(value) {
    return value !== null && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype;
}

function isObjectRecord(value) {
    return (
        isPlainRecord(value) &&
        Number.isInteger(value.id) &&
        typeof value.name === 'string' &&
        typeof value.cost === 'number' &&
        typeof value.stackable === 'boolean' &&
        typeof value.members === 'boolean' &&
        typeof value.equippable === 'boolean' &&
        Number.isInteger(value.certlink) &&
        Number.isInteger(value.certtemplate)
    );
}

function isAliasRecord(value) {
    return isPlainRecord(value) && typeof value.label === 'string' && Array.isArray(value.words);
}

export default class CompatibilityObjectCache extends LoopingBot {
    constructor() {
        super();
        this.mode = 'off';
        this.step = 0;
        this.home = null;
        this.retained = null;
        this.cacheReads = 0;
        this.reuses = 0;
        this.walks = 0;
        this.rowIdentity = false;
        this.aliasIdentity = false;
        this.recordIdentity = false;
        this.plainShape = false;
        this.failed = false;
    }

    fail() {
        this.failed = true;
    }

    readObjects() {
        const records = reader.objCatalog();
        const catalog = liveCatalog();
        const items = catalog.items;
        const byId = catalog.byId;
        const aliases = catalog.aliases;
        if (
            !Array.isArray(records) ||
            records.length === 0 ||
            !Array.isArray(items) ||
            items.length === 0 ||
            !(byId instanceof Map) ||
            byId.size === 0 ||
            !(aliases instanceof Map) ||
            aliases.size === 0
        ) {
            this.fail();
            return;
        }

        if (this.retained === null) {
            const itemIndex = 0;
            const item = items[itemIndex];
            if (!isObjectRecord(item) || byId.get(item.id) !== item) {
                this.fail();
                return;
            }
            let recordIndex = -1;
            for (let i = 0; i < records.length; i++) {
                if (records[i]?.id === item.id) {
                    recordIndex = i;
                    break;
                }
            }
            if (recordIndex < 0 || !isObjectRecord(records[recordIndex])) {
                this.fail();
                return;
            }
            const aliasId = aliases.keys().next().value;
            const alias = aliases.get(aliasId);
            if (!Number.isInteger(aliasId) || !isAliasRecord(alias)) {
                this.fail();
                return;
            }
            this.retained = {
                records,
                catalog,
                items,
                byId,
                aliases,
                itemIndex,
                item,
                recordIndex,
                record: records[recordIndex],
                aliasId,
                alias,
                recordId: records[recordIndex].id,
                itemId: item.id,
                rowCount: records.length + items.length + byId.size + aliases.size,
            };
            this.rowIdentity = true;
            this.aliasIdentity = true;
            this.recordIdentity = true;
            this.plainShape = true;
        } else {
            const kept = this.retained;
            if (
                records !== kept.records ||
                catalog !== kept.catalog ||
                items !== kept.items ||
                byId !== kept.byId ||
                aliases !== kept.aliases ||
                records[kept.recordIndex] !== kept.record ||
                items[kept.itemIndex] !== kept.item ||
                byId.get(kept.item.id) !== kept.item ||
                aliases.get(kept.aliasId) !== kept.alias
            ) {
                this.fail();
                return;
            }
            this.rowIdentity = true;
            this.aliasIdentity = true;
            this.recordIdentity = true;
            this.plainShape =
                isObjectRecord(kept.record) &&
                isObjectRecord(kept.item) &&
                isAliasRecord(kept.alias);
            if (!this.plainShape) {
                this.fail();
                return;
            }
            this.reuses++;
        }
        this.cacheReads++;
    }

    async loop() {
        if (this.failed) return;
        this.mode = this.settings.str('mode', 'off');
        if (this.mode !== 'active' && this.mode !== 'off') {
            this.fail();
            return;
        }

        const here = reader.worldTile();
        if (!reader.ingame() || !here) {
            this.fail();
            return;
        }
        if (this.home === null) {
            this.home = { x: here.x, z: here.z, level: here.level };
        }
        if (this.mode === 'active') {
            this.readObjects();
            if (this.failed) return;
        }

        const target = (this.step++ & 1) === 0
            ? { x: this.home.x + 1, z: this.home.z, level: this.home.level }
            : this.home;
        const arrived = await Traversal.walkTo(target, { radius: 0, timeoutMs: 300000 });
        const landed = reader.worldTile();
        if (
            arrived !== true ||
            !landed ||
            landed.x !== target.x ||
            landed.z !== target.z ||
            landed.level !== target.level
        ) {
            this.fail();
            return;
        }
        this.walks++;
    }

    onPaint() {
        const host = globalThis.__rs2b0t_host;
        const tick = Number.isSafeInteger(host?.tick) ? host.tick : 0;
        const tile = reader.worldTile();
        const kept = this.retained;
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.title('W6 compatibility object cache');
        p.text([
            RECEIPT,
            this.mode,
            tick,
            tile ? tile.x : 0,
            tile ? tile.z : 0,
            tile ? tile.level : 0,
            this.cacheReads,
            this.reuses,
            this.walks,
            kept ? kept.records.length : 0,
            kept ? kept.items.length : 0,
            kept ? kept.byId.size : 0,
            kept ? kept.aliases.size : 0,
            kept ? kept.recordId : 0,
            kept ? kept.itemId : 0,
            kept ? kept.aliasId : 0,
            kept ? kept.rowCount : 0,
            this.recordIdentity ? 1 : 0,
            this.rowIdentity ? 1 : 0,
            this.aliasIdentity ? 1 : 0,
            this.plainShape ? 1 : 0,
            this.failed ? 1 : 0,
        ].join('|'));
        p.end();
    }
}
