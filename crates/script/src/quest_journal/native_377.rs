//! Selected 377 journal facts, isolated from the production 289 layout.
//! Questlist button IDs for all non-miniquest identity rows: canon content/pack/interface.pack:
//! 192,250,554,559,1626,3164,4394,5910,5913,6404,6873,7218-7269,7345,8001,8023,
//! 8324,8462,8565,8855,9813,10001,10021,11018,11743-11744,11793,12015,12025,
//! 12168,12231,12275,12658,12722,12725,12738,13242,13275,13463,13860,14055,
//! 14490,14798,14984,15121,15217,15352,15364,15457,15706,15992,16013,17374,
//! 18169,18380,18547.
//! Component labels: canon content/scripts/interfaces/questlist.if:21-1778.
//! Added journal rows: interface.pack:554 (668=rd), 7231 (7345=blackarmgang),
//! 7239 (7353=cog), 7240 (7354=itexam), 10021 (10135=horror), 12725 (12839=golem);
//! questlist.if labels: 216,348,363,1038,1158,1263 respectively.
//! D5-authorized Wanted product journal (original wording UNKNOWN): scripts/quests/quest_wanted/scripts/wanted_journal.rs2:3-4,30.
//! Client-Java 377 component loader: vendor/client-java/src/main/java/jagex2/config/Component.java:314-321,564-574.
//! Journal root/title/body: canon content/pack/interface.pack:8020-8081,12060-12109;
//! shared open procedure: scripts/player/scripts/quest_journal.rs2:1-105.
//! 9890-9893 (IF_BUTTON), 8540-8549 (IF_SETTEXT), 7973-8008 (IF_OPENMAIN),
//! 4567-4595 (CLOSE_MODAL). The existing host transaction owns those actions.
use super::{failure, ActionError};
use api::quest_facts::QuestFacts;
use api::selected::ClientRevision;
use api::snapshot::{SnapshotView, WidgetRoot};
use std::sync::Arc;

pub(super) const ROOT: i32 = 8134;
pub(super) const TITLE: i32 = 8144;

fn unavailable(reason: &str) -> ActionError {
    ActionError::Unavailable(Arc::from(reason))
}

// Each selected questlist identity row maps to its packed button component.
// The third field is true only when the published journal source uses the shared opener.
// False entries are source-mapped content gaps and remain refused.
// The button-ID source lines and shared journal opener are cited in the file header.
pub(super) const BUTTONS: &[(&str, i32, bool)] = &[
    // Canonical journals and D5's Wanted product journal use the shared opener.
    ("arena", 7358, true),
    ("arthur", 7368, true),
    ("ball", 7351, true),
    ("biohazard", 7352, true),
    ("blackarmgang", 7345, true),
    ("chompybird", 673, true),
    ("cog", 7353, true),
    ("cook", 7333, true),
    ("crest", 7357, true),
    ("death", 8438, true),
    ("demon", 7334, true),
    ("desertrescue", 7379, true),
    ("doric", 7336, true),
    ("dragon", 7383, true),
    ("druid", 7355, true),
    ("druidspirit", 8137, true),
    ("drunkmonk", 7369, true),
    ("eadgar", 8679, true),
    ("elena", 7372, true),
    ("elemental_workshop", 7459, true),
    ("fishingcompo", 7359, true),
    ("fluffs", 7360, true),
    ("fortress", 7332, true),
    ("gobdip", 7338, true),
    ("golem", 12839, true),
    ("grail", 7364, true),
    ("grandtree", 7361, true),
    ("haunted", 7339, true),
    ("hauntedmine", 11857, true),
    ("hazeelcult", 7362, true),
    ("hetty", 7348, true),
    ("hero", 7363, true),
    ("horror", 10135, true),
    ("hunt", 7341, true),
    ("imp", 7340, true),
    ("itexam", 7354, true),
    ("itgronigen", 7371, true),
    ("itwatchtower", 7349, true),
    ("ikov", 7378, true),
    ("junglepotion", 7365, true),
    ("legends", 7366, true),
    ("mcannon", 7356, true),
    ("misc", 6518, true),
    ("mortton", 8969, true),
    ("murder", 7370, true),
    ("priest", 7337, true),
    ("priestperil", 8115, true),
    ("prince", 7342, true),
    ("rd", 668, true),
    ("regicide", 8576, true),
    ("romeojuliet", 7343, true),
    ("routequest", 11907, true),
    ("runemysteries", 7335, true),
    ("scorpcatcher", 7373, true),
    ("seaslug", 7374, true),
    ("sheep", 7344, true),
    ("sheepherder", 7375, true),
    ("squire", 7346, true),
    ("tbwt", 1740, true),
    ("troll", 191, true),
    ("troll_love", 11858, true),
    ("tree", 7380, true),
    ("totem", 7381, true),
    ("upass", 9927, true),
    ("vampire", 7347, true),
    ("viking", 10115, true),
    ("wanted", 6024, true),
    ("waterfall", 7350, true),
    ("zanaris", 7367, true),
    ("zombiequeen", 7376, true),
    // These 34 questlist buttons have no published shared-journal opener.
    ("agrith", 15352, false),
    ("ahoy", 12282, false),
    ("creatureoffenkenstrain", 12129, false),
    ("deal", 14912, false),
    ("deserttreasure", 12852, false),
    ("devious", 15841, false),
    ("dwarfrock", 12772, false),
    ("elid", 15592, false),
    ("enakh", 16149, false),
    ("fairy_farmers", 6987, false),
    ("feud", 12836, false),
    ("fever", 17510, false),
    ("forget", 14169, false),
    ("garden", 14604, false),
    ("giantdwarf", 13577, false),
    ("handsand", 16128, false),
    ("hundred_main", 18306, false),
    ("ics_little", 4508, false),
    ("lost_tribe", 13389, false),
    ("makinghistory", 15487, false),
    ("mdaughter", 12389, false),
    ("mm", 11132, false),
    ("mourning", 13974, false),
    ("mourning2", 6027, false),
    ("myreque_2", 18517, false),
    ("onesmallfavour", 12345, false),
    ("rag", 18684, false),
    ("ratcatch", 15499, false),
    ("roving_elves", 12139, false),
    ("soulbane", 15098, false),
    ("swansong", 249, false),
    ("tog", 3278, false),
    ("twocats", 15235, false),
    ("zogre", 13356, false),
];

pub(super) fn admit(quest: &QuestFacts) -> Result<i32, ActionError> {
    let selected = api::game_data::for_revision(ClientRevision::R377)
        .map_err(|_| unavailable("selected 377 journal game data unavailable"))?;
    let row = selected
        .quest_identity()
        .and_then(|facts| facts.rows.iter().find(|row| row.id == quest.id.0.as_ref()))
        .ok_or_else(|| unavailable("selected 377 journal quest identity unavailable"))?;
    if row.display != quest.display.as_ref()
        || row.journal_title.as_deref() != quest.journal_title.as_deref()
        || row.component != quest.component_name.as_ref()
        || row.journal_script.as_deref() != quest.journal_script.as_deref()
    {
        return Err(unavailable("selected 377 journal quest identity mismatch"));
    }
    // Only source-mapped buttons whose content scripts use the shared opener may be admitted.
    let Some((_, button, shared_opener)) = BUTTONS
        .iter()
        .find(|(component, _, _)| row.component == *component)
    else {
        return Err(unavailable(
            "selected 377 journal quest button table unavailable",
        ));
    };
    if !*shared_opener {
        return Err(unavailable("selected 377 journal content gap"));
    }
    Ok(*button)
}

/// `split_get(0, 0..99)` order. com_5 is a static control, not journal text.
/// The cache assigns com_55 onwards a second, discontiguous ID range.
fn body_component(index: usize) -> i32 {
    match index {
        0 => 8145,
        1..=49 => 8146 + index as i32,
        _ => 12124 + index as i32,
    }
}

pub(super) fn body<'a>(snapshot: SnapshotView<'a>) -> Result<[&'a str; 100], ActionError> {
    let widgets = snapshot
        .widgets()
        .ok_or_else(|| failure("selected 377 journal widgets unavailable"))?;
    let mut lines = [""; 100];
    let mut seen = [false; 100];
    for widget in widgets.value {
        if widget.root != WidgetRoot::Main || widget.root_component_id != ROOT || widget.hidden {
            continue;
        }
        let index = match widget.component_id {
            8145 => 0,
            8147..=8195 => (widget.component_id - 8146) as usize,
            12174..=12223 => (widget.component_id - 12124) as usize,
            _ => continue,
        };
        if seen[index] {
            return Err(failure("selected 377 journal body component ambiguous"));
        }
        if widget.type_ != 4 {
            return Err(failure("selected 377 journal body is not a text component"));
        }
        seen[index] = true;
        // Snapshot decode.rs's non_empty maps a real empty text to None.
        // The observed type-4 component still proves the empty fragment.
        lines[index] = widget.text.as_deref().unwrap_or("");
    }
    if let Some(index) = seen.iter().position(|present| !present) {
        return Err(ActionError::Failed(Arc::from(format!(
            "selected 377 journal body component {} unavailable",
            body_component(index)
        ))));
    }
    Ok(lines)
}
