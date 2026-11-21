export const LOADOUT_SETTING = {
    type: 'string',
    default: '',
    options: [],
    optionsFrom: 'loadouts',
    label: 'Loadout',
    help: 'Use an exact name first. Otherwise, match one loadout name after trimming both names and ignoring ASCII case. A blank setting selects the first saved loadout, or no loadout when none exist. The host refuses unknown or ambiguous names.',
};

export function selectedLoadout(bag) {
    return globalThis.rustyscript.functions.__rs2b0t_selected_loadout(bag.str('loadout', ''));
}
