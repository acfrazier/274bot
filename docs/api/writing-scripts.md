# Writing native JavaScript and TypeScript scripts

Use JS API v2 for native scripts. This guide describes its manifest and migration rules. See the [JS API v2 reference](js-api-v2.md) for the runtime API. To write a script in the experimental Luau runtime instead, see [Writing Luau scripts](writing-luau-scripts.md).

## File shape

A native v2 file exports one direct, literal `manifest` declaration and one named `tick` function. The host reads the manifest without running the module. It removes the declaration from the JavaScript that runs. Do not read `manifest` at runtime. Do not use a default export.

The host accepts `export function tick(api)` and `export async function tick(api)`. Exported arrow functions do not meet this contract.

Use type-only imports from the generated declarations for `NativeApi` and
`ScriptManifest`. This sample uses the relative path from the checked-in
examples.

<!-- ci-example: quickstart-v2 -->
```ts
import type { NativeApi, ScriptManifest } from '../host-js/index.d.ts';
export const manifest = {
    apiVersion: 2,
    name: 'Quick start',
    version: '1',
    author: 'Example author',
    description: 'Log a setting value and stop.',
    category: 'Examples',
    tags: [],
    settings: {
        count: {
            type: 'number',
            default: 3,
            min: 1,
            max: 10,
            label: 'Count',
        },
    },
    capabilities: [],
} as const satisfies ScriptManifest;

export function tick(api: NativeApi): void {
    const count = api.settings.num('count', 3);
    api.log(`Count: ${count}`);
    api.stop(`Quick start complete. Count=${count}`);
}
```

A Rust integration test extracts this exact code block, loads it as a v2 card, checks its setting schema, and runs its tick. It uses no local game data. See [`authoring_guide.rs`](../../crates/script-js/tests/authoring_guide.rs).

## Manifest fields

All nine fields are required. The manifest must be a direct `export const` whose values use literal strings, numbers, booleans, arrays, and objects. Do not use variables, function calls, computed values, or spreads in the manifest.

| Field | Required value |
| --- | --- |
| `apiVersion` | Integer numeric literal `2`. |
| `name` | Non-empty script name. |
| `version` | Non-empty script version string. |
| `author` | Non-empty author string. |
| `description` | Non-empty description string. |
| `category` | Non-empty category string. |
| `tags` | Array of unique strings. Use `[]` when there are no tags. |
| `settings` | Object keyed by setting ID. Use `{}` when there are no settings. |
| `capabilities` | Array of supported capability strings. Use `[]` when none are needed. |

The current v2 capability is `host-packet-protocol`. The generated [host declarations](../../crates/script-js/host-js/index.d.ts) define the manifest and runtime types.

## Settings schema

Each setting is a strict object. It must include `type` and `default`. The accepted types are `boolean`, `number`, `string`, `string[]`, and `tile`. The default must match its type. The host rejects unknown setting fields and invalid values.

The accepted setting fields are `type`, `default`, `label`, `min`, `max`, `options`, `optionLabels`, `group`, `showIf`, `optionsFrom`, and `help`. Use `label`, `group`, and `help` for display text.

Use `min` and `max` only with `type: 'number'`. They must be finite numeric literals. `min` cannot exceed `max`, and the default must be within the range. Number settings use free-text input. The UI displays and enforces the bounds. These fields do not create a slider or a step size.

Use `options` for a non-empty array of unique strings on a `string` or `string[]` setting. Every default choice must appear in that array. `optionLabels` can provide display labels for those choices. Each label key must match an option.

Use `showIf: { key, anyOf }` to show a setting when another `string` or `string[]` setting has a matching value. The referenced setting must exist. It cannot be the setting itself.

`optionsFrom` is also limited to `string` and `string[]` settings. Its only v2 values are `'loadouts'` and `'named-banks'`. Do not combine it with `options`.

- `'loadouts'` reads the saved loadout names. For a setting whose ID is exactly `loadout`, an empty-string default (`''`) is the blank sentinel. When saved loadouts exist, the host resolves that sentinel to the first saved loadout.
- `'named-banks'` reads the named-bank catalog. An empty string is not a sentinel. The host refuses a blank saved value as an invalid option. Set a valid, non-empty default.

## Migration from native v1 declarations

This migration applies to native scripts. An rs2b0t `defineBot` script or default-export class remains on the compatibility v1 path. Do not convert that script by adding a native v2 manifest.

1. Move a standalone `apiVersion` export into the manifest as `apiVersion: 2`. Do not export `apiVersion` separately.
2. Move the old `SETTINGS` object into `manifest.settings`. Give each setting a supported `type` and a matching `default`. Replace any fields that are not in the strict settings schema.
3. Remove the `hostCapabilities` export. Use `capabilities: []` unless the v2 script needs the supported `host-packet-protocol` capability.
4. Replace a native default export with `export function tick(api)` or `export async function tick(api)`.
5. Remove every runtime reference to `manifest`. The host uses the declaration only while it validates and prepares the script.

The host reports these stable refusal codes for common migration errors:

| Condition | Code | Reason |
| --- | --- | --- |
| A native file exports `tick` or `apiVersion` but has no manifest. | `manifest-required` | `native JavaScript and TypeScript scripts require a manifest.` |
| A manifest is combined with a separate `apiVersion`, `SETTINGS`, or `hostCapabilities` export. | `manifest-conflicting-export` | `manifest must not be combined with export "<name>"` |
| A native declaration uses `hostCapabilities` instead of the v2 manifest capability field. | `host-capabilities-api-conflict` | `hostCapabilities is a compat v1 extension only.` |
| A manifest does not have a named exported `tick` function. | `api-version-missing-tick` | `manifest requires an exported tick function` |

The `manifest-required` diagnostic includes the host's migration example. A `manifest-conflicting-export` diagnostic names the conflicting export. The host also reports the source location and field path.

## Curated runnable examples

These examples use the existing JS v2 sample set. The Rust tests load and execute the checked-in files.

- [BoneBurier](../../crates/script-js/examples/bone_burier_v2.ts) demonstrates a setting, snapshot polling, and a bank operation. Its [integration tests](../../crates/script-js/tests/bone_burier_v2.rs) use synthetic snapshots.
- [Brimhaven route inspection](../../crates/script-js/examples/route_inspect_brimhaven_v2.ts) demonstrates an asynchronous native helper and published snapshot results. Its [integration tests](../../crates/script-js/tests/route_inspect_brimhaven_v2.rs) execute the actual example source.

Run `cargo test --locked -p script-js` to check the authoring example and the curated script examples.
