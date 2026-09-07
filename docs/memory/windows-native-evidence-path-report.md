Windows native evidence reader report

Scope

The reader now validates the native cache directory by filesystem identity rather than comparing path strings. This is required because the Rust Windows producer may persist an extended-length spelling (for example, \\?\C:\Users\BotTest\274bot-server-4c95f87\data\pack\client) while the cache fingerprint records the ordinary spelling of the same directory.

Path contract

- Both paths must exist and be directories.
- os.path.samefile is the identity check. It accepts equivalent spellings, including Windows extended-length and ordinary paths when the Windows filesystem resolves them to the same directory, and it preserves symlink/UNC identity semantics.
- A distinct directory, missing path, inaccessible path, or non-directory returns unavailable. The reader does not normalize slash direction, case, prefixes, or UNC syntax globally.
- Cache file hashes, snapshot content identity, and before/after verification brackets remain independently checked.
- A Windows receipt is not rebound on another host merely because its path strings look compatible; the receipt still needs all locally readable, hash-bound artifacts and producer evidence.

Panel terminal metadata

Panel launches are non-terminal. The exact match-key combination frontend=panel, terminal=false, terminal_size=null is now treated as explicit inapplicability, not missing terminal geometry. The reader still fails closed for an omitted terminal_size, terminal=true with null or malformed size, a TUI/non-panel null size, and a non-null size on a non-terminal panel record. CLI validation continues to require metadata terminal state to agree with the effective launcher arguments.

The explicit null does not prove a terminal size, panel/window geometry, or GUI dimensions. No such geometry provenance is claimed by this reader.

Evidence used

Frozen native panel N1: raw 20260907T173738Z_panel_n1_active, completion/qualification status preserved. The original independent-binding receipt is not modified by this change; a later Windows replay may write a separately named derived result with the reader commit recorded.
