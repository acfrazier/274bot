from pathlib import Path
import subprocess,json
root=Path.cwd()
groups=[(147,'ranged-consumable-options','Implement ranged and combat-consumable proof branches',['auto_fighter_range','rock_crab_range','green_dragon_special','green_dragon_potions']), (148,'alternate-camp-guard-options','Implement alternate druid camps and Fight guard-response proofs',['chaos_druid_tower','chaos_druid_yanille','ardy_cakes_fight','ardy_thiever_fight']), (149,'combat-bank-options','Implement remaining combat banking and return proofs',['auto_fighter_bank','moss_giant_bank','hill_giant_bank','chaos_druid_bank','ardy_fighter_bank']), (150,'hazard-camp-return-options','Implement hazardous-camp approach and return proofs',['rock_crab_bank','green_dragon_bank','green_dragon_tele','fire_giant_approach','fire_giant_bank'])]
parent='t_c6765b1f';rows=[]
for number,slug,title,cases in groups:
 brief=f'docs/compat/briefs/{number}-{slug}.md'
 text=f'''# {title}

Use Sol defaults. Active campaign codex/rs2b0t-multirevision. After the named
parent review, own only scenario/core proof fixtures, bounded observer additions,
relevant tests and docs/compat/05-options-{number}.md plus unique
evidence/options-{number}/. Read AGENTS.md/execution.md once. No runtime/client/
foreign script edits, no LIVE, remotes, STATE or support-matrix changes. Native
capability gaps must be reported precisely to root for the serialized native
lane, not implemented in JS or concealed by fake evidence. Commit scoped code/
report and request reviewer on this same card, then stop. Root owns LIVE.

Exact cases (and no others): {', '.join(cases)}.

Implement the remaining distinct branches identified by corrected audit145,
06ao-combat-utility-option-audit.md and
 evidence/combat-utility-option-audit/remaining-cells.json. Its seed/settings
recipes are proposals, not verified implementation: inspect actual frozen card
and siblings from both catalogs and selected native cache/game data before
choosing IDs, requirements, positions, settings or loadout. Do not inject an
undeclared setting (e.g. audit weapon hints may belong in prepared equipment).
Actual source option execution is required; capability existence is not proof.
Do not duplicate existing core/mage/fixture139/Mule work or broaden beyond the
listed cases. Reuse reviewed preparation and the existing observation/oracle
architecture. Any core still failing is a LIVE acceptance gate owned by root;
fixture preparation can proceed, but no imported core PASS may be presumed.

Prepare representative local actors on safe tiles, acknowledge inventory,
equipment, stats, prerequisites and bank stocks before Start, then let the
actual frozen script cause every required transition. Source option settings
must be explicit in identity receipts. No post-Start cheats, forced loot/guard/
combat RNG, fake pickup/energy spend, scripted fixture travel after Start, or
relaxed clocks/predicates. Preserve existing180s/150tick bounds where used;
if a full requested cycle cannot fit, report that concrete constraint and
propose a bounded honest proof decomposition to root instead of stretching it
or calling initial activity a complete cycle. Never seed the required observed
product/XP delta. Bank fixtures may seed bank supplies before Start, but must
show source-caused deposit/restock/closed return/further work as applicable.

Use exact native geometry/operations, not JS policy copies. Range needs Ranged
XP and actual ammo/equipment transition, not Strength or Magic; special needs
real energy spend, not queued arming; potions need dose consumption and effective
boost. Druid variants keep real selected Herb/Law/Nature pickup; Fight modes
must take FightBack rather than Flee. Bank/escape/approach cells need their full
declared world/stock transitions and further work, not readiness/import/startup.
Reuse appropriate existing proofs, and add regressions for meaningful false
positives and stale/seed-only paths rather than mirroring constants.

Tests/checks on an exact isolated export; never restore concurrent files or
claim shared-target provenance. Reused caches must be exclusively owned and
labeled. Run affected scenario/core tests with required features and any
relevant existing ABI test. No full unrelated test campaign. Report exact
source/client/catalog identity, command/results and every unresolved constraint.
Final platform/lifecycle/fleet refresh remains campaign-wide.
'''
 if number==150:text+='\nGreen teleport also waits on native teleport37 review; native144 owns NPC paint\nprojection. Do not duplicate either capability.\n'
 (root/brief).write_text(text)
 args=['hermes','kanban','--board','274bot','create',title,'--body',f'Read {brief} after parent review. Scoped real-script fixture/oracle work for '+', '.join(cases)+'. Same-card reviewer; no runtime changes or LIVE.','--assignee','sol','--parent',parent,'--workspace','dir:'+str(root),'--created-by','orch','--idempotency-key',f'compat-options-{number}','--json']
 if number==147:args+=['--parent','t_a7f32d56']
 if number==150:args+=['--parent','t_1bf9a22e']
 result=json.loads(subprocess.check_output(args,text=True));parent=result['id'];rows.append(dict(number=number,task_id=parent,cases=cases,brief=brief));print(rows[-1],flush=True)
(root/'.superpowers/stage2-root/option-tasks.json').write_text(json.dumps(rows,indent=2)+'\n')
