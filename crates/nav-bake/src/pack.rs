mod banks;
mod config_parse;
mod encode;
mod mapsquare;
mod sidecars;
mod zones;

pub use banks::derive_banks;
pub use config_parse::{
    parse_door_config, parse_door_config_ids, parse_door_open_ids, parse_passable_locs,
};
pub use encode::{encode, encode_grid};
pub use mapsquare::{merge_squares, parse_mapsquare_jm2, Mapsquare};
pub use sidecars::{encode_canlight_sidecar, encode_flags_sidecar, encode_reach_sidecar};

#[cfg(test)]
pub(crate) use mapsquare::parse_mapsquare_text;
pub(crate) use mapsquare::{parse_loc_fields, parse_map_line, section, SQUARE};

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
