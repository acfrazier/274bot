// Our Game module. Reads the host-posted snapshot; unimplemented members
// throw `not v1` — never a fake value.
import { reader, actions } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';
import { host, snap, notV1, proxy, queue, optionalText } from '../../shim/_kernel.js';

const COM_MODE_VARP = 43;
const RUN_VARP = 173;
const RETALIATE_VARP = 172;
const MAGIC_TAB = 6;

const MELEE_STYLE_LABEL = {
    attack: 'accurate',
    strength: 'aggressive',
    controlled: 'controlled',
    defence: 'defensive',
};

function offeredCombatModes() {
    const labels = reader.selectButtonLabelsByVarp(-1, COM_MODE_VARP);
    return labels.length > 0 ? labels : null;
}

function wantedCombatLabel(style) {
    const key = String(style).toLowerCase();
    return MELEE_STYLE_LABEL[key] || key;
}

function matchCombatRow(style) {
    const modes = offeredCombatModes();
    if (!modes) return null;
    const wanted = wantedCombatLabel(style);
    return (
        modes.find((m) => {
            const lab = String(m.label).toLowerCase();
            return lab === wanted || lab.includes(wanted);
        }) || null
    );
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
            const row = matchCombatRow(style);
            if (!row) return false;
            return Game.combatMode() === row.mode;
        },
        combatStyleResolution(style) {
            const row = matchCombatRow(style);
            if (!row) return null;
            return { mode: row.mode, label: row.label };
        },
        setCombatMode(mode) {
            return selectCombatMode(mode);
        },
        setCombatStyle(style) {
            if (typeof style === 'number') {
                if (!selectCombatMode(style)) throw notV1('Game.setCombatStyle');
                return true;
            }
            if (!offeredCombatModes()) throw notV1('Game.setCombatStyle');
            const row = matchCombatRow(style);
            if (!row || !selectCombatMode(row.mode)) throw notV1('Game.setCombatStyle');
            return true;
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
            if (!row || typeof row.component_id !== 'number') {
                throw notV1('Game.castOnItem');
            }
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
        async castOnLoc(spell, loc) {
            if (!loc) return false;
            const wanted = String(spell).toLowerCase();
            const row = (snap().spell_buttons || []).find(
                (s) => s && s.label && String(s.label).toLowerCase() === wanted,
            );
            if (!row || typeof row.component_id !== 'number') {
                throw notV1('Game.castOnLoc');
            }
            const tile = typeof loc.tile === 'function' ? loc.tile() : loc;
            queue({
                op: 'use-widget-on',
                component_id: row.component_id,
                kind: 'loc',
                target_name: loc.name ?? null,
                x: tile && typeof tile.x === 'number' ? tile.x : 0,
                z: tile && typeof tile.z === 'number' ? tile.z : 0,
                level: tile && typeof tile.level === 'number' ? tile.level : 0,
                index: null,
            });
            return true;
        },
        teleport() {
            throw notV1('Game.teleport');
        },
        energy() {
            return typeof snap().run_energy === 'number' ? snap().run_energy : 0;
        },
        weight() {
            throw notV1('Game.weight');
        },
        cameraYaw() {
            throw notV1('Game.cameraYaw');
        },
        cameraPitch() {
            throw notV1('Game.cameraPitch');
        },
        setCameraYaw() {
            throw notV1('Game.setCameraYaw');
        },
        combatStyleMode() {
            throw notV1('Game.combatStyleMode');
        },
        sceneReady() {
            return snap().ingame === true && snap().scene_state === 2;
        },
        sceneState() {
            return typeof snap().scene_state === 'number' ? snap().scene_state : 0;
        },
        attackedByPlayer() {
            throw notV1('Game.attackedByPlayer');
        },
        async castOnNpc() {
            throw notV1('Game.castOnNpc');
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
