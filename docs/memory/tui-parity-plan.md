# TUI parity and layout: proposed follow-up

Purpose: support careful Fairy-Ring / RS2 preservation through an operator interface and agent-driven evidence that share the same Rust host behavior. Plan alongside the memory campaign; implementation follows memory work and is sequenced with the remaining compatibility work by the operator. This is an initial source-backed proposal, not an approved feature specification.

Current evidence: tui/app.rs already exposes slot focus, map/nav, chat/dialogue, inventory/stats/nearby locs, script lifecycle, catalog loading, parameters and loadouts. tui/status.rs includes guardian/queue state. Panel app.rs/session.rs additionally expose richer profile/wall management, detailed nav config, render controls, queue/overlay inspection, script preferences and diagnostics. Verify each gap by operating both frontends before treating it as missing; several capabilities exist but are hard to discover.

Proposed layout at 120x40: a compact top status strip (target, connection, ready/active counts); a left bot list with filtering and visible selection; a main focused-bot area with tabs for Overview, Map, Script, Inventory/Bank, and Diagnostics; a bounded bottom log pane; a context-sensitive key legend. At 80x24 use one main pane with an optional bot drawer. Keep global bot selection separate from keyboard focus so a map/chat key cannot silently operate another bot. Show the target bot for actions affecting it. Multi-selection and bulk operations must visibly distinguish selected bots from the focused bot.

Implementation slices:
1. Build a panel/TUI capability matrix with actual action, Rust owner, current access path, missing host capability versus missing UI, and a matching verification scenario. Separate graphics-specific panel features from terminal equivalents.
2. Introduce responsive layout and keyboard focus/navigation without adding gameplay semantics. Preserve current shortcuts where practical and supply searchable help for changes. Keep errors and script logs visible.
3. Bring over the highest-value missing operator workflows: profile/slot lifecycle, script/config/loadout workflows, queue and navigation status, inventory/bank inspection. Implement only verified gaps through shared host interfaces.
4. Add bounded, opt-in per-slot diagnostics (including memory and nav outcomes) and deterministic scenario export/replay where fixtures permit. Keep routine rendering cheap; render visible data and borrow/cache snapshots at existing ownership boundaries.

Verification: deterministic terminal buffer tests at 80x24, 120x40, and a large layout; keyboard action tests with input-focus changes; same fixture/action traces in both frontends; real-PTY resize/error/long-log checks; 1/32/128 memory and UI latency comparisons as the campaign permits. No game renderer is required for TUI parity. Shared host logic and persisted settings remain authoritative.

Questions to resolve before implementation: priority of bulk fleet management versus detailed single-bot inspection; whether the terminal is mostly for humans over SSH or for agent operation; which panel workflows are most painful to lose. These do not block current measurement work.

## Operator clarification

The TUI serves both humans and agents. Human target: potentially ~1,000 resource-dependent fleet clients on a VPS over SSH (a design target, not a validated scale claim). Agent target: reconstructing RS2 client/server/RuneScript behavior at a historical point in time for Fairy-Ring, including FR377. Prioritize structured protocol/decoded-state assertions for dialogue content, options, format and chathead identity; add visual evidence when rendering/presentation is the behavior under test. Do not require a screenshot for every gameplay assertion. Fleet filtering, bounded logs, focused detail, machine-readable scenario evidence and reproducible failures are central. Fifty-renderer optimization remains desired but is sequenced separately from current scenario stability work.
