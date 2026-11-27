//! Failure modes: query 2 goes out before reply 1 is applied; a reply that
//! predates the query counts; one query closes the barrier; a refusal,
//! duplicate reply or exhausted budget is accepted. A session change needs a
//! real session reset in the snapshot, which this offline fixture lacks.

use super::*;
use client::client::{Client, ClientConfig};
use client::io::ClientRevision;

struct Fixture {
    client: Client,
    snapshot: GameSnapshot,
    _listener: std::net::TcpListener,
}

impl Fixture {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        let addr = listener.local_addr().expect("fixture address");
        let mut client = Client::new_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 0,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            ClientRevision::R377,
        );
        client.stream = Some(
            client::io::ClientStream::connect(&addr.ip().to_string(), addr.port())
                .expect("fixture stream"),
        );
        client.ingame = true;
        client.scene_state = 2;
        client.out.random = Some(client::io::Isaac::new(&[1, 2, 3, 4]));
        client.set_cheat_admission(client::CheatAdmission::Granted);
        client.local_player = Some(client::dash3d::ClientPlayer::at(0, 0));
        client.gens.player += 1;
        client.gens.player_info += 1;
        let mut fixture = Self {
            client,
            snapshot: GameSnapshot::new(),
            _listener: listener,
        };
        fixture.rebuild();
        fixture
    }

    fn rebuild(&mut self) {
        self.snapshot.rebuild(&self.client);
    }

    fn line(&mut self, text: &str) {
        self.client.add_chat(0, text, "");
        self.client.gens.chat += 1;
        self.rebuild();
    }

    fn ticks(&mut self, n: u64) {
        self.client.gens.player += 1;
        self.client.gens.player_info += n;
        self.rebuild();
    }

    fn sent(&self) -> usize {
        self.client.out.pos
    }
}

#[test]
fn second_query_waits_for_the_applied_first_reply() {
    let mut f = Fixture::new();
    f.line("get option_run: 0");
    let mut barrier = Barrier::start(&mut f.client, &f.snapshot, "option_run").expect("query 1");
    let after_one = f.sent();
    assert!(after_one > 0, "query 1 is a real cheat send");
    // The pre-send identical reply is stale and cannot answer query 1.
    assert!(!barrier.poll(&mut f.client, &f.snapshot).unwrap());
    assert_eq!(f.sent(), after_one, "no query 2 before reply 1");
    f.line("get option_run: 0");
    assert!(!barrier.poll(&mut f.client, &f.snapshot).unwrap());
    let after_two = f.sent();
    assert!(after_two > after_one, "reply 1 applied, so query 2 is sent");
    // Reply 1 is now stale for query 2: polling again must not close.
    assert!(!barrier.poll(&mut f.client, &f.snapshot).unwrap());
    assert_eq!(f.sent(), after_two, "never a third query");
    f.line("get option_run: 0");
    assert!(barrier.poll(&mut f.client, &f.snapshot).unwrap());
    let head = f.snapshot.chat_lines().first().map(|line| line.sequence);
    assert_eq!(barrier.boundary(), head);
    assert!(barrier.is_control(head.unwrap()));
}

#[test]
fn refusal_duplicate_and_budget_fail_closed() {
    let mut f = Fixture::new();
    let mut barrier = Barrier::start(&mut f.client, &f.snapshot, "option_run").unwrap();
    f.line("Please finish what you are doing first.");
    assert!(barrier.poll(&mut f.client, &f.snapshot).is_err());

    let mut f = Fixture::new();
    let mut barrier = Barrier::start(&mut f.client, &f.snapshot, "option_run").unwrap();
    f.line("get option_run: 1");
    f.line("get option_run: 1");
    assert!(barrier.poll(&mut f.client, &f.snapshot).is_err());

    let mut f = Fixture::new();
    let mut barrier = Barrier::start(&mut f.client, &f.snapshot, "option_run").unwrap();
    f.ticks(QUERY_TICK_BUDGET);
    assert!(barrier.poll(&mut f.client, &f.snapshot).is_err());
}
