// SETTINGS keys stay a static object so origin parse can inline Object.keys.
// Posted selected-revision rows fill ssb/level/runes; empty objects remain
// when the isolate has no content blob.
const posted = (globalThis.__rs2b0t_host && globalThis.__rs2b0t_host.content) || {};
const rows = posted.spell_db || {};

export const SPELL_DB = {
    'Wind Strike': {},
    'Water Strike': {},
    'Earth Strike': {},
    'Fire Strike': {},
    'Wind Bolt': {},
    'Water Bolt': {},
    'Earth Bolt': {},
    'Fire Bolt': {},
    'Wind Blast': {},
    'Water Blast': {},
    'Earth Blast': {},
    'Fire Blast': {},
    'Wind Wave': {},
    'Water Wave': {},
    'Earth Wave': {},
    'Fire Wave': {},
};

for (const name of Object.keys(SPELL_DB)) {
    if (rows[name]) {
        SPELL_DB[name] = rows[name];
    }
}

export const STAFF_RUNES = posted.staff_runes || {};
