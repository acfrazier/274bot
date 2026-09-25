/// One interact request the shim `Bank`/`Banking` modules queue on the
/// host handle (`__rs2b0t_host.interact`); the isolate thread forwards the
/// queue to the host after each tick, and host-play dispatches each op
/// through the slot Driver. Missing targets fail closed at dispatch (no
/// matching loc/npc/item row → nothing is sent).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(tag = "op")]
pub enum InteractReq {
    /// Open the exact snapshot-selected loc. With no name/action the host
    /// validates `Use-quickly`; named access validates both without fallback.
    #[serde(rename = "open-booth")]
    OpenBooth {
        x: i32,
        z: i32,
        level: i32,
        id: i32,
        name: Option<String>,
        action: Option<String>,
    },
    /// Use a packed stand the player is adjacent to: a booth loc
    /// (Use-quickly) or a teller NPC (its 1-based op slot from the pack;
    /// `choose` is the dialog option the op's dialogue needs, deferred).
    #[serde(rename = "open-stand")]
    OpenStand {
        x: i32,
        z: i32,
        level: i32,
        kind: String,
        name: Option<String>,
        /// The stand's 1-based access op slot (booth Use-quickly or the
        /// teller NPC op from the pack).
        stand_op: Option<i32>,
        choose: Option<String>,
    },
    /// Packed nav (`Traveller` / `ScriptWalkArm`). FindOptions bits are
    /// per-request; serde/old buffers default all three off. v1 world
    /// walk writers set wilderness and bank-fetch explicitly.
    #[serde(rename = "walk")]
    Walk {
        x: i32,
        z: i32,
        level: i32,
        #[serde(default)]
        allow_teleports: bool,
        #[serde(default)]
        allow_wilderness: bool,
        #[serde(default)]
        allow_bank_fetch: bool,
        /// Isolate-allocated walk wait token. `0` on old callers.
        #[serde(default)]
        request_id: u64,
    },
    /// Packed navigation to a reachable tile within the requested radius.
    #[serde(rename = "walk-near")]
    WalkNear {
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
        #[serde(default)]
        allow_teleports: bool,
        #[serde(default)]
        allow_wilderness: bool,
        #[serde(default)]
        allow_bank_fetch: bool,
        /// Isolate-allocated walk wait token. `0` on old callers.
        #[serde(default)]
        request_id: u64,
    },
    /// Read-only, bounded native bank selection; never arms movement.
    #[serde(rename = "select-bank")]
    SelectBank {
        x: i32,
        z: i32,
        level: i32,
        #[serde(default)]
        allow_wilderness: bool,
        #[serde(default)]
        use_mage_bank: bool,
        #[serde(default)]
        use_zanaris_bank: bool,
        request_id: u64,
    },
    /// Select the nearest packed booth stand in Rust and route to that exact stand.
    #[serde(rename = "walk-nearest-bank")]
    WalkNearestBank,
    /// Stop the armed scripted walk follow (frozen: a `walkResilient` that
    /// returned has stopped its walker). A Rust machine sends it when its
    /// own walk wait timed out, before a later click the follow's next
    /// walk packet would otherwise cancel.
    /// `request_id` is the machine's own walk token: the host aborts only
    /// while that walk is the armed one, so a later script walk survives.
    #[serde(rename = "abort-walk")]
    AbortWalk {
        #[serde(default)]
        request_id: u64,
    },
    /// Pure inspect-route preview. `x/z/level` are the destination.
    #[serde(rename = "inspect-route")]
    InspectRoute {
        x: i32,
        z: i32,
        level: i32,
        from_x: i32,
        from_z: i32,
        from_level: i32,
        #[serde(default)]
        allow_teleports: bool,
        #[serde(default)]
        allow_wilderness: bool,
        #[serde(default)]
        allow_bank_fetch: bool,
        #[serde(default)]
        avoid: Vec<InspectAvoidWire>,
        #[serde(default)]
        request_id: u64,
    },
    /// Isolate-applied inspect consume-ack. Not a public JS request.
    /// Host rejects generation mismatch, seq 0, and seq above the posted ring.
    #[serde(rename = "inspect-ack")]
    InspectAck { seq: u64, generation: u64 },
    /// Scene `try_move` packet with nearest fallback
    /// (`Interactions::walk_nearest`) used by `DirectNavigator`, not world
    /// `Traversal`.
    #[serde(rename = "walk-to")]
    WalkTo { x: i32, z: i32, level: i32 },
    /// Deposit-all the bank-side item named `name`.
    #[serde(rename = "deposit")]
    Deposit { name: String },
    /// Withdraw the bank item named `name` with the action label
    /// (`Withdraw All` / `Withdraw 10` / `Withdraw 1`).
    #[serde(rename = "withdraw")]
    Withdraw { name: String, action: String },
    /// Begin a host-owned, bounded Withdraw-X continuation. The host sends
    /// the X menu action now and only answers a later count dialog while the
    /// same bank session generation remains current.
    #[serde(rename = "withdraw-x")]
    WithdrawX {
        name: String,
        count: i32,
        bank_item_id: i32,
        lands_as_id: i32,
        action: String,
        bank_generation: u64,
    },
    /// Fill free inventory slots from one fresh bank row. Rust selects
    /// Withdraw-All or the shared Withdraw-X continuation and observes settlement.
    #[serde(rename = "withdraw-load")]
    WithdrawLoad { name: String, bank_generation: u64 },
    /// Interact with the held item named `name` using the action label
    /// (`Bury`, `Wear`, …). The host resolves the name through ObjNames
    /// and dispatches the item's menu op (rs2b0t `Item.interact`).
    #[serde(rename = "held")]
    Held { name: String, action: String },
    /// Selected component-item operation (`Input.invButton`). Host
    /// dispatch re-resolves the exact current bank row by id/slot/
    /// component and sends `ActionSpec::Operation`. It does not answer
    /// the later count dialog.
    #[serde(rename = "inv-button")]
    InvButton {
        id: i32,
        slot: i32,
        component: i32,
        operation: i32,
        #[serde(default)]
        bank_generation: u64,
    },
    /// Click one piece on the open puzzle board (the widget `obj_ops`
    /// menu's held family, not the component `iop`). Host dispatch
    /// re-resolves the exact posted board row by id/slot/component under
    /// `generation` and sends the Held opcode (`Move`, else op 5): it
    /// never sends INV_BUTTON, and a sent packet is not board progress.
    #[serde(rename = "puzzle-move")]
    PuzzleMove {
        id: i32,
        slot: i32,
        component: i32,
        generation: u64,
    },
    /// Close the open bank modal.
    #[serde(rename = "close")]
    Close,
    /// Interact with an NPC by name using an action label (`Pick`, …).
    #[serde(rename = "npc")]
    Npc {
        name: String,
        action: String,
        index: Option<i32>,
    },
    /// Interact with a loc at `(x, z, level)` using an action label.
    /// `id` is the selected loc type from the posted snapshot row. Host
    /// dispatch matches that identity on the current snapshot and refuses
    /// rather than taking another co-located row. Absent `id` keeps the
    /// previous first-row coordinate match.
    #[serde(rename = "loc")]
    Loc {
        x: i32,
        z: i32,
        level: i32,
        action: String,
        #[serde(default)]
        id: Option<i32>,
    },
    /// Interact with a ground item at `(x, z, level)` using an action label.
    #[serde(rename = "obj")]
    Obj {
        x: i32,
        z: i32,
        level: i32,
        name: Option<String>,
        action: String,
    },
    /// Interact with a player by name using an action label.
    #[serde(rename = "player")]
    Player { name: String, action: String },
    /// Use a held inventory item on a scene target (`Game.castOnItem`).
    #[serde(rename = "use-on")]
    UseOn {
        name: String,
        kind: String,
        target_name: Option<String>,
        x: i32,
        z: i32,
        level: i32,
        index: Option<i32>,
        source_item_id: Option<i32>,
        source_item_slot: Option<i32>,
        target_item_id: Option<i32>,
        target_item_slot: Option<i32>,
    },
    /// Use a widget (spell / interface button) on a scene target.
    #[serde(rename = "use-widget-on")]
    UseWidgetOn {
        component_id: i32,
        kind: String,
        target_name: Option<String>,
        x: i32,
        z: i32,
        level: i32,
        index: Option<i32>,
    },
    /// Continue the open chat dialog.
    #[serde(rename = "continue")]
    ContinueDialog,
    /// Answer the chat modal's `option`-th choice (1-based).
    #[serde(rename = "answer")]
    Answer { option: i32 },
    /// Press an interface button by component id.
    #[serde(rename = "if-button")]
    IfButton { component_id: i32 },
    /// Press one fixed shop Buy/Sell op on the exact posted shop row. Rust
    /// owns the 10/5/1 batching, the packet-per-tick bound and the held-count
    /// settlement; the host re-resolves this exact row and refuses a
    /// same-name fallback.
    #[serde(rename = "shop-button")]
    ShopButton {
        kind: String,
        name: String,
        id: i32,
        slot: i32,
        component: i32,
        chunk: i32,
    },
    /// Press the exact posted anvil/main skill-multi row. Host dispatch
    /// re-resolves id/slot/component on the current main-make rows and
    /// sends `ActionSpec::Operation`. It does not fall back to a
    /// same-name row or answer a count dialog.
    #[serde(rename = "make-panel")]
    MakePanel {
        id: i32,
        slot: i32,
        component: i32,
        operation: i32,
    },
    /// Close the open main/side/chat modal (not the bank).
    #[serde(rename = "close-modal")]
    CloseModal,
    /// Answer the open count dialog with `value`.
    #[serde(rename = "answer-count")]
    AnswerCount { value: i32 },
    /// Switch the active side tab.
    #[serde(rename = "side-tab")]
    SideTab { tab: i32 },
    /// Wear/wield an inventory item by resolved name.
    #[serde(rename = "wear")]
    Wear { name: String },
    /// Remove a worn item by resolved name (the worn component's `Remove`).
    #[serde(rename = "unequip")]
    Unequip { name: String },
    /// Toggle run on/off.
    #[serde(rename = "set-run")]
    SetRun { on: bool },
    /// Toggle auto-retaliate on/off.
    #[serde(rename = "set-retaliate")]
    SetRetaliate { on: bool },
    /// Toggle bank withdraw-as-note on/off.
    #[serde(rename = "set-note-mode")]
    SetNoteMode { on: bool },
    /// Host-side orbit yaw write (`client.orbit_camera_yaw`); no opcode.
    #[serde(rename = "set-camera-yaw")]
    SetCameraYaw { yaw: i32 },
    /// Execution.noteProgress: stamps both watchdog clocks. Not a game op.
    #[serde(rename = "note-progress")]
    NoteProgress,
    /// Compat runner: `loop()` promise fulfilled. Scheduler progress only.
    #[serde(rename = "loop-settled")]
    LoopSettled,
    /// Execution wait enqueue (parked rising edge). Scheduler progress.
    #[serde(rename = "wait-enqueued")]
    WaitEnqueued,
    /// Execution wait settle (including re-park in the same pump).
    #[serde(rename = "wait-settled")]
    WaitSettled,
    /// Validated `recoveryAnchor()` tile, isolate→host, generation-tagged.
    #[serde(rename = "recovery-anchor")]
    RecoveryAnchor { x: i32, z: i32, level: i32 },
    /// `recoveryAnchor()` missing, invalid, or threw.
    #[serde(rename = "recovery-anchor-none")]
    RecoveryAnchorNone,
    /// One canvas KeyboardEvent. `key` is the DOM key string; `code` is
    /// optional. `down` is keydown vs keyup. Host allowlists digits and
    /// Enter onto the slot Client's GameShell.
    #[serde(rename = "key")]
    Key {
        down: bool,
        key: String,
        #[serde(default)]
        code: String,
    },
    /// One canvas MouseEvent. Coordinates stay f64 until native mapping.
    /// `identity` is the native permit at production; JS does not set it.
    #[serde(rename = "mouse")]
    Mouse {
        down: bool,
        #[serde(deserialize_with = "deserialize_js_f64")]
        x: f64,
        #[serde(deserialize_with = "deserialize_js_f64")]
        y: f64,
        #[serde(default, deserialize_with = "deserialize_js_button")]
        button: i32,
        #[serde(default)]
        identity: u64,
    },
}

/// One inspect avoid entry. Typed rects keep their bounds; anything else
/// is `Unsupported` so Rust can refuse `invalid-args` instead of dropping
/// the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectAvoidWire {
    Rect {
        min_x: i32,
        max_x: i32,
        min_z: i32,
        max_z: i32,
        level: Option<i32>,
    },
    Unsupported,
}

impl<'de> serde::Deserialize<'de> for InspectAvoidWire {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let Some(obj) = value.as_object() else {
            return Ok(Self::Unsupported);
        };
        let coord = |camel: &str, snake: &str| {
            obj.get(camel)
                .or_else(|| obj.get(snake))
                .and_then(serde_json::Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
        };
        let (Some(min_x), Some(max_x), Some(min_z), Some(max_z)) = (
            coord("minX", "min_x"),
            coord("maxX", "max_x"),
            coord("minZ", "min_z"),
            coord("maxZ", "max_z"),
        ) else {
            return Ok(Self::Unsupported);
        };
        let level = coord("level", "level");
        Ok(Self::Rect {
            min_x,
            max_x,
            min_z,
            max_z,
            level,
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub(crate) enum MaybeInteractReq {
    Req(InteractReq),
    Skip(RejectedRow),
}

/// A queued interact row no [`InteractReq`] variant accepts, named by its
/// `op` so the tick loop can log what it refused instead of dropping it
/// silently. Accepts every value kind, as the `IgnoredAny` it replaced did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RejectedRow(pub(crate) String);

impl<'de> serde::Deserialize<'de> for RejectedRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(RejectedRow(match Shape::deserialize(d)? {
            Shape::Str(_) => "a string".into(),
            Shape::Object(Some(op)) => format!("op {op:?}"),
            Shape::Object(None) => "an object without a string op".into(),
            Shape::Other(kind) => kind.into(),
        }))
    }
}

/// One queued row, read on its own: a row serde cannot read at all (a
/// BigInt, say) is refused like any other malformed row instead of failing
/// every row beside it.
pub(crate) struct QueuedRow(pub(crate) MaybeInteractReq);

impl<'de> serde::Deserialize<'de> for QueuedRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(QueuedRow(MaybeInteractReq::deserialize(d).unwrap_or_else(
            |e| MaybeInteractReq::Skip(RejectedRow(format!("unreadable ({e})"))),
        )))
    }
}

/// Any value, as much of it as a log line names: a string's text, an
/// object's string `op`, or the kind of anything else.
enum Shape {
    Str(String),
    Object(Option<String>),
    Other(&'static str),
}

impl<'de> serde::Deserialize<'de> for Shape {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Shape;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "any JS value")
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Shape, E> {
                Ok(Shape::Other("a boolean"))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Shape, E> {
                Ok(Shape::Other("a number"))
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Shape, E> {
                Ok(Shape::Other("a number"))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Shape, E> {
                Ok(Shape::Other("a number"))
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Shape, E> {
                Ok(Shape::Str(s.to_string()))
            }
            fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<Shape, E> {
                Ok(Shape::Other("bytes"))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Shape, E> {
                Ok(Shape::Other("null"))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Shape, E> {
                Ok(Shape::Other("null"))
            }
            fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Shape, D::Error> {
                <Shape as serde::Deserialize>::deserialize(d)
            }
            fn visit_newtype_struct<D: serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<Shape, D::Error> {
                <Shape as serde::Deserialize>::deserialize(d)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Shape, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Shape::Other("an array"))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Shape, A::Error> {
                let mut op = None;
                while let Some(key) = map.next_key::<Shape>()? {
                    match key {
                        Shape::Str(key) if key == "op" => {
                            op = match map.next_value::<Shape>()? {
                                Shape::Str(op) => Some(op),
                                _ => None,
                            };
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(Shape::Object(op))
            }
        }
        d.deserialize_any(V)
    }
}

fn deserialize_js_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = f64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a JS number")
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<f64, E> {
            Ok(v)
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<f64, E> {
            Ok(v as f64)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<f64, E> {
            Ok(v as f64)
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<f64, E> {
            Ok(f64::NAN)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<f64, E> {
            Ok(f64::NAN)
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<f64, E> {
            Ok(f64::NAN)
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<f64, E> {
            Ok(f64::NAN)
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<f64, A::Error> {
            while map
                .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                .is_some()
            {}
            Ok(f64::NAN)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<f64, A::Error> {
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(f64::NAN)
        }
    }
    d.deserialize_any(V)
}

fn deserialize_js_button<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = i32;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a JS button number")
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<i32, E> {
            Ok(i32::try_from(v).unwrap_or(i32::MIN))
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<i32, E> {
            Ok(i32::try_from(v).unwrap_or(i32::MIN))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<i32, E> {
            if v.is_finite() && v.fract() == 0.0 && v >= i32::MIN as f64 && v <= i32::MAX as f64 {
                Ok(v as i32)
            } else {
                Ok(i32::MIN)
            }
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<i32, E> {
            Ok(i32::MIN)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<i32, E> {
            Ok(i32::MIN)
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<i32, E> {
            Ok(i32::MIN)
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<i32, E> {
            Ok(i32::MIN)
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<i32, A::Error> {
            while map
                .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                .is_some()
            {}
            Ok(i32::MIN)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<i32, A::Error> {
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(i32::MIN)
        }
    }
    d.deserialize_any(V)
}

impl InteractReq {
    /// Lifecycle facts for the native watchdog. Never dispatched as game ops.
    pub fn is_watchdog_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::NoteProgress
                | Self::LoopSettled
                | Self::WaitEnqueued
                | Self::WaitSettled
                | Self::RecoveryAnchor { .. }
                | Self::RecoveryAnchorNone
        )
    }
}
