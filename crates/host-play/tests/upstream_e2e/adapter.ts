import { plugin } from 'bun';
import { join, resolve } from 'node:path';

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
                    if (!result.ok) reject(new Error(result.error));
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

export class HostPage {
    private connected = false;
    private evaluation = 0;
    on(event: string, _callback: unknown) {
        if (event !== 'pageerror') throw new Error(`unsupported page event: ${event}`);
        // There is no browser. Isolate exceptions reject RPC and fail the test.
    }
    async connect(user: string) {
        if (this.connected) throw new Error('one host session per page');
        await rpc({ op: 'connect', user });
        this.connected = true;
    }
    async evaluate(fn: Function, arg?: unknown): Promise<any> {
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
        if (result.error) throw new Error(result.error);
        return result.value;
    }
    async waitForFunction(fn: Function, arg?: unknown, options?: { timeout?: number }) {
        const deadline = Date.now() + (options?.timeout ?? 30_000);
        for (;;) {
            const value = await this.evaluate(fn, arg);
            if (value) return { jsonValue: async () => value };
            if (Date.now() >= deadline) throw new Error(`waitForFunction timed out: ${fn.toString()}`);
            await delay(100);
        }
    }
    waitForTimeout(ms: number) { return delay(ms); }
    screenshot(options: { path: string }) { return rpc({ op: 'screenshot', path: options.path }); }
    async close() { if (this.connected) { await rpc({ op: 'close' }); this.connected = false; } }
}

export function installAdapters() {
    const directory = import.meta.dir;
    plugin({ name: '274bot-upstream-e2e', setup(build) {
        build.onResolve({ filter: /(?:lib|tutorial)\/harness\.js$/ }, args => ({
            path: join(directory, args.path.includes('tutorial/') ? 'tutorial.ts' : 'harness.ts'),
        }));
    }});
}

export const outputDirectory = () => resolve(process.env.UEP_OUTPUT ?? '.');
