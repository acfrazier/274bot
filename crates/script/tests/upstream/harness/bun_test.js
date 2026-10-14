const ASYMMETRIC = Symbol.for('compat-refreeze.bun-test.asymmetric');
const cases = [];
const root = { name: '', before: [], after: [] };
const suites = [root];

function failure(message) {
    throw new Error(message);
}

function show(value) {
    try {
        const json = JSON.stringify(value);
        return json === undefined ? String(value) : json;
    } catch {
        return String(value);
    }
}

function asymmetric(test) {
    return { [ASYMMETRIC]: test };
}

function equal(actual, expected, seen = new WeakMap()) {
    if (expected && expected[ASYMMETRIC]) return expected[ASYMMETRIC](actual);
    if (Object.is(actual, expected)) return true;
    if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;
    if (seen.get(actual) === expected) return true;
    seen.set(actual, expected);
    if (Array.isArray(expected)) {
        return Array.isArray(actual) && actual.length === expected.length && expected.every((v, i) => equal(actual[i], v, seen));
    }
    if (expected instanceof RegExp) return actual instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;
    if (expected instanceof Date) return actual instanceof Date && actual.getTime() === expected.getTime();
    if (expected instanceof Set) {
        if (!(actual instanceof Set) || actual.size !== expected.size) return false;
        const remaining = [...actual];
        return [...expected].every((v) => {
            const index = remaining.findIndex((candidate) => equal(candidate, v, seen));
            if (index < 0) return false;
            remaining.splice(index, 1);
            return true;
        });
    }
    if (expected instanceof Map) {
        if (!(actual instanceof Map) || actual.size !== expected.size) return false;
        for (const [key, value] of expected) {
            if (!actual.has(key) || !equal(actual.get(key), value, seen)) return false;
        }
        return true;
    }
    const keys = Object.keys(expected);
    return keys.length === Object.keys(actual).length && keys.every((key) => Object.hasOwn(actual, key) && equal(actual[key], expected[key], seen));
}

function matches(actual, expected, seen = new WeakMap()) {
    if (expected && expected[ASYMMETRIC]) return expected[ASYMMETRIC](actual);
    if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return Object.is(actual, expected);
    if (Array.isArray(expected)) return Array.isArray(actual) && actual.length >= expected.length && expected.every((v, i) => matches(actual[i], v, seen));
    return Object.keys(expected).every((key) => Object.hasOwn(actual, key) && matches(actual[key], expected[key], seen));
}

function makeMatchers(actual, negate, hint) {
    const assert = (ok, label, expected) => {
        if (negate ? ok : !ok) failure(`${hint ? `${hint}: ` : ''}expected ${show(actual)} ${negate ? 'not ' : ''}${label}${expected === undefined ? '' : ` ${show(expected)}`}`);
    };
    const out = {
        get not() { return makeMatchers(actual, !negate, hint); },
        toBe(expected) { assert(Object.is(actual, expected), 'to be', expected); },
        toEqual(expected) { assert(equal(actual, expected), 'to equal', expected); },
        toStrictEqual(expected) { assert(equal(actual, expected), 'to strictly equal', expected); },
        toMatchObject(expected) { assert(matches(actual, expected), 'to match object', expected); },
        toContain(expected) {
            const ok = typeof actual === 'string'
                ? actual.includes(expected)
                : actual instanceof Set
                    ? [...actual].some((value) => equal(value, expected))
                    : Array.isArray(actual) && actual.some((value) => equal(value, expected));
            assert(ok, 'to contain', expected);
        },
        toHaveLength(expected) { assert(actual != null && actual.length === expected, 'to have length', expected); },
        toBeNull() { assert(actual === null, 'to be null'); },
        toBeUndefined() { assert(actual === undefined, 'to be undefined'); },
        toBeDefined() { assert(actual !== undefined, 'to be defined'); },
        toBeTruthy() { assert(Boolean(actual), 'to be truthy'); },
        toBeFalsy() { assert(!actual, 'to be falsy'); },
        toBeNaN() { assert(Number.isNaN(actual), 'to be NaN'); },
        toBeInstanceOf(expected) { assert(actual instanceof expected, 'to be instance of', expected?.name); },
        toBeGreaterThan(expected) { assert(actual > expected, 'to be greater than', expected); },
        toBeGreaterThanOrEqual(expected) { assert(actual >= expected, 'to be greater than or equal to', expected); },
        toBeLessThan(expected) { assert(actual < expected, 'to be less than', expected); },
        toBeLessThanOrEqual(expected) { assert(actual <= expected, 'to be less than or equal to', expected); },
        toBeCloseTo(expected, digits = 2) { assert(Math.abs(actual - expected) < 0.5 * 10 ** -digits, 'to be close to', expected); },
        toMatch(expected) { assert((expected instanceof RegExp ? expected.test(actual) : String(actual).includes(String(expected))), 'to match', expected); },
        toHaveProperty(path, expected) {
            const keys = Array.isArray(path) ? path : String(path).split('.');
            let value = actual;
            for (const key of keys) value = value == null ? undefined : value[key];
            const exists = value !== undefined;
            assert(exists && (arguments.length < 2 || equal(value, expected)), 'to have property', path);
        },
        toThrow(expected) {
            if (typeof actual !== 'function') failure('toThrow requires a function');
            let thrown;
            try { actual(); } catch (error) { thrown = error; }
            let ok = thrown !== undefined;
            if (ok && expected !== undefined) {
                const text = String(thrown?.message ?? thrown);
                ok = expected instanceof RegExp ? expected.test(text)
                    : typeof expected === 'string' ? text.includes(expected)
                    : typeof expected === 'function' ? thrown instanceof expected
                    : false;
            }
            assert(ok, 'to throw', expected);
        },
        get resolves() { return promiseMatchers(actual, 'resolve', negate, hint); },
        get rejects() { return promiseMatchers(actual, 'reject', negate, hint); },
    };
    return out;
}

function promiseMatchers(value, mode, negate, hint) {
    const check = async (method, expected) => {
        let result;
        let error;
        try { result = await value; } catch (caught) { error = caught; }
        if (mode === 'resolve') {
            if (error) failure(`${hint ? `${hint}: ` : ''}expected promise to resolve, got ${String(error?.message ?? error)}`);
            return makeMatchers(result, negate, hint)[method](expected);
        }
        if (!error) failure(`${hint ? `${hint}: ` : ''}expected promise to reject`);
        if (method === 'toThrow') {
            let ok = expected === undefined || (expected instanceof RegExp ? expected.test(String(error?.message ?? error)) : String(error?.message ?? error).includes(String(expected)));
            if (negate) ok = !ok;
            if (!ok) failure(`${hint ? `${hint}: ` : ''}expected rejected promise ${negate ? 'not ' : ''}to throw ${show(expected)}`);
            return;
        }
        return makeMatchers(error, negate, hint)[method](expected);
    };
    return {
        toBe: (expected) => check('toBe', expected),
        toEqual: (expected) => check('toEqual', expected),
        toThrow: (expected) => check('toThrow', expected),
    };
}

export function expect(actual, hint) {
    return makeMatchers(actual, false, hint);
}

expect.any = (ctor) => asymmetric((value) => ctor === String ? typeof value === 'string' : ctor === Number ? typeof value === 'number' : ctor === Boolean ? typeof value === 'boolean' : value instanceof ctor);
expect.anything = () => asymmetric((value) => value !== null && value !== undefined);
expect.objectContaining = (value) => asymmetric((actual) => matches(actual, value));
expect.arrayContaining = (values) => asymmetric((actual) => Array.isArray(actual) && values.every((value) => actual.some((candidate) => equal(candidate, value))));
expect.stringContaining = (value) => asymmetric((actual) => typeof actual === 'string' && actual.includes(value));

export function describe(name, body) {
    const suite = { name: String(name), before: [], after: [] };
    suites.push(suite);
    try { body(); } finally { suites.pop(); }
}

export function test(name, body) {
    cases.push({
        name: [...suites.map((suite) => suite.name), String(name)].join(' > '),
        body,
        suites: suites.slice(),
    });
}

export const it = test;
export function beforeEach(body) { suites[suites.length - 1].before.push(body); }
export function afterEach(body) { suites[suites.length - 1].after.push(body); }

export async function runTests() {
    const results = [];
    for (const item of cases) {
        const errors = [];
        try {
            for (const suite of item.suites) for (const hook of suite.before) await hook();
            await item.body();
        } catch (error) {
            errors.push(String(error?.stack ?? error?.message ?? error));
        } finally {
            for (const suite of [...item.suites].reverse()) {
                for (const hook of [...suite.after].reverse()) {
                    try { await hook(); } catch (error) { errors.push(String(error?.stack ?? error?.message ?? error)); }
                }
            }
        }
        results.push({ name: item.name, status: errors.length ? 'fail' : 'pass', error: errors.join('\n') });
    }
    return results;
}
