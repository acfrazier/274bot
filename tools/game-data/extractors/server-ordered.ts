import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { requireGatherText } from './common.ts';
import { parseRs2Blocks, stripComment, type Rs2Block } from './gathering-content.ts';
import { parseBody } from './gathering-rs2.ts';
import { resolveInterfaceAliases, type InterfaceAliasIdentity } from './dialogue-ui.ts';

export const SERVER_ORDERED_SCHEMA = 1;
export const SERVER_ORDERED_REVISION = 289;

export const serverOrderedContentFiles = [
    'pack/category.pack',
    'pack/interface.pack',
    'pack/loc.pack',
    'pack/mesanim.pack',
    'pack/obj.pack',
    'pack/varp.pack',
    'scripts/interface_bank/configs/bank_booth.loc',
    'scripts/interface_bank/interfaces/bank_main.if',
    'scripts/interface_bank/interfaces/bank_side.if',
    'scripts/interface_bank/scripts/bank.rs2',
    'scripts/interface_bank/scripts/bank_booth.rs2',
    'scripts/interface_bank/scripts/bank_deposit.rs2',
    'scripts/interface_bank/scripts/banker.rs2',
    'scripts/interface_chat/interfaces/chat1.if',
    'scripts/interface_chat/interfaces/chat2.if',
    'scripts/interface_chat/interfaces/chat3.if',
    'scripts/interface_chat/interfaces/chat4.if',
    'scripts/interface_chat/interfaces/multi2.if',
    'scripts/interface_chat/interfaces/skill_multi3.if',
    'scripts/interface_chat/scripts/chat.rs2',
    'scripts/skill_cooking/scripts/gnome_cooking/cutting_fruit.rs2',
    'scripts/skill_firemaking/configs/firemaking.obj',
    'scripts/skill_fletching/configs/cut_logs/cut_logs.dbrow',
    'scripts/skill_fletching/scripts/cut_logs.rs2',
    'scripts/tutorial/interfaces/player_kit.if',
    'scripts/tutorial/scripts/tutorial.rs2',
] as const;

/** Engine source files that define the selected binary decoders and split behavior. */
export const serverOrderedDecoderSources = [
    'src/cache/config/Component.ts',
    'src/cache/config/FontType.ts',
    'src/cache/config/MesanimType.ts',
    'src/cache/config/ConfigType.ts',
    'src/io/Jagfile.ts',
    'src/io/Packet.ts',
    'src/io/BZip2.ts',
    'src/engine/script/handlers/StringOps.ts',
    'src/engine/script/ScriptOpcode.ts',
    'src/engine/script/ScriptValidators.ts',
] as const;

/** Other engine source files that define the named server predicates and close effects. */
export const serverOrderedEngineSourceFiles = [
    ...serverOrderedDecoderSources,
    'src/engine/entity/Player.ts',
    'src/network/game/client/ClientGameProt.ts',
    'src/network/game/client/handler/CloseModalHandler.ts',
    'src/network/game/client/handler/IdkSaveDesignHandler.ts',
    'src/network/game/client/handler/IfButtonHandler.ts',
    'src/network/game/client/handler/InvButtonHandler.ts',
    'src/network/game/client/handler/OpHeldUHandler.ts',
    'src/network/game/client/handler/OpLocHandler.ts',
    'src/network/game/client/handler/ResumePCountDialogHandler.ts',
    'src/network/game/client/handler/ResumePauseButtonHandler.ts',
] as const;

/** Cache files consumed by Component, FontType, and MesanimType. */
export const serverOrderedEngineCacheFiles = [
    'data/pack/client/interface',
    'data/pack/server/interface.dat',
    'data/pack/client/title',
    'data/pack/server/mesanim.dat',
] as const;
export const serverOrderedEngineInputPaths = [
    ...serverOrderedEngineSourceFiles,
    ...serverOrderedEngineCacheFiles,
] as const;


export type ServerOrderedSourceSpan = { file: string; first: number; last: number };
export type ServerOrderedSelection = {
    revision: number;
    engine_commit: string;
    content_commit: string;
    cache_id: string;
    content_id: string;
};

export type ServerOrderedComponentType = 'layer' | 'unused' | 'inventory' | 'rect' | 'text' | 'sprite' | 'model' | 'inventory_text';
export type ServerOrderedButtonType = 'none' | 'button' | 'target' | 'close' | 'toggle' | 'select' | 'pause';
export type ServerOrderedComponentProperties =
    | { kind: 'layer'; scroll: number; hidden: boolean; child_ids: number[] }
    | { kind: 'unused' }
    | { kind: 'inventory'; draggable: boolean; operable: boolean; usable: boolean; swappable: boolean; margin_x: number; margin_y: number; actions: (string | null)[]; action_target: number }
    | { kind: 'rect'; fill: boolean }
    | { kind: 'text'; centered: boolean; font_id: number; shadowed: boolean; text: string | null; active_text: string | null }
    | { kind: 'sprite'; graphic: string | null; active_graphic: string | null }
    | { kind: 'model'; model: number; active_model: number; anim: number; active_anim: number; zoom: number; xan: number; yan: number }
    | { kind: 'inventory_text'; centered: boolean; font_id: number; shadowed: boolean; colour: number; margin_x: number; margin_y: number; operable: boolean; actions: (string | null)[] };

export type ServerOrderedComponent = {
    alias: string;
    id: number;
    root_alias: string;
    root_id: number;
    parent_id: number | null;
    /** Direct parent first, then each ancestor through root_id. */
    ancestor_ids: number[];
    component_type: ServerOrderedComponentType;
    button_type: ServerOrderedButtonType;
    client_code: number;
    width: number;
    height: number;
    properties: ServerOrderedComponentProperties;
    source: ServerOrderedSourceSpan[];
};

export type ServerOrderedEntryKey =
    | 'knife_arm'
    | 'bank_withdraw_x'
    | 'bank_deposit_x'
    | 'bank_deposit_all'
    | 'bank_open_deposit'
    | 'bank_close_terminal'
    | 'numeric_bankbooth'
    | 'accept_character_design';

/** Only checked declarations are selected-source facts. Host policy can contain unproved engine assumptions. */
export type ServerOrderedBasis = 'checked' | 'host-policy';

export type ServerOrderedCondition = {
    name: string;
    basis: ServerOrderedBasis;
    sources: ServerOrderedSourceSpan[];
};
type ServerOrderedCloseModalFacts = {
    sources: ServerOrderedSourceSpan[];
    cleanup_effect_sources: {
        stop_bank_main_and_side_inventory_listeners: ServerOrderedSourceSpan[];
        queue_bank_reorganization: ServerOrderedSourceSpan[];
        clear_weak_queue: ServerOrderedSourceSpan[];
        drop_protect_when_not_delayed: ServerOrderedSourceSpan[];
        cancel_suspended_count_or_pause_script: ServerOrderedSourceSpan[];
        run_expected_bank_if_close_trigger: ServerOrderedSourceSpan[];
    };
};

export type ServerOrderedEntry = {
    key: ServerOrderedEntryKey;
    components: string[];
    server_predicates: ServerOrderedCondition[];
    predecessors: { from: string; to: string; basis: ServerOrderedBasis; synchronous: boolean; sources: ServerOrderedSourceSpan[] }[];
    max_packet_cost: number;
    outcome: ServerOrderedCondition[];
    deadline_ticks: number | null;
    recovery_count: number;
    cleanup_effects: ServerOrderedCondition[];
    sources: ServerOrderedSourceSpan[];
};

export type ServerOrderedBank = {
    mode_varp: { alias: string; id: number };
    item_mode: number;
    note_mode: number;
    main_component: { alias: string; id: number };
    side_component: { alias: string; id: number };
    withdraw_x_op: number;
    deposit_x_op: number;
    deposit_all_op: number;
    sources: ServerOrderedSourceSpan[];
};

export type ServerOrderedNumericLoc = {
    alias: string;
    id: number;
    operation: number;
    action: string;
    sources: ServerOrderedSourceSpan[];
};

// One selected content identity joined from its pack alias to its packed id.
export type ServerOrderedContractIdentity = {
    alias: string;
    id: number;
};

// The selected knife/use-item recipe. The logs allowlist holds exactly the
// dbrow rows that reach the skill_multi3 Make path (a declared shafts count
// of at least one); no other item can enter the Make request through this fact.
export type ServerOrderedKnife = {
    knife: ServerOrderedContractIdentity;
    logs: ServerOrderedContractIdentity[];
    sources: ServerOrderedSourceSpan[];
};

export type ServerOrderedBankerPage = {
    font_alias: string;
    font_id: number;
    mesanim_alias: string;
    mesanim_id: number;
    line_width: number;
    line_count: number;
    max_lines: number;
    page_count: number;
    split_lines: string[];
    text: string;
    page_root_alias: string;
    continue_component_alias: string;
    sources: ServerOrderedSourceSpan[];
};

export type ServerOrderedCatalog = {
    schema: 1;
    revision: number;
    engine_commit: string;
    content_commit: string;
    cache_id: string;
    content_id: string;
    components: ServerOrderedComponent[];
    entries: ServerOrderedEntry[];
    banker_page: ServerOrderedBankerPage | null;
    bank: ServerOrderedBank;
    numeric_locs: ServerOrderedNumericLoc[];
    knife: ServerOrderedKnife;
};

/** A small decoded view of one real engine Component. */
export type ServerOrderedCacheComponent = {
    id: number;
    name: string | null;
    root_layer: number;
    component_type: number;
    button_type: number;
    client_code: number;
    width: number;
    height: number;
    margin_x: number;
    margin_y: number;
    child_ids: number[] | null;
    scroll: number;
    hidden: boolean;
    draggable: boolean;
    operable: boolean;
    usable: boolean;
    swappable: boolean;
    actions: (string | null)[] | null;
    action_verb: string | null;
    action: string | null;
    action_target: number;
    fill: boolean;
    centered: boolean;
    font_id: number;
    shadowed: boolean;
    text: string | null;
    active_text: string | null;
    colour: number;
    active_colour: number;
    over_colour: number;
    active_over_colour: number;
    graphic: string | null;
    active_graphic: string | null;
    model: number;
    active_model: number;
    anim: number;
    active_anim: number;
    zoom: number;
    xan: number;
    yan: number;
    option: string | null;
};

export type ServerOrderedCacheFont = {
    alias: string;
    id: number;
    split(text: string, maxWidth: number): string[];
};

export type ServerOrderedCacheMesanim = { alias: string; id: number };
export type ServerOrderedCacheSnapshot = {
    components: readonly ServerOrderedCacheComponent[];
    fonts: readonly ServerOrderedCacheFont[];
    mesanims: readonly ServerOrderedCacheMesanim[];
};

export type ServerOrderedEngineSources = {
    'src/engine/entity/Player.ts': string;
    'src/network/game/client/handler/ResumePCountDialogHandler.ts': string;
};

export type ServerOrderedExtractInput = {
    contentRoot: string;
    selectedPack: ServerOrderedSelection;
    sourceCommits: { engine_commit: string; content_commit: string };
    cache: ServerOrderedCacheSnapshot;
    engineSources: ServerOrderedEngineSources;
};

export type ServerOrderedLoadInput = {
    contentRoot: string;
    engineRoot: string;
    revision: number;
    engineCommit: string;
    contentCommit: string;
    cacheId: string;
    contentId: string;
};

export type ServerOrderedBankFacts = {
    bank: ServerOrderedBank;
    spans: ServerOrderedSourceSpan[];
    countPromptSynchronous: { withdraw: boolean; deposit: boolean };
};

export type ServerOrderedBankBoothFacts = {
    numericLoc: ServerOrderedNumericLoc;
};

export type ServerOrderedCharacterAcceptanceFacts = {
    spans: ServerOrderedSourceSpan[];
};

const COMPONENT_SOURCE = 'src/cache/config/Component.ts';
const FONT_SOURCE = 'src/cache/config/FontType.ts';
const MESANIM_SOURCE = 'src/cache/config/MesanimType.ts';
const STRING_OPS_SOURCE = 'src/engine/script/handlers/StringOps.ts';
const BANKER_SOURCE = 'scripts/interface_bank/scripts/banker.rs2';
const CHAT_SOURCE = 'scripts/interface_chat/scripts/chat.rs2';
const BANK_SOURCE = 'scripts/interface_bank/scripts/bank.rs2';
const BANK_DEPOSIT_SOURCE = 'scripts/interface_bank/scripts/bank_deposit.rs2';
const BANK_BOOTH_SOURCE = 'scripts/interface_bank/scripts/bank_booth.rs2';
const BANK_BOOTH_LOC_SOURCE = 'scripts/interface_bank/configs/bank_booth.loc';
const BANK_MAIN_INTERFACE = 'scripts/interface_bank/interfaces/bank_main.if';
const PLAYER_KIT_INTERFACE = 'scripts/tutorial/interfaces/player_kit.if';
const TUTORIAL_SOURCE = 'scripts/tutorial/scripts/tutorial.rs2';
const CUT_LOGS_SOURCE = 'scripts/skill_fletching/scripts/cut_logs.rs2';
const PLAYER_SOURCE = 'src/engine/entity/Player.ts';
const RESUME_P_COUNT_DIALOG_SOURCE = 'src/network/game/client/handler/ResumePCountDialogHandler.ts';
const BANKER_TEXT = "I'd like to access my bank account, please.";
const BANKER_MESANIM = 'quiz';
const BANKER_FONT = 'q8_full';
const CHAT_LINE_WIDTH = 380;
const CHAT_MAX_LINES = 4;
type ServerOrderedRecipePolicy = Pick<
    ServerOrderedEntry,
    'server_predicates' | 'max_packet_cost' | 'outcome' | 'deadline_ticks' | 'recovery_count' | 'cleanup_effects'
>;

/** Host-policy declarations have no selected-source citations. */
function hostPolicyConditions(names: string[]): ServerOrderedCondition[] {
    return names.map(name => ({ name, basis: 'host-policy', sources: [] }));
}

function checkedCondition(name: string, sources: ServerOrderedSourceSpan[]): ServerOrderedCondition {
    const citations = mergeSpans(sources);
    if (citations.length === 0) throw new Error(`server_ordered: checked condition ${name} has no source spans`);
    return { name, basis: 'checked', sources: citations };
}

/** Budgets and conditions are host policy. Engine assumptions here remain explicitly unproved. */
const SERVER_ORDERED_RECIPE_POLICY = {
    knife_arm: {
        server_predicates: hostPolicyConditions(['exact_knife_and_log_inventory_references', 'skill_multi3_not_observed_open', 'weakqueue_map_clock_differs_from_current_map_clock', 'same_pass_close_guard_is_not_a_successful_arm', 'selected_log_recipe_and_membership_checks_remain_server_owned']),
        max_packet_cost: 4,
        outcome: hostPolicyConditions(['fresh_arm_and_roll_evidence', 'experience_or_product_and_inventory_change', 'content_message_when_present']),
        deadline_ticks: null,
        recovery_count: 1,
        cleanup_effects: hostPolicyConditions(['accepted_resource_click_may_clear_weak_work', 'skill_menu_cleanup_is_not_close_modal', 'refused_target_click_does_not_prove_cleanup']),
    },
    bank_withdraw_x: {
        server_predicates: hostPolicyConditions(['current_bank_generation', 'exact_bank_slot_and_object', 'positive_count_delta', 'observed_bankcert_equals_expected_mode', 'withdrawn_item_or_note_form_matches_bankcert']),
        max_packet_cost: 2,
        outcome: hostPolicyConditions(['exact_transfer_delta_and_expected_item_or_note_form', 'named_terminal_bank_failure_message']),
        deadline_ticks: 2,
        recovery_count: 1,
        cleanup_effects: hostPolicyConditions(['one_owned_empty_enter_only_after_correlated_prompt_and_outcome', 'confirm_local_prompt_closed_before_next_bank_input', 'never_replay_count']),
    },
    bank_deposit_x: {
        server_predicates: hostPolicyConditions(['current_bank_generation', 'exact_side_slot_and_object', 'positive_count_delta', 'item_is_bankable']),
        max_packet_cost: 2,
        outcome: hostPolicyConditions(['exact_transfer_delta_and_expected_item_or_note_form', 'named_terminal_bank_failure_message']),
        deadline_ticks: 2,
        recovery_count: 1,
        cleanup_effects: hostPolicyConditions(['one_owned_empty_enter_only_after_correlated_prompt_and_outcome', 'confirm_local_prompt_closed_before_next_bank_input', 'never_replay_count']),
    },
    bank_deposit_all: {
        server_predicates: hostPolicyConditions(['current_bank_generation', 'exact_side_listener', 'depositable_inventory_items']),
        max_packet_cost: 1,
        outcome: hostPolicyConditions(['matching_held_loss_and_bank_increase', 'named_terminal_bank_failure_message']),
        deadline_ticks: null,
        recovery_count: 0,
        cleanup_effects: hostPolicyConditions(['no_count_prompt_recovery']),
    },
    bank_open_deposit: {
        server_predicates: hostPolicyConditions(['starting_multi2_choice_page_was_received', 'choice_1_is_exact_if_button_component', 'banker_player_text_is_one_selected_font_page', 'new_bank_generation_and_side_listener_precede_deposit']),
        max_packet_cost: 3,
        outcome: hostPolicyConditions(['matching_held_loss_and_bank_increase_after_side_listener', 'named_terminal_bank_failure_message']),
        deadline_ticks: 2,
        recovery_count: 1,
        cleanup_effects: hostPolicyConditions(['failed_chain_requires_ordered_chat_reset_before_reusing_page', 'no_blind_booth_open_sequence']),
    },
    bank_close_terminal: {
        server_predicates: hostPolicyConditions(['expected_modal_is_bank_main', 'owned_reason_is_terminal_bank_exit']),
        max_packet_cost: 1,
        outcome: hostPolicyConditions(['observed_modal_and_prompt_closure', 'named_bank_if_close_effects_when_observable']),
        deadline_ticks: null,
        recovery_count: 1,
        cleanup_effects: hostPolicyConditions(['stop_bank_main_and_side_inventory_listeners', 'queue_bank_reorganization', 'clear_weak_queue', 'drop_protect_when_not_delayed', 'cancel_suspended_count_or_pause_script', 'run_expected_bank_if_close_trigger']),
    },
    numeric_bankbooth: {
        server_predicates: hostPolicyConditions(['exact_bankbooth_loc_identity_and_tile', 'operation_2_is_selected_and_visible', 'loc_origin_bounds_and_current_loc_definition']),
        max_packet_cost: 2,
        outcome: hostPolicyConditions(['observed_bank_main_and_side_open']),
        deadline_ticks: null,
        recovery_count: 0,
        cleanup_effects: hostPolicyConditions(['no_automatic_retry_or_close']),
    },
    accept_character_design: {
        server_predicates: hostPolicyConditions(['valid_appearance_submission_precedes_accept_button', 'saved_appearance_evidence_required', 'tutorial_signal_alone_is_not_success']),
        max_packet_cost: 2,
        outcome: hostPolicyConditions(['saved_appearance_evidence', 'expected_tutorial_progress_signal']),
        deadline_ticks: null,
        recovery_count: 0,
        cleanup_effects: hostPolicyConditions(['accept_if_close_queues_tutorial_advancement', 'never_recover_failed_save_with_general_close']),
    },
} satisfies Record<ServerOrderedEntryKey, ServerOrderedRecipePolicy>;

type ServerOrderedDispatchEdge = ServerOrderedEntry['predecessors'][number];

function hostOrder(from: string, to: string): ServerOrderedDispatchEdge {
    return { from, to, basis: 'host-policy', synchronous: false, sources: [] };
}

/** Reviewed host emission order. These edges do not claim atomic engine execution. */
const SERVER_ORDERED_DISPATCH_ORDER_POLICY = {
    knife_arm: [
        hostOrder('OPHELDU', 'IF_BUTTON(skill_multi3:make1_a)'),
        hostOrder('IF_BUTTON(skill_multi3:make1_a)', 'MOVE_GAMECLICK'),
        hostOrder('MOVE_GAMECLICK', 'OPLOC_OR_OPNPC'),
    ],
    accept_character_design: [
        hostOrder('IDK_SAVEDESIGN', 'IF_BUTTON(player_kit:accept)'),
    ],
} satisfies Pick<Record<ServerOrderedEntryKey, ServerOrderedDispatchEdge[]>, 'knife_arm' | 'accept_character_design'>;

const interfaceFilesByRoot: Record<string, string> = {
    bank_main: BANK_MAIN_INTERFACE,
    bank_side: 'scripts/interface_bank/interfaces/bank_side.if',
    multi2: 'scripts/interface_chat/interfaces/multi2.if',
    skill_multi3: 'scripts/interface_chat/interfaces/skill_multi3.if',
    player_kit: PLAYER_KIT_INTERFACE,
    chat1: 'scripts/interface_chat/interfaces/chat1.if',
    chat2: 'scripts/interface_chat/interfaces/chat2.if',
    chat3: 'scripts/interface_chat/interfaces/chat3.if',
    chat4: 'scripts/interface_chat/interfaces/chat4.if',
};

const baseComponentAliases = [
    'bank_main',
    'bank_main:com_93',
    'bank_main:com_94',
    'bank_main:com_96',
    'bank_main:com_98',
    'bank_main:inv',
    'bank_side',
    'bank_side:inv',
    'multi2',
    'multi2:com_1',
    'player_kit',
    'player_kit:accept',
    'skill_multi3',
    'skill_multi3:make1_a',
] as const;

const componentTypeNames: Record<number, ServerOrderedComponentType> = {
    0: 'layer',
    1: 'unused',
    2: 'inventory',
    3: 'rect',
    4: 'text',
    5: 'sprite',
    6: 'model',
    7: 'inventory_text',
};
const componentTypeValues: Record<string, number> = {
    layer: 0,
    unused: 1,
    inv: 2,
    rect: 3,
    text: 4,
    graphic: 5,
    model: 6,
    invtext: 7,
};
const buttonTypeNames: Record<number, ServerOrderedButtonType> = {
    0: 'none',
    1: 'button',
    2: 'target',
    3: 'close',
    4: 'toggle',
    5: 'select',
    6: 'pause',
};
const buttonTypeValues: Record<string, number> = {
    normal: 1,
    target: 2,
    close: 3,
    toggle: 4,
    select: 5,
    pause: 6,
};

function span(file: string, first: number, last = first): ServerOrderedSourceSpan {
    return { file, first, last };
}

function mergeSpans(spans: readonly ServerOrderedSourceSpan[]): ServerOrderedSourceSpan[] {
    const unique = new Map<string, ServerOrderedSourceSpan>();
    for (const source of spans) unique.set(`${source.file}:${source.first}:${source.last}`, source);
    return [...unique.values()].sort((a, b) => a.file.localeCompare(b.file) || a.first - b.first || a.last - b.last);
}

function assertSelection(selection: ServerOrderedSelection, sourceCommits: { engine_commit: string; content_commit: string }) {
    if (selection.revision !== SERVER_ORDERED_REVISION) {
        throw new Error(`server_ordered: ContractUnavailable for unproved revision ${selection.revision}`);
    }
    if (!/^[0-9a-f]{40}$/.test(selection.engine_commit) || !/^[0-9a-f]{40}$/.test(selection.content_commit)) {
        throw new Error('server_ordered: selected engine/content commit pin is missing or invalid');
    }
    if (!/^[0-9a-f]{64}$/.test(selection.cache_id) || !/^[0-9a-f]{64}$/.test(selection.content_id)) {
        throw new Error('server_ordered: selected cache/content identity is missing or invalid');
    }
    if (sourceCommits.engine_commit !== selection.engine_commit || sourceCommits.content_commit !== selection.content_commit) {
        throw new Error('server_ordered: selected pin disagrees with source roots');
    }
}

function readGitHead(root: string, label: string): string {
    try {
        return execFileSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
    } catch (error) {
        throw new Error(`server_ordered: cannot prove ${label} source commit`, { cause: error });
    }
}

function selectedPackIds(contentRoot: string): number[] {
    const text = requireGatherText(contentRoot, 'pack/interface.pack');
    const ids = new Set<number>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        const equals = line.indexOf('=');
        if (!line || line.startsWith('//') || equals <= 0) continue;
        const id = Number(line.slice(0, equals));
        if (Number.isInteger(id) && id >= 0) ids.add(id);
    }
    return [...ids].sort((a, b) => a - b);
}

type ServerOrderedFontSourceRow = { alias: string; id: number; line: number };

function readFontTypeRows(engineRoot: string): ServerOrderedFontSourceRow[] {
    const source = fs.readFileSync(path.join(engineRoot, FONT_SOURCE), 'utf8');
    const rows = [...source.matchAll(/FontType\.instances\[(\d+)\]\s*=\s*new FontType\(title,\s*'([A-Za-z0-9_]+)'/g)]
        .map(match => ({
            alias: match[2],
            id: Number(match[1]),
            line: source.slice(0, match.index).split(/\r?\n/).length,
        }));
    const aliases = new Set<string>();
    const ids = new Set<number>();
    for (const row of rows) {
        if (aliases.has(row.alias) || ids.has(row.id)) throw new Error('server_ordered: ambiguous FontType source mapping');
        aliases.add(row.alias);
        ids.add(row.id);
    }
    if (!rows.some(row => row.alias === BANKER_FONT)) throw new Error(`server_ordered: FontType source has no ${BANKER_FONT} mapping`);
    return rows.sort((left, right) => left.id - right.id);
}


function resolvePackAlias(contentRoot: string, file: string, alias: string, family: string): InterfaceAliasIdentity {
    const matches: InterfaceAliasIdentity[] = [];
    for (const [index, raw] of requireGatherText(contentRoot, file).split(/\r?\n/).entries()) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const equals = line.indexOf('=');
        if (equals <= 0 || line.slice(equals + 1) !== alias) continue;
        matches.push({ id: Number(line.slice(0, equals)), line: index + 1 });
    }
    if (matches.length === 0 || (matches.length === 1 && (!Number.isInteger(matches[0].id) || matches[0].id <= 0))) {
        throw new Error(`${family}: missing or invalid ${file} alias ${alias}`);
    }
    if (matches.length !== 1) throw new Error(`${family}: ambiguous ${file} alias ${alias}`);
    return matches[0];
}

type InterfaceField = { value: string; line: number };
type InterfaceDefinition = { alias: string; file: string; first: number; last: number; fields: Map<string, InterfaceField[]> };

function parseInterfaceDefinitions(file: string, text: string): InterfaceDefinition[] {
    const definitions: InterfaceDefinition[] = [];
    let current: InterfaceDefinition | null = null;
    for (const [index, raw] of text.split(/\r?\n/).entries()) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const lineNumber = index + 1;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = { alias: line.slice(1, -1), file, first: lineNumber, last: lineNumber, fields: new Map() };
            definitions.push(current);
            continue;
        }
        if (!current) continue;
        current.last = lineNumber;
        const equals = line.indexOf('=');
        if (equals <= 0) continue;
        const key = line.slice(0, equals);
        const values = current.fields.get(key) ?? [];
        values.push({ value: line.slice(equals + 1), line: lineNumber });
        current.fields.set(key, values);
    }
    return definitions;
}

function definitionFor(contentRoot: string, alias: string, cache: Map<string, InterfaceDefinition[]>) {
    const colon = alias.indexOf(':');
    if (colon < 0) return null;
    const rootAlias = alias.slice(0, colon);
    const childAlias = alias.slice(colon + 1);
    const file = interfaceFilesByRoot[rootAlias];
    if (!file) throw new Error(`server_ordered: no authored interface source for ${rootAlias}`);
    let definitions = cache.get(file);
    if (!definitions) {
        definitions = parseInterfaceDefinitions(file, requireGatherText(contentRoot, file));
        cache.set(file, definitions);
    }
    const found = definitions.filter(row => row.alias === childAlias);
    if (found.length !== 1) throw new Error(`server_ordered: expected one authored interface component ${alias}`);
    for (const [key, values] of found[0].fields) {
        if (values.length !== 1) throw new Error(`server_ordered: ambiguous ${key} field for ${alias}`);
    }
    return found[0];
}

function interfaceField(definition: InterfaceDefinition | null, key: string): InterfaceField | undefined {
    return definition?.fields.get(key)?.[0];
}

function interfaceNumber(definition: InterfaceDefinition, key: string, fallback?: number): number {
    const field = interfaceField(definition, key);
    if (!field && fallback !== undefined) return fallback;
    if (!field || !/^-?\d+$/.test(field.value)) throw new Error(`server_ordered: invalid ${key} in ${definition.alias}`);
    return Number(field.value);
}

function optionalInterfaceNumber(definition: InterfaceDefinition, key: string): number | undefined {
    const field = interfaceField(definition, key);
    if (!field) return undefined;
    if (!/^-?\d+$/.test(field.value)) throw new Error(`server_ordered: invalid ${key} in ${definition.alias}`);
    return Number(field.value);
}

function interfaceBoolean(definition: InterfaceDefinition, key: string, fallback = false): boolean {
    const field = interfaceField(definition, key);
    if (!field) return fallback;
    if (field.value === 'yes') return true;
    if (field.value === 'no') return false;
    throw new Error(`server_ordered: invalid ${key} boolean in ${definition.alias}`);
}

function assertEqual<T>(actual: T, expected: T, label: string) {
    if (actual !== expected) throw new Error(`server_ordered: ${label} mismatch`);
}

function exactSourceField(definition: InterfaceDefinition | null, key: string): ServerOrderedSourceSpan[] {
    const field = interfaceField(definition, key);
    return field ? [span(definition!.file, field.line)] : [];
}

function sourceDefinitionSpans(definition: InterfaceDefinition | null): ServerOrderedSourceSpan[] {
    if (!definition) return [];
    return [span(definition.file, definition.first, definition.last)];
}

function rawComponentType(value: ServerOrderedCacheComponent): ServerOrderedComponentType {
    const type = componentTypeNames[value.component_type];
    if (!type) throw new Error(`server_ordered: unsupported decoded component type ${value.component_type}`);
    return type;
}

function rawButtonType(value: ServerOrderedCacheComponent): ServerOrderedButtonType {
    const type = buttonTypeNames[value.button_type];
    if (!type) throw new Error(`server_ordered: unsupported decoded button type ${value.button_type}`);
    return type;
}

function assertSourceComponent(
    alias: string,
    raw: ServerOrderedCacheComponent,
    definition: InterfaceDefinition | null,
    fonts: ReadonlyMap<string, number>,
) {
    const expectedType = definition
        ? componentTypeValues[interfaceField(definition, 'type')?.value ?? '']
        : componentTypeValues.layer;
    if (expectedType === undefined) throw new Error(`server_ordered: unsupported authored component type for ${alias}`);
    assertEqual(raw.component_type, expectedType, `component type for ${alias}`);

    const authoredButton = definition ? interfaceField(definition, 'buttontype')?.value : undefined;
    const expectedButton = authoredButton === undefined ? 0 : buttonTypeValues[authoredButton];
    if (expectedButton === undefined) throw new Error(`server_ordered: unsupported authored button type for ${alias}`);
    assertEqual(raw.button_type, expectedButton, `button type for ${alias}`);

    const expectedClientCode = definition ? interfaceNumber(definition, 'clientcode', 0) : 0;
    assertEqual(raw.client_code, expectedClientCode, `client code for ${alias}`);

    if (!definition) {
        assertEqual(raw.root_layer, raw.id, `root ancestry for ${alias}`);
        return;
    }

    const width = optionalInterfaceNumber(definition, 'width');
    const height = optionalInterfaceNumber(definition, 'height');
    if (width !== undefined) assertEqual(raw.width, width, `width for ${alias}`);
    if (height !== undefined) assertEqual(raw.height, height, `height for ${alias}`);

    switch (raw.component_type) {
        case 0:
            assertEqual(raw.scroll, interfaceNumber(definition, 'scroll', 0), `scroll for ${alias}`);
            assertEqual(raw.hidden, interfaceBoolean(definition, 'hide'), `hidden flag for ${alias}`);
            break;
        case 2: {
            assertEqual(raw.draggable, interfaceBoolean(definition, 'draggable'), `draggable flag for ${alias}`);
            assertEqual(raw.operable, interfaceBoolean(definition, 'operable'), `operable flag for ${alias}`);
            assertEqual(raw.usable, interfaceBoolean(definition, 'usable'), `usable flag for ${alias}`);
            assertEqual(raw.swappable, interfaceBoolean(definition, 'swappable'), `swappable flag for ${alias}`);
            const options = [1, 2, 3, 4, 5].map(index => interfaceField(definition, `option${index}`)?.value ?? '');
            if (!raw.actions || raw.actions.length !== 5 || options.some((value, index) => value !== raw.actions![index])) {
                throw new Error(`server_ordered: inventory actions mismatch for ${alias}`);
            }
            const margin = interfaceField(definition, 'margin')?.value;
            if (margin !== undefined) {
                const parts = margin.split(',');
                if (parts.length !== 2 || !parts.every(value => /^-?\d+$/.test(value))) throw new Error(`server_ordered: invalid margin for ${alias}`);
                assertEqual(raw.margin_x, Number(parts[0]), `horizontal margin for ${alias}`);
                assertEqual(raw.margin_y, Number(parts[1]), `vertical margin for ${alias}`);
            }
            break;
        }
        case 3:
            assertEqual(raw.fill, interfaceBoolean(definition, 'fill'), `fill for ${alias}`);
            break;
        case 4: {
            assertEqual(raw.centered, interfaceBoolean(definition, 'center'), `centered flag for ${alias}`);
            assertEqual(raw.shadowed, interfaceBoolean(definition, 'shadowed'), `shadowed flag for ${alias}`);
            // Component.ts:163-164 decodes missing authored text as an empty gjstr.
            assertEqual(raw.text, interfaceField(definition, 'text')?.value ?? '', `text for ${alias}`);
            assertEqual(raw.active_text, interfaceField(definition, 'activetext')?.value ?? '', `active text for ${alias}`);
            const fontAlias = interfaceField(definition, 'font')?.value;
            if (fontAlias !== undefined) {
                const expectedFont = fonts.get(fontAlias);
                if (expectedFont === undefined) throw new Error(`server_ordered: unproved selected font ${fontAlias} for ${alias}`);
                assertEqual(raw.font_id, expectedFont, `font for ${alias}`);
            }
            break;
        }
        case 5:
            assertEqual(raw.graphic, interfaceField(definition, 'graphic')?.value ?? '', `graphic for ${alias}`);
            assertEqual(raw.active_graphic, interfaceField(definition, 'activegraphic')?.value ?? '', `active graphic for ${alias}`);
            break;
        case 6:
        case 7:
        case 1:
            break;
        default:
            throw new Error(`server_ordered: unsupported component properties for ${alias}`);
    }

    const option = interfaceField(definition, 'option')?.value;
    if (option !== undefined) assertEqual(raw.option, option, `button option for ${alias}`);
}

function componentProperties(raw: ServerOrderedCacheComponent): ServerOrderedComponentProperties {
    switch (raw.component_type) {
        case 0:
            if (!raw.child_ids) throw new Error(`server_ordered: layer ${raw.id} has no decoded children`);
            return { kind: 'layer', scroll: raw.scroll, hidden: raw.hidden, child_ids: [...raw.child_ids] };
        case 1:
            return { kind: 'unused' };
        case 2:
            if (!raw.actions) throw new Error(`server_ordered: inventory ${raw.id} has no decoded actions`);
            return {
                kind: 'inventory', draggable: raw.draggable, operable: raw.operable, usable: raw.usable,
                swappable: raw.swappable, margin_x: raw.margin_x, margin_y: raw.margin_y,
                actions: [...raw.actions], action_target: raw.action_target,
            };
        case 3:
            return { kind: 'rect', fill: raw.fill };
        case 4:
            return { kind: 'text', centered: raw.centered, font_id: raw.font_id, shadowed: raw.shadowed, text: raw.text, active_text: raw.active_text };
        case 5:
            return { kind: 'sprite', graphic: raw.graphic, active_graphic: raw.active_graphic };
        case 6:
            return {
                kind: 'model', model: raw.model, active_model: raw.active_model, anim: raw.anim,
                active_anim: raw.active_anim, zoom: raw.zoom, xan: raw.xan, yan: raw.yan,
            };
        case 7:
            if (!raw.actions) throw new Error(`server_ordered: inventory text ${raw.id} has no decoded actions`);
            return {
                kind: 'inventory_text', centered: raw.centered, font_id: raw.font_id, shadowed: raw.shadowed,
                colour: raw.colour, margin_x: raw.margin_x, margin_y: raw.margin_y, operable: raw.operable, actions: [...raw.actions],
            };
        default:
            throw new Error(`server_ordered: unsupported decoded component type ${raw.component_type}`);
    }
}

function collectRequiredAliases(contentRoot: string, initialAliases: readonly string[], definitionCache: Map<string, InterfaceDefinition[]>) {
    const aliases = new Set(initialAliases);
    const visit = (alias: string, visited: Set<string>) => {
        if (visited.has(alias)) throw new Error(`server_ordered: authored component ancestry cycle at ${alias}`);
        visited.add(alias);
        const definition = definitionFor(contentRoot, alias, definitionCache);
        if (!definition) return;
        const layer = interfaceField(definition, 'layer')?.value;
        const rootAlias = alias.slice(0, alias.indexOf(':'));
        const parentAlias = layer ? `${rootAlias}:${layer}` : rootAlias;
        if (parentAlias !== alias) {
            aliases.add(parentAlias);
            visit(parentAlias, visited);
        }
    };
    for (const alias of initialAliases) visit(alias, new Set());
    return [...aliases].sort();
}

function buildComponents(
    contentRoot: string,
    aliases: readonly string[],
    identities: Record<string, InterfaceAliasIdentity>,
    cache: ServerOrderedCacheSnapshot,
    definitionCache: Map<string, InterfaceDefinition[]>,
) {
    const componentsById = new Map<number, ServerOrderedCacheComponent>();
    const namesToIds = new Map<string, number[]>();
    const parentCandidates = new Map<number, number[]>();
    for (const component of cache.components) {
        if (componentsById.has(component.id)) throw new Error(`server_ordered: duplicate decoded component id ${component.id}`);
        componentsById.set(component.id, component);
        if (component.name !== null) {
            const ids = namesToIds.get(component.name) ?? [];
            ids.push(component.id);
            namesToIds.set(component.name, ids);
        }
        for (const child of component.child_ids ?? []) {
            const parents = parentCandidates.get(child) ?? [];
            parents.push(component.id);
            parentCandidates.set(child, parents);
        }
    }

    const fonts = new Map<string, number>();
    for (const font of cache.fonts) {
        if (fonts.has(font.alias)) throw new Error(`server_ordered: ambiguous selected font ${font.alias}`);
        if (!Number.isInteger(font.id) || font.id < 0) throw new Error(`server_ordered: invalid selected font id for ${font.alias}`);
        fonts.set(font.alias, font.id);
    }

    const records = new Map<string, ServerOrderedComponent>();
    for (const alias of aliases) {
        const identity = identities[alias];
        const rootAlias = alias.includes(':') ? alias.slice(0, alias.indexOf(':')) : alias;
        const root = identities[rootAlias];
        if (!identity || !root) throw new Error(`server_ordered: missing packed component alias ${alias}`);
        const names = namesToIds.get(alias) ?? [];
        if (names.length !== 1 || names[0] !== identity.id) throw new Error(`server_ordered: selected cache alias disagrees for ${alias}`);
        const raw = componentsById.get(identity.id);
        if (!raw) throw new Error(`server_ordered: selected cache component is missing for ${alias}`);
        const definition = definitionFor(contentRoot, alias, definitionCache);
        assertSourceComponent(alias, raw, definition, fonts);
        assertEqual(raw.root_layer, root.id, `root ancestry for ${alias}`);

        const parentIds = parentCandidates.get(raw.id) ?? [];
        let parentId: number | null = null;
        let ancestorIds: number[] = [];
        if (alias === rootAlias) {
            if (parentIds.length !== 0) throw new Error(`server_ordered: interface root has an authored parent for ${alias}`);
        } else {
            const layer = interfaceField(definition, 'layer')?.value;
            const parentAlias = layer ? `${rootAlias}:${layer}` : rootAlias;
            const expectedParent = identities[parentAlias];
            if (!expectedParent) throw new Error(`server_ordered: missing authored parent alias ${parentAlias}`);
            if (parentIds.length !== 1 || parentIds[0] !== expectedParent.id) {
                throw new Error(`server_ordered: decoded parent ancestry disagrees for ${alias}`);
            }
            parentId = expectedParent.id;
            const seen = new Set<number>([raw.id]);
            let cursor: number | null = parentId;
            while (cursor !== null) {
                if (seen.has(cursor)) throw new Error(`server_ordered: decoded component ancestry cycle for ${alias}`);
                seen.add(cursor);
                ancestorIds.push(cursor);
                if (cursor === root.id) break;
                const parents = parentCandidates.get(cursor) ?? [];
                if (parents.length !== 1) throw new Error(`server_ordered: incomplete decoded root ancestry for ${alias}`);
                cursor = parents[0];
            }
            if (ancestorIds[ancestorIds.length - 1] !== root.id) throw new Error(`server_ordered: component does not reach selected root for ${alias}`);
        }

        const decoderSpans = [
            span(COMPONENT_SOURCE, 49, 64),
            span(COMPONENT_SOURCE, 100, 118),
            span(COMPONENT_SOURCE, 220, 237),
            span(COMPONENT_SOURCE, 274, 281),
        ];
        const definitionSpans = sourceDefinitionSpans(definition);
        const fieldSpans = definition ? [...definition.fields.keys()].flatMap(key => exactSourceField(definition, key)) : [];
        records.set(alias, {
            alias,
            id: raw.id,
            root_alias: rootAlias,
            root_id: root.id,
            parent_id: parentId,
            ancestor_ids: ancestorIds,
            component_type: rawComponentType(raw),
            button_type: rawButtonType(raw),
            client_code: raw.client_code,
            width: raw.width,
            height: raw.height,
            properties: componentProperties(raw),
            source: mergeSpans([
                span('pack/interface.pack', identity.line),
                ...definitionSpans,
                ...fieldSpans,
                ...decoderSpans,
            ]),
        });
    }
    return records;
}

function oneBlock(blocks: Rs2Block[], kind: string, name: string, label: string): Rs2Block {
    const found = blocks.filter(block => block.kind === kind && block.name === name);
    if (found.length !== 1) throw new Error(`server_ordered: expected one ${label} block`);
    return found[0];
}

function sourceBody(block: Rs2Block): string {
    return block.body.map(line => stripComment(line.text)).join('\n');
}

function findBalancedBrace(source: string, open: number): number {
    if (source[open] !== '{') throw new Error('server_ordered: expected switch block');
    let depth = 0;
    let inString = false;
    let escaped = false;
    for (let index = open; index < source.length; index += 1) {
        const character = source[index];
        if (inString) {
            if (escaped) escaped = false;
            else if (character === '\\') escaped = true;
            else if (character === '"') inString = false;
            continue;
        }
        if (character === '"') inString = true;
        else if (character === '{') depth += 1;
        else if (character === '}') {
            depth -= 1;
            if (depth === 0) return index;
        }
    }
    throw new Error('server_ordered: unbalanced content switch');
}

type SwitchCase = { label: string; body: string };

/** Match only the procedure's own scope, not choices inside another branch. */
function topLevelMatches(source: string, pattern: RegExp): RegExpMatchArray[] {
    const candidates = [...source.matchAll(pattern)];
    const matches: RegExpMatchArray[] = [];
    let next = 0;
    let depth = 0;
    let quoted = false;
    let escaped = false;
    for (let index = 0; index < source.length; index += 1) {
        if (next < candidates.length && candidates[next].index === index) {
            if (depth === 0 && !quoted) matches.push(candidates[next]);
            next += 1;
        }
        const character = source[index];
        if (quoted) {
            if (escaped) escaped = false;
            else if (character === '\\') escaped = true;
            else if (character === '"') quoted = false;
        } else if (character === '"') quoted = true;
        else if (character === '{') depth += 1;
        else if (character === '}') depth -= 1;
    }
    if (depth !== 0 || quoted) throw new Error('server_ordered: unbalanced content scope');
    return matches;
}

function switchCases(source: string, kind: 'switch_int' | 'switch_component', subject: string): SwitchCase[] {
    const pattern = new RegExp(`\\b${kind}\\s*\\(\\s*${subject.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\)\\s*\\{`, 'g');
    const matches = topLevelMatches(source, pattern);
    if (matches.length !== 1) throw new Error(`server_ordered: expected one ${kind}(${subject}) branch`);
    const open = matches[0].index! + matches[0][0].lastIndexOf('{');
    const close = findBalancedBrace(source, open);
    const inner = source.slice(open + 1, close);
    const caseRows: { label: string; start: number; bodyStart: number }[] = [];
    let depth = 0;
    let inString = false;
    let escaped = false;
    let lineStart = 0;
    for (let index = 0; index <= inner.length; index += 1) {
        if (index === inner.length || inner[index] === '\n') {
            const line = inner.slice(lineStart, index);
            if (depth === 0) {
                const match = /^\s*case\s+(.+?)\s*:(?=\s|$)\s*/.exec(line);
                if (match) caseRows.push({ label: match[1].trim(), start: lineStart, bodyStart: lineStart + match[0].length });
            }
            lineStart = index + 1;
            continue;
        }
        const character = inner[index];
        if (inString) {
            if (escaped) escaped = false;
            else if (character === '\\') escaped = true;
            else if (character === '"') inString = false;
            continue;
        }
        if (character === '"') inString = true;
        else if (character === '{') depth += 1;
        else if (character === '}') depth -= 1;
    }
    return caseRows.map((row, index) => ({
        label: row.label,
        body: inner.slice(row.bodyStart, caseRows[index + 1]?.start ?? inner.length).trim(),
    }));
}

function simpleStatements(source: string): string[] {
    const statements: string[] = [];
    let start = 0;
    let depth = 0;
    let inString = false;
    let escaped = false;
    for (let index = 0; index < source.length; index += 1) {
        const character = source[index];
        if (inString) {
            if (escaped) escaped = false;
            else if (character === '\\') escaped = true;
            else if (character === '"') inString = false;
            continue;
        }
        if (character === '"') inString = true;
        else if (character === '(' || character === '{') depth += 1;
        else if (character === ')' || character === '}') depth -= 1;
        else if (character === ';' && depth === 0) {
            const statement = source.slice(start, index).replace(/\s+/g, ' ').trim();
            if (statement) statements.push(statement);
            start = index + 1;
        }
    }
    if (source.slice(start).trim()) throw new Error('server_ordered: selected branch has a statement without a terminator');
    return statements;
}

function decodeStringLiteral(value: string, label: string): string {
    try {
        const decoded: unknown = JSON.parse(value);
        if (typeof decoded !== 'string') throw new Error('not a string');
        return decoded;
    } catch (error) {
        throw new Error(`server_ordered: invalid ${label} string literal`, { cause: error });
    }
}

type BankerPath = {
    text: string;
    mesanimAlias: string;
    bankerBlock: Rs2Block;
    choiceBlock: Rs2Block;
    chatPlayerBlock: Rs2Block;
    chatPlayerPageBlock: Rs2Block;
};

function verifyBankerSource(contentRoot: string): BankerPath {
    const bankerBlocks = parseRs2Blocks(BANKER_SOURCE, requireGatherText(contentRoot, BANKER_SOURCE));
    const banker = oneBlock(bankerBlocks, 'label', 'talk_to_banker', 'banker talk label');
    const bankerCode = sourceBody(banker);
    const choiceCalls = topLevelMatches(bankerCode, /~p_choice2\s*\(\s*("(?:\\.|[^"\\])*")\s*,\s*1\s*,\s*("(?:\\.|[^"\\])*")\s*,\s*2\s*\)/g);
    if (choiceCalls.length !== 1) throw new Error('server_ordered: banker choice shape changed');
    const choiceText = decodeStringLiteral(choiceCalls[0][1], 'banker choice');
    const cases = switchCases(bankerCode, 'switch_int', '$option');
    const selected = cases.filter(row => row.label === '1');
    if (selected.length !== 1) throw new Error('server_ordered: banker choice-1 branch is missing or ambiguous');
    const selectedStatements = simpleStatements(selected[0].body);
    const playerCall = /^~chatplayer\s*\(\s*("(?:\\.|[^"\\])*")\s*\)$/.exec(selectedStatements[0] ?? '');
    if (selectedStatements.length !== 2 || !playerCall || selectedStatements[1] !== '@openbank') {
        throw new Error('server_ordered: banker choice-1 branch is not synchronous chatplayer then openbank');
    }
    const playerText = decodeStringLiteral(playerCall[1], 'banker player page');
    const prefixed = /^<p,([A-Za-z0-9_]+)>(.*)$/.exec(playerText);
    if (!prefixed || prefixed[1] !== BANKER_MESANIM || choiceText !== BANKER_TEXT || prefixed[2] !== choiceText) {
        throw new Error('server_ordered: banker player text or animation changed');
    }

    const chatBlocks = parseRs2Blocks(CHAT_SOURCE, requireGatherText(contentRoot, CHAT_SOURCE));
    const choice = oneBlock(chatBlocks, 'proc', 'p_choice2', 'two-option choice procedure');
    const choiceCode = sourceBody(choice);
    const openChoice = choiceCode.indexOf('if_openchat(multi2)');
    const registerChoice = choiceCode.indexOf('if_addresumebutton(multi2:com_1)');
    const pauseChoice = choiceCode.indexOf('p_pausebutton');
    const choiceSwitch = choiceCode.indexOf('switch_component (last_com)');
    if (openChoice < 0 || registerChoice <= openChoice || pauseChoice <= registerChoice || choiceSwitch <= pauseChoice || /\bp_delay\s*\(/.test(choiceCode)) {
        throw new Error('server_ordered: multi2 choice is not a zero-delay received page');
    }
    const choiceCases = switchCases(choiceCode, 'switch_component', 'last_com');
    if (choiceCases.length !== 2
        || simpleStatements(choiceCases.find(row => row.label === 'multi2:com_1')?.body ?? '').join(';') !== 'return($ret1)'
        || simpleStatements(choiceCases.find(row => row.label === 'default')?.body ?? '').join(';') !== 'return($ret2)') {
        throw new Error('server_ordered: multi2 choice component no longer selects bank branch one');
    }

    const chatPlayer = oneBlock(chatBlocks, 'proc', 'chatplayer', 'chatplayer procedure');
    const chatPlayerCode = sourceBody(chatPlayer);
    const splitCalls = [...chatPlayerCode.matchAll(/split_init\s*\(\s*\$string\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*([A-Za-z0-9_]+)\s*\)/g)];
    if (splitCalls.length !== 1
        || Number(splitCalls[0][1]) !== CHAT_LINE_WIDTH
        || Number(splitCalls[0][2]) !== CHAT_MAX_LINES
        || splitCalls[0][3] !== BANKER_FONT
        || /\bp_delay\s*\(/.test(chatPlayerCode)) {
        throw new Error('server_ordered: chatplayer width, font, page size, or delay changed');
    }
    const splitPosition = chatPlayerCode.indexOf(splitCalls[0][0]);
    const pageCountPosition = chatPlayerCode.indexOf('split_pagecount');
    const pageLoopPosition = chatPlayerCode.indexOf('while ($page < $pagetotal)');
    const pageCallPosition = chatPlayerCode.indexOf('~chatplayer_page($page)');
    const pausePosition = chatPlayerCode.indexOf('p_pausebutton');
    if (pageCountPosition <= splitPosition || pageLoopPosition <= pageCountPosition || pageCallPosition <= pageLoopPosition || pausePosition <= pageCallPosition) {
        throw new Error('server_ordered: chatplayer page continuation order changed');
    }

    const chatPlayerPage = oneBlock(chatBlocks, 'proc', 'chatplayer_page', 'chatplayer page procedure');
    const chatPlayerPageCode = sourceBody(chatPlayerPage);
    if (!/split_linecount\s*\(\s*\$page\s*\)/.test(chatPlayerPageCode)
        || !/if_openchat\s*\(\s*\$interface\s*\)/.test(chatPlayerPageCode)
        || /\bp_delay\s*\(/.test(chatPlayerPageCode)) {
        throw new Error('server_ordered: chatplayer page path changed or gained a delay');
    }

    return {
        text: playerText,
        mesanimAlias: prefixed[1],
        bankerBlock: banker,
        choiceBlock: choice,
        chatPlayerBlock: chatPlayer,
        chatPlayerPageBlock: chatPlayerPage,
    };
}

function proveBankerPage(pathFacts: BankerPath, cache: ServerOrderedCacheSnapshot, contentRoot: string) {
    const splitCall = /split_init\s*\(\s*\$string\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*([A-Za-z0-9_]+)\s*\)/.exec(sourceBody(pathFacts.chatPlayerBlock));
    if (!splitCall) throw new Error('server_ordered: banker page split parameters are unavailable');
    const lineWidth = Number(splitCall[1]);
    const maxLines = Number(splitCall[2]);
    const fontAlias = splitCall[3];
    const fontMatches = cache.fonts.filter(font => font.alias === fontAlias);
    if (fontMatches.length !== 1) throw new Error(`server_ordered: selected font ${fontAlias} is missing or ambiguous`);
    const font = fontMatches[0];
    if (font.alias !== BANKER_FONT) throw new Error('server_ordered: selected banker font identity is unproved');

    const mesanimPack = resolvePackAlias(contentRoot, 'pack/mesanim.pack', pathFacts.mesanimAlias, 'server_ordered');
    const mesanimMatches = cache.mesanims.filter(row => row.alias === pathFacts.mesanimAlias);
    if (mesanimMatches.length !== 1 || mesanimMatches[0].id !== mesanimPack.id) {
        throw new Error(`server_ordered: selected mesanim alias disagrees with cache for ${pathFacts.mesanimAlias}`);
    }

    const splitLines = font.split(pathFacts.text.slice(pathFacts.text.indexOf('>') + 1), lineWidth);
    if (!Array.isArray(splitLines) || splitLines.some(line => typeof line !== 'string')) {
        throw new Error('server_ordered: selected font returned an invalid line split');
    }
    const lineCount = splitLines.length;
    const pageCount = Math.ceil(lineCount / maxLines);
    if (lineCount < 1 || lineCount > maxLines || pageCount !== 1) {
        throw new Error(`server_ordered: banker player text does not fit one selected-font page (${pageCount} pages)`);
    }

    const pageRootAlias = `chat${lineCount}`;
    const continueComponentAlias = `${pageRootAlias}:com_${lineCount + 2}`;
    const pageCode = sourceBody(pathFacts.chatPlayerPageBlock);
    const pageBranchPattern = new RegExp(`(?:^|[}\\n])\\s*(?:else\\s+)?if\\s*\\(\\s*\\$lines\\s*=\\s*${lineCount}\\s*\\)\\s*\\{`, 'g');
    const pageBranchMatches = [...pageCode.matchAll(pageBranchPattern)];
    if (pageBranchMatches.length !== 1) throw new Error(`server_ordered: chatplayer line-count ${lineCount} branch is missing or ambiguous`);
    const pageBranchOpen = pageBranchMatches[0].index! + pageBranchMatches[0][0].lastIndexOf('{');
    const pageBranchClose = findBalancedBrace(pageCode, pageBranchOpen);
    const selectedPageBranch = pageCode.slice(pageBranchOpen + 1, pageBranchClose);
    if (!new RegExp(`\\$interface\\s*=\\s*${pageRootAlias}\\s*;`).test(selectedPageBranch)
        || !new RegExp(`if_setanim\\(${pageRootAlias}:com_0,\\s*split_getanim\\(\\$page\\)\\)`).test(selectedPageBranch)
        || !new RegExp(`if_settext\\(${pageRootAlias}:com_1,\\s*displayname\\)`).test(selectedPageBranch)) {
        throw new Error(`server_ordered: chatplayer line-count ${lineCount} no longer selects and fills ${pageRootAlias}`);
    }
    for (let line = 0; line < lineCount; line += 1) {
        if (!new RegExp(`if_settext\\(${pageRootAlias}:com_${line + 2},\\s*split_get\\(\\$page,\\s*${line}\\)\\)`).test(selectedPageBranch)) {
            throw new Error(`server_ordered: chatplayer line-count ${lineCount} no longer displays split line ${line}`);
        }
    }
    const openPagePosition = pageCode.indexOf('if_openchat($interface)');
    if (openPagePosition <= pageBranchClose) throw new Error('server_ordered: chatplayer does not open the selected page after filling it');

    return {
        page: {
            font_alias: fontAlias,
            font_id: font.id,
            mesanim_alias: pathFacts.mesanimAlias,
            mesanim_id: mesanimMatches[0].id,
            line_width: lineWidth,
            line_count: lineCount,
            max_lines: maxLines,
            page_count: pageCount,
            split_lines: [...splitLines],
            text: pathFacts.text,
            page_root_alias: pageRootAlias,
            continue_component_alias: continueComponentAlias,
            sources: mergeSpans([
                pathFacts.bankerBlock.span,
                pathFacts.choiceBlock.span,
                pathFacts.chatPlayerBlock.span,
                pathFacts.chatPlayerPageBlock.span,
                span('src/network/game/client/handler/ResumePauseButtonHandler.ts', 6, 14),
                span('pack/mesanim.pack', mesanimPack.line),
                span(STRING_OPS_SOURCE, 76, 96),
                span(FONT_SOURCE, 6, 13),
                span(FONT_SOURCE, 125, 195),
                span(MESANIM_SOURCE, 11, 34),
                span(MESANIM_SOURCE, 41, 43),
            ]),
        },
    };
}

function tryProveBankerPage(contentRoot: string, cache: ServerOrderedCacheSnapshot): ServerOrderedBankerPage | null {
    try {
        const pathFacts = verifyBankerSource(contentRoot);
        return proveBankerPage(pathFacts, cache, contentRoot).page;
    } catch {
        return null;
    }
}


function requiredScriptStatement(contentRoot: string, file: string, kind: string, name: string, expected: RegExp, label: string) {
    const block = oneBlock(parseRs2Blocks(file, requireGatherText(contentRoot, file)), kind, name, label);
    const statements = parseBody(block.body);
    if (statements.length !== 1 || statements[0].kind !== 'stmt' || !expected.test(statements[0].text)) {
        throw new Error(`server_ordered: ${label} source predicate changed`);
    }
    return block;
}

function verifySynchronousCountPrompt(block: Rs2Block, inventory: 'bank' | 'inv', label: string) {
    const code = sourceBody(block).replace(/\s+/g, ' ').trim();
    const promptCalls = [...code.matchAll(/\bp_countdialog\s*;/g)];
    if (promptCalls.length !== 1) throw new Error(`server_ordered: ${label} count prompt is not a synchronous label path`);

    const promptPosition = promptCalls[0].index!;
    const beforePrompt = code.slice(0, promptPosition).trim();
    if (/\bp_delay\s*\(|\b(?:weakqueue|queue)\*?\s*\(|[~@]\s*[A-Za-z_][A-Za-z0-9_]*/.test(beforePrompt)) {
        throw new Error(`server_ordered: ${label} count prompt is not a synchronous label path`);
    }
    const expectedPrefix = `def_obj $item = inv_getobj(${inventory}, $slot); if ($requested_number <= 0) {`;
    const promptBranch = code.slice(promptPosition);
    if (beforePrompt !== expectedPrefix
        || !/^p_countdialog\s*;\s*if\s*\(\s*last_int\s*<=\s*0\s*\)\s*return\s*;\s*\$requested_number\s*=\s*last_int\s*;/.test(promptBranch)) {
        throw new Error(`server_ordered: ${label} count prompt is not a synchronous label path`);
    }
    return true;
}



function extractBankFacts(contentRoot: string, identities: Record<string, InterfaceAliasIdentity>): ServerOrderedBankFacts {
    const bankBlocks = parseRs2Blocks(BANK_SOURCE, requireGatherText(contentRoot, BANK_SOURCE));
    const depositBlocks = parseRs2Blocks(BANK_DEPOSIT_SOURCE, requireGatherText(contentRoot, BANK_DEPOSIT_SOURCE));
    const openBank = oneBlock(bankBlocks, 'label', 'openbank', 'bank open label');
    const withdrawLabel = oneBlock(bankBlocks, 'label', 'bank_withdraw', 'withdraw-X count label');
    const depositLabel = oneBlock(depositBlocks, 'label', 'bank_deposit', 'deposit-X count label');
    const withdrawPromptSynchronous = verifySynchronousCountPrompt(withdrawLabel, 'bank', 'bank_withdraw');
    const depositPromptSynchronous = verifySynchronousCountPrompt(depositLabel, 'inv', 'bank_deposit');
    const withdraw = oneBlock(bankBlocks, 'inv_button5', 'bank_main:inv', 'withdraw-X handler');
    const depositX = oneBlock(depositBlocks, 'inv_button5', 'bank_side:inv', 'deposit-X handler');
    const depositAll = oneBlock(depositBlocks, 'inv_button4', 'bank_side:inv', 'deposit-all handler');
    const modeItem = requiredScriptStatement(contentRoot, BANK_SOURCE, 'if_button', 'bank_main:com_94', /^%bankcert = 0$/, 'item mode handler');
    const modeNote = requiredScriptStatement(contentRoot, BANK_SOURCE, 'if_button', 'bank_main:com_93', /^%bankcert = 1$/, 'note mode handler');
    if (!/%bankcert\s*=\s*0\s*;/.test(sourceBody(openBank))) throw new Error('server_ordered: bank open does not set item mode');
    if (!/@bank_withdraw\(last_slot, null\)/.test(sourceBody(withdraw))) throw new Error('server_ordered: withdraw-X no longer requests a count');
    if (!/@bank_deposit\(last_slot, null\)/.test(sourceBody(depositX))) throw new Error('server_ordered: deposit-X no longer requests a count');
    if (!/@bank_deposit\(last_slot, \^max_32bit_int\)/.test(sourceBody(depositAll))) throw new Error('server_ordered: deposit-all handler changed');
    const withdrawOperation = Number(/^inv_button(\d+)$/.exec(withdraw.kind)?.[1]);
    const depositOperation = Number(/^inv_button(\d+)$/.exec(depositX.kind)?.[1]);
    const depositAllOperation = Number(/^inv_button(\d+)$/.exec(depositAll.kind)?.[1]);
    if (withdrawOperation !== 5 || depositOperation !== 5 || depositAllOperation !== 4) {
        throw new Error('server_ordered: selected bank operation numbers changed');
    }

    const bankMainDefs = parseInterfaceDefinitions(BANK_MAIN_INTERFACE, requireGatherText(contentRoot, BANK_MAIN_INTERFACE));
    const byName = (name: string) => {
        const rows = bankMainDefs.filter(definition => definition.alias === name);
        if (rows.length !== 1) throw new Error(`server_ordered: expected one bank mode interface component ${name}`);
        return rows[0];
    };
    const itemButton = byName('com_94');
    const noteButton = byName('com_93');
    const itemLabel = byName('com_96');
    const noteLabel = byName('com_98');
    const buttonScriptMatches = (definition: InterfaceDefinition, mode: number) =>
        interfaceField(definition, 'script1op1')?.value === 'pushvar,bankcert'
        && interfaceField(definition, 'script1')?.value === `eq,${mode}`;
    if (!buttonScriptMatches(itemButton, 0) || !buttonScriptMatches(noteButton, 1)) {
        throw new Error('server_ordered: bank mode button varp predicates changed');
    }
    const within = (button: InterfaceDefinition, label: InterfaceDefinition) => {
        const bx = interfaceNumber(button, 'x');
        const by = interfaceNumber(button, 'y');
        const bw = interfaceNumber(button, 'width');
        const bh = interfaceNumber(button, 'height');
        const lx = interfaceNumber(label, 'x');
        const ly = interfaceNumber(label, 'y');
        const lw = interfaceNumber(label, 'width');
        const lh = interfaceNumber(label, 'height');
        return lx >= bx && ly >= by && lx + lw <= bx + bw && ly + lh <= by + bh;
    };
    if (interfaceField(itemLabel, 'text')?.value !== 'Item' || !within(itemButton, itemLabel)
        || interfaceField(noteLabel, 'text')?.value !== 'Note' || !within(noteButton, noteLabel)) {
        throw new Error('server_ordered: bank item/note mode labels no longer match their controls');
    }
    const modeVarp = resolvePackAlias(contentRoot, 'pack/varp.pack', 'bankcert', 'server_ordered');
    const mainIdentity = identities['bank_main:inv'];
    const sideIdentity = identities['bank_side:inv'];
    if (!mainIdentity || !sideIdentity) throw new Error('server_ordered: bank inventory aliases are unavailable');

    return {
        bank: {
            mode_varp: { alias: 'bankcert', id: modeVarp.id },
            item_mode: 0,
            note_mode: 1,
            main_component: { alias: 'bank_main:inv', id: mainIdentity.id },
            side_component: { alias: 'bank_side:inv', id: sideIdentity.id },
            withdraw_x_op: withdrawOperation,
            deposit_x_op: depositOperation,
            deposit_all_op: depositAllOperation,
            sources: mergeSpans([
                span('pack/varp.pack', modeVarp.line),
                span(BANK_SOURCE, openBank.span.first, openBank.span.last),
                modeItem.span,
                modeNote.span,
                withdraw.span,
                withdrawLabel.span,
                depositX.span,
                depositAll.span,
                span(BANK_MAIN_INTERFACE, itemButton.first, itemButton.last),
                span(BANK_MAIN_INTERFACE, noteButton.first, noteButton.last),
                span(BANK_MAIN_INTERFACE, itemLabel.first, itemLabel.last),
                span(BANK_MAIN_INTERFACE, noteLabel.first, noteLabel.last),
                span(COMPONENT_SOURCE, 100, 150),
                span('src/network/game/client/handler/InvButtonHandler.ts', 10, 60),
            ]),
        },
        spans: mergeSpans([
            openBank.span,
            withdraw.span,
            withdrawLabel.span,
            depositLabel.span,
            depositX.span,
            depositAll.span,
            modeItem.span,
            modeNote.span,
            span(BANK_MAIN_INTERFACE, itemButton.first, itemButton.last),
            span(BANK_MAIN_INTERFACE, noteButton.first, noteButton.last),
            span(BANK_MAIN_INTERFACE, itemLabel.first, itemLabel.last),
            span(BANK_MAIN_INTERFACE, noteLabel.first, noteLabel.last),
            span(BANK_DEPOSIT_SOURCE, 8, 27),
            span('src/network/game/client/handler/InvButtonHandler.ts', 10, 60),
        ]),
        countPromptSynchronous: {
            withdraw: withdrawPromptSynchronous,
            deposit: depositPromptSynchronous,
        },
    };
}

function extractBankBooth(contentRoot: string): ServerOrderedBankBoothFacts {
    const alias = resolvePackAlias(contentRoot, 'pack/loc.pack', 'bankbooth', 'server_ordered');
    const locBlocks = parseInterfaceDefinitions(BANK_BOOTH_LOC_SOURCE, requireGatherText(contentRoot, BANK_BOOTH_LOC_SOURCE));
    const definition = locBlocks.filter(row => row.alias === 'bankbooth');
    if (definition.length !== 1 || interfaceField(definition[0], 'op2')?.value !== 'Use-quickly') {
        throw new Error('server_ordered: bankbooth op2 definition is missing or changed');
    }
    const op2 = oneBlock(parseRs2Blocks(BANK_BOOTH_SOURCE, requireGatherText(contentRoot, BANK_BOOTH_SOURCE)), 'oploc2', 'bankbooth', 'bankbooth operation-2 handler');
    if (!/^\s*@openbank\s*;\s*$/.test(sourceBody(op2))) throw new Error('server_ordered: bankbooth operation 2 no longer opens the bank');
    const operation = Number(/^oploc(\d+)$/.exec(op2.kind)?.[1]);
    if (operation !== 2) throw new Error('server_ordered: bankbooth selected operation changed');
    return {
        numericLoc: {
            alias: 'bankbooth', id: alias.id, operation, action: 'open_bank',
            sources: mergeSpans([
                span('pack/loc.pack', alias.line),
                span(BANK_BOOTH_LOC_SOURCE, definition[0].first, definition[0].last),
                op2.span,
                span('src/network/game/client/handler/OpLocHandler.ts', 10, 57),
            ]),
        } satisfies ServerOrderedNumericLoc,
    };
}

function extractCharacterAcceptance(contentRoot: string): ServerOrderedCharacterAcceptanceFacts {
    const source = parseRs2Blocks(TUTORIAL_SOURCE, requireGatherText(contentRoot, TUTORIAL_SOURCE));
    const accept = oneBlock(source, 'if_button', 'player_kit:accept', 'character accept handler');
    const close = oneBlock(source, 'if_close', 'player_kit', 'character design close handler');
    const queue = oneBlock(source, 'queue', 'tutorial_designed_character', 'character design tutorial queue');
    const acceptStatements = parseBody(accept.body);
    if (acceptStatements.length !== 1 || acceptStatements[0].kind !== 'stmt' || !/^if_close(?:\(\))?$/.test(acceptStatements[0].text)) {
        throw new Error('server_ordered: character acceptance no longer closes the design modal');
    }
    if (!/%tutorial\s*=\s*\^newbie_basics_instructor_designed_character\s*;/.test(sourceBody(queue))
        || !/queue\(tutorial_designed_character,\s*0,\s*0\)/.test(sourceBody(close))) {
        throw new Error('server_ordered: character acceptance tutorial signal changed');
    }
    return {
        spans: mergeSpans([
            accept.span,
            close.span,
            queue.span,
            span(PLAYER_KIT_INTERFACE, 733, 742),
            span('vendor/fr-client-rust/crates/client/src/client/client.rs', 7070, 7082),
            span('vendor/fr-client-rust/crates/client/src/client/client.rs', 4521, 4538),
            span('src/network/game/client/handler/IdkSaveDesignHandler.ts', 10, 56),
            span('src/network/game/client/ClientGameProt.ts', 89),
        ]),
    };
}

function engineMethodBody(source: string, signature: RegExp, label: string): string {
    const matches = [...source.matchAll(signature)];
    if (matches.length !== 1) throw new Error(`server_ordered: selected engine ${label} signature changed`);
    const open = matches[0].index! + matches[0][0].lastIndexOf('{');
    return source.slice(open + 1, findBalancedBrace(source, open));
}

function verifyPlayerCloseModalEngineEffects(source: string) {
    const body = engineMethodBody(
        source,
        /\bcloseModal\s*\(\s*clearWeakQueue\s*:\s*boolean\s*=\s*true\s*\)\s*\{/g,
        'Player.closeModal',
    );
    const clearPosition = body.indexOf('this.weakQueue.clear();');
    const protectPosition = body.indexOf('this.protect = false;');
    const noModalPosition = body.indexOf('if (this.modalState === ModalState.NONE) {');
    if (!/if\s*\(\s*clearWeakQueue\s*\)\s*\{\s*this\.weakQueue\.clear\(\);\s*\}/.test(body)
        || !/if\s*\(\s*!this\.delayed\s*\)\s*\{\s*this\.protect\s*=\s*false;\s*\}/.test(body)
        || clearPosition < 0 || protectPosition < clearPosition || noModalPosition < protectPosition) {
        throw new Error('server_ordered: selected engine Player.closeModal queue or protection effects changed');
    }
    if (!/if\s*\(\s*this\.activeScript\?\.execution\s*===\s*ScriptState\.COUNTDIALOG\s*\|\|\s*this\.activeScript\?\.execution\s*===\s*ScriptState\.PAUSEBUTTON\s*\)\s*\{\s*this\.activeScript\s*=\s*null;\s*this\.resumeButtons\s*=\s*\[\];\s*\}/.test(body)) {
        throw new Error('server_ordered: selected engine Player.closeModal suspended-script effects changed');
    }
    for (const modal of ['modalMain', 'modalChat', 'modalSide']) {
        const open = body.indexOf(`if (this.${modal} !== -1) {`);
        const reset = body.indexOf(`this.${modal} = -1;`, open);
        if (open < 0 || reset < 0) throw new Error('server_ordered: selected engine Player.closeModal modal effects changed');
        const block = body.slice(open, reset);
        if (!block.includes(`ScriptProvider.getByTrigger(ServerTriggerType.IF_CLOSE, this.${modal})`)
            || !/if\s*\(closeTrigger\)\s*\{\s*this\.executeScript\(ScriptRunner\.init\(closeTrigger,\s*this\),\s*false\);\s*\}/.test(block)
            || !block.includes(`this.clearComListeners(this.${modal});`)) {
            throw new Error('server_ordered: selected engine Player.closeModal IF_CLOSE or listener effects changed');
        }
    }
    if (!/this\.modalState\s*=\s*ModalState\.NONE;/.test(body)
        || !/this\.refreshModalClose\s*=\s*true;/.test(body)) {
        throw new Error('server_ordered: selected engine Player.closeModal modal-state effects changed');
    }
}

function verifyCountResumeHandler(source: string) {
    const body = engineMethodBody(
        source,
        /\bhandle\s*\(\s*message\s*:\s*ResumePCountDialog\s*,\s*player\s*:\s*Player\s*\)\s*:\s*boolean\s*\{/g,
        'ResumePCountDialogHandler.handle',
    );
    if (!/const\s*\{\s*input\s*\}\s*=\s*message;[\s\S]*?if\s*\(\s*!player\.activeScript\s*\|\|\s*player\.activeScript\.execution\s*!==\s*ScriptState\.COUNTDIALOG\s*\)\s*\{\s*return\s+false;\s*\}[\s\S]*?player\.activeScript\.lastInt\s*=\s*input;[\s\S]*?player\.executeScript\(player\.activeScript,\s*true,\s*true\);[\s\S]*?return\s+true;/.test(body)) {
        throw new Error('server_ordered: selected engine count-resume handler changed');
    }
    return true;
}

function extractCloseModal(contentRoot: string, engineSources: ServerOrderedEngineSources): ServerOrderedCloseModalFacts {
    verifyPlayerCloseModalEngineEffects(engineSources[PLAYER_SOURCE]);
    const bankBlocks = parseRs2Blocks(BANK_SOURCE, requireGatherText(contentRoot, BANK_SOURCE));
    const close = oneBlock(bankBlocks, 'if_close', 'bank_main', 'terminal bank close handler');
    const closeBank = oneBlock(bankBlocks, 'label', 'closebank', 'bank close cleanup label');
    const reorganize = oneBlock(bankBlocks, 'queue', 'reorganize_bank', 'bank reorganization queue');
    if (parseBody(close.body).length !== 1 || parseBody(close.body)[0].kind !== 'stmt' || parseBody(close.body)[0].text !== '@closebank') {
        throw new Error('server_ordered: bank IF_CLOSE no longer invokes the owned bank cleanup');
    }
    if (!/inv_stoptransmit\(bank_side:inv\)/.test(sourceBody(closeBank))
        || !/inv_stoptransmit\(bank_main:inv\)/.test(sourceBody(closeBank))
        || !/queue\(reorganize_bank,\s*0,\s*0\)/.test(sourceBody(closeBank))) {
        throw new Error('server_ordered: bank close listener or reorganization effects changed');
    }
    if (!/reorganize_inv\(bank\)/.test(sourceBody(reorganize))) throw new Error('server_ordered: bank close reorganization queue changed');
    const playerCloseModalSource = span(PLAYER_SOURCE, 760, 814);
    const cleanup_effect_sources = {
        stop_bank_main_and_side_inventory_listeners: mergeSpans([closeBank.span, playerCloseModalSource]),
        queue_bank_reorganization: mergeSpans([closeBank.span, reorganize.span]),
        clear_weak_queue: [playerCloseModalSource],
        drop_protect_when_not_delayed: [playerCloseModalSource],
        cancel_suspended_count_or_pause_script: [playerCloseModalSource],
        run_expected_bank_if_close_trigger: mergeSpans([close.span, closeBank.span, playerCloseModalSource]),
    };
    return {
        sources: mergeSpans([
            close.span,
            closeBank.span,
            reorganize.span,
            span('src/network/game/client/handler/CloseModalHandler.ts', 5, 15),
            playerCloseModalSource,
        ]),
        cleanup_effect_sources,
    };
}

function extractKnifeSource(contentRoot: string) {
    const blocks = parseRs2Blocks(CUT_LOGS_SOURCE, requireGatherText(contentRoot, CUT_LOGS_SOURCE));
    const fletch = oneBlock(blocks, 'label', 'fletch_log', 'fletching skill menu handler');
    const process = oneBlock(blocks, 'label', 'process_fletch_logs', 'fletching queued work handler');
    const queue = oneBlock(blocks, 'queue', 'fletch_logs', 'fletching work queue');
    const chatBlocks = parseRs2Blocks(CHAT_SOURCE, requireGatherText(contentRoot, CHAT_SOURCE));
    const menu = oneBlock(chatBlocks, 'proc', 'skill_multi3', 'three-option skill menu procedure');
    const menuCode = sourceBody(menu);
    if (!/if_openchat\(skill_multi3\)/.test(menuCode)
        || !/if_addresumebutton\(skill_multi3:make1_a\)/.test(menuCode)
        || !/p_pausebutton/.test(menuCode)) {
        throw new Error('server_ordered: skill_multi3 make-one synchronous path changed');
    }
    // chat.rs2:831-834 delays a same-pass re-arm after an earlier close.
    // An equal close stamp cannot authorize a no-delay arm in the current pass.
    const guards = topLevelMatches(menuCode, /if\s*\(\s*%weakqueue_map_clock\s*=\s*map_clock\s*\)\s*\{/g);
    if (guards.length !== 1) throw new Error('server_ordered: skill_multi3 close-clock guard changed');
    const guardOpen = guards[0].index! + guards[0][0].lastIndexOf('{');
    const guardClose = findBalancedBrace(menuCode, guardOpen);
    const guardStatements = simpleStatements(menuCode.slice(guardOpen + 1, guardClose));
    if (guardStatements.length !== 2
        || !/^if_close(?:\(\))?$/.test(guardStatements[0])
        || guardStatements[1] !== 'p_delay(0)'
        || /\bp_delay\s*\(/.test(menuCode.slice(0, guardOpen) + menuCode.slice(guardClose + 1))) {
        throw new Error('server_ordered: skill_multi3 close-clock guard changed');
    }
    const makeCase = switchCases(menuCode, 'switch_component', '$last_com').find(row => row.label === 'skill_multi3:make1_a');
    const makeStatements = simpleStatements(makeCase?.body ?? '');
    if (makeStatements.length !== 1 || !/^return\(\$obj1,\s*1\)$/.test(makeStatements[0])) {
        throw new Error('server_ordered: skill_multi3 make-one branch changed');
    }
    const close = oneBlock(chatBlocks, 'if_close', 'skill_multi3', 'skill menu close-clock hook');
    if (!/if\s*\(p_finduid\(uid\)\s*=\s*true\)/.test(sourceBody(close))
        || !/%weakqueue_map_clock\s*=\s*map_clock\s*;/.test(sourceBody(close))) {
        throw new Error('server_ordered: skill_multi3 close-clock stamp changed');
    }
    if (!/~skill_multi3\s*\(/.test(sourceBody(fletch)) || !/if \(\$product = null\)/.test(sourceBody(fletch))) {
        throw new Error('server_ordered: selected log recipe no longer reaches skill_multi3');
    }
    if (!/weakqueue\*\(fletch_logs/.test(sourceBody(process)) || !/inv_del\(inv, \$log, 1\)/.test(sourceBody(queue))) {
        throw new Error('server_ordered: fletching outcome queue changed');
    }
    return mergeSpans([
        fletch.span,
        process.span,
        queue.span,
        menu.span,
        close.span,
        span('src/network/game/client/handler/OpHeldUHandler.ts', 16, 118),
    ]);
}

const CUT_LOGS_DBROW_SOURCE = 'scripts/skill_fletching/configs/cut_logs/cut_logs.dbrow';
const CUTTING_FRUIT_SOURCE = 'scripts/skill_cooking/scripts/gnome_cooking/cutting_fruit.rs2';
const LOG_OBJ_SOURCE = 'scripts/skill_firemaking/configs/firemaking.obj';
const OBJ_PACK = 'pack/obj.pack';
const CATEGORY_PACK = 'pack/category.pack';

// The selected knife/use-item recipe: the exact knife alias from the OPHELDU
// case that reaches the fletching label, and exactly the dbrow log rows whose
// declared shafts count selects the skill_multi3 Make path. Both the
// category-block recipe and the knife-specific delegation must agree, because
// the engine tries the specific [opheldu,knife] trigger before the category.
function extractKnifeFacts(contentRoot: string): ServerOrderedKnife {
    const blocks = parseRs2Blocks(CUT_LOGS_SOURCE, requireGatherText(contentRoot, CUT_LOGS_SOURCE));
    const opheldus = blocks.filter(block => block.kind === 'opheldu');
    if (opheldus.length !== 1) throw new Error('server_ordered: expected one cut_logs OPHELDU recipe block');
    const categoryName = /^_([A-Za-z][A-Za-z0-9_]*)$/.exec(opheldus[0].name)?.[1];
    if (!categoryName) throw new Error('server_ordered: cut_logs OPHELDU recipe is not a category dispatch');
    const category = resolvePackAlias(contentRoot, CATEGORY_PACK, categoryName, 'server_ordered');
    const useCases = switchCases(sourceBody(opheldus[0]), 'switch_obj', 'last_useitem');
    const fletchCases = useCases.filter(row => /@fletch_log\s*\(/.test(row.body));
    if (fletchCases.length !== 1) throw new Error('server_ordered: expected one knife case reaching fletch_log');
    const knifeAlias = fletchCases[0].label;
    if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(knifeAlias)) throw new Error('server_ordered: knife case does not name one use item');
    const knife = resolvePackAlias(contentRoot, OBJ_PACK, knifeAlias, 'server_ordered');
    const fletch = oneBlock(blocks, 'label', 'fletch_log', 'fletching skill menu handler');
    const fletchBody = sourceBody(fletch);
    const dbTable = /db_find\(\s*([A-Za-z][A-Za-z0-9_]*)\s*:\s*log\s*,\s*\$log\s*\)/.exec(fletchBody)?.[1];
    if (!dbTable) throw new Error('server_ordered: fletch_log does not look up its log row by table');
    if (!new RegExp(`db_getfield\\(\\$data,\\s*${dbTable}:shafts,\\s*0\\s*\\)`).test(fletchBody)
        || !/if\s*\(\s*\$shaft_count\s*<\s*1\s*\)/.test(fletchBody)) {
        throw new Error('server_ordered: fletch_log shafts Make gate changed');
    }
    const dbRows = parseInterfaceDefinitions(CUT_LOGS_DBROW_SOURCE, requireGatherText(contentRoot, CUT_LOGS_DBROW_SOURCE))
        .filter(row => (row.fields.get('table') ?? []).some(field => field.value === dbTable));
    if (dbRows.length === 0) throw new Error(`server_ordered: no ${dbTable} log recipe rows`);
    const objSections = parseInterfaceDefinitions(LOG_OBJ_SOURCE, requireGatherText(contentRoot, LOG_OBJ_SOURCE));
    const logs: ServerOrderedContractIdentity[] = [];
    const logSpans: ServerOrderedSourceSpan[] = [];
    for (const row of dbRows) {
        const logFields = (row.fields.get('data') ?? []).filter(field => field.value.startsWith('log,'));
        if (logFields.length !== 1) throw new Error(`server_ordered: ${row.alias} does not name one log`);
        const logAlias = logFields[0].value.slice('log,'.length);
        if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(logAlias)) throw new Error(`server_ordered: ${row.alias} names an invalid log`);
        const shaftsFields = (row.fields.get('data') ?? []).filter(field => field.value.startsWith('shafts,'));
        if (shaftsFields.length > 1) throw new Error(`server_ordered: ambiguous shafts count in ${row.alias}`);
        const shafts = shaftsFields.length === 0 ? 0 : Number(shaftsFields[0].value.slice('shafts,'.length));
        if (!Number.isInteger(shafts) || shafts < 0) throw new Error(`server_ordered: invalid shafts count in ${row.alias}`);
        if (shafts < 1) continue;
        const log = resolvePackAlias(contentRoot, OBJ_PACK, logAlias, 'server_ordered');
        const configs = objSections.filter(section => section.alias === logAlias);
        if (configs.length !== 1) throw new Error(`server_ordered: expected one [${logAlias}] obj config`);
        const categories = configs[0].fields.get('category') ?? [];
        if (categories.length !== 1 || categories[0].value !== categoryName) {
            throw new Error(`server_ordered: [${logAlias}] is not in the OPHELDU recipe category`);
        }
        logs.push({ alias: logAlias, id: log.id });
        logSpans.push(span(CUT_LOGS_DBROW_SOURCE, row.first, row.last));
        logSpans.push(span(LOG_OBJ_SOURCE, configs[0].first, configs[0].last));
        logSpans.push(span(OBJ_PACK, log.line));
    }
    logs.sort((left, right) => left.id - right.id);
    if (logs.length === 0) throw new Error('server_ordered: no log recipe reaches the Make menu');
    const fruitBlocks = parseRs2Blocks(CUTTING_FRUIT_SOURCE, requireGatherText(contentRoot, CUTTING_FRUIT_SOURCE));
    const knifeHeaders = fruitBlocks.filter(block => block.kind === 'opheldu' && block.name === knifeAlias);
    if (knifeHeaders.length !== 1) throw new Error('server_ordered: expected one knife-specific OPHELDU delegation');
    const fruitCases = switchCases(sourceBody(knifeHeaders[0]), 'switch_obj', 'last_useitem');
    const fallback = fruitCases.find(row => row.label === 'default');
    if (!fallback
        || !new RegExp(`oc_category\\(last_useitem\\)\\s*=\\s*${categoryName}`).test(fallback.body)
        || !/@fletch_log\s*\(\s*last_useitem\s*\)/.test(fallback.body)) {
        throw new Error('server_ordered: knife-specific OPHELDU no longer delegates its category to fletch_log');
    }
    for (const log of logs) {
        if (fruitCases.some(row => row.label.split(',').map(part => part.trim()).includes(log.alias))) {
            throw new Error(`server_ordered: [${log.alias}] bypasses the knife category delegation`);
        }
    }
    return {
        knife: { alias: knifeAlias, id: knife.id },
        logs,
        sources: mergeSpans([
            opheldus[0].span,
            fletch.span,
            knifeHeaders[0].span,
            ...logSpans,
            span(OBJ_PACK, knife.line),
            span(CATEGORY_PACK, category.line),
        ]),
    };
}

function buildEntries(
    bankerPage: ServerOrderedBankerPage | null,
    bankFacts: ServerOrderedBankFacts,
    bankCloseSources: ServerOrderedCloseModalFacts,
    bankBooth: ServerOrderedBankBoothFacts,
    character: ServerOrderedCharacterAcceptanceFacts,
    knifeSources: ServerOrderedSourceSpan[],
    countResumeVerified: boolean,
) {
    const makeEntry = (
        key: ServerOrderedEntryKey,
        components: string[],
        predecessors: ServerOrderedEntry['predecessors'],
        sources: ServerOrderedSourceSpan[],
        checkedConditions: ServerOrderedCondition[] = [],
    ): ServerOrderedEntry => {
        const policy = SERVER_ORDERED_RECIPE_POLICY[key];
        const withCheckedConditions = (conditions: ServerOrderedCondition[]) => checkedConditions.length === 0
            ? conditions
            : conditions.map(condition => checkedConditions.find(checked => checked.name === condition.name) ?? condition);
        return {
            key,
            components,
            server_predicates: withCheckedConditions(policy.server_predicates),
            predecessors,
            max_packet_cost: policy.max_packet_cost,
            outcome: withCheckedConditions(policy.outcome),
            deadline_ticks: policy.deadline_ticks,
            recovery_count: policy.recovery_count,
            cleanup_effects: withCheckedConditions(policy.cleanup_effects),
            sources,
        };
    };
    const bankCloseConditions = Object.entries(bankCloseSources.cleanup_effect_sources)
        .map(([name, sources]) => checkedCondition(name, sources));
    const entries = [
        makeEntry('knife_arm', ['skill_multi3', 'skill_multi3:make1_a'], SERVER_ORDERED_DISPATCH_ORDER_POLICY.knife_arm, knifeSources),
        makeEntry('bank_withdraw_x', ['bank_main', 'bank_main:com_92', 'bank_main:inv', 'bank_main:com_93', 'bank_main:com_94', 'bank_main:com_96', 'bank_main:com_98'], [
            {
                from: `INV_BUTTON${bankFacts.bank.withdraw_x_op}(bank_main:inv)`,
                to: 'RESUME_P_COUNTDIALOG',
                basis: 'checked',
                synchronous: bankFacts.countPromptSynchronous.withdraw && countResumeVerified,
                sources: mergeSpans([...bankFacts.spans, span(RESUME_P_COUNT_DIALOG_SOURCE, 6, 16)]),
            },
        ], mergeSpans([
            ...bankFacts.spans,
            ...bankFacts.bank.sources,
            span(RESUME_P_COUNT_DIALOG_SOURCE, 6, 16),
        ])),
        makeEntry('bank_deposit_x', ['bank_side', 'bank_side:inv'], [
            {
                from: `INV_BUTTON${bankFacts.bank.deposit_x_op}(bank_side:inv)`,
                to: 'RESUME_P_COUNTDIALOG',
                basis: 'checked',
                synchronous: bankFacts.countPromptSynchronous.deposit && countResumeVerified,
                sources: mergeSpans([...bankFacts.spans, span(RESUME_P_COUNT_DIALOG_SOURCE, 6, 16)]),
            },
        ], mergeSpans([
            ...bankFacts.spans,
            ...bankFacts.bank.sources,
            span(RESUME_P_COUNT_DIALOG_SOURCE, 6, 16),
        ])),
        makeEntry('bank_deposit_all', ['bank_side', 'bank_side:inv'], [], bankFacts.spans),
        ...(bankerPage === null ? [] : [
            makeEntry('bank_open_deposit', [
                'multi2', 'multi2:com_1', bankerPage.page_root_alias, bankerPage.continue_component_alias,
                'bank_main', 'bank_main:com_92', 'bank_main:inv', 'bank_side', 'bank_side:inv',
            ], [
                hostOrder('RECEIVED_MULTI2_CHOICE', 'IF_BUTTON(multi2:com_1)'),
                hostOrder('IF_BUTTON(multi2:com_1)', 'RESUME_PAUSEBUTTON(player_page)'),
                hostOrder('RESUME_PAUSEBUTTON(player_page)', 'INV_BUTTON4(bank_side:inv)'),
            ], mergeSpans([
                ...bankerPage.sources,
                ...bankFacts.spans,
                ...bankFacts.bank.sources,
            ]), [
                checkedCondition('banker_player_text_is_one_selected_font_page', bankerPage.sources),
            ]),
        ]),
        makeEntry('bank_close_terminal', ['bank_main', 'bank_side', 'bank_main:inv', 'bank_side:inv'], [], bankCloseSources.sources, bankCloseConditions),
        makeEntry('numeric_bankbooth', ['bank_main', 'bank_main:com_92', 'bank_main:inv', 'bank_side', 'bank_side:inv'], [], bankBooth.numericLoc.sources),
        makeEntry('accept_character_design', ['player_kit', 'player_kit:accept'], SERVER_ORDERED_DISPATCH_ORDER_POLICY.accept_character_design, character.spans),
    ];
    return entries satisfies ServerOrderedEntry[];
}

/** Extract the catalog from pinned content and a decoded selected-cache snapshot. */
export function extractServerOrderedFacts(input: ServerOrderedExtractInput): ServerOrderedCatalog {
    const { contentRoot, selectedPack, sourceCommits, cache, engineSources } = input;
    assertSelection(selectedPack, sourceCommits);
    for (const relative of serverOrderedContentFiles) requireGatherText(contentRoot, relative);

    const bankerPage = tryProveBankerPage(contentRoot, cache);
    const booth = extractBankBooth(contentRoot);
    const pageAliases = bankerPage === null ? [] : [bankerPage.page_root_alias, bankerPage.continue_component_alias];
    const initialAliases = [...baseComponentAliases, ...pageAliases];
    const definitionCache = new Map<string, InterfaceDefinition[]>();
    const aliases = collectRequiredAliases(contentRoot, initialAliases, definitionCache);
    const identities = resolveInterfaceAliases(contentRoot, aliases, 'server_ordered');
    const componentRecords = buildComponents(contentRoot, aliases, identities, cache, definitionCache);
    const bankFacts = extractBankFacts(contentRoot, identities);
    const countResumeVerified = verifyCountResumeHandler(engineSources[RESUME_P_COUNT_DIALOG_SOURCE]);
    const bankCloseSources = extractCloseModal(contentRoot, engineSources);
    const character = extractCharacterAcceptance(contentRoot);
    const knifeSources = extractKnifeSource(contentRoot);
    const knife = extractKnifeFacts(contentRoot);
    const entries = buildEntries(bankerPage, bankFacts, bankCloseSources, booth, character, knifeSources, countResumeVerified);

    return {
        schema: SERVER_ORDERED_SCHEMA,
        revision: selectedPack.revision,
        engine_commit: selectedPack.engine_commit,
        content_commit: selectedPack.content_commit,
        cache_id: selectedPack.cache_id,
        content_id: selectedPack.content_id,
        components: aliases.map(alias => {
            const component = componentRecords.get(alias);
            if (!component) throw new Error(`server_ordered: component extraction omitted ${alias}`);
            return component;
        }),
        entries,
        banker_page: bankerPage,
        bank: bankFacts.bank,
        numeric_locs: [booth.numericLoc],
        knife,
    };
}

/**
 * Load the pinned engine's real cache decoders, then call the fixture-friendly pure extractor.
 * The engine root is read-only. Imports resolve through that pinned engine package's `#` map.
 */
export async function loadServerOrderedFacts(input: ServerOrderedLoadInput): Promise<ServerOrderedCatalog> {
    const selectedPack: ServerOrderedSelection = {
        revision: input.revision,
        engine_commit: input.engineCommit,
        content_commit: input.contentCommit,
        cache_id: input.cacheId,
        content_id: input.contentId,
    };
    const sourceCommits = {
        engine_commit: readGitHead(input.engineRoot, 'engine'),
        content_commit: readGitHead(input.contentRoot, 'content'),
    };
    assertSelection(selectedPack, sourceCommits);

    type EngineComponent = {
        id: number;
        rootLayer: number;
        comType: number;
        buttonType: number;
        clientCode: number;
        width: number;
        height: number;
        marginX: number;
        marginY: number;
        childId: ArrayLike<number> | null;
        scroll: number;
        hide: boolean;
        draggable: boolean;
        operable: boolean;
        usable: boolean;
        swappable: boolean;
        iop: (string | null)[] | null;
        actionVerb: string | null;
        action: string | null;
        actionTarget: number;
        fill: boolean;
        center: boolean;
        font: number;
        shadowed: boolean;
        text: string | null;
        activeText: string | null;
        colour: number;
        activeColour: number;
        overColour: number;
        activeOverColour: number;
        graphic: string | null;
        activeGraphic: string | null;
        model: number;
        activeModel: number;
        anim: number;
        activeAnim: number;
        zoom: number;
        xan: number;
        yan: number;
        option: string | null;
        comName: string | null;
    };
    type ComponentDecoder = { load(dir: string): void; get(id: number): EngineComponent | undefined };
    type FontInstance = { split(text: string, maxWidth: number): string[] };
    type FontDecoder = { load(dir: string): void; get(id: number): FontInstance | undefined };
    type MesanimInstance = { debugname: string | null };
    type MesanimDecoder = { load(dir: string): void; getId(alias: string): number; get(id: number): MesanimInstance | undefined };

    const engineBase = path.resolve(input.engineRoot);
    const engineSources: ServerOrderedEngineSources = {
        [PLAYER_SOURCE]: requireGatherText(engineBase, PLAYER_SOURCE),
        [RESUME_P_COUNT_DIALOG_SOURCE]: requireGatherText(engineBase, RESUME_P_COUNT_DIALOG_SOURCE),
    };
    const [componentModule, fontModule, mesanimModule] = await Promise.all([
        import(pathToFileURL(path.join(engineBase, 'src/cache/config/Component.ts')).href) as Promise<{ default: ComponentDecoder }>,
        import(pathToFileURL(path.join(engineBase, 'src/cache/config/FontType.ts')).href) as Promise<{ default: FontDecoder }>,
        import(pathToFileURL(path.join(engineBase, 'src/cache/config/MesanimType.ts')).href) as Promise<{ default: MesanimDecoder }>,
    ]);
    const cacheRoot = path.join(engineBase, 'data/pack');
    componentModule.default.load(cacheRoot);
    fontModule.default.load(cacheRoot);
    mesanimModule.default.load(cacheRoot);

    const components: ServerOrderedCacheComponent[] = [];
    for (const id of selectedPackIds(input.contentRoot)) {
        const decoded = componentModule.default.get(id);
        if (!decoded) continue;
        components.push({
            id: decoded.id,
            name: decoded.comName,
            root_layer: decoded.rootLayer,
            component_type: decoded.comType,
            button_type: decoded.buttonType,
            client_code: decoded.clientCode,
            width: decoded.width,
            height: decoded.height,
            child_ids: decoded.childId ? Array.from(decoded.childId) : null,
            margin_x: decoded.marginX,
            margin_y: decoded.marginY,
            scroll: decoded.scroll,
            hidden: decoded.hide,
            draggable: decoded.draggable,
            operable: decoded.operable,
            usable: decoded.usable,
            swappable: decoded.swappable,
            actions: decoded.iop ? [...decoded.iop] : null,
            action_verb: decoded.actionVerb,
            action: decoded.action,
            action_target: decoded.actionTarget,
            fill: decoded.fill,
            centered: decoded.center,
            font_id: decoded.font,
            shadowed: decoded.shadowed,
            text: decoded.text,
            active_text: decoded.activeText,
            colour: decoded.colour,
            active_colour: decoded.activeColour,
            over_colour: decoded.overColour,
            active_over_colour: decoded.activeOverColour,
            graphic: decoded.graphic,
            active_graphic: decoded.activeGraphic,
            model: decoded.model,
            active_model: decoded.activeModel,
            anim: decoded.anim,
            active_anim: decoded.activeAnim,
            zoom: decoded.zoom,
            xan: decoded.xan,
            yan: decoded.yan,
            option: decoded.option,
        });
    }

    const fonts: ServerOrderedCacheFont[] = readFontTypeRows(engineBase).map(({ alias, id }) => {
        const decoded = fontModule.default.get(id);
        if (!decoded) throw new Error(`server_ordered: selected cache font ${alias} is missing`);
        return { alias, id, split: decoded.split.bind(decoded) };
    });
    const mesanimAlias = BANKER_MESANIM;
    const mesanimId = mesanimModule.default.getId(mesanimAlias);
    const mesanim = mesanimId < 0 ? undefined : mesanimModule.default.get(mesanimId);
    if (!mesanim || mesanim.debugname !== mesanimAlias) throw new Error(`server_ordered: selected cache mesanim ${mesanimAlias} is missing`);

    return extractServerOrderedFacts({
        contentRoot: input.contentRoot,
        selectedPack,
        sourceCommits,
        cache: {
            components,
            fonts,
            mesanims: [{ alias: mesanimAlias, id: mesanimId }],
        },
        engineSources,
    });
}
