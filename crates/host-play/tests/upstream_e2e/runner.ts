import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { installAdapters, rpc } from './adapter.ts';

const allCases = [
    'alcher-nearest-bank-live', 'flour-collector-310-live',
    'firemaker-empty-bank-live', 'herbcleaner-empty-bank-live',
    'potionmaker-empty-secondary-live',
];
const cases = process.env.UEP_CASES?.split(',') ?? allCases;
if (!cases.length || new Set(cases).size !== cases.length || cases.some(name => !allCases.includes(name))) {
    throw new Error('UEP_CASES must select unique known pilot cases');
}
const upstream = process.env.UEP_UPSTREAM!;
const output = process.env.UEP_OUTPUT!;
const selected = process.argv.indexOf('--case');
if (selected >= 0) {
    const name = process.argv[selected + 1];
    if (!cases.includes(name)) throw new Error(`unknown pilot case ${name}`);
    const file = join(upstream, 'e2e', `${name}.ts`);
    const directory = join(output, name);
    mkdirSync(directory, { recursive: true });
    process.chdir(directory);
    process.env.USER_NAME = `uep${Date.now().toString(36).slice(-7)}`;
    process.argv = [process.argv[0], file, 'http://127.0.0.1:1080'];
    installAdapters();
    // The test file is selected at runtime, after the adapter installs its import bindings.
    await import(pathToFileURL(file).href);
} else {
    const results = [];
    for (const name of cases) {
        const file = join(upstream, 'e2e', `${name}.ts`);
        const source = readFileSync(file);
        const started = Date.now();
        const process = Bun.spawn(['bun', import.meta.path, '--case', name], {
            stdout: 'pipe', stderr: 'pipe', env: { ...globalThis.process.env },
        });
        const [code, stdout, stderr] = await Promise.all([
            process.exited, new Response(process.stdout).text(), new Response(process.stderr).text(),
        ]);
        await rpc({ op: 'close' });
        writeFileSync(join(output, `${name}.log`), stdout + stderr);
        const result = { test: `e2e/${name}.ts`, sha256: createHash('sha256').update(source).digest('hex'),
            exit: code, verdict: code === 0 ? 'PASS' : 'FAIL', seconds: (Date.now() - started) / 1000 };
        results.push(result);
        console.log(JSON.stringify(result));
    }
    writeFileSync(join(output, 'pilot-results.json'), JSON.stringify(results, null, 2) + '\n');
    // Assertion failures remain data for this pilot. The report quotes each verdict.
    if (results.some(row => row.exit !== 0)) globalThis.process.exitCode = 1;
}
