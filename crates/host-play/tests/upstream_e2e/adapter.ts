import { plugin } from 'bun';
import { isAbsolute, join, resolve } from 'node:path';

export async function rpc(request: unknown): Promise<any> {
    const endpoint = process.env.UEP_ENDPOINT;
    if (!endpoint) throw new Error('missing UEP_ENDPOINT');
    const [hostname, rawPort] = endpoint.split(':');
    const { promise, resolve, reject } = Promise.withResolvers<any>();
    let response = '';
    Bun.connect({ hostname, port: Number(rawPort), socket: {
            open(socket) { socket.write(`${JSON.stringify(request)}\n`); },
            data(socket, data) {
                response += Buffer.from(data).toString();
                if (!response.includes('\n')) return;
                socket.end();
                try {
                    const result = JSON.parse(response.trim());
                    if (!result.ok) {
                        const message = String(result.error);
                        if (/^unsupported\b/i.test(message)) reject(new UnsupportedCapabilityError(message));
                        else reject(new Error(message));
                    }
                    else resolve(result.value);
                } catch (error) { reject(error); }
            },
            error(_socket, error) { reject(error); },
    }}).catch(reject);
    return promise;
}

function delay(ms: number) {
    const { promise, resolve } = Promise.withResolvers<void>();
    setTimeout(resolve, ms);
    return promise;
}

export class UnsupportedCapabilityError extends Error {
    constructor(capability: string) {
        super(`UNSUPPORTED_CAPABILITY: ${capability}`);
        this.name = 'UnsupportedCapabilityError';
    }
}

type HostConsoleRecord = {
    account?: string;
    tick?: number;
    source?: string;
    level?: string;
    message?: string;
};

type HostConsoleMessage = {
    text(): string;
    type(): string;
};

type HostConsoleListener = (message: HostConsoleMessage) => unknown;

export class HostPage {
    private connected = false;
    private connectedUser: string | undefined;
    private closed = false;
    private evaluation = 0;
    private readonly consoleListeners = new Set<HostConsoleListener>();
    private readonly pageErrorListeners = new Set<(error: Error) => unknown>();
    private lastScriptError: string | undefined;
    private lastHostEvents: string[] = [];
    private consoleTimer: ReturnType<typeof setTimeout> | undefined;
    private consolePoll: Promise<void> | undefined;
    private consoleFailure: unknown;
    private closing: Promise<void> | undefined;

    readonly keyboard = {
        type: () => this.unsupported('page.keyboard.type'),
        press: () => this.unsupported('page.keyboard.press'),
    };
    readonly mouse = {
        click: () => this.unsupported('page.mouse.click'),
    };

    private unsupported(capability: string): never {
        throw new UnsupportedCapabilityError(capability);
    }
    private throwConsoleFailure() {
        if (this.consoleFailure === undefined) return;
        if (this.consoleFailure instanceof Error) throw this.consoleFailure;
        throw new Error(`host console observation failed: ${String(this.consoleFailure)}`);
    }

    private scheduleConsolePolling() {
        if (this.closed || !this.connected || (!this.consoleListeners.size && !this.pageErrorListeners.size) ||
            this.consoleFailure !== undefined || this.consoleTimer !== undefined) return;
        this.consoleTimer = setTimeout(() => {
            this.consoleTimer = undefined;
            const poll = this.pollConsoleLogs();
            this.consolePoll = poll;
            void poll.finally(() => {
                if (this.consolePoll === poll) this.consolePoll = undefined;
                this.scheduleConsolePolling();
            });
        }, 1000);
    }

    private async pollConsoleLogs() {
        try {
            const diagnostics = await rpc({ op: 'diagnostics' });
            if (this.closed || !this.connected) return;
            if (!diagnostics || !Array.isArray(diagnostics.host_events)) {
                throw new Error('native diagnostics omitted host_events');
            }
            const nativeError = typeof diagnostics.error === 'string' ? diagnostics.error : undefined;
            if (nativeError && nativeError !== this.lastScriptError) {
                for (const listener of this.pageErrorListeners) await listener(new Error(nativeError));
            }
            this.lastScriptError = nativeError;
            const events = diagnostics.host_events as HostConsoleRecord[];
            const keys = events.map(event => JSON.stringify([
                event.account, event.tick, event.source, event.level, event.message,
            ]) ?? '');
            let overlap = 0;
            const maxOverlap = Math.min(this.lastHostEvents.length, keys.length);
            for (let count = maxOverlap; count > 0; count--) {
                let matches = true;
                const previousStart = this.lastHostEvents.length - count;
                for (let index = 0; index < count; index++) {
                    if (this.lastHostEvents[previousStart + index] !== keys[index]) {
                        matches = false;
                        break;
                    }
                }
                if (matches) {
                    overlap = count;
                    break;
                }
            }
            for (let index = overlap; index < events.length; index++) {
                const record = events[index];
                if (!record || typeof record.message !== 'string') {
                    throw new Error('native host event has no message');
                }
                const message: HostConsoleMessage = {
                    text: () => record.message!,
                    type: () => typeof record.level === 'string' ? record.level.toLowerCase() : 'log',
                };
                for (const listener of this.consoleListeners) {
                    try {
                        const result = listener(message);
                        if (result && typeof (result as PromiseLike<unknown>).then === 'function') {
                            void Promise.resolve(result).catch(error => { this.consoleFailure = error; });
                        }
                    } catch (error) {
                        this.consoleFailure = error;
                        return;
                    }
                }
            }
            this.lastHostEvents = keys;
        } catch (error) {
            if (!this.closed) this.consoleFailure = error;
        }
    }


    on(event: string, callback: unknown): this {
        if (event !== 'console' && event !== 'pageerror') return this.unsupported(`page.on(${event})`);
        if (typeof callback !== 'function') throw new TypeError(`page.on ${event} callback must be a function`);
        if (event === 'console') this.consoleListeners.add(callback as HostConsoleListener);
        else this.pageErrorListeners.add(callback as (error: Error) => unknown);
        this.scheduleConsolePolling();
        return this;
    }
    async connect(user: string) {
        if (this.closed) throw new Error('host page is closed');
        if (this.connected) {
            if (this.connectedUser !== undefined && this.connectedUser !== user) {
                throw new UnsupportedCapabilityError('multiple users on one native page');
            }
            this.connectedUser = user;
            return;
        }
        await rpc({ op: 'connect', user });
        this.connected = true;
        this.connectedUser = user;
        this.scheduleConsolePolling();
    }
    async goto(url: string) {
        let target: URL;
        try {
            target = new URL(url);
        } catch {
            return this.unsupported('page.goto requires a local absolute URL');
        }
        if (!['http:', 'https:'].includes(target.protocol) ||
            !['localhost', '127.0.0.1', '[::1]', '::1'].includes(target.hostname)) {
            return this.unsupported('page.goto outside the local native host');
        }
        const user = process.env.USER_NAME;
        if (!user) throw new Error('missing USER_NAME for native host page');
        if (!this.connected) {
            await this.connect(user);
            // Boot has not selected the upstream helper's logical account yet.
            this.connectedUser = undefined;
        }
    }
    isClosed() { return this.closed; }
    reload(): never { return this.unsupported('page.reload'); }
    locator(): never { return this.unsupported('page.locator'); }
    getByRole(): never { return this.unsupported('page.getByRole'); }
    getByText(): never { return this.unsupported('page.getByText'); }
    waitForSelector(): never { return this.unsupported('page.waitForSelector'); }
    route(): never { return this.unsupported('page.route'); }
    addInitScript(): never { return this.unsupported('page.addInitScript'); }
    url(): never { return this.unsupported('page.url'); }
    async evaluate(fn: Function, arg?: unknown): Promise<any> {
        if (this.closed) throw new Error('host page is closed');
        this.throwConsoleFailure();
        const id = ++this.evaluation;
        const expression = `(() => {
            const finish = value => globalThis.__e2eEvaluation = { id: ${id}, done: true, value: value ?? null };
            const fail = error => globalThis.__e2eEvaluation = { id: ${id}, done: true, error: String(error?.stack ?? error) };
            globalThis.__e2eEvaluation = { id: ${id}, done: false };
            try {
                const value = (${fn.toString()})(${JSON.stringify(arg) ?? 'undefined'});
                if (value && typeof value.then === 'function') value.then(finish, fail);
                else finish(value);
            } catch (error) { fail(error); }
            return globalThis.__e2eEvaluation;
        })()`;
        let result = await rpc({ op: 'evaluate', expression });
        const deadline = Date.now() + 90_000;
        while (!result.done) {
            if (Date.now() >= deadline) throw new Error('evaluate promise deadline');
            await delay(20);
            result = await rpc({ op: 'evaluate', expression: 'globalThis.__e2eEvaluation' });
            if (!result || result.id !== id) throw new Error('evaluation isolate retired before its promise settled');
        }
        if (result.error) {
            const message = String(result.error);
            if (message.startsWith('UNSUPPORTED_CAPABILITY:')) {
                throw new UnsupportedCapabilityError(message.slice('UNSUPPORTED_CAPABILITY:'.length).trim());
            }
            throw new Error(message);
        }
        this.throwConsoleFailure();
        return result.value;
    }
    async waitForFunction(fn: Function, arg?: unknown, options?: { timeout?: number; polling?: number | 'raf' }) {
        if (options?.polling === 'raf') return this.unsupported('waitForFunction polling=raf');
        const polling = options?.polling ?? 100;
        if (!Number.isSafeInteger(polling) || polling <= 0) {
            throw new Error('waitForFunction polling must be a positive integer');
        }
        const deadline = Date.now() + (options?.timeout ?? 30_000);
        for (;;) {
            let value;
            try {
                value = await this.evaluate(fn, arg);
            } catch (error) {
                if (!(error instanceof Error) ||
                    (error.message !== 'probe: isolate thread gone' &&
                     error.message !== 'probe: channel is empty and sending half is closed')) throw error;
                const remaining = deadline - Date.now();
                if (remaining <= 0) throw new Error(`waitForFunction timed out: ${fn.toString()}`);
                // Polls may cross a clean target stop. Wait for native terminal truth
                // before the next poll; one-shot evaluate never replays an expression.
                await rpc({ op: 'await-terminal', timeout_ms: remaining });
                continue;
            }
            if (value) return { jsonValue: async () => value };
            if (Date.now() >= deadline) throw new Error(`waitForFunction timed out: ${fn.toString()}`);
            await delay(polling);
        }
    }
    async waitForTimeout(ms: number) {
        const deadline = Date.now() + ms;
        while (Date.now() < deadline) {
            this.throwConsoleFailure();
            await delay(Math.min(500, deadline - Date.now()));
        }
        this.throwConsoleFailure();
    }
    screenshot(options: { path: string }) {
        if (isAbsolute(options.path)) throw new Error('screenshot path must be relative');
        const caseId = process.env.UEP_CASE_ID;
        const path = caseId ? join(caseId, options.path) : options.path;
        return rpc({ op: 'screenshot', path });
    }
    async close() {
        if (this.closing) return this.closing;
        this.closed = true;
        if (this.consoleTimer !== undefined) {
            clearTimeout(this.consoleTimer);
            this.consoleTimer = undefined;
        }
        this.closing = (async () => {
            if (this.consolePoll) await this.consolePoll;
            if (this.connected) {
                await rpc({ op: 'close' });
                this.connected = false;
                this.connectedUser = undefined;
            }
            this.throwConsoleFailure();
        })();
        return this.closing;
    }
}

export function installAdapters() {
    const directory = import.meta.dir;
    plugin({ name: '274bot-upstream-e2e', setup(build) {
        build.onResolve({ filter: /(?:lib|tutorial)\/harness\.js$/ }, args => ({
            path: join(directory, args.path.includes('tutorial/') ? 'tutorial.ts' : 'harness.ts'),
        }));
    }});
}

export const outputDirectory = () => resolve(process.env.UEP_CASE_OUTPUT ?? process.env.UEP_OUTPUT ?? '.');
