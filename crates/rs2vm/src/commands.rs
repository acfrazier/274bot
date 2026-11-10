//! The closed slice-1 command inventory and engine number operations.

/// A supported command and its engine-handler authority.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// Command name used by the content compiler.
    pub name: &'static str,
    /// Relative engine handler path and source lines.
    pub handler: &'static str,
}
/// The 95 needed commands plus the two modal/player core commands.
pub const COMMANDS: &[Command] = &[
    // engine/script/handlers/PlayerOps.ts:448-452
    Command { name: "p_teleport", handler: "engine/script/handlers/PlayerOps.ts:448-452" },
    // engine/script/handlers/PlayerOps.ts:440-444
    Command { name: "p_telejump", handler: "engine/script/handlers/PlayerOps.ts:440-444" },
    // engine/script/handlers/PlayerOps.ts:939-947
    Command { name: "p_exactmove", handler: "engine/script/handlers/PlayerOps.ts:939-947" },
    // engine/script/handlers/PlayerOps.ts:376-380
    Command { name: "p_delay", handler: "engine/script/handlers/PlayerOps.ts:376-380" },
    // engine/script/handlers/PlayerOps.ts:359-367
    Command { name: "p_arrivedelay", handler: "engine/script/handlers/PlayerOps.ts:359-367" },
    // engine/script/handlers/LocOps.ts:18-43
    Command { name: "loc_add", handler: "engine/script/handlers/LocOps.ts:18-43" },
    // engine/script/handlers/LocOps.ts:60-67
    Command { name: "loc_change", handler: "engine/script/handlers/LocOps.ts:60-67" },
    // engine/script/handlers/LocOps.ts:74-77
    Command { name: "loc_del", handler: "engine/script/handlers/LocOps.ts:74-77" },
    // engine/script/handlers/PlayerOps.ts:655-657
    Command { name: "if_openchat", handler: "engine/script/handlers/PlayerOps.ts:655-657" },
    // engine/script/handlers/PlayerOps.ts:246-248
    Command { name: "if_close", handler: "engine/script/handlers/PlayerOps.ts:246-248" },
    // engine/script/handlers/PlayerOps.ts:425-427
    Command { name: "p_pausebutton", handler: "engine/script/handlers/PlayerOps.ts:425-427" },
    // engine/script/handlers/PlayerOps.ts:78-97
    Command { name: "p_finduid", handler: "engine/script/handlers/PlayerOps.ts:78-97" },
    // engine/script/handlers/PlayerOps.ts:632-634
    Command { name: "uid", handler: "engine/script/handlers/PlayerOps.ts:632-634" },
    // engine/script/handlers/PlayerOps.ts:724-727
    Command { name: "if_openoverlay", handler: "engine/script/handlers/PlayerOps.ts:724-727" },
    // engine/script/handlers/PlayerOps.ts:149-158
    Command { name: "queue", handler: "engine/script/handlers/PlayerOps.ts:149-158" },
    // engine/script/handlers/PlayerOps.ts:172-181
    Command { name: "longqueue", handler: "engine/script/handlers/PlayerOps.ts:172-181" },
    // engine/script/handlers/PlayerOps.ts:890-900
    Command { name: "settimer", handler: "engine/script/handlers/PlayerOps.ts:890-900" },
    // engine/script/handlers/PlayerOps.ts:902-904
    Command { name: "cleartimer", handler: "engine/script/handlers/PlayerOps.ts:902-904" },
    // engine/script/handlers/PlayerOps.ts:1102-1105
    Command { name: "clearqueue", handler: "engine/script/handlers/PlayerOps.ts:1102-1105" },
    // engine/script/handlers/InvOps.ts:129-141
    Command { name: "inv_del", handler: "engine/script/handlers/InvOps.ts:129-141" },
    // engine/script/handlers/ObjOps.ts:20-55
    Command { name: "obj_add", handler: "engine/script/handlers/ObjOps.ts:20-55" },
    // engine/script/handlers/PlayerOps.ts:343-347
    Command { name: "mes", handler: "engine/script/handlers/PlayerOps.ts:343-347" },
    // engine/script/handlers/NpcOps.ts:57-68
    Command { name: "npc_add", handler: "engine/script/handlers/NpcOps.ts:57-68" },
    // engine/script/handlers/NpcOps.ts:93-95
    Command { name: "npc_del", handler: "engine/script/handlers/NpcOps.ts:93-95" },
    // engine/script/handlers/NpcOps.ts:206-248
    Command { name: "npc_setmode", handler: "engine/script/handlers/NpcOps.ts:206-248" },
    // engine/script/handlers/NpcOps.ts:466-470
    Command { name: "npc_walk", handler: "engine/script/handlers/NpcOps.ts:466-470" },
    // engine/script/handlers/NpcOps.ts:185-187
    Command { name: "npc_say", handler: "engine/script/handlers/NpcOps.ts:185-187" },
    // engine/script/handlers/NpcOps.ts:70-75
    Command { name: "npc_anim", handler: "engine/script/handlers/NpcOps.ts:70-75" },
    // engine/script/handlers/NpcOps.ts:103-107
    Command { name: "npc_facesquare", handler: "engine/script/handlers/NpcOps.ts:103-107" },
    // engine/script/handlers/NpcOps.ts:297-303
    Command { name: "spotanim_npc", handler: "engine/script/handlers/NpcOps.ts:297-303" },
    // engine/script/handlers/PlayerOps.ts:196-201
    Command { name: "anim", handler: "engine/script/handlers/PlayerOps.ts:196-201" },
    // engine/script/handlers/PlayerOps.ts:476-487
    Command { name: "sound_synth", handler: "engine/script/handlers/PlayerOps.ts:476-487" },
    // engine/script/handlers/PlayerOps.ts:240-244
    Command { name: "facesquare", handler: "engine/script/handlers/PlayerOps.ts:240-244" },
    // engine/script/handlers/PlayerOps.ts:979-986
    Command { name: "p_locmerge", handler: "engine/script/handlers/PlayerOps.ts:979-986" },
    // engine/script/handlers/PlayerOps.ts:741-783
    Command { name: "if_settext", handler: "engine/script/handlers/PlayerOps.ts:741-783" },
    // engine/script/handlers/PlayerOps.ts:704-710
    Command { name: "if_setanim", handler: "engine/script/handlers/PlayerOps.ts:704-710" },
    // engine/script/handlers/PlayerOps.ts:785-792
    Command { name: "if_setnpchead", handler: "engine/script/handlers/PlayerOps.ts:785-792" },
    // engine/script/handlers/PlayerOps.ts:207-212
    Command { name: "cam_lookat", handler: "engine/script/handlers/PlayerOps.ts:207-212" },
    // engine/script/handlers/PlayerOps.ts:214-219
    Command { name: "cam_moveto", handler: "engine/script/handlers/PlayerOps.ts:214-219" },
    // engine/script/handlers/PlayerOps.ts:227-229
    Command { name: "cam_reset", handler: "engine/script/handlers/PlayerOps.ts:227-229" },
    // engine/script/handlers/PlayerOps.ts:923-928
    Command { name: "hint_coord", handler: "engine/script/handlers/PlayerOps.ts:923-928" },
    // engine/script/handlers/PlayerOps.ts:930-932
    Command { name: "hint_stop", handler: "engine/script/handlers/PlayerOps.ts:930-932" },
    // engine/script/handlers/ServerOps.ts:203-211
    Command { name: "projanim_map", handler: "engine/script/handlers/ServerOps.ts:203-211" },
    // engine/script/handlers/ServerOps.ts:85-92
    Command { name: "spotanim_map", handler: "engine/script/handlers/ServerOps.ts:85-92" },
    // engine/script/handlers/PlayerOps.ts:231-234
    Command { name: "coord", handler: "engine/script/handlers/PlayerOps.ts:231-234" },
    // engine/script/handlers/PlayerOps.ts:493-497
    Command { name: "stat", handler: "engine/script/handlers/PlayerOps.ts:493-497" },
    // engine/script/handlers/InvOps.ts:616-628
    Command { name: "inv_total", handler: "engine/script/handlers/InvOps.ts:616-628" },
    // engine/script/handlers/InvOps.ts:631-638
    Command { name: "inv_totalcat", handler: "engine/script/handlers/InvOps.ts:631-638" },
    // engine/script/handlers/InvOps.ts:278-283
    Command { name: "inv_getobj", handler: "engine/script/handlers/InvOps.ts:278-283" },
    // engine/script/handlers/ServerOps.ts:20-22
    Command { name: "map_members", handler: "engine/script/handlers/ServerOps.ts:20-22" },
    // engine/script/handlers/ServerOps.ts:16-18
    Command { name: "map_clock", handler: "engine/script/handlers/ServerOps.ts:16-18" },
    // engine/script/handlers/PlayerOps.ts:906-921
    Command { name: "gettimer", handler: "engine/script/handlers/PlayerOps.ts:906-921" },
    // engine/script/handlers/PlayerOps.ts:838-845
    Command { name: "text_gender", handler: "engine/script/handlers/PlayerOps.ts:838-845" },
    // engine/script/handlers/LocOps.ts:69-72
    Command { name: "loc_coord", handler: "engine/script/handlers/LocOps.ts:69-72" },
    // engine/script/handlers/LocOps.ts:125-127
    Command { name: "loc_type", handler: "engine/script/handlers/LocOps.ts:125-127" },
    // engine/script/handlers/LocOps.ts:45-47
    Command { name: "loc_angle", handler: "engine/script/handlers/LocOps.ts:45-47" },
    // engine/script/handlers/LocOps.ts:133-135
    Command { name: "loc_shape", handler: "engine/script/handlers/LocOps.ts:133-135" },
    // engine/script/handlers/LocOps.ts:56-58
    Command { name: "loc_category", handler: "engine/script/handlers/LocOps.ts:56-58" },
    // engine/script/handlers/LocOps.ts:114-123
    Command { name: "loc_param", handler: "engine/script/handlers/LocOps.ts:114-123" },
    // engine/script/handlers/LocOps.ts:129-131
    Command { name: "loc_name", handler: "engine/script/handlers/LocOps.ts:129-131" },
    // engine/script/handlers/LocConfigOps.ts:15-25
    Command { name: "lc_param", handler: "engine/script/handlers/LocConfigOps.ts:15-25" },
    // engine/script/handlers/LocOps.ts:96-100
    Command { name: "loc_findallzone", handler: "engine/script/handlers/LocOps.ts:96-100" },
    // engine/script/handlers/LocOps.ts:102-112
    Command { name: "loc_findnext", handler: "engine/script/handlers/LocOps.ts:102-112" },
    // engine/script/handlers/NpcOps.ts:351-382
    Command { name: "npc_find", handler: "engine/script/handlers/NpcOps.ts:351-382" },
    // engine/script/handlers/NpcOps.ts:428-437
    Command { name: "npc_findall", handler: "engine/script/handlers/NpcOps.ts:428-437" },
    // engine/script/handlers/NpcOps.ts:418-426
    Command { name: "npc_findallany", handler: "engine/script/handlers/NpcOps.ts:418-426" },
    // engine/script/handlers/NpcOps.ts:340-348
    Command { name: "npc_huntall", handler: "engine/script/handlers/NpcOps.ts:340-348" },
    // engine/script/handlers/NpcOps.ts:445-456
    Command { name: "npc_findnext", handler: "engine/script/handlers/NpcOps.ts:445-456" },
    // engine/script/handlers/NpcOps.ts:88-91
    Command { name: "npc_coord", handler: "engine/script/handlers/NpcOps.ts:88-91" },
    // engine/script/handlers/NpcOps.ts:274-276
    Command { name: "npc_type", handler: "engine/script/handlers/NpcOps.ts:274-276" },
    // engine/script/handlers/NpcOps.ts:83-85
    Command { name: "npc_category", handler: "engine/script/handlers/NpcOps.ts:83-85" },
    // engine/script/handlers/NpcOps.ts:488-490
    Command { name: "npc_getmode", handler: "engine/script/handlers/NpcOps.ts:488-490" },
    // engine/script/handlers/NpcOps.ts:285-287
    Command { name: "npc_name", handler: "engine/script/handlers/NpcOps.ts:285-287" },
    // engine/script/handlers/NpcConfigOps.ts:9-13
    Command { name: "nc_name", handler: "engine/script/handlers/NpcConfigOps.ts:9-13" },
    // engine/script/handlers/NumberOps.ts:8-12
    Command { name: "add", handler: "engine/script/handlers/NumberOps.ts:8-12" },
    // engine/script/handlers/NumberOps.ts:14-18
    Command { name: "sub", handler: "engine/script/handlers/NumberOps.ts:14-18" },
    // engine/script/handlers/NumberOps.ts:20-24
    Command { name: "multiply", handler: "engine/script/handlers/NumberOps.ts:20-24" },
    // engine/script/handlers/NumberOps.ts:26-30
    Command { name: "divide", handler: "engine/script/handlers/NumberOps.ts:26-30" },
    // engine/script/handlers/NumberOps.ts:69-72
    Command { name: "modulo", handler: "engine/script/handlers/NumberOps.ts:69-72" },
    // engine/script/handlers/NumberOps.ts:64-67
    Command { name: "testbit", handler: "engine/script/handlers/NumberOps.ts:64-67" },
    // engine/script/handlers/NumberOps.ts:148-152
    Command { name: "getbit_range", handler: "engine/script/handlers/NumberOps.ts:148-152" },
    // engine/script/handlers/NumberOps.ts:59-62
    Command { name: "clearbit", handler: "engine/script/handlers/NumberOps.ts:59-62" },
    // engine/script/handlers/NumberOps.ts:143-146
    Command { name: "clearbit_range", handler: "engine/script/handlers/NumberOps.ts:143-146" },
    // engine/script/handlers/NumberOps.ts:154-163
    Command { name: "setbit_range_toint", handler: "engine/script/handlers/NumberOps.ts:154-163" },
    // engine/script/handlers/ServerOps.ts:103-108
    Command { name: "movecoord", handler: "engine/script/handlers/ServerOps.ts:103-108" },
    // engine/script/handlers/ServerOps.ts:114-116
    Command { name: "coordx", handler: "engine/script/handlers/ServerOps.ts:114-116" },
    // engine/script/handlers/ServerOps.ts:122-124
    Command { name: "coordz", handler: "engine/script/handlers/ServerOps.ts:122-124" },
    // engine/script/handlers/ServerOps.ts:94-101
    Command { name: "distance", handler: "engine/script/handlers/ServerOps.ts:94-101" },
    // engine/script/handlers/ServerOps.ts:276-335
    Command { name: "map_findsquare", handler: "engine/script/handlers/ServerOps.ts:276-335" },
    // engine/script/handlers/EnumOps.ts:7-23
    Command { name: "enum", handler: "engine/script/handlers/EnumOps.ts:7-23" },
    // engine/script/handlers/StringOps.ts:54-56
    Command { name: "string_length", handler: "engine/script/handlers/StringOps.ts:54-56" },
    // engine/script/handlers/StringOps.ts:29-31
    Command { name: "lowercase", handler: "engine/script/handlers/StringOps.ts:29-31" },
    // engine/script/handlers/StringOps.ts:76-96
    Command { name: "split_init", handler: "engine/script/handlers/StringOps.ts:76-96" },
    // engine/script/handlers/StringOps.ts:104-106
    Command { name: "split_pagecount", handler: "engine/script/handlers/StringOps.ts:104-106" },
    // engine/script/handlers/StringOps.ts:108-112
    Command { name: "split_linecount", handler: "engine/script/handlers/StringOps.ts:108-112" },
    // engine/script/handlers/StringOps.ts:98-102
    Command { name: "split_get", handler: "engine/script/handlers/StringOps.ts:98-102" },
    // engine/script/handlers/StringOps.ts:114-122
    Command { name: "split_getanim", handler: "engine/script/handlers/StringOps.ts:114-122" },
];
/// Resolve an opcode name without allocating a lowercase copy.
pub fn lookup(opcode: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|row| row.name.eq_ignore_ascii_case(opcode))
}
/// Engine integer arithmetic operations.
#[derive(Clone, Copy, Debug)]
pub enum ArithOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Multiply,
    /// Division, including infinite results.
    Divide,
    /// Remainder, including NaN results.
    Modulo,
}
/// ECMAScript ToInt32, as used by ScriptState.ts:304-306 and util/Numbers.ts:7-9.
pub fn to_int32(value: f64) -> i32 {
    if !value.is_finite() || value == 0.0 { return 0; }
    let reduced = value.trunc().rem_euclid(4_294_967_296.0);
    if reduced >= 2_147_483_648.0 { (reduced - 4_294_967_296.0) as i32 } else { reduced as i32 }
}
/// Evaluate NumberOps.ts:8-30,69-72 in f64, then apply engine pushInt truncation.
pub fn js_arith(op: ArithOp, a: i32, b: i32) -> i32 {
    let a=f64::from(a); let b=f64::from(b);
    to_int32(match op {ArithOp::Add=>a+b,ArithOp::Sub=>a-b,ArithOp::Multiply=>a*b,ArithOp::Divide=>a/b,ArithOp::Modulo=>a%b})
}
