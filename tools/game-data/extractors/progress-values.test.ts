import assert from 'node:assert/strict';
import { parseRs2Blocks, type Rs2Block } from './gathering-content.ts';
import { createProgressValueResolver, type Guard } from './progress-values.ts';
import { parseProgressStatements } from './prereq-preservation.ts';

type Statement = { text: string; line: number; guards: readonly Guard[] };
type GuardFor = (block: Rs2Block, text: string) => readonly Guard[];

function stripStrings(text: string) {
    let inString = false;
    let output = '';
    for (const char of text) {
        if (char === '"') {
            inString = !inString;
            output += ' ';
        } else output += inString ? ' ' : char;
    }
    return output;
}

function argumentsOf(text: string, open: number) {
    const clean = stripStrings(text);
    let depth = 0;
    let close = -1;
    for (let index = open; index < clean.length; index += 1) {
        if (clean[index] === '(') depth += 1;
        else if (clean[index] === ')' && --depth === 0) {
            close = index;
            break;
        }
    }
    if (close < 0) return [];
    const argumentsText = text.slice(open + 1, close);
    const cleanArguments = clean.slice(open + 1, close);
    const values: string[] = [];
    let nested = 0;
    let start = 0;
    for (let index = 0; index < cleanArguments.length; index += 1) {
        if (cleanArguments[index] === '(') nested += 1;
        else if (cleanArguments[index] === ')') nested -= 1;
        else if (cleanArguments[index] === ',' && nested === 0) {
            values.push(argumentsText.slice(start, index).trim());
            start = index + 1;
        }
    }
    const last = argumentsText.slice(start).trim();
    if (last || values.length > 0) values.push(last);
    return values;
}

function prepare(source: string, guardFor: GuardFor = () => [], constants = new Map<string, number>()) {
    const blocks = parseRs2Blocks('selector-test.rs2', source);
    const statementsFor = (block: Rs2Block): readonly Statement[] =>
        parseProgressStatements(block, '', constants).map((statement) => ({
            ...statement,
            guards: [...statement.guards, ...guardFor(block, statement.text)],
        }));
    const resolve = createProgressValueResolver(blocks, constants, statementsFor, argumentsOf, stripStrings);
    return { blocks, resolve };
}

function blockNamed(blocks: readonly Rs2Block[], kind: string, name: string) {
    const block = blocks.find((candidate) => candidate.kind === kind && candidate.name === name);
    assert.ok(block, `missing [${kind},${name}] test block`);
    return block;
}

function lineMatching(block: Rs2Block, predicate: (text: string) => boolean) {
    const row = block.body.find(({ text }) => predicate(text));
    assert.ok(row, `missing matching line in ${block.name}`);
    return row.line;
}

function hasGuard(paths: readonly (readonly Guard[])[], expression: string, polarity: boolean) {
    return paths.every((path) => path.some((guard) => guard.expression === expression && guard.polarity === polarity));
}

const menuSource = `[proc,choose_from_menu](string $firstText, int $firstValue, string $secondText, int $secondValue, string $fallbackText, int $fallbackValue)(int)
switch_component(last_component) {
    case panel:button_1 : return($firstValue);
    case panel:button_2 : return($secondValue);
    case default : return($fallbackValue);
}

[opnpc1,conversation]
def_int $selection;
if (%progress = ^progress_marker) {
    $selection = ~choose_from_menu("Special, option", 7, "Ordinary", 1, "Fallback", 2);
} else {
    $selection = ~choose_from_menu("One", 1, "Two", 2, "Three", 3);
}
switch_int($selection) {
    case 7 : return;
    case default : return;
}
`;

const menuResolver = prepare(menuSource, (block, text) => {
    if (block.kind === 'proc' && text.includes('return($firstValue)')) return [{ expression: 'last_component = panel:button_1', polarity: true }];
    if (block.kind === 'proc' && text.includes('return($secondValue)')) return [{ expression: 'last_component = panel:button_2', polarity: true }];
    if (block.kind === 'proc' && text.includes('return($fallbackValue)')) {
        return [{ expression: 'last_component != panel:button_1 & last_component != panel:button_2', polarity: true }];
    }
    if (block.name === 'conversation' && text.includes('~choose_from_menu("Special')) return [{ expression: '%progress = ^progress_marker', polarity: true }];
    if (block.name === 'conversation' && text.includes('~choose_from_menu("One"')) return [{ expression: '%progress = ^progress_marker', polarity: false }];
    return [];
});
const conversation = blockNamed(menuResolver.blocks, 'opnpc1', 'conversation');
const menuSwitchLine = lineMatching(conversation, (text) => text.startsWith('switch_int($selection)'));
const specialMenuPaths = menuResolver.resolve(conversation, menuSwitchLine, '$selection', [7], false);
assert.ok(specialMenuPaths);
assert.ok(specialMenuPaths.length > 0);
assert.ok(hasGuard(specialMenuPaths, '%progress = ^progress_marker', true));
assert.ok(specialMenuPaths.every((path) => !path.some((guard) => guard.expression === '%progress = ^progress_marker' && !guard.polarity)));

const ordinaryMenuPaths = menuResolver.resolve(conversation, menuSwitchLine, '$selection', [7], true);
assert.ok(ordinaryMenuPaths);
assert.ok(ordinaryMenuPaths.length > 0);
assert.ok(ordinaryMenuPaths.every((path) => path.some((guard) => guard.expression.startsWith('last_component'))));

const defaultSource = `[proc,return_selected_value](int $fallback)(int)
switch_component(interface:button_1) {
    case interface:button_1 : return(4);
    case default : return($fallback);
}
[opnpc1,menu_owner]
def_int $selection = ~return_selected_value(9);
switch_int($selection) {
    case 9 : return;
}
`;
const defaultResolver = prepare(defaultSource, (block, text) => {
    if (block.kind === 'proc' && text.includes('return(4)')) return [{ expression: 'selected_component = interface:button_1', polarity: true }];
    if (block.kind === 'proc' && text.includes('return($fallback)')) return [{ expression: 'selected_component != interface:button_1', polarity: true }];
    return [];
});
const defaultOwner = blockNamed(defaultResolver.blocks, 'opnpc1', 'menu_owner');
const defaultSwitchLine = lineMatching(defaultOwner, (text) => text.startsWith('switch_int($selection)'));
const defaultReturnPaths = defaultResolver.resolve(defaultOwner, defaultSwitchLine, '$selection', [9], false);
assert.deepEqual(defaultReturnPaths, [[{ expression: 'selected_component != interface:button_1', polarity: true }]]);

const labelSource = `[label,receive_choice](int $selection)
switch_int($selection) {
    case 5 : return;
}
[opnpc1,dispatcher]
if (%phase = ^phase_gate) {
    @receive_choice(5);
} else {
    @receive_choice(2);
}
`;
const labelResolver = prepare(labelSource, (_block, text) => {
    if (text.includes('@receive_choice(5)')) return [{ expression: '%phase = ^phase_gate', polarity: true }];
    if (text.includes('@receive_choice(2)')) return [{ expression: '%phase = ^phase_gate', polarity: false }];
    return [];
});
const receiver = blockNamed(labelResolver.blocks, 'label', 'receive_choice');
const receiverSwitchLine = lineMatching(receiver, (text) => text.startsWith('switch_int($selection)'));
const receivedFive = labelResolver.resolve(receiver, receiverSwitchLine, '$selection', [5], false);
assert.ok(receivedFive);
assert.deepEqual(receivedFive, [[{ expression: '%phase = ^phase_gate', polarity: true }]]);

const uninitializedSource = `[opnpc1,partial_menu]
def_int $selection;
if (%enabled = ^enabled_marker) {
    $selection = 1;
}
switch_int($selection) {
    case 1 : return;
}
`;
const uninitializedResolver = prepare(uninitializedSource, (_block, text) => text.includes('$selection = 1')
    ? [{ expression: '%enabled = ^enabled_marker', polarity: true }]
    : []);
const partialMenu = blockNamed(uninitializedResolver.blocks, 'opnpc1', 'partial_menu');
const partialSwitchLine = lineMatching(partialMenu, (text) => text.startsWith('switch_int($selection)'));
const possibleOne = uninitializedResolver.resolve(partialMenu, partialSwitchLine, '$selection', [1], false);
assert.ok(possibleOne);
assert.equal(possibleOne.length, 2);
assert.ok(possibleOne.some((path) => path.some((guard) => guard.expression === '%enabled = ^enabled_marker' && !guard.polarity)));

const constantSource = `[opnpc1,constant_menu]
def_int $selection = ^selected_value;
switch_int($selection) {
    case 11 : return;
}
`;
const constantResolver = prepare(constantSource, () => [], new Map([['selected_value', 11]]));
const constantMenu = blockNamed(constantResolver.blocks, 'opnpc1', 'constant_menu');
const constantSwitchLine = lineMatching(constantMenu, (text) => text.startsWith('switch_int($selection)'));
assert.deepEqual(constantResolver.resolve(constantMenu, constantSwitchLine, '$selection', [11], false), [[]]);

const dynamicLabelSource = `[label,receive_choice](int $selection)
switch_int($selection) {
    case 5 : return;
}
[opnpc1,dispatcher]
@receive_choice($runtime_value);
@receive_choice;
`;
const dynamicLabelResolver = prepare(dynamicLabelSource);
const dynamicReceiver = blockNamed(dynamicLabelResolver.blocks, 'label', 'receive_choice');
const dynamicSwitchLine = lineMatching(dynamicReceiver, (text) => text.startsWith('switch_int($selection)'));
const possibleDynamic = dynamicLabelResolver.resolve(dynamicReceiver, dynamicSwitchLine, '$selection', [5], false);
assert.ok(possibleDynamic === null || possibleDynamic.length > 0);
assert.ok(possibleDynamic === null || possibleDynamic.some((path) => path.length === 0));

const recursiveSource = `[proc,recursive_pick](int $value)(int)
return(~recursive_pick($value));
[opnpc1,recursive_caller]
def_int $selection = ~recursive_pick(5);
switch_int($selection) {
    case 5 : return;
}
`;
const recursiveResolver = prepare(recursiveSource);
const recursiveCaller = blockNamed(recursiveResolver.blocks, 'opnpc1', 'recursive_caller');
const recursiveSwitchLine = lineMatching(recursiveCaller, (text) => text.startsWith('switch_int($selection)'));
const possibleRecursive = recursiveResolver.resolve(recursiveCaller, recursiveSwitchLine, '$selection', [5], false);
assert.ok(possibleRecursive === null || possibleRecursive.length > 0);

const guardedReturnSource = `[proc,choose_from_phase](int $provided)(int)
if (%phase = ^phase_gate) {
    return($provided);
}
return(3);
[opnpc1,phase_caller]
def_int $selection = ~choose_from_phase(6);
switch_int($selection) {
    case 6 : return;
}
`;
const guardedReturnResolver = prepare(guardedReturnSource, (_block, text) => {
    if (text.includes('return($provided)')) return [{ expression: '%phase = ^phase_gate', polarity: true }];
    if (text.includes('return(3)')) return [{ expression: '%phase = ^phase_gate', polarity: false }];
    return [];
});
const phaseCaller = blockNamed(guardedReturnResolver.blocks, 'opnpc1', 'phase_caller');
const phaseSwitchLine = lineMatching(phaseCaller, (text) => text.startsWith('switch_int($selection)'));
assert.deepEqual(
    guardedReturnResolver.resolve(phaseCaller, phaseSwitchLine, '$selection', [6], false),
    [[{ expression: '%phase = ^phase_gate', polarity: true }]],
);

const incompleteReturnSource = `[proc,maybe_value](int $provided)(int)
if (%phase = ^phase_gate) {
    return($provided);
}
[opnpc1,incomplete_caller]
def_int $selection = ~maybe_value(6);
switch_int($selection) {
    case 6 : return;
}
`;
const incompleteReturnResolver = prepare(incompleteReturnSource, (_block, text) => text.includes('return($provided)')
    ? [{ expression: '%phase = ^phase_gate', polarity: true }]
    : []);
const incompleteCaller = blockNamed(incompleteReturnResolver.blocks, 'opnpc1', 'incomplete_caller');
const incompleteSwitchLine = lineMatching(incompleteCaller, (text) => text.startsWith('switch_int($selection)'));
const possibleIncomplete = incompleteReturnResolver.resolve(incompleteCaller, incompleteSwitchLine, '$selection', [6], false);
assert.ok(possibleIncomplete);
assert.ok(possibleIncomplete.some((path) => path.some((guard) => guard.expression === '%phase = ^phase_gate' && !guard.polarity)));

const missingSourceCall = `[opnpc1,missing_caller]
def_int $selection = ~unknown_source(12);
switch_int($selection) {
    case 12 : return;
}
`;
const missingSourceResolver = prepare(missingSourceCall);
const missingCaller = blockNamed(missingSourceResolver.blocks, 'opnpc1', 'missing_caller');
const missingSwitchLine = lineMatching(missingCaller, (text) => text.startsWith('switch_int($selection)'));
const possibleMissing = missingSourceResolver.resolve(missingCaller, missingSwitchLine, '$selection', [12], false);
assert.ok(possibleMissing === null || possibleMissing.length > 0);

const zeroArgumentSource = `[proc,constant_choice]()(int)
return(12);
[opnpc1,zero_argument_caller]
switch_int(~constant_choice) {
    case 12 : return;
}
`;
const zeroArgumentResolver = prepare(zeroArgumentSource);
const zeroArgumentCaller = blockNamed(zeroArgumentResolver.blocks, 'opnpc1', 'zero_argument_caller');
const zeroArgumentSwitchLine = lineMatching(zeroArgumentCaller, (text) => text.startsWith('switch_int(~constant_choice)'));
assert.deepEqual(zeroArgumentResolver.resolve(zeroArgumentCaller, zeroArgumentSwitchLine, '~constant_choice', [12], false), [[]]);

const indirectLabelSource = `[label,receive_choice](int $selection)
switch_int($selection) {
    case 5 : return;
}
[label,route_handler](label $destination)
jump($destination);
[opnpc1,indirect_dispatcher]
@receive_choice(2);
@route_handler(receive_choice);
`;
const indirectLabelResolver = prepare(indirectLabelSource);
const indirectReceiver = blockNamed(indirectLabelResolver.blocks, 'label', 'receive_choice');
const indirectSwitchLine = lineMatching(indirectReceiver, (text) => text.startsWith('switch_int($selection)'));
const possibleIndirect = indirectLabelResolver.resolve(indirectReceiver, indirectSwitchLine, '$selection', [5], false);
assert.ok(possibleIndirect);
assert.ok(possibleIndirect.length > 0);

for (const update of ['$selection += 1;', '$selection++;', '++$selection;']) {
    const changed = prepare(`[opnpc1,changed_local]
def_int $selection = 1;
${update}
switch_int($selection) {
    case 2: return;
}
`);
    const block = blockNamed(changed.blocks, 'opnpc1', 'changed_local');
    const line = lineMatching(block, (text) => text.startsWith('switch_int($selection)'));
    const possible = changed.resolve(block, line, '$selection', [2], false);
    assert.ok(possible === null || possible.length > 0);
}

{
    const changed = prepare(`[opnpc1,changed_guard]
def_int $flag = 0;
def_int $selection;
if ($flag = 0) {
    $selection = 7;
} else {
    $selection = 1;
}
$flag = 1;
if ($flag = 0) {
    $selection = 1;
}
switch_int($selection) {
    case 7: return;
}
`);
    const block = blockNamed(changed.blocks, 'opnpc1', 'changed_guard');
    const line = lineMatching(block, (text) => text.startsWith('switch_int($selection)'));
    const possible = changed.resolve(block, line, '$selection', [7], false);
    assert.ok(possible === null || possible.length > 0);
}

{
    const commented = prepare(`[proc,commented_menu](int $first, int $second)(int)
switch_component(last_com) {
    case panel:first: return($first);
    case default: return($second);
}
/** Used for things like ladder options. **/
[opnpc1,commented_caller]
switch_int(~commented_menu(1, 2)) {
    case 3: return;
}
`);
    const block = blockNamed(commented.blocks, 'opnpc1', 'commented_caller');
    const line = lineMatching(block, (text) => text.startsWith('switch_int(~commented_menu'));
    assert.deepEqual(commented.resolve(block, line, '~commented_menu(1, 2)', [3], false), []);
}

for (const body of [
    `switch_component(last_com) {
    case panel:first: return(1);
}`,
    `def_int $result = 1;
switch_component(last_com) {
    case panel:first: $result = 7;
    case default: $result = 1;
}
return($result);`,
]) {
    const opaque = prepare(`[proc,opaque_menu]()(int)
${body}
[opnpc1,opaque_caller]
switch_int(~opaque_menu) {
    case 7: return;
}
`);
    const block = blockNamed(opaque.blocks, 'opnpc1', 'opaque_caller');
    const line = lineMatching(block, (text) => text.startsWith('switch_int(~opaque_menu'));
    const possible = opaque.resolve(block, line, '~opaque_menu', [7], false);
    assert.ok(possible === null || possible.length > 0);
}

{
    const changed = prepare(`[opnpc1,changed_varp_guard]
def_int $selection;
if (%flag = 0) {
    $selection = 7;
} else {
    $selection = 1;
}
%flag = 1;
if (%flag = 0) {
    $selection = 1;
}
switch_int($selection) {
    case 7: return;
}
`);
    const block = blockNamed(changed.blocks, 'opnpc1', 'changed_varp_guard');
    const line = lineMatching(block, (text) => text.startsWith('switch_int($selection)'));
    const possible = changed.resolve(block, line, '$selection', [7], false);
    assert.ok(possible === null || possible.length > 0);
}

{
    const changed = prepare(`[label,guarded_receiver](int $selection)
switch_int($selection) {
    case 7: return;
}
[opnpc1,changed_caller_guard]
if (%flag = 0) {
    %flag = 1;
    @guarded_receiver(7);
} else {
    @guarded_receiver(1);
}
`);
    const block = blockNamed(changed.blocks, 'label', 'guarded_receiver');
    const line = lineMatching(block, (text) => text.startsWith('switch_int($selection)'));
    const possible = changed.resolve(block, line, '$selection', [7], false);
    assert.ok(possible?.some((path) => path.length === 0));
}
