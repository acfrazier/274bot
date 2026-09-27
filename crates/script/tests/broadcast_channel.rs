use script::isolate_fb::encode_interact_batch;
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use std::thread;
use std::time::{Duration, Instant};

fn spawn_ready(source: &str) -> LoadIsolate {
    let isolate =
        LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).expect("spawn isolate");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match isolate.poll_ready() {
            script::Ready::Ready => return isolate,
            script::Ready::Failed(error) => panic!("isolate setup failed: {error}"),
            script::Ready::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(2));
            }
            script::Ready::Pending => panic!("isolate setup timed out"),
        }
    }
}

use serde_json::json;

#[test]
fn isolate_broadcast_channel_round_trips_bounded_values_at_tick_boundary() {
    let source = r#"
const channel = new BroadcastChannel('rs2b0t:kq:v1:a,b,c,d');
globalThis.__received = null;
globalThis.__ticks = 0;
channel.onmessage = event => {
    globalThis.__received = { data: event.data, sender: event.sender };
};
export function tick(_api) {
    globalThis.__ticks += 1;
    if (globalThis.__ticks === 1) {
        channel.postMessage({ member: { name: 'a', ready: true, food: 12 }, order: [1, 2, 3] });
    }
}
"#;
    let isolate = spawn_ready(source);
    isolate.on_game_tick(1);
    assert_eq!(isolate.probe("globalThis.__ticks").unwrap(), 1);

    let outbound = isolate.drain_interacts();
    let channel_id = match outbound.as_slice() {
        [InteractReq::ChannelOpen { channel_id, name }, InteractReq::ChannelPost {
            channel_id: posted_id,
            name: posted_name,
            data,
        }] => {
            assert_eq!(name, "rs2b0t:kq:v1:a,b,c,d");
            assert_eq!(posted_name, name);
            assert_eq!(posted_id, channel_id);
            assert_eq!(
                script::channel::decode(data).unwrap(),
                json!({"member":{"name":"a","ready":true,"food":12.0},"order":[1.0,2.0,3.0]})
            );
            *channel_id
        }
        other => panic!("unexpected channel requests: {other:?}"),
    };

    let delivered = json!({"release":{"stage":"upper","trip":2.0},"sender":"b"});
    let batch = encode_interact_batch(&[InteractReq::ChannelMessage {
        channel_id,
        sender: "b".into(),
        seq: 1,
        data: script::channel::encode(&delivered).unwrap(),
    }]);
    assert!(isolate.post_channel_events(batch));
    assert_eq!(
        isolate.probe("globalThis.__received").unwrap(),
        serde_json::Value::Null,
        "delivery must remain queued until a game tick"
    );

    isolate.on_game_tick(2);
    assert_eq!(
        isolate.probe("globalThis.__received").unwrap(),
        json!({"data":{"release":{"stage":"upper","trip":2},"sender":"b"},"sender":"b"})
    );
    assert_eq!(isolate.probe("globalThis.__ticks").unwrap(), 2);
    assert!(isolate.drain_logs().is_empty());
    isolate.join();
}

const LISTENER: &str = r#"
const channel = new BroadcastChannel('rs2b0t:kq:v1:a,b,c,d');
globalThis.__messages = [];
globalThis.__errors = [];
channel.onmessage = event => { globalThis.__messages.push(event.data); };
channel.onmessageerror = event => { globalThis.__errors.push(event.error); };
export function tick(_api) {}
"#;

fn opened_channel(isolate: &LoadIsolate) -> u64 {
    isolate.on_game_tick(1);
    // The probe is the tick's barrier: its batch has been forwarded.
    isolate.probe("true").unwrap();
    match isolate.drain_interacts().as_slice() {
        [InteractReq::ChannelOpen { channel_id, .. }] => *channel_id,
        other => panic!("unexpected channel requests: {other:?}"),
    }
}

fn message(channel_id: u64, n: i32) -> InteractReq {
    InteractReq::ChannelMessage {
        channel_id,
        sender: "b".into(),
        seq: 1,
        data: script::channel::encode(&json!({ "n": n })).unwrap(),
    }
}

/// The isolate thread holds broker deliveries for the next tick: the
/// oldest past 64 drop with one log line each, a refusal is logged and
/// raised as `messageerror`, and a reconnect keeps what is held (the
/// membership belongs to the script run, not the connection).
#[test]
fn held_deliveries_are_bounded_logged_and_survive_a_reconnect() {
    let isolate = spawn_ready(LISTENER);
    let id = opened_channel(&isolate);
    let rows = (0..66)
        .map(|n| message(id, n))
        .chain([InteractReq::ChannelStatus {
            channel_id: id,
            message: "waiting for all four JiveKQ roster members".into(),
        }])
        .collect::<Vec<_>>();
    assert!(isolate.post_channel_events(encode_interact_batch(&rows)));
    isolate.reconnect_session_work();
    isolate.on_game_tick(2);

    let messages = isolate.probe("globalThis.__messages").unwrap();
    let received = messages.as_array().unwrap();
    assert_eq!(received.len(), 63, "64 held, the refusal among them");
    assert_eq!(received[0], json!({ "n": 3 }), "the oldest dropped first");
    assert_eq!(received[62], json!({ "n": 65 }));
    assert_eq!(
        isolate.probe("globalThis.__errors").unwrap(),
        json!(["waiting for all four JiveKQ roster members"])
    );
    let logs = isolate.drain_logs();
    assert_eq!(
        logs.iter()
            .filter(|line| line.contains("BroadcastChannel delivery queue overflow"))
            .count(),
        3,
        "{logs:?}"
    );
    assert!(
        logs.iter().any(|line| {
            line.contains("BroadcastChannel refused: waiting for all four JiveKQ roster members")
        }),
        "{logs:?}"
    );
    isolate.join();
}
