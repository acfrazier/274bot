# AutoFighter mage and native autocast fixture

## Fixture

`auto_fighter_mage` is a separate AutoFighter scenario, leaving the existing melee/Strength fixture unchanged. Before Start it prepares Magic 13, Hitpoints 40, eight Trout, a Staff of fire, 150 Mind runes, and 300 Air runes on the safe seed tile. It acknowledges every inventory/stat prerequisite, wears the staff, then teleports to the Ardougne Guard area. Its injected catalog settings are the frozen schema's exact supported branch: `target=Guard`, `spot=Start position`, `combatStyle=mage`, `spell=Fire Strike`, `runesWithdraw=150`, `food=Trout`, `foodWithdraw=8`, `banking=None`, and clue/special/bone options off.

Both frozen AutoFighter sources are byte-identical at SHA-256 `d82972d038849d4a7f897a264ead850b4de218239d54eab16007a4e0c12266c2`. They declare `mage`, Fire Strike, and 150 casts, and route the branch through the imported native `Autocast` API. The selected revision data test passes for both revision 274 and 289, including staff/spell facts and native autocast controls: varp 108, selected value 2, and armed value 3.

## Qualification

The independent catalog baseline requires the exact ready loadout: Magic base/effective 13, Hitpoints 40, eight Trout, worn Staff of fire 1387 with no duplicate in inventory, Mind rune 558 x150, and Air rune 556 x300 near the Guard. After Start it reuses the real `CombatCoreCycle` but selects Magic XP for this branch and additionally requires exact armed varp 108 value 3, consumption of at least one Mind rune and two Air runes from baseline, a selected Guard life ending in verified death, a second selected engagement, and fresh further work. Existing melee cases continue to require Strength XP and their prior gear/loot conditions.

The regression admits a complete Fire Strike combat sequence and rejects missing, wrong, or merely inventoried staff; short/surplus cast stock; short Air stock; selected-but-not-armed autocast; selected UI without combat; direct staff melee; unchanged Magic XP; missing Mind-rune use; wrong target; and no further work. Unknown spell/content remains fail-closed through selected revision game data and the existing `spell_button_com("unknown") == -1` coverage.

## Verification and limits

Implementation commit `7706f053b2a71069e4487cd990784b7258e97552` includes the root-owned combat receipt serializer parent `cffd62c598ceab2ea80b9bcc29e0eaf58f4dd885`; this task did not change that serializer or the combat observer's death semantics. The exact review export is `.superpowers/review-exports/t_70a9e1b5-r1` with client gitlink `aef3952d1cd7bb3b93d39c497f0f476b68021c59`.

From that export, all 105 scenario tests passed; the catalog boundary suite passed 49 tests with its one LIVE test ignored; the selected-revision spell/staff test passed for both revisions; strict affected Clippy, workspace rustfmt check, and owned diff check passed. The 180-second global deadline and 150-tick watches are unchanged. No runtime, client, JavaScript, matrix, ledger, or STATE file changed, and no LIVE process was launched. Root retains the four catalog x client-revision LIVE cells and acceptance.
