import { HostPage, rpc } from './adapter.ts';
import { cheatQuiet, login as nativeLogin, logout } from './harness.ts';
export { cheatQuiet, startScript } from './harness.ts';

async function ready(page: HostPage, timeout = 120_000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
        const status = await rpc({ op: 'status' });
        if (status.ingame && status.scene === 2) return;
        await page.waitForTimeout(100);
    }
    throw new Error('host session did not reach ingame scene2');
}
export async function bootAndLogin(page: HostPage, base: string, user: string, clientPage = '/bot.html') {
    await page.goto(`${base}${clientPage}`);
    if (!(await nativeLogin(page, user))) throw new Error('bootAndLogin: login did not reach scene2');
}
export async function relog(page: HostPage, _user: string) {
    if (!(await logout(page))) throw new Error('clean logout refused');
    await page.waitForTimeout(1200);
    await rpc({ op: 'login' });
    await ready(page);
}
export async function mainlandAccount(page: HostPage, _base: string, user: string, _clientPage?: string) {
    await page.connect(user);
    await ready(page);
    await cheatQuiet(page, 'tele 0,50,50,20,20');
    await cheatQuiet(page, 'setvar tutorial 1000');
    if (await getServerVarQuiet(page, 'tutorial') !== 1000) throw new Error('tutorial seed did not land');
    await relog(page, user);
    const status = await rpc({ op: 'status' });
    if (status.inventoryTab === -1) throw new Error('tutorial sidebar remains locked');
}
export async function getServerVarQuiet(page: HostPage, name: string): Promise<number | null> {
    await cheatQuiet(page, `getvar ${name}`, 900);
    const lines = await page.evaluate(() => (globalThis as any).rs2b0t.reader.chat(8));
    const line = lines.find((row: any) => row.text.toLowerCase().startsWith(`get ${name.toLowerCase()}:`));
    if (!line) return null;
    const value = Number.parseInt(line.text.split(':')[1]?.trim(), 10);
    return Number.isNaN(value) ? null : value;
}
export async function getServerVar(page: HostPage, name: string): Promise<number | null> {
    return getServerVarQuiet(page, name);
}
export async function clearChatDialogs(page: HostPage, _label?: string) {
    let quiet = 0;
    for (let i = 0; i < 120; i++) {
        const state = await page.evaluate(() => {
            const { reader } = (globalThis as any).rs2b0t;
            return { chat: reader.modals().chat, button: reader.chatContinueComId(), options: reader.chatOptions() };
        });
        if (state.chat === -1 && state.button === -1 && state.options.length === 0) {
            if (++quiet >= 4) return;
        } else {
            quiet = 0;
            await page.evaluate(() => {
                const { reader, actions } = (globalThis as any).rs2b0t;
                if (reader.chatContinueComId() !== -1) return actions.continueDialog();
                const options = reader.chatOptions();
                return options.length > 0 && actions.ifButton(options[0].comId);
            });
        }
        await page.waitForTimeout(250);
    }
    throw new Error('chat dialogs did not clear');
}
export async function maxmeAndClearDialogs(page: HostPage) {
    if (!(await cheatQuiet(page, '~maxme'))) throw new Error('~maxme not sent (not ingame?)');
    await page.waitForFunction(() => {
        const skills = (globalThis as unknown as { __rs2b0t: { Skills: { level(name: string): number } } }).__rs2b0t.Skills;
        return skills.level('attack') >= 99 && skills.level('hitpoints') >= 99;
    }, undefined, { timeout: 45_000 });
    await clearChatDialogs(page, 'level-up dialog(s)');
    await page.waitForTimeout(1500);
    await clearChatDialogs(page, 'straggler dialog(s)');
}
export async function teleTo(page: HostPage, tile: { x: number; z: number; level: number }, radius = 8, timeoutMs = 20_000) {
    const ox = Math.floor(tile.x / 64), oz = Math.floor(tile.z / 64);
    if (!(await cheatQuiet(page, `tele ${tile.level},${ox},${oz},${tile.x % 64},${tile.z % 64}`))) return false;
    try {
        await page.waitForFunction(({ target, radius }: any) => {
            const here = (globalThis as any).rs2b0t.reader.worldTile();
            return here && here.level === target.level && Math.max(Math.abs(here.x - target.x), Math.abs(here.z - target.z)) <= radius;
        }, { target: tile, radius }, { timeout: timeoutMs });
        return true;
    } catch { return false; }
}
export type BankSeedItem = { debugName: string; displayName: string; qty: number };
export async function seedItemsToBank(page: HostPage, items: readonly { debugName: string; displayName: string; qty: number }[], stand: { x: number; z: number; level: number }) {
    if (!(await teleTo(page, stand, 6, 25_000))) throw new Error('bank seed teleport failed');
    for (const item of items) {
        if (!Number.isSafeInteger(item.qty) || item.qty <= 0) throw new Error('invalid seed quantity');
        await cheatQuiet(page, `givebank ${item.debugName} ${item.qty}`);
    }
    // Only fixture placement differs. Verify the real bank before the test starts.
    const opened = await page.evaluate(async () => (globalThis as any).__rs2b0t.Bank.openNearest('Bank booth', 'Use-quickly'));
    if (!opened) throw new Error('seed bank did not open');
    await page.waitForFunction(() => (globalThis as any).__rs2b0t.Bank.ready());
    for (const item of items) {
        const actual = await page.evaluate((name: string) => (globalThis as any).__rs2b0t.Bank.count(name), item.displayName);
        if (actual !== item.qty) throw new Error(`seed ${item.debugName}: bank has ${actual}, expected ${item.qty}`);
    }
    if (!(await page.evaluate(async () => (globalThis as any).__rs2b0t.Bank.close()))) throw new Error('seed bank did not close');
}
