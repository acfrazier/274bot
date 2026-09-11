import { Bank } from '../../api/bank/Bank.js';
import { Equipment } from '../../api/equipment/Equipment.js';
import { selectedLoadout } from '../../api/loadout/loadoutSetting.js';
import { suppliesOf, weaponOf } from '../../api/loadout/loadoutPlan.js';

export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__done || globalThis.__busy) return;
        globalThis.__busy = true;
        try {
            const loadout = selectedLoadout(this.settings);
            const supplies = suppliesOf(loadout);
            const weapon = weaponOf(loadout, null);
            globalThis.__weapon = weapon;
            globalThis.__qty = supplies && supplies.length ? supplies[0].qty : 0;
            if (!weapon || !supplies || !supplies.length || !supplies[0].item || !supplies[0].qty) {
                globalThis.__err = 'missing loadout weapon or supply';
                globalThis.__done = true;
                return;
            }
            if (!Bank.ready()) {
                const opened = await Bank.openNearestWorld();
                if (!opened || !Bank.ready()) {
                    globalThis.__stale = true;
                    return;
                }
            }
            globalThis.__generation = Bank.snapshotGeneration();
            const supplyOk = await Bank.withdrawX(supplies[0].item, supplies[0].qty);
            globalThis.__withdraw_supply = supplyOk;
            if (!supplyOk) {
                globalThis.__done = true;
                return;
            }
            const weaponOk = await Bank.withdrawX(weapon, 1);
            globalThis.__withdraw_weapon = weaponOk;
            if (!weaponOk) {
                globalThis.__done = true;
                return;
            }
            globalThis.__equipped = await Equipment.equip(weapon);
            globalThis.__done = true;
        } catch (e) {
            globalThis.__err = String(e && e.message ? e.message : e);
            globalThis.__done = true;
        } finally {
            globalThis.__busy = false;
        }
    }
}
