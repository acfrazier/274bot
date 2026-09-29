# 274V10 navigation pack fixture

`tiny-274V10.bin` is a real version-10 navigation pack, the format 0.1.9 and
0.1.9.1 wrote to `~/.274bot/289/274bot.navpack`. It was written by the v10
encoder itself, not patched from newer bytes.

## Provenance

- Source: commit `a167fb6e9e0db0ac2a83bee9c953b691d596b759` (last `274V10`
  encoder, `nav::pack::VERSION = 10`), extracted with `git archive`.
- Generator: a throwaway `nav` example run in that tree, encoding the same
  2×1 world as `tiny_v8_pack()` in `tests/session_profile.rs` (origin
  `(3200, 3200, 0)`, all-open walk bytes, empty `TransportGraph`, no banks):

  ```rust
  let (walk, blocked) = pack_walk(&[0; 8]);
  let bytes = nav::pack::encode(
      &WorldCollision { origin, width: 2, height: 1, walk, blocked, flags: None },
      &TransportGraph::default(),
      &[],
  );
  ```

- Size: 61 bytes. SHA-256
  `eeb3b52cd4e4d02e08cca0e2fc1773f38c3fb96fe47788f61b0b6f1b9dc4d7ea`.

The extension is `.bin` because the repository ignores `*.navpack`; tests copy
it to the pack paths they exercise. Current code must refuse it as
`BadVersion(10)` and ask for a rebake, and must never rewrite or delete it.
