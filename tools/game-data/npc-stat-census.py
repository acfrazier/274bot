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
    return "\n".join(lines) + "\n"


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
