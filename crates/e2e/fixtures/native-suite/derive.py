#!/usr/bin/env python3
"""Derive `suite-manifest.json` for the native e2e suite from the frozen reference.

This script is a one-off authoring tool, not part of the runtime: the suite reads the
derived JSON (embedded at compile time, or passed with `--manifest`). It never runs
foreign JavaScript; it only reads the frozen 96410ec5 reference sources textually.

    python3 derive.py --reference-root <read-only 96410ec5 checkout> --out suite-manifest.json

Inputs (frozen reference, read-only):
    e2e/manifest.ts         case list, statuses, budgets, args/env, coverage
    e2e/manifestTypes.ts    case schema
    e2e/manifestQuery.ts    level/smart/--only selection semantics
    (the native mapping itself is the approved root design, docs/compat/p3-runner-design.md)

The emitted manifest is compact, portable (no absolute local paths) and carries the
reference identity (commit/tree/archive hash) as provenance. Reference statuses are
historical evidence, never a native PASS.
"""

import argparse
import hashlib
import json
import pathlib
import re
import sys
from collections import Counter, OrderedDict

# --------------------------------------------------------------------------------------
# Approved native mapping (docs/compat/p3-runner-design.md "Intended 964 enabled
# inventory and mapping"). Only live scenarios that exist in `scenario::names()` are
# declared runnable; everything else is an explicit unavailable row with a reason.
# --------------------------------------------------------------------------------------

CORE = "core"
PAIR = "pair"


def case(script, live, *, runner=CORE, reference=(), variants=(), options=(),
         unsupported=(), budget=None, unavailable=None, note=None):
    return {
        "script": script,
        "live": live,
        "runner": runner,
        "reference": list(reference),
        "variants": list(variants),
        "options": list(options),
        "unsupported": list(unsupported),
        "budget": budget,
        "unavailable": unavailable,
        "note": note,
    }


MAPPING = [
    case("AIO Teleport", "aio_teleport",
         reference=["relogin-test"],
         variants=["aio_teleport_falador", "aio_teleport_no_staff"],
         options=["teleportName/progressive", "lawBatchSize/minLawRunes/useStaffRunes"],
         unsupported=[("login/relogin observation",
                       "reference case asserts a relogin round trip; the native scenario covers the teleport legs only")]),
    case("ChickenKiller", "chicken_killer", reference=["hosted-proof-test"],
         variants=["chicken_killer_bank"],
         options=["seed chicken/feather", "food/bank state"],
         unsupported=[("hosted/manual prerequisite",
                       "reference case is documented/manual (hosted wall); native run is a script witness only")]),
    case("Duel Arena Combat Trainer", "duel_arena", runner=PAIR,
         options=["two actual actors", "native counterpart identity", "target Attack/Strength/Defence"],
         unsupported=[("reference case", "frozen e2e/manifest.ts has no Duel entry; native FirstCombat/ResetAndFurther is declared native coverage, not a frozen reference PASS"),
                      ("LIVE qualification", "the headed adapter is runnable and unvetted until actual LIVE")]),
    case("ChaosDruidKiller", "chaos_druid", variants=["chaos_druid_tower", "chaos_druid_yanille", "chaos_druid_bank"],
         options=["location/drop/loot"],
         unsupported=[("selected herb/law/nature pickup", "combat fixture for selected pickups is still open")]),
    case("RockCrab", "rock_crab", reference=["rockcrab-dart-test"],
         variants=["rock_crab_range", "rock_crab_bank"],
         note="Native stand (2712,3707,0) is inside the source spawn visibility window and outside wake range; "
              "the live baseline must still prove dormant Rocks and subsequent script activation. No LIVE qualification is implied."),
    case("MossGiant", "moss_giant", reference=["mossgiant-dart-test"], variants=["moss_giant_bank"],
         options=["dart/location/bank"],
         unsupported=[("fixture prerequisites", "dart/location/bank fixture reconciliation remains open")]),
    case("GreenDragon", "green_dragon", reference=["greendragon-pk-flee-test", "greendragon-test"],
         variants=["green_dragon_special", "green_dragon_potions", "green_dragon_bank", "green_dragon_tele"],
         options=["combat style/special/potions", "flee/teleport"],
         unsupported=[("actual Strength-XP/return evidence",
                       "reference asks for XP/return evidence; older failures are preserved, not re-proved here")]),
    case("FireGiant", "fire_giant", reference=["firegiant-test"],
         variants=["fire_giant_approach", "fire_giant_bank"],
         options=["approach/bank"],
         unsupported=[("reader.npcBox paint/readback",
                       "native paint/readback for the FireGiant witness needs reader.npcBox projection or a "
                       "declared capture substitute; capture expectation is pending adapter work")]),
    case("ArdyFighter", "ardy_fighter", reference=["ardyfighter-restock-loop-live"], budget=5,
         variants=["ardy_fighter_bank"], options=["restock/food", "explicit combat style"]),
    case("Thiever", "thiever", options=["target/food/bank"]),
    case("AutoFighter", "auto_fighter",
         reference=["autofighter-targets-loot-live", "autofighter-special-live"],
         variants=[
             {"live": "auto_fighter_mage"},
             {"live": "auto_fighter_range"},
             {"live": "auto_fighter_bank",
              "reference": ["autofighter-bank-resume-live", "common-reward-banking-test"],
              "options": ["reward banking"]},
         ],
         options=["combat style/special", "targets/loot"],
         unsupported=[("panel CSV capture/readback",
                       "loot-csv-panel-live is a panel capture/readback case; the native panel capture adapter is "
                       "separate adapter work and is not claimed here")]),
    case("ArdyThiever", "ardy_thiever", variants=["ardy_thiever_fight", "ardy_thiever_knight"],
         options=["target/knight/fight", "bank"], unsupported=[("per-revision failure cells", "retained; not re-proved")]),
    case("ArdyCakes", "ardy_cakes", variants=["ardy_cakes_fight"],
         unsupported=[("production/guard-response fixture",
                       "fixture prerequisite; the case stays visible rather than dimmed or excluded")]),
    case("GnomeMagicChopper", "gnome_chop", variants=["gnome_fletch_short", "gnome_fletch_long"],
         options=["tree/axe/fletch/burn"],
         unsupported=[("actual-tree start and required tools", "verify actual tree start and tools before any PASS claim")]),
    case("CoalTrucks", "coal_trucks", reference=["coaltrucks-test"],
         unsupported=[("combat level 55 / coal-truck route prerequisite", "reference prerequisite; branch accounting open")]),
    case("RuneCrafter", "rune_crafter", reference=["runecrafter-multibox-test"],
         variants=["rune_crafter_earth"], options=["rune type/altar/bank"],
         unsupported=[("multibox identities", "reference case is a multibox run; native case is a single-act actor witness")]),
    case("NatureCrafter", "nature_crafter_air", runner=PAIR,
         reference=["nature-runner-coins-739-live", "naturecrafter-soak-test"], budget=12,
         note="outer budget is the max of the mapped reference budgets; the 3-minute coin case budget and the "
              "12-minute soak budget stay recorded in reference_cases, and native inner deadlines are unchanged",
         options=["two actual actors", "partner names/trade bootstrap"],
         unsupported=[("coin drop/take witness", "adaptation carries its own declared support evidence, not upstream coin739 proof"),
                      ("12-minute soak", "the soak case is not claimed by the pair adaptation")]),
    case("CookBot", "cook_bot", variants=["cook_bot_lobster"], options=["fish", "bank/return"],
         unsupported=[("exact food/XP/deposit/further-work prerequisite", "fixture prerequisite")]),
    case("BankFletcher", "bank_fletcher", reference=["bankfletcher-live"],
         variants=["bank_fletcher_shafts", "bank_fletcher_headless", "bank_fletcher_string",
                   "bank_fletcher_cut_string"],
         options=["product/knife/string/headless branches"]),
    case("DartFletcher", "dart_fletcher", reference=["dartfletcher-test"], variants=["dart_fletcher_iron"],
         options=["bronze/iron", "selected dart/material"], unsupported=[("seed exact bars/feathers", "fixture prerequisite")]),
    case("HerbloreSecondaries", "herblore_secondaries", reference=["herblore-secondaries-test"],
         variants=["herblore_secondaries_newt"], options=["secondary/egg/newt branches"],
         unsupported=[("pre-Start bank readiness", "fixture prerequisite"),
                      ("exact XP/product witness", "verify before any PASS claim")]),
    case("HerbCleaner", "herb_cleaner", reference=["herbcleaner-empty-bank-live"], budget=8,
         variants=["herb_cleaner_named"], options=["default/named herb filters", "empty-bank stop"]),
    case("PotionMaker", "potion_maker", variants=["potion_maker_named"], options=["recipe/default/named"],
         unsupported=[("seed unfinished/finished products", "fixture prerequisite")]),
    case("BoneBurier", "bone_burier", reference=["external-script-test"],
         unsupported=[("external loader smoke contract",
                       "the reference loader smoke (register once, auto-select without auto-start, 10+ burials in "
                       "180000 ms, Stop within 10000 ms, no duplicate registration) is a separate contract; this "
                       "native case is the catalog core witness only")]),
    case("SmelterBot", "smelter_bot", reference=["smelter-swarm-422-live"], variants=["smelter_bot_steel"],
         options=["bar/ore", "swarm interruption"], unsupported=[("seed exact ores and bank-return state", "fixture prerequisite")]),
    case("Superheater", "superheater", reference=["superheater-smelt-live"], budget=10,
         variants=[
             {"live": "superheater_steel"},
             {"live": "superheater_fire_battlestaff", "reference": ["superheater-fire-battlestaff-live"],
              "budget": 10, "options": ["staff selection"],
              "unsupported": [("Attack 30 fixture",
                               "the fire-battlestaff cell needs the Attack 30 fixture, not the historical Attack 1 seed")]},
         ],
         options=["bar selection"]),
    case("Alcher", "alcher_defaults",
         reference=["alcher-nearest-bank-live", "alcher-low-744-live", "alcher-fire-battlestaff-live",
                    "alcher-swarm-drain-live"],
         budget=12,
         variants=["alcher", "alcher_custom", "alcher_custom_alias", "alcher_custom_name", "alcher_ordered",
                   "alcher_large_batch"],
         options=["spell=High default", "spell=Low", "item alias/name/ordered/batch", "staff/swarm branches"],
         unsupported=[("full-cycle assertion for loader smoke", "no full-cycle assertion is claimed for the loader smoke")]),
    case("SmithingBot", "smithing_bot", reference=["smithingbot-bank-loop-live"], budget=10,
         variants=["smithing_bot_platebody"], options=["Adamantite/Runite (frozen 964 options)"],
         unsupported=[("persisted Adamant/Rune override migration",
                       "persisted old overrides must be migrated before selection; migration is pending")]),
    case("FlaxPicker", "flax_picker", options=["location/quantity", "bank-return"],
         unsupported=[("exact flax/XP witness", "fixture prerequisite")]),
    case("FlaxSpinner", "flax_spinner", options=["partner/spin/return settings if native support is added"],
         unsupported=[("reference case", "no reference case covers FlaxSpinner; nothing is treated as qualification")]),
    case("FlaxAIO", "flax_aio", reference=["flaxaio-pick-spin-live"], budget=18,
         variants=["flax_aio_pick", "flax_aio_spin"],
         unsupported=[("spin cell", "the spin cell is known broken and must not be silently promoted")]),
    case("GemCutter", "gem_cutter", variants=["gem_cutter_named"], options=["gem/default/named"],
         unsupported=[("seed uncuts and exact XP/bank return", "fixture prerequisite")]),
    case("GnomeCourse", "gnome_course", variants=["gnome_course_radius"], options=["course/radius"],
         unsupported=[("start tile/obstacle fixture", "fixture prerequisite")]),
    case("WildyAgility", "wildy_agility", reference=["wildyagility-food-startup-live"], budget=6,
         options=["named food startup", "banked cake/chocolate-cake branch"]),
    case("MuleCrafter", "mule_crafter_air", runner=PAIR, reference=["mulecrafter-test"],
         options=["two actual actors", "role-specific partner names", "trade/crafter fixture", "mode/material options"]),
    case("ClimbingBoots", "climbing_boots", variants=["climbing_boots_teleport"],
         options=["useTeleport false walk cell / true teleport cell",
                  "runeStock 1 explicit fixture value (product default 50)"],
         unsupported=[("Death Plateau completion/Tenzing NPC/dialogue prerequisite", "fixture prerequisite"),
                      ("custom Sherpa paint interaction", "not claimed by the native witness")]),
    case("ShopBuyout", "shop_buyout", variants=["shop_buyout_aubury"], options=["shop selection/Aubury branch"],
         unsupported=[("stock/coin prerequisite", "fixture prerequisite"),
                      ("actual buy/XP/inventory witness", "verify before any PASS claim")]),
    case("DoorOpener", "door_opener", variants=["door_opener_gate"], options=["door/gate branch"],
         unsupported=[("world fixture", "success is an actual open/route transition, not paint")]),
    case("TannerBot", "tanner_bot", variants=["tanner_bot_hard"], options=["leather/hard-leather branch"],
         unsupported=[("hides/coins/bank prerequisite", "fixture prerequisite")]),
    case("HillGiant", "hill_giant", reference=["hillgiant-test"],
         variants=[{"live": "hill_giant_bank", "reference": ["hillgiant-bank-428-live"]}],
         options=["combat/drop/bank"], unsupported=[("kill and return proof", "the fixture must prove kill and return")]),
    case("VialFiller", "vial_filler", reference=["vialfiller-test"], variants=["vial_filler_east"],
         options=["bank/source branch"], unsupported=[("seed empty vials/water", "verify filled inventory/XP")]),
    case("LeatherCrafter", "leather_crafter", reference=["leathercrafter-nearest-bank-live"], budget=8,
         variants=["leather_crafter_hard_body"], options=["leather/hard-body branch", "nearest-bank constraint"]),
    case("Firemaker", "firemaker", variants=["firemaker_oak"], options=["log/fire-spot", "oak branch"],
         unsupported=[("seed tinderbox/logs", "verify fire placement and light progression")]),
    case("FlaxRunner", "flax_runner", runner=PAIR,
         options=["two actual actors", "Runner/Spinner roles", "partner", "minFlaxCapacity"],
         unsupported=[("reference case", "no frozen reference case covers FlaxRunner; the native pair witness is "
                                         "the adaptation and nothing is relabelled as a reference PASS")]),
]

# Reference cases that are reachable only under the frozen reference contract. Their
# scripts are outside the intended native set; retained visibly with a reason.
EXCLUDED_SCRIPTS = {
    "AIOQuester": ("excluded_script", "quests are not imported into the native runner; the reference case documents "
                                      "coverage and prerequisites only"),
    "GatheringBot": ("excluded_script", "gatherer/Fisher planner is not imported into the native runner"),
    "ClueSolver": ("excluded_script", "clue solver is not imported into the native runner"),
    "MarketMaker": ("excluded_script", "market-maker planner is not imported into the native runner"),
    "WalkToBot": ("excluded_script", "native navigation; it stays outside the foreign catalog rows"),
}
BANK_SORTER = ("operator_decision", "UNAVAILABLE_BY_OPERATOR_DECISION: no native sorter; cases and history are "
                                    "retained, nothing is replaced or silently removed")

# Manual reference cases never selected by a level (reference semantics, retained).
MANUAL_NOTE = "manual in the reference manifest; reachable only through --only"

DEFAULT_BUDGET_MIN = 12

# Display name (as the approved design table names it) -> registry script name used by
# the frozen manifest's `covers.scripts`. Most cards share the name; these differ.
SCRIPT_KEY_BY_DISPLAY = {
    "AIO Teleport": "AIOTeleport",
    "Duel Arena Combat Trainer": "DuelArena",
    "Thiever": "ThievingBot",
    "GnomeCourse": "AgilityBot",
}


def parse_script_names(path):
    src = path.read_text()
    block = src[src.index("export const SCRIPT_NAMES"):]
    block = block[:block.index("] as const;")]
    return set(re.findall(r"'([A-Za-z0-9_]+)'", block))


def split_case_blocks(src):
    """Line-based scan for top-level `    { ... },` entries (regex splitting is brittle)."""
    lines = src.splitlines()
    blocks = []
    current = None
    for line in lines:
        if current is None:
            if line == "    {":
                current = []
            continue
        if line in ("    },", "    }"):
            blocks.append("\n".join(current))
            current = None
            continue
        current.append(line)
    if current is not None:
        raise SystemExit("unterminated case block in the frozen manifest")
    return blocks


def parse_reference_manifest(path):
    src = path.read_text()
    body = src[src.index('export const CASES'):]
    entries = split_case_blocks(body)
    cases = []
    for block in entries:
        get = lambda pat: (re.search(pat, block, re.S).group(1) if re.search(pat, block, re.S) else None)
        entry = {
            "id": get(r"id:\s*'([^']*)'"),
            "harness": get(r"harness:\s*'([^']*)'"),
            "status": get(r"status:\s*'([^']*)'"),
            "manual": bool(re.search(r"manual:\s*true", block)),
        }
        budget = get(r"budgetMin:\s*(\d+)")
        if budget:
            entry["budget_min"] = int(budget)
        for key, pat in (("proven_at", r"provenAt:\s*'([^']*)'"), ("documented_in", r"documentedIn:\s*'([^']*)'")):
            value = get(pat)
            if value:
                entry[key] = value
        cov = re.search(r"covers:\s*\{(.*?)\}\s*,?\s*\n", block, re.S)
        scripts, subsystems = [], []
        if cov:
            sm = re.search(r"scripts:\s*\[(.*?)\]", cov.group(1), re.S)
            if sm:
                scripts = re.findall(r"'([^']*)'", sm.group(1))
            ssm = re.search(r"subsystems:\s*\[(.*?)\]", cov.group(1), re.S)
            if ssm:
                subsystems = re.findall(r"'([^']*)'", ssm.group(1))
        entry["scripts"] = scripts
        entry["subsystems"] = subsystems
        am = re.search(r"args:\s*\[(.*?)\]", block, re.S)
        if am:
            entry["args"] = re.findall(r"'([^']*)'", am.group(1))
        em = re.search(r"env:\s*\{(.*?)\}", block, re.S)
        if em:
            entry["env"] = {k: v for k, v in re.findall(r"'?([A-Za-z0-9_]+)'?:\s*'([^']*)'", em.group(1))}
        nm = re.search(r"note:\s*'((?:[^'\\]|\\.)*)'", block)
        if nm:
            entry["note"] = nm.group(1).replace("\\'", "'")[:240]
        cases.append(entry)
    ids = [c["id"] for c in cases]
    if len(set(ids)) != len(ids):
        raise SystemExit("duplicate reference case ids")
    if not cases:
        raise SystemExit("no reference cases parsed")
    return cases


def _parse_fn_block(path, anchor):
    """The body of the first `pub fn parse` after `anchor`, up to that function's close."""
    src = path.read_text()
    start = src.index(anchor)
    start = src.index("pub fn parse", start)
    end = src.index("\n    }\n", start)
    return src[start:end]


def parse_core_cases(path):
    """`CoreCase::parse` name -> variant, the scenarios that carry a core witness."""
    block = _parse_fn_block(path, "impl CoreCase {")
    return OrderedDict(re.findall(r'"([a-z0-9_]+)"\s*=>\s*Ok\(Self::(\w+)\)', block))


def parse_pair_cases(path):
    """Canonical headed pair live name -> witness identity (`Air`/`Mule`/`Flax`/`Duel`)."""
    block = _parse_fn_block(path, "impl PairCase {")
    canonical = {}
    for name, variant in re.findall(
            r'"([a-z0-9_]+)"(?:\s*\|\s*"[a-z0-9_]+")?\s*=>\s*Ok\(Self::(\w+)\)', block):
        canonical.setdefault(variant, name)
    return OrderedDict((name, variant) for variant, name in canonical.items())


def wire_case(variant):
    """The serde wire form of a host enum variant (`rename_all = "snake_case"`).

    The panel's CATALOG_CORE/PAIRED_CORE receipt carries this, not the variant name, so
    the manifest must declare it or the suite would reject real native output.
    """
    if variant is None:
        return None
    out = []
    for i, ch in enumerate(variant):
        if ch.isupper():
            prev = variant[i - 1] if i else ""
            nxt = variant[i + 1] if i + 1 < len(variant) else ""
            if i and (prev.islower() or prev.isdigit() or (prev.isupper() and nxt.islower())):
                out.append("_")
            out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)


def scenario_names(path):
    src = path.read_text()
    block = src[src.index("pub fn names()"):]
    block = block[:block.index("]\n}")]
    return re.findall(r'"([a-z0-9_]+)"', block)


def external_constants(path):
    """The producer's own external loader constants.

    The dedicated loader smoke is not a scenario catalog row: its live name, live scenario
    token and terminal-shot label come from `host_play::external_loader`, exactly as a catalog
    row's live name comes from `scenario::names()`. The frozen fixture digest is read from the
    same module *and* checked against the tracked file, so a manifest can never declare a source
    the producer would refuse.
    """
    src = path.read_text()
    def constant(name):
        match = re.search(rf'pub const {name}: &str = "([^"]*)"', src)
        if match is None:
            raise SystemExit(f"external loader constant {name} not found in {path}")
        return match.group(1)
    return OrderedDict(
        (name, constant(name)) for name in
        ("LIVE_NAME", "LIVE_SCENARIO", "RECEIPT_PREFIX", "TERMINAL_SHOT", "PREREQ_SHOT", "FROZEN_SHA256")
    )


def covered_scripts(reference_cases, script):
    return [c["id"] for c in reference_cases if script in c["scripts"]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--reference-root", required=True, type=pathlib.Path)
    ap.add_argument("--workspace-root", required=True, type=pathlib.Path)
    ap.add_argument("--out", required=True, type=pathlib.Path)
    ns = ap.parse_args()

    reference_cases = parse_reference_manifest(ns.reference_root / "e2e/manifest.ts")
    by_id = {c["id"]: c for c in reference_cases}
    core_by_live = parse_core_cases(ns.workspace_root / "crates/host-play/src/catalog_core.rs")
    pair_by_live = parse_pair_cases(ns.workspace_root / "crates/host-play/src/paired_core.rs")
    live_names = scenario_names(ns.workspace_root / "crates/scenario/src/lib.rs")
    script_names = parse_script_names(ns.reference_root / "e2e/manifestTypes.ts")

    intended = []
    rows = []
    mapped_reference = set()

    def reference_rows(ids):
        out = []
        for rid in ids:
            c = by_id.get(rid)
            if c is None:
                raise SystemExit(f"mapped reference case not in the frozen manifest: {rid}")
            mapped_reference.add(rid)
            out.append({
                "id": rid,
                "status": c["status"],
                "manual": c["manual"],
                "budget_min": c.get("budget_min", DEFAULT_BUDGET_MIN),
            })
        return out

    def add_row(script, script_key, live, *, runner, refs, options, unsupported, budget, unavailable, primary,
                variants=(), note=None):
        # An unavailable row may name a case that has no live scenario yet (a declared gap);
        # a runnable row must resolve to a real scenario with the declared witness.
        exists = live in live_names
        if unavailable:
            core_case = core_by_live.get(live) if exists and runner != PAIR else None
            pair_case = pair_by_live.get(live) if exists and runner == PAIR else None
        elif runner == PAIR:
            if live not in pair_by_live:
                raise SystemExit(f"{live}: not a headed pair cell")
            pair_case = pair_by_live[live]
            core_case = None
        else:
            if live not in core_by_live:
                raise SystemExit(f"{live}: no core witness (CoreCase::parse) for this scenario")
            core_case = core_by_live[live]
            pair_case = None
        statuses = [r["status"] for r in refs]
        if "broken" in statuses:
            status = "broken"
        elif "vetted" in statuses:
            status = "vetted"
        elif "documented" in statuses:
            status = "documented"
        else:
            status = "unvetted"
        manual = bool(refs) and all(r["manual"] for r in refs)
        if budget is None:
            budget = max([r["budget_min"] for r in refs], default=DEFAULT_BUDGET_MIN)
        row = OrderedDict([
            ("id", live),
            ("kind", "native"),
            ("script", script),
            ("script_key", script_key),
            ("primary", primary),
            ("status", status),
            ("manual", manual),
            ("budget_min", budget),
            ("runner", runner),
            ("live", f"script_{live}"),
            ("scenario", live),
            # The witness identity is what the panel actually prints: the host enum's
            # serde wire form (`CoreCase::Thiever` -> "thiever"), never the Rust variant.
            ("core_case", wire_case(core_case)),
            ("pair_case", wire_case(pair_case)),
            ("options", list(options)),
            ("unsupported", [{"option": o, "reason": r} for o, r in unsupported]),
            ("covers", {"scripts": [script_key] if script_key else [], "subsystems": [], "paths": []}),
            ("variants", list(variants)),
            ("reference_cases", refs),
        ])
        if note:
            row["budget_note"] = note
        if unavailable:
            row["unavailable"] = {"code": unavailable[0], "reason": unavailable[1]}
        rows.append(row)

    for spec in MAPPING:
        script = spec["script"]
        script_key = SCRIPT_KEY_BY_DISPLAY.get(script, script)
        if script_key not in script_names:
            raise SystemExit(f"{script}: {script_key} is not a frozen registry script name")
        if script not in intended:
            intended.append(script)
        live = spec["live"]
        unavailable = spec["unavailable"]
        refs = reference_rows(spec["reference"]) if spec["reference"] else []
        if unavailable:
            # An unavailable row keeps its reference metadata but is never executed; its
            # variants are separate observable cases and stay visible the same way.
            add_row(script, script_key, live, runner=spec["runner"], refs=refs, options=spec["options"],
                    unsupported=spec["unsupported"], budget=spec["budget"], unavailable=unavailable, primary=True)
            for variant in spec["variants"]:
                vname = variant["live"] if isinstance(variant, dict) else variant
                options = variant.get("options", spec["options"]) if isinstance(variant, dict) else spec["options"]
                unsupported = (variant.get("unsupported", spec["unsupported"]) if isinstance(variant, dict)
                               else spec["unsupported"])
                add_row(script, script_key, vname, runner=spec["runner"], refs=[],
                        options=options, unsupported=unsupported, budget=spec["budget"], unavailable=unavailable,
                        primary=False)
        else:
            if script not in intended:
                intended.append(script)
            add_row(script, script_key, live, runner=spec["runner"], refs=refs, options=spec["options"],
                    unsupported=spec["unsupported"], budget=spec["budget"], unavailable=None, primary=True,
                    variants=[v["live"] if isinstance(v, dict) else v for v in spec["variants"]],
                    note=spec.get("note"))
            for variant in spec["variants"]:
                if isinstance(variant, dict):
                    vrefs = reference_rows(variant.get("reference", []))
                    add_row(script, script_key, variant["live"], runner=spec["runner"], refs=vrefs,
                            options=variant.get("options", spec["options"]),
                            unsupported=variant.get("unsupported", spec["unsupported"]),
                            budget=variant.get("budget", spec["budget"]), unavailable=None, primary=False)
                else:
                    add_row(script, script_key, variant, runner=spec["runner"], refs=[], options=spec["options"],
                            unsupported=spec["unsupported"], budget=spec["budget"], unavailable=None, primary=False)

    # The dedicated external raw TypeScript loader smoke. It is *not* a scenario catalog row: its
    # live name, live scenario and terminal-shot label come from `host_play::external_loader`, and
    # it adapts the frozen `external-script-test` loader smoke directly. The catalog `bone_burier`
    # row keeps its own core-witness contract; the historical association stays recorded there as
    # an unsupported gap *and* here as the row that actually runs the loader reference.
    external = external_constants(ns.workspace_root / "crates/host-play/src/external_loader.rs")
    fixture = ns.workspace_root / "crates/host-play/fixtures/ExampleBot.ts"
    if hashlib.sha256(fixture.read_bytes()).hexdigest() != external["FROZEN_SHA256"]:
        raise SystemExit(f"the tracked external fixture {fixture} does not match FROZEN_SHA256")
    if external["LIVE_SCENARIO"] in live_names:
        raise SystemExit(f"{external['LIVE_SCENARIO']}: the loader smoke must not be a catalog scenario")
    if any(row["id"] == external["LIVE_SCENARIO"] for row in rows):
        raise SystemExit(f"duplicate case id: {external['LIVE_SCENARIO']}")
    loader_refs = reference_rows(["external-script-test"])
    loader_status = "unvetted"
    for status in ("broken", "vetted", "documented"):
        if any(ref["status"] == status for ref in loader_refs):
            loader_status = status
            break
    loader_row = OrderedDict([
        ("id", external["LIVE_SCENARIO"]),
        ("kind", "native"),
        # Not a catalog script case: the smoke loads a raw `.ts` fixture, so it declares no
        # `script`/`script_key` and `--only BoneBurier` keeps selecting the catalog row only.
        ("script", None),
        ("script_key", None),
        ("primary", False),
        # The frozen reference case is `manual` (hosted/manual upstream); this native adapter is
        # automated, so the row is selectable rather than hidden behind a manual flag.
        ("status", loader_status),
        ("manual", False),
        ("budget_min", loader_refs[0]["budget_min"] if loader_refs else DEFAULT_BUDGET_MIN),
        ("runner", "external"),
        ("live", external["LIVE_NAME"]),
        ("scenario", external["LIVE_SCENARIO"]),
        ("core_case", None),
        ("pair_case", None),
        ("options", ["raw TypeScript fixture (default: crates/host-play/fixtures/ExampleBot.ts)",
                     "typed --external-ts absolute override"]),
        ("unsupported", []),
        ("covers", {"scripts": [], "subsystems": [], "paths": [
            "crates/host-play/src/external_loader.rs",
            "crates/host-play/fixtures/ExampleBot.ts",
            "crates/panel/examples/external_watch.rs",
        ]}),
        ("capture", {"label": external["TERMINAL_SHOT"], "required": True}),
        ("variants", []),
        ("reference_cases", loader_refs),
        ("note", f"dedicated {external['RECEIPT_PREFIX']} loader smoke: register once, select without "
                 f"auto-start, observe the burials/Prayer/inventory gate inside the producer's own "
                 f"deadlines, Stop, reload identity and one {external['TERMINAL_SHOT']!r} capture. The "
                 f"prerequisite {external['PREREQ_SHOT']!r} capture is a different shot and never "
                 f"satisfies it; visual approval stays a human readback. The catalog `bone_burier` "
                 f"core witness is unchanged and does not carry this contract."),
    ])
    anchor = next((index for index, row in enumerate(rows) if row["id"] == "bone_burier"), len(rows) - 1)
    rows.insert(anchor + 1, loader_row)

    # Native cases with no primary row yet (ClimbingBoots has none).
    for spec in MAPPING:
        if spec["script"] not in intended and not spec["unavailable"]:
            raise SystemExit(f"{spec['script']}: neither intended nor unavailable")

    # Every enabled script must be represented exactly once as a primary row.
    primaries = [r["script"] for r in rows if r.get("primary") and r["kind"] == "native"]
    duplicates = [s for s, n in Counter(primaries).items() if n > 1]
    if duplicates:
        raise SystemExit(f"duplicate primary rows: {duplicates}")
    missing = [s for s in intended if s not in primaries]
    if missing:
        raise SystemExit(f"intended scripts without a primary row: {missing}")

    # The frozen loader reference must reach the adapter that actually runs it: the dedicated
    # row carries the direct association (the catalog `bone_burier` row keeps its own contract
    # and the historical note, and is not the smoke's adapter).
    loader_reached = [
        row["id"] for row in rows
        if row["kind"] == "native"
        and any(ref["id"] == "external-script-test" for ref in row.get("reference_cases", []))
    ]
    if external["LIVE_SCENARIO"] not in loader_reached:
        raise SystemExit("the external loader row does not carry the external-script-test reference")

    # Remaining reference cases stay visible with an explicit reason.
    for c in reference_cases:
        if c["id"] in mapped_reference:
            continue
        unavailable = None
        for script in c["scripts"]:
            if script in EXCLUDED_SCRIPTS:
                unavailable = EXCLUDED_SCRIPTS[script]
                break
            if script in ("BankSorter",):
                unavailable = BANK_SORTER
                break
        if unavailable is None:
            if c["manual"]:
                unavailable = ("unmapped_reference", MANUAL_NOTE)
            else:
                unavailable = ("unmapped_reference",
                               "reference case covers an intended script but has no 1:1 native adapter declared in "
                               "the approved mapping; the script runs natively as its primary case")
        row = OrderedDict([
            ("id", c["id"]),
            ("kind", "reference"),
            ("script", c["scripts"][0] if c["scripts"] else None),
            ("status", c["status"]),
            ("manual", c["manual"]),
            ("budget_min", c.get("budget_min", DEFAULT_BUDGET_MIN)),
            ("harness", c["harness"]),
            ("covers", {"scripts": c["scripts"], "subsystems": c["subsystems"], "paths": []}),
            ("unavailable", {"code": unavailable[0], "reason": unavailable[1]}),
        ])
        for key in ("args", "env"):
            if key in c:
                row[f"reference_{key}"] = c[key]
        if "proven_at" in c:
            row["proven_at"] = c["proven_at"]
        if "documented_in" in c:
            row["documented_in"] = c["documented_in"]
        if "note" in c:
            row["note"] = c["note"]
        rows.append(row)

    manifest = OrderedDict([
        ("schema_version", 1),
        ("suite_id", "native-suite-96410ec5"),
        ("provenance", OrderedDict([
            ("reference_commit", "96410ec5c779f3d8fe537268cae1a21c0174d16c"),
            ("reference_tree", "026b4f960a17c8c916cb5c87996b4b4c11b1d2e1"),
            ("reference_archive_sha256", "e7d37273b80e3eaf6075dec9c52e10a67b8f1ade5d0b8fb5df9a64f4537f1b5d"),
            ("reference_upstream", "https://github.com/rs2b2t/rs2b0t.git"),
            ("inputs", ["e2e/manifest.ts", "e2e/manifestTypes.ts", "e2e/manifestQuery.ts", "e2e/runner.ts",
                        "docs/reference/e2e-manifest.md"]),
            ("derived_by", "crates/e2e/fixtures/native-suite/derive.py"),
            ("reference_statuses_are_evidence", True),
        ])),
        ("defaults", OrderedDict([
            ("budget_min", DEFAULT_BUDGET_MIN),
            ("jobs", 1),
            ("exec", OrderedDict([
                ("core", OrderedDict([("program", "cargo"),
                                      ("args", ["run", "--quiet", "-p", "panel", "--example", "catalog_watch",
                                                "--"])])),
                ("pair", OrderedDict([("program", "cargo"),
                                      ("args", ["run", "--quiet", "-p", "panel", "--example", "pair_watch",
                                                "--"])])),
                # The dedicated loader smoke's own entrypoint: `catalog_watch` never produces an
                # EXTERNAL_LOADER receipt, so the row must not borrow the core template.
                ("external", OrderedDict([("program", "cargo"),
                                          ("args", ["run", "--quiet", "-p", "panel", "--example",
                                                    "external_watch", "--"])])),
            ])),
            # Desired run options and whether the current native adapters can select them.
            # An unsupported desire is declared pending adapter work, never claimed done.
            ("options", OrderedDict([
                ("headed", OrderedDict([
                    ("desired", "always"),
                    ("supported", True),
                    ("note", "`panel --live script_<name>` runs a visible window; the suite never asks for a "
                             "headless client"),
                ])),
                ("nav_paints", OrderedDict([
                    ("desired", "on"),
                    ("supported", True),
                    ("note", "--nav-paints on|off controls session-only diagnostic layers; suite headed runs default on. "
                             "Routing, teleport policy, deadlines and saved operator settings are unchanged."),
                ])),
                ("capture", OrderedDict([
                    ("desired", "native_case_contract"),
                    ("supported", True),
                    ("note", "native terminal capture contracts are validated against newly written PNG/JSON pairs; "
                             "paired cases require both actual actors and the external loader requires its terminal capture. "
                             "Successful captures remain pending visual review until readback."),
                    ("pending", "an operator CLI request for extra captures beyond each native case contract is not exposed"),
                ])),
                ("external_ts", OrderedDict([
                    ("desired", "typed absolute override"),
                    ("supported", True),
                    ("note", "the dedicated loader smoke row binds its raw source by path and content: the "
                             "tracked default fixture when `--external-ts ABS` is absent, the named absolute "
                             "file when it is given. The field name in the receipt is the loaded card's "
                             "origin cache key, which for a raw file is the raw source digest. A "
                             "core/pair-only selection never resolves that resource."),
                ])),
                ("memory", OrderedDict([
                    ("desired", "per_run_lowmem_or_highmem"),
                    ("supported", True),
                    ("note", "the suite passes exactly one typed --lowmem/--highmem panel flag to every core, pair, "
                             "and external child; absent panel selection preserves the vault profile and UI gate"),
                ])),
            ])),
        ])),
        ("smart", OrderedDict([
            ("shared_paths", ["crates/host-play/", "crates/api/", "crates/host/", "crates/vault/",
                              "crates/script/", "crates/scenario/", "vendor/fr-client-rust/", "Cargo.toml",
                              "Cargo.lock"]),
            ("subsystem_paths", OrderedDict([("crates/nav/", "nav"), ("crates/panel/", "panel"),
                                             ("crates/tui/", "panel")])),
            ("ignored_paths", ["crates/e2e/", "docs/", "target/"]),
            ("shared_reason", "shared code changed (host-play/api/host/script/scenario adapter, runtime or api) — "
                              "every runnable case is reachable"),
        ])),
        ("intended_scripts", intended),
        ("unavailable_by_operator_decision", ["BankSorter"]),
        ("cases", rows),
    ])

    ns.out.write_text(json.dumps(manifest, indent=1) + "\n")
    natives = [r for r in rows if r["kind"] == "native"]
    runnable = [r for r in natives if "unavailable" not in r]
    print(f"cases: {len(rows)} (native {len(natives)}, reference {len(rows) - len(natives)})")
    print(f"native runnable: {len(runnable)}  native unavailable: {len(natives) - len(runnable)}")
    print("statuses:", dict(Counter(r["status"] for r in runnable)))
    quick = [r["id"] for r in runnable if r["status"] == "vetted" and not r["manual"]]
    print(f"quick (vetted, non-manual, runnable): {len(quick)} -> {quick}")
    print(
        f"external loader smoke: id {external['LIVE_SCENARIO']}, live {external['LIVE_NAME']}, "
        f"receipt {external['RECEIPT_PREFIX']}, capture {external['TERMINAL_SHOT']!r}, "
        f"reference external-script-test"
    )
    print(f"intended scripts: {len(intended)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
