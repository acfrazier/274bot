import { HostPage, rpc } from './adapter.ts';

let page: HostPage | undefined;
let deployment: string | undefined;
export function fail(message: string): never { throw new Error(`FAIL: ${message}`); }
export function deployIsolatedClient(tag: string) {
    if (deployment) throw new Error('pilot permits one deployment per test');
    deployment = tag;
    return { page: `host://${tag}`, cleanup() { deployment = undefined; } };
}
export async function launchBrowser() {
    return {
        async newPage() {
            if (page) throw new Error('pilot permits one page per test');
            page = new HostPage();
            return page;
        },
        async close() { await page?.close(); page = undefined; },
    };
}
export async function cheatQuiet(page: HostPage, command: string, waitMs = 700): Promise<boolean> {
    const sent = await rpc({ op: 'cheat', command });
    await page.waitForTimeout(waitMs);
    return sent;
}
export async function setSettings(_page: HostPage, name: string, bag: Record<string, unknown>) {
    await rpc({ op: 'settings', name, bag });
}
export async function stopScript(page: HostPage) {
    await page.evaluate(() => (globalThis as any).rs2b0t.runner.stop('harness stopScript()'));
}
export async function logout(page: HostPage) {
    const sent = await rpc({ op: 'logout' });
    await page.waitForTimeout(800);
    return sent;
}
export function positionalArgs(argv: string[], fallbackBase: string): string[] {
    const flags = new Set(['--base', '--minutes', '--stage', '--engine', '--item']);
    const bare: string[] = [];
    let base: string | undefined;
    for (let i = 0; i < argv.length; i++) {
        if (flags.has(argv[i])) { if (argv[i] === '--base') base = argv[i + 1]; i++; }
        else if (!argv[i].startsWith('-')) bare.push(argv[i]);
    }
    const positional = bare[0]?.includes('://') ? bare.shift() : undefined;
    return [base ?? positional ?? fallbackBase, ...bare];
}
export function parseArgs(argv: string[], defaults?: { base?: string; minutes?: number }) {
    let base = defaults?.base ?? 'http://localhost:8890';
    let minutes = defaults?.minutes ?? 0;
    const rest: string[] = [];
    for (let i = 0; i < argv.length; i++) {
        const value = argv[i];
        if (value === '--base') base = argv[++i];
        else if (value === '--minutes') minutes = Number(argv[++i]);
        else if (value.includes('://')) base = value;
        else if (value.trim() && Number.isFinite(Number(value))) minutes = Number(value);
        else if (!value.startsWith('-')) rest.push(value);
    }
    return { base, minutes, rest };
}
export type Rs2b0t = any;
