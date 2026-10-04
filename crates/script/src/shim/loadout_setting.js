export const LOADOUT_SETTING = {
    type: 'string',
    default: '',
    options: [],
    optionsFrom: 'loadouts',
    label: 'Loadout',
    help: 'select a loadout by its exact, case-sensitive name; blank selects none',
};

export function selectedLoadout(bag) {
    return globalThis.rustyscript.functions.__rs2b0t_selected_loadout(bag.str('loadout', ''));
}
