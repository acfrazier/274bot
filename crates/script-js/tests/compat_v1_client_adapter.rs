//! COMPAT-FINISH-029 ClientAdapter census contracts and removals.

use script::shim::InteractReq;
use script_js::wire::{
    ChatLineInput, ChatOptionInput, ClientAdapterFactsInput, CollisionViewInput, ItemRowInput,
    MainModalTextsInput, MakeButtonInput, MakeProductInput, NativeFactsInput, NpcBoxInput,
    ProjectionSceneInput, PuzzleBoardInput, SceneEntityInput, SideTabIfaceInput, SnapshotInput,
    TileInput, VarpInput, WidgetFactInput, WidgetTextInput,
};
use script_js::{LoadIsolate, LoadShape, Ready};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

mod common;

const INDEX_D_TS: &str = include_str!("../compat-js/index.d.ts");
const MODULES_JSON: &str = include_str!("../compat-js/modules.json");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

const READER_CATALOG_SCRIPT: &str = r#"
import { reader } from '../../adapter/ClientAdapter.js';
import Skill from '../../client/shell/Skill.js';
export default class ReaderCatalog extends LoopingBot {
    loop() {
        if (globalThis.__clientAdapterDone) return;
        globalThis.__clientAdapterDone = true;
        const capture = (read) => {
            try { return read(); }
            catch (error) { return { __error: String(error && error.message || error) }; }
        };
        const modalBefore = capture(() => reader.modalCloseObservation());
        const modalAfter = capture(() => reader.modalCloseObservation());
        globalThis.__clientAdapterCatalog = {
            worldTile: capture(() => reader.worldTile()),
            serverTile: capture(() => reader.serverTile()),
            selfSlot: capture(() => reader.selfSlot()),
            inventorySize: capture(() => reader.inventorySize()),
            inventory: capture(() => reader.inventory()),
            bankSideItems: capture(() => reader.bankSideItems()),
            sceneState: capture(() => reader.sceneState()),
            ingame: capture(() => reader.ingame()),
            npcs: capture(() => reader.npcs()),
            locs: capture(() => reader.locs()),
            players: capture(() => reader.players()),
            groundItems: capture(() => reader.groundItems()),
            equipment: capture(() => reader.equipment()),
            modals: capture(() => reader.modals()),
            chatContinueComId: capture(() => reader.chatContinueComId()),
            chatOptions: capture(() => reader.chatOptions()),
            chatModalTexts: capture(() => reader.chatModalTexts()),
            activeSideTab: capture(() => reader.activeSideTab()),
            localPlayerName: capture(() => reader.localPlayerName()),
            combatLevel: capture(() => reader.combatLevel()),
            selfChat: capture(() => reader.selfChat()),
            hintTile: capture(() => reader.hintTile()),
            npcBox: capture(() => reader.npcBox(7)),
            retaliateControls: capture(() => reader.retaliateControls()),
            inCombat: capture(() => reader.inCombat()),
            selfAnim: capture(() => reader.selfAnim()),
            selfFaceEntity: capture(() => reader.selfFaceEntity()),
            energy: capture(() => reader.energy()),
            varp: capture(() => reader.varp(99)),
            stat: capture(() => reader.stat(25)),
            skillCount: capture(() => reader.skillCount()),
            skillUsed: capture(() => reader.skillUsed(18)),
            statsReady: capture(() => reader.statsReady()),
            weight: capture(() => reader.weight()),
            chat: capture(() => reader.chat(2)),
            playerCount: capture(() => reader.playerCount()),
            npcCount: capture(() => reader.npcCount()),
            cameraYaw: capture(() => reader.cameraYaw()),
            cameraPitch: capture(() => reader.cameraPitch()),
            takingDamage: capture(() => reader.takingDamage()),
            inventorySnapshotReady: capture(() => reader.inventorySnapshotReady()),
            bankComId: capture(() => reader.bankComId()),
            bankSnapshotReady: capture(() => reader.bankSnapshotReady()),
            bankSideSnapshotReady: capture(() => reader.bankSideSnapshotReady()),
            bankSnapshotGeneration: capture(() => reader.bankSnapshotGeneration()),
            runControls: capture(() => reader.runControls()),
            toWorld: capture(() => reader.toWorld(1, 2)),
            collisionFlags: capture(() => ({
                value: reader.collisionFlags(1, 0),
                bounds: [
                    reader.collisionFlags(-1, 0),
                    reader.collisionFlags(104, 0),
                    reader.collisionFlags(0, -1),
                    reader.collisionFlags(0, 104),
                ],
            })),
            lastWalkPathWorld: capture(() => reader.lastWalkPathWorld()),
            loginMessage: capture(() => reader.loginMessage()),
            menuEntries: capture(() => reader.menuEntries()),
            shopInv: capture(() => reader.shopInv(700)),
            shopInvMissing: capture(() => reader.shopInv(-1)),
            puzzleBoardComId: capture(() => reader.puzzleBoardComId()),
            puzzleBoard: capture(() => reader.puzzleBoard()),
            puzzleBoardSize: capture(() => reader.puzzleBoardSize()),
            duelOffers: capture(() => reader.duelOffers()),
            tradeOfferOpen: capture(() => reader.tradeOfferOpen()),
            tradeConfirmOpen: capture(() => reader.tradeConfirmOpen()),
            tradeMyOffer: capture(() => reader.tradeMyOffer()),
            tradeTheirOffer: capture(() => reader.tradeTheirOffer()),
            tradeSidePack: capture(() => reader.tradeSidePack()),
            tradePartner: capture(() => reader.tradePartner()),
            tradeConfirmOffers: capture(() => reader.tradeConfirmOffers()),
            tradeConfirmReady: capture(() => reader.tradeConfirmReady()),
            closeButtonComId: capture(() => reader.closeButtonComId(100)),
            mainModalTexts: capture(() => reader.mainModalTexts()),
            ifModelObjId: capture(() => reader.ifModelObjId(17)),
            modalButtons: capture(() => reader.modalButtons(100)),
            buttonByText: capture(() => reader.buttonByText(100, 'ok')),
            targetButtonByBase: capture(() => reader.targetButtonByBase(100, 'gObLiN')),
            selectButtonByVarp: capture(() => [
                reader.selectButtonByVarp(900, 43, 2),
                reader.selectButtonByVarp(-1, 43, 2),
            ]),
            selectButtonLabelsByVarp: capture(() => reader.selectButtonLabelsByVarp(900, 43)),
            mainModalButtonNearText: capture(() => reader.mainModalButtonNearText('continue')),
            mainSkillMultiItems: capture(() => reader.mainSkillMultiItems()),
            questStatuses: capture(() => reader.questStatuses()),
            sideTabInterface: capture(() => reader.sideTabInterface(6)),
            sideTabInterfaceMissing: capture(() => reader.sideTabInterface(99)),
            attached: capture(() => reader.attached()),
            mapBuildBase: capture(() => reader.mapBuildBase()),
            modalCloseObservation: capture(() => ({
                sessionType: typeof modalBefore.session,
                sameSession: modalBefore.session === modalAfter.session,
                generation: modalBefore.generation,
            })),
            ifText: capture(() => reader.ifText(11)),
            countDialogOpen: capture(() => reader.countDialogOpen()),
            makeProducts: capture(() => reader.makeProducts()),
            toLocal: capture(() => reader.toLocal(3201, 3202)),
            overlayPosWorld: capture(() => reader.overlayPosWorld(3222, 3222)),
            projectAreaGameWorld: capture(() => reader.projectAreaGameWorld(3222, 3222)),
            locBox: capture(() => reader.locBox({ x: 3222, z: 3222, id: 900 })),
            locBoxOutside: capture(() => reader.locBox({ x: 3200, z: 3200, id: 900 })),
            itemIconPixels: capture(() => reader.itemIconPixels(-1)),
            objCatalog: capture(() => reader.objCatalog()),
        };
        globalThis.__clientAdapterSkill = {
            kind: typeof Skill,
            name: Skill.name,
            names: Skill.names,
            used: Skill.used,
            namesFrozen: Object.isFrozen(Skill.names),
            usedFrozen: Object.isFrozen(Skill.used),
            countPresent: Object.prototype.hasOwnProperty.call(Skill, 'count'),
        };
    }
}
"#;

fn item_row<'a>(
    name: Option<&'a str>,
    id: i32,
    count: i32,
    slot: i32,
    component_id: i32,
    ops: &'a [String],
) -> ItemRowInput<'a> {
    ItemRowInput {
        name,
        count,
        id,
        ops,
        noted: false,
        cert: -1,
        component_id,
        slot,
    }
}

fn entity<'a>(
    index: i32,
    id: i32,
    name: &'a str,
    x: i32,
    z: i32,
    size: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id,
        name: Some(name),
        x,
        z,
        level: 0,
        distance: 2,
        health: 7,
        max_health: 11,
        in_combat: false,
        animating: true,
        actions,
        reachable: true,
        reachable_adj: false,
        combat_level: 3,
        target_kind: 0,
        target_index: -1,
        size,
        nx: x,
        nz: z,
        shape: 10,
        angle: 1,
    }
}

fn widget<'a>(component_id: i32, root_component_id: i32, parent_id: i32) -> WidgetFactInput<'a> {
    WidgetFactInput {
        component_id,
        root_component_id,
        parent_id,
        type_: 0,
        button_type: 0,
        client_code: 0,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        hidden: false,
        text: None,
        alternate_text: None,
        button_text: None,
        target_base: None,
        model_type: 0,
        model_id: 0,
        script_opcode: 0,
        script_varp: 0,
        script_value: 0,
        colour: 0,
        inventory_ready: false,
        items: &[],
    }
}

fn reader_catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let inv_ops = ["Use".to_string()];
        let bank_ops = ["Withdraw 1".to_string(), "Withdraw X".to_string()];
        let wear_ops = ["Wield".to_string()];
        let inv = [item_row(Some("Coins"), 995, 17, 0, 3214, &inv_ops)];
        let bank = [item_row(Some("Feather"), 314, 20, 2, 5382, &bank_ops)];
        let side_pack = [item_row(Some("Token"), 995, 2, 1, 300, &inv_ops)];
        let equipment = [item_row(Some("Bronze sword"), 1277, 1, 3, 1688, &wear_ops)];
        let trade_mine = [item_row(Some("Coins"), 995, 10, 0, 3415, &inv_ops)];
        let trade_theirs = [item_row(Some("Feather"), 314, 4, 0, 3416, &inv_ops)];
        let trade_side = [item_row(Some("Air rune"), 556, 20, 0, 3417, &inv_ops)];
        let puzzle_items = [item_row(Some("Red bead"), 1470, 1, 0, 7000, &inv_ops)];
        let shop_items = [item_row(Some("Rope"), 954, 5, 0, 700, &inv_ops)];
        let mine_small = [item_row(Some("Red bead"), 1470, 1, 0, 3542, &inv_ops)];
        let mine_large = [item_row(Some("Coins"), 995, 5, 0, 3538, &inv_ops)];
        let their_small = [item_row(Some("Feather"), 314, 3, 0, 3532, &inv_ops)];
        let their_large = [item_row(Some("Air rune"), 556, 8, 0, 3539, &inv_ops)];
        let main_make = [item_row(Some("Steel bar"), 2353, 1, 0, 3214, &inv_ops)];
        let make_buttons = [
            MakeButtonInput {
                qty: 1,
                com_id: 501,
            },
            MakeButtonInput {
                qty: 5,
                com_id: 502,
            },
            MakeButtonInput {
                qty: 10,
                com_id: 503,
            },
            MakeButtonInput {
                qty: 0,
                com_id: 504,
            },
        ];
        let products = [MakeProductInput {
            object_id: 2353,
            name: "Steel bar",
            buttons: &make_buttons,
        }];
        let npc_actions = ["Attack".to_string()];
        let loc_actions = ["Chop down".to_string()];
        let player_actions = ["Trade with".to_string()];
        let ground_actions = ["Take".to_string()];
        let npcs = [entity(7, 1234, "Goblin", 3224, 3222, 2, &npc_actions)];
        let locs = [entity(4, 900, "Tree", 3223, 3222, 0, &loc_actions)];
        let players = [entity(3, 55, "Friend", 3221, 3222, 0, &player_actions)];
        let ground = [entity(0, 995, "Coins", 3222, 3221, 0, &ground_actions)];
        let chat_options = [
            ChatOptionInput {
                text: "Yes",
                com_id: 401,
            },
            ChatOptionInput {
                text: "No",
                com_id: 402,
            },
        ];
        let chat_lines = [
            ChatLineInput {
                seq: 1,
                text: "Hello",
                type_: 1,
                username: Some("Runner"),
            },
            ChatLineInput {
                seq: 2,
                text: "You found Coins",
                type_: 2,
                username: None,
            },
        ];
        let side_tabs = [
            SideTabIfaceInput { index: 0, id: 900 },
            SideTabIfaceInput { index: 2, id: 200 },
            SideTabIfaceInput { index: 6, id: 6600 },
        ];
        let varps = [VarpInput {
            index: 99,
            value: 17,
        }];
        let inv_item = item_row(Some("Shop rope"), 954, 5, 0, 700, &inv_ops);
        let _ = inv_item;

        let mut main_text = widget(11, 100, 100);
        main_text.type_ = 4;
        main_text.text = Some("Would you like to Continue?");
        main_text.x = 10;
        main_text.y = 10;
        main_text.width = 80;
        main_text.height = 20;

        let mut target_button = widget(12, 100, 100);
        target_button.button_type = 2;
        target_button.target_base = Some("Goblin");
        target_button.text = Some("Attack");
        target_button.x = 20;
        target_button.y = 40;
        target_button.width = 20;
        target_button.height = 10;

        let mut ok_button = widget(13, 100, 100);
        ok_button.button_type = 1;
        ok_button.button_text = Some("OK");
        ok_button.text = Some("Continue");
        ok_button.x = 45;
        ok_button.y = 40;
        ok_button.width = 20;
        ok_button.height = 10;

        let mut close_button = widget(14, 100, 100);
        close_button.button_type = 3;
        close_button.button_text = Some("Close");

        let mut model = widget(17, 100, 100);
        model.model_type = 4;
        model.model_id = 415;

        let mut accurate_button = widget(18, 900, 901);
        accurate_button.button_type = 5;
        accurate_button.script_opcode = 5;
        accurate_button.script_varp = 43;
        accurate_button.script_value = 1;
        accurate_button.x = 10;
        accurate_button.y = 300;
        accurate_button.width = 20;
        accurate_button.height = 20;

        let mut accurate_label = widget(19, 900, 901);
        accurate_label.type_ = 4;
        accurate_label.text = Some("(Accurate)");
        accurate_label.y = 300;
        accurate_label.height = 20;

        let mut aggressive_button = widget(20, 900, 901);
        aggressive_button.button_type = 5;
        aggressive_button.script_opcode = 5;
        aggressive_button.script_varp = 43;
        aggressive_button.script_value = 2;
        aggressive_button.x = 10;
        aggressive_button.y = 330;
        aggressive_button.width = 20;
        aggressive_button.height = 20;

        let mut aggressive_label = widget(21, 900, 901);
        aggressive_label.type_ = 4;
        aggressive_label.text = Some("(Aggressive)");
        aggressive_label.y = 330;
        aggressive_label.height = 20;
        let mut punch_button = widget(23, 900, 901);
        punch_button.button_type = 5;
        punch_button.script_opcode = 5;
        punch_button.script_varp = 43;
        punch_button.script_value = 3;
        punch_button.x = 10;
        punch_button.y = 360;
        punch_button.width = 20;
        punch_button.height = 20;

        let mut punch_label = widget(24, 900, 901);
        punch_label.type_ = 4;
        punch_label.text = Some("Punch");
        punch_label.y = 360;
        punch_label.height = 20;

        let mut pause_button = widget(22, 100, 100);
        pause_button.button_type = 6;
        pause_button.button_text = Some("Pause");
        pause_button.text = Some("Pause");

        let mut shop_widget = widget(700, 500, 500);
        shop_widget.items = &shop_items;

        let mut mine_small_widget = widget(3542, 600, 600);
        mine_small_widget.items = &mine_small;
        let mut mine_large_widget = widget(3538, 600, 600);
        mine_large_widget.items = &mine_large;
        let mut their_small_widget = widget(3532, 600, 600);
        their_small_widget.items = &their_small;
        let mut their_large_widget = widget(3539, 600, 600);
        their_large_widget.items = &their_large;

        let mut quest_text = widget(50, 200, 200);
        quest_text.type_ = 4;
        quest_text.text = Some("Dragon Slayer");
        quest_text.colour = 0x55ff55;

        let widgets = [
            main_text,
            target_button,
            ok_button,
            close_button,
            model,
            accurate_button,
            accurate_label,
            aggressive_button,
            aggressive_label,
            punch_button,
            punch_label,
            pause_button,
            shop_widget,
            mine_small_widget,
            mine_large_widget,
            their_small_widget,
            their_large_widget,
            quest_text,
        ];

        let widget_texts = [WidgetTextInput {
            component_id: 11,
            text: "Would you like to Continue?",
            item_count: -1,
        }];
        let main_texts = [
            "Would you like to Continue?".to_string(),
            "Choose an option".to_string(),
        ];
        let chat_texts = ["Continue dialog".to_string(), "Pick a choice".to_string()];
        let npc_boxes = [NpcBoxInput {
            index: 7,
            points: [
                (1, 2),
                (3, 4),
                (5, 6),
                (7, 8),
                (9, 10),
                (11, 12),
                (13, 14),
                (15, 16),
            ],
        }];
        let mut snapshot: SnapshotInput<'_> = common::ingame_snapshot();
        snapshot.here = Some(TileInput {
            x: 3222,
            z: 3222,
            level: 0,
        });
        snapshot.widgets = &widget_texts;
        snapshot.inv = &inv;
        snapshot.inv_size = 28;
        snapshot.bank = &bank;
        snapshot.bank_side = &side_pack;
        snapshot.bank_open = true;
        snapshot.bank_loaded = true;
        snapshot.bank_generation = 9;
        snapshot.count_dialog_open = true;
        snapshot.npcs = &npcs;
        snapshot.locs = &locs;
        snapshot.players = &players;
        snapshot.ground = &ground;
        snapshot.equipment = &equipment;
        snapshot.chat_options = &chat_options;
        snapshot.side_tab = 3;
        snapshot.varps = &varps;
        snapshot.run_energy = 42;
        snapshot.my_name = Some("Runner");
        snapshot.in_combat = true;
        snapshot.animating = true;
        snapshot.main_modal_id = 100;
        snapshot.chat_modal_id = 200;
        snapshot.make_products = &products;
        snapshot.side_tab_ifaces = &side_tabs;
        snapshot.chat_lines = &chat_lines;
        snapshot.scene_state = 2;
        snapshot.weight = 12;
        snapshot.combat_level = 15;
        snapshot.camera_yaw = 2049;
        snapshot.camera_pitch = 512;
        snapshot.self_slot = 7;
        snapshot.trade_offer_open = true;
        snapshot.trade_confirm_open = true;
        snapshot.trade_partner = Some("Alice");
        snapshot.trade_mine = &trade_mine;
        snapshot.trade_theirs = &trade_theirs;
        snapshot.trade_side = &trade_side;
        snapshot.trade_accept_id = 3418;
        snapshot.trade_decline_id = 3419;
        snapshot.shop_open = true;
        snapshot.shop_stock = &shop_items;
        snapshot.self_target_kind = 2;
        snapshot.self_target_index = 3;

        let mut heights = vec![0_i32; 105 * 105];
        let heights_above = Vec::<i32>::new();
        let link_below = vec![0_u8; 104 * 104];
        let projection_locs = [api::snapshot::ProjectionLocView {
            id: 900,
            name: Some(std::sync::Arc::<str>::from("Tree")),
            level: 0,
            rank: 1,
            scene_x: 22 * 128 + 64,
            scene_z: 22 * 128 + 64,
            min_tile_x: 22,
            max_tile_x: 22,
            min_tile_z: 22,
            max_tile_z: 22,
            half_width: 64,
            half_length: 64,
            top_height: 128,
        }];
        let projection = api::snapshot::ProjectionSceneView {
            available: true,
            level: 0,
            camera_x: 0,
            camera_y: 0,
            camera_z: 0,
            sin_pitch: 0,
            cos_pitch: 65_536,
            sin_yaw: 0,
            cos_yaw: 65_536,
            heights: std::mem::take(&mut heights).into(),
            heights_above: heights_above.into(),
            link_below: link_below.into(),
            locs: projection_locs.into(),
        };
        let mut collision_flags = vec![0; 104 * 104];
        collision_flags[104] = 3;
        let client = ClientAdapterFactsInput {
            attached: true,
            map_build_base: (3200, 3200),
            server_tile: Some(TileInput {
                x: 3111,
                z: 3222,
                level: 1,
            }),
            hint_tile_level: Some(0),
            bank_com_id: 5382,
            chat_continue_component_id: 3012,
            chat_modal_texts: MainModalTextsInput {
                root: 200,
                texts: &chat_texts,
            },
            widget_facts: &widgets,
            player_count: 2,
            npc_count: 1,
            run_controls: Some((91, 92)),
            login_message: "Welcome back",
            menu_entries: &["Walk here", "Examine"],
            last_walk_path_local: &[(1, 2), (2, 2)],
            session_generation: 7,
            modal_close_generation: 3,
            projection_scene: Some(ProjectionSceneInput { view: &projection }),
        };
        let native = NativeFactsInput {
            stats_ready: true,
            inv_ready: true,
            bank_side_ready: true,
            self_chat: Some("hello"),
            hint_tile: Some((3224, 3225)),
            retaliate_controls: Some((51, 52)),
            main_modal_texts: Some(MainModalTextsInput {
                root: 100,
                texts: &main_texts,
            }),
            puzzle_board: Some(PuzzleBoardInput {
                component_id: 7000,
                size: 5,
                items: &puzzle_items,
                generation: 7,
            }),
            npc_boxes: Some(&npc_boxes),
            main_make: Some(&main_make),
            collision: Some(CollisionViewInput {
                available: true,
                base_x: 3200,
                base_z: 3200,
                level: 0,
                width: 104,
                height: 104,
                flags: &collision_flags,
            }),
            bank_snapshot_generation: Some(9),
            side_modal_id: Some(300),
            client_adapter: Some(client),
            taking_damage: Some(true),
            ..NativeFactsInput::default()
        };
        let isolate = LoadIsolate::spawn(
            READER_CATALOG_SCRIPT.to_string(),
            LoadShape::CompatClass,
            vec![],
        )
        .expect("load ClientAdapter catalog probe");
        isolate.post_snapshot(script_js::wire::encode_snapshot_with_native(
            &snapshot, native,
        ));
        isolate.on_game_tick(1);
        let _ = isolate.probe("true");
        let catalog = isolate
            .probe("__clientAdapterCatalog")
            .expect("reader catalog");
        let loc_projection_wanted = isolate.loc_projection_wanted_for_test();
        let skill = isolate
            .probe("__clientAdapterSkill")
            .expect("Skill catalog");
        isolate.join();
        let mut all = serde_json::Map::new();
        all.insert("reader".to_string(), catalog);
        all.insert("skill".to_string(), skill);
        all.insert(
            "locProjectionWanted".to_string(),
            Value::Bool(loc_projection_wanted),
        );
        Value::Object(all)
    })
}

fn checked<'a>(value: &'a Value, label: &str) -> &'a Value {
    assert!(value.get("__error").is_none(), "{label} failed: {value}");
    value
}

macro_rules! reader_contract {
    ($name:ident, $key:literal, $value:ident, $assertions:block) => {
        #[test]
        #[expect(non_snake_case, reason = "the census names this contract test")]
        fn $name() {
            let catalog = reader_catalog();
            let $value = checked(&catalog["reader"][$key], $key);
            $assertions
        }
    };
}
macro_rules! skill_contract {
    ($name:ident, $key:literal, $value:ident, $assertions:block) => {
        #[test]
        #[expect(non_snake_case, reason = "the census names this contract test")]
        fn $name() {
            let catalog = reader_catalog();
            let $value = checked(&catalog["skill"], $key);
            $assertions
        }
    };
}

fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !ident(before) && !ident(after)
    })
}
fn client_adapter_declarations(text: &str) -> &str {
    const MODULE: &str = "declare module '*adapter/ClientAdapter.js' {";
    let start = text.find(MODULE).expect("ClientAdapter declaration");
    let body = &text[start + MODULE.len()..];
    let end = body.find("\n}\n").expect("ClientAdapter declaration end");
    &body[..end]
}

fn bot(imports: &str, body: &str) -> String {
    format!("{imports}\nexport default class T extends LoopingBot {{ loop() {{ {body} }} }}\n")
}

fn run(src: &str, expr: &str) -> Value {
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let value = iso.probe(expr).unwrap();
    iso.join();
    value
}

fn assert_client_adapter_siblings() {
    let value = run(
        &bot(
            "import { reader, actions } from '../../adapter/ClientAdapter.js';",
            "globalThis.__ok = typeof reader.worldTile === 'function' && typeof reader.selfFaceEntity === 'function' && typeof reader.bankComId === 'function' && typeof actions.sayPublic === 'function' && typeof actions.continueDialog === 'function';",
        ),
        "__ok",
    );
    assert_eq!(
        value, true,
        "supported ClientAdapter siblings stay available"
    );
}

fn assert_export_removed(name: &str) {
    let src = bot(
        &format!("import {{ {name} }} from '../../adapter/ClientAdapter.js';"),
        "",
    );
    let iso = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        match iso.poll_ready() {
            Ready::Failed(error) => break error,
            Ready::Ready => {
                iso.join();
                panic!("importing removed {name} linked");
            }
            Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ready::Pending => panic!("isolate setup timed out importing removed {name}"),
        }
    };
    iso.join();
    assert!(
        error.contains(name),
        "link failure names removed {name}: {error}"
    );
    for (file, text) in [
        ("index.d.ts", INDEX_D_TS),
        ("modules.json", MODULES_JSON),
        ("abi-contract.json", ABI_CONTRACT),
    ] {
        let declarations = if file == "index.d.ts" {
            client_adapter_declarations(text)
        } else {
            text
        };
        assert!(
            !mentions(declarations, name),
            "{file} still promises removed {name}"
        );
    }
    assert_client_adapter_siblings();
}

macro_rules! removed_export {
    ($name:ident, $export:literal) => {
        #[test]
        #[expect(non_snake_case, reason = "the census names this removal test")]
        fn $name() {
            assert_export_removed($export);
        }
    };
}

reader_contract!(
    compat_v1_client_adapter__reader_worldtile__contract,
    "worldTile",
    value,
    {
        assert_eq!(value, &json!({"x": 3222, "z": 3222, "level": 0}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_servertile__contract,
    "serverTile",
    value,
    {
        assert_eq!(value, &json!({"x": 3111, "z": 3222, "level": 1}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_selfslot__contract,
    "selfSlot",
    value,
    {
        assert_eq!(value, 7);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_inventorysize__contract,
    "inventorySize",
    value,
    {
        assert_eq!(value, 28);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_inventory__contract,
    "inventory",
    value,
    {
        assert_eq!(value[0]["slot"], 0);
        assert_eq!(value[0]["id"], 995);
        assert_eq!(value[0]["name"], "Coins");
        assert_eq!(value[0]["count"], 17);
        assert_eq!(value[0]["comId"], 3214);
        assert_eq!(value[0]["ops"], json!(["Use"]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_banksideitems__contract,
    "bankSideItems",
    value,
    {
        assert_eq!(value[0]["id"], 995);
        assert_eq!(value[0]["name"], "Token");
        assert_eq!(value[0]["count"], 2);
        assert_eq!(value[0]["slot"], 1);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_scenestate__contract,
    "sceneState",
    value,
    {
        assert_eq!(value, 2);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_ingame__contract,
    "ingame",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_npcs__contract,
    "npcs",
    value,
    {
        assert_eq!(value[0]["id"], 1234);
        assert_eq!(value[0]["name"], "Goblin");
        assert_eq!(value[0]["network_x"], 3225);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_locs__contract,
    "locs",
    value,
    {
        assert_eq!(value[0]["id"], 900);
        assert_eq!(value[0]["name"], "Tree");
        assert_eq!(value[0]["actions"], json!(["Chop down"]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_players__contract,
    "players",
    value,
    {
        assert_eq!(value[0]["index"], 3);
        assert_eq!(value[0]["name"], "Friend");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_grounditems__contract,
    "groundItems",
    value,
    {
        assert_eq!(value[0]["id"], 995);
        assert_eq!(value[0]["name"], "Coins");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_equipment__contract,
    "equipment",
    value,
    {
        assert_eq!(value[0]["id"], 1277);
        assert_eq!(value[0]["name"], "Bronze sword");
        assert_eq!(value[0]["slot"], 3);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_modals__contract,
    "modals",
    value,
    {
        assert_eq!(value, &json!({"main": 100, "chat": 200, "side": 300}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_chatcontinuecomid__contract,
    "chatContinueComId",
    value,
    {
        assert_eq!(value, 3012);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_chatoptions__contract,
    "chatOptions",
    value,
    {
        assert_eq!(
            value,
            &json!([{"text": "Yes", "comId": 401}, {"text": "No", "comId": 402}])
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_chatmodaltexts__contract,
    "chatModalTexts",
    value,
    {
        assert_eq!(value, &json!(["Continue dialog", "Pick a choice"]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_activesidetab__contract,
    "activeSideTab",
    value,
    {
        assert_eq!(value, 3);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_localplayername__contract,
    "localPlayerName",
    value,
    {
        assert_eq!(value, "Runner");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_combatlevel__contract,
    "combatLevel",
    value,
    {
        assert_eq!(value, 15);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_selfchat__contract,
    "selfChat",
    value,
    {
        assert_eq!(value, "hello");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_hinttile__contract,
    "hintTile",
    value,
    {
        assert_eq!(value, &json!({"x": 3224, "z": 3225, "level": 0}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_npcbox__contract,
    "npcBox",
    value,
    {
        assert_eq!(value.as_array().map(Vec::len), Some(8));
        assert_eq!(value[0], json!({"x": 1, "y": 2}));
        assert_eq!(value[7], json!({"x": 15, "y": 16}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_retaliatecontrols__contract,
    "retaliateControls",
    value,
    {
        assert_eq!(value, &json!({"onComId": 51, "offComId": 52}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_incombat__contract,
    "inCombat",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_selfanim__contract,
    "selfAnim",
    value,
    {
        assert_eq!(value, -1, "missing posted animation is the absent sentinel");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_selffaceentity__contract,
    "selfFaceEntity",
    value,
    {
        assert_eq!(value, 32771);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_energy__contract,
    "energy",
    value,
    {
        assert_eq!(value, 42);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_varp__contract,
    "varp",
    value,
    {
        assert_eq!(value, 17);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_stat__contract,
    "stat",
    value,
    {
        assert_eq!(
            value,
            &json!({"name": "#25", "effective": 0, "base": 0, "xp": 0})
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_skillcount__contract,
    "skillCount",
    value,
    {
        assert_eq!(value, 25);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_skillused__contract,
    "skillUsed",
    value,
    {
        assert_eq!(value, false);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_statsready__contract,
    "statsReady",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_weight__contract,
    "weight",
    value,
    {
        assert_eq!(value, 12);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_chat__contract,
    "chat",
    value,
    {
        assert_eq!(value.as_array().map(Vec::len), Some(2));
        assert_eq!(
            value[0],
            json!({"type": 1, "username": "Runner", "text": "Hello"})
        );
        assert_eq!(
            value[1],
            json!({"type": 2, "username": null, "text": "You found Coins"})
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_playercount__contract,
    "playerCount",
    value,
    {
        assert_eq!(value, 2);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_npccount__contract,
    "npcCount",
    value,
    {
        assert_eq!(value, 1);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_camerayaw__contract,
    "cameraYaw",
    value,
    {
        assert_eq!(value, 1);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_camerapitch__contract,
    "cameraPitch",
    value,
    {
        assert_eq!(value, 512);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_takingdamage__contract,
    "takingDamage",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_inventorysnapshotready__contract,
    "inventorySnapshotReady",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_bankcomid__contract,
    "bankComId",
    value,
    {
        assert_eq!(value, 5382);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_banksnapshotready__contract,
    "bankSnapshotReady",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_banksidesnapshotready__contract,
    "bankSideSnapshotReady",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_banksnapshotgeneration__contract,
    "bankSnapshotGeneration",
    value,
    {
        assert_eq!(value, 9);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_runcontrols__contract,
    "runControls",
    value,
    {
        assert_eq!(value, &json!({"onComId": 91, "offComId": 92}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_toworld__contract,
    "toWorld",
    value,
    {
        assert_eq!(value, &json!({"x": 3201, "z": 3202, "level": 0}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_collisionflags__contract,
    "collisionFlags",
    value,
    {
        assert_eq!(value["value"], 3);
        assert_eq!(value["bounds"], json!([null, null, null, null]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_lastwalkpathworld__contract,
    "lastWalkPathWorld",
    value,
    {
        assert_eq!(
            value,
            &json!([
                {"x": 3201, "z": 3202, "level": 1},
                {"x": 3202, "z": 3202, "level": 1}
            ])
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_loginmessage__contract,
    "loginMessage",
    value,
    {
        assert_eq!(value, "Welcome back");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_menuentries__contract,
    "menuEntries",
    value,
    {
        assert_eq!(value, &json!(["Walk here", "Examine"]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_shopinv__contract,
    "shopInv",
    value,
    {
        assert_eq!(value[0]["id"], 954);
        assert_eq!(value[0]["name"], "Rope");
        assert_eq!(value[0]["count"], 5);
        let missing = checked(
            &reader_catalog()["reader"]["shopInvMissing"],
            "shopInvMissing",
        );
        assert_eq!(missing, &json!([]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_puzzleboardcomid__contract,
    "puzzleBoardComId",
    value,
    {
        assert_eq!(value, 7000);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_puzzleboard__contract,
    "puzzleBoard",
    value,
    {
        assert_eq!(value[0]["id"], 1470);
        assert_eq!(value[0]["name"], "Red bead");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_puzzleboardsize__contract,
    "puzzleBoardSize",
    value,
    {
        assert_eq!(value, 5);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_dueloffers__contract,
    "duelOffers",
    value,
    {
        assert_eq!(value, &json!({"ready": false, "mine": [], "theirs": []}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tradeofferopen__contract,
    "tradeOfferOpen",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tradeconfirmopen__contract,
    "tradeConfirmOpen",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_trademyoffer__contract,
    "tradeMyOffer",
    value,
    {
        assert_eq!(value[0]["id"], 995);
        assert_eq!(value[0]["count"], 10);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tradetheiroffer__contract,
    "tradeTheirOffer",
    value,
    {
        assert_eq!(value[0]["id"], 314);
        assert_eq!(value[0]["count"], 4);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tradesidepack__contract,
    "tradeSidePack",
    value,
    {
        assert_eq!(value[0]["id"], 556);
        assert_eq!(value[0]["name"], "Air rune");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tradepartner__contract,
    "tradePartner",
    value,
    {
        assert_eq!(value, "Alice");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tradeconfirmoffers__contract,
    "tradeConfirmOffers",
    value,
    {
        assert_eq!(value["mine"][0]["id"], 1470);
        assert_eq!(value["mine"][1]["id"], 995);
        assert_eq!(value["theirs"][0]["id"], 314);
        assert_eq!(value["theirs"][1]["id"], 556);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_closebuttoncomid__contract,
    "closeButtonComId",
    value,
    {
        assert_eq!(value, 14);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_mainmodaltexts__contract,
    "mainModalTexts",
    value,
    {
        assert_eq!(
            value,
            &json!(["Would you like to Continue?", "Choose an option"])
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_ifmodelobjid__contract,
    "ifModelObjId",
    value,
    {
        assert_eq!(value, 415);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_modalbuttons__contract,
    "modalButtons",
    value,
    {
        let buttons = value.as_array().expect("button array");
        assert!(buttons
            .iter()
            .any(|button| button["comId"] == 13 && button["menu"] == "OK"));
        assert!(
            !buttons.iter().any(|button| button["comId"] == 14),
            "close is not a modal action button"
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_buttonbytext__contract,
    "buttonByText",
    value,
    {
        assert_eq!(value, 13);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_targetbuttonbybase__contract,
    "targetButtonByBase",
    value,
    {
        assert_eq!(value, 12);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_selectbuttonbyvarp__contract,
    "selectButtonByVarp",
    value,
    {
        assert_eq!(value, &json!([20, 20]));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_selectbuttonlabelsbyvarp__contract,
    "selectButtonLabelsByVarp",
    value,
    {
        assert_eq!(
            value,
            &json!([{"mode": 1, "label": "(Accurate)"}, {"mode": 2, "label": "(Aggressive)"}])
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_mainmodalbuttonneartext__contract,
    "mainModalButtonNearText",
    value,
    {
        assert_eq!(value, 13);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_mainskillmultiitems__contract,
    "mainSkillMultiItems",
    value,
    {
        assert_eq!(value[0]["id"], 2353);
        assert_eq!(value[0]["name"], "Steel bar");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_queststatuses__contract,
    "questStatuses",
    value,
    {
        assert_eq!(
            value,
            &json!([{"comId": 50, "name": "Dragon Slayer", "colour": 0x55ff55}])
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_sidetabinterface__contract,
    "sideTabInterface",
    value,
    {
        assert_eq!(value, 6600);
        let missing = checked(
            &reader_catalog()["reader"]["sideTabInterfaceMissing"],
            "sideTabInterfaceMissing",
        );
        assert_eq!(missing, -1);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_attached__contract,
    "attached",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_mapbuildbase__contract,
    "mapBuildBase",
    value,
    {
        assert_eq!(value, &json!({"x": 3200, "z": 3200}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_modalcloseobservation__contract,
    "modalCloseObservation",
    value,
    {
        assert_eq!(
            value,
            &json!({"sessionType": "symbol", "sameSession": true, "generation": 3})
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_iftext__contract,
    "ifText",
    value,
    {
        assert_eq!(value, "Would you like to Continue?");
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_countdialogopen__contract,
    "countDialogOpen",
    value,
    {
        assert_eq!(value, true);
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_makeproducts__contract,
    "makeProducts",
    value,
    {
        assert_eq!(
            value,
            &json!([{"obj": 2353, "name": "Steel bar", "buttons": [
                {"qty": 1, "comId": 501}, {"qty": 5, "comId": 502},
                {"qty": 10, "comId": 503}, {"qty": 0, "comId": 504}
            ]}])
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_tolocal__contract,
    "toLocal",
    value,
    {
        assert_eq!(value, &json!({"lx": 1, "lz": 2}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_overlayposworld__contract,
    "overlayPosWorld",
    value,
    {
        assert_eq!(value, &json!({"x": 772, "y": 171}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_projectareagameworld__contract,
    "projectAreaGameWorld",
    value,
    {
        assert_eq!(value, &json!({"x": 768, "y": 167}));
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_locbox__contract,
    "locBox",
    value,
    {
        assert_eq!(value.as_array().map(Vec::len), Some(8));
        assert!(checked(
            &reader_catalog()["reader"]["locBoxOutside"],
            "locBoxOutside"
        )
        .is_null());
        assert_eq!(
            reader_catalog()["locProjectionWanted"],
            Value::Bool(true),
            "the native reader must publish demand on the first locBox call"
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_itemiconpixels__contract,
    "itemIconPixels",
    value,
    {
        assert!(
            value.is_null(),
            "unknown item icon has no fabricated pixel data"
        );
    }
);
reader_contract!(
    compat_v1_client_adapter__reader_objcatalog__contract,
    "objCatalog",
    value,
    {
        assert!(
            value.is_array(),
            "object catalog is a synchronous array result"
        );
    }
);

skill_contract!(
    compat_v1_client_adapter__default__contract,
    "skill",
    value,
    {
        assert_eq!(value["kind"], "function");
        assert_eq!(value["name"], "Skill");
        assert_eq!(value["names"].as_array().map(Vec::len), Some(25));
        assert_eq!(value["used"].as_array().map(Vec::len), Some(25));
        assert_eq!(
            value["countPresent"], false,
            "uncalled Skill.count is not exported"
        );
    }
);
skill_contract!(
    compat_v1_client_adapter__skill_names__contract,
    "skill",
    value,
    {
        assert_eq!(
            value["names"],
            json!([
                "attack",
                "defence",
                "strength",
                "hitpoints",
                "ranged",
                "prayer",
                "magic",
                "cooking",
                "woodcutting",
                "fletching",
                "fishing",
                "firemaking",
                "crafting",
                "smithing",
                "mining",
                "herblore",
                "agility",
                "thieving",
                "slayer",
                "-unused-",
                "runecraft",
                "-unused-",
                "-unused-",
                "-unused-",
                "-unused-"
            ])
        );
        assert_eq!(value["namesFrozen"], true);
    }
);
skill_contract!(
    compat_v1_client_adapter__skill_used__contract,
    "skill",
    value,
    {
        assert_eq!(
            value["used"],
            json!([
                true, true, true, true, true, true, true, true, true, true, true, true, true, true,
                true, true, true, true, false, false, true, false, false, false, false
            ])
        );
        assert_eq!(value["usedFrozen"], true);
    }
);

removed_export!(
    compat_v1_client_adapter__invalidatelocsnapshots__removed,
    "invalidateLocSnapshots"
);
removed_export!(compat_v1_client_adapter__detach__removed, "detach");
removed_export!(
    compat_v1_client_adapter__activestatsready__removed,
    "activeStatsReady"
);
removed_export!(
    compat_v1_client_adapter__currentloginstatsready__removed,
    "currentLoginStatsReady"
);
removed_export!(
    compat_v1_client_adapter__lochullheight__removed,
    "locHullHeight"
);
removed_export!(
    compat_v1_client_adapter__resolvelocmodelextents__removed,
    "resolveLocModelExtents"
);
removed_export!(compat_v1_client_adapter__attach__removed, "attach");
removed_export!(
    compat_v1_client_adapter__setpacketlistener__removed,
    "setPacketListener"
);
removed_export!(
    compat_v1_client_adapter__resetobjcatalog__removed,
    "resetObjCatalog"
);
removed_export!(
    compat_v1_client_adapter__readretaliatecontrols__removed,
    "readRetaliateControls"
);
removed_export!(
    compat_v1_client_adapter__readmodalbuttons__removed,
    "readModalButtons"
);
removed_export!(
    compat_v1_client_adapter__readselectbuttonlabelsbyvarp__removed,
    "readSelectButtonLabelsByVarp"
);
removed_export!(
    compat_v1_client_adapter__combatshowingthreshold__removed,
    "combatShowingThreshold"
);
removed_export!(
    compat_v1_client_adapter__welcome_screen__removed,
    "WELCOME_SCREEN"
);

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_client_adapter__reader_selftarget__removed() {
    let value = run(
        &bot(
            "import { reader } from '../../adapter/ClientAdapter.js';",
            r#"
const trap = (key) => { try { reader[key]; return 'read'; } catch (error) { return error.message; } };
globalThis.__surface = {
    own: Object.prototype.hasOwnProperty.call(reader, 'selfTarget'),
    descriptor: Object.getOwnPropertyDescriptor(reader, 'selfTarget') === undefined,
    keys: Object.keys(reader).includes('selfTarget'),
    removed: trap('selfTarget'),
    unknown: trap('__noSuchClientAdapterMember'),
    siblings: typeof reader.selfFaceEntity === 'function' && typeof reader.npcs === 'function',
};
"#,
        ),
        "__surface",
    );
    assert_eq!(value["own"], false);
    assert_eq!(value["descriptor"], true);
    assert_eq!(value["keys"], false);
    assert_eq!(value["siblings"], true);
    assert_eq!(
        value["removed"]
            .as_str()
            .map(|message| message.replace("selfTarget", "<key>")),
        value["unknown"]
            .as_str()
            .map(|message| message.replace("__noSuchClientAdapterMember", "<key>")),
        "removed reader member uses the generic unknown-member trap"
    );
    for (file, text) in [
        ("index.d.ts", INDEX_D_TS),
        ("modules.json", MODULES_JSON),
        ("abi-contract.json", ABI_CONTRACT),
    ] {
        assert!(!mentions(
            if file == "index.d.ts" {
                client_adapter_declarations(text)
            } else {
                text
            },
            "selfTarget"
        ),);
    }
}

fn client_facts<'a>(attached: bool, continue_id: i32) -> ClientAdapterFactsInput<'a> {
    ClientAdapterFactsInput {
        attached,
        map_build_base: (3200, 3200),
        server_tile: None,
        hint_tile_level: None,
        bank_com_id: -1,
        chat_continue_component_id: continue_id,
        chat_modal_texts: MainModalTextsInput {
            root: -1,
            texts: &[],
        },
        widget_facts: &[],
        player_count: 0,
        npc_count: 0,
        run_controls: None,
        login_message: "",
        menu_entries: &[],
        last_walk_path_local: &[],
        session_generation: 1,
        modal_close_generation: 0,
        projection_scene: None,
    }
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_client_adapter__actions_saypublic__contract() {
    let iso = LoadIsolate::spawn(
        bot(
            "import { actions } from '../../adapter/ClientAdapter.js'; globalThis.__sayValues = ['  hello  ', '   ', 23]; globalThis.__sayResults = [];",
            "for (const value of globalThis.__sayValues.splice(0)) { globalThis.__sayResults.push(actions.sayPublic(value)); }",
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let snapshot: SnapshotInput<'_> = common::ingame_snapshot();
    let facts = client_facts(true, -1);
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &snapshot,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__sayResults").unwrap(),
        json!([true, false, true])
    );
    assert_eq!(
        iso.drain_interacts(),
        vec![
            InteractReq::SayPublic {
                text: "hello".to_string()
            },
            InteractReq::SayPublic {
                text: "23".to_string()
            },
        ],
        "coerced nonempty messages queue one typed public-chat action; blank text queues none"
    );

    let _ = iso.probe("globalThis.__sayValues = ['detached']");
    let facts = client_facts(false, -1);
    let mut next = snapshot;
    next.tick = 2;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__sayResults").unwrap(),
        json!([true, false, true, false])
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "detached session queues no public chat"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_client_adapter__actions_continuedialog__contract() {
    let iso = LoadIsolate::spawn(
        bot(
            "import { actions } from '../../adapter/ClientAdapter.js'; globalThis.__continueRequests = 1; globalThis.__continueResults = [];",
            "if (globalThis.__continueRequests > 0) { globalThis.__continueRequests--; globalThis.__continueResults.push(actions.continueDialog()); }",
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let snapshot: SnapshotInput<'_> = common::ingame_snapshot();
    let facts = client_facts(true, 3012);
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &snapshot,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__continueResults").unwrap(), json!([true]));
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::ContinueDialog {
            component_id: Some(3012)
        }]
    );

    let facts = client_facts(true, -1);
    let mut next = snapshot;
    next.tick = 2;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    let _ = iso.probe("globalThis.__continueRequests = 1");
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__continueResults").unwrap(),
        json!([true, false])
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "missing continue component queues nothing"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_client_adapter__resolveworldnumber__contract() {
    let value = run(
        &bot(
            "import { resolveWorldNumber } from '../../client/config/worlds.js';",
            r#"
const capture = (fn) => { try { return { ok: true, value: fn() }; }
    catch (error) { return { ok: false, name: error.constructor.name, message: error.message }; } };
const params = (...values) => ({ getAll: (name) => name === 'world' ? values : [] });
globalThis.__worlds = {
    host: resolveWorldNumber('w2.rs2b2t.com', params()),
    explicit: resolveWorldNumber('w3.rs2b2t.com', params('1')),
    fallback: resolveWorldNumber('elsewhere.example', params()),
    invalid: capture(() => resolveWorldNumber('w1.rs2b2t.com', params('4'))),
    duplicate: capture(() => resolveWorldNumber('w1.rs2b2t.com', params('1', '2'))),
};
"#,
        ),
        "__worlds",
    );
    assert_eq!(value["host"], 2);
    assert_eq!(value["explicit"], 1);
    assert_eq!(value["fallback"], 1);
    assert_eq!(value["invalid"]["ok"], false);
    assert_eq!(value["invalid"]["message"], "world must be 1, 2 or 3");
    assert_eq!(value["duplicate"]["ok"], false);
    assert_eq!(value["duplicate"]["message"], "world must be 1, 2 or 3");
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_client_adapter__reader_tradeconfirmready__contract() {
    let iso = LoadIsolate::spawn(
        bot(
            "import { reader } from '../../adapter/ClientAdapter.js';",
            "globalThis.__ready = () => reader.tradeConfirmReady();",
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let snapshot: SnapshotInput<'_> = common::ingame_snapshot();
    let inv_ops = ["Offer".to_string()];
    let mine = [item_row(Some("Coins"), 995, 1, 0, 3542, &inv_ops)];
    let theirs = [item_row(Some("Feather"), 314, 1, 0, 3539, &inv_ops)];
    let mut mine_widget = widget(3542, 600, 600);
    mine_widget.items = &mine;
    let mut their_widget = widget(3539, 600, 600);
    their_widget.items = &theirs;
    let both = [mine_widget, their_widget];
    let mut client = client_facts(true, -1);
    client.widget_facts = &both;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &snapshot,
        NativeFactsInput {
            client_adapter: Some(client),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__ready()").unwrap(),
        true,
        "both posted sides are ready"
    );

    let only_mine = [mine_widget];
    let mut client = client_facts(true, -1);
    client.widget_facts = &only_mine;
    let mut next = snapshot;
    next.tick = 2;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(client),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__ready()").unwrap(),
        false,
        "one offer side is not a confirmed page"
    );

    let mut mine_nothing = widget(3557, 600, 600);
    mine_nothing.text = Some("Absolutely nothing!");
    let mut their_nothing = widget(3558, 600, 600);
    their_nothing.text = Some("Absolutely nothing!");
    let nothing = [mine_nothing, their_nothing];
    let mut client = client_facts(true, -1);
    client.widget_facts = &nothing;
    next.tick = 3;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(client),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(3);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__ready()").unwrap(),
        true,
        "actual nothing labels confirm empty sides"
    );

    let one_nothing = [mine_nothing];
    let mut client = client_facts(true, -1);
    client.widget_facts = &one_nothing;
    next.tick = 4;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(client),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(4);
    let _ = iso.probe("true");
    assert_eq!(
        iso.probe("__ready()").unwrap(),
        false,
        "one missing nothing label is not ready"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_client_adapter__reader_modalcloseobservation__session_contract() {
    let iso = LoadIsolate::spawn(
        bot(
            "import { reader } from '../../adapter/ClientAdapter.js';",
            r#"
globalThis.__sessions ??= [];
globalThis.__observation = () => {
    const value = reader.modalCloseObservation();
    if (value === null) return null;
    const same = globalThis.__sessions.length === 0
        || globalThis.__sessions[0] === value.session;
    globalThis.__sessions.push(value.session);
    return [
        typeof value.session,
        same,
        value.generation,
    ];
};
"#,
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let snapshot: SnapshotInput<'_> = common::ingame_snapshot();
    let mut facts = client_facts(true, -1);
    facts.session_generation = 10;
    facts.modal_close_generation = 2;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &snapshot,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let first = iso.probe("__observation()").unwrap();
    let same = iso.probe("__observation()").unwrap();
    assert_eq!(first[0], "symbol");
    assert_eq!(first[1], true);
    assert_eq!(same[0], "symbol");
    assert_eq!(same[1], true, "one client session preserves its token");
    assert_eq!(first[2], 2);

    let mut next = snapshot;
    next.tick = 2;
    facts.session_generation = 11;
    facts.modal_close_generation = 0;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    let changed = iso.probe("__observation()").unwrap();
    assert_eq!(changed[0], "symbol");
    assert_eq!(
        changed[1], false,
        "a new client session must not reuse a stale token"
    );
    assert_eq!(changed[2], 0);

    facts.attached = false;
    next.tick = 3;
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        &next,
        NativeFactsInput {
            client_adapter: Some(facts),
            ..NativeFactsInput::default()
        },
    ));
    iso.on_game_tick(3);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__observation()").unwrap(), Value::Null);
    iso.join();
}
