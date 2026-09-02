import { actions, reader } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';
import { Game } from '../game/Game.js';
import { notV1, snap } from '../../shim/_kernel.js';

const COMBAT_TAB = 0;
const STEP_MS = 3000;

function postedCom(label) {
    const wanted = String(label).toLowerCase();
    const styles = snap().combat_styles || [];
    const row = styles.find((s) => s && s.label && String(s.label).toLowerCase().includes(wanted));
    return row && typeof row.component_id === 'number' ? row.component_id : -1;
}

export const Autocast = {
    armed() {
        return reader.varp(108) === 3;
    },
    staffTabAttached() {
        try {
            const id = reader.sideTabInterface(COMBAT_TAB);
            return typeof id === 'number' && id !== -1;
        } catch (_) {
            return false;
        }
    },
    async arm(spellName, log) {
        const spells = snap().spell_buttons || [];
        const wanted = String(spellName).toLowerCase();
        const spell = spells.find((s) => s && s.label && String(s.label).toLowerCase() === wanted);
        const choose = postedCom('choose') !== -1 ? postedCom('choose') : -1;
        const toggle = postedCom('autocast') !== -1 ? postedCom('autocast') : postedCom('auto');
        if (!spell || choose === -1 || toggle === -1) {
            log?.(`not v1: Autocast.arm needs posted coms for '${spellName}'`);
            throw notV1('Autocast.arm');
        }
        if (!(await Game.openSideTab(COMBAT_TAB))) {
            log?.('could not open the combat tab');
            return false;
        }
        actions.ifButton(choose);
        actions.ifButton(spell.component_id);
        actions.ifButton(toggle);
        return Execution.delayUntil(() => this.armed(), STEP_MS);
    },
};
