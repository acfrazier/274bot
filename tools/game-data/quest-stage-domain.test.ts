import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { extractQuestIdentityFacts } from './extractors/quests.ts';
import { questFixture } from './quest-fixture.ts';

const content = process.env.GAME_DATA_289_CONTENT;
if (content) {
    const stageFile = 'scripts/quests/quest_priestperil/scripts/priestperil.rs2';
    const constantFile = 'scripts/quests/quest_priestperil/configs/priestperil.constant';
    const source = fs.readFileSync(path.join(content, stageFile), 'utf8');
    assert.match(source, /%priestperil\s*=\s*\^priestperil_started\s*;/, 'the selected Priest stage source declares a constant assignment');
    const questSource = fs.readFileSync(path.join(content, 'scripts/general/scripts/quests.rs2'), 'utf8');
    const colourCall = questSource.split(/\r?\n/).find((line) => line.includes('~send_quest_progress_colour(questlist:priestperil,'));
    assert.ok(colourCall, 'the selected content declares Priest in Peril quest progress');
    const root = questFixture((files) => {
        files[stageFile] = source;
        files[constantFile] = fs.readFileSync(path.join(content, constantFile), 'utf8');
        files['scripts/general/configs/quest.constant'] = fs.readFileSync(path.join(content, 'scripts/general/configs/quest.constant'), 'utf8');
        files['pack/varp.pack'] = fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8');
        files['scripts/general/scripts/quests.rs2'] += `${colourCall}\n`;
        files['scripts/player/interfaces/questlist.if'] += '\n[priestperil]\ny=6\ntext=Priest in Peril\n';
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'priestperil');
        assert.ok(row, 'Priest in Peril is extracted from the selected stage source');
        assert.deepEqual(row.stage_domain, { min: 0, max: 60 }, 'constant Priest stages must establish a non-null bounded stage domain');
        console.log('Priest in Peril content-backed stage-domain regression passed');
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}
