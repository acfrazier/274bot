# Sustained fleet prerequisite

Root built exact host 6d750e65/client 56d8027 in a new empty target with only a
seven-line private TUI entry calling IsolatedEnv before the shipped frontend.
The unchanged frozen Thiever uses Memory food / Lobster, Auto bank, initial
four food, target 22 and threshold 3. Both revisions completed 120-second warmup,
600-second observation and the existing 60-second teardown.

`harvest_n1.py` independently verifies source/binary/log hashes, actual depleted
food, loaded bank at its real tile, inventory 3 to 22, return and further steals,
plus positive actual client XP in each of four 150-second windows. Both pass;
observation gains are 2,902 on 274 and 2,574 on 289. Script paint counters alone
are not the witness. The 274 record captures bank 2,000 to 1,981; 289's 1 Hz
sampler misses the bank-after value, but captures actual replenishment at bank
and return. Its bank trip occurred in warmup before sustained observation.

Source/build/raw/provenance are retained alongside `n1-results-6d750e65.json`.
These overlap Grok work and other live/build activity. RSS/CPU are shipped
diagnostics, with no matched performance comparison or savings claim. N32
started separately and must independently qualify every actor; N1 does not
establish the fleet, controls, or full frontend acceptance.

## N32 274 on 6d750e65

All 32 actors pass bank/restock, observed return and further theft, with
1,965–4,540 XP per actor during 600 seconds of observation. Each of their
four 150-second intervals is positive (minimum 187 XP). Fourteen bank-after
counts were sampled; the other actors still have loaded bank-before, actual
food replenishment at bank, return and subsequent XP/coins. Some actors eat
to two food while stunned before banking, which is valid for the threshold
of three. The collector records the initial four food after Script Start,
including while the seed runner is still waiting for first theft; it verifies
that runner later passes. Requiring its Passed label for the initial baseline
incorrectly skipped two real baselines. No product/fixture behavior changed.

The shipped samples report median resident memory 1,268,727,808 bytes and
mean process CPU about 199.5% across the observation window. Concurrent agent
work means these are diagnostics, with no matched performance claim. N32 289
also passes; final integrated-source preservation remains a later
gate. `harvest_fleet.py` verifies every actor and source/binary/log hash.

## N32 289 on 6d750e65

All 32 actors independently pass the same food depletion, loaded bank,
replenishment to 22, return and further theft witnesses. The process exits
zero after 600.037 seconds of observation; gains are 2,340–4,492 XP per actor
and every 150-second interval is positive (minimum 234 XP). Fifteen bank-after
counts were sampled. Source (1,969 files), binary and all raw log hashes verify.
`n32-r289-6d750e65/qualification.json` preserves each actor's observations.
This completes N1 and N32 qualification on this candidate for both revisions;
final integrated-source regression and other frontend/control gates remain.
