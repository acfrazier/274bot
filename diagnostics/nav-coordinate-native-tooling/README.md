Coordinate native tooling handoff

Candidate and source

The candidate ID is `coordinate-ebf0f30`. Every coordinate entry point requires both an external `coordinate-source-binding.json` path and its caller-supplied SHA-256. The portable provenance embedded in artifacts is schema `nav-coordinate-source-v1`, filename `coordinate-source-binding.json`, candidate ID, and SHA-256 `7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb`.

The dense control remains the complete tree at `29b7aea779322c8611f83dc193e939ca7d756f75`. The coordinate arm is the complete refined tree at `8385babb23fd15b876506d4a3f6154984a6b2df1` with only `crates/nav/src/collision.rs` replaced by blob `07e0b215aa1fd21df87f308f0a889aa4ce8bb4a7` from `ebf0f30f0422229ea75de39db06944091d6497a5`. The materializer checks the displaced blob, file mode, complete effective manifest, and exact one-path difference.

Generated/local evidence

`result.json` is the machine-readable evidence index. The final generated differential run is `docs/memory/nav-tiled-differential/coordinate-generated-02`; its audit passed over 12,954 cases and 10,601,143 input bytes. The final generated Stage A run is `docs/memory/nav-tiled-stage-a/coordinate-native-generated-04`; all four clean/counting admissions and both 12-comparison results were produced from fresh materializations.

The local host is Darwin. The scheduler qualification `coordinate-ebf0f30-qualification-03` therefore records `native_hard_as_qualified=false`, executes zero GQ probe children, and remains tooling evidence. It does not authorize native execution. Linux must rerun the root workflow with hard address-space enforcement before CF1 authorization. Failed/interrupted generated attempts were retained and were not overwritten.

Finite child contract

The legacy `stage-a-singleton-v2` path remains at its 118-child ceiling. Coordinate execution uses `stage-a-coordinate-v1`, the exact reviewed decision digest `6e53a13303442b5f6087e9321babc1d7be340031adf22a5a5c88c0edd7f225a7`, and the immutable prior ledger of four old F1 children. The cumulative CF1/CF2 ceiling is 122 children: four fresh CF1 children followed, after review, by 114 fresh CF2 children. CA remains a separate 708-child phase and requires an accepted method review. Generated scheduler qualification uses phase `GQ`, not CF1 or CF2, so its cost cannot be mistaken for real evidence.

Every coordinate authorization, claim, checkpoint, child record, and result carries the candidate/source binding and phase. CF1/CF2 authorizations and results also carry the exact decision digest and prior campaign ledger. Coordinate result entry lists are content-addressed references to full immutable record files, preserving the 256 KiB JSON bound.

Root workflow

1. Package the committed reviewed tooling with `package_reviewed_singleton_tool.py`, passing `--source-binding` and `--source-binding-sha256`. The package contains the dense/refined/client frozen packs plus a separate pack containing only the admitted overlay blob; it does not import the whole collision commit/tree.
2. On the Linux builder, run `build_native_singleton.py` with the expected source-binding SHA-256. Coordinate mode forbids `--reuse-root`, creates a `coordinate-native-build-*` root, makes all four builds fresh, runs integration/guard checks, and writes a `coordinate-ebf0f30-qualification-*` GQ proof.
3. Package with `package_native_singleton_for_concord.py`, using coordinate-prefixed run, qualification, and Concord destination names plus the expected binding SHA-256.
4. On Concord, run `qualify_native_singleton_concord.py` with a fresh coordinate qualification name and binding hash. It must produce `native_hard_as_qualified=true`.
5. Run `prepare_singleton_f1.py` with `--source-binding`, `--source-binding-sha256`, and `--child-decision-sha256 6e53a13303442b5f6087e9321babc1d7be340031adf22a5a5c88c0edd7f225a7`. This writes a CF1 authorization only; it does not launch children.
6. Launch CF1 through `sharded.py`, review its four-child result, then issue a hash-bound CF2 continuation authorization. Review CF2 before issuing CA, and require `method_review.accepted=true` for CA. No helper auto-approves either continuation.

The generated/local evidence is not a native result, CF1, CF2, CA, acceptance result, publication decision, or release approval.
