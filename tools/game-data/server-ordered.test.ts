import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import {
    extractServerOrderedFacts,
    serverOrderedContentFiles,
    type ServerOrderedCacheComponent,
    type ServerOrderedExtractInput,
} from './extractors/server-ordered.ts';

const ENGINE_COMMIT = 'a'.repeat(40);
const CONTENT_COMMIT = 'b'.repeat(40);
const BANKER_TEXT = "I'd like to access my bank account, please.";
const PLAYER_ENGINE_SOURCE = 'src/engine/entity/Player.ts';
const COUNT_RESUME_ENGINE_SOURCE = 'src/network/game/client/handler/ResumePCountDialogHandler.ts';
const PLAYER_CLOSE_MODAL_SOURCE = `
closeModal(clearWeakQueue: boolean = true) {
    if (clearWeakQueue) {
        this.weakQueue.clear();
    }
    if (!this.delayed) {
        this.protect = false;
    }
    if (this.modalState === ModalState.NONE) {
        return;
    }
    this.modalState = ModalState.NONE;
    if (this.activeScript?.execution === ScriptState.COUNTDIALOG || this.activeScript?.execution === ScriptState.PAUSEBUTTON) {
        this.activeScript = null;
        this.resumeButtons = [];
    }
    if (this.modalMain !== -1) {
        const closeTrigger = ScriptProvider.getByTrigger(ServerTriggerType.IF_CLOSE, this.modalMain);
        if (closeTrigger) {
            this.executeScript(ScriptRunner.init(closeTrigger, this), false);
        }
        this.clearComListeners(this.modalMain);
        this.modalMain = -1;
    }
    if (this.modalChat !== -1) {
        const closeTrigger = ScriptProvider.getByTrigger(ServerTriggerType.IF_CLOSE, this.modalChat);
        if (closeTrigger) {
            this.executeScript(ScriptRunner.init(closeTrigger, this), false);
        }
        this.clearComListeners(this.modalChat);
        this.modalChat = -1;
    }
    if (this.modalSide !== -1) {
        const closeTrigger = ScriptProvider.getByTrigger(ServerTriggerType.IF_CLOSE, this.modalSide);
        if (closeTrigger) {
            this.executeScript(ScriptRunner.init(closeTrigger, this), false);
        }
        this.clearComListeners(this.modalSide);
        this.modalSide = -1;
    }
    this.refreshModalClose = true;
}
`;
const COUNT_RESUME_HANDLER_SOURCE = `
handle(message: ResumePCountDialog, player: Player): boolean {
    const { input } = message;
    if (!player.activeScript || player.activeScript.execution !== ScriptState.COUNTDIALOG) {
        return false;
    }
    player.activeScript.lastInt = input;
    player.executeScript(player.activeScript, true, true);
    return true;
}
`;
const GROUPS: Record<string, string[]> = {
    bank_main: ['com_92', 'inv', 'com_93', 'com_94', 'com_96', 'com_98'],
    bank_side: ['inv'],
    multi2: ['com_1'],
    skill_multi3: ['make1_a'],
    player_kit: ['accept'],
    chat1: ['com_3'],
};

type ServerOrderedTestFixture = {
    root: string;
    files: Map<string, string>;
    input: ServerOrderedExtractInput;
    save(): void;
};

function definition(alias: string, fields: Record<string, string>): string {
    return `[${alias}]\n${Object.entries(fields).map(([key, value]) => `${key}=${value}`).join('\n')}\n`;
}

function fixture(split: (text: string, width: number) => string[] = text => [text]): ServerOrderedTestFixture {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'server-ordered-'));
    const files = new Map<string, string>(serverOrderedContentFiles.map(file => [file, ''] as const));
    const aliases = Object.entries(GROUPS).flatMap(([rootAlias, children]) => [
        rootAlias,
        ...children.map(child => `${rootAlias}:${child}`),
    ]);
    const ids = new Map<string, number>(aliases.map((alias, index) => [alias, index + 100] as const));
    const actions = ['Withdraw 1', 'Withdraw 5', 'Withdraw 10', 'Withdraw All', 'Withdraw X'];
    const depositActions = ['Deposit 1', 'Deposit 5', 'Deposit 10', 'Deposit All', 'Deposit X'];
    const mainSource = [
        definition('com_92', { type: 'layer', x: '37', y: '55', width: '427', height: '229', scroll: '0' }),
        definition('inv', {
            type: 'inv', layer: 'com_92', clientcode: '206', width: '8', height: '36',
            draggable: 'yes', margin: '15,6', ...Object.fromEntries(actions.map((value, index) => [`option${index + 1}`, value])),
        }),
        definition('com_93', {
            type: 'graphic', buttontype: 'select', x: '364', y: '291', width: '72', height: '36',
            'script1op1': 'pushvar,bankcert', script1: 'eq,1', graphic: 'combatboxes,2', activegraphic: 'combatboxes,3',
        }),
        definition('com_94', {
            type: 'graphic', buttontype: 'select', x: '291', y: '291', width: '72', height: '36',
            'script1op1': 'pushvar,bankcert', script1: 'eq,0', graphic: 'combatboxes,2', activegraphic: 'combatboxes,3',
        }),
        definition('com_96', {
            type: 'text', x: '315', y: '301', width: '24', height: '14', font: 'q8_full', shadowed: 'yes', text: 'Item',
        }),
        definition('com_98', {
            type: 'text', x: '387', y: '301', width: '27', height: '14', font: 'q8_full', shadowed: 'yes', text: 'Note',
        }),
    ].join('\n');
    const sideSource = definition('inv', {
        type: 'inv', width: '4', height: '7', draggable: 'yes', margin: '10,4',
        ...Object.fromEntries(depositActions.map((value, index) => [`option${index + 1}`, value])),
    });
    const componentSources = new Map<string, string>([
        ['scripts/interface_bank/interfaces/bank_main.if', mainSource],
        ['scripts/interface_bank/interfaces/bank_side.if', sideSource],
        ['scripts/interface_chat/interfaces/multi2.if', definition('com_1', { type: 'text', buttontype: 'normal', font: 'q8_full' })],
        ['scripts/interface_chat/interfaces/skill_multi3.if', definition('make1_a', { type: 'text', buttontype: 'normal', font: 'q8_full' })],
        ['scripts/tutorial/interfaces/player_kit.if', definition('accept', { type: 'rect', buttontype: 'normal', clientcode: '326' })],
        ['scripts/interface_chat/interfaces/chat1.if', definition('com_3', { type: 'text', buttontype: 'pause', font: 'q8_full' })],
    ]);
    for (const [file, text] of componentSources) files.set(file, text);

    const packRows = aliases.map(alias => `${ids.get(alias)}=${alias}`);
    files.set('pack/interface.pack', `${packRows.join('\n')}\n`);
    files.set('pack/varp.pack', '7=bankcert\n');
    files.set('pack/loc.pack', '8=bankbooth\n');
    files.set('pack/mesanim.pack', '9=quiz\n');
    files.set('scripts/interface_bank/configs/bank_booth.loc', definition('bankbooth', { op2: 'Use-quickly' }));
    files.set('scripts/interface_bank/scripts/bank_booth.rs2', '[oploc2,bankbooth]\n@openbank;\n');
    files.set('scripts/interface_bank/scripts/banker.rs2', [
        '[label,talk_to_banker]',
        `~p_choice2(${JSON.stringify(BANKER_TEXT)}, 1, "No thanks", 2);`,
        'switch_int ($option) {',
        'case 1:',
        `~chatplayer(${JSON.stringify(`<p,quiz>${BANKER_TEXT}`)});`,
        '@openbank;',
        'case 2:',
        '$option = ~p_choice2("And what do you do?", 1, "What happened to Varrock?", 2);',
        'switch_int ($option) {',
        'case 1:',
        'return;',
        'case 2:',
        'return;',
        '}',
        '}',
    ].join('\n'));
    files.set('scripts/interface_chat/scripts/chat.rs2', [
        '[proc,p_choice2]',
        'if_openchat(multi2);',
        'if_addresumebutton(multi2:com_1);',
        'p_pausebutton;',
        'switch_component (last_com) {',
        'case multi2:com_1 : return($ret1);',
        'case default : return($ret2);',
        '}',
        '[proc,chatplayer]',
        'split_init($string, 380, 4, q8_full);',
        'split_pagecount;',
        'while ($page < $pagetotal) {',
        '~chatplayer_page($page);',
        '}',
        'p_pausebutton;',
        '[proc,chatplayer_page]',
        'def_int $lines = split_linecount($page);',
        'if ($lines = 1) {',
        '$interface = chat1;',
        'if_setanim(chat1:com_0, split_getanim($page));',
        'if_settext(chat1:com_1, displayname);',
        'if_settext(chat1:com_2, split_get($page, 0));',
        '}',
        'if_openchat($interface);',
        '[proc,skill_multi3]',
        'if_openchat(skill_multi3);',
        'if_addresumebutton(skill_multi3:make1_a);',
        'p_pausebutton;',
        'def_component $last_com = last_com;',
        'if(%weakqueue_map_clock = map_clock) {',
        'if_close;',
        'p_delay(0);',
        '}',
        'switch_component ($last_com) {',
        'case skill_multi3:make1_a : return($obj1, 1);',
        'case default : return(null, null);',
        '}',
        '[if_close,skill_multi3]',
        'if(p_finduid(uid) = true) {',
        '%weakqueue_map_clock = map_clock;',
        '}',
    ].join('\n'));
    files.set('scripts/interface_bank/scripts/bank.rs2', [
        '[label,openbank]',
        '%bankcert = 0;',
        '[inv_button5,bank_main:inv]',
        '@bank_withdraw(last_slot, null);',
        '[label,bank_withdraw](int $slot, int $requested_number)',
        'def_obj $item = inv_getobj(bank, $slot);',
        'if ($requested_number <= 0) {',
        'p_countdialog;',
        'if (last_int <= 0) return;',
        '$requested_number = last_int;',
        '}',
        '[if_button,bank_main:com_94]',
        '%bankcert = 0;',
        '[if_button,bank_main:com_93]',
        '%bankcert = 1;',
        '[if_close,bank_main]',
        '@closebank;',
        '[label,closebank]',
        'inv_stoptransmit(bank_side:inv);',
        'inv_stoptransmit(bank_main:inv);',
        'queue(reorganize_bank, 0, 0);',
        '[queue,reorganize_bank]',
        'reorganize_inv(bank);',
    ].join('\n'));
    files.set('scripts/interface_bank/scripts/bank_deposit.rs2', [
        '[inv_button5,bank_side:inv]',
        '@bank_deposit(last_slot, null);',
        '[inv_button4,bank_side:inv]',
        '@bank_deposit(last_slot, ^max_32bit_int);',
        '[label,bank_deposit](int $slot, int $requested_number)',
        'def_obj $item = inv_getobj(inv, $slot);',
        'if ($requested_number <= 0) {',
        'p_countdialog;',
        'if (last_int <= 0) return;',
        '$requested_number = last_int;',
        '}',
    ].join('\n'));
    files.set('scripts/tutorial/scripts/tutorial.rs2', [
        '[if_button,player_kit:accept]',
        'if_close;',
        '[if_close,player_kit]',
        'queue(tutorial_designed_character, 0, 0);',
        '[queue,tutorial_designed_character]',
        '%tutorial = ^newbie_basics_instructor_designed_character;',
    ].join('\n'));
    files.set('scripts/skill_fletching/scripts/cut_logs.rs2', [
        '[label,fletch_log]',
        '~skill_multi3();',
        'if ($product = null) {',
        'return;',
        '}',
        '[label,process_fletch_logs]',
        'weakqueue*(fletch_logs, 0);',
        '[queue,fletch_logs]',
        'inv_del(inv, $log, 1);',
    ].join('\n'));

    const save = () => {
        for (const [file, text] of files) {
            const absolute = path.join(root, file);
            fs.mkdirSync(path.dirname(absolute), { recursive: true });
            fs.writeFileSync(absolute, text);
        }
    };
    save();

    const sourceFields = new Map<string, Record<string, string>>();
    for (const [file, text] of componentSources) {
        const rootAlias = file.includes('bank_main.if') ? 'bank_main'
            : file.includes('bank_side.if') ? 'bank_side'
                : file.includes('multi2.if') ? 'multi2'
                    : file.includes('skill_multi3.if') ? 'skill_multi3'
                        : file.includes('player_kit.if') ? 'player_kit' : 'chat1';
        let alias: string | null = null;
        let fields: Record<string, string> = {};
        const saveFields = () => {
            if (alias !== null) sourceFields.set(`${rootAlias}:${alias}`, fields);
        };
        for (const raw of text.split(/\r?\n/)) {
            const header = /^\[([^\]]+)\]$/.exec(raw.trim());
            if (header) {
                saveFields();
                alias = header[1];
                fields = {};
            } else {
                const equals = raw.indexOf('=');
                if (alias !== null && equals > 0) fields[raw.slice(0, equals)] = raw.slice(equals + 1);
            }
        }
        saveFields();
    }
    const components: ServerOrderedCacheComponent[] = aliases.map(alias => {
        const rootAlias = alias.includes(':') ? alias.slice(0, alias.indexOf(':')) : alias;
        const rootId = ids.get(rootAlias)!;
        const isRoot = alias === rootAlias;
        const fields = sourceFields.get(alias) ?? {};
        const componentType = isRoot ? 0 : fields.type === 'inv' ? 2 : fields.type === 'text' ? 4 : fields.type === 'layer' ? 0 : fields.type === 'graphic' ? 5 : 3;
        const buttonType = fields.buttontype === 'select' ? 5 : fields.buttontype === 'pause' ? 6 : fields.buttontype === 'normal' ? 1 : 0;
        const rootChildren = (GROUPS[rootAlias] ?? [])
            .filter(child => rootAlias !== 'bank_main' || child !== 'inv')
            .map(child => `${rootAlias}:${child}`);
        const children = isRoot ? rootChildren : alias === 'bank_main:com_92' ? ['bank_main:inv'] : [];
        return {
            id: ids.get(alias)!,
            name: alias,
            root_layer: rootId,
            component_type: componentType,
            button_type: buttonType,
            client_code: Number(fields.clientcode ?? 0),
            width: Number(fields.width ?? 0),
            height: Number(fields.height ?? 0),
            margin_x: Number(fields.margin?.split(',')[0] ?? 0),
            margin_y: Number(fields.margin?.split(',')[1] ?? 0),
            child_ids: componentType === 0 ? children.map(child => ids.get(child)!) : null,
            scroll: Number(fields.scroll ?? 0),
            hidden: fields.hide === 'yes',
            draggable: fields.draggable === 'yes',
            operable: fields.operable === 'yes',
            usable: fields.usable === 'yes',
            swappable: fields.swappable === 'yes',
            actions: componentType === 2 ? Array.from({ length: 5 }, (_, index) => fields[`option${index + 1}`] ?? '') : null,
            action_verb: null,
            action: null,
            action_target: -1,
            fill: fields.fill === 'yes',
            centered: fields.center === 'yes',
            font_id: 3,
            shadowed: fields.shadowed === 'yes',
            text: componentType === 4 ? fields.text ?? '' : null,
            active_text: componentType === 4 ? fields.activetext ?? '' : null,
            colour: 0,
            active_colour: 0,
            over_colour: 0,
            active_over_colour: 0,
            graphic: componentType === 5 ? fields.graphic ?? '' : null,
            active_graphic: componentType === 5 ? fields.activegraphic ?? '' : null,
            model: -1,
            active_model: -1,
            anim: -1,
            active_anim: -1,
            zoom: 0,
            xan: 0,
            yan: 0,
            option: fields.option ?? null,
        };
    });
    const input: ServerOrderedExtractInput = {
        contentRoot: root,
        selectedPack: {
            revision: 289,
            engine_commit: ENGINE_COMMIT,
            content_commit: CONTENT_COMMIT,
            cache_id: 'c'.repeat(64),
            content_id: 'd'.repeat(64),
        },
        sourceCommits: { engine_commit: ENGINE_COMMIT, content_commit: CONTENT_COMMIT },
        cache: {
            components,
            fonts: [{ alias: 'q8_full', id: 3, split }],
            mesanims: [{ alias: 'quiz', id: 9 }],
        },
        engineSources: {
            [PLAYER_ENGINE_SOURCE]: PLAYER_CLOSE_MODAL_SOURCE,
            [COUNT_RESUME_ENGINE_SOURCE]: COUNT_RESUME_HANDLER_SOURCE,
        },
    };
    return { root, files, input, save };
}

function extract(sample: ServerOrderedTestFixture) {
    return extractServerOrderedFacts(sample.input);
}

const BANKER_INDEPENDENT_ENTRY_KEYS = [
    'knife_arm',
    'bank_withdraw_x',
    'bank_deposit_x',
    'bank_deposit_all',
    'bank_close_terminal',
    'numeric_bankbooth',
    'accept_character_design',
];

function assertBankerProofFailureKeepsIndependentEntries(sample: ServerOrderedTestFixture, label: string) {
    const catalog = extract(sample);
    assert.equal(catalog.banker_page, null, label);
    assert.deepEqual(catalog.entries.map(entry => entry.key), BANKER_INDEPENDENT_ENTRY_KEYS, label);
}

const success = fixture();
try {
    const catalog = extract(success);
    assert.ok(catalog.banker_page);
    assert.equal(catalog.banker_page.page_count, 1);
    assert.equal(catalog.banker_page.page_root_alias, 'chat1');
    assert.equal(catalog.banker_page.continue_component_alias, 'chat1:com_3');
    assert.equal(catalog.bank.mode_varp.id, 7);
    assert.equal(catalog.numeric_locs[0].operation, 2);
    assert.equal(catalog.entries.find(entry => entry.key === 'bank_withdraw_x')?.deadline_ticks, 2);
    assert.equal(catalog.entries.find(entry => entry.key === 'bank_deposit_all')?.deadline_ticks, null);
    assert.equal(catalog.entries.find(entry => entry.key === 'bank_close_terminal')?.recovery_count, 1);
    assert.deepEqual(catalog.components.find(component => component.alias === 'bank_main:inv')?.ancestor_ids, [success.input.cache.components.find(component => component.name === 'bank_main:com_92')!.id, success.input.cache.components.find(component => component.name === 'bank_main')!.id]);
    // These handlers are deliberately absent from this fixture. Their declarations
    // must stay unproved policy rather than acquire authority from a citation.
    for (const [key, name] of [
        ['bank_withdraw_x', 'exact_bank_slot_and_object'],
        ['bank_deposit_x', 'exact_side_slot_and_object'],
        ['numeric_bankbooth', 'loc_origin_bounds_and_current_loc_definition'],
        ['numeric_bankbooth', 'operation_2_is_selected_and_visible'],
        ['knife_arm', 'exact_knife_and_log_inventory_references'],
        ['accept_character_design', 'valid_appearance_submission_precedes_accept_button'],
    ]) {
        const condition = catalog.entries.find(entry => entry.key === key)?.server_predicates.find(condition => condition.name === name);
        assert.ok(condition, `${key}: missing ${name}`);
        assert.equal(condition.basis, 'host-policy', `${key}: unchecked engine assertion`);
    }
    const checkedConditionSources = new Map<string, readonly string[]>([
        ['bank_close_terminal:stop_bank_main_and_side_inventory_listeners', ['scripts/interface_bank/scripts/bank.rs2', 'src/engine/entity/Player.ts']],
        ['bank_close_terminal:queue_bank_reorganization', ['scripts/interface_bank/scripts/bank.rs2']],
        ['bank_close_terminal:clear_weak_queue', ['src/engine/entity/Player.ts']],
        ['bank_close_terminal:drop_protect_when_not_delayed', ['src/engine/entity/Player.ts']],
        ['bank_close_terminal:cancel_suspended_count_or_pause_script', ['src/engine/entity/Player.ts']],
        ['bank_close_terminal:run_expected_bank_if_close_trigger', ['scripts/interface_bank/scripts/bank.rs2', 'src/engine/entity/Player.ts']],
        ['bank_open_deposit:banker_player_text_is_one_selected_font_page', [
            'pack/mesanim.pack',
            'scripts/interface_bank/scripts/banker.rs2',
            'scripts/interface_chat/scripts/chat.rs2',
            'src/cache/config/FontType.ts',
            'src/cache/config/MesanimType.ts',
            'src/engine/script/handlers/StringOps.ts',
            'src/network/game/client/handler/ResumePauseButtonHandler.ts',
        ]],
    ]);
    for (const entry of catalog.entries) {
        for (const condition of [...entry.server_predicates, ...entry.outcome, ...entry.cleanup_effects]) {
            const expectedFiles = checkedConditionSources.get(`${entry.key}:${condition.name}`);
            assert.equal(condition.basis, expectedFiles ? 'checked' : 'host-policy', `${entry.key}: ${condition.name} authority`);
            const sourceFiles = [...new Set(condition.sources.map(source => source.file))].sort();
            assert.deepEqual(sourceFiles, expectedFiles ? [...expectedFiles].sort() : [], `${entry.key}: ${condition.name} source spans`);
        }
        for (const edge of entry.predecessors) {
            if (entry.key === 'bank_withdraw_x' || entry.key === 'bank_deposit_x') {
                assert.equal(edge.basis, 'checked');
                assert.equal(edge.synchronous, true);
                assert.ok(edge.sources.some(source => source.file === COUNT_RESUME_ENGINE_SOURCE));
                assert.ok(edge.sources.some(source => source.file.startsWith('scripts/interface_bank/')));
            } else {
                assert.equal(edge.basis, 'host-policy');
                assert.equal(edge.synchronous, false, 'host send order must not assert server synchrony');
                assert.deepEqual(edge.sources, []);
            }
        }
    }
} finally {
    fs.rmSync(success.root, { recursive: true, force: true });
}

const refusals: readonly [string, (sample: ServerOrderedTestFixture) => void, RegExp][] = [
    ['source pin disagreement', sample => { sample.input.sourceCommits.engine_commit = 'e'.repeat(40); }, /selected pin disagrees with source roots/],
    ['unproved revision', sample => { sample.input.selectedPack.revision += 1; }, /ContractUnavailable for unproved revision/],
    ['missing packed alias', sample => {
        sample.files.set('pack/interface.pack', sample.files.get('pack/interface.pack')!.replace(/^\d+=bank_main:inv\n/m, ''));
        sample.save();
    }, /missing or invalid interface alias bank_main:inv/],
    ['ambiguous packed alias', sample => {
        sample.files.set('pack/interface.pack', sample.files.get('pack/interface.pack')! + '9999=bank_main:inv\n');
        sample.save();
    }, /ambiguous interface alias bank_main:inv/],
    ['wrong root ancestry', sample => {
        const inventory = sample.input.cache.components.find(component => component.name === 'bank_main:inv')!;
        inventory.root_layer += 1;
    }, /root ancestry for bank_main:inv mismatch/],
    ['wrong authored component type', sample => {
        const inventory = sample.input.cache.components.find(component => component.name === 'bank_main:inv')!;
        inventory.component_type = 3;
    }, /component type for bank_main:inv mismatch/],
    ['wrong button type', sample => {
        const accept = sample.input.cache.components.find(component => component.name === 'player_kit:accept')!;
        accept.button_type = 2;
    }, /button type for player_kit:accept mismatch/],
    ['wrong client code', sample => {
        const accept = sample.input.cache.components.find(component => component.name === 'player_kit:accept')!;
        accept.client_code = 0;
    }, /client code for player_kit:accept mismatch/],
    ['changed same-pass skill guard', sample => {
        sample.files.set('scripts/interface_chat/scripts/chat.rs2', sample.files.get('scripts/interface_chat/scripts/chat.rs2')!.replace('p_delay(0);', 'p_delay(1);'));
        sample.save();
    }, /skill_multi3 close-clock guard changed/],
    ['removed closeModal weak-queue clear', sample => {
        sample.input.engineSources[PLAYER_ENGINE_SOURCE] = sample.input.engineSources[PLAYER_ENGINE_SOURCE].replace('this.weakQueue.clear();', 'this.weakQueue.clearAll();');
    }, /Player\.closeModal queue or protection effects changed/],
    ['changed closeModal protection effect', sample => {
        sample.input.engineSources[PLAYER_ENGINE_SOURCE] = sample.input.engineSources[PLAYER_ENGINE_SOURCE].replace('this.protect = false;', 'this.protect = true;');
    }, /Player\.closeModal queue or protection effects changed/],
    ['changed closeModal suspended script effect', sample => {
        sample.input.engineSources[PLAYER_ENGINE_SOURCE] = sample.input.engineSources[PLAYER_ENGINE_SOURCE].replace('this.activeScript = null;', 'this.activeScript = undefined;');
    }, /Player\.closeModal suspended-script effects changed/],
    ['changed closeModal resume-button effect', sample => {
        sample.input.engineSources[PLAYER_ENGINE_SOURCE] = sample.input.engineSources[PLAYER_ENGINE_SOURCE].replace('this.resumeButtons = [];', 'this.resumeButtons = [1];');
    }, /Player\.closeModal suspended-script effects changed/],
    ['changed closeModal IF_CLOSE trigger', sample => {
        sample.input.engineSources[PLAYER_ENGINE_SOURCE] = sample.input.engineSources[PLAYER_ENGINE_SOURCE].replace('ServerTriggerType.IF_CLOSE', 'ServerTriggerType.OPLOC1');
    }, /Player\.closeModal IF_CLOSE or listener effects changed/],
    ['changed closeModal listener cleanup', sample => {
        sample.input.engineSources[PLAYER_ENGINE_SOURCE] = sample.input.engineSources[PLAYER_ENGINE_SOURCE].replace('this.clearComListeners(this.modalMain);', 'this.clearComListeners(this.modalChat);');
    }, /Player\.closeModal IF_CLOSE or listener effects changed/],
    ['changed count-resume engine state guard', sample => {
        sample.input.engineSources[COUNT_RESUME_ENGINE_SOURCE] = sample.input.engineSources[COUNT_RESUME_ENGINE_SOURCE].replace('ScriptState.COUNTDIALOG', 'ScriptState.PAUSEBUTTON');
    }, /selected engine count-resume handler changed/],
    ['delayed withdraw count label', sample => {
        const file = 'scripts/interface_bank/scripts/bank.rs2';
        sample.files.set(file, sample.files.get(file)!.replace('p_countdialog;', 'p_delay(1);\np_countdialog;'));
        sample.save();
    }, /bank_withdraw count prompt is not a synchronous label path/],
    ['delayed deposit count label', sample => {
        const file = 'scripts/interface_bank/scripts/bank_deposit.rs2';
        sample.files.set(file, sample.files.get(file)!.replace('p_countdialog;', 'p_delay(1);\np_countdialog;'));
        sample.save();
    }, /bank_deposit count prompt is not a synchronous label path/],
    ['queued deposit count label', sample => {
        const file = 'scripts/interface_bank/scripts/bank_deposit.rs2';
        sample.files.set(file, sample.files.get(file)!.replace('p_countdialog;', 'queue(count_work, 0, 0);\np_countdialog;'));
        sample.save();
    }, /bank_deposit count prompt is not a synchronous label path/],
    ['unchecked withdraw call before count prompt', sample => {
        const file = 'scripts/interface_bank/scripts/bank.rs2';
        sample.files.set(file, sample.files.get(file)!.replace('p_countdialog;', '~unchecked_count_helper();\np_countdialog;'));
        sample.save();
    }, /bank_withdraw count prompt is not a synchronous label path/],
];

for (const [label, mutate, expected] of refusals) {
    const sample = fixture();
    try {
        mutate(sample);
        assert.throws(() => extract(sample), expected, label);
    } finally {
        fs.rmSync(sample.root, { recursive: true, force: true });
    }
}
const bankerPageFailures: readonly [string, (sample: ServerOrderedTestFixture) => void][] = [
    ['changed banker text', sample => {
        sample.files.set('scripts/interface_bank/scripts/banker.rs2', sample.files.get('scripts/interface_bank/scripts/banker.rs2')!.replace(BANKER_TEXT, 'Changed bank text'));
        sample.save();
    }],
    ['changed banker choice path', sample => {
        sample.files.set('scripts/interface_chat/scripts/chat.rs2', sample.files.get('scripts/interface_chat/scripts/chat.rs2')!.replace('case multi2:com_1 :', 'case multi2:com_2 :'));
        sample.save();
    }],
    ['delayed banker page continuation', sample => {
        sample.files.set('scripts/interface_chat/scripts/chat.rs2', sample.files.get('scripts/interface_chat/scripts/chat.rs2')!.replace('split_init($string, 380, 4, q8_full);', 'split_init($string, 380, 4, q8_full);\np_delay(1);'));
        sample.save();
    }],
    ['failed selected-font page proof', sample => {
        sample.input.cache.fonts[0].split = () => ['a', 'b', 'c', 'd', 'e'];
    }],
];

for (const [label, mutate] of bankerPageFailures) {
    const sample = fixture();
    try {
        mutate(sample);
        assertBankerProofFailureKeepsIndependentEntries(sample, label);
    } finally {
        fs.rmSync(sample.root, { recursive: true, force: true });
    }
}

console.log('server_ordered source and selected-cache regressions passed');
