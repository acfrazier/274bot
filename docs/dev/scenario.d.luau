-- Generated from the File scenario registries. Do not edit by hand.
-- Regenerate: UPDATE_SCENARIO_DEFINITIONS=1 cargo test -p host-play \
--   --no-default-features --features scenario-luau,test-support --lib scenario_definitions

export type Mark = number
export type Handle = number
export type Tile = { x: number, z: number, level: number }
export type Amount = number | { eq: number?, min: number?, max: number? }
export type Container = "inventory" | "worn" | "bank" | number

export type Check_action_sent = {
    after: number?,
    label: string?,
    within: number?,
}
export type Check_active_continue = {
    label: string?,
    within: number?,
}
export type Check_all = {
    of: any?,
    label: string?,
    within: number?,
}
export type Check_amount_prompt = {
    label: string?,
    within: number?,
}
export type Check_arrived = {
    x: number?,
    z: number?,
    level: number?,
    label: string?,
    within: number?,
}
export type Check_arrived_near = {
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_arrived_ring = {
    x: number?,
    z: number?,
    level: number?,
    min: number?,
    max: number?,
    label: string?,
    within: number?,
}
export type Check_bank_closed = {
    label: string?,
    within: number?,
}
export type Check_bank_exact = {
    expected: any?,
    label: string?,
    within: number?,
}
export type Check_bank_item = {
    name: string?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_bank_item_id = {
    id: number?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_bank_item_id_any = {
    ids: { number }?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_bank_item_id_at_most = {
    id: number?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_bank_item_at_most = {
    name: string?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_bank_open = {
    label: string?,
    within: number?,
}
export type Check_chat = {
    needle: string?,
    label: string?,
    within: number?,
}
export type Check_chat_closed = {
    label: string?,
    within: number?,
}
export type Check_chat_choice = {
    label: string?,
    within: number?,
}
export type Check_clue_replaced = {
    seeded: number?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_clue_protect_window = {
    seeded: number?,
    label: string?,
    within: number?,
}
export type Check_container_unchanged = {
    where: Container?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_count = {
    record: any?,
    from: number?,
    to: number | string?,
    amount: Amount?,
    exact: string?,
    label: string?,
    within: number?,
}
export type Check_dialogue = {
    open: boolean?,
    root: number?,
    continue: number?,
    lines: { string }?,
    options: { string }?,
    label: string?,
    within: number?,
}
export type Check_equipment_id = {
    id: number?,
    label: string?,
    within: number?,
}
export type Check_essence_mine = {
    label: string?,
    within: number?,
}
export type Check_fresh_chat = {
    needle: string?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_fresh_chat_since_previous_step = {
    needle: string?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_fresh_quest_completion = {
    chat_needle: string?,
    main_root_component_id: number?,
    component_id: number?,
    text: string?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_fresh_stat_xp_gain = {
    id: number?,
    min: number?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_fresh_tutorial = {
    since: number?,
    label: string?,
    within: number?,
}
export type Check_ground_item_ids_absent_near = {
    ids: { number }?,
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_hint = {
    kind: string?,
    at: Tile?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_ingame_scene_2 = {
    label: string?,
    within: number?,
}
export type Check_input_consumed = {
    id: number?,
    count: number?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_item = {
    name: string?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_item_at_most = {
    name: string?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_item_count = {
    item: number?,
    where: Container?,
    amount: Amount?,
    label: string?,
    within: number?,
}
export type Check_item_delta = {
    item: number?,
    where: Container?,
    since: number?,
    amount: Amount?,
    label: string?,
    within: number?,
}
export type Check_item_id = {
    id: number?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_item_id_any = {
    ids: { number }?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_item_id_at_most = {
    id: number?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_item_id_in_full_pack = {
    id: number?,
    count: number?,
    label: string?,
    within: number?,
}
export type Check_loc_action_near = {
    id: number?,
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    action: string?,
    present: boolean?,
    label: string?,
    within: number?,
}
export type Check_loc_id_near = {
    id: number?,
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_loc = {
    at: Tile?,
    name: string?,
    form: number?,
    options: { string }?,
    first_option: string?,
    no_option: string?,
    label: string?,
    within: number?,
}
export type Check_logged_out = {
    label: string?,
    within: number?,
}
export type Check_login_packets = {
    family: string?,
    form: string?,
    amount: Amount?,
    label: string?,
    within: number?,
}
export type Check_local_targeting_npc_name = {
    name: string?,
    label: string?,
    within: number?,
}
export type Check_npc = {
    type: number?,
    near: Tile?,
    radius: number?,
    index: number?,
    form: number?,
    options: { string }?,
    no_option: string?,
    sequence: number?,
    frame: number?,
    present: boolean?,
    label: string?,
    within: number?,
}
export type Check_npc_at = {
    type: number?,
    x: number?,
    z: number?,
    label: string?,
    within: number?,
}
export type Check_npc_ids_absent_near = {
    ids: { number }?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_npc_name_near = {
    name: string?,
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_npc_name_targeting_local = {
    name: string?,
    label: string?,
    within: number?,
}
export type Check_npc_name_unengaged_in_sight = {
    name: string?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_npc_near = {
    type: number?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_no_active_continue = {
    label: string?,
    within: number?,
}
export type Check_order = {
    first: string?,
    then: string?,
    gap: Amount?,
    label: string?,
    within: number?,
}
export type Check_product_delta = {
    id: number?,
    count: number?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_quest_done = {
    name: string?,
    label: string?,
    within: number?,
}
export type Check_received = {
    after: number?,
    stats: any?,
    inventories: any?,
    messages: any?,
    vars: any?,
    exact: string?,
    label: string?,
    within: number?,
}
export type Check_render_view_ready = {
    x: number?,
    z: number?,
    level: number?,
    orbit_yaw: number?,
    orbit_pitch: number?,
    label: string?,
    within: number?,
}
export type Check_side_tab_available = {
    index: number?,
    label: string?,
    within: number?,
}
export type Check_script_idle = {
    label: string?,
    within: number?,
}
export type Check_script_receipt = {
    prefix: string?,
    label: string?,
    within: number?,
}
export type Check_script_running = {
    label: string?,
    within: number?,
}
export type Check_signal = {
    name: string?,
    label: string?,
    within: number?,
}
export type Check_silent = {
    after: number?,
    ticks: number?,
    stats: any?,
    inventories: any?,
    vars: any?,
    messages: any?,
    dialogue: boolean?,
    exact: string?,
    label: string?,
    within: number?,
}
export type Check_standing_bank_closed = {
    stand: Tile?,
    label: string?,
    within: number?,
}
export type Check_stat = {
    id: number?,
    min: number?,
    label: string?,
    within: number?,
}
export type Check_stat_at_most = {
    id: number?,
    max: number?,
    label: string?,
    within: number?,
}
export type Check_stat_value = {
    skill: string?,
    base: Amount?,
    effective: Amount?,
    xp: Amount?,
    label: string?,
    within: number?,
}
export type Check_stat_xp_gain = {
    id: number?,
    min: number?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_tile = {
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    label: string?,
    within: number?,
}
export type Check_tutorial_closed = {
    label: string?,
    within: number?,
}
export type Check_var = {
    id: number?,
    value: Amount?,
    label: string?,
    within: number?,
}
export type Check_varp = {
    id: number?,
    min: number?,
    label: string?,
    within: number?,
}
export type Check_varp_exact = {
    id: number?,
    value: number?,
    label: string?,
    within: number?,
}
export type Check_visited_within = {
    min_x: number?,
    max_x: number?,
    min_z: number?,
    max_z: number?,
    level: number?,
    label: string?,
    within: number?,
}
export type Check_xp_delta = {
    id: number?,
    min: number?,
    since: number?,
    label: string?,
    within: number?,
}
export type Check_xp_since = {
    skill: string?,
    since: number?,
    amount: Amount?,
    label: string?,
    within: number?,
}
export type Check_world_speed_changed = {
    ms: number?,
    label: string?,
    within: number?,
}

declare scenario: {
    case: string,
    role: string,
    step: (label: string, why: string) -> (),
    phase: (name: "fixture" | "product") -> (),
    mark: (label: string?) -> Mark,
    note: (key: string, value: string) -> (),
    signal: (name: string) -> (),
    seal: () -> (),
    finish: (options: { logout: boolean? }?) -> never,
    relog: (options: { why: string }) -> (),
    shot: (options: { label: string, tab: string?, settle: "frame"?, again_after_s: number? }) -> (),
    expect: {
        action_sent: (args: Check_action_sent) -> (),
        active_continue: (args: Check_active_continue) -> (),
        all: (args: Check_all) -> (),
        amount_prompt: (args: Check_amount_prompt) -> (),
        arrived: (args: Check_arrived) -> (),
        arrived_near: (args: Check_arrived_near) -> (),
        arrived_ring: (args: Check_arrived_ring) -> (),
        bank_closed: (args: Check_bank_closed) -> (),
        bank_exact: (args: Check_bank_exact) -> (),
        bank_item: (args: Check_bank_item) -> (),
        bank_item_id: (args: Check_bank_item_id) -> (),
        bank_item_id_any: (args: Check_bank_item_id_any) -> (),
        bank_item_id_at_most: (args: Check_bank_item_id_at_most) -> (),
        bank_item_at_most: (args: Check_bank_item_at_most) -> (),
        bank_open: (args: Check_bank_open) -> (),
        chat: (args: Check_chat) -> (),
        chat_closed: (args: Check_chat_closed) -> (),
        chat_choice: (args: Check_chat_choice) -> (),
        clue_replaced: (args: Check_clue_replaced) -> (),
        clue_protect_window: (args: Check_clue_protect_window) -> (),
        container_unchanged: (args: Check_container_unchanged) -> (),
        count: (args: Check_count) -> (),
        dialogue: (args: Check_dialogue) -> (),
        equipment_id: (args: Check_equipment_id) -> (),
        essence_mine: (args: Check_essence_mine) -> (),
        fresh_chat: (args: Check_fresh_chat) -> (),
        fresh_chat_since_previous_step: (args: Check_fresh_chat_since_previous_step) -> (),
        fresh_quest_completion: (args: Check_fresh_quest_completion) -> (),
        fresh_stat_xp_gain: (args: Check_fresh_stat_xp_gain) -> (),
        fresh_tutorial: (args: Check_fresh_tutorial) -> (),
        ground_item_ids_absent_near: (args: Check_ground_item_ids_absent_near) -> (),
        hint: (args: Check_hint) -> (),
        ingame_scene_2: (args: Check_ingame_scene_2) -> (),
        input_consumed: (args: Check_input_consumed) -> (),
        item: (args: Check_item) -> (),
        item_at_most: (args: Check_item_at_most) -> (),
        item_count: (args: Check_item_count) -> (),
        item_delta: (args: Check_item_delta) -> (),
        item_id: (args: Check_item_id) -> (),
        item_id_any: (args: Check_item_id_any) -> (),
        item_id_at_most: (args: Check_item_id_at_most) -> (),
        item_id_in_full_pack: (args: Check_item_id_in_full_pack) -> (),
        loc_action_near: (args: Check_loc_action_near) -> (),
        loc_id_near: (args: Check_loc_id_near) -> (),
        loc: (args: Check_loc) -> (),
        logged_out: (args: Check_logged_out) -> (),
        login_packets: (args: Check_login_packets) -> (),
        local_targeting_npc_name: (args: Check_local_targeting_npc_name) -> (),
        npc: (args: Check_npc) -> (),
        npc_at: (args: Check_npc_at) -> (),
        npc_ids_absent_near: (args: Check_npc_ids_absent_near) -> (),
        npc_name_near: (args: Check_npc_name_near) -> (),
        npc_name_targeting_local: (args: Check_npc_name_targeting_local) -> (),
        npc_name_unengaged_in_sight: (args: Check_npc_name_unengaged_in_sight) -> (),
        npc_near: (args: Check_npc_near) -> (),
        no_active_continue: (args: Check_no_active_continue) -> (),
        order: (args: Check_order) -> (),
        product_delta: (args: Check_product_delta) -> (),
        quest_done: (args: Check_quest_done) -> (),
        received: (args: Check_received) -> (),
        render_view_ready: (args: Check_render_view_ready) -> (),
        side_tab_available: (args: Check_side_tab_available) -> (),
        script_idle: (args: Check_script_idle) -> (),
        script_receipt: (args: Check_script_receipt) -> (),
        script_running: (args: Check_script_running) -> (),
        signal: (args: Check_signal) -> (),
        silent: (args: Check_silent) -> (),
        standing_bank_closed: (args: Check_standing_bank_closed) -> (),
        stat: (args: Check_stat) -> (),
        stat_at_most: (args: Check_stat_at_most) -> (),
        stat_value: (args: Check_stat_value) -> (),
        stat_xp_gain: (args: Check_stat_xp_gain) -> (),
        tile: (args: Check_tile) -> (),
        tutorial_closed: (args: Check_tutorial_closed) -> (),
        var: (args: Check_var) -> (),
        varp: (args: Check_varp) -> (),
        varp_exact: (args: Check_varp_exact) -> (),
        visited_within: (args: Check_visited_within) -> (),
        xp_delta: (args: Check_xp_delta) -> (),
        xp_since: (args: Check_xp_since) -> (),
        world_speed_changed: (args: Check_world_speed_changed) -> (),
        signal: (args: { name: string, label: string?, within: number? }) -> (),
        server_var: (args: { name: string, value: number | string, label: string?, within: number? }) -> (),
    },
    watch: {
        action_sent: (args: Check_action_sent) -> Handle,
        active_continue: (args: Check_active_continue) -> Handle,
        all: (args: Check_all) -> Handle,
        amount_prompt: (args: Check_amount_prompt) -> Handle,
        arrived: (args: Check_arrived) -> Handle,
        arrived_near: (args: Check_arrived_near) -> Handle,
        arrived_ring: (args: Check_arrived_ring) -> Handle,
        bank_closed: (args: Check_bank_closed) -> Handle,
        bank_exact: (args: Check_bank_exact) -> Handle,
        bank_item: (args: Check_bank_item) -> Handle,
        bank_item_id: (args: Check_bank_item_id) -> Handle,
        bank_item_id_any: (args: Check_bank_item_id_any) -> Handle,
        bank_item_id_at_most: (args: Check_bank_item_id_at_most) -> Handle,
        bank_item_at_most: (args: Check_bank_item_at_most) -> Handle,
        bank_open: (args: Check_bank_open) -> Handle,
        chat: (args: Check_chat) -> Handle,
        chat_closed: (args: Check_chat_closed) -> Handle,
        chat_choice: (args: Check_chat_choice) -> Handle,
        clue_replaced: (args: Check_clue_replaced) -> Handle,
        clue_protect_window: (args: Check_clue_protect_window) -> Handle,
        container_unchanged: (args: Check_container_unchanged) -> Handle,
        count: (args: Check_count) -> Handle,
        dialogue: (args: Check_dialogue) -> Handle,
        equipment_id: (args: Check_equipment_id) -> Handle,
        essence_mine: (args: Check_essence_mine) -> Handle,
        fresh_chat: (args: Check_fresh_chat) -> Handle,
        fresh_chat_since_previous_step: (args: Check_fresh_chat_since_previous_step) -> Handle,
        fresh_quest_completion: (args: Check_fresh_quest_completion) -> Handle,
        fresh_stat_xp_gain: (args: Check_fresh_stat_xp_gain) -> Handle,
        fresh_tutorial: (args: Check_fresh_tutorial) -> Handle,
        ground_item_ids_absent_near: (args: Check_ground_item_ids_absent_near) -> Handle,
        hint: (args: Check_hint) -> Handle,
        ingame_scene_2: (args: Check_ingame_scene_2) -> Handle,
        input_consumed: (args: Check_input_consumed) -> Handle,
        item: (args: Check_item) -> Handle,
        item_at_most: (args: Check_item_at_most) -> Handle,
        item_count: (args: Check_item_count) -> Handle,
        item_delta: (args: Check_item_delta) -> Handle,
        item_id: (args: Check_item_id) -> Handle,
        item_id_any: (args: Check_item_id_any) -> Handle,
        item_id_at_most: (args: Check_item_id_at_most) -> Handle,
        item_id_in_full_pack: (args: Check_item_id_in_full_pack) -> Handle,
        loc_action_near: (args: Check_loc_action_near) -> Handle,
        loc_id_near: (args: Check_loc_id_near) -> Handle,
        loc: (args: Check_loc) -> Handle,
        logged_out: (args: Check_logged_out) -> Handle,
        login_packets: (args: Check_login_packets) -> Handle,
        local_targeting_npc_name: (args: Check_local_targeting_npc_name) -> Handle,
        npc: (args: Check_npc) -> Handle,
        npc_at: (args: Check_npc_at) -> Handle,
        npc_ids_absent_near: (args: Check_npc_ids_absent_near) -> Handle,
        npc_name_near: (args: Check_npc_name_near) -> Handle,
        npc_name_targeting_local: (args: Check_npc_name_targeting_local) -> Handle,
        npc_name_unengaged_in_sight: (args: Check_npc_name_unengaged_in_sight) -> Handle,
        npc_near: (args: Check_npc_near) -> Handle,
        no_active_continue: (args: Check_no_active_continue) -> Handle,
        order: (args: Check_order) -> Handle,
        product_delta: (args: Check_product_delta) -> Handle,
        quest_done: (args: Check_quest_done) -> Handle,
        received: (args: Check_received) -> Handle,
        render_view_ready: (args: Check_render_view_ready) -> Handle,
        side_tab_available: (args: Check_side_tab_available) -> Handle,
        script_idle: (args: Check_script_idle) -> Handle,
        script_receipt: (args: Check_script_receipt) -> Handle,
        script_running: (args: Check_script_running) -> Handle,
        signal: (args: Check_signal) -> Handle,
        silent: (args: Check_silent) -> Handle,
        standing_bank_closed: (args: Check_standing_bank_closed) -> Handle,
        stat: (args: Check_stat) -> Handle,
        stat_at_most: (args: Check_stat_at_most) -> Handle,
        stat_value: (args: Check_stat_value) -> Handle,
        stat_xp_gain: (args: Check_stat_xp_gain) -> Handle,
        tile: (args: Check_tile) -> Handle,
        tutorial_closed: (args: Check_tutorial_closed) -> Handle,
        var: (args: Check_var) -> Handle,
        varp: (args: Check_varp) -> Handle,
        varp_exact: (args: Check_varp_exact) -> Handle,
        visited_within: (args: Check_visited_within) -> Handle,
        xp_delta: (args: Check_xp_delta) -> Handle,
        xp_since: (args: Check_xp_since) -> Handle,
        world_speed_changed: (args: Check_world_speed_changed) -> Handle,
        signal: (args: { name: string, label: string?, within: number? }) -> Handle,
    },
    probe: {
        action_sent: (args: Check_action_sent) -> boolean,
        active_continue: (args: Check_active_continue) -> boolean,
        all: (args: Check_all) -> boolean,
        amount_prompt: (args: Check_amount_prompt) -> boolean,
        arrived: (args: Check_arrived) -> boolean,
        arrived_near: (args: Check_arrived_near) -> boolean,
        arrived_ring: (args: Check_arrived_ring) -> boolean,
        bank_closed: (args: Check_bank_closed) -> boolean,
        bank_exact: (args: Check_bank_exact) -> boolean,
        bank_item: (args: Check_bank_item) -> boolean,
        bank_item_id: (args: Check_bank_item_id) -> boolean,
        bank_item_id_any: (args: Check_bank_item_id_any) -> boolean,
        bank_item_id_at_most: (args: Check_bank_item_id_at_most) -> boolean,
        bank_item_at_most: (args: Check_bank_item_at_most) -> boolean,
        bank_open: (args: Check_bank_open) -> boolean,
        chat: (args: Check_chat) -> boolean,
        chat_closed: (args: Check_chat_closed) -> boolean,
        chat_choice: (args: Check_chat_choice) -> boolean,
        clue_replaced: (args: Check_clue_replaced) -> boolean,
        clue_protect_window: (args: Check_clue_protect_window) -> boolean,
        container_unchanged: (args: Check_container_unchanged) -> boolean,
        count: (args: Check_count) -> boolean,
        dialogue: (args: Check_dialogue) -> boolean,
        equipment_id: (args: Check_equipment_id) -> boolean,
        essence_mine: (args: Check_essence_mine) -> boolean,
        fresh_chat: (args: Check_fresh_chat) -> boolean,
        fresh_chat_since_previous_step: (args: Check_fresh_chat_since_previous_step) -> boolean,
        fresh_quest_completion: (args: Check_fresh_quest_completion) -> boolean,
        fresh_stat_xp_gain: (args: Check_fresh_stat_xp_gain) -> boolean,
        fresh_tutorial: (args: Check_fresh_tutorial) -> boolean,
        ground_item_ids_absent_near: (args: Check_ground_item_ids_absent_near) -> boolean,
        hint: (args: Check_hint) -> boolean,
        ingame_scene_2: (args: Check_ingame_scene_2) -> boolean,
        input_consumed: (args: Check_input_consumed) -> boolean,
        item: (args: Check_item) -> boolean,
        item_at_most: (args: Check_item_at_most) -> boolean,
        item_count: (args: Check_item_count) -> boolean,
        item_delta: (args: Check_item_delta) -> boolean,
        item_id: (args: Check_item_id) -> boolean,
        item_id_any: (args: Check_item_id_any) -> boolean,
        item_id_at_most: (args: Check_item_id_at_most) -> boolean,
        item_id_in_full_pack: (args: Check_item_id_in_full_pack) -> boolean,
        loc_action_near: (args: Check_loc_action_near) -> boolean,
        loc_id_near: (args: Check_loc_id_near) -> boolean,
        loc: (args: Check_loc) -> boolean,
        logged_out: (args: Check_logged_out) -> boolean,
        login_packets: (args: Check_login_packets) -> boolean,
        local_targeting_npc_name: (args: Check_local_targeting_npc_name) -> boolean,
        npc: (args: Check_npc) -> boolean,
        npc_at: (args: Check_npc_at) -> boolean,
        npc_ids_absent_near: (args: Check_npc_ids_absent_near) -> boolean,
        npc_name_near: (args: Check_npc_name_near) -> boolean,
        npc_name_targeting_local: (args: Check_npc_name_targeting_local) -> boolean,
        npc_name_unengaged_in_sight: (args: Check_npc_name_unengaged_in_sight) -> boolean,
        npc_near: (args: Check_npc_near) -> boolean,
        no_active_continue: (args: Check_no_active_continue) -> boolean,
        order: (args: Check_order) -> boolean,
        product_delta: (args: Check_product_delta) -> boolean,
        quest_done: (args: Check_quest_done) -> boolean,
        received: (args: Check_received) -> boolean,
        render_view_ready: (args: Check_render_view_ready) -> boolean,
        side_tab_available: (args: Check_side_tab_available) -> boolean,
        script_idle: (args: Check_script_idle) -> boolean,
        script_receipt: (args: Check_script_receipt) -> boolean,
        script_running: (args: Check_script_running) -> boolean,
        signal: (args: Check_signal) -> boolean,
        silent: (args: Check_silent) -> boolean,
        standing_bank_closed: (args: Check_standing_bank_closed) -> boolean,
        stat: (args: Check_stat) -> boolean,
        stat_at_most: (args: Check_stat_at_most) -> boolean,
        stat_value: (args: Check_stat_value) -> boolean,
        stat_xp_gain: (args: Check_stat_xp_gain) -> boolean,
        tile: (args: Check_tile) -> boolean,
        tutorial_closed: (args: Check_tutorial_closed) -> boolean,
        var: (args: Check_var) -> boolean,
        varp: (args: Check_varp) -> boolean,
        varp_exact: (args: Check_varp_exact) -> boolean,
        visited_within: (args: Check_visited_within) -> boolean,
        xp_delta: (args: Check_xp_delta) -> boolean,
        xp_since: (args: Check_xp_since) -> boolean,
        world_speed_changed: (args: Check_world_speed_changed) -> boolean,
        signal: (args: { name: string, label: string?, within: number? }) -> boolean,
    },
    seed: {
        teleport: (args: { why: string, x: number, z: number, level: number }) -> (),
        give: (args: { why: string, item: string, count: number }) -> (),
        give_bank: (args: { why: string, item: string, count: number }) -> (),
        clear_inventory: (args: { why: string }) -> (),
        clear_bank: (args: { why: string }) -> (),
        set_stat: (args: { why: string, skill: string, level: number }) -> (),
        reset_and_advance_stat: (args: { why: string, skill: string, level: number }) -> (),
        set_var: (args: { why: string, name: string, value: number, confirmation: ("state" | "operation")? }) -> (),
        energy_full_run_on: (args: { why: string }) -> (),
        ext: (args: { op: string, why: string, [string]: any }) -> (),
    },
}
