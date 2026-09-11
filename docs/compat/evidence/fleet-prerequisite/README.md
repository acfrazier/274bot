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
