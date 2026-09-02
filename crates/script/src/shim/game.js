// Our Game module. Reads the host-posted snapshot; unimplemented members
// throw `not v1` — never a fake value.
import { reader, actions } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';
import { host, snap, notV1, proxy, queue, optionalText } from '../../shim/_kernel.js';

const COM_MODE_VARP = 43;
const RUN_VARP = 173;
const RETALIATE_VARP = 172;
const MAGIC_TAB = 6;

function offeredCombatModes() {
    const labels = reader.selectButtonLabelsByVarp(-1, COM_MODE_VARP);
    return labels.length > 0 ? labels : null;
}

function selectCombatMode(mode) {
    const btn = reader.selectButtonByVarp(-1, COM_MODE_VARP, mode);
    return btn !== -1 && actions.ifButton(btn);
}

export const Game = new Proxy(
    {
        ingame() {
            return snap().ingame === true;
        },
        tile() {
            return snap().here || null;
        },
        tick() {
            return host().tick || 0;
        },
        inCombat() {
            return reader.inCombat();
        },
        animating() {
            return reader.selfAnim() !== -1;
        },
        runEnabled() {
            if (snap().run_enabled !== undefined) return snap().run_enabled === true;
            return reader.varp(RUN_VARP) === 1;
        },
        autoRetaliate() {
            if (snap().retaliate_enabled !== undefined) return snap().retaliate_enabled === true;
            return reader.varp(RETALIATE_VARP) === 0;
        },
        autoRetaliateOn() {
            return Game.autoRetaliate();
        },
        myName() {
            return optionalText(snap().my_name);
        },
        combatMode() {
            return reader.varp(COM_MODE_VARP);
        },
        combatStyles() {
            return offeredCombatModes();
        },
        hasCombatStyle(style) {
            const modes = offeredCombatModes();
            if (!modes) return false;
            const wanted = String(style).toLowerCase();
            return modes.some((m) => String(m.label).toLowerCase().includes(wanted));
        },
        combatStyleResolution(style) {
            const modes = offeredCombatModes();
            if (!modes) return null;
            const wanted = String(style).toLowerCase();
            const idx = modes.findIndex((m) => String(m.label).toLowerCase().includes(wanted));
            if (idx === -1) return null;
            return { mode: modes[idx].mode, label: modes[idx].label };
        },
        setCombatMode(mode) {
            return selectCombatMode(mode);
        },
        setCombatStyle(style) {
            if (typeof style === 'number') return Game.setCombatMode(style);
            const res = Game.combatStyleResolution(style);
            return res ? selectCombatMode(res.mode) : false;
        },
        setAutoRetaliate(on) {
            return actions.setRetaliate(on);
        },
        async openSideTab(tab) {
            if (reader.activeSideTab() === tab) return true;
            if (!actions.clickSideTab(tab)) return false;
            return Execution.delayUntil(() => reader.activeSideTab() === tab, 2000);
        },
        async castOnItem(spell, item) {
            if (!item) return false;
            const wanted = String(spell).toLowerCase();
            const row = (snap().spell_buttons || []).find(
                (s) => s && s.label && String(s.label).toLowerCase() === wanted,
            );
            if (!row || typeof row.component_id !== 'number') return false;
            queue({
                op: 'use-widget-on',
                component_id: row.component_id,
                kind: 'held',
                target_name: item.name ?? null,
                x: 0,
                z: 0,
                level: 0,
                index: null,
            });
            return true;
        },
        teleport() {
            throw notV1('Game.teleport');
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Game.' + String(prop));
        },
    },
);
