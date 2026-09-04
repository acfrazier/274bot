// Name-map: tinderbox use-on logs. No burn-lane wait / XP poll.
import { Inventory } from '../inventory/Inventory.js';

export function lightFire(logName) {
    const logs = Inventory.first(logName);
    const tinder = Inventory.first('Tinderbox');
    if (!logs || !tinder) return false;
    return tinder.useOn(logs);
}
