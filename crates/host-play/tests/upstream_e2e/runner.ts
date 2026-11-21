import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { installAdapters, rpc, UnsupportedCapabilityError } from './adapter.ts';

const PIN = '2de6bb2397ceb9ffd05f2354f67dde328e324531';
const ABI_PIN = 'ae56ccc95ba5681b12f1f66ec2adea14427fe497';
const repository = resolve(import.meta.dir, '../../../../');
const metadataPath = join(repository, 'tools/live-map/upstream-cases.json');
const root = JSON.parse(readFileSync(metadataPath, 'utf8')) as unknown;

type GateClass = 'gating' | 'non-gating';
type Eligibility = 'adopted' | 'excluded';
type Verdict = 'PASS' | 'FAIL' | 'UNSUPPORTED';
type Case = {
    id: string;
    path: string;
    family: string;
    capabilities: string[];
    deadline_seconds: number;
    gate_class: GateClass;
    eligibility: Eligibility;
    reason: string;
    family_paths: string[];
    sha256: string;
};
type Manifest = { pin: string; abi_pin: string; cases: Case[] };
type Outcome = { verdict: Verdict; reason: string; classification?: string };
type Result = Omit<Case, 'reason'> & {
    eligibility_reason: string;
    verdict: Verdict;
    reason: string;
    seconds: number;
    classification?: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
    return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
    return Array.isArray(value) && value.every(item => typeof item === 'string');
}

function isGateClass(value: unknown): value is GateClass {
    return value === 'gating' || value === 'non-gating';
}

function isEligibility(value: unknown): value is Eligibility {
    return value === 'adopted' || value === 'excluded';
}

function isVerdict(value: unknown): value is Verdict {
    return value === 'PASS' || value === 'FAIL' || value === 'UNSUPPORTED';
}

function parseCase(value: unknown): Case {
    if (!isRecord(value) ||
        typeof value.id !== 'string' ||
        typeof value.path !== 'string' ||
        typeof value.family !== 'string' ||
        !isStringArray(value.capabilities) ||
        typeof value.deadline_seconds !== 'number' ||
        !Number.isSafeInteger(value.deadline_seconds) || value.deadline_seconds <= 0 ||
        !isGateClass(value.gate_class) ||
        !isEligibility(value.eligibility) ||
        typeof value.reason !== 'string' ||
        typeof value.sha256 !== 'string' || !/^[0-9a-f]{64}$/i.test(value.sha256) ||
        !isStringArray(value.family_paths) || value.family_paths.length === 0) {
        throw new Error('invalid upstream case metadata');
    }
    const testCase: Case = {
        id: value.id,
        path: value.path,
        family: value.family,
        capabilities: value.capabilities,
        deadline_seconds: value.deadline_seconds,
        gate_class: value.gate_class,
        eligibility: value.eligibility,
        reason: value.reason,
        family_paths: value.family_paths,
        sha256: value.sha256,
    };
    if (testCase.id !== testCase.path.split('/').at(-1)?.replace(/\.ts$/, '') ||
        !testCase.path.startsWith('e2e/') ||
        (testCase.eligibility === 'excluded' && (testCase.reason === '' || testCase.gate_class !== 'non-gating'))) {
        throw new Error(`invalid upstream case contract: ${testCase.id}`);
    }
    return testCase;
}

function parseManifest(value: unknown): Manifest {
    if (!isRecord(value) || value.pin !== PIN || value.abi_pin !== ABI_PIN || !Array.isArray(value.cases)) {
        throw new Error('upstream case manifest pin or shape is invalid');
    }
    const cases = value.cases.map(parseCase);
    const ids = new Set(cases.map(testCase => testCase.id));
    if (cases.length !== 90 || ids.size !== cases.length) {
        throw new Error(`upstream manifest must contain 90 unique A/B cases; got ${cases.length}`);
    }
    return { pin: PIN, abi_pin: ABI_PIN, cases };
}

const manifest = parseManifest(root);
const upstreamValue = process.env.UEP_UPSTREAM;
const outputValue = process.env.UEP_OUTPUT;
if (!upstreamValue || !outputValue) throw new Error('missing UEP_UPSTREAM or UEP_OUTPUT');
const upstream = resolve(upstreamValue);
const output = resolve(outputValue);
const requested = (process.env.UEP_CASES ?? '').split(',').map(id => id.trim()).filter(Boolean);
if (new Set(requested).size !== requested.length) throw new Error('UEP_CASES must contain unique IDs');
const byId = new Map(manifest.cases.map(testCase => [testCase.id, testCase]));
for (const id of requested) {
    if (!byId.has(id)) throw new Error(`unknown upstream case ${id}`);
}
const cases = requested.length
    ? requested.map(id => byId.get(id)!)
    : manifest.cases.filter(testCase => testCase.eligibility === 'adopted');

function sourcePath(testCase: Case): string {
    const file = resolve(upstream, testCase.path);
    const pathFromRoot = relative(upstream, file);
    if (pathFromRoot.startsWith(`..${sep}`) || pathFromRoot === '..' || isAbsolute(pathFromRoot)) {
        throw new Error(`upstream path escapes source root: ${testCase.id}`);
    }
    return file;
}

function digest(bytes: Uint8Array): string {
    return createHash('sha256').update(bytes).digest('hex');
}

function messageOf(error: unknown): string {
    return (error instanceof Error ? error.message : String(error)).slice(0, 500);
}

function objectString(value: unknown, key: string): string | undefined {
    return isRecord(value) && typeof value[key] === 'string' ? value[key] : undefined;
}

function diagnosticMessage(value: unknown): string | undefined {
    const current = objectString(value, 'error');
    if (current) return current;
    const last = isRecord(value) ? value.last_issue : undefined;
    return objectString(last, 'error');
}

function childOutcome(path: string, exitCode: number, timedOut: boolean): Outcome {
    if (timedOut) {
        return { verdict: 'FAIL', reason: 'case deadline exceeded', classification: 'environment' };
    }
    if (!existsSync(path)) {
        return { verdict: 'FAIL', reason: `case process exited ${exitCode} without a result record`, classification: 'adapter defect' };
    }
    const value: unknown = JSON.parse(readFileSync(path, 'utf8'));
    if (!isRecord(value) ||
        !isVerdict(value.verdict) ||
        typeof value.reason !== 'string' ||
        (value.classification !== undefined && typeof value.classification !== 'string')) {
        throw new Error('invalid child result record');
    }
    const verdict = value.verdict;
    if (exitCode !== 0 && verdict === 'PASS') {
        return { verdict: 'FAIL', reason: `case process exited ${exitCode}`, classification: 'adapter defect' };
    }
    return {
        verdict,
        reason: value.reason,
        ...(typeof value.classification === 'string' ? { classification: value.classification } : {}),
    };
}

async function runChild(testCase: Case, file: string, directory: string, resultPath: string) {
    mkdirSync(directory, { recursive: true });
    if (existsSync(resultPath)) throw new Error(`stale child result exists for ${testCase.id}`);
    process.chdir(directory);
    process.env.UEP_CASE_ID = testCase.id;
    process.env.UEP_CASE_OUTPUT = directory;
    process.env.USER_NAME = `uep${Date.now().toString(36).slice(-7)}-${testCase.id}`;
    delete process.env.CLIENT_PAGE;
    const argv = [process.argv[0], file, '--base', 'http://127.0.0.1:1080'];
    if (testCase.capabilities.includes('upstream-no-deploy')) {
        argv.push('--no-deploy');
        process.env.CLIENT_PAGE = `/bot-${testCase.id}-${process.pid}.html`;
    }
    process.argv = argv;
    installAdapters();
    let outcome: Outcome;
    const exit = process.exit.bind(process);
    process.exit = code => {
        const status = code == null ? Number(process.exitCode ?? 0) : Number(code);
        const outcome: Outcome = status === 0
            ? { verdict: 'PASS', reason: 'upstream process exited 0' }
            : { verdict: 'FAIL', reason: `upstream process exited ${status}`, classification: 'compat defect' };
        writeFileSync(resultPath, `${JSON.stringify(outcome)}\n`);
        return exit(code);
    };
    try {
        // The import path is selected from the pinned runtime manifest; a static import cannot select one case.
        await import(pathToFileURL(file).href);
        outcome = { verdict: 'PASS', reason: 'upstream assertions completed' };
    } catch (error) {
        const reason = messageOf(error);
        if (error instanceof UnsupportedCapabilityError || reason.startsWith('UNSUPPORTED_CAPABILITY:')) {
            outcome = { verdict: 'UNSUPPORTED', reason };
        } else {
            outcome = {
                verdict: 'FAIL',
                reason,
                classification: reason.startsWith('FAIL:') ? 'compat defect' : 'adapter defect',
            };
        }
        console.error(reason);
    }
    writeFileSync(resultPath, `${JSON.stringify(outcome)}\n`);
    if (outcome.verdict !== 'PASS') process.exitCode = 1;
}

async function runCorpus() {
    mkdirSync(output, { recursive: true });
    const verdictPath = join(output, 'verdict.json');
    if (existsSync(verdictPath)) throw new Error('UEP_OUTPUT already contains verdict.json; use a new run directory');
    const revision = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], { cwd: repository, stdout: 'pipe', stderr: 'pipe' });
    const hostCommit = Buffer.from(revision.stdout).toString('utf8').trim();
    if (revision.exitCode !== 0 || !/^[0-9a-f]{40}$/i.test(hostCommit)) {
        throw new Error('could not resolve the full host commit');
    }
    const results: Result[] = [];
    let infrastructureFailure = false;
    let previousAccount: string | undefined;

    for (const testCase of cases) {
        const started = Date.now();
        const file = sourcePath(testCase);
        const source = readFileSync(file);
        const sha256 = digest(source);
        if (sha256 !== testCase.sha256) throw new Error(`pinned upstream source hash mismatch: ${testCase.id}`);
        let outcome: Outcome;

        if (testCase.eligibility === 'excluded') {
            outcome = { verdict: 'UNSUPPORTED', reason: testCase.reason };
        } else {
            const directory = join(output, testCase.id);
            const resultPath = join(directory, '.runner-result.json');
            const child = Bun.spawn(['bun', import.meta.path, '--case', testCase.id], {
                cwd: repository,
                stdout: 'pipe',
                stderr: 'pipe',
                env: {
                    ...process.env,
                    UEP_CASE_OUTPUT: directory,
                    UEP_CASE_ID: testCase.id,
                    UEP_CASES: testCase.id,
                },
            });
            let timedOut = false;
            const timer = setTimeout(() => {
                timedOut = true;
                child.kill();
            }, testCase.deadline_seconds * 1000);
            const [exitCode, stdout, stderr] = await Promise.all([
                child.exited,
                new Response(child.stdout).text(),
                new Response(child.stderr).text(),
            ]);
            clearTimeout(timer);

            let diagnostics: unknown;
            let diagnosticError: string | undefined;
            try {
                diagnostics = await rpc({ op: 'diagnostics' });
            } catch (error) {
                diagnosticError = messageOf(error);
                infrastructureFailure = true;
            }
            const account = objectString(diagnostics, 'account');
            const freshDiagnostics = account && account !== previousAccount ? diagnostics : undefined;
            if (account) previousAccount = account;
            try {
                await rpc({ op: 'close' });
            } catch (error) {
                diagnosticError = [diagnosticError, messageOf(error)].filter(Boolean).join('; ');
                infrastructureFailure = true;
            }

            writeFileSync(join(output, `${testCase.id}.log`), stdout + stderr);
            outcome = childOutcome(resultPath, exitCode, timedOut);
            if (diagnosticError) {
                outcome = {
                    verdict: 'FAIL',
                    reason: `native diagnostics/cleanup failed: ${diagnosticError}`,
                    classification: 'environment',
                };
            } else if (outcome.verdict !== 'PASS') {
                const hostError = diagnosticMessage(freshDiagnostics);
                if (hostError && outcome.reason.length < 80) outcome.reason = `${outcome.reason}; host: ${hostError}`.slice(0, 500);
                if (hostError && outcome.verdict === 'FAIL') outcome.classification = 'compat defect';
            }
        }

        const { reason: eligibilityReason, ...metadata } = testCase;
        const result: Result = {
            ...metadata,
            eligibility_reason: eligibilityReason,
            verdict: outcome.verdict,
            reason: outcome.reason,
            seconds: Math.round((Date.now() - started) / 1000 * 1000) / 1000,
            sha256,
            ...(outcome.classification ? { classification: outcome.classification } : {}),
        };
        results.push(result);
        console.log(JSON.stringify(result));
    }

    const verdict = {
        pin: manifest.pin,
        abi_pin: manifest.abi_pin,
        host_commit: hostCommit,
        results,
    };
    const temporary = join(output, `.verdict-${process.pid}.tmp`);
    writeFileSync(temporary, `${JSON.stringify(verdict, null, 2)}\n`);
    renameSync(temporary, verdictPath);

    if (infrastructureFailure || results.some(result => result.gate_class === 'gating' && result.verdict !== 'PASS')) {
        process.exitCode = 1;
    }
}

const caseIndex = process.argv.indexOf('--case');
if (caseIndex >= 0) {
    const id = process.argv[caseIndex + 1];
    const testCase = byId.get(id);
    if (!testCase || !cases.some(selected => selected.id === id) || testCase.eligibility !== 'adopted') {
        throw new Error(`unknown or excluded upstream case ${id}`);
    }
    await runChild(testCase, sourcePath(testCase), resolve(output, testCase.id), join(resolve(output, testCase.id), '.runner-result.json'));
} else {
    if (!cases.length) throw new Error('no upstream cases selected');
    await runCorpus();
}
