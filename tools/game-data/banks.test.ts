import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractNpcBankAccesses } from './extractors/banks.ts';
import { extractBankPlacements } from './generate.ts';

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'bank-accesses-'));
try {
    for (const dir of ['pack', 'scripts', 'maps']) fs.mkdirSync(path.join(root, dir));
    fs.writeFileSync(path.join(root, 'pack/npc.pack'), [
        '1=teller', '2=shilo', '3=decoy', '4=override', '5=nested', '6=dialogue',
        '7=quest_guard', '8=else_guard', '9=members_guard', '10=nested_choice',
        '11=proc_guard', '12=label_guard', '13=approach_teller', '14=approach_misc',
        '15=arbitrary_range_guard', '16=early_return', '17=unsupported_choice',
        '18=inline_guard',
    ].join('\n') + '\n');
    fs.writeFileSync(path.join(root, 'pack/loc.pack'), '1=crate\n');
    fs.writeFileSync(path.join(root, 'scripts/crate.loc'), '[crate]\nname=Crate\n');
    fs.writeFileSync(path.join(root, 'scripts/banks.npc'), `
[teller]
name=Banker
category=bank_teller
op3=Store
[shilo]
name=Banker
op3=Store
[decoy]
name=Banker
op3=Bank
[override]
name=Banker
category=bank_teller
op3=Bank
[nested]
name=Vault keeper
op2=Store
[dialogue]
name=Keeper
op1=Speak
[quest_guard]
name=Banker
op3=Bank
[else_guard]
name=Banker
op3=Bank
[members_guard]
name=Banker
op3=Bank
[nested_choice]
name=Keeper
op1=Talk-to
[proc_guard]
name=Keeper
op3=Bank
[label_guard]
name=Keeper
op3=Bank
[approach_teller]
name=Banker
category=bank_teller
op3=Bank
[approach_misc]
name=Miscellania banker
op3=Bank
[arbitrary_range_guard]
name=Banker
op3=Bank
[early_return]
name=Banker
op3=Bank
[unsupported_choice]
name=Keeper
op1=Talk-to
[inline_guard]
name=Keeper
op3=Bank
`);
    fs.writeFileSync(path.join(root, 'scripts/banks.rs2'), `
[apnpc3,_bank_teller]
if (npc_range(coord) > 2) {
    p_aprange(2);
    return;
}
@renamed_open;
[opnpc3,shilo] @renamed_open;
[opnpc3,decoy] mes("@renamed_open"); // @renamed_open;
[opnpc3,override] mes("Not a bank.");
[opnpc2,nested] ~open_procedure;
[proc,open_procedure] @renamed_open;
[opnpc1,dialogue]
def_int $choice = ~p_choice2("Use my vault.", 1, "Goodbye.", 2);
if ($choice = 2) { return; }
@renamed_open;
[opnpc3,quest_guard]
if (%some_quest < 5) {
    ~chatnpc("<p,neutral>Finish my quest first.");
    return;
}
@renamed_open;
[opnpc3,else_guard]
if (%some_quest >= 5) {
    @renamed_open;
} else {
    ~chatnpc("<p,neutral>No.");
}
[opnpc3,members_guard]
if (map_members = ^false) { mes("Members only."); return; }
@renamed_open;
[opnpc1,nested_choice]
def_int $choice = ~p_choice2("Open my bank.", 1, "Bye.", 2);
if ($choice = 2) { return; }
if (%some_quest < 5) { ~chatnpc("<p,neutral>Not yet."); return; }
@renamed_open;
[opnpc3,proc_guard] ~guarded_open;
[proc,guarded_open]
if (%some_quest < 1) { return; }
@renamed_open;
[opnpc3,label_guard] @guarded_label;
[label,guarded_label]
if (map_members = ^false) { return; }
@renamed_open;
[apnpc3,approach_misc]
if (npc_range(coord) > 2) {
    p_aprange(2);
    return;
}
@renamed_open;
[opnpc3,arbitrary_range_guard]
if (npc_range(coord) > 2) {
    p_aprange(2);
    return;
}
@renamed_open;
[opnpc3,early_return]
return;
@renamed_open;
[opnpc1,unsupported_choice]
def_int $choice = ~p_choice4("Open my bank.", 1, "Bye.", 2, "Later.", 3, "Never mind.", 4);
switch_int ($choice) {
    case 1 : @renamed_open;
    case 2 : return;
}
[opnpc3,inline_guard]
if (%some_quest >= 5) {
    inv_transmit(bank, vault:items);
    if_openmain_side(vault, vault_side);
}
[label,renamed_open]
if (%bankpin_session = 0) {
    ~unlock_bank_session;
    return;
}
inv_transmit(bank, vault:items);
if_openmain_side(vault, vault_side);
`);
    fs.writeFileSync(path.join(root, 'maps/m44_46.jm2'), '==== NPC ====\n0 35 10: 2\n0 36 10: 3\n');
    const evidence = extractNpcBankAccesses(root);
    assert.deepEqual(evidence.rows.map(row => [row.id, row.op, row.choose]), [
        [1, 3, null], [2, 3, null], [5, 2, null], [6, 1, 'Use my vault.'],
        [13, 3, null], [14, 3, null],
    ], 'plain, nested, supported-choice, and approach-range paths reach the bank procedure despite its shared session guard');
    assert.deepEqual(evidence.refusals.map(({ id, op, reason }) => [id, op, reason]), [
        [7, 3, 'conditional-control-flow'],
        [8, 3, 'conditional-control-flow'],
        [9, 3, 'conditional-control-flow'],
        [10, 1, 'conditional-control-flow'],
        [11, 3, 'conditional-control-flow'],
        [12, 3, 'conditional-control-flow'],
        [15, 3, 'conditional-control-flow'],
        [16, 3, 'early-return'],
        [17, 1, 'unsupported-choice'],
        [18, 3, 'conditional-control-flow'],
    ], 'guarded candidates remain visible with their refusal reasons');
    const catalog = [{ name: 'Test bank', tile: { x: 2851, z: 2954, level: 0 }, npcAccess: { name: 'Banker', op: 'Store' } }];
    assert.deepEqual(extractBankPlacements(root, catalog).facts.rows.map(row => row.id), [2]);
    const decoyCatalog = [{ name: 'Decoy', tile: catalog[0].tile, npcAccess: { name: 'Banker', op: 'Bank' } }];
    assert.deepEqual(extractBankPlacements(root, decoyCatalog).facts.rows, [], 'matching Bank menu text cannot invent a bank placement');
} finally {
    fs.rmSync(root, { recursive: true, force: true });
}

for (const revision of [274, 289, 377]) {
    const content = process.env[`GAME_DATA_${revision}_CONTENT`];
    if (!content) continue;
    const { rows, refusals } = extractNpcBankAccesses(content);
    for (const alias of ['shilobanker', 'duel_fadli']) {
        const row = rows.find(candidate => candidate.alias === alias);
        assert.ok(row, `the selected ${revision} content admits the ${alias} bank handler`);
        assert.equal(row.op, 3);
        assert.equal(row.action, 'Bank');
        assert.equal(row.choose, null);
        assert.ok(!refusals.some(refusal => refusal.alias === alias && refusal.op === 3), `${alias} op3 is not refused`);
    }
    assert.ok(rows.find(row => row.alias === 'shilobanker')!.sources.some(source => source.file === 'scripts/interface_bank/scripts/banker.rs2'));
    assert.ok(rows.find(row => row.alias === 'duel_fadli')!.sources.some(source => source.file === 'scripts/minigames/game_duelarena/scripts/fadli.rs2'));
}
