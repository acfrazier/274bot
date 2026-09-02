export const LOADOUT_SETTING = {
    type: 'string',
    default: '',
    options: [],
    optionsFrom: 'loadouts',
    label: 'Loadout',
    help: 'gear and supplies to wear, defined in the Loadouts panel; blank uses the first one',
};

export function selectedLoadout(_bag) {
    return null;
}
