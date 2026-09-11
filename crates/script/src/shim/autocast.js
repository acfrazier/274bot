import { actions, reader } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';
import { Game } from '../game/Game.js';
import { spellButtonCom } from '../combat/CombatStyleLogic.js';
import { notImpl } from '../../shim/_kernel.js';

const COMBAT_TAB = 0;
const STEP_MS = 3000;

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_autocast(payload);
}

function controls() {
    return call({ op: 'controls' });
}

function combatTabRoot() {
    try {
        const id = reader.sideTabInterface(COMBAT_TAB);
        return typeof id === 'number' ? id : -1;
    } catch (_) {
        return -1;
    }
}

function magicValue(index) {
    return reader.varp(index);
}

function live(c) {
    return call({
        op: 'observe',
        combat_tab_root: combatTabRoot(),
        magic_varp_value: magicValue(c.magic_varp),
    });
}

export const Autocast = {
    armed() {
        const c = controls();
        if (!c || c.available !== true) return false;
        return live(c).armed === true;
    },
    staffTabAttached() {
        const c = controls();
        if (!c || c.available !== true) return false;
        return live(c).staff_attached === true;
    },
    async arm(spellName, log) {
        const c = controls();
        if (!c || c.available !== true || c.choose_com < 0 || c.toggle_com < 0 || c.spell_grid_base < 0) {
            log?.(`not impl: Autocast.arm needs posted coms for '${spellName}'`);
            throw notImpl('Autocast.arm');
        }
        const ssbCom = spellButtonCom(spellName);
        if (ssbCom === -1) {
            log?.(`'${spellName}' is not an autocastable spell — see SPELL_DB (Wind Strike … Fire Wave)`);
            return false;
        }
        if (!this.staffTabAttached()) {
            log?.('combat tab is not the staff layout — is a staff wielded?');
            return false;
        }
        const token = call({ op: 'begin' }).token;
        const aborted = () => call({ op: 'current_token' }) !== token;
        if (!(await Game.openSideTab(COMBAT_TAB))) {
            log?.('could not open the combat tab');
            return false;
        }
        if (aborted()) return false;
        actions.ifButton(c.choose_com);
        if (
            !(await Execution.delayUntil(
                () => aborted() || live(c).panel_open === true,
                STEP_MS,
            ))
        ) {
            log?.('spell chooser did not open');
            return false;
        }
        if (aborted()) return false;
        actions.ifButton(ssbCom);
        if (
            !(await Execution.delayUntil(
                () => aborted() || live(c).selected === true,
                STEP_MS,
            ))
        ) {
            log?.(`choosing '${spellName}' did not take — magic level too low?`);
            return false;
        }
        if (aborted()) return false;
        actions.ifButton(c.toggle_com);
        if (
            !(await Execution.delayUntil(() => aborted() || this.armed(), STEP_MS))
        ) {
            log?.('autocast toggle did not arm');
            return false;
        }
        if (aborted()) return false;
        log?.(`autocast armed: ${spellName}`);
        return true;
    },
};
