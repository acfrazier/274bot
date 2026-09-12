# Shop transfer evidence (task t_1591d140, brief 38)

Selected-cache interface decode, both frozen catalog roots' engine caches:

| File | Cache | Result |
|---|---|---|
| `iface-274.txt` | `ENGINE_DIR=/Users/acfrazier/experiments/Server/engine` | jag 10984 components; 3822 TYPE_LAYER → `[3823]`; 3823 TYPE_INV layer 3822 `iop` = Value/Sell 1/Sell 5/Sell 10; 3824 TYPE_LAYER; 3900 TYPE_INV layer 3824 `iop` = Value/Buy 1/Buy 5/Buy 10 |
| `iface-289.txt` | `ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine` | jag 11942 components; the same five interfaces decode identically |

Reproduce either log with the probe kept as `shop_iface_probe.rs` here:

```
cp docs/compat/evidence/shop-capabilities/shop_iface_probe.rs crates/api/tests/shop_iface_probe.rs
ENGINE_DIR=<engine> cargo test -p api --test shop_iface_probe -- --nocapture
rm crates/api/tests/shop_iface_probe.rs
```

The same identities are asserted by the committed test
`crates/api/tests/snapshot.rs::packed_shop_interfaces_post_main_stock_and_player_pack`,
which reads the local client jag and skips when none is present.

Composed and unit receipts (isolation: no client, no LIVE):

```
cargo test -p script --lib shop                        # 13 passed
cargo test -p script --test shop                        # 9 passed
cargo test -p script --test load_isolate shop            # 6 passed
cargo test -p api --test interact shop                   # 4 passed
cargo test -p api --test snapshot shop                   # 4 passed
cargo test -p script --test host_js                      # golden index.d.ts not stale
cargo clippy -p api -p script --all-targets -- -D warnings
rustfmt --edition 2021 --check <owned Rust>
git diff --check
```

Reviewer notes: the frozen target is
`.superpowers/inputs/rs2b0t-100adccc.../src/bot/api/shop/Shop.ts`; the
`shopOpBatch` 10/5/1 plan and the 3000 ms + one-tick settlement are implemented
in `crates/script/src/shop.rs`, never in JavaScript. Not LIVE — root owns the
headed cells.
