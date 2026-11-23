import { SettingsStore } from '../../runtime/Settings.js';

export const SETTINGS = { mode: { type: 'string', default: 'default' } };

export default class UpgradeProbe extends LoopingBot {
    onStart() {
        const mode = this.settings.str('mode', 'default');
        const raw = SettingsStore.saved('UpgradeProbe', 'sentinel');
        this.log(`UPGRADE_READ mode=${mode} raw=${JSON.stringify(raw)}`);
        if (mode === 'seed') SettingsStore.save('UpgradeProbe', 'sentinel', '  release:029  ');
    }
    loop() {}
}
