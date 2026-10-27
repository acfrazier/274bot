#!/usr/bin/env python3
"""Census of NPC rows whose config has no strength= line.

Authority for NAV-S2C-0 (plan-r2 §2.1, plan-r3 §8). The set is every packed
row whose `.npc` block has no `strength=` line. On the pre-change extract
that set is exactly `strength: null`. After S2c-0 those rows publish the
engine default of 1 (`NpcType.ts:106` on 289; `:107` on 274) and an explicit
value, including 0, is kept.

Exit 0 only when the computed 289 counts match the plan. The printed PASS
line is the verdict.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path


PLAN_COUNTS = {
    "rows": 1596,
    "attackable": 616,
    "attackable_no_strength_line": 96,
    "plain": 75,
    "bespoke": 11,
    "bespoke_mixed": 7,
    "bespoke_mixed_forced": 3,
    "stay_unknown": 21,
    "attackable_hunt": 9,
    "all_hunt": 28,
    "all_hunt_bespoke": 15,
}
ENGINE_SOURCE = "NpcType.ts:106"
STAT_DEFAULT = 1


def stat_max(stat: int, bonus: int) -> int:
    """`select.rs:204`: `((stat + 9) * (bonus + 64) + 320) / 640`, floored at 0."""
    hit = ((stat + 9) * (bonus + 64) + 320) // 640
    return max(hit, 0)


def attackable(row: dict) -> bool:
    return "Attack" in (row.get("ops") or [])


def bucket(row: dict) -> str:
    bespoke = bool(row.get("bespoke"))
    kind = row.get("attack_kind")
    forced = row.get("forced_max_hit") is not None
    if not bespoke and kind is None and not forced:
        return "plain"
    if bespoke and kind is None:
        return "bespoke"
    if bespoke and kind == "mixed" and not forced:
        return "bespoke-mixed"
    if bespoke and kind == "mixed" and forced:
        return "bespoke-mixed-forced"
    return f"other bespoke={bespoke} kind={kind} forced={forced}"


def old_class(row: dict) -> str:
    """Classification while strength is absent, mirroring `facts.rs:103-158`."""
    if row.get("poison_severity") is None:
        return "MissingFacts"
    if row.get("dragonfire") is not None:
        return "Dragonfire"
    if row.get("counter_protect"):
        return "CounterProtect"
    if row.get("attack_kind") == "magic":
        return "Magic"
    if row.get("attack_kind") == "mixed":
        return "Mixed"
    if row.get("bespoke"):
        return "Bespoke"
    if row.get("attack_kind") is None:
        return "MissingStat"
    return "other"


def new_class(row: dict) -> str:
    """Classification after the engine strength default, same order as `facts.rs`."""
    if row.get("attack_kind") == "mixed":
        return "Mixed"
    if row.get("bespoke"):
        return "Bespoke"
    if row.get("dragonfire") is not None:
        return "Dragonfire"
    if row.get("counter_protect"):
        return "CounterProtect"
    if row.get("attack_kind") == "magic":
        return "Magic"
    if row.get("poison_severity") is None:
        return "MissingFacts"
    rate = row.get("attackrate")
    if rate == 0:
        return "ZeroRate"
    poison = row.get("poison_severity") or 0
    if poison > 0:
        return "Poison"
    return "Known"


def scan_configs(content: Path) -> dict[str, dict]:
    found: dict[str, dict] = {}
    scripts = content / "scripts"
    if not scripts.is_dir():
        raise SystemExit(f"content scripts directory missing: {scripts}")
    for path in sorted(scripts.rglob("*.npc")):
        rel = path.relative_to(content).as_posix()
        current: str | None = None
        strength_line: str | None = None
        ranged_line: str | None = None
        for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
            line = raw.strip()
            if not line or line.startswith("//"):
                continue
            if line.startswith("[") and line.endswith("]"):
                if current is not None:
                    found[current] = {
                        "file": rel,
                        "strength_line": strength_line,
                        "ranged_line": ranged_line,
                    }
                current = line[1:-1]
                strength_line = None
                ranged_line = None
                if current in found:
                    raise SystemExit(f"duplicate config [{current}]")
                continue
            if current is None or "=" not in line:
                continue
            key, _, value = line.partition("=")
            if key == "strength":
                strength_line = value
            elif key == "ranged":
                ranged_line = value
        if current is not None:
            found[current] = {
                "file": rel,
                "strength_line": strength_line,
                "ranged_line": ranged_line,
            }
    return found


# facts.rs from_npc order. PROJ_LAG is combat/risk.rs.
PROJ_LAG = 3
# Review filter that does not apply the range-overflow test (review F3).
FILTER_COUNTS = {
    "rows": 975,
    "attackable": 75,
    "not_attackable": 900,
    "not_attackable_hunt": 8,
}
# Rows whose old class is MissingStat and whose new class is not.
LEAVE_COUNTS = {
    "rows": 956,
    "attackable": 75,
    "not_attackable": 881,
    "not_attackable_hunt": 8,
    "overflow_stay": 19,
}


def facts_stat_max(stat: int | None, bonus: int | None) -> int | None:
    """`select.rs` `stat_max`: a missing stat or bonus stays unknown."""
    if stat is None or bonus is None or stat < 0:
        return None
    hit = ((stat + 9) * (bonus + 64) + 320) // 640
    if hit > 2_147_483_647:
        return None
    return max(hit, 0)


def facts_to_hit(value: int) -> int | None:
    if value < 0 or value > 255:
        return None
    return value


def facts_max_hit(row: dict, strength: int | None, ranged: int | None) -> int | None:
    forced = row.get("forced_max_hit")
    if forced is not None:
        return facts_to_hit(forced)
    if row.get("bespoke"):
        return None
    kind = row.get("attack_kind")
    if kind == "ranged":
        main = facts_stat_max(ranged, row.get("rangebonus"))
    elif kind == "magic":
        return None
    else:
        main = facts_stat_max(strength, row.get("strengthbonus"))
    if main is None:
        return None
    return facts_to_hit(main)


def facts_ranges(row: dict) -> tuple[int, int] | None:
    """`facts.rs` `compact_ranges`."""
    maxrange = row.get("maxrange")
    attackrange = row.get("attackrange")
    size = row.get("size")
    if (
        not isinstance(maxrange, int)
        or not isinstance(attackrange, int)
        or not isinstance(size, int)
        or maxrange < 0
        or attackrange < 0
        or size <= 0
    ):
        return None
    radius = maxrange + max(attackrange, 1)
    if radius > 255:
        return None
    reach = (attackrange if row.get("ap_attack") else 1) + size - 1
    if reach < 0 or reach > 255:
        return None
    return radius, reach


def facts_poison(severity: int | None) -> int | None:
    """`facts.rs` `poison_pulse`."""
    if severity is None or severity < 0:
        return None
    pulse = (severity + 4) // 5
    if pulse > 255:
        return None
    return pulse


def facts_flight(attackrange: int) -> int | None:
    if attackrange < 0:
        return None
    ticks = (attackrange * 5 + 32 + 29) // 30
    if ticks > 255:
        return None
    return ticks


def facts_class(row: dict, strength: int | None, ranged: int | None) -> str:
    """`facts.rs` `from_npc` class, in that function's order."""
    maximum = facts_max_hit(row, strength, ranged)
    if row.get("poison_severity") is None:
        return "MissingFacts"
    poison = facts_poison(row.get("poison_severity"))
    if (
        facts_ranges(row) is None
        or poison is None
        or (maximum is None and row.get("forced_max_hit") is not None)
    ):
        return "Overflow"
    if row.get("dragonfire") is not None:
        return "Dragonfire"
    if row.get("counter_protect"):
        return "CounterProtect"
    kind = row.get("attack_kind")
    if kind == "magic":
        return "Magic"
    if kind == "mixed":
        return "Mixed"
    if maximum is None and row.get("bespoke"):
        return "Bespoke"
    if kind == "ranged":
        missing = ranged is None
    elif kind in ("magic", "mixed"):
        missing = False
    else:
        missing = strength is None
    if maximum is None and missing:
        return "MissingStat"
    if maximum is None:
        return "Overflow"
    # `npc_rate` defaults an absent rate to 4 and keeps an explicit 0.
    rate = row.get("attackrate")
    rate = 4 if rate is None else rate
    if rate == 0:
        return "ZeroRate"
    if kind == "ranged":
        flight = facts_flight(row.get("attackrange") or 0)
        if flight is None or flight > PROJ_LAG or rate <= flight:
            return "FlightRate"
    if poison > 0:
        return "Poison"
    return "Known"


def absent_stat(row: dict, configs: dict[str, dict], key: str) -> int | None:
    """Published value, or None when the `.npc` block has no line for `key`."""
    config = configs.get(row["config"])
    if config is None or config[f"{key}_line"] is None:
        return None
    return row.get(key)


def review_filter(row: dict, configs: dict[str, dict]) -> bool:
    """Rows the review counted without the range-overflow test."""
    if row.get("poison_severity") is None or row.get("dragonfire") is not None:
        return False
    if row.get("forced_max_hit") is not None or row.get("counter_protect"):
        return False
    if row.get("bespoke") or row.get("attack_kind") in ("magic", "mixed"):
        return False
    if row["config"] not in configs:
        return False
    if row.get("attack_kind") == "ranged":
        return configs[row["config"]]["ranged_line"] is None
    return configs[row["config"]]["strength_line"] is None


def stat_departure(rows: list[dict], configs: dict[str, dict]) -> dict:
    filtered = [row for row in rows if review_filter(row, configs)]
    detailed = []
    for row in filtered:
        old = facts_class(
            row,
            absent_stat(row, configs, "strength"),
            absent_stat(row, configs, "ranged"),
        )
        new = facts_class(row, row.get("strength"), row.get("ranged"))
        detailed.append((row, old, new))
    leaves = [item for item in detailed if item[1] == "MissingStat" and item[2] != "MissingStat"]
    stays = [item for item in detailed if not (item[1] == "MissingStat" and item[2] != "MissingStat")]
    return {
        "filtered": detailed,
        "leaves": leaves,
        "stays": stays,
    }


def load_rows(json_path: Path) -> list[dict]:
    payload = json.loads(json_path.read_text(encoding="utf-8"))
    rows = payload.get("npc_names", {}).get("rows")
    if not isinstance(rows, list) or not rows:
        raise SystemExit(f"{json_path}: npc_names.rows missing")
    return rows


def census(rows: list[dict], configs: dict[str, dict]) -> dict:
    missing_block = [row for row in rows if row["config"] not in configs]
    no_line = []
    for row in rows:
        config = configs.get(row["config"])
        if config is None:
            continue
        if config["strength_line"] is None:
            no_line.append(row)
    attackable_rows = [row for row in rows if attackable(row)]
    attackable_no_line = [row for row in no_line if attackable(row)]
    buckets = Counter(bucket(row) for row in attackable_no_line)
    hunt_attackable = [row for row in attackable_no_line if (row.get("huntrange") or 0) > 0]
    hunt_all = [row for row in no_line if (row.get("huntrange") or 0) > 0]
    plain = [row for row in attackable_no_line if bucket(row) == "plain"]
    stay = [row for row in attackable_no_line if bucket(row) != "plain"]
    max_hits = Counter(stat_max(STAT_DEFAULT, row.get("strengthbonus") or 0) for row in plain)
    published_null = sum(1 for row in rows if row.get("strength") is None)
    published_default = sum(
        1
        for row in no_line
        if row.get("strength") == STAT_DEFAULT
    )
    return {
        "rows": len(rows),
        "missing_block": len(missing_block),
        "no_strength_line": len(no_line),
        "attackable": len(attackable_rows),
        "attackable_no_strength_line": len(attackable_no_line),
        "buckets": buckets,
        "hunt_attackable": hunt_attackable,
        "hunt_all": hunt_all,
        "plain": plain,
        "stay": stay,
        "max_hits": max_hits,
        "published_strength_null": published_null,
        "published_engine_default": published_default,
        "json_null_matches_no_line": published_null == len(no_line),
        "departure": stat_departure(rows, configs),
    }


def format_report(result: dict, configs: dict[str, dict]) -> str:
    buckets = result["buckets"]
    lines = [
        "npc-stat-census",
        f"rows {result['rows']}",
        f"config blocks missing {result['missing_block']}",
        f"no strength= line {result['no_strength_line']}",
        f"attackable {result['attackable']}",
        f"attackable, no strength= line {result['attackable_no_strength_line']}",
        f"plain {buckets.get('plain', 0)}",
        f"bespoke {buckets.get('bespoke', 0)}",
        f"bespoke-mixed {buckets.get('bespoke-mixed', 0)}",
        f"bespoke-mixed-forced {buckets.get('bespoke-mixed-forced', 0)}",
        f"stay unknown {len(result['stay'])}",
        f"attackable, no strength= line, huntrange > 0: {len(result['hunt_attackable'])}",
        f"all rows, no strength= line, huntrange > 0: {len(result['hunt_all'])}",
        f"all-rows hunt bespoke {sum(1 for row in result['hunt_all'] if row.get('bespoke'))}",
        f"published strength null {result['published_strength_null']}",
        f"published engine default on no-line rows {result['published_engine_default']}",
        f"json null matches no-line {str(result['json_null_matches_no_line']).lower()}",
        "max hit at strength 1 among the plain rows: "
        + ", ".join(f"{hit}={count}" for hit, count in sorted(result["max_hits"].items())),
        f"source when the line is absent: {ENGINE_SOURCE}",
        "plain rows that leave MissingStat:",
    ]
    for row in sorted(result["plain"], key=lambda item: item["id"]):
        bonus = row.get("strengthbonus") or 0
        hit = stat_max(STAT_DEFAULT, bonus)
        config = configs[row["config"]]
        lines.append(
            f"{row['id']}\t{row['config']}\tMissingStat\tKnown\t"
            f"strength={STAT_DEFAULT}\tmax_hit={hit}\tbonus={bonus}\t"
            f"{config['file']}\tline=absent\t{ENGINE_SOURCE}"
        )
    lines.append("rows that stay unknown:")
    for row in sorted(result["stay"], key=lambda item: item["id"]):
        config = configs[row["config"]]
        lines.append(
            f"{row['id']}\t{row['config']}\t{old_class(row)}\t{new_class(row)}\t"
            f"bespoke={str(bool(row.get('bespoke'))).lower()}\t"
            f"attack_kind={row.get('attack_kind')}\t"
            f"forced_max_hit={row.get('forced_max_hit')}\t"
            f"{config['file']}\tline=absent\t{ENGINE_SOURCE}"
        )
    lines.append(
        "attackable, strength: null, huntrange > 0: "
        f"{len(result['hunt_attackable'])} "
        "(no strength= line; the plan's 9)"
    )
    lines.append(
        "all rows, strength: null, huntrange > 0: "
        f"{len(result['hunt_all'])} "
        "(no strength= line; the plan's 28, of which "
        f"{sum(1 for row in result['hunt_all'] if row.get('bespoke'))} bespoke)"
    )
    lines.extend(format_departure(result["departure"], configs))
    return "\n".join(lines) + "\n"


def format_departure(departure: dict, configs: dict[str, dict]) -> list[str]:
    """List every review-filter row, and which of them leave MissingStat."""
    filtered = departure["filtered"]
    leaves = departure["leaves"]
    stays = departure["stays"]
    attackable_filter = [item for item in filtered if attackable(item[0])]
    quiet = [item for item in filtered if not attackable(item[0])]
    quiet_hunt = [item for item in quiet if (item[0].get("huntrange") or 0) > 0]
    leave_attackable = [item for item in leaves if attackable(item[0])]
    leave_quiet = [item for item in leaves if not attackable(item[0])]
    leave_hunt = [item for item in leave_quiet if (item[0].get("huntrange") or 0) > 0]
    lines = [
        "review filter, no range-overflow test: "
        f"{len(filtered)} "
        f"({len(attackable_filter)} attackable, {len(quiet)} not attackable, "
        f"{len(quiet_hunt)} of those hunt)",
        "leave MissingStat: "
        f"{len(leaves)} "
        f"({len(leave_attackable)} attackable, {len(leave_quiet)} not attackable, "
        f"{len(leave_hunt)} of those hunt)",
        f"review filter rows that do not leave MissingStat: {len(stays)}",
        "review filter rows, by id:",
    ]
    for row, old, new in sorted(filtered, key=lambda item: item[0]["id"]):
        config = configs[row["config"]]
        hunt = (row.get("huntrange") or 0) > 0
        lines.append(
            f"{row['id']}\t{row['config']}\t"
            f"attackable={str(attackable(row)).lower()}\t"
            f"hunt={str(hunt).lower()}\t"
            f"{old}\t{new}\t"
            f"leaves={str(old == 'MissingStat' and new != 'MissingStat').lower()}\t"
            f"{config['file']}"
        )
    lines.append("not attackable, huntrange > 0, in the review filter:")
    for row, old, new in sorted(quiet_hunt, key=lambda item: item[0]["id"]):
        lines.append(
            f"{row['id']}\t{row['config']}\thuntrange={row.get('huntrange')}\t{old}\t{new}"
        )
    return lines


def counts_match(result: dict) -> bool:
    buckets = result["buckets"]
    hunt_bespoke = sum(1 for row in result["hunt_all"] if row.get("bespoke"))
    return (
        result["rows"] == PLAN_COUNTS["rows"]
        and result["missing_block"] == 0
        and result["attackable"] == PLAN_COUNTS["attackable"]
        and result["attackable_no_strength_line"] == PLAN_COUNTS["attackable_no_strength_line"]
        and buckets.get("plain", 0) == PLAN_COUNTS["plain"]
        and buckets.get("bespoke", 0) == PLAN_COUNTS["bespoke"]
        and buckets.get("bespoke-mixed", 0) == PLAN_COUNTS["bespoke_mixed"]
        and buckets.get("bespoke-mixed-forced", 0) == PLAN_COUNTS["bespoke_mixed_forced"]
        and len(result["stay"]) == PLAN_COUNTS["stay_unknown"]
        and len(result["hunt_attackable"]) == PLAN_COUNTS["attackable_hunt"]
        and len(result["hunt_all"]) == PLAN_COUNTS["all_hunt"]
        and hunt_bespoke == PLAN_COUNTS["all_hunt_bespoke"]
        and not any(bucket(row).startswith("other") for row in result["plain"] + result["stay"])
        and all(old_class(row) == "MissingStat" and new_class(row) == "Known" for row in result["plain"])
        and all(old_class(row) == new_class(row) and new_class(row) != "Known" for row in result["stay"])
        and departure_matches(result["departure"])
    )


def departure_matches(departure: dict) -> bool:
    filtered = departure["filtered"]
    leaves = departure["leaves"]
    quiet = [item for item in filtered if not attackable(item[0])]
    leave_quiet = [item for item in leaves if not attackable(item[0])]
    return (
        len(filtered) == FILTER_COUNTS["rows"]
        and sum(attackable(item[0]) for item in filtered) == FILTER_COUNTS["attackable"]
        and len(quiet) == FILTER_COUNTS["not_attackable"]
        and sum((item[0].get("huntrange") or 0) > 0 for item in quiet)
        == FILTER_COUNTS["not_attackable_hunt"]
        and len(leaves) == LEAVE_COUNTS["rows"]
        and sum(attackable(item[0]) for item in leaves) == LEAVE_COUNTS["attackable"]
        and len(leave_quiet) == LEAVE_COUNTS["not_attackable"]
        and sum((item[0].get("huntrange") or 0) > 0 for item in leave_quiet)
        == LEAVE_COUNTS["not_attackable_hunt"]
        and len(departure["stays"]) == LEAVE_COUNTS["overflow_stay"]
        and all(item[1] == "Overflow" and item[2] == "Overflow" for item in departure["stays"])
        and all(item[2] == "Known" for item in leaves)
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--content", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    rows = load_rows(args.json)
    configs = scan_configs(args.content)
    result = census(rows, configs)
    report = format_report(result, configs)
    ok = counts_match(result)
    verdict = (
        "PASS census 75 11 7 3 96 9 28 21"
        if ok
        else "FAIL census "
        + " ".join(
            str(value)
            for value in (
                result["buckets"].get("plain", 0),
                result["buckets"].get("bespoke", 0),
                result["buckets"].get("bespoke-mixed", 0),
                result["buckets"].get("bespoke-mixed-forced", 0),
                result["attackable_no_strength_line"],
                len(result["hunt_attackable"]),
                len(result["hunt_all"]),
                len(result["stay"]),
            )
        )
    )
    text = report + verdict + "\n"
    sys.stdout.write(text)
    if args.out is not None:
        args.out.mkdir(parents=True, exist_ok=True)
        (args.out / "census.txt").write_text(text, encoding="utf-8")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
