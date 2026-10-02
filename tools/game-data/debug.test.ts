import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { assertEngineCommandDrift, engineHandlerRelative, extractDebugCatalog, parseDebugHelp, parseDebugprocSource, sharedTestBlocks } from './extractors/debug.ts';
import { revisions } from './generate.ts';

const help = parseDebugHelp(`
~mesbox("@blu@Account commands|@whi@::demo - Set the demo account state");
~mesbox("@blu@Teleport commands|@whi@::warp - Move one tile");
`);
const parsed = parseDebugprocSource(
    '[debugproc,demo](stat $stat, int $level) // account mutation\nsetvar demo;\n[debugproc,warp](coord $destination)\np_telejump($destination);',
    'scripts/demo.rs2',
    help,
);
assert.equal(parsed[0].category, 'Account');
assert.equal(parsed[0].description, 'Set the demo account state');
assert.deepEqual(parsed[0].args.map((arg) => [arg.name, arg.kind, arg.optional]), [['stat', 'stat', false], ['level', 'int', false]]);
assert.equal(parsed[1].category, 'Teleport');
assert.equal(parsed[1].destructive, false);
assert.throws(() => parseDebugprocSource('[debugproc,bad](float $value)', 'scripts/mutated.rs2'));
assert.equal(parseDebugprocSource('[debugproc,demo](int $value)', 'scripts/mutated.rs2', help)[0].args[0].kind, 'int');

const handlerPath = path.join(revisions.find((spec) => spec.revision === 289)!.engine, engineHandlerRelative());
const statPath = path.join(revisions.find((spec) => spec.revision === 289)!.engine, 'src/engine/entity/PlayerStat.ts');
const statText = fs.readFileSync(statPath, 'utf8');
const handler = fs.readFileSync(handlerPath, 'utf8');
assertEngineCommandDrift(handler);
assert.throws(() => assertEngineCommandDrift(`${handler}\nif (cmd === 'future_command') {}`));
const real289Content = revisions.find((spec) => spec.revision === 289)!.content;
const shared289 = sharedTestBlocks(real289Content);
const questRows = parseDebugprocSource(
    fs.readFileSync(path.join(real289Content, 'scripts/_test/scripts/cheats/cheat_quest.rs2'), 'utf8'),
    'scripts/_test/scripts/cheats/cheat_quest.rs2',
    new Map(),
    shared289,
);
const questByName = new Map(questRows.map((row) => [row.name, row]));
for (const [name, destructive] of [
    ['~quest', true],
    ['~quests', true],
    ['~resetquests', true],
    ['~completequests', true],
    ['~cq', true],
    ['~rq', true],
] as const) {
    assert.equal(questByName.get(name)?.destructive, destructive, `${name} destructive`);
}
const sourceRows = (relative: string) =>
    parseDebugprocSource(fs.readFileSync(path.join(real289Content, relative), 'utf8'), relative, new Map(), shared289);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_maxme.rs2').find((row) => row.name === '~maxme')?.destructive, false);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_other.rs2').find((row) => row.name === '~addxp')?.destructive, false);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_clearinv.rs2').find((row) => row.name === '~clearinv')?.destructive, true);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_magic.rs2').find((row) => row.name === '~giverunes')?.destructive, false);
assert.equal(sourceRows('scripts/_test/scripts/engine/debug_stat.rs2').find((row) => row.name === '~minstat')?.destructive, true);
assert.equal(sourceRows('scripts/_test/scripts/engine/debug_stat.rs2').find((row) => row.name === '~stat_boost')?.destructive, false);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_teles.rs2').find((row) => row.name === '~east')?.destructive, false);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_serverstats.rs2').find((row) => row.name === '~lag')?.destructive, false);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_help.rs2').find((row) => row.name === '~help')?.destructive, true);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_treasuretrails.rs2').find((row) => row.name === '~giveclues')?.destructive, true);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_interactions.rs2').find((row) => row.name === '~itest')?.destructive, true);
// A destructive arm behind a menu flags the command: every p_choice branch is followed.
const branched = parseDebugprocSource(
    '[debugproc,menu] @branch_menu;\n[label,branch_menu]\ndef_int $choice = ~p_choice2_header("Wipe", 0, "Leave", 1, "Pick");\nif ($choice = 0) {\ninv_clear(inv);\n} else {\nmes("kept");\n}\n[debugproc,calm] @calm_menu;\n[label,calm_menu]\ndef_int $choice = ~p_choice2_header("Greet", 0, "Leave", 1, "Pick");\nif ($choice = 0) {\nmes("hello");\n}',
    'scripts/branched.rs2',
);
assert.equal(branched.find((row) => row.name === '~menu')?.destructive, true);
assert.equal(branched.find((row) => row.name === '~calm')?.destructive, false);
// A gosub chain is followed transitively; a cycle still terminates.
const gosubbed = parseDebugprocSource(
    '[debugproc,wipe] gosub(scrub);\n[proc,scrub]\ngosub(rinse);\n[proc,rinse]\ninv_clear(worn);\n[debugproc,loop] gosub(spin_a);\n[proc,spin_a]\ngosub(spin_b);\n[proc,spin_b]\ngosub(spin_a);\nmes("spinning");',
    'scripts/gosubbed.rs2',
);
assert.equal(gosubbed.find((row) => row.name === '~wipe')?.destructive, true);
assert.equal(gosubbed.find((row) => row.name === '~loop')?.destructive, false);
const stringOnlyEffect = parseDebugprocSource(
    '[debugproc,help]\nmes("Reset your progress with give coins");',
    'scripts/string-only.rs2',
);
assert.equal(stringOnlyEffect[0].destructive, false);
const codeEffectAfterQuotedText = parseDebugprocSource(
    '[debugproc,mutate]\n// operator\'s note\nmes("http://example.invalid/reset");\ninv_clear(inv);',
    'scripts/comment-and-string.rs2',
);
assert.equal(codeEffectAfterQuotedText[0].destructive, true);
assert.equal(sourceRows('scripts/_test/scripts/cheats/cheat_treasuretrails.rs2').find((row) => row.name === '~giveclues')?.category, 'Item');
const bounded = parseDebugprocSource(
    '[debugproc,inspect]\nmes("read only");\n[label,unrelated]\ninv_add(inv, coins, 1);\n[debugproc,next]\nmes("next");',
    'scripts/fixture.rs2',
);
assert.equal(bounded.find((row) => row.name === '~inspect')?.destructive, false);
for (const [name, destructive] of [
    ['minme', true],
    ['give', false],
    ['givebank', false],
    ['tele', false],
    ['setstat', false],
    ['advancestat', false],
    ['setvar', false],
    ['speed', false],
    ['reload', false],
    ['locadd', false],
    ['getvar', false],
] as const) {
    assert.equal(assertEngineCommandDrift(handler).find((row) => row.name === name)?.destructive, destructive, `${name} destructive`);
}
const removedProductionGate = handler.replace("cmd === 'setvarother' && Environment.node.production", "cmd === 'setvarother'");
assert.throws(() => assertEngineCommandDrift(removedProductionGate), /production gate drift/);
const removedNonProductionGate = handler.replace("cmd === 'givebank' && !Environment.node.production", "cmd === 'givebank'");
assert.throws(() => assertEngineCommandDrift(removedNonProductionGate), /production gate drift/);

const content = fs.mkdtempSync(path.join(os.tmpdir(), 'debug-catalog-'));
fs.mkdirSync(path.join(content, 'scripts/_test/scripts/cheats'), { recursive: true });
fs.writeFileSync(path.join(content, 'scripts/_test/scripts/cheats/cheat_help.rs2'), '@unused\n');
fs.writeFileSync(path.join(content, 'scripts/demo.rs2'), '[debugproc,demo](int $value)\nsetvar demo;\n');
for (const [relative, alias] of [
    ['obj.pack', 'coins'],
    ['npc.pack', 'guard'],
    ['loc.pack', 'altar'],
    ['seq.pack', 'wave'],
    ['spotanim.pack', 'spark'],
    ['interface.pack', 'root:main'],
    ['varp.pack', 'tutorial'],
    ['varbit.pack', 'tutorial_done'],
    ['inv.pack', 'inventory'],
    ['idk.pack', 'kit'],
] as const) {
    fs.mkdirSync(path.join(content, 'pack'), { recursive: true });
    fs.writeFileSync(path.join(content, 'pack', relative), `7=${alias}\n`);
}
fs.writeFileSync(
    path.join(content, 'scripts/_test/scripts/cheats/cheat_help.rs2'),
    '~mesbox("@blu@Account commands|@whi@::demo - Demo");\n',
);
const catalog = extractDebugCatalog(
    content,
    handler,
    statText,
    [{ id: 2, debugname: 'guard', name: 'Guard' }],
);
assert.equal(catalog.commands.find((row) => row.name === '~demo')?.category, 'Account');
assert.equal(catalog.names.loc[0].alias, 'altar');
assert.equal(catalog.names.npc[0].name, 'Guard');
assert.equal(catalog.names.varp.some((row) => row.alias === 'tutorial_done'), true);
assert.equal(catalog.names.stat.length, 19);
const mutatedPack = path.join(content, 'pack/loc.pack');
fs.writeFileSync(mutatedPack, '7=mutated_altar\n');
const mutated = extractDebugCatalog(
    content,
    handler,
    statText,
    [{ id: 2, debugname: 'guard', name: 'Guard' }],
);
assert.equal(mutated.names.loc[0].alias, 'mutated_altar');
