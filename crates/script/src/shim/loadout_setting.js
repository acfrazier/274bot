export const LOADOUT_SETTING = {
    type: 'string',
    default: '',
    options: [],
    optionsFrom: 'loadouts',
    label: 'Loadout',
    help: 'gear and supplies to wear, defined in the Loadouts panel; blank uses the first one',
};

export function selectedLoadout(bag) {
    return globalThis.rustyscript.functions.__rs2b0t_selected_loadout(globalThis.__rs2b0t_host.loadouts || [], bag.str('loadout', ''));
}
