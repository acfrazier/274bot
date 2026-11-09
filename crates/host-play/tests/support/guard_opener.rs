use std::sync::Mutex;

use api::interact::{Interactions, OpTarget, SendResult};
use api::obj_names::ItemDefView;
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, SceneView};
use client::client::{Client, ClientConfig, ClientRevision};
use client::io::ClientProt289;
use host_play::tick_engine_test_support::TickEngineTestHook;

const KNIFE_ID: i32 = 946;
const LOGS_ID: i32 = 1_511;
const ROLL_ANIMATION: i32 = 623;

/// One stale-menu opener sent from a real Client before the native batch.
#[derive(Debug, Clone, Default)]
pub struct GuardOpenerState {
    pub armed: bool,
    pub sent: bool,
    pub error: Option<String>,
    pub client_tick: Option<u32>,
    pub host_frame: Option<u64>,
    pub encoded_length: Option<usize>,
}

/// Inject one ordinary Knife-on-logs OPHELDU on the fresh roll frame before
/// the native dispatcher drains its first full batch.
pub fn frame(
    client: &mut Client,
    hold: bool,
    hook: &TickEngineTestHook,
    state: &Mutex<GuardOpenerState>,
) {
    if hold {
        return;
    }
    match state.lock() {
        Ok(state) if !state.armed || state.sent || state.error.is_some() => return,
        Ok(_) => {}
        Err(poisoned) => {
            let mut state = poisoned.into_inner();
            state.armed = false;
            state
                .error
                .get_or_insert_with(|| "guard opener state poisoned".into());
            return;
        }
    }

    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    let Some((previous_tick, previous_animation)) = hook.observed_client_frame() else {
        return;
    };
    let Some(animation) = client.local_animation_update() else {
        return;
    };
    if !snapshot.ingame()
        || snapshot.scene_state() != 2
        || snapshot.tick() == previous_tick
        || animation.sequence != ROLL_ANIMATION
        || previous_animation.is_some_and(|previous| previous.serial == animation.serial)
    {
        return;
    }

    let client_tick = snapshot.tick();
    let host_frame = hook.observed_host_frame();
    match state.lock() {
        Ok(mut state) => {
            if state.sent || state.error.is_some() {
                return;
            }
            if !state.armed {
                state.client_tick = Some(client_tick);
                state.host_frame = Some(host_frame);
                state.error = Some("guard opener was disarmed before its fresh roll frame".into());
                return;
            }
            state.armed = false;
            state.client_tick = Some(client_tick);
            state.host_frame = Some(host_frame);
        }
        Err(poisoned) => {
            let mut state = poisoned.into_inner();
            state.armed = false;
            state.client_tick = Some(client_tick);
            state.host_frame = Some(host_frame);
            state
                .error
                .get_or_insert_with(|| "guard opener state poisoned".into());
            return;
        }
    }

    if client.out.pos != 0 {
        record_error(
            state,
            client_tick,
            host_frame,
            format!(
                "Client.out had {} pending bytes before guard opener",
                client.out.pos
            ),
        );
        return;
    }

    let previous_wire_count = hook.trace().wire.len();
    let encoded_length = match emit_guard_opener(client, &snapshot) {
        Ok(encoded_length) => encoded_length,
        Err(error) => {
            record_error(state, client_tick, host_frame, error);
            return;
        }
    };
    if client.out.pos != encoded_length {
        record_error(
            state,
            client_tick,
            host_frame,
            format!(
                "guard opener encoded length {encoded_length} did not match Client.out.pos {}",
                client.out.pos
            ),
        );
        return;
    }

    let opcode = ClientProt289::OPHELDU.id as u8;
    let trace = hook.trace();
    let has_one_matching_wire = trace.wire.get(previous_wire_count..).is_some_and(|wires| {
        wires.len() == 1
            && wires[0].host_frame == host_frame
            && wires[0].request_id.is_none()
            && wires[0].opcode == opcode
            && wires[0].encoded_length == encoded_length
    });
    if !has_one_matching_wire {
        record_error(
            state,
            client_tick,
            host_frame,
            format!(
                "expected one ordinary OPHELDU ({opcode}) of {encoded_length} bytes on host frame {host_frame}; newly observed wire={:?}",
                trace.wire.get(previous_wire_count..)
            ),
        );
        return;
    }
    match state.lock() {
        Ok(mut state) => {
            state.sent = true;
            state.encoded_length = Some(encoded_length);
        }
        Err(poisoned) => {
            let mut state = poisoned.into_inner();
            state.sent = false;
            state
                .error
                .get_or_insert_with(|| "guard opener state poisoned".into());
        }
    }
}

fn emit_guard_opener(client: &mut Client, snapshot: &GameSnapshot) -> Result<usize, String> {
    if client.out.pos != 0 {
        return Err(format!(
            "Client.out had {} pending bytes before guard opener",
            client.out.pos
        ));
    }
    let knife = unique_inventory_item(snapshot, KNIFE_ID)?;
    let logs = unique_inventory_item(snapshot, LOGS_ID)?;
    let sent = {
        let mut interactions = Interactions::new(snapshot, client);
        matches!(
            interactions.use_item_on(knife, OpTarget::Item(logs)),
            SendResult::Sent { .. }
        )
    };
    if !sent {
        return Err("real Knife-on-logs use_item_on interaction was refused".into());
    }
    client.out.finish_packet();
    if client.out.pos == 0 {
        return Err("real Knife-on-logs interaction emitted no Client bytes".into());
    }
    Ok(client.out.pos)
}

fn unique_inventory_item(snapshot: &GameSnapshot, id: i32) -> Result<&ItemView, String> {
    let mut rows = snapshot
        .inventory()
        .iter()
        .filter(|row| row.def.id == id && row.count > 0);
    let Some(row) = rows.next() else {
        return Err(format!("real inventory has no positive-count item {id}"));
    };
    if rows.next().is_some() {
        return Err(format!(
            "real inventory has multiple positive-count rows for item {id}"
        ));
    }
    Ok(row)
}

fn record_error(state: &Mutex<GuardOpenerState>, client_tick: u32, host_frame: u64, error: String) {
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.armed = false;
    state.client_tick = Some(client_tick);
    state.host_frame = Some(host_frame);
    state.error = Some(error);
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::interact::Driver;
    use client::io::{ClientStream, Isaac};
    use std::net::{TcpListener, TcpStream};

    fn item(id: i32, name: &str, slot: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some(name.to_owned()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        }
    }

    fn real_client_snapshot() -> (Client, GameSnapshot, TcpListener, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let stream = ClientStream::connect(&address.ip().to_string(), address.port()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut client = Client::new_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            ClientRevision::R289,
        );
        client.stream = Some(stream);
        client.ingame = true;
        client.scene_state = 2;
        client.out.random = Some(Isaac::new(&[1, 2, 3, 4]));

        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        snapshot.seed_ingame(2);
        snapshot.seed_scene(SceneView {
            available: true,
            ..SceneView::default()
        });
        snapshot.seed_inventory(
            vec![item(KNIFE_ID, "Knife", 0), item(LOGS_ID, "Logs", 1)],
            28,
        );
        (client, snapshot, listener, server)
    }

    #[test]
    fn opener_uses_the_real_encrypted_client_item_on_encoder() {
        let (mut client, snapshot, _listener, _server) = real_client_snapshot();
        let checkpoint = Driver::packet_checkpoint(&client).unwrap();

        let encoded_length = emit_guard_opener(&mut client, &snapshot).unwrap();
        let mut opcodes = Vec::new();
        assert!(Driver::trace_packets(&client, *checkpoint, &mut |opcode| {
            opcodes.push(opcode)
        }));
        assert_eq!(opcodes, [ClientProt289::OPHELDU.id as u8]);
        assert_eq!(encoded_length, client.out.pos);
        assert_eq!(encoded_length, ClientProt289::OPHELDU.length as usize + 1);
    }
}
