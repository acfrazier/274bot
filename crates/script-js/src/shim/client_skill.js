const selected = (method) =>
    globalThis.__rs2b0t_selected_facts('client-adapter-reader', method);

const names = Object.freeze(selected('skillNames'));
const used = Object.freeze(selected('skillUsedMask'));

export default class Skill {
    static names = names;
    static used = used;
}
