import { actions, reader } from '../../adapter/ClientAdapter.js';
import { Execution } from '../execution/Execution.js';
import { spellButtonCom } from '../combat/CombatStyleLogic.js';
import { notImpl } from '../../shim/_kernel.js';

const COMBAT_TAB = 0;

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

function observation(c) {
    return {
        ingame: globalThis.__rs2b0t_host?.snapshot?.ingame === true,
        active_side_tab: reader.activeSideTab(),
        combat_tab_root: combatTabRoot(),
        magic_varp_value: magicValue(c.magic_varp),
    };
}

function live(c) {
    const obs = observation(c);
    return call({
        op: 'observe',
        combat_tab_root: obs.combat_tab_root,
        magic_varp_value: obs.magic_varp_value,
    });
}

function logFailure(reason, spellName, log) {
    if (reason === 'staff-missing') {
        log?.('combat tab is not the staff layout — is a staff wielded?');
    } else if (reason === 'open-tab') {
        log?.('could not open the combat tab');
    } else if (reason === 'chooser') {
        log?.('spell chooser did not open');
    } else if (reason === 'select') {
        log?.(`choosing '${spellName}' did not take — magic level too low?`);
    } else if (reason === 'toggle') {
        log?.('autocast toggle did not arm');
    }
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
        if (
            !c ||
            c.available !== true ||
            c.choose_com < 0 ||
            c.toggle_com < 0 ||
            c.spell_grid_base < 0
        ) {
            log?.(`not impl: Autocast.arm needs posted coms for '${spellName}'`);
            throw notImpl('Autocast.arm');
        }
        const ssbCom = spellButtonCom(spellName);
        if (ssbCom === -1) {
            log?.(`'${spellName}' is not an autocastable spell — see SPELL_DB (Wind Strike … Fire Wave)`);
            return false;
        }
        let step = call({
            op: 'begin',
            spell_com: ssbCom,
            observation: observation(c),
        });
        const token = step?.token;
        while (step && step.kind !== 'done' && step.kind !== 'aborted') {
            if (step.kind === 'side-tab') {
                actions.clickSideTab(step.tab);
            } else if (step.kind === 'if-button') {
                actions.ifButton(step.component_id);
            } else if (step.kind !== 'wait') {
                return false;
            }
            let next = null;
            await Execution.delayUntil(() => {
                next = call({
                    op: 'next',
                    token,
                    observation: observation(c),
                });
                return next?.kind !== 'wait';
            }, 0);
            step = next;
        }
        if (step?.kind !== 'done' || step.ok !== true) {
            logFailure(step?.reason, spellName, log);
            return false;
        }
        log?.(`autocast armed: ${spellName}`);
        return true;
    },
};