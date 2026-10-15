'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const { assertGenerated, classifyModule, validateArtifacts } = require('./abi-contract.cjs');

const here = __dirname;
const contractText = fs.readFileSync(path.join(here, 'abi-contract.json'), 'utf8');
const predecessorText = fs.readFileSync(path.join(here, 'family-predecessors.json'), 'utf8');
const contract = JSON.parse(contractText);
const predecessors = JSON.parse(predecessorText);

test('the generated ABI contract and family graph are complete', () => {
    validateArtifacts(contract, predecessors);
});

test('a seeded member drift is rejected by the generated-table check', () => {
    const drift = structuredClone(JSON.parse(contractText));
    const declaration = drift.declarations.find(row => row.members.length > 0);
    assert.ok(declaration, 'the frozen ABI has a member to seed');
    declaration.members[0].name += '__seeded_drift';
    assert.throws(
        () => assertGenerated(contractText, `${JSON.stringify(drift, null, 2)}\n`),
        /ABI contract differs from deterministic generation/,
    );
});

test('an unclassified source module fails closed', () => {
    assert.throws(
        () => classifyModule('api/new-unclassified/Member.js'),
        /Missing ABI family classification/,
    );
});

test('a missing required predecessor fails graph validation', () => {
    const drift = structuredClone(predecessors);
    drift.families[0].predecessors = drift.families[0].predecessors.filter(value => value !== 'T1.3');
    assert.throws(() => validateArtifacts(contract, drift), /missing predecessor T1.3/);
});
