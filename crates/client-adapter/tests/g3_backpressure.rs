//! G3-F: the real Client pump retains encoded bytes when the adapter queue
//! refuses, and the reactor's later drain receives every byte exactly once.

use client::client::{Client, ClientConfig, ClientRevision};
use client::io::client_stream::ClientStream;
use client::io::host_network::{
    HostAttemptIdentity, HostCloseDisposition, HostGameStream, HostStreamExit, HostStreamIdentity,
    SealReceipt,
};
use client::io::Isaac;
use client_adapter::control::BindingId;
use client_adapter::park::Park;
use client_adapter::records::RECORD_SLOTS;
use client_adapter::shared::{AppendRefusal, SharedConnection};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct HeldReactor {
    shared: Arc<SharedConnection>,
    binding: BindingId,
}

impl HostGameStream for HeldReactor {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::WouldBlock.into())
    }

    fn available(&mut self) -> io::Result<usize> {
        Ok(0)
    }

    fn wait_available(&mut self, _: usize, _: Instant) {}

    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        match self.shared.publish_record(self.binding, record) {
            Ok(()) => Ok(()),
            Err(AppendRefusal::Backpressured) => Err(io::ErrorKind::WouldBlock.into()),
            Err(reason) => Err(io::Error::other(format!("unexpected refusal: {reason:?}"))),
        }
    }

    fn set_deadline(&mut self, _: Option<Instant>) -> io::Result<()> {
        Ok(())
    }

    fn binding_identity(&self) -> HostStreamIdentity {
        HostStreamIdentity::new(self.binding)
    }

    fn attach_cancel(
        &mut self,
        _: &client::io::client_stream::LoginCancel,
        _: &HostAttemptIdentity,
    ) -> io::Result<()> {
        Ok(())
    }

    fn rebind(&mut self, _: &HostAttemptIdentity, _: Option<Instant>) -> io::Result<()> {
        Ok(())
    }

    fn close(&mut self, _: HostCloseDisposition) {
        self.shared.publish_closed();
    }

    fn take_exit_reason(&mut self) -> Option<HostStreamExit> {
        None
    }

    fn seal_login(&mut self) -> io::Result<SealReceipt> {
        Ok(Box::new(()))
    }

    fn commit_login(&mut self, _: SealReceipt) -> io::Result<()> {
        Ok(())
    }
}

fn drain(shared: &SharedConnection, received: &mut Vec<u8>) {
    while let Some(len) = shared.records.front() {
        let start = received.len();
        received.resize(start + usize::from(len), 0);
        assert_eq!(
            shared.outbound.pop(&mut received[start..]),
            usize::from(len)
        );
        shared.records.pop_front();
    }
}

fn held_client() -> (Client, Arc<SharedConnection>, BindingId) {
    let binding = BindingId {
        incarnation: 1,
        connection: 7,
        binding: 1,
        attempt: 1,
    };
    let shared = SharedConnection::new(binding.connection, Arc::new(Park::new()));
    shared.accept_binding(binding);
    assert!(shared.commit_session(binding));
    let mut client = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: String::new(),
            members: true,
            lowmem: false,
        },
        ClientRevision::R289,
    );
    client.ingame = true;
    client.ptype = -1;
    client.stream = Some(ClientStream::from_host(Box::new(HeldReactor {
        shared: Arc::clone(&shared),
        binding,
    })));
    (client, shared, binding)
}

#[test]
fn starved_reactor_over_64_flushes_retains_encoded_bytes_and_session() {
    let (mut client, shared, _) = held_client();
    client.out.random = Some(Isaac::new(&[1, 2, 3, 4]));
    let mut expected_cipher = client.out.random.as_ref().unwrap().clone();
    let expected: Vec<u8> = (0..RECORD_SLOTS + 13)
        .map(|_| 181_i32.wrapping_add(expected_cipher.next_int()) as u8)
        .collect();
    // The reactor does not consume anything for 76 consecutive Client passes.
    // Each pass emits a real NO_TIMEOUT action through the output gate.
    for pass in 0..RECORD_SLOTS + 12 {
        client.last_outbound = Instant::now() - Duration::from_secs(1);
        client.game_loop();
        assert!(client.ingame, "queue capacity must not end pass {pass}");
        assert!(client.take_session_exit_reason().is_none());
    }
    assert_eq!(shared.records.len(), RECORD_SLOTS);
    assert_eq!(client.out.pos, 12, "the rejected suffix remains encoded");
    assert_eq!(shared.bytes_out(), RECORD_SLOTS as u64);
    let retained_cipher = client.out.random.as_ref().unwrap().clone().next_int();
    client.last_outbound = Instant::now();
    client.game_loop();
    assert_eq!(client.out.pos, 12);
    assert_eq!(
        client.out.random.as_ref().unwrap().clone().next_int(),
        retained_cipher,
        "retrying Backpressured bytes must not re-encode or advance ISAAC"
    );
    let mut received = Vec::new();
    drain(&shared, &mut received);
    assert_eq!(&received, &expected[..RECORD_SLOTS]);
    client.last_outbound = Instant::now() - Duration::from_secs(1);
    client.game_loop();
    assert!(client.ingame);
    assert_eq!(client.out.pos, 0);
    drain(&shared, &mut received);
    assert_eq!(received, expected);
    assert_eq!(shared.bytes_out(), received.len() as u64);
    assert!(shared.records.is_empty());
}

#[test]
fn full_byte_ring_retains_the_encoded_action_until_drain() {
    let (mut client, shared, binding) = held_client();
    let prefix = [0x5a; 4999];
    shared.publish_record(binding, &prefix).unwrap();
    client.last_outbound = Instant::now() - Duration::from_secs(1);
    client.game_loop();
    assert!(client.ingame);
    assert_eq!(client.out.pos, 0);
    assert_eq!(shared.outbound.len(), 5000);
    client.last_outbound = Instant::now() - Duration::from_secs(1);
    client.game_loop();
    assert!(client.ingame);
    assert_eq!(client.out.pos, 1);

    let mut received = Vec::new();
    drain(&shared, &mut received);
    assert_eq!(&received[..4999], &prefix);
    assert_eq!(received[4999], 181);
    client.last_outbound = Instant::now() - Duration::from_secs(1);
    client.game_loop();
    drain(&shared, &mut received);
    assert_eq!(&received[4999..], &[181, 181, 181]);
    assert!(client.ingame);
}

#[test]
fn exhausted_packet_extent_still_ends_the_session() {
    let (mut client, _, _) = held_client();
    client.out.pos = 5000;
    client.last_outbound = Instant::now() - Duration::from_secs(1);
    client.game_loop();
    assert!(!client.ingame);
    assert!(matches!(
        client.take_session_exit_reason(),
        Some(client::client::client::SessionExitReason::ConnectionLost)
    ));
}
