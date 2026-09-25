use api::snapshot::GameSnapshot;
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfType, IfTypeMut};
use client::io::ServerProt;
use script::isolate_fb::decode_snapshot;

use super::script_snapshot_fb;
use super::SettledStart;

const HINT: usize = 0;
const BOARD: usize = 1;
const ROOT: usize = 2;

/// A client whose open main modal walks the hint panel first: the root's
/// children are pushed in reverse (the search pops the last child), so
/// `HINT` is visited before `BOARD` and must be rejected for its missing
/// `obj_ops` rather than picked as the board.
fn client_with_hint_and_board() -> Client {
    let mut c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    let hint = c.push_iface(IfType {
        id: HINT as i32,
        r#type: ComponentType::TYPE_INV,
        ..IfType::default()
    });
    assert_eq!(hint, HINT, "fixture ids");
    c.set_iface_mut(
        HINT,
        IfTypeMut {
            // A populated hint container: order alone must not make it
            // the board.
            link_obj_type: Some(vec![4001, 4002]),
            link_obj_number: Some(vec![1, 1]),
            ..IfTypeMut::default()
        },
    );
    let board = c.push_iface(IfType {
        id: BOARD as i32,
        r#type: ComponentType::TYPE_INV,
        obj_ops: true,
        iop: [Some("Take".into()), None, None, None, None],
        ..IfType::default()
    });
    assert_eq!(board, BOARD, "fixture ids");
    c.set_iface_mut(
        BOARD,
        IfTypeMut {
            // Slot 1 is empty: the stored rows stay sparse.
            link_obj_type: Some(vec![2001, 0, 2003]),
            link_obj_number: Some(vec![1, 0, 1]),
            ..IfTypeMut::default()
        },
    );
    let root = c.push_iface(IfType {
        id: ROOT as i32,
        r#type: 0,
        children: Some(vec![BOARD as i32, HINT as i32]),
        ..IfType::default()
    });
    assert_eq!(root, ROOT, "fixture ids");
    c.set_iface_mut(
        root,
        IfTypeMut {
            text: "Puzzle board".into(),
            ..IfTypeMut::default()
        },
    );
    c.main_modal_id = root as i32;
    c
}

fn post(snap: &GameSnapshot, tick: u64) -> Vec<u8> {
    script_snapshot_fb(
        None,
        false,
        tick,
        None,
        true,
        None,
        Some(snap),
        None,
        None,
        false,
        false,
        false,
    )
    .0
}

/// The board is the `obj_ops` component (never the hint), its size is
/// the observed `link_obj_type` length, its rows are sparse, and it is
/// posted in the same buffer as the widget-text map — borrowing the
/// identified widget's rows instead of copying the world.
#[test]
fn observed_board_posts_bounded_rows_beside_the_widget_texts() {
    let mut c = client_with_hint_and_board();
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);

    let board = snap.puzzle_board();
    assert_eq!(board.component_id, BOARD as i32, "the obj_ops TYPE_INV");
    assert_eq!(board.size, 3, "size is link_obj_type.length");
    assert_eq!(board.items.len(), 2, "the empty slot yields no row");
    assert_eq!((board.items[0].def.id, board.items[0].slot), (2000, 0));
    assert_eq!((board.items[1].def.id, board.items[1].slot), (2002, 2));
    assert_eq!(
        board.items[0].actions[0].as_deref(),
        Some("Take"),
        "component ops, not held ops"
    );
    // No world copy: the board borrows the identified widget's own rows.
    let widget = snap
        .widgets()
        .iter()
        .find(|w| w.component_id == board.component_id)
        .expect("the board widget was walked");
    assert!(std::ptr::eq(board.items.as_ptr(), widget.items.as_ptr()));

    let bytes = post(&snap, 1);
    let view = decode_snapshot(&bytes).expect("snapshot");
    let posted = view.puzzle_board().expect("board posted");
    assert_eq!(posted.component_id(), BOARD as i32);
    assert_eq!(posted.size(), 3);
    let rows = posted.items();
    assert_eq!(rows.len(), 2, "bounded to the stored slots");
    assert_eq!(rows[0].id(), 2000);
    assert_eq!(rows[0].slot(), 0);
    assert_eq!(rows[0].component_id(), BOARD as i32);
    assert_eq!(rows[0].ops(), vec!["Take"]);
    assert_eq!(rows[1].id(), 2002);
    assert_eq!(rows[1].slot(), 2);
    assert_eq!(
        view.puzzle_board_generation(),
        board.generation,
        "the generation is posted with the table"
    );
    // The widget-text map is untouched by the board: same buffer, own rows.
    let texts = view.widgets();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].component_id(), ROOT as i32);
    assert_eq!(texts[0].text(), "Puzzle board");
}

/// The session generation advances on a session open, a session close
/// or a new board component — never on a piece move.
#[test]
fn generation_bumps_only_on_session_events() {
    let mut c = client_with_hint_and_board();
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let opened = snap.puzzle_board().generation;
    assert_eq!(opened, 1, "the session open bumps once");

    // A piece move: the inv gen moves (rows change), identity holds.
    c.set_iface_mut(
        BOARD,
        IfTypeMut {
            link_obj_type: Some(vec![2003, 0, 0]),
            link_obj_number: Some(vec![1, 0, 0]),
            ..IfTypeMut::default()
        },
    );
    c.bump_gens(ServerProt::UPDATE_INV_PARTIAL);
    assert!(snap.rebuild(&c));
    assert_eq!(snap.puzzle_board().items.len(), 1);
    assert_eq!(
        snap.puzzle_board().generation,
        opened,
        "a piece move is not a session event"
    );

    // Close: a present closed board on a bumped generation, so a later
    // delta keep cannot leak the live board onto this isolate.
    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_CLOSE);
    snap.rebuild(&c);
    let closed = snap.puzzle_board();
    assert_eq!(closed.component_id, -1);
    assert_eq!(closed.size, 0);
    assert!(closed.items.is_empty());
    assert_eq!(closed.generation, opened + 1, "the close bumps once");
    let bytes = post(&snap, 2);
    let view = decode_snapshot(&bytes).expect("snapshot");
    let posted = view.puzzle_board().expect("closed board is present");
    assert_eq!(posted.component_id(), -1);
    assert_eq!(posted.size(), 0);
    assert!(posted.items().is_empty());
    assert_eq!(view.puzzle_board_generation(), opened + 1);
}

// ---- puzzle-move (T-HOST-PUZZLE-OP) ----

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use client::client::MiniMenuAction;
use client::config::{Cache, ObjType};
use nav::world::NavWorld;

use super::{
    install_dispatch_barrier, script_observe_cached, script_slot, script_slot_or_insert,
    DispatchBarrier, NavBot, ScriptWall,
};

/// The puzzle fixture's component ids in push order (an empty iface table
/// takes the push index as the id): the bank withdraw component, the
/// `obj_ops` piece container, the open main-modal root, the open side
/// modal's root and its deposit container.
const MOVE_BANK: usize = 0;
const MOVE_BOARD: usize = 1;
const MOVE_ROOT: usize = 2;
const MOVE_SIDE_ROOT: usize = 3;
const MOVE_DEPOSIT: usize = 4;
/// The frozen board size the send gate refuses around.
const MOVE_SIZE: usize = 25;
/// The deposit row's obj id (named `Bones` for the fixture's ObjNames).
const MOVE_BONES: i32 = 1;

const PIECE_MOVE: i32 = 2749;
const PIECE_SPARE: i32 = 2750;
const PIECE_INDEX_FIVE: i32 = 2751;
const PIECE_NO_OPS: i32 = 2752;
struct QueueMove;

impl script::Script for QueueMove {
    fn name(&self) -> &str {
        "queue-move"
    }

    fn tick(&mut self, ctx: &mut script::ScriptCtx<'_>) {
        ctx.compiled
            .interacts
            .as_mut()
            .expect("compiled tick queue")
            .push(script::shim::InteractReq::PuzzleMove {
                id: PIECE_MOVE,
                slot: 0,
                component: MOVE_BOARD as i32,
                generation: 1,
            });
    }
}

/// A v2 script that queues the exact board click beside four stale or
/// forged shapes in the same frame, and reports the posted board.
const PUZZLE_MOVE_V2: &str = r#"
export const apiVersion = 2;
export function tick(api) {
  if (globalThis.__queued) return;
  globalThis.__queued = true;
  const board = api.snapshot.puzzle_board;
  const generation = api.snapshot.puzzle_board_generation;
  const row = board.items.find((piece) => piece.slot === 0);
  globalThis.__posted = {
    component_id: board.component_id,
    size: board.size,
    slots: board.items.map((piece) => piece.slot),
    generation,
  };
  api.request({ op: 'puzzle-move', id: row.id, slot: row.slot, component: row.component_id, generation });
  api.request({ op: 'puzzle-move', id: row.id, slot: row.slot, component: row.component_id, generation: generation - 1 });
  api.request({ op: 'puzzle-move', id: row.id, slot: 7, component: row.component_id, generation });
  api.request({ op: 'puzzle-move', id: 9999, slot: row.slot, component: row.component_id, generation });
  api.request({ op: 'puzzle-move', id: row.id, slot: row.slot, component: row.component_id + 50, generation });
  try {
    api.request({ op: 'puzzle-move', id: row.id, slot: row.slot, component: row.component_id });
    globalThis.__missing_generation = 'queued';
  } catch (e) {
    globalThis.__missing_generation = String(e && (e.message || e));
  }
}
"#;

/// A v2 script that deposits on its first tick and then queues the same
/// deposit beside a board click on its second, so the later frame runs
/// with the bank gate already closed by the armed op.
const PUZZLE_MOVE_BESIDE_DEPOSIT_V2: &str = r#"
export const apiVersion = 2;
export function tick(api) {
  const n = (globalThis.__tick = (globalThis.__tick || 0) + 1);
  api.request({ op: 'deposit', name: 'Bones' });
  if (n < 2) return;
  const board = api.snapshot.puzzle_board;
  const generation = api.snapshot.puzzle_board_generation;
  const row = board.items.find((piece) => piece.slot === 0);
  api.request({ op: 'puzzle-move', id: row.id, slot: row.slot, component: row.component_id, generation });
}
"#;

/// Records the resolved menu op the dispatch hands the Driver. The opcode
/// constant is where the packet family is decided (`OP_HELD1..5` for the
/// Held family, `INV_BUTTON1..5` for the component family), so this is
/// where "OPHELD5, never INV_BUTTON5" is observable offline.
#[derive(Default)]
struct MenuRec {
    menus: Vec<(i32, i32, i32, i32, i32)>,
    actions: Vec<i32>,
    sink: Sink,
}

/// A sink that drops writes: every assertion here is on the resolved menu
/// op, not on wire bytes.
#[derive(Default)]
struct Sink;

impl api::prot::Out for Sink {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}

impl api::interact::Driver for MenuRec {
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
        self.menus.push((slot, action, a, b, c));
    }
    fn do_action(&mut self, slot: i32) -> bool {
        self.actions.push(slot);
        true
    }
    #[allow(clippy::too_many_arguments)]
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _ty: i32,
    ) -> bool {
        false
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.sink
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

/// An attached, ingame and scene-ready client: the `Interactions`
/// preconditions a real dispatch runs behind (the same loopback-stream
/// trick the bank fixtures use).
fn attached_client() -> Client {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
    std::mem::forget(listener);
    let mut c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.minusedlevel = 0;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(client::dash3d::ClientPlayer::at(5, 5));
    c
}

/// The board fixture: an attached client whose open main modal holds a
/// withdraw component (the bank session another flow owns) beside the
/// `obj_ops` piece container, with an obj table carrying the pieces' held
/// ops. Slots 0/1 define `Move` in the fifth op, slot 2 defines a fifth op
/// without that label (the frozen index-5 path) and slot 3 defines none
/// (only `cache_held_ops`' padded `Drop`).
fn puzzle_move_client() -> Client {
    let mut c = attached_client();
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache
            .objs
            .resize(PIECE_NO_OPS as usize + 1, ObjType::default());
        cache.objs[MOVE_BONES as usize].id = MOVE_BONES;
        cache.objs[MOVE_BONES as usize].name = "Bones".into();
        for (id, fifth) in [
            (PIECE_MOVE, Some("Move")),
            (PIECE_SPARE, Some("Move")),
            (PIECE_INDEX_FIVE, Some("Op-5")),
        ] {
            let obj = &mut cache.objs[id as usize];
            obj.id = id;
            obj.name = "Sliding piece".into();
            obj.iop = [None, None, None, None, fifth.map(str::to_string)];
        }
        let obj = &mut cache.objs[PIECE_NO_OPS as usize];
        obj.id = PIECE_NO_OPS;
        obj.name = "Sliding piece".into();
    }
    let bank = c.push_iface(IfType {
        id: MOVE_BANK as i32,
        r#type: ComponentType::TYPE_INV,
        iop: [Some("Withdraw 1".into()), None, None, None, None],
        ..IfType::default()
    });
    assert_eq!(bank, MOVE_BANK, "fixture ids");
    c.set_iface_mut(
        MOVE_BANK,
        IfTypeMut {
            link_obj_type: Some(vec![0; MOVE_SIZE]),
            link_obj_number: Some(vec![0; MOVE_SIZE]),
            ..IfTypeMut::default()
        },
    );
    let board = c.push_iface(IfType {
        id: MOVE_BOARD as i32,
        r#type: ComponentType::TYPE_INV,
        obj_ops: true,
        iop: [Some("Take".into()), None, None, None, None],
        ..IfType::default()
    });
    assert_eq!(board, MOVE_BOARD, "fixture ids");
    let mut ids = vec![0; MOVE_SIZE];
    let mut counts = vec![0; MOVE_SIZE];
    for (slot, id) in [
        (0usize, PIECE_MOVE),
        (1, PIECE_SPARE),
        (2, PIECE_INDEX_FIVE),
        (3, PIECE_NO_OPS),
    ] {
        // The stored column is `obj_id + 1`: 0 is an empty slot.
        ids[slot] = id + 1;
        counts[slot] = 1;
    }
    c.set_iface_mut(
        MOVE_BOARD,
        IfTypeMut {
            link_obj_type: Some(ids),
            link_obj_number: Some(counts),
            ..IfTypeMut::default()
        },
    );
    let root = c.push_iface(IfType {
        id: MOVE_ROOT as i32,
        r#type: 0,
        children: Some(vec![MOVE_BANK as i32, MOVE_BOARD as i32]),
        ..IfType::default()
    });
    assert_eq!(root, MOVE_ROOT, "fixture ids");
    c.set_iface_mut(
        MOVE_ROOT,
        IfTypeMut {
            text: "Puzzle box".into(),
            ..IfTypeMut::default()
        },
    );
    c.main_modal_id = MOVE_ROOT as i32;
    // The side modal's deposit container: one Bones row the control frame
    // can deposit, so a later refusal of the same request is the bank
    // gate rather than a missing row.
    let side = c.push_iface(IfType {
        id: MOVE_SIDE_ROOT as i32,
        r#type: 0,
        children: Some(vec![MOVE_DEPOSIT as i32]),
        ..IfType::default()
    });
    assert_eq!(side, MOVE_SIDE_ROOT, "fixture ids");
    let deposit = c.push_iface(IfType {
        id: MOVE_DEPOSIT as i32,
        r#type: ComponentType::TYPE_INV,
        iop: [Some("Deposit All".into()), None, None, None, None],
        ..IfType::default()
    });
    assert_eq!(deposit, MOVE_DEPOSIT, "fixture ids");
    c.set_iface_mut(
        MOVE_DEPOSIT,
        IfTypeMut {
            link_obj_type: Some(vec![MOVE_BONES + 1]),
            link_obj_number: Some(vec![5]),
            ..IfTypeMut::default()
        },
    );
    c.side_modal_id = MOVE_SIDE_ROOT as i32;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    // The withdraw component has transmitting full contents, so the bank
    // session is loaded: the frame that owns it can hold a pending op,
    // which is what the board click must ride past.
    let mut full = client::io::Packet::new(vec![
        (MOVE_BANK >> 8) as u8,
        MOVE_BANK as u8,
        1,
        0,
        MOVE_BONES as u8 + 1,
        20,
    ]);
    c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut full);
    c
}

fn move_req(id: i32, slot: i32, component: i32, generation: u64) -> script::shim::InteractReq {
    script::shim::InteractReq::PuzzleMove {
        id,
        slot,
        component,
        generation,
    }
}

/// One production observe frame for the slot: the fixture's own cache and
/// ObjNames, which is the tuple a real puzzle-move needs (no cache means
/// no held-op table, and then nothing is sent).
#[allow(clippy::too_many_arguments)] // puzzle frame observe packs wall/cheat/nav handles
fn observe_puzzle_frame(
    rec: &mut MenuRec,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    snap: &GameSnapshot,
    cache: &Arc<Cache>,
    names: &Arc<api::obj_names::ObjNames>,
    tick_edge: bool,
) -> bool {
    script_observe_cached(
        rec,
        "alice",
        true,
        tick_edge,
        1,
        Some((3205, 3205, 0)),
        None,
        None,
        Some(snap),
        None,
        Some(names.as_ref()),
        scripts,
        cheats,
        navs,
        world,
        false,
        false,
        None,
        None,
        Some(Arc::clone(cache)),
        Some(Arc::clone(names)),
    )
}

/// The required public-path proof: a v2 `api.request({ op: 'puzzle-move' })`
/// leaves the isolate, drains through the FlatBuffer interact batch and
/// reaches the Driver as the Held family's `OPHELD5` on the posted board
/// component — never `INV_BUTTON5` — while a stale generation, an empty
/// slot, a missing piece and a wrong component queued in the same frame
/// send nothing.
#[test]
fn puzzle_move_drains_as_opheld_on_the_posted_board() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let world: Option<Arc<NavWorld>> = None;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_with_loadouts_settled(
            PUZZLE_MOVE_V2.into(),
            script::LoadShape::NativeTick,
            vec![],
            &[],
        )
        .expect("the isolate starts");
    let client = puzzle_move_client();
    let cache = Arc::clone(&client.cache);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&client);
    assert!(snap.attached() && snap.ingame(), "the fixture is live");
    let board = snap.puzzle_board();
    assert_eq!(board.component_id, MOVE_BOARD as i32, "the obj_ops board");
    assert_eq!(board.size, MOVE_SIZE as i32, "a full 5x5 board");
    assert_eq!(board.items.len(), 4, "the sparse piece rows");
    let generation = board.generation;
    assert_eq!(generation, 1, "the fixture opens the board once");
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
    let mut rec = MenuRec::default();
    // The JS tick queues on the first frame; the probe settles it before
    // the following frames drain the batch.
    observe_puzzle_frame(
        &mut rec, &scripts, &cheats, &navs, &world, &snap, &cache, &names, true,
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    let posted = slot
        .lock()
        .unwrap()
        .probe("globalThis.__posted")
        .expect("the v2 tick ran");
    let missing = slot
        .lock()
        .unwrap()
        .probe("globalThis.__missing_generation")
        .expect("the required-field probe ran");
    for _ in 0..2 {
        observe_puzzle_frame(
            &mut rec, &scripts, &cheats, &navs, &world, &snap, &cache, &names, false,
        );
    }
    assert_eq!(posted["component_id"], MOVE_BOARD as i32, "{posted}");
    assert_eq!(posted["size"], MOVE_SIZE as i32, "{posted}");
    assert_eq!(posted["generation"], generation, "{posted}");
    assert_eq!(
        posted["slots"],
        serde_json::json!([0, 1, 2, 3]),
        "the posted rows are the piece slots, not 25 filled slots"
    );
    assert_eq!(
        missing, "not impl: request.puzzle-move missing generation",
        "the v2 wrapper requires the generation"
    );
    assert_eq!(
        rec.menus,
        vec![(
            0,
            MiniMenuAction::OP_HELD5,
            PIECE_MOVE,
            0,
            MOVE_BOARD as i32
        )],
        "one Held-family op 5 on the posted board row"
    );
    assert_ne!(
        rec.menus[0].1,
        MiniMenuAction::INV_BUTTON5,
        "a board click must never be the component family"
    );

    assert_eq!(rec.actions, vec![0], "one menu action, no count answer");
}
/// A stop racing the handoff from the slot's first lock to interaction
/// dispatch must invalidate the drained requests before any Driver call.
#[test]
fn observe_dispatch_refuses_requests_after_stop_race() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let world: Option<Arc<NavWorld>> = None;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(QueueMove), None)
        .expect("the compiled script starts");
    let client = puzzle_move_client();
    let cache = Arc::clone(&client.cache);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&client);
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
    let barrier = DispatchBarrier::new();
    install_dispatch_barrier(Arc::clone(&barrier));

    let scripts_for_thread = Arc::clone(&scripts);
    let cheats_for_thread = Arc::clone(&cheats);
    let navs_for_thread = Arc::clone(&navs);
    let cache_for_thread = Arc::clone(&cache);
    let names_for_thread = Arc::clone(&names);
    let barrier_for_thread = Arc::clone(&barrier);
    let thread = std::thread::spawn(move || {
        barrier_for_thread.arm_for_current_thread();
        let mut rec = MenuRec::default();
        observe_puzzle_frame(
            &mut rec,
            &scripts_for_thread,
            &cheats_for_thread,
            &navs_for_thread,
            &world,
            &snap,
            &cache_for_thread,
            &names_for_thread,
            true,
        );
        rec
    });

    barrier.wait_entered();
    let slot = script_slot(&scripts, "alice").expect("slot remains addressable");
    slot.lock().unwrap().stop();
    barrier.release();
    let rec = thread.join().expect("observe thread completes");
    assert!(
        rec.menus.is_empty() && rec.actions.is_empty(),
        "a stopped generation must not reach the Driver: {:?} {:?}",
        rec.menus,
        rec.actions
    );
}

/// A Pause that wins the final dispatch fence preserves the drained batch.
/// Nothing sends while paused; Resume dispatches that exact batch once.
#[test]
fn observe_dispatch_restores_requests_after_pause_race() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let world: Option<Arc<NavWorld>> = None;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_compiled(Box::new(QueueMove), None)
        .expect("the compiled script starts");
    let client = puzzle_move_client();
    let cache = Arc::clone(&client.cache);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&client);
    let snap = Arc::new(snap);
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
    let barrier = DispatchBarrier::new();
    install_dispatch_barrier(Arc::clone(&barrier));

    let scripts_for_thread = Arc::clone(&scripts);
    let cheats_for_thread = Arc::clone(&cheats);
    let navs_for_thread = Arc::clone(&navs);
    let world_for_thread = world.clone();
    let cache_for_thread = Arc::clone(&cache);
    let names_for_thread = Arc::clone(&names);
    let barrier_for_thread = Arc::clone(&barrier);
    let snap_for_thread = Arc::clone(&snap);
    let thread = std::thread::spawn(move || {
        barrier_for_thread.arm_for_current_thread();
        let mut rec = MenuRec::default();
        observe_puzzle_frame(
            &mut rec,
            &scripts_for_thread,
            &cheats_for_thread,
            &navs_for_thread,
            &world_for_thread,
            snap_for_thread.as_ref(),
            &cache_for_thread,
            &names_for_thread,
            true,
        );
        rec
    });

    barrier.wait_entered();
    let slot = script_slot(&scripts, "alice").expect("slot remains addressable");
    slot.lock().unwrap().pause();
    barrier.release();
    let paused = thread.join().expect("observe thread completes");
    assert!(
        paused.menus.is_empty() && paused.actions.is_empty(),
        "Pause must win before the first Driver call: {:?} {:?}",
        paused.menus,
        paused.actions
    );

    slot.lock().unwrap().resume();
    let mut resumed = MenuRec::default();
    observe_puzzle_frame(
        &mut resumed,
        &scripts,
        &cheats,
        &navs,
        &world,
        snap.as_ref(),
        &cache,
        &names,
        false,
    );
    assert_eq!(resumed.menus.len(), 1, "the restored row dispatches once");
    assert_eq!(
        resumed.actions,
        vec![0],
        "no duplicate dispatch after Resume"
    );
}

/// Every stale or forged shape refuses with no send: closed board, stale
/// generation (including one that would be a valid *bank* generation),
/// wrong size, empty slot, missing piece, wrong component, and a piece
/// whose fifth op is only the padded `Drop` default.
#[test]
fn puzzle_move_refuses_closed_stale_and_forged_shapes() {
    let client = puzzle_move_client();
    let cache = Arc::clone(&client.cache);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&client);
    let board = snap.puzzle_board();
    let generation = board.generation;
    let row = |slot: i32| {
        board
            .items
            .iter()
            .find(|item| item.slot == slot)
            .expect("the fixture row")
            .clone()
    };
    let piece = row(0);
    let mut rec = MenuRec::default();
    let mut dispatch = |snap: &GameSnapshot, req| {
        rec.menus.clear();
        rec.actions.clear();
        let _ = super::dispatch_script_interact_cached(
            &mut rec,
            snap,
            None,
            Some((3205, 3205, 0)),
            &navs_for_test(),
            &None,
            None,
            "alice",
            vec![req],
            Some(Arc::clone(&cache)),
            None,
        );
        rec.menus.clone()
    };
    assert_eq!(
        dispatch(
            &snap,
            move_req(piece.def.id, piece.slot, piece.component_id, generation)
        ),
        vec![(
            0,
            MiniMenuAction::OP_HELD5,
            PIECE_MOVE,
            0,
            MOVE_BOARD as i32
        )],
        "the exact posted row is the baseline that must send"
    );
    // A generation that is *not* the puzzle session even though it is the
    // bank session: a modal packet bump advances the bank session's
    // generation while the board's own session stands still, so the
    // bank's live number is stale for a click and only the board's own
    // number sends.
    let mut bumped_client = puzzle_move_client();
    let mut bumped = GameSnapshot::new();
    bumped.rebuild(&bumped_client);
    let board_generation = bumped.puzzle_board().generation;
    let bank_generation = bumped.bank_session_generation();
    bumped_client.apply_if_openmain(&mut client::io::Packet::new(vec![
        (MOVE_ROOT >> 8) as u8,
        MOVE_ROOT as u8,
    ]));
    bumped_client.bump_gens(ServerProt::IF_OPENMAIN);
    assert!(bumped.rebuild(&bumped_client));
    assert_eq!(
        bumped_client.main_modal_id, MOVE_ROOT as i32,
        "the same modal stays open"
    );
    assert_eq!(
        bumped.puzzle_board().component_id,
        MOVE_BOARD as i32,
        "the same board"
    );
    assert_eq!(
        bumped.puzzle_board().generation,
        board_generation,
        "a modal packet bump is not a board session event"
    );
    assert_ne!(
        bumped.bank_session_generation(),
        bank_generation,
        "the bank session moved on"
    );
    let bank_now = bumped.bank_session_generation();
    assert_ne!(bank_now, board_generation, "the two sessions differ now");
    assert!(
        dispatch(
            &bumped,
            move_req(piece.def.id, piece.slot, piece.component_id, bank_now)
        )
        .is_empty(),
        "a live bank generation is stale for the board"
    );
    assert_eq!(
        dispatch(
            &bumped,
            move_req(
                piece.def.id,
                piece.slot,
                piece.component_id,
                board_generation
            )
        ),
        vec![(
            0,
            MiniMenuAction::OP_HELD5,
            PIECE_MOVE,
            0,
            MOVE_BOARD as i32
        )],
        "the board's own generation is what sends"
    );
    for (case, req) in [
        (
            "stale generation",
            move_req(piece.def.id, piece.slot, piece.component_id, generation - 1),
        ),
        (
            "empty slot",
            move_req(piece.def.id, 9, piece.component_id, generation),
        ),
        (
            "missing piece",
            move_req(
                piece.def.id + 100,
                piece.slot,
                piece.component_id,
                generation,
            ),
        ),
        (
            "wrong component",
            move_req(
                piece.def.id,
                piece.slot,
                piece.component_id + 50,
                generation,
            ),
        ),
        (
            "padded Drop only",
            move_req(PIECE_NO_OPS, 3, piece.component_id, generation),
        ),
    ] {
        assert!(dispatch(&snap, req).is_empty(), "{case} must send nothing");
    }
    // The def's own fifth op is the frozen index-5 path, and it stays the
    // Held family: no label named `Move` is needed for it.
    assert_eq!(
        dispatch(
            &snap,
            move_req(PIECE_INDEX_FIVE, 2, MOVE_BOARD as i32, generation)
        ),
        vec![(
            0,
            MiniMenuAction::OP_HELD5,
            PIECE_INDEX_FIVE,
            2,
            MOVE_BOARD as i32
        )],
        "a def-defined fifth op is the frozen index-5 path"
    );
    // A closed board is a present observation that refuses the move, and
    // its generation has moved on.
    let mut closed_client = puzzle_move_client();
    closed_client.main_modal_id = -1;
    closed_client.bump_gens(ServerProt::IF_CLOSE);
    let mut closed = GameSnapshot::new();
    closed.rebuild(&closed_client);
    assert_eq!(closed.puzzle_board().component_id, -1, "closed is posted");
    assert!(
        dispatch(
            &closed,
            move_req(piece.def.id, piece.slot, piece.component_id, generation)
        )
        .is_empty(),
        "a closed board sends nothing"
    );
    // The same board one slot short of the frozen size: the posted size is
    // the observation, and the send gate refuses around it.
    let mut short_client = puzzle_move_client();
    short_client.set_iface_mut(
        MOVE_BOARD,
        IfTypeMut {
            link_obj_type: Some(vec![PIECE_MOVE + 1; MOVE_SIZE - 1]),
            link_obj_number: Some(vec![1; MOVE_SIZE - 1]),
            ..IfTypeMut::default()
        },
    );
    short_client.bump_gens(ServerProt::UPDATE_INV_PARTIAL);
    let mut short = GameSnapshot::new();
    short.rebuild(&short_client);
    assert_eq!(short.puzzle_board().size, MOVE_SIZE as i32 - 1);
    assert!(
        dispatch(
            &short,
            move_req(
                PIECE_MOVE,
                0,
                MOVE_BOARD as i32,
                short.puzzle_board().generation
            )
        )
        .is_empty(),
        "a board that is not the frozen size sends nothing"
    );
    let _ = client;
}

/// A board click is not bank-serialized: it sends on the very frame that
/// refuses the neighbouring deposit because a bank op is already pending,
/// it never arms a bank op of its own, and the two families stay distinct
/// (`OPHELD5` for the board, `INV_BUTTON1` for the deposit).
#[test]
fn puzzle_move_rides_a_frame_with_an_active_bank_op() {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let world: Option<Arc<NavWorld>> = None;
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_load_with_loadouts_settled(
            PUZZLE_MOVE_BESIDE_DEPOSIT_V2.into(),
            script::LoadShape::NativeTick,
            vec![],
            &[],
        )
        .expect("the isolate starts");
    let client = puzzle_move_client();
    let cache = Arc::clone(&client.cache);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&client);
    assert!(snap.bank_loaded(), "the bank session is loaded");
    assert_eq!(snap.bank_side().len(), 1, "the deposit row is posted");
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&client.cache.objs));
    let stage = |tick_edge: bool, rec: &mut MenuRec| {
        observe_puzzle_frame(
            rec, &scripts, &cheats, &navs, &world, &snap, &cache, &names, tick_edge,
        )
    };
    let mut control = MenuRec::default();
    // Tick 1 queues the deposit alone: with no pending op it is accepted
    // and arms the host-owned continuation.
    stage(true, &mut control);
    assert!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("true")
            .is_ok(),
        "tick 1 settles"
    );
    stage(false, &mut control);
    stage(false, &mut control);
    assert_eq!(
        control.menus,
        vec![(
            0,
            MiniMenuAction::INV_BUTTON1,
            MOVE_BONES,
            0,
            MOVE_DEPOSIT as i32
        )],
        "the bank deposit is the component family"
    );
    let armed = script_slot(&scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .pending_bank_op();
    assert!(armed.is_some(), "the accepted deposit arms a bank op");
    // Tick 2 queues the same deposit beside the board click.
    let mut rec = MenuRec::default();
    stage(true, &mut rec);
    assert!(
        script_slot(&scripts, "alice")
            .unwrap()
            .lock()
            .unwrap()
            .probe("true")
            .is_ok(),
        "tick 2 settles"
    );
    stage(false, &mut rec);
    stage(false, &mut rec);
    assert_eq!(
        rec.menus,
        vec![(
            0,
            MiniMenuAction::OP_HELD5,
            PIECE_MOVE,
            0,
            MOVE_BOARD as i32
        )],
        "the board click sends while the bank gate is closed"
    );
    let slot = script_slot(&scripts, "alice").unwrap();
    assert!(
        slot.lock().unwrap().pending_bank_op().is_none(),
        "the second deposit was refused by the pending op, not replaced"
    );
    assert_eq!(
        slot.lock().unwrap().bank_op_result(),
        (1, false),
        "the refused deposit is the only bank result"
    );
}

/// F14 R2: the walk refusal guard protects a published outcome until a
/// snapshot carrying it reaches the isolate. A refused post keeps it; an
/// accepted post of an older outcome keeps it too; only an accepted post
/// of the latest outcome releases it.
#[test]
fn walk_refusal_guard_is_released_only_by_an_accepted_post() {
    let navs = navs_for_test();
    navs.lock().unwrap().insert(
        "alice".into(),
        NavBot {
            walk_outcome_seq: 3,
            walk_live_refusal_id: 9,
            ..Default::default()
        },
    );
    let guard = || navs.lock().unwrap()["alice"].walk_live_refusal_id;
    // The bytes are never decoded for this guard; only acceptance matters.
    let bytes = Vec::new;
    // No isolate takes the post: refused.
    let mut idle = script::SlotScript::new();
    assert!(!super::post_script_snapshot(
        &mut idle,
        &navs,
        "alice",
        3,
        bytes()
    ));
    assert_eq!(guard(), 9, "a refused post leaves the refusal guarded");
    let mut slot = script::SlotScript::new();
    slot.start_load_with_loadouts_settled(
        "export function tick(api) {}".into(),
        script::LoadShape::NativeTick,
        vec![],
        &[],
    )
    .expect("the isolate starts");
    assert!(super::post_script_snapshot(
        &mut slot,
        &navs,
        "alice",
        2,
        bytes()
    ));
    assert_eq!(guard(), 9, "the post carried an older outcome");
    assert!(super::post_script_snapshot(
        &mut slot,
        &navs,
        "alice",
        3,
        bytes()
    ));
    assert_eq!(guard(), 0, "the latest outcome reached the isolate");
    slot.stop();
}

/// A throwaway nav wall for the dispatch-level refuses.
fn navs_for_test() -> Arc<Mutex<HashMap<String, NavBot>>> {
    Arc::new(Mutex::new(HashMap::new()))
}
