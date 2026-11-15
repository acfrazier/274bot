use super::*;
use crate::quester::progress::normalize_text;

/// The journal lines the client shows: the server joins them with `|`.
fn lines(text: &str) -> Vec<Arc<str>> {
    text.split('|').map(Arc::from).collect()
}

// Text of `dragon_journal.rs2`, one fixture per branch of the script.
const HEAD: &str = "@str@The Guildmaster of the Champions' Guild said his friend|@str@Oziach could help me with a Rune Plate mail body.|";
const SELL: &str = "@dre@Oziach@dbl@ will sell me @dre@Rune Plate Mail@dbl@ if I can prove myself|worthy by killing the @dre@Dragon Elvarg@dbl@ who lives on @dre@Crandor@dbl@|";
const RETURN_TO_OZIACH: &str =
    "@dbl@I should return to @dre@Oziach@dbl@ for more detailed instructions.";
const NEEDS: &str = "@dbl@To defeat the dragon I will need to find a @dre@map@dbl@ to|Crandor, a @dre@ship@dbl@, a @dre@captain@dbl@ to take me there and some kind|@dbl@of @dre@protection@dbl@ against the dragon's breath.|";
const MAP_PIECES: &str = "@str@I found the piece of the map that was hidden in Melzar's|@str@Maze.|One-third of the map is hidden, and only the @dre@Oracle@dbl@ on @dre@Ice|Mountain@dbl@ will know where it is.|@dbl@One-third of the map was stolen by a @dre@goblin@dbl@ from the|@dre@Goblin Village@dbl@.|";
const SHIELD: &str = "@dbl@I should ask the @dre@Duke of Lumbridge@dbl@ for an @dre@anti-|dragonbreath shield@dbl@.|";
const SHIP_FOR_SALE: &str =
    "@dbl@I should see if there is a @dre@ship@dbl@ for sale in @dre@Port Sarim@dbl@.|";
const BOUGHT: &str = "@str@I bought a ship in Port Sarim called the Lady Lumbridge.|";
const NEEDS_REPAIRS: &str = "@dbl@My ship will need some repairs, using @dre@wooden planks@dbl@ and|@dre@steel nails@dbl@, before it is seaworthy again.|";
const REPAIRED: &str = "@str@I have repaired my ship using wooden planks and steel|@str@nails.|";
const NEED_CAPTAIN: &str = "@dbl@I still need to find a @dre@captain@dbl@ for my ship.|";
const NED_AGREED: &str =
    "@str@Captain Ned from Draynor Village has agreed to sail the|@str@ship to Crandor for me.|";
const SET_SAIL: &str =
    "@dbl@Now I should go to my ship in @dre@Port Sarim@dbl@ and set sail for|@dre@Crandor@dbl@!";
const WALL: &str = "@str@Now that I have found the secret passage leading from|@str@Karamja to Crandor I don't need to worry about having a|@str@seaworthy ship or a captain fit to sail it anymore.|";
const KILL: &str = "@dbl@Now all I need to do is kill the @dre@dragon@dbl@!";

fn journal(parts: &[&str]) -> Vec<Arc<str>> {
    lines(&parts.concat())
}

#[test]
fn stage_values_match_the_content_constants() {
    // `quest_dragon.constant:1-7` and `general/configs/quest.constant:18`.
    let content = [
        ("NOT_STARTED", 0),
        ("SPOKEN_GUILDMASTER", 1),
        ("SPOKEN_OZIACH", 2),
        ("BOUGHT_SHIP", 3),
        ("REPAIRED_SHIP", 7),
        ("NED_GIVEN_MAP", 8),
        ("SAILED_TO_CRANDOR", 9),
        ("COMPLETE", 10),
    ];
    let ours: Vec<_> = DragonStage::ALL
        .iter()
        .map(|stage| (stage.upstream_name(), stage.value()))
        .collect();
    assert_eq!(ours, content);
    for (_, value) in content {
        let stage = DragonStage::from_value(value).expect("content stage");
        assert_eq!(stage.value(), value);
    }
    for skipped in [-1, 4, 5, 6, 11] {
        assert_eq!(DragonStage::from_value(skipped), None, "value {skipped}");
    }
}

#[test]
fn every_content_journal_reads_as_its_stage() {
    let cases: Vec<(&str, Vec<Arc<str>>, DragonStage)> = vec![
        (
            "stage 0, below 32 quest points",
            lines("@dbl@I can start this quest by speaking to the @dre@Guildmaster@dbl@ in|the @dre@Champions' Guild@dbl@, south-west of Varrock.|@dbl@I will need to be able to defeat a @dre@level 83 dragon@dbl@.|@dbl@To enter the Champions' Guild I need @dre@32 Quest Points@dbl@."),
            DragonStage::NotStarted,
        ),
        (
            "stage 1",
            journal(&[HEAD, SELL, "@dbl@I should speak to @dre@Oziach@dbl@, who lives by the cliffs to the|west of @dre@Edgeville@dbl@."]),
            DragonStage::SpokenGuildmaster,
        ),
        (
            // Stage 3 shows this same line until the three hints are known
            // (`dragon_journal.rs2:39`), so the journal cannot say 3 here.
            "stage 2, or stage 3 before every hint",
            journal(&[HEAD, SELL, RETURN_TO_OZIACH]),
            DragonStage::SpokenOziach,
        ),
        (
            "stage 2, hints known",
            journal(&[HEAD, SELL, NEEDS, MAP_PIECES, SHIELD, SHIP_FOR_SALE]),
            DragonStage::SpokenOziach,
        ),
        (
            "stage 3, planks still missing",
            journal(&[HEAD, SELL, NEEDS, MAP_PIECES, SHIELD, BOUGHT, NEEDS_REPAIRS, NEED_CAPTAIN]),
            DragonStage::BoughtShip,
        ),
        (
            "stage 7, ship repaired",
            journal(&[HEAD, SELL, NEEDS, MAP_PIECES, SHIELD, BOUGHT, REPAIRED, NEED_CAPTAIN]),
            DragonStage::RepairedShip,
        ),
        (
            "stage 8",
            journal(&[HEAD, SELL, MAP_PIECES, SHIELD, BOUGHT, REPAIRED, NED_AGREED, SET_SAIL]),
            DragonStage::NedGivenMap,
        ),
        (
            "stage 9, through the ship",
            journal(&[HEAD, SELL, MAP_PIECES, SHIELD, NED_AGREED, REPAIRED, KILL]),
            DragonStage::SailedToCrandor,
        ),
        (
            "stage 9, secret passage found",
            journal(&[HEAD, SELL, MAP_PIECES, SHIELD, WALL, KILL]),
            DragonStage::SailedToCrandor,
        ),
        (
            "complete",
            lines("@str@The Guildmaster of the Champions' Guild said his friend|@str@Oziach could help me with a Rune Plate mail body.|@str@Now I had proved myself worthy by killing Elvarg, Oziach|@str@agreed to sell me Rune Plate, the armour of heroes!||@red@QUEST COMPLETE!"),
            DragonStage::Complete,
        ),
    ];
    for (label, text, expected) in cases {
        assert_eq!(resolve(&text), Some(expected), "{label}");
    }
}

#[test]
fn unrelated_text_reads_as_no_stage() {
    assert_eq!(
        resolve(&lines("Close window||Something else entirely.")),
        None
    );
    assert_eq!(resolve(&[]), None);
}

#[test]
fn each_rule_is_normalized_and_reaches_itself() {
    for rule in &RULES {
        for needle in rule.all {
            assert_eq!(normalize_text(needle), *needle);
        }
        let own = Arc::<str>::from(rule.all.join(" "));
        assert_eq!(resolve(&[own]), Some(rule.stage), "needles {:?}", rule.all);
    }
}

#[test]
fn only_the_dragon_quest_answers() {
    let complete = lines("@red@QUEST COMPLETE!");
    assert_eq!(journal_stage("dragon", &complete), Some(10));
    assert_eq!(journal_stage("cook", &complete), None);
    assert_eq!(journal_stage("dragon", &lines("nothing useful")), None);
}
