use std::sync::Arc;

use api::game_data::SelectedGameData;
use host_play::tick_engine_test_support::TickEngineTrace;

pub fn native_click_within(trace: &TickEngineTrace, origin: u32, budget: u32) -> bool {
    trace.wire.iter().any(|packet| {
        packet.request_id.is_none()
            && packet.game_tick.wrapping_sub(origin) <= budget
            && [
                client::io::ClientProt289::OPLOC1.id as u8,
                client::io::ClientProt289::OPNPC1.id as u8,
            ]
            .contains(&packet.opcode)
    })
}

pub fn first_arm_budget(
    selected: Arc<SelectedGameData>,
    method_id: &str,
    tool: i32,
    start_edges: u32,
) -> Result<u32, String> {
    let catalog =
        api::selected::FamilyPreparation::run(move |worker| selected.prepare_gathering(worker))
            .map_err(|error| format!("start gathering preparation: {error:?}"))?
            .join()
            .map_err(|error| format!("join gathering preparation: {error:?}"))?
            .map_err(|error| format!("prepare gathering startup facts: {error:?}"))?;
    let arm = &catalog
        .methods()
        .iter()
        .find(|method| method.id.0.as_ref() == method_id)
        .ok_or_else(|| format!("no selected startup method {method_id}"))?
        .tick_arm;
    let native_arm = arm
        .for_tool(tool)
        .map(|fact| fact.native_arm)
        .or(arm.native_arm)
        .ok_or_else(|| format!("no selected native-arm offset for {method_id} and tool {tool}"))?;
    // Start first emits the ordinary click. Allow its real client-to-server
    // window, then the content's native roll delay, before expecting Knife.
    Ok(start_edges + 1 + u32::from(native_arm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_knife_wait_includes_the_selected_native_roll_delay() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        assert_eq!(
            first_arm_budget(Arc::clone(&selected), "fishing.loc_2027.op1", 309, 2).unwrap(),
            7
        );
        assert_eq!(
            first_arm_budget(Arc::clone(&selected), "fishing.saltfish.op1", 303, 2).unwrap(),
            7
        );
        assert_eq!(
            first_arm_budget(Arc::clone(&selected), "mining.iron", 1273, 2).unwrap(),
            7
        );
        assert!(first_arm_budget(selected, "missing", 309, 2).is_err());
    }

    #[test]
    fn late_or_ordered_clicks_do_not_satisfy_the_native_start_bound() {
        let mut trace = TickEngineTrace::default();
        trace
            .wire
            .push(host_play::tick_engine_test_support::WireObservation {
                host_frame: 1,
                game_tick: 13,
                request_id: None,
                opcode: client::io::ClientProt289::OPLOC1.id as u8,
                encoded_length: 7,
            });
        assert!(!native_click_within(&trace, 10, 2));
        trace.wire[0].game_tick = 12;
        assert!(native_click_within(&trace, 10, 2));
        trace.wire[0].request_id = Some(1);
        assert!(!native_click_within(&trace, 10, 2));
    }
}
