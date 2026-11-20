import { HostPage, rpc, UnsupportedCapabilityError } from './adapter.ts';
type NativeFacade = {
    rs2b0t: {
        runner: { start(script: unknown): void };
        registry: { get(name: string): unknown };
    };
};

let page: HostPage | undefined;
let reservedPage: HostPage | undefined;
export const HARNESS_VIEWPORT = { width: 1280, height: 720 } as const;
export function fail(message: string): never { throw new Error(`FAIL: ${message}`); }

/** Native host slots provide per-test isolation without deploying a browser bundle. */
export function deployIsolatedClient(_tag: string) {
    if (reservedPage || page) throw new Error('one native session per test');
    const nativePage = new HostPage();
    page = nativePage;
    reservedPage = nativePage;
    return {
        page: '/bot.html',
        async cleanup() {
            if (reservedPage === nativePage) reservedPage = undefined;
            await nativePage.close();
            if (page === nativePage) page = undefined;
        },
    };
}

export async function launchBrowser() {
    return {
        async newPage() {
            if (reservedPage) {
                const nativePage = reservedPage;
                reservedPage = undefined;
                return nativePage;
            }
            if (page) throw new Error('one page per test');
            page = new HostPage();
            return page;
        },
        async newContext(): Promise<never> {
            throw new UnsupportedCapabilityError('browser.newContext');
        },
        async close() {
            const active = page;
            await active?.close();
            if (page === active) page = undefined;
            if (reservedPage === active) reservedPage = undefined;
        },
    };
}

export async function boot(page: HostPage) {
    await page.goto('http://127.0.0.1:1080/bot.html');
    await rpc({ op: 'status' });
}

export async function login(page: HostPage, user: string, pass = 'test'): Promise<boolean> {
    if (pass !== 'test') throw new UnsupportedCapabilityError('non-test password login');
    await page.connect(user);
    let status = await rpc({ op: 'status' });
    if (status.ingame && status.scene === 2) return true;
    await rpc({ op: 'login' });
    const deadline = Date.now() + (Number(process.env.LOGIN_MS) || 120_000);
    while (Date.now() < deadline) {
        status = await rpc({ op: 'status' });
        if (status.ingame && status.scene === 2) return true;
        await page.waitForTimeout(100);
    }
    return false;
}

export async function cheatQuiet(page: HostPage, command: string, waitMs = 700): Promise<boolean> {
    const sent = await rpc({ op: 'cheat', command });
    await page.waitForTimeout(waitMs);
    return sent;
}

export async function setSettings(_page: HostPage, name: string, bag: Record<string, unknown>) {
    await rpc({ op: 'settings', name, bag });
}

export async function startScript(page: HostPage, name: string) {
    await page.evaluate((script: string) => {
        const host = globalThis as unknown as NativeFacade;
        const runtime = host.rs2b0t;
        runtime.runner.start(runtime.registry.get(script));
    }, name);
}

export async function startFromLibrary(page: HostPage, _category: string, script: string) {
    await startScript(page, script);
}

export async function stopScript(page: HostPage) {
    await page.evaluate(() => (globalThis as any).rs2b0t.runner.stop('harness stopScript()'));
    await page.waitForTimeout(400);
}

export async function logout(page: HostPage) {
    const sent = await rpc({ op: 'logout' });
    await page.waitForTimeout(800);
    return sent;
}

export async function type(page: HostPage, text: string, waitMs?: number) {
    if (!text.startsWith('::')) throw new UnsupportedCapabilityError('page.keyboard text input');
    const sent = await cheatQuiet(page, text.slice(2), waitMs ?? 1400);
    if (!sent) throw new Error('game command was not sent');
}

export async function bringUpOffIsland(page: HostPage, opts: { user: string; typeWaitMs?: number }) {
    if (!(await login(page, opts.user))) throw new Error('initial login failed');
    await type(page, '::tele 0,50,50,20,20', opts.typeWaitMs);
    if (!(await logout(page))) throw new Error('logout after off-island teleport failed');
    if (!(await login(page, opts.user))) throw new Error('relogin after off-island teleport failed');
}

export async function requireSim(base: string) {
    let target: URL;
    try {
        target = new URL(base);
    } catch {
        throw new UnsupportedCapabilityError('external simulator preflight');
    }
    if (!['localhost', '127.0.0.1', '::1'].includes(target.hostname)) {
        throw new UnsupportedCapabilityError('external simulator preflight');
    }
    await rpc({ op: 'preflight' });
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
