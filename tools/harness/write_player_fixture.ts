/**
 * Offline player-save fixture writer for production harness prepare.
 *
 * Uses the server's own Player.save() + PlayerLoading.load/verify — does not
 * reimplement the .sav byte layout or hardcode item/varp IDs. Resolves names
 * through packed configs under <server-root>/data/pack.
 *
 * Run from the server engine package root (so `#/` imports resolve):
 *   cd <server-root> && npx tsx <path-to-this-file> -- \
 *     --username NAME --output PATH.sav [--profile main] [--fixture thiever] \
 *     [--overwrite] [--receipt PATH.json]
 *
 * Never writes under a path unless --output is explicit. Refuses to overwrite
 * an existing .sav unless --overwrite is set. Does not edit engine runtime.
 */

import crypto from 'crypto';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

import InvType from '#/cache/config/InvType.js';
import ObjType from '#/cache/config/ObjType.js';
import ParamType from '#/cache/config/ParamType.js';
import VarPlayerType from '#/cache/config/VarPlayerType.js';
import Player, { getExpByLevel, getLevelByExp } from '#/engine/entity/Player.js';
import { PlayerLoading } from '#/engine/entity/PlayerLoading.js';
import { PlayerStat, PlayerStatMap } from '#/engine/entity/PlayerStat.js';
import Packet from '#/io/Packet.js';
import { toBase37, fromBase37 } from '#/util/JString.js';

type StatSpec = { skill: string; level: number };
type ItemSpec = { name: string; count: number; inv?: 'inv' | 'bank' };

type FixturePreset = {
    id: string;
    x: number;
    z: number;
    level: number;
    tutorial: number;
    stats: StatSpec[];
    items: ItemSpec[];
};

/** Named presets derived from scenario fixture_prereqs (not script XP). */
const PRESETS: Record<string, FixturePreset> = {
    thiever: {
        id: 'thiever',
        x: 2661,
        z: 3306,
        level: 0,
        tutorial: 1000,
        stats: [
            { skill: 'thieving', level: 50 },
            { skill: 'hitpoints', level: 50 },
        ],
        items: [
            { name: 'lobster', count: 10, inv: 'inv' },
            // Auto food banking needs durable stock after the initial pack is
            // eaten; the server-native writer persists this in the bank tab.
            { name: 'lobster', count: 200, inv: 'bank' },
        ],
    },
    bone_burier_v2: {
        id: 'bone_burier_v2',
        x: 3220,
        z: 3212,
        level: 0,
        tutorial: 1000,
        stats: [],
        items: [
            { name: 'bones', count: 5, inv: 'inv' },
            { name: 'bones', count: 28, inv: 'bank' },
        ],
    },
};

type Args = {
    username: string;
    output: string;
    receipt: string | null;
    profile: string;
    fixture: string;
    overwrite: boolean;
    serverRoot: string;
};

function usage(msg?: string): never {
    if (msg) console.error(`error: ${msg}`);
    console.error(
        'usage: write_player_fixture --username NAME --output PATH.sav [--fixture thiever|bone_burier_v2] [--profile main] [--overwrite] [--receipt PATH.json] [--server-root PATH]'
    );
    process.exit(2);
}

function parseArgs(argv: string[]): Args {
    let username = '';
    let output = '';
    let receipt: string | null = null;
    let profile = 'main';
    let fixture = 'thiever';
    let overwrite = false;
    // When launched via `cd serverRoot && npx tsx thisfile`, cwd is the engine root.
    let serverRoot = process.cwd();

    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        const next = () => {
            const v = argv[++i];
            if (v === undefined) usage(`missing value for ${a}`);
            return v;
        };
        switch (a) {
            case '--username':
                username = next();
                break;
            case '--output':
                output = next();
                break;
            case '--receipt':
                receipt = next();
                break;
            case '--profile':
                profile = next();
                break;
            case '--fixture':
                fixture = next();
                break;
            case '--overwrite':
                overwrite = true;
                break;
            case '--server-root':
                serverRoot = path.resolve(next());
                break;
            case '--help':
            case '-h':
                usage();
            default:
                if (a === '--') continue;
                usage(`unknown arg ${a}`);
        }
    }
    if (!username.trim()) usage('--username required');
    if (!/^[a-z0-9_]{1,12}$/i.test(username)) {
        usage('--username must be 1-12 alnum/underscore (engine safe name)');
    }
    if (!output.trim()) usage('--output required (isolated .sav path)');
    output = path.resolve(output);
    if (receipt) receipt = path.resolve(receipt);
    return { username, output, receipt, profile, fixture, overwrite, serverRoot };
}

function loadPacks(packDir: string) {
    if (!fs.existsSync(path.join(packDir, 'server', 'obj.dat'))) {
        throw new Error(`missing pack data under ${packDir}`);
    }
    // Order matches World bootstrap: params before objs (members autodisable).
    ParamType.load(packDir);
    VarPlayerType.load(packDir);
    ObjType.load(packDir);
    InvType.load(packDir);
    if (VarPlayerType.count < 1 || ObjType.configs.length < 1 || InvType.count < 1) {
        throw new Error(`pack load produced empty tables under ${packDir}`);
    }
    if (InvType.INV < 0) {
        throw new Error('InvType.INV unresolved after pack load');
    }
}

function setSkillLevel(player: Player, skill: string, level: number) {
    const key = skill.toUpperCase();
    const stat = PlayerStatMap.get(key);
    if (typeof stat === 'undefined') {
        throw new Error(`unknown skill ${skill}`);
    }
    const lv = Math.min(99, Math.max(1, level | 0));
    // setLevel uses getExpByLevel which is undefined for level 1; mirror empty-save defaults.
    player.baseLevels[stat] = lv;
    player.levels[stat] = lv;
    player.stats[stat] = lv <= 1 ? 0 : getExpByLevel(lv);
}

function giveItem(player: Player, name: string, count: number, invName: 'inv' | 'bank') {
    const objId = ObjType.getId(name.toLowerCase());
    if (objId === -1) {
        throw new Error(`obj debugname not in pack: ${name}`);
    }
    const invId = invName === 'inv' ? InvType.INV : InvType.getId('bank');
    if (invId < 0) {
        throw new Error(`inventory type unresolved: ${invName}`);
    }
    const added = player.invAdd(invId, objId, count);
    if (added < count) {
        throw new Error(`invAdd shortfall for ${name}: wanted ${count}, added ${added}`);
    }
}

function applyPreset(player: Player, preset: FixturePreset) {
    player.x = preset.x;
    player.z = preset.z;
    player.level = preset.level;

    // Baseline: empty-save defaults (HP 10, rest 1) via PlayerStatMap names.
    for (const [name, id] of PlayerStatMap.entries()) {
        setSkillLevel(player, name, id === PlayerStat.HITPOINTS ? 10 : 1);
    }

    for (const s of preset.stats) {
        setSkillLevel(player, s.skill, s.level);
    }

    const tutorialId = VarPlayerType.getId('tutorial');
    if (tutorialId < 0) {
        throw new Error('varp debugname "tutorial" missing from pack');
    }
    player.vars[tutorialId] = preset.tutorial;

    for (const it of preset.items) {
        giveItem(player, it.name, it.count, it.inv ?? 'inv');
    }

    player.combatLevel = player.getCombatLevel();
}

function sha256(buf: Uint8Array): string {
    return crypto.createHash('sha256').update(buf).digest('hex');
}

function assertRoundtrip(username: string, bytes: Uint8Array, preset: FixturePreset) {
    const pkt = new Packet(bytes);
    if (!PlayerLoading.verify(new Packet(bytes))) {
        throw new Error('PlayerLoading.verify failed on written save');
    }
    const loaded = PlayerLoading.load(username, new Packet(bytes), null);
    if (loaded.x !== preset.x || loaded.z !== preset.z || loaded.level !== preset.level) {
        throw new Error(
            `position roundtrip mismatch: got (${loaded.x},${loaded.z},${loaded.level}) want (${preset.x},${preset.z},${preset.level})`
        );
    }
    for (const s of preset.stats) {
        const stat = PlayerStatMap.get(s.skill.toUpperCase())!;
        const base = loaded.baseLevels[stat];
        if (base < s.level) {
            throw new Error(`stat ${s.skill} roundtrip ${base} < required ${s.level}`);
        }
        // XP/level consistency through server helpers
        if (getLevelByExp(loaded.stats[stat]) !== loaded.baseLevels[stat]) {
            throw new Error(`stat ${s.skill} xp/baseLevel inconsistency after load`);
        }
    }
    const tutorialId = VarPlayerType.getId('tutorial');
    if (loaded.vars[tutorialId] !== preset.tutorial) {
        throw new Error(`tutorial varp roundtrip ${loaded.vars[tutorialId]} != ${preset.tutorial}`);
    }
    for (const it of preset.items) {
        const objId = ObjType.getId(it.name.toLowerCase());
        const invId = (it.inv ?? 'inv') === 'inv' ? InvType.INV : InvType.getId('bank');
        const inv = loaded.getInventory(invId);
        if (!inv) throw new Error(`missing inventory ${it.inv ?? 'inv'} after load`);
        let total = 0;
        for (let slot = 0; slot < inv.capacity; slot++) {
            const obj = inv.get(slot);
            if (obj && obj.id === objId) total += obj.count;
        }
        if (total < it.count) {
            throw new Error(`item ${it.name} roundtrip count ${total} < ${it.count}`);
        }
    }
    // staffmodlevel is NOT in the save — production LoginThread assigns 0 offline.
    return loaded;
}

function engineProvenance(serverRoot: string): Record<string, string> {
    const out: Record<string, string> = {
        server_root: path.resolve(serverRoot),
    };
    try {
        const head = path.join(serverRoot, '.git', 'HEAD');
        if (fs.existsSync(head)) {
            const raw = fs.readFileSync(head, 'utf8').trim();
            if (raw.startsWith('ref:')) {
                const ref = raw.slice(4).trim();
                const refPath = path.join(serverRoot, '.git', ref);
                if (fs.existsSync(refPath)) {
                    out.engine_git_head = fs.readFileSync(refPath, 'utf8').trim();
                } else {
                    out.engine_git_head = raw;
                }
            } else {
                out.engine_git_head = raw;
            }
        }
    } catch {
        /* optional */
    }
    // Prefer parent Server repo HEAD when engine is a subfolder without its own git.
    try {
        const parent = path.resolve(serverRoot, '..');
        const head = path.join(parent, '.git', 'HEAD');
        if (!out.engine_git_head && fs.existsSync(head)) {
            const raw = fs.readFileSync(head, 'utf8').trim();
            if (raw.startsWith('ref:')) {
                const ref = raw.slice(4).trim();
                const refPath = path.join(parent, '.git', ref);
                if (fs.existsSync(refPath)) {
                    out.engine_git_head = fs.readFileSync(refPath, 'utf8').trim();
                }
            } else {
                out.engine_git_head = raw;
            }
        }
    } catch {
        /* optional */
    }
    out.pack_dir = path.join(path.resolve(serverRoot), 'data', 'pack');
    return out;
}

function main() {
    const args = parseArgs(process.argv.slice(2));
    const preset = PRESETS[args.fixture];
    if (!preset) {
        usage(`unknown --fixture ${args.fixture} (known: ${Object.keys(PRESETS).join(', ')})`);
    }

    const serverRoot = path.resolve(args.serverRoot);
    const packDir = path.join(serverRoot, 'data', 'pack');
    process.chdir(serverRoot);

    if (fs.existsSync(args.output) && !args.overwrite) {
        console.error(`refusing to overwrite existing save without --overwrite: ${args.output}`);
        process.exit(1);
    }
    // Refuse accidental writes into a non-owned path only when parent looks like
    // a real multi-account tree without an explicit isolated marker — still require
    // --output always; caller owns isolation.
    const outDir = path.dirname(args.output);
    fs.mkdirSync(outDir, { recursive: true });

    loadPacks(packDir);

    const name37 = toBase37(args.username);
    const safeName = fromBase37(name37);
    if (safeName !== args.username.toLowerCase() && safeName !== args.username) {
        // fromBase37 normalizes; keep engine-safe form for the file/login name.
        console.error(`note: engine safe username is ${safeName} (from ${args.username})`);
    }

    const player = new Player(safeName, name37, name37);
    applyPreset(player, preset);

    const bytes = player.save();
    if (!(bytes instanceof Uint8Array) || bytes.length < 8) {
        throw new Error('Player.save() returned empty buffer');
    }
    assertRoundtrip(safeName, bytes, preset);

    fs.writeFileSync(args.output, bytes);

    const digest = sha256(bytes);
    const receipt = {
        version: 1,
        kind: 'offline_player_save_fixture',
        fixture: preset.id,
        profile: args.profile,
        username: safeName,
        // Password is NOT invented here — login identity is owned by the panel
        // vault mint / fixture identity receipt. This receipt is save provenance only.
        sav_path: args.output,
        sav_sha256: digest,
        sav_bytes: bytes.length,
        sav_magic: PlayerLoading.SAV_MAGIC,
        sav_version: PlayerLoading.SAV_VERSION,
        position: { x: preset.x, z: preset.z, level: preset.level },
        tutorial: preset.tutorial,
        stats: preset.stats,
        items: preset.items.map((it) => ({
            name: it.name,
            count: it.count,
            inv: it.inv ?? 'inv',
            obj_id: ObjType.getId(it.name.toLowerCase()),
        })),
        // Explicit: staff is not serialized; production offline login assigns 0.
        staffmodlevel_in_save: false,
        prepared_at_unix_ms: Date.now(),
        provenance: engineProvenance(serverRoot),
        helper: path.relative(process.cwd(), fileURLToPath(import.meta.url)) || 'write_player_fixture.ts',
    };

    const receiptPath =
        args.receipt ??
        args.output.replace(/\.sav$/i, '') + '.receipt.json';
    fs.mkdirSync(path.dirname(receiptPath), { recursive: true });
    fs.writeFileSync(receiptPath, JSON.stringify(receipt, null, 2) + '\n');

    console.log(
        JSON.stringify(
            {
                ok: true,
                username: safeName,
                sav_path: args.output,
                sav_sha256: digest,
                receipt: receiptPath,
                fixture: preset.id,
            },
            null,
            2
        )
    );
}

main();
