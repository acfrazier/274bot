# Offline production-harness fixture tools
#
# write_player_fixture.ts — uses server Player.save() + PlayerLoading.verify/load.
# run_write_player_fixture.sh — esbuild bundle with World/Environment stubs + real bzip2.
# test_server_root_selection.sh — CLI --server-root must override env/default for source+pack.
#
# Example:
#   ./tools/harness/run_write_player_fixture.sh \
#     --username fixtest01 \
#     --output /tmp/fixtest01.sav \
#     --receipt /tmp/fixtest01.receipt.json \
#     --fixture thiever \
#     --server-root /path/to/engine \
#     --overwrite
#
# Canonical root (source, node_modules, data/pack, bzip2, receipt):
#   --server-root CLI > SERVER_ROOT > BOT_SERVER_ROOT > default Server/engine
# Missing root path / missing --server-root value fail closed.
