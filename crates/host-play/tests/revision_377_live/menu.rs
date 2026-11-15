//! Two real players, normal mouse menus, and read-only runner observations.
use super::*;
use api::snapshot::{MenuEntryQuery, MenuOperation, MenuTargetSelector};
use client::client::MiniMenuAction;
use client::render::Renderer;
use client::util::JString;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use std::io::Write;

// Selected 377 content: pack/loc.pack and maps/m52_51.jm2 (0 52 4).
const CRATE_ID: i32 = 356;
const CRATE_TILE: (i32, i32, i32) = (3380, 3268, 0);
// Outside the fighting rings, inside the selected Duel Arena challenge zone.
const START_TILES: [(i32, i32, i32); 2] = [(3383, 3269, 0), (3383, 3270, 0)];

struct Peer {
    name: String,
    client: Client,
    snapshot: GameSnapshot,
    pump: Pump,
    renderer: Renderer,
}

impl Peer {
    fn new(bound: &Bound377, name: String, offset: i32) -> Result<Self, String> {
        let mut client = bound.template.prepare_client(fresh_uid(offset), false)?;
        client.set_draw(false);
        client.maininit();
        assert_377_client(&client, false)?;
        if client.error_loading {
            return Err(format!(
                "377 asset initialization: {}",
                client.last_progress_message
            ));
        }
        Ok(Self {
            name,
            client,
            snapshot: GameSnapshot::new(),
            pump: Pump::new(),
            renderer: Renderer::new_prefer(false, false),
        })
    }

    fn login(&mut self) -> Result<(), String> {
        if interact::login(
            &mut self.client,
            &self.name,
            &host_play::profile_password(&self.name),
            false,
        ) {
            Ok(())
        } else {
            Err(format!(
                "{} login refused: {} {}",
                self.name, self.client.login_mes1, self.client.login_mes2
            ))
        }
    }

    fn tick(&mut self) {
        self.client.mainloop();
        host::publish_snapshot(
            &mut self.snapshot,
            &self.client,
            self.pump.drain_client(&self.client),
        );
    }

    fn render(&mut self) {
        self.client.set_draw(true);
        drop(self.renderer.mainredraw(&mut self.client));
        self.client.set_draw(false);
    }

    fn state(&self) -> Value {
        json!({
            "account":self.name,
            "tile":self.snapshot.tile(),
            "tick":self.snapshot.tick(),
            "ingame":self.snapshot.ingame(),
            "attached":self.snapshot.attached(),
            "scene_state":self.snapshot.scene_state(),
            "main_modal":self.client.main_modal_id,
            "chat_modal":self.client.chat_modal_id,
            "inventory":self.snapshot.inventory(),
            "stats":self.snapshot.stats(),
            "players":self.client.players.iter().enumerate().filter_map(|(slot, player)| {
                player.as_ref().map(|player| json!({"slot":slot,"name":player.name,"x":player.x,"z":player.z}))
            }).collect::<Vec<_>>(),
            "chat":self.client.chat_text.iter().enumerate().filter(|(_,text)| !text.is_empty()).map(|(i,text)| {
                json!({"type":self.client.chat_type[i],"text":text})
            }).collect::<Vec<_>>(),
        })
    }

    fn player_slot(&self, name: &str) -> Option<usize> {
        let userhash = JString::to_userhash(name);
        self.client.players.iter().position(|player| {
            player
                .as_ref()
                .and_then(|player| player.name.as_deref())
                .is_some_and(|actual| JString::to_userhash(actual) == userhash)
        })
    }

    fn click(&mut self, x: i32, y: i32, button: i32) -> Result<(), String> {
        self.client.shell.apply_mouse_move(x, y);
        self.render();
        self.client.shell.apply_mouse_down(button, x, y);
        self.client.shell.latch_click();
        self.tick();
        self.render();
        self.client.shell.apply_mouse_up();
        self.client.shell.latch_click();
        self.tick();
        self.render();
        if self.client.ingame {
            Ok(())
        } else {
            Err("mouse input ended the session".into())
        }
    }

    fn player_row(&mut self, name: &str, label: &str) -> Result<(i32, i32, usize), String> {
        let slot = self
            .player_slot(name)
            .ok_or_else(|| format!("{} does not see {name}", self.name))?;
        let player = self.client.players[slot]
            .as_ref()
            .ok_or("observed player disappeared")?;
        let (fine_x, fine_z) = (player.x, player.z);
        for height in [100, 50, 150, 0] {
            self.render();
            self.renderer
                .get_overlay_pos(&mut self.client, fine_x, fine_z, height);
            let center = (self.renderer.project_x + 4, self.renderer.project_y + 4);
            for (dx, dy) in [(0, 0), (-6, 0), (6, 0), (0, -8), (0, 8)] {
                let (x, y) = (center.0 + dx, center.1 + dy);
                if !(5..516).contains(&x) || !(5..338).contains(&y) {
                    continue;
                }
                self.client.shell.apply_mouse_move(x, y);
                self.render();
                if let Some(index) = (0..self.client.menu_num_entries as usize).find(|&index| {
                    self.client.menu_param_a[index] == slot as i32
                        && self.client.menu_option[index].starts_with(label)
                }) {
                    return Ok((x, y, index));
                }
            }
        }
        Err(format!(
            "normal pointer over {name} did not expose {label}; state={}",
            self.state()
        ))
    }

    fn send_player_option(&mut self, name: &str, label: &str) -> Result<Value, String> {
        let (x, y, _) = self.player_row(name, label)?;
        self.click(x, y, 2)?;
        if !self.client.is_menu_open {
            return Err("right click did not open the real menu".into());
        }
        let index = (0..self.client.menu_num_entries as usize)
            .find(|&index| self.client.menu_option[index].starts_with(label))
            .ok_or_else(|| format!("open player menu lost {label}"))?;
        let row = raw_row(&self.client, index);
        let (origin_x, origin_y) = match self.client.menu_area {
            0 => (4, 4),
            1 => (553, 205),
            2 => (17, 357),
            _ => return Err("unknown menu area".into()),
        };
        let x = origin_x + self.client.menu_x + 9;
        let y = origin_y
            + self.client.menu_y
            + (self.client.menu_num_entries - 1 - index as i32) * 15
            + 27;
        if self.client.menu_option_at(x, y) != Some(index as i32) {
            return Err("normal menu row coordinate did not hit the selected option".into());
        }
        self.click(x, y, 1)?;
        Ok(
            json!({"sender":self.name,"recipient":name,"input":"right click player, left click real row","row":row}),
        )
    }

    fn chat_row(&mut self, action: i32, sender: &str) -> Option<usize> {
        let sender = JString::to_screen_name(sender).to_ascii_lowercase();
        for y in 358..434 {
            self.client.shell.apply_mouse_move(100, y);
            self.client.build_minimenu();
            if let Some(index) = (0..self.client.menu_num_entries as usize).find(|&index| {
                self.client.menu_action[index] == action
                    && self.client.menu_option[index]
                        .to_ascii_lowercase()
                        .contains(&sender)
            }) {
                return Some(index);
            }
        }
        None
    }

    fn crate_row(&mut self) -> Result<usize, String> {
        let local_x = CRATE_TILE.0 - self.client.map_build_base_x;
        let local_z = CRATE_TILE.1 - self.client.map_build_base_z;
        let scene_type = self.client.world.scene_type(CRATE_TILE.2, local_x, local_z);
        if (scene_type >> 14) & 0x7fff != CRATE_ID {
            return Err(format!(
                "selected content crate is absent at {CRATE_TILE:?}: scene_type={scene_type}"
            ));
        }
        for height in [50, 0, 100] {
            self.render();
            self.renderer.get_overlay_pos(
                &mut self.client,
                local_x * 128 + 64,
                local_z * 128 + 64,
                height,
            );
            let center = (self.renderer.project_x + 4, self.renderer.project_y + 4);
            for dx in [-12, -6, 0, 6, 12] {
                for dy in [-12, -6, 0, 6, 12] {
                    let (x, y) = (center.0 + dx, center.1 + dy);
                    if !(5..516).contains(&x) || !(5..338).contains(&y) {
                        continue;
                    }
                    self.client.shell.apply_mouse_move(x, y);
                    self.render();
                    if let Some(index) = (0..self.client.menu_num_entries as usize).find(|&index| {
                        self.client.menu_action[index] == MiniMenuAction::OP_LOC6
                            && self.client.menu_param_a[index] == CRATE_ID << 14
                            && self.client.menu_param_b[index] == local_x
                            && self.client.menu_param_c[index] == local_z
                    }) {
                        return Ok(index);
                    }
                }
            }
        }
        Err(format!(
            "normal pointer did not pick Crate {CRATE_ID} at {CRATE_TILE:?}"
        ))
    }
}

fn raw_row(client: &Client, index: usize) -> Value {
    json!({"index":index,"action":client.menu_action[index],"params":[client.menu_param_a[index],client.menu_param_b[index],client.menu_param_c[index]],"option":client.menu_option[index]})
}

fn wait_pair(
    peers: &mut [Peer; 2],
    phase: &str,
    mut ready: impl FnMut(&mut [Peer; 2]) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let started = Instant::now();
        for peer in peers.iter_mut() {
            peer.tick();
        }
        if ready(peers) {
            return Ok(());
        }
        if peers.iter().any(|peer| peer.client.error_loading) || Instant::now() >= deadline {
            return Err(format!(
                "{phase}: readiness deadline; states={}",
                json!([peers[0].state(), peers[1].state()])
            ));
        }
        if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn observe(
    peers: &mut [Peer; 2],
    index: usize,
    query: MenuEntryQuery,
    checks: &mut Vec<Value>,
) -> Result<(), String> {
    let peer = &peers[0];
    let decoded = api::snapshot::menu_entries(&peer.client)
        .nth(index)
        .ok_or("observed row is outside decoder prefix")?;
    let query_matches = api::query::menu_has_entry(&peer.client, Some(query));
    let wrong_player_query = matches!(query.target, MenuTargetSelector::Raw(_)).then(|| {
        api::query::menu_has_entry(
            &peer.client,
            Some(MenuEntryQuery {
                operation: query.operation,
                target: MenuTargetSelector::PlayerIndex(0),
            }),
        )
    });
    checks.push(json!({
        "operation":query.operation,"expected_target":format!("{:?}",query.target),
        "raw":raw_row(&peer.client,index),"decoded":decoded,"query_matches":query_matches,
        "incorrect_player_zero_query":wrong_player_query,
        "rows":(0..peer.client.menu_num_entries as usize).map(|i|raw_row(&peer.client,i)).collect::<Vec<_>>(),
        "mouse":[peer.client.shell.mouse_x,peer.client.shell.mouse_y],"state":peer.state(),
    }));
    if !query.matches(decoded) || !query_matches || wrong_player_query == Some(true) {
        return Err(format!(
            "product menu decode mismatch: {}",
            checks.last().unwrap()
        ));
    }
    let proof = Proof::MenuHasEntry {
        operation: query.operation,
        target: query.target,
    };
    let mut runner = ScenarioRunner::new(Scenario {
        name: "recorded-377-menu",
        seed: Seed {
            profiles: vec![("menu", "menu")],
            mainland: false,
        },
        steps: vec![Step {
            name: "observe current real menu",
            kind: StepKind::ObserveMenu,
            wait: Wait {
                arm: proof,
                budget_ticks: 20,
            },
        }],
        proof,
        companions: vec![],
        settings: ScenarioSettings::default(),
    });
    runner.set_live_names(std::slice::from_ref(&peers[0].name));
    runner.set_scene_settle(Duration::ZERO);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let started = Instant::now();
        for peer in peers.iter_mut() {
            peer.tick();
        }
        // Retain the pointer. The client, not this test, rebuilds the rows.
        peers[0].render();
        runner.tick(&mut peers[0].client);
        match runner.status() {
            RunnerStatus::Passed => {
                checks.last_mut().unwrap()["runner_outcome"] = json!("PASS");
                return Ok(());
            }
            RunnerStatus::Failed(reason) => {
                return Err(format!("ObserveMenu failed on real row: {reason}"))
            }
            _ if Instant::now() >= deadline => return Err("ObserveMenu real-row deadline".into()),
            _ => {}
        }
        if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn event(log: &mut fs::File, phase: &str, detail: Value) -> Result<(), String> {
    writeln!(log, "{}", json!({"phase":phase,"detail":detail}))
        .and_then(|()| log.flush())
        .map_err(|error| error.to_string())
}

#[test]
#[ignore = "two fresh 377 accounts, LIVE=1, canonical resources, PARITY_2C_EVIDENCE_DIR and PARITY_2C_SERVED_IDENTITY"]
fn trade_duel_loc_menu_377() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let directory = required_path("PARITY_2C_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let served: Value = serde_json::from_slice(
        &fs::read(required_path("PARITY_2C_SERVED_IDENTITY")?)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let identity = json!({"host":build_stamp::HOST,"client":build_stamp::CLIENT,"served":served,"game_data":api::build_info::game_data_pack(377),"runtime_cache":bound.profile.prepared_cache().map(|cache|cache.identity.content_id_hex())});
    let names = host_play::mint_live_names(2);
    let mut log = fs::File::create(directory.join(format!("{}-menu.jsonl", names[0])))
        .map_err(|error| error.to_string())?;
    event(&mut log, "identity", identity.clone())?;
    let mut peers = [
        Peer::new(&bound, names[0].clone(), 110)?,
        Peer::new(&bound, names[1].clone(), 111)?,
    ];
    let mut checks = Vec::new();
    let mut seeds = Vec::new();
    let mut requests = Vec::new();
    let mut failure_class = "fixture";
    let result = (|| {
        for peer in &mut peers {
            peer.login()?;
        }
        wait_pair(&mut peers, "both fresh logins", |peers| {
            peers.iter().all(|peer| {
                peer.snapshot.ingame()
                    && peer.snapshot.attached()
                    && peer.snapshot.scene_state() == 2
            })
        })?;
        event(
            &mut log,
            "unseeded-account-starts",
            json!([peers[0].state(), peers[1].state()]),
        )?;
        for peer in &mut peers {
            let seed = json!({"account":peer.name,"helper":"api::interact::mainland_hop","scope":"own tutorial progress and own position","before":peer.state()});
            event(&mut log, "seed", seed.clone())?;
            seeds.push(seed);
            interact::mainland_hop(&mut peer.client);
        }
        wait_pair(&mut peers, "both mainland", |peers| {
            peers.iter().all(|peer| {
                peer.snapshot.tile() == Some((3220, 3220, 0)) && peer.snapshot.scene_state() == 2
            })
        })?;
        // A clean relog publishes the mainland appearance after the tutorial fixture.
        for peer in &mut peers {
            let ifaces = Arc::clone(&peer.client.ifaces);
            if !interact::logout(&mut peer.client, ifaces.as_slice()) {
                return Err("mainland clean logout unavailable".into());
            }
        }
        wait_pair(&mut peers, "both logged out", |peers| {
            peers.iter().all(|peer| !peer.client.ingame)
        })?;
        for peer in &mut peers {
            peer.login()?;
        }
        wait_pair(&mut peers, "both clean relogs", |peers| {
            peers.iter().all(|peer| {
                peer.snapshot.ingame()
                    && peer.snapshot.attached()
                    && peer.snapshot.scene_state() == 2
            })
        })?;
        for (peer, tile) in peers.iter_mut().zip(START_TILES) {
            let command = format!(
                "tele {},{},{},{},{}",
                tile.2,
                tile.0 >> 6,
                tile.1 >> 6,
                tile.0 & 63,
                tile.1 & 63
            );
            let seed = json!({"account":peer.name,"command":command,"scope":"own position only","before":peer.state(),"expected_tile":tile});
            event(&mut log, "seed", seed.clone())?;
            seeds.push(seed);
            if !interact::cheat(&mut peer.client, &command).is_sent() {
                return Err(format!("own-position fixture refused: {command}"));
            }
        }
        wait_pair(&mut peers, "both arena positions", |peers| {
            peers.iter().zip(START_TILES).all(|(peer, tile)| {
                peer.snapshot.tile() == Some(tile) && peer.snapshot.scene_state() == 2
            })
        })?;
        wait_pair(&mut peers, "both players visible", |peers| {
            peers[0].player_slot(&names[1]).is_some() && peers[1].player_slot(&names[0]).is_some()
        })?;
        event(
            &mut log,
            "prepared-starts",
            json!([peers[0].state(), peers[1].state()]),
        )?;

        let slot = peers[0].player_slot(&names[1]).ok_or("B disappeared")?;
        if slot == 0 {
            return Err(
                "fixture needs a nonzero B slot to distinguish world-player identity".into(),
            );
        }
        let (_, _, index) = peers[0].player_row(&names[1], "Trade with")?;
        let decoded = api::snapshot::menu_entries(&peers[0].client)
            .nth(index)
            .ok_or("world row absent")?;
        let MenuOperation::PlayerOption(option) = decoded.operation else {
            return Err("world Trade with is not a player operation".into());
        };
        failure_class = "product";
        observe(
            &mut peers,
            index,
            MenuEntryQuery {
                operation: MenuOperation::PlayerOption(option),
                target: MenuTargetSelector::PlayerIndex(slot as i32),
            },
            &mut checks,
        )?;
        event(
            &mut log,
            "world-player-observed",
            checks.last().unwrap().clone(),
        )?;
        failure_class = "fixture";
        let index = peers[0].crate_row()?;
        let (local_x, local_z) = (
            CRATE_TILE.0 - peers[0].client.map_build_base_x,
            CRATE_TILE.1 - peers[0].client.map_build_base_z,
        );
        failure_class = "product";
        observe(
            &mut peers,
            index,
            MenuEntryQuery {
                operation: MenuOperation::LocExamine,
                target: MenuTargetSelector::Loc {
                    typecode: CRATE_ID,
                    x: local_x,
                    z: local_z,
                },
            },
            &mut checks,
        )?;
        event(
            &mut log,
            "loc-examine-observed",
            json!({"content_alias":"crate_old","content_name":"Crate","content_id":CRATE_ID,"world_tile":CRATE_TILE,"observation":checks.last()}),
        )?;

        for (label, action, operation) in [
            (
                "Trade with",
                MiniMenuAction::ACCEPT_TRADEREQ,
                MenuOperation::AcceptTradeRequest,
            ),
            (
                "Challenge",
                MiniMenuAction::ACCEPT_DUELREQ,
                MenuOperation::AcceptDuelRequest,
            ),
        ] {
            failure_class = "fixture";
            let request = peers[1].send_player_option(&names[0], label)?;
            event(&mut log, "normal-player-request", request.clone())?;
            requests.push(request);
            let mut row = None;
            wait_pair(&mut peers, "real chat request", |peers| {
                row = peers[0].chat_row(action, &names[1]);
                row.is_some()
            })?;
            let index = row.ok_or("real chat request row disappeared")?;
            let params = [
                peers[0].client.menu_param_a[index],
                peers[0].client.menu_param_b[index],
                peers[0].client.menu_param_c[index],
            ];
            failure_class = "product";
            observe(
                &mut peers,
                index,
                MenuEntryQuery {
                    operation,
                    target: MenuTargetSelector::Raw(params),
                },
                &mut checks,
            )?;
            event(
                &mut log,
                "chat-accept-observed",
                checks.last().unwrap().clone(),
            )?;
        }
        Ok(())
    })();
    let receipt = json!({
        "cell":"host.menu-377-trade-duel-loc-live","outcome":if result.is_ok(){"PASS"}else{"FAIL"},
        "failure_class":if result.is_ok(){Value::Null}else{json!(failure_class)},"error":result.as_ref().err(),
        "identity":identity,"accounts":names,"seeds":seeds,"requests":requests,"observations":checks,
        "loc":{"alias":"crate_old","name":"Crate","id":CRATE_ID,"tile":CRATE_TILE},
        "end_states":[peers[0].state(),peers[1].state()],
        "normal_input_only":true,"synthetic_menu_or_pick_or_chat_rows":false,
    });
    let path = directory.join(format!("{}-menu.json", names[0]));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    event(&mut log, "verdict", receipt)?;
    for peer in &mut peers {
        if peer.client.ingame {
            peer.client.logout();
        }
    }
    println!(
        "{}",
        json!({"cell":"host.menu-377-trade-duel-loc-live","outcome":if result.is_ok(){"PASS"}else{"FAIL"},"receipt":path})
    );
    result
}
