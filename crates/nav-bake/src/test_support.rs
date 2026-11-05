use std::path::PathBuf;
use std::sync::Arc;

use api::selected::ClientRevision;

fn data_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../api/data/game-data")
}

fn client_revision(revision: u16) -> ClientRevision {
    match revision {
        274 => ClientRevision::R274,
        289 => ClientRevision::R289,
        _ => panic!("unsupported test revision {revision}"),
    }
}

pub(crate) fn selected_game_data(revision: u16) -> Arc<api::game_data::SelectedGameData> {
    let path = data_root().join(format!("{revision}.json"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read local game data {}: {error}", path.display()));
    api::game_data::decode_game_data(&bytes, client_revision(revision))
        .unwrap_or_else(|error| panic!("decode local game data {}: {error}", path.display()))
}

pub(crate) fn quest_facts(revision: u16) -> api::game_data::QuestIdentityFacts {
    let path = data_root().join(revision.to_string()).join("quests.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read local quest facts {}: {error}", path.display()));
    api::game_data::decode_quest_identity(&bytes)
        .unwrap_or_else(|error| panic!("decode local quest facts {}: {error:?}", path.display()))
        .0
}
