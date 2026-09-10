# Direct-owner account identity timing question

Root has native-qualified controller6f2b410 and freshly verified runtime392dbecc.
Before issuing actual typed account/population receipts, root inspected exact
frozen production diagnostic source archive2c36d254 (the manifest-bound archive).

Its host-play memory setup calls mint_live_names(config.n). mint_live_names derives
names from process::id() and a process-local serial, then creates a throwaway vault.
This account identity is not available before frontend Popen. No preselected name
override is visible in this function. Source excerpt and blob hashes are retained
in diagnostics/direct-owner-managed-extension/account-admission-gap/root-source-observation.json.

Plan section5.3 requires account identity bound privately by root and actual
existing disposable account/population/settings identity at release. Controller
validate_direct_admissions instead receives an opaque fixture_binding_sha256 and
slot counts before Popen; it cannot by itself establish the future generated
name. It is not yet proven whether the intended binding can lawfully represent
the unchanged account-selection recipe plus a later actual identity proof, or
whether that would change the approved contract. Do not fabricate an observed
preexisting account, count a prospective active slot as logged-in, or change the
fixture/selection algorithm simply to admit it.

Request bounded independent fidelity review of this concrete timing mismatch,
using exact source/plan/controller. No new implementation or account/live work
until the review identifies a faithful route or required operator decision.
Navigation CF2 continues independently; owner live remains unissued.
