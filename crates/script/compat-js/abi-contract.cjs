'use strict';

const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

const BASE_PREDECESSORS = ['F0', 'T1.3', 'T2.1'];
const ACTIVE_FAMILIES = ['bank', 'skills'];
// These nav values are expected assertions only; generated classifications
// come from frozen import graphs and native backend source evidence.
const SECTION_FAMILIES = new Map([
    ['root', { id: 'compat-runtime', nav: false }],
    ['world primitives', { id: 'world-primitives', nav: false }],
    ['execution', { id: 'execution', nav: false }],
    ['game state', { id: 'game-state', nav: false }],
    ['entities and queries', { id: 'entities', nav: false }],
    ['hud', { id: 'inventory', nav: false }],
    ['high-level banking', { id: 'bank', nav: true }],
    ['movement', { id: 'walking', nav: true }],
    ['events', { id: 'events', nav: false }],
    ['bot base classes', { id: 'bot', nav: false }],
    ['item acquisition', { id: 'acquisition', nav: true }],
    ['manifest', { id: 'bot', nav: false }],
    ['world catalogs (data tables and pure helpers)', { id: 'world-catalogs', nav: false }],
]);

const DECLARATION_FAMILY_OVERRIDES = new Map([
    ['BranchTask', 'bot'],
    ['LeafTask', 'bot'],
    ['TreeNode', 'bot'],
    ['QuestStatus', 'questlog'],
    ['TradeItem', 'trade'],
    ['BankItemSnapshot', 'bank'],
    ['BankObjectAccess', 'bank'],
]);

const WORLD_CATALOGS_SECTION = 'world catalogs (data tables and pure helpers)';
const WORLD_CATALOG_FAMILY_GROUPS = [
    ['bank', ['BankRequirement', 'BankLocation', 'BANK_LOCATIONS', 'bankDistance', 'nearestUsableBank', 'bankUnlocked', 'nearestBank']],
    ['acquisition', ['ToolTier', 'ToolReq', 'PICKAXES', 'AXES', 'TINDERBOX', 'HAMMER', 'KNIFE', 'CHISEL', 'NEEDLE', 'pickaxeReq', 'axeReq', 'exactTool', 'tinderboxReq', 'toolAttackLevel', 'canWieldTool', 'bestFromTiers', 'bestPickaxe', 'bestAxe', 'toolKeepNames', 'hasToolReq', 'hasAllTools', 'missingToolLabels', 'toolKitLabel', 'ToolRestockStep', 'toolRestockPlan', 'bankHasBetterGatherTool', 'toolsNeedingEquip', 'bestHeldToolNames', 'surplusHeldToolNames', 'COINS', 'BROKEN_PICKAXE', 'BROKEN_AXE', 'ToolAcquireMode', 'parseToolAcquireMode', 'TOOL_ACQUIRE_OPTIONS', 'TOOL_ACQUIRE_SETTING', 'FORGETFUL_BANK_ODDS', 'FORGETFUL_BANK_SETTING', 'ToolVendor', 'ShopOffer', 'BOB_VENDOR', 'NURMOF_VENDOR', 'GERRANT_VENDOR', 'HARRY_VENDOR', 'VARROCK_ANVIL_STAND', 'VARROCK_ANVIL_BANK', 'PICKAXE_SHOP_COSTS', 'AXE_SHOP_COSTS', 'AXE_SMITH_LEVEL', 'AXE_BAR_FOR', 'ToolAcquirePlan', 'AcquireWorld', 'bestOwnedTier', 'pickaxeShopOffers', 'axeShopOffers', 'bestAffordableShopTier', 'bestSmithableAxe', 'planBrokenToolRepair', 'planPickaxeAcquire', 'planAxeAcquire', 'planGatherToolAcquire', 'coinsToWithdraw', 'canFundPlan', 'acquireKeepNames']],
    ['fishing', ['GERRANT_ONLY_FISHING', 'FISHING_SHOP_COSTS', 'FishingVendorNear', 'fishingVendorFor', 'fishingShopCost', 'isFishingBaitPiece', 'withBaitTarget', 'PlanFishingGearOpts', 'FishingGearBuyPlan', 'planFishingGearBuys', 'planFishingGearAcquire', 'buyPlansCost', 'fishingGearShopCart', 'shopableMissingFishingGear', 'FishingLocation', 'FISHING_LOCATIONS', 'FISHING_LOCATION_OPTIONS', 'resolveFishingLocation', 'FishingGearPiece', 'FishingMethod', 'WHIRLPOOL_IDS', 'FISHING_METHODS', 'FISHING_METHOD_OPTIONS', 'ALL_FISHING_GEAR_NAMES', 'resolveFishMethod', 'gearKeepNames', 'hasFishingGear', 'missingFishingGear', 'gearLabel', 'fishingRestockPlan', 'spotMatchesMethod']],
    ['thieving', ['PickpocketTarget', 'PICKPOCKET_TARGETS', 'PICKPOCKET_TARGET_NAMES', 'ARDOUGNE_PICKPOCKET_TARGETS']],
    ['gathering', ['GatheringLocation', 'DEFAULT_BOOTH_NAME', 'DEFAULT_BOOTH_OP', 'MAP_SQUARE', 'sameMapSquare', 'locationOptions', 'boothFields', 'resolveGatheringLocation']],
    ['mining', ['MiningLocation', 'MINING_LOCATIONS', 'MINING_LOCATION_OPTIONS', 'MINING_LOCATION_OPTION_LABELS', 'miningLocationLabel', 'resolveMiningLocation', 'ROCK_TYPES', 'ROCK_OPTIONS', 'GAS_ROCK_IDS', 'GAS_ROCK_TICKS', 'resolveRockIds']],
    ['woodcutting', ['WoodcuttingLocation', 'WOODCUTTING_LOCATIONS', 'WOODCUTTING_LOCATION_OPTIONS', 'resolveWoodcuttingLocation', 'ENT_NPC_IDS', 'ENT_LIFE_TICKS', 'isEntNpcId', 'entNpcOnTile']],
    ['walking', ['WalkDestination', 'WALK_DESTINATIONS', 'WALK_OPTIONS', 'resolveDestination']],
    ['cow', ['CowLocation', 'COW_LOCATIONS', 'COW_LOCATION_OPTIONS', 'AL_KHARID_BANK', 'TOLL_COIN_TARGET', 'isCowFieldLootTile', 'resolveCowLocation', 'nearestCowLocation', 'needsTollCoins', 'shouldBootstrapTollCoins']],
    ['runecrafting', ['RuneRoute', 'RUNES', 'RuneType', 'RUNE_OPTIONS', 'DEFAULT_RUNE']],
    ['client-adapter', ['reader']],
];
const WORLD_CATALOG_FAMILY_BY_NAME = new Map();
for (const [familyId, names] of WORLD_CATALOG_FAMILY_GROUPS) {
    for (const name of names) {
        if (WORLD_CATALOG_FAMILY_BY_NAME.has(name)) fail(`Duplicate world-catalog family classification for ${name}`);
        WORLD_CATALOG_FAMILY_BY_NAME.set(name, familyId);
    }
}
const ADDITIONAL_FAMILY_POLICIES = [
    { id: 'fishing', nav: false },
    { id: 'gathering', nav: false },
    { id: 'mining', nav: false },
    { id: 'woodcutting', nav: false },
    { id: 'cow', nav: false },
    { id: 'runecrafting', nav: false },
    { id: 'catalog-data', nav: false },
];

// These are explicit source-domain classifications, not a catch-all fallback.
// A newly served module outside these domains must be classified before generation.
const MODULE_RULES = [
    ['@rs2b0t/api', { id: 'compat-runtime', nav: false }],
    ['api/ai/clues/', { id: 'clues', nav: true }],
    ['api/ai/quests/', { id: 'quests', nav: true }],
    ['api/combat/hunting/', { id: 'hunt', nav: true }],
    ['api/combat/', { id: 'combat', nav: false }],
    ['api/walking/', { id: 'walking', nav: true }],
    ['event/webwalk/', { id: 'walking', nav: true }],
    ['api/duel/', { id: 'duel', nav: true }],
    ['api/tasks/', { id: 'tasks', nav: true }],
    ['api/bank/', { id: 'bank', nav: true }],
    ['api/trade/', { id: 'trade', nav: false }],
    ['api/shop/', { id: 'shop', nav: true }],
    ['api/ui/dialogue/', { id: 'dialogue', nav: false }],
    ['api/ui/widgets/', { id: 'dialogue', nav: false }],
    ['api/ui/questlog/', { id: 'questlog', nav: false }],
    ['api/execution/', { id: 'execution', nav: false }],
    ['api/bot/', { id: 'bot', nav: false }],
    ['api/game/', { id: 'game-state', nav: false }],
    ['api/inventory/', { id: 'inventory', nav: false }],
    ['api/equipment/', { id: 'inventory', nav: false }],
    ['api/skills/', { id: 'skills', nav: false }],
    ['api/query/', { id: 'entities', nav: false }],
    ['api/npcs/', { id: 'entities', nav: false }],
    ['api/players/', { id: 'entities', nav: false }],
    ['api/locs/', { id: 'entities', nav: false }],
    ['api/grounditems/', { id: 'entities', nav: false }],
    ['api/model/', { id: 'entities', nav: false }],
    ['api/acquisition/', { id: 'acquisition', nav: true }],
    ['api/thieving/', { id: 'thieving', nav: true }],
    ['api/firemaking/', { id: 'firemaking', nav: false }],
    ['api/cooking/', { id: 'cooking', nav: false }],
    ['api/prayer/', { id: 'prayer', nav: false }],
    ['api/magic/', { id: 'magic', nav: false }],
    ['api/sustain/', { id: 'combat', nav: false }],
    ['api/chatbox/', { id: 'chatbox', nav: false }],
    ['api/market/', { id: 'market', nav: false }],
    ['api/loadout/', { id: 'loadouts', nav: false }],
    ['adapter/', { id: 'client-adapter', nav: false }],
    ['geometry/', { id: 'world-primitives', nav: false }],
    ['data/cowKillerLocations.js', { id: 'cow', nav: false }],
    ['data/miningRocks.js', { id: 'mining', nav: false }],
    ['data/pickpocketTargets.js', { id: 'thieving', nav: true }],
    ['data/woodcuttingLocations.js', { id: 'woodcutting', nav: false }],
    ['data/runeCraftLocations.js', { id: 'runecrafting', nav: false }],
    ['data/cookLocations.js', { id: 'cooking', nav: false }],
    ['data/spelldb.js', { id: 'catalog-data', nav: false }],
    ['data/itemdb.js', { id: 'catalog-data', nav: false }],
    ['data/herbs.js', { id: 'catalog-data', nav: false }],
    ['data/dropdb.js', { id: 'catalog-data', nav: false }],
    ['data/shopdb.js', { id: 'catalog-data', nav: false }],
    ['data/', { id: 'catalog-data', nav: false }],
    ['runtime/', { id: 'runtime', nav: true }],
    ['paint/', { id: 'paint', nav: false }],
    ['input/', { id: 'input', nav: false }],
    ['event/', { id: 'walking', nav: true }],
    ['shim/', { id: 'compat-runtime', nav: false }],
];


function fail(message) {
    throw new Error(message);
}

function parseSource(text, fileName) {
    const source = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    if (source.parseDiagnostics.length) {
        const diagnostic = source.parseDiagnostics[0];
        fail(`Unable to parse ${fileName}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n')}`);
    }
    return source;
}

function exported(node) {
    return (node.modifiers || []).some(modifier =>
        modifier.kind === ts.SyntaxKind.ExportKeyword || modifier.kind === ts.SyntaxKind.DefaultKeyword,
    );
}

function nodeName(node, source) {
    if (!node) return null;
    if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return node.text;
    if (ts.isStringLiteralLike(node) || ts.isNumericLiteral(node)) return node.text;
    return node.getText(source);
}

function internal(node) {
    return ts.getJSDocTags(node).some(tag => tag.tagName.text === 'internal');
}

function privateMember(node) {
    return (node.modifiers || []).some(modifier =>
        modifier.kind === ts.SyntaxKind.PrivateKeyword || modifier.kind === ts.SyntaxKind.ProtectedKeyword,
    );
}

function sectionSpans(source) {
    const spans = [{ offset: 0, section: 'root' }];
    const lines = source.text.split(/\n/);
    let offset = 0;
    const heading = /^\/\/ ([a-z][a-z0-9 -]*(?: \([^)]*\))?)$/;
    for (const line of lines) {
        const match = line.match(heading);
        if (match) {
            if (!SECTION_FAMILIES.has(match[1])) fail(`Unclassified ABI section: ${match[1]}`);
            spans.push({ offset, section: match[1] });
        }
        offset += line.length + 1;
    }
    return spans;
}

function sectionAt(spans, offset) {
    let low = 0;
    let high = spans.length;
    while (low + 1 < high) {
        const middle = (low + high) >>> 1;
        if (spans[middle].offset <= offset) low = middle;
        else high = middle;
    }
    return spans[low].section;
}

function moduleFamilyId(module) {
    const rule = MODULE_RULES
        .filter(([prefix]) => module === prefix || module.startsWith(prefix))
        .sort((a, b) => b[0].length - a[0].length)[0];
    if (!rule) fail(`Missing ABI family classification for module ${module}`);
    return rule[1].id;
}

function familyPolicies() {
    return [
        ...SECTION_FAMILIES.values(),
        ...MODULE_RULES.map(([, value]) => value),
        ...ADDITIONAL_FAMILY_POLICIES,
    ];
}

function familyPolicyById(id, navFamilies = null) {
    const candidates = familyPolicies().filter(policy => policy.id === id);
    const expected = new Set(candidates.map(policy => policy.nav));
    if (!candidates.length || expected.size !== 1) fail(`Missing or conflicting policy for family ${id}`);
    const nav = navFamilies ? navFamilies.has(id) : candidates[0].nav;
    if (navFamilies && nav !== candidates[0].nav) {
        fail(`Navigation classification disagrees with code evidence for family ${id}: policy=${candidates[0].nav}, code=${nav}`);
    }
    return { id, nav };
}

function classifyModule(module, navFamilies = null) {
    return familyPolicyById(moduleFamilyId(module), navFamilies);
}

function familyForDeclaration(name, section, navFamilies = null) {
    const override = DECLARATION_FAMILY_OVERRIDES.get(name);
    if (override) return familyPolicyById(override, navFamilies);
    if (section === WORLD_CATALOGS_SECTION) {
        const familyId = WORLD_CATALOG_FAMILY_BY_NAME.get(name);
        if (!familyId) fail(`Missing world-catalog family classification for ${name}`);
        return familyPolicyById(familyId, navFamilies);
    }
    const policy = SECTION_FAMILIES.get(section);
    if (!policy) fail(`Missing ABI family classification for ${name} in ${section}`);
    return familyPolicyById(policy.id, navFamilies);
}

// The module registry owns the served URL-to-shim relation. Resolve callback
// and directly called backend families from those shim callers; ambiguous reuse
// is resolved only by same-module sibling families, otherwise generation fails closed.
function deriveNativeFamilyFamilies(callbackSnapshot, registry, shimSources) {
    if (callbackSnapshot?.schema_version !== 2 || !Array.isArray(callbackSnapshot.rows)) fail('Missing native callback census schema version 2');
    const nativeFamilies = new Set(callbackSnapshot.rows.flatMap(row => row.families || []));
    // Include every literal backend a registered shim calls, even if it has no callback rows.
    for (const entry of registry) {
        const text = shimSources[entry.shim];
        if (typeof text !== 'string') fail(`Missing registered shim source ${entry.shim}`);
        const source = parseSource(text, entry.shim);
        const findCalledFamilies = node => {
            if (
                ts.isCallExpression(node) &&
                ts.isIdentifier(node.expression) &&
                node.expression.text === 'runMachine'
            ) {
                const family = node.arguments[0];
                if (family && ts.isStringLiteralLike(family)) nativeFamilies.add(family.text);
            }
            ts.forEachChild(node, findCalledFamilies);
        };
        findCalledFamilies(source);
    }
    const callers = new Map([...nativeFamilies].map(name => [name, []]));
    for (const entry of registry) {
        const text = shimSources[entry.shim];
        if (typeof text !== 'string') fail(`Missing registered shim source ${entry.shim}`);
        const source = parseSource(text, entry.shim);
        const familyId = moduleFamilyId(entry.module);
        const names = new Set();
        let hasDynamicFamilyCall = false;
        const findFamilyCalls = node => {
            if (
                ts.isCallExpression(node) &&
                ts.isIdentifier(node.expression) &&
                node.expression.text === 'runMachine'
            ) {
                const family = node.arguments[0];
                if (family && ts.isStringLiteralLike(family) && nativeFamilies.has(family.text)) names.add(family.text);
                else hasDynamicFamilyCall = true;
            }
            ts.forEachChild(node, findFamilyCalls);
        };
        findFamilyCalls(source);
        if (hasDynamicFamilyCall) {
            const findDynamicFamilies = node => {
                if (ts.isStringLiteralLike(node) && nativeFamilies.has(node.text)) names.add(node.text);
                ts.forEachChild(node, findDynamicFamilies);
            };
            findDynamicFamilies(source);
        }
        for (const name of names) callers.get(name).push({ family_id: familyId, module: entry.module });
    }
    const result = new Map();
    for (const name of nativeFamilies) {
        const entries = callers.get(name);
        if (!entries.length) continue;
        const familyIds = new Set(entries.map(entry => entry.family_id));
        if (familyIds.size === 1) {
            result.set(name, entries[0].family_id);
            continue;
        }
        const support = new Map([...familyIds].map(id => [id, new Set()]));
        for (const entry of entries) {
            for (const [otherName, otherEntries] of callers) {
                if (otherName === name) continue;
                const otherFamilyIds = new Set(otherEntries.map(other => other.family_id));
                if (
                    otherFamilyIds.size === 1 &&
                    otherFamilyIds.has(entry.family_id) &&
                    otherEntries.some(other => other.module === entry.module)
                ) support.get(entry.family_id).add(otherName);
            }
        }
        const ranked = [...support].sort((a, b) => b[1].size - a[1].size);
        if (!ranked[0][1].size || ranked[0][1].size === ranked[1][1].size) {
            fail(`Ambiguous registered shim callers for native family ${name}: ${[...familyIds].join(', ')}`);
        }
        result.set(name, ranked[0][0]);
    }
    // Internal native subfamilies with no direct runMachine caller inherit
    // only from repeated, unanimous callback-census co-membership.
    for (let pass = 0; pass < nativeFamilies.size; pass++) {
        let changed = false;
        for (const name of nativeFamilies) {
            if (result.has(name)) continue;
            const support = new Map();
            for (const row of callbackSnapshot.rows) {
                if (!row.families.includes(name)) continue;
                const siblingOwners = new Set(row.families
                    .filter(sibling => sibling !== name && result.has(sibling))
                    .map(sibling => result.get(sibling)));
                if (siblingOwners.size === 1) {
                    const owner = [...siblingOwners][0];
                    support.set(owner, (support.get(owner) || 0) + 1);
                }
            }
            const ranked = [...support].sort((a, b) => b[1] - a[1]);
            if (ranked[0] && ranked[0][1] >= 2 && ranked[0][1] > (ranked[1]?.[1] || 0)) {
                result.set(name, ranked[0][0]);
                changed = true;
            }
        }
        if (!changed) break;
    }
    for (const name of nativeFamilies) {
        if (!result.has(name)) fail(`No registered shim caller or shared callback owner for native family ${name}`);
    }
    return result;
}

const NAV_IMPORT_NAMES = new Set([
    'DirectNavigator', 'Navigator', 'PathFinder', 'Reach',
    'Reachability', 'Traversal', 'Walk', 'WalkExecutor', 'Walker',
]);
const NAV_RUST_USE = /\b(?:crate::walk::|native::walk::|walk::|crate::reach_entity::|reach_entity::|NpcReach\b|nav::|InteractReq::WalkTo\b)/;

function hasRuntimeModuleBinding(node) {
    if (ts.isImportDeclaration(node)) {
        const clause = node.importClause;
        if (!clause) return true;
        if (clause.isTypeOnly) return false;
        const bindings = clause.namedBindings;
        return !bindings ||
            !ts.isNamedImports(bindings) ||
            bindings.elements.length === 0 ||
            bindings.elements.some(element => !element.isTypeOnly);
    }
    if (ts.isExportDeclaration(node)) {
        if (node.isTypeOnly) return false;
        const clause = node.exportClause;
        return !clause ||
            !ts.isNamedExports(clause) ||
            clause.elements.length === 0 ||
            clause.elements.some(element => !element.isTypeOnly);
    }
    return false;
}

function hasNavigationImport(source) {
    let found = false;
    const visit = node => {
        if (
            (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
            hasRuntimeModuleBinding(node)
        ) {
            const specifier = node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)
                ? node.moduleSpecifier.text
                : '';
            const pathUse = /(?:^|\/)(?:walking|webwalk)(?:\/|$)/.test(specifier);
            const bindings = ts.isImportDeclaration(node)
                ? node.importClause?.namedBindings
                : node.exportClause;
            const nameUse = bindings &&
                (ts.isNamedImports(bindings) || ts.isNamedExports(bindings)) &&
                bindings.elements.some(element =>
                    !element.isTypeOnly && NAV_IMPORT_NAMES.has((element.propertyName || element.name).text));
            if (pathUse || nameUse) found = true;
        }
        if (!found) ts.forEachChild(node, visit);
    };
    visit(source);
    return found;
}

function frozenSourceModuleFile(botRoot, module) {
    if (typeof module !== 'string' || !module.endsWith('.js')) return null;
    const file = path.resolve(botRoot, `${module.slice(0, -3)}.ts`);
    if (!file.startsWith(`${botRoot}${path.sep}`)) fail(`Registered source module escapes frozen bot root: ${module}`);
    return fs.existsSync(file) && fs.statSync(file).isFile() ? file : null;
}

function sourceModuleFamilyId(botRoot, file) {
    const module = path.relative(botRoot, file)
        .split(path.sep)
        .join('/')
        .replace(/\.ts$/, '.js');
    const rule = MODULE_RULES
        .filter(([prefix]) => module === prefix || module.startsWith(prefix))
        .sort((a, b) => b[0].length - a[0].length)[0];
    return rule?.[1].id || null;
}

function frozenSourceImportCandidates(botRoot, file, specifier) {
    const sourceRoot = path.dirname(botRoot);
    const unresolved = specifier.startsWith('.')
        ? path.resolve(path.dirname(file), specifier)
        : specifier.startsWith('#/')
            ? path.resolve(sourceRoot, specifier.slice(2))
            : null;
    if (!unresolved || !unresolved.startsWith(`${botRoot}${path.sep}`)) return [];
    const extension = path.extname(unresolved);
    const candidates = extension === '.js'
        ? [`${unresolved.slice(0, -3)}.ts`]
        : extension === '.ts'
            ? [unresolved]
            : extension
                ? []
                : [`${unresolved}.ts`, path.join(unresolved, 'index.ts')];
    return candidates.filter(candidate =>
        candidate.startsWith(`${botRoot}${path.sep}`) &&
        fs.existsSync(candidate) &&
        fs.statSync(candidate).isFile(),
    );
}

function frozenSourceImportTargets(botRoot, file, source, familyId) {
    const targets = new Set();
    const visit = node => {
        if (
            (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
            hasRuntimeModuleBinding(node) &&
            node.moduleSpecifier &&
            ts.isStringLiteralLike(node.moduleSpecifier)
        ) {
            for (const candidate of frozenSourceImportCandidates(botRoot, file, node.moduleSpecifier.text)) {
                if (sourceModuleFamilyId(botRoot, candidate) === familyId) targets.add(candidate);
            }
        }
        ts.forEachChild(node, visit);
    };
    visit(source);
    return targets;
}

function frozenDeclaredSourceRoots(root, botRoot) {
    const frozenRoot = path.dirname(root);
    const declarationsFile = path.join(frozenRoot, 'packages/rs2b0t-api/index.d.ts');
    const runtimeIndexFile = path.join(frozenRoot, 'packages/rs2b0t-api/index.js');
    const abiFile = path.join(botRoot, 'runtime/abi.ts');
    const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });
    const declaredRuntimeRecords = new Map(
        declarations(parseSource(fs.readFileSync(declarationsFile, 'utf8'), declarationsFile), printer)
            .filter(record => record.runtime)
            .map(record => [record.name, record]),
    );
    const runtimeNames = new Set(runtimeExports(
        fs.readFileSync(runtimeIndexFile, 'utf8'),
        runtimeIndexFile,
    ));
    for (const name of declaredRuntimeRecords.keys()) {
        if (!runtimeNames.has(name)) declaredRuntimeRecords.delete(name);
    }

    const abiSource = parseSource(fs.readFileSync(abiFile, 'utf8'), abiFile);
    const imports = new Map();
    const namespaceImports = new Map();
    for (const statement of abiSource.statements) {
        if (
            !ts.isImportDeclaration(statement) ||
            !statement.importClause ||
            !ts.isStringLiteralLike(statement.moduleSpecifier)
        ) continue;
        const clause = statement.importClause;
        if (clause.isTypeOnly) continue;
        const specifier = statement.moduleSpecifier.text;
        if (clause.name) imports.set(clause.name.text, specifier);
        const bindings = clause.namedBindings;
        if (bindings && ts.isNamespaceImport(bindings)) {
            namespaceImports.set(bindings.name.text, specifier);
        } else if (bindings && ts.isNamedImports(bindings)) {
            for (const element of bindings.elements) {
                if (!element.isTypeOnly) imports.set(element.name.text, specifier);
            }
        }
    }

    let abiObject = null;
    const findAbiObject = node => {
        if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === 'abi') {
            const initializer = node.initializer;
            if (
                initializer &&
                ts.isCallExpression(initializer) &&
                ts.isPropertyAccessExpression(initializer.expression) &&
                initializer.expression.expression.getText(abiSource) === 'Object' &&
                initializer.expression.name.text === 'freeze' &&
                initializer.arguments[0] &&
                ts.isObjectLiteralExpression(initializer.arguments[0])
            ) {
                abiObject = initializer.arguments[0];
            }
        }
        if (!abiObject) ts.forEachChild(node, findAbiObject);
    };
    findAbiObject(abiSource);
    if (!abiObject) fail(`Unable to locate frozen runtime ABI object in ${abiFile}`);

    const roots = new Map();
    for (const property of abiObject.properties) {
        const name = ts.isShorthandPropertyAssignment(property)
            ? property.name.text
            : ts.isPropertyAssignment(property)
                ? nodeName(property.name, abiSource)
                : null;
        if (!name || !declaredRuntimeRecords.has(name)) continue;
        const record = declaredRuntimeRecords.get(name);
        const value = ts.isShorthandPropertyAssignment(property)
            ? property.name
            : ts.isPropertyAssignment(property)
                ? property.initializer
                : null;
        const specifier = value && ts.isIdentifier(value)
            ? imports.get(value.text)
            : value && ts.isPropertyAccessExpression(value) && ts.isIdentifier(value.expression)
                ? namespaceImports.get(value.expression.text)
                : null;
        if (!specifier) continue;
        for (const file of frozenSourceImportCandidates(botRoot, abiFile, specifier)) {
            const familyId = sourceModuleFamilyId(botRoot, file) || familyForDeclaration(name, record.section).id;
            roots.set(`${file}\0${familyId}`, { file, familyId });
        }
    }
    return [...roots.values()];
}

// Registered modules.json entries are served roots. Runtime-declared ABI members
// without a served source module add roots; only same-family imports are followed.
// Cross-family dependencies remain classified by their own ABI, and unserved siblings are not roots.
function frozenSourceNavFamilies(root, registry) {
    const botRoot = path.resolve(root, 'bot');
    const navFamilies = new Set();
    const roots = [];
    const servedRootFiles = new Set();
    for (const entry of registry) {
        const rootFile = frozenSourceModuleFile(botRoot, entry.module);
        if (!rootFile) continue;
        roots.push({ file: rootFile, familyId: moduleFamilyId(entry.module) });
        servedRootFiles.add(rootFile);
    }
    for (const declaredRoot of frozenDeclaredSourceRoots(root, botRoot)) {
        if (!servedRootFiles.has(declaredRoot.file)) roots.push(declaredRoot);
    }
    for (const { file: rootFile, familyId } of roots) {
        const pending = [rootFile];
        const visited = new Set();
        while (pending.length) {
            const file = pending.pop();
            if (visited.has(file)) continue;
            visited.add(file);
            const source = parseSource(fs.readFileSync(file, 'utf8'), file);
            if (hasNavigationImport(source)) navFamilies.add(familyId);
            for (const dependency of frozenSourceImportTargets(botRoot, file, source, familyId)) pending.push(dependency);
        }
    }
    return navFamilies;
}

function rustSources(root) {
    const files = [];
    const visit = directory => {
        for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
            const file = path.join(directory, entry.name);
            if (entry.isDirectory()) visit(file);
            else if (entry.isFile() && file.endsWith('.rs')) files.push(file);
        }
    };
    visit(root);
    return files;
}

function rustProductionSource(source) {
    const testModule = /^\s*#\[cfg\(test\)\]\s*(?:\r?\n\s*)?(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_]\w*\s*\{/m.exec(source);
    const production = testModule ? source.slice(0, testModule.index) : source;
    return production
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/\/\/.*$/gm, '');
}

function rustFamilySources(root, nativeFamilies) {
    const result = new Map();
    for (const file of rustSources(root)) {
        const source = rustProductionSource(fs.readFileSync(file, 'utf8'));
        for (const match of source.matchAll(/\bconst\s+NAME\s*:\s*[^=;]+?=\s*["']([^"']+)["']/g)) {
            if (!nativeFamilies.has(match[1])) continue;
            const files = result.get(match[1]) || [];
            files.push({ file, source });
            result.set(match[1], files);
        }
    }
    return result;
}

function deriveNavFamilies({ frozenSourceRoot, nativeSourceRoot, nativeFamilyFamilies, registry }) {
    const navFamilies = frozenSourceNavFamilies(frozenSourceRoot, registry);
    const nativeFamilySources = rustFamilySources(nativeSourceRoot, new Set(nativeFamilyFamilies.keys()));
    for (const [nativeFamily, familyId] of nativeFamilyFamilies) {
        const sources = nativeFamilySources.get(nativeFamily) || [];
        if (sources.some(source => NAV_RUST_USE.test(source.source))) navFamilies.add(familyId);
    }
    return navFamilies;
}

function printNode(node, source, printer) {
    return printer.printNode(ts.EmitHint.Unspecified, node, source).replace(/\r\n/g, '\n').trim();
}


function directMembers(nodes, source, printer, familyId, prefix, branch = '') {
    const groups = new Map();
    for (const member of nodes) {
        if (internal(member) || privateMember(member)) continue;
        let name;
        let kind;
        if (ts.isConstructorDeclaration(member)) {
            name = 'constructor';
            kind = 'constructor';
        } else if (ts.isCallSignatureDeclaration(member)) {
            name = '<call>';
            kind = 'call-signature';
        } else if (ts.isConstructSignatureDeclaration(member)) {
            name = '<construct>';
            kind = 'construct-signature';
        } else if (ts.isIndexSignatureDeclaration(member)) {
            name = '<index>';
            kind = 'index-signature';
        } else if (ts.isGetAccessorDeclaration(member) || ts.isSetAccessorDeclaration(member)) {
            name = nodeName(member.name, source);
            kind = 'accessor';
        } else if (ts.isMethodSignature(member) || ts.isMethodDeclaration(member)) {
            name = nodeName(member.name, source);
            kind = 'method';
        } else if (ts.isPropertySignature(member) || ts.isPropertyDeclaration(member)) {
            name = nodeName(member.name, source);
            kind = 'property';
        } else {
            continue;
        }
        if (!name) fail(`Unclassified ABI member in ${prefix}`);
        const key = `${branch}|${name}|${kind}`;
        let row = groups.get(key);
        if (!row) {
            row = {
                name,
                kind,
                path: branch ? `${prefix}.${branch}.${name}` : `${prefix}.${name}`,
                family_id: familyId,
                signatures: [],
            };
            groups.set(key, row);
        }
        row.signatures.push({
            ordinal: row.signatures.length + 1,
            family_id: familyId,
            text: printNode(member, source, printer),
        });
    }
    return [...groups.values()];
}

function aliasTypeMembers(type, source, printer, familyId, owner, branch = '') {
    if (!type) return [];
    if (ts.isParenthesizedTypeNode(type)) return aliasTypeMembers(type.type, source, printer, familyId, owner, branch);
    if (ts.isTypeLiteralNode(type)) return directMembers(type.members, source, printer, familyId, owner, branch);
    if (ts.isUnionTypeNode(type) || ts.isIntersectionTypeNode(type)) {
        const members = [];
        type.types.forEach((variant, index) => {
            const variantPath = branch ? `${branch}-variant-${index + 1}` : `variant-${index + 1}`;
            members.push(...aliasTypeMembers(variant, source, printer, familyId, owner, variantPath));
        });
        return members;
    }
    return [];
}

function declarations(source, printer) {
    const spans = sectionSpans(source);
    const records = [];
    const functions = new Map();
    const add = record => {
        const prior = records.find(item => item.name === record.name && item.kind === record.kind);
        if (prior && record.kind !== 'function') fail(`Duplicate exported ABI declaration ${record.name}`);
        if (prior) {
            prior.signatures.push(...record.signatures);
            return prior;
        }
        records.push(record);
        return record;
    };
    for (const statement of source.statements) {
        if (internal(statement) || !exported(statement)) continue;
        const section = sectionAt(spans, statement.getStart(source));
        if (ts.isFunctionDeclaration(statement)) {
            const name = nodeName(statement.name, source);
            if (!name || internal(statement)) continue;
            let record = functions.get(name);
            if (!record) {
                record = {
                    name,
                    kind: 'function',
                    section,
                    source_module: null,
                    runtime: true,
                    signatures: [],
                    members: [],
                    family_id: null,
                };
                functions.set(name, add(record));
            }
            record.signatures.push({ ordinal: record.signatures.length + 1, family_id: null, text: printNode(statement, source, printer) });
            continue;
        }
        if (ts.isClassDeclaration(statement) || ts.isInterfaceDeclaration(statement)) {
            const name = nodeName(statement.name, source) || 'default';
            const family = null;
            const members = directMembers(statement.members, source, printer, family, name);
            const bases = (statement.heritageClauses || []).flatMap(clause => clause.types.map(type => printNode(type, source, printer)));
            add({
                name,
                kind: ts.isClassDeclaration(statement) ? 'class' : 'interface',
                section,
                source_module: null,
                runtime: ts.isClassDeclaration(statement),
                signatures: [],
                members,
                bases,
                family_id: null,
            });
            continue;
        }
        if (ts.isTypeAliasDeclaration(statement)) {
            const name = nodeName(statement.name, source);
            const members = aliasTypeMembers(statement.type, source, printer, null, name);
            add({
                name,
                kind: 'type-alias',
                section,
                source_module: null,
                runtime: false,
                alias_target: printNode(statement.type, source, printer),
                signatures: [],
                members,
                family_id: null,
            });
            continue;
        }
        if (ts.isEnumDeclaration(statement)) {
            const name = nodeName(statement.name, source);
            const members = statement.members.map((member, index) => ({
                name: nodeName(member.name, source),
                kind: 'enum-member',
                path: `${name}.${nodeName(member.name, source)}`,
                family_id: null,
                signatures: [{ ordinal: index + 1, family_id: null, text: printNode(member, source, printer) }],
            }));
            add({ name, kind: 'enum', section, source_module: null, runtime: true, signatures: [], members, family_id: null });
            continue;
        }
        if (ts.isVariableStatement(statement)) {
            for (const declaration of statement.declarationList.declarations) {
                if (!ts.isIdentifier(declaration.name)) fail(`Unsupported ABI export binding ${declaration.name.getText(source)}`);
                const name = declaration.name.text;
                const type = declaration.type;
                const members = type && ts.isTypeLiteralNode(type)
                    ? directMembers(type.members, source, printer, null, name)
                    : [];
                add({
                    name,
                    kind: 'value',
                    section,
                    source_module: null,
                    runtime: true,
                    signatures: [],
                    members,
                    family_id: null,
                    declaration_type: type ? printNode(type, source, printer) : null,
                });
            }
            continue;
        }
        if (ts.isExportDeclaration(statement)) {
            // The frozen declaration entrypoint is a named declaration surface;
            // an export edge here would need an explicit name-map row.
            if (statement.exportClause && ts.isNamedExports(statement.exportClause)) {
                for (const specifier of statement.exportClause.elements) {
                    const name = specifier.name.text;
                    add({ name, kind: 'alias', section, source_module: null, runtime: !specifier.isTypeOnly && !statement.isTypeOnly, signatures: [], members: [], family_id: null, alias_target: (specifier.propertyName || specifier.name).text });
                }
            }
        }
    }
    return records;
}

function runtimeExports(source, fileName) {
    const file = parseSource(source, fileName);
    const names = [];
    for (const statement of file.statements) {
        if (ts.isVariableStatement(statement) && exported(statement)) {
            for (const declaration of statement.declarationList.declarations) {
                if (!ts.isObjectBindingPattern(declaration.name)) continue;
                for (const element of declaration.name.elements) {
                    const name = nodeName(element.propertyName || element.name, file);
                    if (!name) fail(`Unsupported runtime ABI binding in ${fileName}`);
                    names.push(name);
                }
            }
        }
        if (ts.isExportDeclaration(statement) && statement.exportClause && ts.isNamedExports(statement.exportClause)) {
            for (const specifier of statement.exportClause.elements) names.push(specifier.name.text);
        }
    }
    if (new Set(names).size !== names.length) fail(`Duplicate runtime ABI export in ${fileName}`);
    return names;
}

function rootNameMaps(rootSource, sourceRecords, registry, moduleSymbols, navFamilies) {
    const source = parseSource(rootSource, 'declared_surface.js');
    const moduleShims = new Map(registry.map(entry => [entry.module, entry.shim]));
    const sourceByName = new Map();
    for (const record of sourceRecords) {
        const list = sourceByName.get(record.name) || [];
        list.push(record);
        sourceByName.set(record.name, list);
    }
    const maps = [];
    const byPublicName = new Map();
    const add = row => {
        if (byPublicName.has(row.public_name)) fail(`Duplicate compatibility export mapping ${row.public_name}`);
        byPublicName.set(row.public_name, row);
        maps.push(row);
    };
    for (const statement of source.statements) {
        if (ts.isExportDeclaration(statement) && statement.exportClause && ts.isNamedExports(statement.exportClause)) {
            const raw = statement.moduleSpecifier && ts.isStringLiteralLike(statement.moduleSpecifier)
                ? statement.moduleSpecifier.text
                : null;
            const module = raw ? raw.replace(/^(?:\.\.\/)+/, '') : null;
            if (module && !moduleShims.has(module)) fail(`Compatibility name map points to unregistered module ${module}`);
            for (const specifier of statement.exportClause.elements) {
                const publicName = specifier.name.text;
                const sourceName = (specifier.propertyName || specifier.name).text;
                const sourceRecords = sourceByName.get(publicName) || sourceByName.get(sourceName) || [];
                const sourceRecord = sourceRecords.find(record => record.section === WORLD_CATALOGS_SECTION);
                const policy = sourceRecord
                    ? familyForDeclaration(sourceRecord.name, sourceRecord.section, navFamilies)
                    : module
                        ? classifyModule(module, navFamilies)
                        : null;
                add({
                    public_name: publicName,
                    source_name: sourceName,
                    module,
                    family_id: policy?.id || null,
                    shim: module ? moduleShims.get(module) : null,
                    nav_consuming: policy?.nav ?? null,
                    source_modules: [module],
                    is_alias: publicName !== sourceName || sourceName === 'default',
                    mapping_kind: 're-export',
                });
            }
            continue;
        }
        const addDirect = (name, kind) => {
            const candidates = sourceByName.get(name) || [];
            const sourceInfo = moduleSymbols.get(name);
            const modulePolicy = sourceInfo ? moduleFamilyPolicy(sourceInfo, navFamilies) : null;
            const candidatePolicies = new Set(candidates.map(record => familyForDeclaration(record.name, record.section, navFamilies).id));
            if (candidatePolicies.size > 1 && !modulePolicy) fail(`Ambiguous family classification for direct compatibility export ${name}`);
            const preferred = candidates.find(record =>
                DECLARATION_FAMILY_OVERRIDES.has(record.name) || record.section === WORLD_CATALOGS_SECTION,
            );
            const policy = preferred
                ? familyForDeclaration(preferred.name, preferred.section, navFamilies)
                : modulePolicy || (candidatePolicies.size
                    ? familyPolicyById([...candidatePolicies][0], navFamilies)
                    : null);
            if (!policy) fail(`Missing direct ABI family classification for ${name}`);
            add({
                public_name: name,
                source_name: name,
                module: null,
                shim: null,
                source_modules: sourceInfo?.modules || [],
                family_id: policy.id,
                nav_consuming: policy.nav,
                is_alias: false,
                mapping_kind: kind,
            });
        };
        if (ts.isFunctionDeclaration(statement) && exported(statement)) {
            const name = nodeName(statement.name, source);
            if (name) addDirect(name, 'direct-function');
        } else if ((ts.isClassDeclaration(statement) || ts.isClassExpression(statement)) && exported(statement)) {
            const name = nodeName(statement.name, source);
            if (name) addDirect(name, 'direct-class');
        } else if (ts.isVariableStatement(statement) && exported(statement)) {
            for (const declaration of statement.declarationList.declarations) {
                if (ts.isIdentifier(declaration.name)) addDirect(declaration.name.text, 'direct-value');
            }
        }
    }
    return { rows: maps, byName: byPublicName };
}

function currentRootExports(indexText) {
    const source = parseSource(indexText, 'generated-index.d.ts');
    const names = new Set();
    for (const statement of source.statements) {
        if (!ts.isModuleDeclaration(statement) || nodeName(statement.name, source) !== '@rs2b0t/api') continue;
        if (!statement.body || !ts.isModuleBlock(statement.body)) fail('Malformed @rs2b0t/api declaration module');
        for (const member of statement.body.statements) {
            if (ts.isExportDeclaration(member) && member.exportClause && ts.isNamedExports(member.exportClause)) {
                for (const specifier of member.exportClause.elements) names.add(specifier.name.text);
            } else if ((ts.isClassDeclaration(member) || ts.isFunctionDeclaration(member) || ts.isInterfaceDeclaration(member) || ts.isTypeAliasDeclaration(member)) && exported(member)) {
                const name = nodeName(member.name, source);
                if (name) names.add(name);
            } else if (ts.isVariableStatement(member) && exported(member)) {
                for (const declaration of member.declarationList.declarations) {
                    if (ts.isIdentifier(declaration.name)) names.add(declaration.name.text);
                }
            }
        }
    }
    return names;
}

function typedModuleFamilies(indexText, navFamilies = null) {
    const source = parseSource(indexText, 'generated-index.d.ts');
    const modules = new Map();
    for (const statement of source.statements) {
        if (!ts.isModuleDeclaration(statement)) continue;
        const ambientName = nodeName(statement.name, source);
        const module = ambientName?.startsWith('*') ? ambientName.slice(1) : ambientName;
        if (!statement.body || !ts.isModuleBlock(statement.body)) fail(`Malformed generated ABI module ${module}`);
        if (modules.has(module)) fail(`Duplicate typed module family row for ${module}`);
        const policy = classifyModule(module, navFamilies);
        modules.set(module, { module, family_id: policy.id, nav_consuming: policy.nav });
    }
    return [...modules.values()].sort((a, b) => a.module.localeCompare(b.module));
}

function moduleSymbolIndex(indexText, registry) {
    const source = parseSource(indexText, 'generated-index.d.ts');
    const registered = new Set(registry.map(entry => entry.module));
    const symbols = new Map();
    const add = (name, module) => {
        const modules = symbols.get(name) || new Set();
        modules.add(module);
        symbols.set(name, modules);
    };
    for (const statement of source.statements) {
        if (!ts.isModuleDeclaration(statement)) continue;
        const ambientName = nodeName(statement.name, source);
        if (!ambientName?.startsWith('*')) continue;
        const module = ambientName.slice(1);
        if (!registered.has(module)) fail(`Generated ABI module has no registry row: ${module}`);
        moduleFamilyId(module);
        if (!statement.body || !ts.isModuleBlock(statement.body)) fail(`Malformed generated ABI module ${module}`);
        for (const member of statement.body.statements) {
            if ((ts.isClassDeclaration(member) || ts.isInterfaceDeclaration(member) || ts.isTypeAliasDeclaration(member) || ts.isFunctionDeclaration(member)) && exported(member)) {
                const name = nodeName(member.name, source);
                if (name) add(name, module);
            } else if (ts.isVariableStatement(member) && exported(member)) {
                for (const declaration of member.declarationList.declarations) {
                    if (ts.isIdentifier(declaration.name)) add(declaration.name.text, module);
                }
            } else if (ts.isExportDeclaration(member) && member.exportClause && ts.isNamedExports(member.exportClause)) {
                for (const specifier of member.exportClause.elements) add(specifier.name.text, module);
            }
        }
    }
    return new Map([...symbols].map(([name, modules]) => [name, { modules: [...modules].sort() }]));
}


function moduleFamilyPolicy(sourceInfo, navFamilies) {
    const ids = new Set(sourceInfo.modules.map(module => moduleFamilyId(module)));
    if (ids.size !== 1) return null;
    return familyPolicyById([...ids][0], navFamilies);
}

function collectRootRefusals(rootSource, maps, sourceRecords, navFamilies) {
    const source = parseSource(rootSource, 'declared_surface.js');
    const byName = new Map(sourceRecords.map(record => [record.name, record]));
    const debt = [];
    const familyFor = (owner, directName) => {
        const mapped = maps.byName.get(owner || directName);
        if (mapped?.family_id) return { id: mapped.family_id, nav: mapped.nav_consuming };
        const sourceRecord = byName.get(owner || directName);
        if (sourceRecord) return familyForDeclaration(sourceRecord.name, sourceRecord.section, navFamilies);
        return null;
    };
    const notImplName = node => ts.isCallExpression(node) && node.expression.getText(source) === 'notImpl';
    const isRefusalBody = body => body && ts.isBlock(body) && body.statements.length === 1 &&
        ts.isThrowStatement(body.statements[0]) && notImplName(body.statements[0].expression);
    const push = (selector, member, owner, directName, sourceText) => {
        const policy = familyFor(owner, directName);
        if (!policy) fail(`Missing family classification for refusal ${selector}`);
        debt.push({
            family_id: policy.id,
            member,
            selector,
            module: null,
            state: 'not-impl',
            source: sourceText,
            reason: 'The current compatibility name map unconditionally refuses this declared member.',
        });
    };
    const visit = (node, owner = null) => {
        let nextOwner = owner;
        if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) nextOwner = node.name ? node.name.text : owner;
        if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) nextOwner = node.name.text;
        if ((ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node) || ts.isGetAccessorDeclaration(node) || ts.isSetAccessorDeclaration(node)) && isRefusalBody(node.body)) {
            const member = node.name ? nodeName(node.name, source) : nextOwner;
            if (member) push(nextOwner && member !== nextOwner ? `${nextOwner}.${member}` : member, member, nextOwner, member, 'declared_surface.js');
        }
        if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
            if (isRefusalBody(node.body)) {
                const memberNode = node.parent && (ts.isPropertyAssignment(node.parent) || ts.isPropertyDeclaration(node.parent) || ts.isVariableDeclaration(node.parent))
                    ? node.parent.name
                    : null;
                const member = nodeName(memberNode, source);
                if (member) push(nextOwner ? `${nextOwner}.${member}` : member, member, nextOwner, member, 'declared_surface.js');
            }
        }
        if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && ts.isCallExpression(node.initializer) && node.initializer.expression.getText(source) === 'notImplValue') {
            push(node.name.text, node.name.text, nextOwner, node.name.text, 'declared_surface.js');
        }
        ts.forEachChild(node, child => visit(child, nextOwner));
    };
    visit(source);
    return debt;
}

function collectModuleNotImplValueExports(registry, shimSources, navFamilies) {
    const debt = [];
    for (const entry of registry) {
        const source = parseSource(shimSources[entry.shim], entry.shim);
        const policy = familyPolicyById(moduleFamilyId(entry.module), navFamilies);
        for (const statement of source.statements) {
            if (!ts.isVariableStatement(statement) || !exported(statement)) continue;
            for (const declaration of statement.declarationList.declarations) {
                if (!ts.isIdentifier(declaration.name) || !ts.isCallExpression(declaration.initializer)) continue;
                if (declaration.initializer.expression.getText(source) !== 'notImplValue') continue;
                const member = declaration.name.text;
                const line = source.getLineAndCharacterOfPosition(declaration.getStart(source)).line + 1;
                debt.push({
                    family_id: policy.id,
                    member,
                    selector: member,
                    module: entry.module,
                    state: 'not-impl',
                    source: `crates/script/src/shim/${entry.shim}:${line}`,
                    reason: 'The exported value throws when accessed through notImplValue.',
                });
            }
        }
    }
    return debt;
}

function callbackFamilyRows(callbackSnapshot, nativeFamilyFamilies, navFamilies) {
    if (callbackSnapshot?.schema_version !== 2 || !Array.isArray(callbackSnapshot.rows)) fail('Missing native callback census schema version 2');
    const rows = [];
    for (const row of callbackSnapshot.rows) {
        if (!row.member || !row.path || !row.value || !Array.isArray(row.family_semantics)) {
            fail(`Incomplete callback characterization for ${row.member || '<unnamed>'}`);
        }
        const semantics = new Map();
        for (const item of row.family_semantics) {
            if (!item.family || !item.order || !item.multiplicity || !item.cadence || semantics.has(item.family)) {
                fail(`Incomplete or duplicate callback family semantics for ${row.member}`);
            }
            semantics.set(item.family, item);
        }
        if (!Array.isArray(row.families) || semantics.size !== row.families.length) {
            fail(`Callback family semantics do not match family expansion for ${row.member}`);
        }
        for (const nativeFamily of row.families) {
            const meaning = semantics.get(nativeFamily);
            if (!meaning) fail(`Missing callback family semantics for ${row.member}/${nativeFamily}`);
            const familyId = nativeFamilyFamilies.get(nativeFamily);
            if (!familyId) fail(`Missing caller-derived family for native family ${nativeFamily}`);
            const policy = familyPolicyById(familyId, navFamilies);
            rows.push({
                family_id: policy.id,
                native_family: nativeFamily,
                member: row.member,
                call_site: row.call_site,
                path: row.path,
                value: row.value,
                promise: row.promise,
                thrown: row.thrown,
                order: meaning.order,
                multiplicity: meaning.multiplicity,
                cadence: meaning.cadence,
                frozen_path: row.frozen_path,
                evidence: row.evidence,
            });
        }
    }
    return rows;
}

function addCounts(counts, id, navFamilies) {
    let row = counts.get(id);
    if (!row) {
        const policy = familyPolicyById(id, navFamilies);
        row = { family_id: id, nav_consuming: policy.nav, declarations: 0, members: 0, overloads: 0, aliases: 0, type_aliases: 0, name_aliases: 0, callbacks: 0, not_impl_debt: 0 };
        counts.set(id, row);
    }
    return row;
}
function assignFamilies(records, nameMap, moduleSymbols, navFamilies) {
    for (const record of records) {
        const mapped = nameMap.byName.get(record.name);
        const sourceInfo = moduleSymbols.get(record.name);
        const sectionIsAuthoritative = !record.runtime
            || DECLARATION_FAMILY_OVERRIDES.has(record.name)
            || record.section === WORLD_CATALOGS_SECTION;
        const policy = mapped?.family_id
            ? familyPolicyById(mapped.family_id, navFamilies)
            : sectionIsAuthoritative
                ? familyForDeclaration(record.name, record.section, navFamilies)
                : (sourceInfo && moduleFamilyPolicy(sourceInfo, navFamilies)) || familyForDeclaration(record.name, record.section, navFamilies);
        record.source_modules = mapped?.source_modules || (mapped?.module ? [mapped.module] : sourceInfo?.modules || []);
        record.source_module = mapped?.module || (record.source_modules.length === 1 ? record.source_modules[0] : null);
        record.family_id = policy.id;
        record.nav_consuming = policy.nav;
        for (const member of record.members) {
            member.family_id = policy.id;
            for (const signature of member.signatures) signature.family_id = policy.id;
        }
        for (const signature of record.signatures) signature.family_id = policy.id;
    }
}

function buildArtifacts({
    frozenRevision,
    frozenIndex,
    frozenRuntimeIndex,
    currentIndex,
    rootSource,
    registry,
    overlays,
    callbackSnapshot,
    nativeFamilyFamilies,
    navFamilies,
    shimSources,
}) {
    const knownFamilyIds = new Set(familyPolicies().map(policy => policy.id));
    for (const familyId of navFamilies) {
        if (!knownFamilyIds.has(familyId)) fail(`Navigation evidence has no family classification for ${familyId}`);
    }
    for (const familyId of knownFamilyIds) familyPolicyById(familyId, navFamilies);

    const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });
    const frozen = parseSource(frozenIndex, 'packages/rs2b0t-api/index.d.ts');
    const sourceRecords = declarations(frozen, printer);
    const moduleSymbols = moduleSymbolIndex(currentIndex, registry);
    const typedModules = typedModuleFamilies(currentIndex, navFamilies);
    const typedModuleNames = new Set(typedModules.map(row => row.module));
    for (const entry of registry) {
        if (!typedModuleNames.has(entry.module)) fail(`Served ABI module has no typed declaration: ${entry.module}`);
    }
    const nameMap = rootNameMaps(rootSource, sourceRecords, registry, moduleSymbols, navFamilies);
    const sourceRuntimeNames = runtimeExports(frozenRuntimeIndex, 'packages/rs2b0t-api/index.js');
    const sourceRecordNames = new Set(sourceRecords.filter(record => record.runtime).map(record => record.name));
    for (const name of sourceRuntimeNames) {
        if (!sourceRecordNames.has(name)) fail(`Frozen runtime export ${name} is missing from index.d.ts`);
        if (!nameMap.byName.has(name)) fail(`Frozen runtime export ${name} is missing a compatibility name map`);
    }
    for (const name of sourceRecordNames) {
        if (!sourceRuntimeNames.includes(name)) fail(`Runtime declaration ${name} is not in frozen index.js`);
    }
    const currentNames = currentRootExports(currentIndex);
    for (const map of nameMap.rows) {
        if (!currentNames.has(map.public_name)) fail(`Compatibility name map ${map.public_name} is absent from generated declarations`);
    }
    for (const name of currentNames) {
        if (!nameMap.byName.has(name)) fail(`Generated compatibility export ${name} has no name-map classification`);
    }
    assignFamilies(sourceRecords, nameMap, moduleSymbols, navFamilies);

    const classifiedOverlays = overlays.map(entry => {
        const selectorParts = entry.selector.split('.');
        const sourceRecord = sourceRecords.find(record => record.name === selectorParts[0])
            || sourceRecords.find(record => record.name === selectorParts.at(-1));
        const sourceFamilyIsAuthoritative = sourceRecord && (
            sourceRecord.section === WORLD_CATALOGS_SECTION
            || DECLARATION_FAMILY_OVERRIDES.has(sourceRecord.name)
            || entry.module === '@rs2b0t/api'
        );
        const policy = sourceFamilyIsAuthoritative
            ? familyForDeclaration(sourceRecord.name, sourceRecord.section, navFamilies)
            : entry.module ? classifyModule(entry.module, navFamilies) : familyPolicyById('compat-runtime', navFamilies);
        if (!entry.id || !entry.selector || !policy) fail(`Unclassified ABI overlay ${entry.id || '<unnamed>'}`);
        return { ...entry, family_id: policy.id, nav_consuming: policy.nav };
    });
    const debtCandidates = classifiedOverlays
        .filter(entry => entry.operation === 'not-impl')
        .map(entry => ({
            family_id: entry.family_id,
            member: entry.selector.split('.').at(-1),
            selector: entry.selector,
            module: entry.module,
            state: 'not-impl',
            source: entry.runtime,
            reason: entry.reason,
            overlay_id: entry.id,
        }));
    debtCandidates.push(...collectModuleNotImplValueExports(registry, shimSources, navFamilies));
    debtCandidates.push(...collectRootRefusals(rootSource, nameMap, sourceRecords, navFamilies));
    const uniqueDebt = new Map();
    for (const row of debtCandidates) if (!uniqueDebt.has(row.selector)) uniqueDebt.set(row.selector, row);
    const debt = [...uniqueDebt.values()];
    const callbacks = callbackFamilyRows(callbackSnapshot, nativeFamilyFamilies, navFamilies);

    const counts = new Map();
    for (const row of typedModules) addCounts(counts, row.family_id, navFamilies);
    for (const record of sourceRecords) {
        const row = addCounts(counts, record.family_id, navFamilies);
        row.declarations++;
        if (record.kind === 'type-alias') {
            row.aliases++;
            row.type_aliases++;
        }
        row.members += record.members.length;
        row.overloads += Math.max(0, record.signatures.length - 1)
            + record.members.reduce((count, member) => count + Math.max(0, member.signatures.length - 1), 0);
    }
    for (const map of nameMap.rows) {
        if (map.is_alias) {
            const row = addCounts(counts, map.family_id, navFamilies);
            row.aliases++;
            row.name_aliases++;
        }
    }
    for (const callback of callbacks) addCounts(counts, callback.family_id, navFamilies).callbacks++;
    for (const row of debt) addCounts(counts, row.family_id, navFamilies).not_impl_debt++;
    for (const row of classifiedOverlays) addCounts(counts, row.family_id, navFamilies);

    const familyCounts = [...counts.values()].sort((a, b) => a.family_id.localeCompare(b.family_id));
    const familyPredecessors = familyCounts.map(row => {
        const predecessors = [...BASE_PREDECESSORS];
        if (row.family_id === 'bank') predecessors.push('T2.2');
        if (row.nav_consuming) predecessors.push('T5.1');
        return {
            family_id: row.family_id,
            nav_consuming: row.nav_consuming,
            activation_unit: `I-family(${row.family_id})`,
            activation_endpoint: row.family_id === 'bank' ? 'T2.3' : 'T2.rest',
            activation_state: ACTIVE_FAMILIES.includes(row.family_id) ? 'activated' : 'not-activated',
            canonical_endpoint_predecessors: row.family_id === 'bank'
                ? ['F0', 'T1.3', 'T2.2', 'T5.1']
                : predecessors,
            predecessors,
            counts: {
                declarations: row.declarations,
                members: row.members,
                overloads: row.overloads,
                aliases: row.aliases,
                callbacks: row.callbacks,
                type_aliases: row.type_aliases,
                name_aliases: row.name_aliases,
                not_impl_debt: row.not_impl_debt,
            },
        };
    });
    const contract = {
        schema_version: 1,
        endpoint: 'T2.1',
        activation_state: 'partially-activated',
        frozen_source: {
            repository: 'https://github.com/rs2b2t/rs2b0t.git',
            revision: frozenRevision,
            runtime_declarations: 'packages/rs2b0t-api/index.js',
            typed_declarations: 'packages/rs2b0t-api/index.d.ts',
        },
        declarations: sourceRecords,
        compatibility_name_maps: nameMap.rows,
        typed_modules: typedModules,
        overlays: classifiedOverlays,
        callbacks,
        not_impl_debt: debt,
        active_families: ACTIVE_FAMILIES,
        family_counts: familyCounts,
    };
    const predecessors = {
        schema_version: 1,
        endpoint: 'T2.1',
        frozen_revision: frozenRevision,
        base_predecessors: BASE_PREDECESSORS,
        families: familyPredecessors,
    };
    validateArtifacts(contract, predecessors, navFamilies, currentIndex);
    return {
        'abi-contract.json': `${JSON.stringify(contract, null, 2)}\n`,
        'family-predecessors.json': `${JSON.stringify(predecessors, null, 2)}\n`,
    };
}

function validateArtifacts(contract, predecessors, navFamilies = null, currentIndex = null) {
    if (contract.activation_state !== 'partially-activated' ||
        JSON.stringify(contract.active_families) !== JSON.stringify(ACTIVE_FAMILIES)) {
        fail('Activated families must match the complete family units');
    }
    const familyIds = new Set(predecessors.families.map(row => row.family_id));
    if (familyIds.size !== predecessors.families.length) fail('Duplicate family predecessor row');
    const checkFamily = (row, where) => {
        if (!row.family_id || !familyIds.has(row.family_id)) fail(`Missing family predecessor classification for ${where}`);
    };
    if (!Array.isArray(contract.typed_modules)) fail('Missing typed module family inventory');
    const typedModules = new Map();
    for (const row of contract.typed_modules) {
        checkFamily(row, `typed module ${row.module}`);
        if (typedModules.has(row.module)) fail(`Duplicate typed module family row for ${row.module}`);
        const policy = classifyModule(row.module, navFamilies);
        if (row.family_id !== policy.id || row.nav_consuming !== policy.nav) {
            fail(`Typed module family classification disagrees with policy for ${row.module}`);
        }
        typedModules.set(row.module, row);
    }
    if (currentIndex !== null) {
        const expectedModules = typedModuleFamilies(currentIndex, navFamilies);
        for (const row of expectedModules) {
            if (!typedModules.has(row.module)) fail(`Missing typed module family row for ${row.module}`);
        }
        if (expectedModules.length !== typedModules.size) fail('Unexpected typed module family row');
    }
    for (const declaration of contract.declarations) {
        checkFamily(declaration, declaration.name);
        for (const member of declaration.members) {
            checkFamily(member, `${declaration.name}.${member.name}`);
            for (const signature of member.signatures) checkFamily(signature, member.path);
        }
        for (const signature of declaration.signatures) checkFamily(signature, declaration.name);
    }
    for (const collection of ['compatibility_name_maps', 'overlays', 'callbacks', 'not_impl_debt']) {
        for (const row of contract[collection]) checkFamily(row, row.selector || row.member || row.public_name || collection);
    }
    for (const family of predecessors.families) {
        const expectedState = ACTIVE_FAMILIES.includes(family.family_id) ? 'activated' : 'not-activated';
        if (family.activation_state !== expectedState) fail(`Unexpected activation state for family ${family.family_id}`);
        for (const required of BASE_PREDECESSORS) {
            if (!family.predecessors.includes(required)) fail(`Family ${family.family_id} is missing predecessor ${required}`);
        }
        if (navFamilies) {
            const nav = navFamilies.has(family.family_id);
            familyPolicyById(family.family_id, navFamilies);
            if (family.nav_consuming !== nav) {
                fail(`Navigation classification disagrees with code evidence for family ${family.family_id}`);
            }
            const classified = contract.family_counts.find(row => row.family_id === family.family_id);
            if (!classified || classified.nav_consuming !== nav) {
                fail(`Contract navigation classification disagrees with code evidence for family ${family.family_id}`);
            }
            for (const collection of ['declarations', 'compatibility_name_maps', 'overlays']) {
                for (const row of contract[collection]) {
                    if (row.family_id === family.family_id && typeof row.nav_consuming === 'boolean' && row.nav_consuming !== nav) {
                        fail(`Navigation classification disagrees with code evidence for ${row.selector || row.name || row.public_name}`);
                    }
                }
            }
            if (nav && !family.predecessors.includes('T5.1')) fail(`Nav-consuming family ${family.family_id} is missing T5.1`);
            if (!nav && family.predecessors.includes('T5.1')) fail(`Non-nav family ${family.family_id} must not include T5.1`);
        } else if (family.nav_consuming && !family.predecessors.includes('T5.1')) {
            fail(`Nav-consuming family ${family.family_id} is missing T5.1`);
        }
        if (family.family_id === 'bank' && !family.predecessors.includes('T2.2')) fail('Bank family is missing T2.2');
        const canonical = family.canonical_endpoint_predecessors;
        if (!Array.isArray(canonical)) fail(`Missing canonical endpoint predecessors for ${family.family_id}`);
        const canonicalExpected = family.family_id === 'bank'
            ? ['F0', 'T1.3', 'T2.2', 'T5.1']
            : family.predecessors;
        if (JSON.stringify(canonical) !== JSON.stringify(canonicalExpected)) {
            fail(`Non-canonical endpoint predecessor list for ${family.family_id}`);
        }
        if (family.predecessors.some(predecessor => /consolidated backend/i.test(predecessor))) fail(`Vague predecessor in ${family.family_id}`);
    }
    const debtSelectors = new Set();
    for (const debt of contract.not_impl_debt) {
        if (debtSelectors.has(debt.selector)) fail(`Duplicate not-impl debt selector ${debt.selector}`);
        debtSelectors.add(debt.selector);
        if (debt.state !== 'not-impl' || typeof debt.member !== 'string' || !debt.member.trim()) {
            fail(`Unlabelled not-impl debt at ${debt.selector || '<unknown>'}`);
        }
    }
}

function assertGenerated(expected, actual, label = 'ABI contract') {
    if (expected !== actual) fail(`${label} differs from deterministic generation`);
}

module.exports = {
    assertGenerated,
    buildArtifacts,
    callbackFamilyRows,
    classifyModule,
    collectModuleNotImplValueExports,
    deriveNativeFamilyFamilies,
    deriveNavFamilies,
    validateArtifacts,
};
