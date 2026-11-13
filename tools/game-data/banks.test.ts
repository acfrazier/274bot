import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractNpcBankAccesses } from './extractors/banks.ts';
import { extractBankPlacements } from './generate.ts';

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'bank-accesses-'));
try {
    for (const dir of ['pack', 'scripts', 'maps']) fs.mkdirSync(path.join(root, dir));
    fs.writeFileSync(path.join(root, 'pack/npc.pack'), '1=teller\n2=shilo\n3=decoy\n4=override\n5=nested\n6=dialogue\n');
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
`);
    fs.writeFileSync(path.join(root, 'scripts/banks.rs2'), `
[apnpc3,_bank_teller] @renamed_open;
[opnpc3,shilo] @renamed_open;
[opnpc3,decoy] mes("@renamed_open"); // @renamed_open;
[opnpc3,override] mes("Not a bank.");
[opnpc2,nested] ~open_procedure;
[proc,open_procedure] @renamed_open;
[opnpc1,dialogue]
def_int $choice = ~p_choice2("Use my vault.", 1, "Goodbye.", 2);
if ($choice = 2) { return; }
@renamed_open;
[label,renamed_open]
inv_transmit(bank, vault:items);
if_openmain_side(vault, vault_side);
`);
    fs.writeFileSync(path.join(root, 'maps/m44_46.jm2'), '==== NPC ====\n0 35 10: 2\n0 36 10: 3\n');
    const accesses = extractNpcBankAccesses(root);
    assert.deepEqual(accesses.map(row => [row.id, row.op, row.choose]), [
        [1, 3, null], [2, 3, null], [5, 2, null], [6, 1, 'Use my vault.'],
    ], 'bank identity follows content calls, category dispatch, and explicit override, never the option label');
    const catalog = [{ name: 'Test bank', tile: { x: 2851, z: 2954, level: 0 }, npcAccess: { name: 'Banker', op: 'Store' } }];
    assert.deepEqual(extractBankPlacements(root, catalog).facts.rows.map(row => row.id), [2]);
    const decoyCatalog = [{ name: 'Decoy', tile: catalog[0].tile, npcAccess: { name: 'Banker', op: 'Bank' } }];
    assert.deepEqual(extractBankPlacements(root, decoyCatalog).facts.rows, [], 'matching Bank menu text cannot invent a bank placement');
} finally {
    fs.rmSync(root, { recursive: true, force: true });
}
