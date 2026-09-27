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
