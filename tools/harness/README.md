# Offline production-harness fixture tools
#
# write_player_fixture.ts — uses server Player.save() + PlayerLoading.verify/load.
# run_write_player_fixture.sh — esbuild bundle with World/Environment stubs + real bzip2.
#
# Example:
#   ./tools/harness/run_write_player_fixture.sh \
#     --username fixtest01 \
#     --output /tmp/fixtest01.sav \
#     --receipt /tmp/fixtest01.receipt.json \
#     --fixture thiever \
#     --overwrite
#
# Requires BOT_SERVER_ROOT (or default Server/engine) with data/pack and node_modules.
