# Account admission clarification — approved 2026-09-10

This proposal follows independent Grok4.6 review t_d6f23813 of the exact frozen
runtime and controller6f2b410. The operator approved this clarification; it is not itself a live release.

Replace only the account/population timing requirement in
`direct-per-bot-owner-capture-plan.md` section5 item3 with:

> Preserve the existing N1 account-selection and seed contract: the diagnostic
> mints one disposable account after the frontend starts, creates its throwaway
> vault, and uses the existing Thiever setup. Before launch, root privately binds
> the exact fixture recipe, settings, N=1 active workload, source/binary, cache and
> live server identities. Prelaunch account/population receipt counts denote the
> admitted one-slot fixture; they do not assert that a named save already exists,
> is logged in, or has qualified. After launch, use the actual qualification
> `slots[].name` and client/workload observations to establish the run's account
> and `ingame && scene_state==2` readiness before accepting the capture. Do not
> predict a future name, reuse a prior run's name, equate the recipe hash with an
> account identity, or add an unsupported hash-to-name join. Keep account names
> and passwords out of the owner ledger. Preserve all existing workload, frame,
> lifecycle, resource, cleanup, and single-attempt requirements.

No change to minting, seed logic, frontend arguments, vault policy, Rust runtime,
Python controller, timeouts, source admission, or capture limits is proposed.
Server/config/cache identities still must be freshly observed before release.
The actual account does not become a prelaunch claim under this clarification.

Alternative: retain a literal preexisting named-account requirement. The current
frozen diagnostic cannot satisfy it; owner live remains held pending a separately
approved selection design. No such design or runtime change is proposed here.

Operator approved: preserve fixture behavior. Plan section5.3 now records this decision. No private receipt or live launch has yet been issued.
