# Native singleton reference boundary

The 007346f Linux scheduler qualification passed all 24 guard tests (57.819s),
then failed its first historical layout equality assertion. That attempt remains
failed at `/home/builder/274bot-campaign/nav-singleton-007346f-1624`; no real
feasibility run was launched. The earlier inherited-address-limit failure remains
separately preserved at `nav-singleton-e6958b2-1601`.

The initial source package supplied the macOS run-05 historical outputs. The
unchanged layout helper reports allocator usable sizes at positions 6 and 7,
using the platform's allocation introspection. In the uniform fixture, dense
usable sizes were 16384/2048 on macOS and 16392/2056 on Linux; tiled directory
usable bytes were 128 and 136 respectively. Structural capacities, object sizes
and alignment did not account for this mismatch.

Root selected the existing original f24de7c Linux generated outputs from
`/home/builder/274bot-campaign/nav-stage-a-f24de7c-1327`. This fixes the platform
of the comparison reference; it does not skip or normalize the layout check.
Root revalidated all four original source/executable admissions, checked the
original tool hashes against f24de7c Git objects, and copied all twelve original
clean/counting outputs with exact hashes. Read-only comparison found all seven
required fields equal for every original-Linux/new-Linux pair: aggregate,
logical cells, lookup and narrow checksums, full layout, narrow allocation count,
and narrow requested bytes.

Evidence: `diagnostics/nav-stage-a-native-preparation/root-linux-reference-manifest.json`,
`root-native-layout-comparison.txt`, and the exact copied `linux-f24-reference/`
files. The new source package includes that reference manifest. The previous
macOS-reference packages and both failed native directories remain intact.
Production sources, frozen helper, probe, scheduler, limits and comparison keys
are unchanged. The fresh qualification must still finish successfully; this
reference audit is neither qualification completion nor performance acceptance.
