import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { assertPrereqPreservation } from './prereq-preservation.ts';
import type { PrereqJournalWire, PrereqRowWire } from './prereq-steps.ts';

function row(proves: [number, number], after: [number, number]): PrereqRowWire {
    return {
        id: 'fixture:step',
        crossings: [],
        establishes: { quest: 'fixture', signal: 'progress', values: { min: 0, max: 0 } },
        after: [{ proves: { min: proves[0], max: proves[1] }, after: { min: after[0], max: after[1] } }],
        reviewed_writes: [],
        probe: { refuse: [] },
        target: { npc: 0, op: 1, anchor: { x: 0, z: 0, level: 0 }, leash: 0, source: '' },
        preconditions: [],
        talk: { strict: true, prefer: [], line_rules: [], branches: [] },
        memory: 'none',
        consent: 'prereq_dialogue',
    };
}

function withContent(files: Record<string, string>, run: (content: string) => void) {
    const content = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-preservation-'));
    try {
        for (const [relative, text] of Object.entries(files)) {
            const filename = path.join(content, relative);
            fs.mkdirSync(path.dirname(filename), { recursive: true });
            fs.writeFileSync(filename, text);
        }
        run(content);
    } finally {
        fs.rmSync(content, { recursive: true, force: true });
    }
}

const constants = 'scripts/test.constant';
withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/calls.rs2': [
        '[oploc1,root]',
        '@direct_target;',
        'gosub(gosub_target);',
        '~proc_target;',
        'queue(queue_target, 0, 0);',
        'weakqueue(weak_target, 0);',
        'strongqueue(strong_target, 0);',
        'longqueue(long_target, 0);',
        'npc_queue(test_npc);',
        'settimer(timer_target, 0);',
        'softtimer(softtimer_target, 0);',
        '[label,direct_target] %progress = ^one;',
        '[label,gosub_target] %progress = ^one;',
        '[proc,proc_target] %progress = ^one;',
        '[queue,queue_target] %progress = ^one;',
        '[queue,weak_target] %progress = ^one;',
        '[queue,strong_target] %progress = ^one;',
        '[queue,long_target] %progress = ^one;',
        '[ai_queue1,npc_target] %progress = ^one;',
        '[timer,timer_target] %progress = ^one;',
        '[softtimer,softtimer_target] %progress = ^one;',
    ].join('\n'),
}, (content) => assertPrereqPreservation(content, [row([0, 0], [0, 1])]));

withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/switch.rs2': [
        '[oploc1,root] @choose;',
        '[label,choose]',
        'switch_int(%progress) {',
        'case ^zero:',
        '    @write_zero;',
        'case ^one:',
        '    @write_one;',
        '}',
        '[label,write_zero] %progress = ^zero;',
        '[label,write_one] %progress = ^one;',
    ].join('\n'),
}, (content) => assertPrereqPreservation(content, [row([1, 1], [1, 1])]));

withContent({
    [constants]: '^one = 1\n',
    'scripts/early-return.rs2': [
        '[oploc1,root] @write;',
        '[label,write]',
        'if (%progress >= ^one) {',
        '    return;',
        '}',
        '%progress = 0;',
    ].join('\n'),
}, (content) => assertPrereqPreservation(content, [row([1, 1], [1, 1])]));

withContent({
    [constants]: '^one = 1\n',
    'scripts/bounded-increment.rs2': [
        '[oploc1,root] @increment;',
        '[label,increment]',
        'if (%progress >= ^one) {',
        '    return;',
        '}',
        '%progress = add(%progress, 1);',
    ].join('\n'),
}, (content) => assertPrereqPreservation(content, [row([0, 0], [0, 1])]));

withContent({
    [constants]: '^one = 1\n',
    'scripts/uncalled.rs2': '[label,orphan] %progress = ^one;\n',
}, (content) => assert.throws(
    () => assertPrereqPreservation(content, [row([0, 0], [0, 1])]),
    /writer path set is empty/,
));

withContent({
    [constants]: '^one = 1\n',
    'scripts/unbounded-increment.rs2': [
        '[oploc1,root] @increment;',
        '[label,increment] %progress = add(%progress, 1);',
    ].join('\n'),
}, (content) => assert.throws(
    () => assertPrereqPreservation(content, [row([0, 0], [0, 1])]),
    /unbounded add/,
));

withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/dialogue-arguments.rs2': [
        '[opnpc1,root]',
        'if (%progress = ^zero) {',
        '    @multi2("Sure.", write, "No.", decline);',
        '}',
        '[label,multi2](string $a, label $yes, string $b, label $no)',
        'return;',
        '[label,write] %progress = ^one;',
        '[label,decline] return;',
    ].join('\n'),
}, (content) => {
    assertPrereqPreservation(content, [row([0, 0], [0, 1])]);
    assertPrereqPreservation(content, [row([1, 1], [1, 1])]);
});

withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/nested-return.rs2': [
        '[oplocu,root]',
        'if (last_useitem = key) {',
        '    if (npc_find(coord, monk, 15, 0) = true) {',
        '        if (%progress >= ^one) { return; }',
        '        %progress = ^zero;',
        '    }',
        '}',
    ].join('\n'),
}, (content) => assertPrereqPreservation(content, [row([1, 1], [1, 1])]));

withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/return-scope.rs2': [
        '[oplocu,root]',
        'if (last_useitem = key) {',
        '    if (%progress >= ^one) { return; }',
        '}',
        '%progress = ^zero;',
    ].join('\n'),
}, (content) => assert.throws(
    () => assertPrereqPreservation(content, [row([1, 1], [1, 1])]),
    /after \[1,1\] is \[0,1\]/,
));

function journal(proves: [number, number], after: [number, number]): PrereqJournalWire {
    return {
        id: 'fixture:journal',
        quest: 'fixture',
        establishes: {
            quest: 'fixture',
            signal: 'progress',
            values: { min: after[0], max: after[1] },
        },
        after: [{ proves: { min: proves[0], max: proves[1] }, after: { min: after[0], max: after[1] } }],
        reviewed_writes: [],
        rules: [{ all: ['journal text'], proves: { min: proves[0], max: proves[1] }, source: 'scripts/journal.rs2:1' }],
        source: 'scripts/journal.rs2:1',
    };
}

withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/journal.rs2': '[oploc1,root] @write;\n[label,write] %progress = ^one;\n',
}, (content) => {
    const fact = journal([0, 0], [0, 1]);
    assertPrereqPreservation(content, [], undefined, [fact]);
    const stage = { id: 'fixture', colour_authoritative: false, transmission: 'not_transmitted', stage_domain: { min: 0, max: 1 } };
    assertPrereqPreservation(content, [], [stage], [fact]);
    assert.throws(() => assertPrereqPreservation(
        content,
        [],
        [{ ...stage, stage_domain: { min: 0, max: 0 } }],
        [fact],
    ), /leaves stage domain/);
});

withContent({
    [constants]: '^zero = 0\n^one = 1\n',
    'scripts/condition-call.rs2': [
        '[oploc1,root]',
        'if (~update = true) { return; }',
        '[proc,update]()(boolean)',
        '%progress = ^one;',
        'return(true);',
    ].join('\n'),
}, (content) => {
    assertPrereqPreservation(content, [], undefined, [journal([0, 0], [0, 1])]);
    assert.throws(
        () => assertPrereqPreservation(content, [], undefined, [journal([0, 0], [0, 0])]),
        /after \[0,0\] is \[0,1\]/,
    );
});

