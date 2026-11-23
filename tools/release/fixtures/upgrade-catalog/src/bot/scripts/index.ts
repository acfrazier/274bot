import UpgradeProbe, { SETTINGS } from './UpgradeProbe.js';

ScriptRegistry.register({
    name: 'UpgradeCatalog',
    settingsSchema: SETTINGS,
    create: () => new UpgradeProbe(),
});
