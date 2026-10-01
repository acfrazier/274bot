# NOTICE

**274bot** is an independent Rust **bot host** for a 274-era client. It is **not** a RuneScape official product, **not** endorsed by Jagex Ltd, **not** official Lost City / LostCityRS, and **not** a Fairy Ring release.

**RuneScape** and related marks are trademarks of Jagex Ltd. Period assets (jag files, cache, maps) remain Jagex IP where they exist in *other* trees — they are **not** redistributed from this git repository.

The macOS, Windows and Linux release packages include derived **WalkTo map imagery**: terrain PNGs baked from the revision-289 client cache (`map/289/`), as rs2b0t ships its map images. No other period assets are redistributed.

## Client (submodule)

Headless clients in this process are [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) (`r274-bh-modular`), vendored at `vendor/fr-client-rust`. That tree is a bot-host fork of the modularized [Fairy-Ring/FR-client-rust](https://github.com/Fairy-Ring/FR-client-rust) 274 client (`r274-modular` is the same refactor without bot-host hooks; `r274-bothost` is the pre-modular fork). Upstream is a derivation of open **Lost City / LostCityRS** client work (Client-TS 274, Client-Java 274) under MIT. See the submodule’s [NOTICE.md](vendor/fr-client-rust/NOTICE.md) and [LICENSE](vendor/fr-client-rust/LICENSE).

This repository does **not** relicense Lost City–originated client code as original work of 274bot. Bot crates (`host`, `vault`, `api`, `host-play`, `panel`, `nav`, `script`, `scenario`, `e2e`) are original to this project under [LICENSE](LICENSE) (MIT).

Do **not** present this repo as “Lost City Client,” “LC,” “Fairy Ring,” or “rs2b0t.”

## Ideas borrowed, not copied

- **API shape:** snapshot → query → interact → settle is a **borrowed idea** from m8aq-style bot APIs. This is not a file port of m8aq (or any other bot framework).
- **Product:** a Rust-first **rewrite** of the rs2b0t *idea* (many headless 274 clients, login queue, live harnesses), not a port of its TypeScript implementation. Script-API declarations in `crates/script/compat-js/index.d.ts` are generated from rs2b0t `00d39a17e0`, Copyright 2026 N64Jive (MIT), with host-specific type overlays; the emitted file retains the MIT notice. Listed TS for the 0.1.5 shim is loaded from an operator `$RS2B0T` checkout (`src/bot/scripts`), not vendored here.

## Vendored rendering backend

`vendor/dear-imgui-wgpu-0.15.1` is dear-imgui-wgpu 0.15.1, Copyright 2025 Mingzhen Zhuang, distributed under MIT OR Apache-2.0. The pinned shader modification replaces the renderer-wide Gamma22/Auto-on-sRGB power approximation with the IEC 61966-2-1 inverse sRGB transfer; it applies to all Dear ImGui samples on that path, including chrome and game content.

## Embedded fonts

Pass-2 native Canvas metrics/raster embed Liberation Sans Regular and Liberation Mono Regular 2.1.5 (`crates/script/fonts/`) under the SIL Open Font License 1.1. See that directory’s LICENSE and AUTHORS. Reserved Font Name: Liberation.
Panel chrome embeds a subset of 3270 Nerd Font Regular. Copyright 2022 The 3270font Authors; Copyright (c) 2011–2022 Ricardo Banffy; Copyright (c) 1993–2011 Paul Mattes; Copyright (c) 2004–2005 Don Russell; Copyright (c) 2004 Dick Altenbern; Copyright (c) 1990 Jeff Sparkes; and Copyright (c) 1989 Georgia Tech Research Corporation (GTRC), Atlanta, GA 30332. All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
3. Neither the name of Ricardo Banffy, Paul Mattes, Don Russell, Dick Altenbern, Jeff Sparkes, GTRC nor the names of their contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL RICARDO BANFFY, PAUL MATTES, DON RUSSELL, DICK ALTENBERN, JEFF SPARKES OR GTRC BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

The embedded Nerd Fonts icon glyphs are Font Awesome icons. Copyright Fonticons, Inc.; licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), via Nerd Fonts.

## AI use (explicit)

Development of this repository **uses AI tools and coding agents**. Humans own product judgment.
