import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export function writeQuestFixture(rootDir: string, mutate?: (files: Record<string, string>) => void) {
    const files: Record<string, string> = {
        'scripts/general/scripts/quests.rs2': `~send_quest_progress_colour($component, $progress, $complete_progress);
%qp = $questpointrecount;
~send_quest_progress_colour(questlist:runemysteries, %runemysteries, ^runemysteries_complete);
~send_quest_progress_colour(questlist:cook, %cookquest, ^cook_complete);
~send_quest_progress_colour(questlist:zanaris, %zanaris, ^zanaris_complete);
~send_quest_progress_colour(questlist:waterfall, %waterfall_quest, ^waterfall_complete);
~send_quest_progress_colour(questlist:murder, %murderquest, ^murder_complete);
~send_quest_progress_colour(questlist:death, %death_equiproom, ^death_complete);
~send_quest_progress_colour(questlist:itexam, ~itexam_progress, ^itexam_complete);
~send_quest_progress_colour(questlist:blackarmgang, %phoenixgang, ^phoenixgang_complete);
~send_quest_progress_colour(questlist:blackarmgang, %blackarmgang, ^blackarmgang_complete);
~send_quest_progress_colour(questlist:elemental_workshop, %elemental_workshop_bits, sub(pow(2, ^elemental_workshop_complete), 1));
~send_quest_progress_colour(questlist:legends, %legendsquest, ^legends_complete);
~send_quest_progress_colour(questlist:regicide, %regicide_quest, ^regicide_complete);
`,
        'scripts/general/configs/quest.constant': `^cook_complete = 2
^cookquest_not_started = 0
^cookquest_complete = 7
^cook_questpoints = 1
^runemysteries_complete = 6
^runemysteries_questpoints = 1
^murder_complete = 2
^murder_questpoints = 3
^waterfall_complete = 10
^waterfall_questpoints = 1
^death_equiproom_complete = 999
^death_complete = 80
^death_questpoints = 1
^zanaris_complete = 6
^zanaris_questpoints = 3
`,
        'scripts/player/interfaces/questlist.if': `[death]
y=1
text=Death Plateau
[zanaris]
y=2
text=Lost City
[cook]
y=99
text=Cook's Assistant
[runemysteries]
y=3
text=Rune Mysteries Quest
[murder]
y=4
text=Murder Mystery
[waterfall]
y=5
text=Waterfall Quest
`,
        'pack/varp.pack': `29=cookquest
63=runemysteries
192=murderquest
65=waterfall_quest
219=death
314=death_equiproom
147=zanaris
101=qp
`,
        'pack/npc.pack': `100=cook
`,
        'pack/loc.pack': `200=cook_start
`,
        'scripts/areas/area_test/scripts/start.rs2': `[opnpc1,cook]
if (%cookquest = ^cookquest_not_started) {
    @cook_start;
}
[label,cook_start]
%cookquest = 1;
`,
        'scripts/general/configs/quest.enum': `[quest_names_enum]
val=1,Cook's Assistant
val=13,Rune Mysteries Quest
val=37,Murder Mystery
val=50,Waterfall Quest
val=55,Death Plateau
val=34,Lost City
`,
        'scripts/quests/quest_cook/scripts/quest_cook.rs2': `if(inv_total(inv, pot_flour) > 0 & inv_total(inv, egg) > 0 & inv_total(inv, bucket_milk) > 0) {
}
inv_del(inv, egg, 1);
inv_del(inv, bucket_milk, 1);
inv_del(inv, pot_flour, 1);
`,
        'scripts/quests/quest_waterfall/scripts/quest_waterfall.rs2': `// rope comment is not a requirement list
if(last_useitem ! rope) {
}
if(last_useitem ! rope) {
}
`,
        'scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2': `if(stat(woodcutting) < 36) {
}
if(stat(crafting) < 31) {
}
if(inv_total(inv, axe) > 0) {
}
inv_del(inv, knife, 1);
if(last_useitem ! dramen_branch) {
}
`,
        'scripts/quests/quest_death/scripts/death_sherpa.rs2': `inv_del(inv, death_climbingboots, 1);
inv_del(inv, death_secretwaymap, 1);
inv_del(inv, trout, 1);
`,
    };
    mutate?.(files);
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
}
export function questFixture(mutate?: (files: Record<string, string>) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'quest-identity-'));
    writeQuestFixture(rootDir, mutate);
    return rootDir;
}
