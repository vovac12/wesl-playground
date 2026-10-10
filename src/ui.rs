//! Project dropdown, status overlay and in-world project labels.

use bevy::{
    camera_controller::free_camera::FreeCameraState,
    input::mouse::{MouseScrollUnit, MouseWheel},
    picking::hover::{HoverMap, Hovered},
    prelude::*,
    ui::IsDefaultUiCamera,
    ui_widgets::{Activate, Button},
};

use crate::{
    bridge::ViewTarget,
    playground::{CatalogRes, CurrentView, MainCamera},
    project::Catalog,
};

const PANEL_BG: Color = Color::srgba(0.08, 0.08, 0.1, 0.92);
const ITEM_BG: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);
const ITEM_HOVER: Color = Color::srgba(0.3, 0.35, 0.5, 0.6);
const ITEM_SELECTED: Color = Color::srgba(0.25, 0.4, 0.7, 0.6);
const TEXT: Color = Color::srgb(0.9, 0.9, 0.92);
const TEXT_DIM: Color = Color::srgb(0.6, 0.62, 0.68);
const ERROR: Color = Color::srgb(1.0, 0.45, 0.4);

pub struct UiPlugin {
    pub mcp_url: String,
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(McpUrl(self.mcp_url.clone()))
            .init_resource::<MenuState>()
            .add_systems(Startup, setup_ui)
            .add_systems(
                Update,
                (
                    rebuild_menu,
                    update_header,
                    update_item_colors,
                    update_status,
                    update_labels,
                    scroll_menu,
                    keyboard_shortcuts,
                )
                    .after(crate::playground::PlaygroundSystems),
            );
    }
}

#[derive(Resource)]
struct McpUrl(String);

#[derive(Resource, Default)]
struct MenuState {
    open: bool,
    /// Fingerprint of the catalog the menu was built from.
    built_for: Option<String>,
}

#[derive(Component)]
struct MenuToggle;

#[derive(Component)]
struct MenuHeaderText;

#[derive(Component)]
struct MenuPanel;

#[derive(Component)]
struct MenuList;

#[derive(Component)]
struct MenuItem(ViewTarget);

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct ErrorText;

#[derive(Component)]
struct LabelLayer;

#[derive(Component)]
struct ProjectLabel {
    world: Vec3,
}

fn font(size: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn setup_ui(mut commands: Commands, cameras: Query<Entity, With<MainCamera>>, url: Res<McpUrl>) {
    if let Ok(camera) = cameras.single() {
        commands.entity(camera).insert(IsDefaultUiCamera);
    }

    // Labels live in their own full-screen layer below the menu.
    commands.spawn((
        LabelLayer,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        Pickable::IGNORE,
    ));

    let root = commands
        .spawn((
            GlobalZIndex(10),
            Node {
                position_type: PositionType::Absolute,
                top: px(10),
                left: px(10),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                ..default()
            },
        ))
        .id();

    commands
        .spawn((
            MenuToggle,
            Button,
            Hovered::default(),
            Node {
                min_width: px(360),
                padding: UiRect::axes(px(12), px(8)),
                border_radius: BorderRadius::all(px(6)),
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(12),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            children![
                (
                    MenuHeaderText,
                    Text::new("No projects"),
                    font(16.0),
                    TextColor(TEXT)
                ),
                (Text::new("v"), font(16.0), TextColor(TEXT_DIM)),
            ],
            ChildOf(root),
        ))
        .observe(|_: On<Activate>, mut menu: ResMut<MenuState>| menu.open = !menu.open);

    commands.spawn((
        MenuPanel,
        Node {
            display: Display::None,
            flex_direction: FlexDirection::Column,
            min_width: px(360),
            max_height: vh(70),
            padding: UiRect::all(px(4)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        ChildOf(root),
        children![(
            MenuList,
            Node {
                flex_direction: FlexDirection::Column,
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ScrollPosition::default(),
        )],
    ));

    commands.spawn((
        StatusText,
        Text::new(""),
        font(13.0),
        TextColor(TEXT_DIM),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(10),
            left: px(10),
            ..default()
        },
    ));
    commands.spawn((
        ErrorText,
        Text::new(""),
        font(13.0),
        TextColor(ERROR),
        Node {
            position_type: PositionType::Absolute,
            top: px(60),
            right: px(10),
            max_width: px(560),
            ..default()
        },
    ));
    info!(
        "connect Claude Code with: claude mcp add --transport http shadertoy {}",
        url.0
    );
}

fn catalog_fingerprint(catalog: &Catalog) -> String {
    let mut s = String::new();
    for g in &catalog.groups {
        s.push_str(&format!("g:{}:{};", g.id, g.display_name()));
        for p in &g.projects {
            s.push_str(&format!("p:{}:{};", p.id, p.display_name()));
        }
    }
    for p in &catalog.ungrouped {
        s.push_str(&format!("p:{}:{};", p.id, p.display_name()));
    }
    s
}

fn rebuild_menu(
    mut commands: Commands,
    catalog: Res<CatalogRes>,
    mut menu: ResMut<MenuState>,
    list: Query<Entity, With<MenuList>>,
    mut panel: Query<&mut Node, With<MenuPanel>>,
    mut cameras: Query<&mut FreeCameraState>,
) {
    if menu.is_changed() {
        if let Ok(mut node) = panel.single_mut() {
            node.display = if menu.open {
                Display::Flex
            } else {
                Display::None
            };
        }
        // Mouse wheel scrolls the menu instead of changing camera speed while it is open.
        for mut state in &mut cameras {
            state.enabled = !menu.open;
        }
    }
    let fingerprint = catalog_fingerprint(&catalog.catalog);
    if menu.built_for.as_ref() == Some(&fingerprint) {
        return;
    }
    menu.built_for = Some(fingerprint);
    let Ok(list) = list.single() else { return };
    commands.entity(list).despawn_children();

    let item = |commands: &mut Commands,
                target: ViewTarget,
                label: String,
                indent: f32,
                color: Color,
                size: f32| {
        commands
            .spawn((
                MenuItem(target),
                Button,
                Hovered::default(),
                Node {
                    padding: UiRect {
                        left: px(10.0 + indent),
                        right: px(10),
                        top: px(5),
                        bottom: px(5),
                    },
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(ITEM_BG),
                children![(
                    Text::new(label),
                    font(size),
                    TextColor(color),
                    Pickable::IGNORE
                )],
                ChildOf(list),
            ))
            .observe(
                |activate: On<Activate>,
                 items: Query<&MenuItem>,
                 mut view: ResMut<CurrentView>,
                 mut menu: ResMut<MenuState>| {
                    if let Ok(item) = items.get(activate.entity) {
                        view.open(item.0.clone());
                        menu.open = false;
                    }
                },
            );
    };

    let catalog = &catalog.catalog;
    if catalog.groups.is_empty() && catalog.ungrouped.is_empty() {
        commands.spawn((
            Text::new("No projects yet. Ask Claude to create some!"),
            font(14.0),
            TextColor(TEXT_DIM),
            Node {
                padding: UiRect::all(px(10)),
                ..default()
            },
            ChildOf(list),
        ));
    }
    for group in &catalog.groups {
        let label = format!(
            "{}  ({} projects, view all)",
            group.display_name(),
            group.projects.len()
        );
        item(
            &mut commands,
            ViewTarget::Group(group.id.clone()),
            label,
            0.0,
            TEXT,
            15.0,
        );
        for project in &group.projects {
            let mut label = project.display_name();
            if project.manifest.is_err() {
                label.push_str("  [manifest error]");
            }
            item(
                &mut commands,
                ViewTarget::Project(project.id.clone()),
                label,
                18.0,
                TEXT_DIM,
                14.0,
            );
        }
    }
    for project in &catalog.ungrouped {
        let mut label = project.display_name();
        if project.manifest.is_err() {
            label.push_str("  [manifest error]");
        }
        item(
            &mut commands,
            ViewTarget::Project(project.id.clone()),
            label,
            0.0,
            TEXT,
            15.0,
        );
    }
}

fn target_label(target: &ViewTarget, catalog: &Catalog) -> String {
    match target {
        ViewTarget::Group(g) => match catalog.group(g) {
            Some(group) => format!("{} (all)", group.display_name()),
            None => format!("{g} (missing)"),
        },
        ViewTarget::Project(id) => match catalog.find(id) {
            Some(p) => match p.group.as_deref().and_then(|g| catalog.group(g)) {
                Some(group) => format!("{} / {}", group.display_name(), p.display_name()),
                None => p.display_name(),
            },
            None => format!("{id} (missing)"),
        },
    }
}

fn update_header(
    view: Res<CurrentView>,
    catalog: Res<CatalogRes>,
    mut text: Query<&mut Text, With<MenuHeaderText>>,
) {
    if !view.is_changed() && !catalog.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let label = match &view.target {
        Some(target) => target_label(target, &catalog.catalog),
        None => "Select a project".into(),
    };
    if text.0 != label {
        text.0 = label;
    }
}

fn update_item_colors(
    view: Res<CurrentView>,
    mut items: Query<(&MenuItem, &Hovered, &mut BackgroundColor)>,
    mut toggle: Query<(&Hovered, &mut BackgroundColor), (With<MenuToggle>, Without<MenuItem>)>,
) {
    for (item, hovered, mut bg) in &mut items {
        let color = if hovered.get() {
            ITEM_HOVER
        } else if view.target.as_ref() == Some(&item.0) {
            ITEM_SELECTED
        } else {
            ITEM_BG
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
    for (hovered, mut bg) in &mut toggle {
        let color = if hovered.get() {
            Color::srgba(0.15, 0.16, 0.2, 0.95)
        } else {
            PANEL_BG
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
}

fn update_status(
    view: Res<CurrentView>,
    url: Res<McpUrl>,
    camera: Query<&Transform, With<MainCamera>>,
    mut status: Query<&mut Text, (With<StatusText>, Without<ErrorText>)>,
    mut errors: Query<&mut Text, (With<ErrorText>, Without<StatusText>)>,
) {
    if let (Ok(mut status), Ok(cam)) = (status.single_mut(), camera.single()) {
        let p = cam.translation;
        let line = format!(
            "Right mouse: look | WASD/QE: move | Shift: fast | Wheel: speed | R: reload | Tab: projects\nMCP: {}   camera ({:.1}, {:.1}, {:.1})",
            url.0, p.x, p.y, p.z
        );
        if status.0 != line {
            status.0 = line;
        }
    }
    if view.is_changed()
        && let Ok(mut errors) = errors.single_mut()
    {
        errors.0 = view.errors.join("\n");
    }
}

fn update_labels(
    mut commands: Commands,
    view: Res<CurrentView>,
    layer: Query<Entity, With<LabelLayer>>,
    mut labels: Query<(&ProjectLabel, &mut Node, &ComputedNode)>,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut built_generation: Local<u64>,
) {
    let Ok(layer) = layer.single() else { return };
    if *built_generation != view.generation {
        *built_generation = view.generation;
        commands.entity(layer).despawn_children();
        if view.projects.len() > 1 {
            for project in &view.projects {
                commands.spawn((
                    ProjectLabel {
                        world: project.origin + Vec3::Y * (project.radius.min(4.0) * 1.2 + 1.5),
                    },
                    Text::new(project.name.clone()),
                    font(15.0),
                    TextColor(TEXT),
                    TextShadow {
                        offset: Vec2::splat(1.5),
                        color: Color::srgba(0.0, 0.0, 0.0, 0.9),
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    Pickable::IGNORE,
                    ChildOf(layer),
                ));
            }
        }
        return;
    }
    let Ok((camera, transform)) = camera.single() else {
        return;
    };
    for (label, mut node, computed) in &mut labels {
        match camera.world_to_viewport(transform, label.world) {
            Ok(pos) => {
                let size = computed.size() * computed.inverse_scale_factor();
                node.display = Display::Flex;
                node.left = px(pos.x - size.x * 0.5);
                node.top = px(pos.y - size.y);
            }
            Err(_) => node.display = Display::None,
        }
    }
}

const LINE_HEIGHT: f32 = 24.0;

fn scroll_menu(
    mut wheel: MessageReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    menu: Res<MenuState>,
    mut lists: Query<(Entity, &mut ScrollPosition, &ComputedNode), With<MenuList>>,
    parents: Query<&ChildOf>,
) {
    let events: Vec<_> = wheel.read().collect();
    if !menu.open || events.is_empty() {
        return;
    }
    let Ok((list, mut scroll, computed)) = lists.single_mut() else {
        return;
    };
    let over_list = hover_map
        .values()
        .flat_map(|m| m.keys())
        .any(|&e| e == list || parents.iter_ancestors(e).any(|a| a == list));
    if !over_list {
        return;
    }
    let max = ((computed.content_size().y - computed.size().y) * computed.inverse_scale_factor())
        .max(0.0);
    for event in events {
        let dy = match event.unit {
            MouseScrollUnit::Line => event.y * LINE_HEIGHT,
            MouseScrollUnit::Pixel => event.y,
        };
        scroll.y = (scroll.y - dy).clamp(0.0, max);
    }
}

fn keyboard_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<MenuState>,
    mut view: ResMut<CurrentView>,
    mut catalog: ResMut<CatalogRes>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        menu.open = !menu.open;
    }
    if keys.just_pressed(KeyCode::Escape) && menu.open {
        menu.open = false;
    }
    if keys.just_pressed(KeyCode::KeyR) && !menu.open {
        catalog.dirty = true;
        if let Some(target) = view.target.clone() {
            view.reload(target);
        }
    }
}
