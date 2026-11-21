use super::{Driver, SendReason};
use crate::snapshot::{npc_menu_form, GameSnapshot, NpcView, WidgetRoot, WidgetView};
use client::client::{Client, MiniMenuAction};
use client::config::if_type::ComponentType;

/// A received identity whose operation requires a current client-built row.
#[derive(Clone, Copy)]
pub enum NativeMenuTarget<'a> {
    /// Operation one on an NPC whose current operation is Talk-to.
    NpcTalk(&'a NpcView),
    /// One exact visible component and its requested minimenu action.
    Component {
        /// The received component identity and open-root path.
        widget: &'a WidgetView,
        /// The client's minimenu action, not a network opcode.
        action: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    action: i32,
    a: i32,
    b: i32,
    c: i32,
}

fn select_row(client: &Client, wanted: Row) -> Result<i32, SendReason> {
    let count = usize::try_from(client.menu_num_entries)
        .ok()
        .filter(|&count| {
            count <= client.menu_action.len()
                && count <= client.menu_param_a.len()
                && count <= client.menu_param_b.len()
                && count <= client.menu_param_c.len()
                && count <= client.menu_option.len()
        })
        .ok_or(SendReason::MenuUnavailable)?;
    let mut selected = None;
    for index in 0..count {
        let action = client.menu_action[index];
        let row = Row {
            action: if action >= MiniMenuAction::_PRIORITY {
                action - MiniMenuAction::_PRIORITY
            } else {
                action
            },
            a: client.menu_param_a[index],
            b: client.menu_param_b[index],
            c: client.menu_param_c[index],
        };
        if row == wanted && selected.replace(index as i32).is_some() {
            return Err(SendReason::MenuAmbiguous);
        }
    }
    selected.ok_or(SendReason::MenuUnavailable)
}

fn check_scene(client: &Client, snapshot: &GameSnapshot) -> Result<(), SendReason> {
    if !client.ingame || !snapshot.ingame() {
        return Err(SendReason::NotIngame);
    }
    let gens = snapshot.gens();
    if snapshot.native_menu_scope() != Some((client.gens.session, client.gens.invalidations)) {
        return Err(SendReason::StaleSession);
    }
    let scene = snapshot.scene();
    if client.scene_state != super::SCENE_READY
        || snapshot.scene_state() != super::SCENE_READY
        || !scene.available
    {
        return Err(SendReason::SceneUnavailable);
    }
    if gens.scene != client.gens.scene
        || scene.base_x != client.map_build_base_x
        || scene.base_z != client.map_build_base_z
        || scene.level != client.minusedlevel
    {
        return Err(SendReason::StaleScene);
    }
    if client.obj_drag_area != 0 || client.is_menu_open || client.dialog_input_open {
        return Err(SendReason::MenuIneligible);
    }
    Ok(())
}

fn npc_row(
    client: &Client,
    snapshot: &GameSnapshot,
    npc: &NpcView,
) -> Result<(usize, Row), SendReason> {
    if snapshot.gens().varp != client.gens.varp || snapshot.gens().npc_info != client.gens.npc_info
    {
        return Err(SendReason::StaleTarget);
    }
    let present = usize::try_from(client.npc_count)
        .ok()
        .and_then(|count| client.npc_ids.get(..count))
        .is_some_and(|ids| {
            ids.iter()
                .any(|&id| usize::try_from(id).ok() == Some(npc.index))
        });
    let live = client.npc.get(npc.index).and_then(Option::as_deref);
    let live = live.filter(|_| present).ok_or(SendReason::StaleTarget)?;
    let placed = live
        .r#type
        .filter(|id| Some(*id) == npc.r#type)
        .ok_or(SendReason::StaleTarget)?;
    // The placed type and unchanged received varps identify the same resolved form.
    let form = npc_menu_form(client, placed).ok_or(SendReason::MenuIneligible)?;
    if !form
        .op
        .first()
        .and_then(Option::as_deref)
        .is_some_and(|op| op.eq_ignore_ascii_case("Talk-to"))
        || !npc
            .actions
            .first()
            .and_then(Option::as_deref)
            .is_some_and(|op| op.eq_ignore_ascii_case("Talk-to"))
        || npc.name.as_deref() != Some(form.name.as_str())
    {
        return Err(SendReason::MenuIneligible);
    }
    let (x, z) = (live.route_x[0], live.route_z[0]);
    let scene = snapshot.scene();
    if x < 0 || z < 0 || x >= scene.width || z >= scene.height {
        return Err(SendReason::OffScene);
    }
    if npc.network.x != x + scene.base_x
        || npc.network.z != z + scene.base_z
        || npc.network.level != scene.level
    {
        return Err(SendReason::StaleTarget);
    }
    if client.local_player.is_none()
        || client.main_modal_id != -1
        || client.use_mode != 0
        || client.target_mode != 0
    {
        return Err(SendReason::MenuIneligible);
    }
    Ok((
        form.id as usize,
        Row {
            action: MiniMenuAction::OP_NPC1,
            a: i32::try_from(npc.index).map_err(|_| SendReason::StaleTarget)?,
            b: x,
            c: z,
        },
    ))
}

#[derive(Clone, Copy)]
struct Bounds {
    x: i32,
    y: i32,
    right: i32,
    bottom: i32,
}

impl Bounds {
    fn intersect(self, other: Self) -> Result<Self, SendReason> {
        let bounds = Self {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        if bounds.x >= bounds.right || bounds.y >= bounds.bottom {
            Err(SendReason::MenuIneligible)
        } else {
            Ok(bounds)
        }
    }
}

// The origins and strict viewport clips are Client::build_minimenu's own regions.
// vendor/fr-client-rust/crates/client/src/client/client.rs:8084-8157.
fn root_bounds(client: &Client, widget: &WidgetView) -> Result<Bounds, SendReason> {
    let (root, bounds) = match widget.root {
        WidgetRoot::Main => (
            client.main_modal_id,
            Bounds {
                x: 5,
                y: 5,
                right: 516,
                bottom: 338,
            },
        ),
        WidgetRoot::Side => {
            let root = if client.side_modal_id != -1 {
                client.side_modal_id
            } else {
                usize::try_from(client.active_icon)
                    .ok()
                    .and_then(|i| client.side_icon.get(i))
                    .copied()
                    .unwrap_or(-1)
            };
            (
                root,
                Bounds {
                    x: 554,
                    y: 206,
                    right: 743,
                    bottom: 466,
                },
            )
        }
        WidgetRoot::Chat => (
            client.chat_modal_id,
            Bounds {
                x: 18,
                y: 358,
                right: 496,
                bottom: 453,
            },
        ),
        WidgetRoot::Tutorial => return Err(SendReason::MenuIneligible),
    };
    if root < 0 || root != widget.root_component_id {
        return Err(SendReason::StaleTarget);
    }
    Ok(bounds)
}

fn component_bounds(
    client: &Client,
    snapshot: &GameSnapshot,
    widget: &WidgetView,
    depth: usize,
) -> Result<(i32, i32, Bounds), SendReason> {
    if depth == 0 {
        return Err(SendReason::StaleTarget);
    }
    let live = usize::try_from(widget.component_id)
        .ok()
        .and_then(|id| client.if_(id))
        .ok_or(SendReason::StaleTarget)?;
    if live.id != widget.component_id
        || live.button_type != widget.button_type
        || live.r#type != widget.type_
    {
        return Err(SendReason::StaleTarget);
    }
    if live.hide {
        return Err(SendReason::MenuIneligible);
    }
    let (x, y, clip) = if widget.parent_id == -1 {
        if widget.component_id != widget.root_component_id
            || live.r#type != ComponentType::TYPE_LAYER
        {
            return Err(SendReason::MenuIneligible);
        }
        let clip = root_bounds(client, widget)?;
        let (x, y) = match widget.root {
            WidgetRoot::Main => (4, 4),
            WidgetRoot::Side => (553, 205),
            WidgetRoot::Chat => (17, 357),
            WidgetRoot::Tutorial => return Err(SendReason::MenuIneligible),
        };
        (x, y, clip)
    } else {
        let mut parents = snapshot.widgets().iter().filter(|parent| {
            parent.component_id == widget.parent_id
                && parent.root == widget.root
                && parent.root_component_id == widget.root_component_id
        });
        let parent = parents.next().ok_or(SendReason::StaleTarget)?;
        if parents.next().is_some() {
            return Err(SendReason::MenuAmbiguous);
        }
        let (px, py, clip) = component_bounds(client, snapshot, parent, depth - 1)?;
        let parent_live = client
            .if_(parent.component_id as usize)
            .ok_or(SendReason::StaleTarget)?;
        if parent_live.r#type != ComponentType::TYPE_LAYER {
            return Err(SendReason::MenuIneligible);
        }
        let mut positions = parent_live
            .children
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter(|(_, id)| **id == widget.component_id);
        let index = positions
            .next()
            .map(|(index, _)| index)
            .ok_or(SendReason::StaleTarget)?;
        if positions.next().is_some() {
            return Err(SendReason::MenuAmbiguous);
        }
        let dx = parent_live
            .child_x
            .as_deref()
            .and_then(|v| v.get(index))
            .copied()
            .ok_or(SendReason::StaleTarget)?;
        let dy = parent_live
            .child_y
            .as_deref()
            .and_then(|v| v.get(index))
            .copied()
            .ok_or(SendReason::StaleTarget)?;
        let scroll = if parent.parent_id == -1 {
            0
        } else {
            parent_live.scroll_pos
        };
        (px + dx + live.x, py + dy - scroll + live.y, clip)
    };
    let clip = clip.intersect(Bounds {
        x,
        y,
        right: x + live.width,
        bottom: y + live.height,
    })?;
    Ok((x, y, clip))
}

fn component_point(
    client: &Client,
    snapshot: &GameSnapshot,
    widget: &WidgetView,
    action: i32,
) -> Result<(i32, i32), SendReason> {
    if snapshot.gens().iface != client.gens.iface {
        return Err(SendReason::StaleTarget);
    }
    if super::button_opcode(widget.button_type) != Some(action) {
        return Err(SendReason::MenuIneligible);
    }
    let live = usize::try_from(widget.component_id)
        .ok()
        .and_then(|id| client.if_(id))
        .ok_or(SendReason::StaleTarget)?;
    if live.client_code != 0
        || (action == MiniMenuAction::PAUSE_BUTTON && client.resumed_pause_button)
    {
        return Err(SendReason::MenuIneligible);
    }
    let (_, _, bounds) = component_bounds(client, snapshot, widget, snapshot.widgets().len())?;
    Ok((
        bounds.x + (bounds.right - bounds.x) / 2,
        bounds.y + (bounds.bottom - bounds.y) / 2,
    ))
}

// Client's fixed menu capacity. Move strings instead of cloning menu allocations.
// vendor/fr-client-rust/crates/client/src/client/client.rs:79.
const MENU_CAPACITY: usize = 500;

struct SavedMenu {
    labels: [String; MENU_CAPACITY],
    actions: [i32; MENU_CAPACITY],
    a: [i32; MENU_CAPACITY],
    b: [i32; MENU_CAPACITY],
    c: [i32; MENU_CAPACITY],
    count: i32,
}

impl SavedMenu {
    fn take(client: &mut Client) -> Result<Self, SendReason> {
        if [
            client.menu_option.len(),
            client.menu_action.len(),
            client.menu_param_a.len(),
            client.menu_param_b.len(),
            client.menu_param_c.len(),
        ]
        .into_iter()
        .any(|len| len != MENU_CAPACITY)
        {
            return Err(SendReason::MenuUnavailable);
        }
        Ok(Self {
            labels: std::array::from_fn(|i| std::mem::take(&mut client.menu_option[i])),
            actions: std::array::from_fn(|i| client.menu_action[i]),
            a: std::array::from_fn(|i| client.menu_param_a[i]),
            b: std::array::from_fn(|i| client.menu_param_b[i]),
            c: std::array::from_fn(|i| client.menu_param_c[i]),
            count: client.menu_num_entries,
        })
    }

    fn restore(&mut self, client: &mut Client) {
        for (current, saved) in client.menu_option.iter_mut().zip(&mut self.labels) {
            *current = std::mem::take(saved);
        }
        client.menu_action.copy_from_slice(&self.actions);
        client.menu_param_a.copy_from_slice(&self.a);
        client.menu_param_b.copy_from_slice(&self.b);
        client.menu_param_c.copy_from_slice(&self.c);
        client.menu_num_entries = self.count;
    }
}

struct BuilderState {
    mouse: (i32, i32),
    hover: [i32; 6],
    redraw: (bool, bool),
    scroll: (i32, bool, i32),
}

impl BuilderState {
    fn capture(client: &Client) -> Self {
        Self {
            mouse: (client.shell.mouse_x, client.shell.mouse_y),
            hover: [
                client.last_over_com_id,
                client.over_main_com_id,
                client.over_side_com_id,
                client.over_chat_com_id,
                client.hovered_slot,
                client.hovered_slot_com_id,
            ],
            redraw: (client.redraw_side, client.redraw_chat),
            scroll: (
                client.scroll_cycle,
                client.scroll_grabbed,
                client.scroll_input_padding,
            ),
        }
    }

    fn restore(self, client: &mut Client) {
        (client.shell.mouse_x, client.shell.mouse_y) = self.mouse;
        [
            client.last_over_com_id,
            client.over_main_com_id,
            client.over_side_com_id,
            client.over_chat_com_id,
            client.hovered_slot,
            client.hovered_slot_com_id,
        ] = self.hover;
        (client.redraw_side, client.redraw_chat) = self.redraw;
        (
            client.scroll_cycle,
            client.scroll_grabbed,
            client.scroll_input_padding,
        ) = self.scroll;
    }
}

struct MenuScope<'a> {
    client: &'a mut Client,
    saved: SavedMenu,
    builder: Option<BuilderState>,
}

impl MenuScope<'_> {
    fn restore_builder(&mut self) {
        if let Some(builder) = self.builder.take() {
            builder.restore(self.client);
        }
    }
}

impl Drop for MenuScope<'_> {
    fn drop(&mut self) {
        self.restore_builder();
        self.saved.restore(self.client);
    }
}

pub(super) fn dispatch(
    client: &mut Client,
    snapshot: &GameSnapshot,
    target: NativeMenuTarget<'_>,
) -> Result<(), SendReason> {
    check_scene(client, snapshot)?;
    let (npc_type, point, wanted) = match target {
        NativeMenuTarget::NpcTalk(npc) => {
            let (npc_type, row) = npc_row(client, snapshot, npc)?;
            (Some(npc_type), (0, 0), row)
        }
        NativeMenuTarget::Component { widget, action } => {
            let point = component_point(client, snapshot, widget, action)?;
            (
                None,
                point,
                Row {
                    action,
                    a: 0,
                    b: 0,
                    c: widget.component_id,
                },
            )
        }
    };
    let builder = BuilderState::capture(client);
    let saved = SavedMenu::take(client)?;
    let mut scope = MenuScope {
        client,
        saved,
        builder: Some(builder),
    };
    (scope.client.shell.mouse_x, scope.client.shell.mouse_y) = point;
    // Menu construction is a query, not a held scrollbar gesture or a click.
    // do_scrollbar mutates positions only when scroll_cycle is nonzero (client.rs:11120-11142).
    scope.client.scroll_cycle = 0;
    scope.client.build_minimenu();
    if let Some(npc_type) = npc_type {
        scope
            .client
            .add_npc_options(npc_type as i32, wanted.a, wanted.b, wanted.c);
    }
    let index = select_row(scope.client, wanted)?;
    scope.restore_builder();
    let before = scope.client.out.pos;
    if !Driver::do_action(scope.client, index) || scope.client.out.pos <= before {
        return Err(SendReason::DriverRejected);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::client::ClientConfig;

    fn client() -> Client {
        Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: String::new(),
            members: true,
            lowmem: false,
        })
    }

    #[test]
    fn selects_exact_action_kind_and_all_parameters_not_slot_zero() {
        let mut client = client();
        let wanted = Row {
            action: MiniMenuAction::OP_NPC1,
            a: 7,
            b: 20,
            c: 21,
        };
        client.menu_num_entries = 6;
        for i in 0..6 {
            client.menu_action[i] = wanted.action;
            client.menu_param_a[i] = wanted.a;
            client.menu_param_b[i] = wanted.b;
            client.menu_param_c[i] = wanted.c;
        }
        client.menu_action[0] = MiniMenuAction::CANCEL;
        client.menu_action[1] = MiniMenuAction::OP_PLAYER1;
        client.menu_param_a[2] += 1;
        client.menu_param_b[3] += 1;
        client.menu_param_c[4] += 1;
        client.menu_action[5] += MiniMenuAction::_PRIORITY;
        assert_eq!(select_row(&client, wanted), Ok(5));
        client.menu_action[5] = MiniMenuAction::CANCEL;
        assert_eq!(
            select_row(&client, wanted),
            Err(SendReason::MenuUnavailable)
        );
    }

    #[test]
    fn refuses_ambiguous_and_unpublished_rows() {
        let mut client = client();
        let wanted = Row {
            action: MiniMenuAction::IF_BUTTON,
            a: 0,
            b: 0,
            c: 42,
        };
        client.menu_num_entries = 2;
        for i in 0..2 {
            client.menu_action[i] = wanted.action;
            client.menu_param_c[i] = wanted.c;
        }
        assert_eq!(select_row(&client, wanted), Err(SendReason::MenuAmbiguous));
        client.menu_num_entries = 0;
        assert_eq!(
            select_row(&client, wanted),
            Err(SendReason::MenuUnavailable)
        );
        client.menu_num_entries = 501;
        assert_eq!(
            select_row(&client, wanted),
            Err(SendReason::MenuUnavailable)
        );
    }

    #[test]
    fn saved_menu_moves_storage_without_allocating() {
        let mut client = client();
        client.menu_num_entries = 1;
        client.menu_option[0] = "Previous row".into();
        let pointer = client.menu_option[0].as_ptr();
        let allocation = allocation_counter::measure(|| {
            let mut saved = SavedMenu::take(&mut client).unwrap();
            saved.restore(&mut client);
        });
        assert_eq!(allocation.count_total, 0);
        assert_eq!(client.menu_option[0].as_ptr(), pointer);
        assert_eq!(client.menu_num_entries, 1);
    }

    fn ready() -> (Client, GameSnapshot) {
        let mut client = client();
        client.ingame = true;
        client.scene_state = super::super::SCENE_READY;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(super::super::SCENE_READY);
        snapshot.seed_scene(crate::snapshot::SceneView {
            available: true,
            width: 104,
            height: 104,
            ..Default::default()
        });
        (client, snapshot)
    }

    #[test]
    fn refuses_disconnected_unready_changed_session_and_changed_scene() {
        let (mut client, snapshot) = ready();
        assert_eq!(check_scene(&client, &snapshot), Ok(()));
        client.ingame = false;
        assert_eq!(check_scene(&client, &snapshot), Err(SendReason::NotIngame));
        client.ingame = true;
        client.scene_state = 1;
        assert_eq!(
            check_scene(&client, &snapshot),
            Err(SendReason::SceneUnavailable)
        );
        client.scene_state = super::super::SCENE_READY;
        client.gens.session += 1;
        assert_eq!(
            check_scene(&client, &snapshot),
            Err(SendReason::StaleSession)
        );
        client.gens.session -= 1;
        client.gens.invalidations += 1;
        assert_eq!(
            check_scene(&client, &snapshot),
            Err(SendReason::StaleSession)
        );
        client.gens.invalidations -= 1;
        client.gens.scene += 1;
        assert_eq!(check_scene(&client, &snapshot), Err(SendReason::StaleScene));
        client.gens.scene -= 1;
        client.map_build_base_x = 1;
        assert_eq!(check_scene(&client, &snapshot), Err(SendReason::StaleScene));
        client.map_build_base_x = 0;
        client.map_build_base_z = 1;
        assert_eq!(check_scene(&client, &snapshot), Err(SendReason::StaleScene));
        client.map_build_base_z = 0;
        client.minusedlevel = 1;
        assert_eq!(check_scene(&client, &snapshot), Err(SendReason::StaleScene));
    }

    #[test]
    fn refuses_drag_open_menu_and_amount_prompt_without_mutation() {
        let (mut client, snapshot) = ready();
        client.obj_drag_area = 1;
        assert_eq!(
            check_scene(&client, &snapshot),
            Err(SendReason::MenuIneligible)
        );
        assert_eq!(client.obj_drag_area, 1);
        client.obj_drag_area = 0;
        client.is_menu_open = true;
        assert_eq!(
            check_scene(&client, &snapshot),
            Err(SendReason::MenuIneligible)
        );
        assert!(client.is_menu_open);
        client.is_menu_open = false;
        client.dialog_input_open = true;
        assert_eq!(
            check_scene(&client, &snapshot),
            Err(SendReason::MenuIneligible)
        );
        assert!(client.dialog_input_open);
        assert_eq!(client.out.pos, 0);
    }
}
