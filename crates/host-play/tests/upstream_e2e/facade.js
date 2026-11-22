import { Inventory as E2EInventory } from '#/bot/api/inventory/Inventory.js';
import { Bank as E2EBank } from '#/bot/api/bank/Bank.js';
import { Skills as E2ESkills } from '#/bot/api/skills/Skills.js';
import { Quests as E2EQuests } from '#/bot/api/ui/questlog/Quests.js';
import { Game as E2EGame } from '#/bot/api/game/Game.js';
import { Equipment as E2EEquipment } from '#/bot/api/equipment/Equipment.js';
import { Locs as E2ELocs } from '#/bot/api/locs/Locs.js';
import { Npcs as E2ENpcs } from '#/bot/api/npcs/Npcs.js';
import { Prayer as E2EPrayer } from '#/bot/api/prayer/Prayer.js';
import { Shop as E2EShop } from '#/bot/api/shop/Shop.js';
import { reader as E2EReader, actions as E2EActions } from '#/bot/adapter/ClientAdapter.js';

// Test-only aliases. Game facts and actions remain the existing native ABI.
globalThis.window = globalThis;
globalThis.__e2eCommands = [];
globalThis.__e2eStatus = { state: 'idle', logs: [] };
globalThis.__rs2b0t = {
    Inventory: E2EInventory, Bank: E2EBank, Skills: E2ESkills, Quests: E2EQuests,
    Game: E2EGame, Equipment: E2EEquipment, Locs: E2ELocs, Npcs: E2ENpcs,
    Prayer: E2EPrayer, Shop: E2EShop,
    host: { get tickCount() { return E2EGame.tick(); } },
    reader: E2EReader
};
globalThis.rs2b0t = {
    reader: E2EReader,
    actions: E2EActions,
    host: globalThis.__rs2b0t.host,
    registry: { get: name => ({ name }) },
    runner: {
        get state() { return globalThis.__e2eStatus.state; },
        get bot() {
            const state = globalThis.__e2eStatus.state;
            // Upstream retains the last bot. After stop, this adapter replaces it with E2EObserver.
            if (state === 'idle' || state === 'stopped') return null;
            return globalThis.__rs_bot ?? null;
        },
        get ctx() { return { log: globalThis.__e2eStatus.logs }; },
        start(meta) {
            if (!meta || typeof meta.name !== 'string') throw new Error('missing script metadata');
            globalThis.__e2eCommands.push({ op: 'start', name: meta.name });
        },
        stop(reason) { globalThis.__e2eCommands.push({ op: 'stop', reason }); },
        pause() { globalThis.__e2eCommands.push({ op: 'pause' }); },
        resume() { globalThis.__e2eCommands.push({ op: 'resume' }); }
    }
};
