# NOTICE

**274bot** is an independent Rust **bot host** for a 274-era client. It is **not** a RuneScape official product, **not** endorsed by Jagex Ltd, **not** official Lost City / LostCityRS, and **not** a Fairy Ring release.

**RuneScape** and related marks are trademarks of Jagex Ltd. Period assets (jag files, cache, maps) remain Jagex IP where they exist in *other* trees — they are **not** redistributed from this git repository.

## Client (submodule)

Headless clients in this process are [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) (`r274-bh-modular`), vendored at `vendor/fr-client-rust`. That tree is a bot-host fork of the modularized [Fairy-Ring/FR-client-rust](https://github.com/Fairy-Ring/FR-client-rust) 274 client (`r274-modular` is the same refactor without bot-host hooks; `r274-bothost` is the pre-modular fork). Upstream is a derivation of open **Lost City / LostCityRS** client work (Client-TS 274, Client-Java 274) under MIT. See the submodule’s [NOTICE.md](vendor/fr-client-rust/NOTICE.md) and [LICENSE](vendor/fr-client-rust/LICENSE).

This repository does **not** relicense Lost City–originated client code as original work of 274bot. Bot crates (`host`, `vault`, `api`, `host-play`, `panel`, `nav`, `script`, `e2e`) are original to this project under [LICENSE](LICENSE) (MIT).

Do **not** present this repo as “Lost City Client,” “LC,” “Fairy Ring,” or “rs2b0t.”

## Ideas borrowed, not copied

- **API shape:** snapshot → query → interact → settle is a **borrowed idea** from m8aq-style bot APIs. This is not a file port of m8aq (or any other bot framework).
- **Product:** a Rust-first **rewrite** of the rs2b0t *idea* (many headless 274 clients, login queue, live harnesses). It is **not** a port of the rs2b0t TypeScript tree.
- Script TS under `scripts/rs2b0t/` is a **reference copy** of rs2b0t bots for hand-ports, not a runtime.

## AI use (explicit)

Development of this repository **uses AI tools and coding agents**. Humans own product judgment.
