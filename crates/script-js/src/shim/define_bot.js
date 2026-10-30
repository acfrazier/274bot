import { ScriptRegistry } from './ScriptRegistry.js';

export const defineBot = globalThis.defineBot;

export function isBotManifest(value) {
    return typeof value === 'object' && value !== null && value.__rs2b0tManifest === 1;
}

export function registerScript(manifest, origin) {
    const tagged = isBotManifest(manifest) ? manifest : defineBot(manifest);
    const meta = {
        name: tagged.name,
        description: tagged.description ?? '',
        version: tagged.version,
        category: tagged.category,
        tags: tagged.tags,
        origin,
        settingsSchema: tagged.settingsSchema,
        create: tagged.create,
    };

    ScriptRegistry.register(meta);
    return meta;
}
