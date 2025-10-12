use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use lc_engine::{
    ActionSpec, ActionState, ControlButton, ControlEvent, Definition, Engine, EngineError,
    EnvironmentSettings, Landscape, MovementProfile, ObjectId, ObjectSnapshot, Scenario,
    SimulationSnapshot, SpawnConfig, Vector2,
};
use lc_frontend::{
    GraphicsOverlay, GraphicsSystem, GuiPoint, InputDispatcher, KeyCode, ScenarioEntry,
    ScenarioKind, ScenarioSummary, StartupMenu, StartupMenuAction,
};
use lc_graphics::Color;
use lc_platform::AppPaths;
use lc_resources::scenario as resource_scenario;
use pixels::{Pixels, SurfaceTexture};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Event, KeyboardInput, MouseButton, TouchPhase, VirtualKeyCode, WindowEvent,
};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

const WINDOW_WIDTH: u32 = 960;
const WINDOW_HEIGHT: u32 = 540;
const PLAYER_OWNER: i32 = 1;
const FRAME_INTERVAL: Duration = Duration::from_micros(16_666); // ~60 FPS
const DEFAULT_SCENARIO_LABEL: &str = "Rust Sandbox";
const DEFAULT_GROUND_HEIGHT: i32 = 360;

fn main() -> Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("LegacyClonk (Rust preview)")
        .with_inner_size(LogicalSize::new(
            f64::from(WINDOW_WIDTH),
            f64::from(WINDOW_HEIGHT),
        ))
        .build(&event_loop)
        .context("failed to create application window")?;

    let size = enforce_min_size(window.inner_size());
    let surface = SurfaceTexture::new(size.width, size.height, &window);
    let mut pixels = Pixels::new(size.width, size.height, surface)
        .context("failed to create pixel framebuffer")?;

    let mut app =
        GameApp::new(size.width, size.height).context("failed to initialise app state")?;

    let mut last_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => {
                if let Err(err) =
                    handle_window_event(&window, &mut app, &mut pixels, event, control_flow)
                {
                    eprintln!("error: {err:?}");
                    control_flow.set_exit();
                }
            }
            Event::MainEventsCleared => {
                if last_frame.elapsed() >= FRAME_INTERVAL {
                    if let Err(err) = app.update() {
                        eprintln!("tick failed: {err:?}");
                        control_flow.set_exit();
                        return;
                    }
                    window.request_redraw();
                    last_frame = Instant::now();
                }
            }
            Event::RedrawRequested(id) if id == window.id() => {
                if let Err(err) = app.render(pixels.frame_mut()) {
                    eprintln!("render failed: {err:?}");
                    control_flow.set_exit();
                    return;
                }
                if let Err(err) = pixels.render() {
                    eprintln!("present failed: {err:?}");
                    control_flow.set_exit();
                }
            }
            Event::LoopDestroyed => {}
            _ => {}
        }
    });
}

fn handle_window_event(
    window: &Window,
    app: &mut GameApp,
    pixels: &mut Pixels,
    event: WindowEvent,
    control_flow: &mut ControlFlow,
) -> Result<()> {
    match event {
        WindowEvent::CloseRequested => {
            control_flow.set_exit();
        }
        WindowEvent::Resized(size) => {
            let clamped = enforce_min_size(size);
            pixels
                .resize_surface(clamped.width, clamped.height)
                .context("failed to resize pixel surface")?;
            pixels
                .resize_buffer(clamped.width, clamped.height)
                .context("failed to resize pixel buffer")?;
            app.resize(clamped.width, clamped.height)?;
        }
        WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
            let clamped = enforce_min_size(*new_inner_size);
            pixels
                .resize_surface(clamped.width, clamped.height)
                .context("failed to resize pixel surface")?;
            pixels
                .resize_buffer(clamped.width, clamped.height)
                .context("failed to resize pixel buffer")?;
            app.resize(clamped.width, clamped.height)?;
        }
        WindowEvent::CursorMoved { position, .. } => {
            app.handle_cursor_moved(position)
                .context("failed to process cursor movement")?;
        }
        WindowEvent::CursorLeft { .. } => {
            app.pointer_left();
        }
        WindowEvent::MouseInput { state, button, .. } => {
            if button == MouseButton::Left {
                app.handle_mouse_button(state)
                    .context("failed to process mouse button")?;
            }
        }
        WindowEvent::KeyboardInput {
            input:
                KeyboardInput {
                    state,
                    virtual_keycode: Some(keycode),
                    ..
                },
            ..
        } => {
            app.handle_key(keycode, state)
                .context("failed to process key input")?;
        }
        WindowEvent::Touch(touch) => {
            let position = GuiPoint::new(touch.location.x as f32, touch.location.y as f32);
            app.handle_touch(touch.phase, position)
                .context("failed to process touch input")?;
        }
        WindowEvent::Focused(focused) => {
            if focused {
                window.request_redraw();
            } else {
                app.pointer_left();
            }
        }
        _ => {}
    }
    Ok(())
}

fn enforce_min_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

struct GameApp {
    engine: Engine,
    graphics: GraphicsSystem,
    input: InputDispatcher,
    snapshot: SimulationSnapshot,
    focus_id: Option<ObjectId>,
    focus_snapshot: Option<lc_engine::ObjectSnapshot>,
    frame_text: String,
    status_text: String,
    energy_fraction: f32,
    scenario_label: String,
    fallback_ground: i32,
    mode: AppMode,
    active_scenario: Option<FrontendScenario>,
}

enum AppMode {
    Menu(MenuState),
    Running,
}

struct MenuState {
    menu: StartupMenu,
    pointer_position: Option<GuiPoint>,
    entries: Vec<FrontendScenario>,
}

#[derive(Clone, Debug)]
struct FrontendScenario {
    identifier: String,
    title: String,
    description: Option<String>,
    kind: ScenarioKind,
    is_editable: bool,
    is_playable: bool,
    path: Option<PathBuf>,
}

impl FrontendScenario {
    fn to_ui_entry(&self) -> ScenarioEntry {
        ScenarioEntry {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            kind: self.kind,
            is_editable: self.is_editable,
            is_playable: self.is_playable,
        }
    }

    fn from_resource(entry: &resource_scenario::ScenarioEntry) -> Self {
        Self {
            identifier: entry.identifier.clone(),
            title: entry.title.clone(),
            description: entry.description.clone(),
            kind: ScenarioKind::Scenario,
            is_editable: entry.is_editable,
            is_playable: entry.is_playable,
            path: Some(entry.path.clone()),
        }
    }

    fn fallback() -> Self {
        Self {
            identifier: "rust_sandbox".to_string(),
            title: DEFAULT_SCENARIO_LABEL.to_string(),
            description: Some("Spawn a Rust-driven walker in a flat test landscape.".to_string()),
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            path: None,
        }
    }
}

impl GameApp {
    fn new(width: u32, height: u32) -> Result<Self> {
        let engine = Engine::new();
        let snapshot = engine.snapshot();
        let scenario_label = DEFAULT_SCENARIO_LABEL.to_string();
        let mut graphics =
            GraphicsSystem::new(width, height, DEFAULT_GROUND_HEIGHT, &scenario_label);
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));

        let scenarios = load_frontend_scenarios();
        let menu_entries = scenarios
            .iter()
            .map(FrontendScenario::to_ui_entry)
            .collect::<Vec<_>>();
        let mut menu = StartupMenu::new(menu_entries)
            .map_err(|err| anyhow!("failed to create startup menu: {err}"))?;
        menu.resize(width as f32, height as f32);

        Ok(Self {
            engine,
            graphics,
            input: InputDispatcher::new(),
            snapshot,
            focus_id: None,
            focus_snapshot: None,
            frame_text: String::new(),
            status_text: String::new(),
            energy_fraction: 0.0,
            scenario_label,
            fallback_ground: DEFAULT_GROUND_HEIGHT,
            mode: AppMode::Menu(MenuState {
                menu,
                pointer_position: None,
                entries: scenarios,
            }),
            active_scenario: None,
        })
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let mut graphics =
            GraphicsSystem::new(width, height, self.fallback_ground, &self.scenario_label);
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics = graphics;

        if let AppMode::Menu(state) = &mut self.mode {
            state.menu.resize(width as f32, height as f32);
            state.pointer_position = None;
        }
        Ok(())
    }

    fn handle_key(&mut self, key: VirtualKeyCode, state: ElementState) -> Result<(), EngineError> {
        if matches!(self.mode, AppMode::Menu(_)) {
            if let Some(gui_key) = map_key_code(key) {
                match state {
                    ElementState::Pressed => {
                        self.handle_menu_input(|menu| menu.menu.handle_key_down(gui_key))?
                    }
                    ElementState::Released => {
                        self.handle_menu_input(|menu| menu.menu.handle_key_up(gui_key))?
                    }
                }
            }
            return Ok(());
        }

        if matches!(self.mode, AppMode::Running) {
            self.handle_engine_key(key, state)?;
        }
        Ok(())
    }

    fn handle_engine_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let event = match (key, state) {
            (VirtualKeyCode::Left, ElementState::Pressed) => {
                Some(ControlEvent::Press(ControlButton::Left))
            }
            (VirtualKeyCode::Left, ElementState::Released) => {
                Some(ControlEvent::Release(ControlButton::Left))
            }
            (VirtualKeyCode::Right, ElementState::Pressed) => {
                Some(ControlEvent::Press(ControlButton::Right))
            }
            (VirtualKeyCode::Right, ElementState::Released) => {
                Some(ControlEvent::Release(ControlButton::Right))
            }
            (VirtualKeyCode::Up, ElementState::Pressed) => {
                Some(ControlEvent::Press(ControlButton::Up))
            }
            (VirtualKeyCode::Up, ElementState::Released) => {
                Some(ControlEvent::Release(ControlButton::Up))
            }
            (VirtualKeyCode::Down, ElementState::Pressed) => {
                Some(ControlEvent::Press(ControlButton::Down))
            }
            (VirtualKeyCode::Down, ElementState::Released) => {
                Some(ControlEvent::Release(ControlButton::Down))
            }
            (VirtualKeyCode::Space, ElementState::Pressed) => Some(ControlEvent::ClearPressed),
            _ => None,
        };

        if let Some(event) = event {
            let _ = self
                .input
                .handle_event(&mut self.engine, PLAYER_OWNER, event)?;
        }
        Ok(())
    }

    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> Result<(), EngineError> {
        let point = gui_point_from_position(position);
        self.handle_menu_input(|state| {
            state.pointer_position = Some(point);
            state.menu.handle_pointer_move(point)
        })
    }

    fn handle_mouse_button(&mut self, button_state: ElementState) -> Result<(), EngineError> {
        let position = match &self.mode {
            AppMode::Menu(state) => state.pointer_position,
            _ => None,
        };
        if let Some(point) = position {
            match button_state {
                ElementState::Pressed => {
                    self.handle_menu_input(|state| state.menu.handle_pointer_down(point))?
                }
                ElementState::Released => {
                    self.handle_menu_input(|state| state.menu.handle_pointer_up(point))?
                }
            }
        }
        Ok(())
    }

    fn handle_touch(&mut self, phase: TouchPhase, position: GuiPoint) -> Result<(), EngineError> {
        match phase {
            TouchPhase::Started => self.handle_menu_input(|state| {
                state.pointer_position = Some(position);
                state.menu.handle_pointer_down(position)
            }),
            TouchPhase::Moved => self.handle_menu_input(|state| {
                state.pointer_position = Some(position);
                state.menu.handle_pointer_move(position)
            }),
            TouchPhase::Ended => {
                let result = self.handle_menu_input(|state| {
                    state.pointer_position = Some(position);
                    state.menu.handle_pointer_up(position)
                });
                self.pointer_left();
                result
            }
            TouchPhase::Cancelled => {
                self.pointer_left();
                Ok(())
            }
        }
    }

    fn pointer_left(&mut self) {
        if let AppMode::Menu(state) = &mut self.mode {
            state.pointer_position = None;
        }
    }

    fn handle_menu_input<F>(&mut self, handler: F) -> Result<(), EngineError>
    where
        F: FnOnce(&mut MenuState) -> Vec<StartupMenuAction>,
    {
        let start_entry = {
            if let AppMode::Menu(state) = &mut self.mode {
                let actions = handler(state);
                extract_start_action(state, actions)
            } else {
                None
            }
        };

        if let Some(entry) = start_entry {
            self.start_scenario(entry)?;
        }
        Ok(())
    }

    fn update(&mut self) -> Result<(), EngineError> {
        if matches!(self.mode, AppMode::Running) {
            self.snapshot = self.engine.tick()?;
            self.refresh_focus();
        }
        Ok(())
    }

    fn refresh_focus(&mut self) {
        if !matches!(self.mode, AppMode::Running) {
            self.focus_snapshot = None;
            return;
        }

        if self
            .focus_id
            .and_then(|id| self.snapshot.object(id))
            .is_none()
        {
            self.focus_id = self.snapshot.objects.first().map(|object| object.id);
        }

        self.focus_snapshot = self
            .focus_id
            .and_then(|id| self.snapshot.object(id).cloned());

        if let Some(object) = &self.focus_snapshot {
            self.frame_text = format!(
                "FRAME {:05} POS {:04}/{:04} VEL {:03}/{:03}",
                self.snapshot.frame,
                object.position.x,
                object.position.y,
                object.velocity.x,
                object.velocity.y
            );
            self.status_text = format!(
                "ENERGY {:03} DAMAGE {:03} OWNER {}",
                object.energy.max(0),
                object.damage.max(0),
                object.owner
            );
            self.energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;
        } else {
            self.frame_text = format!("FRAME {:05}", self.snapshot.frame);
            self.status_text.clear();
            self.energy_fraction = 0.0;
        }
    }

    fn render(&mut self, frame: &mut [u8]) -> Result<()> {
        if let AppMode::Menu(state) = &mut self.mode {
            render_menu_frame(&mut self.graphics, &mut state.menu, frame);
            return Ok(());
        }
        self.render_running(frame)
    }

    fn render_running(&mut self, frame: &mut [u8]) -> Result<()> {
        if let Some(focus) = self.focus_snapshot.as_ref() {
            let overlay = GraphicsOverlay {
                frame_text: &self.frame_text,
                status_text: &self.status_text,
                energy_fraction: self.energy_fraction,
            };
            self.graphics
                .update_overlay(&overlay)
                .context("failed to update overlay")?;
            self.graphics.render_frame(&self.snapshot, focus);
        } else {
            self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        }

        let surface = self.graphics.surface();
        let pixels = surface.pixels();
        if pixels.len() == frame.len() {
            frame.copy_from_slice(pixels);
        } else {
            copy_surface(pixels, surface.width(), surface.height(), frame);
        }
        Ok(())
    }

    fn start_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        if self.try_start_real_scenario(&scenario)? {
            return Ok(());
        }
        self.start_sandbox_scenario(scenario)
    }

    fn try_start_real_scenario(
        &mut self,
        scenario: &FrontendScenario,
    ) -> Result<bool, EngineError> {
        let Some(path) = scenario.path.as_ref() else {
            return Ok(false);
        };

        let scenario_data = match Scenario::load_from_path(path) {
            Ok(data) => data,
            Err(err) => {
                eprintln!(
                    "Failed to load scenario '{}' from {}: {err}",
                    scenario.title,
                    path.display()
                );
                return Ok(false);
            }
        };

        println!(
            "Starting scenario '{}' from {}",
            scenario.title,
            path.display()
        );

        self.engine = Engine::new();
        self.input = InputDispatcher::new();

        if let Err(err) = scenario_data.apply(&mut self.engine) {
            eprintln!(
                "Failed to apply scenario '{}' from {}: {err}",
                scenario.title,
                path.display()
            );
            return Ok(false);
        }

        self.snapshot = self.engine.snapshot();

        let label = scenario_data
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| scenario.title.clone());
        let ground = scenario_data
            .ground_height_hint()
            .unwrap_or(DEFAULT_GROUND_HEIGHT)
            .max(0);

        self.configure_running_state(label, ground);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.refresh_focus();
        self.active_scenario = Some(scenario.clone());
        Ok(true)
    }

    fn start_sandbox_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        println!("Starting scenario '{}' (sandbox fallback)", scenario.title);

        self.engine = Engine::new();
        self.input = InputDispatcher::new();

        let mut definition = Definition::from_script("Walker", "Rust Walker", walker_script())?;
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("Walk"),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_crew_member(true);
        let profile = MovementProfile::default()
            .with_walk_speed(8)
            .with_walk_acceleration(2);
        definition.set_movement_profile(profile);
        self.engine.register_definition(definition)?;

        self.engine.set_environment(EnvironmentSettings::default());
        self.engine
            .set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));

        let spawn = SpawnConfig::new("Walker")
            .with_owner(PLAYER_OWNER)
            .with_position(Vector2::new(240, 180))
            .with_energy(100)
            .with_action(ActionState::new("Walk"))
            .with_crew_member(true);
        self.engine.spawn_object(spawn)?;

        self.snapshot = self.engine.snapshot();
        self.configure_running_state(scenario.title.clone(), DEFAULT_GROUND_HEIGHT);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.refresh_focus();
        self.active_scenario = Some(scenario);
        Ok(())
    }

    fn configure_running_state(&mut self, label: String, fallback_ground: i32) {
        self.scenario_label = label;
        self.fallback_ground = fallback_ground;
        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        self.graphics =
            GraphicsSystem::new(width, height, self.fallback_ground, &self.scenario_label);
        self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.mode = AppMode::Running;
    }

    fn apply_focus_selection(&mut self) {
        if let Some((object_id, owner, crew_member)) = select_focus_candidate(&self.snapshot) {
            self.focus_id = Some(object_id);
            if crew_member && owner >= 0 {
                if let Err(err) = self.engine.select_crew(owner, [object_id]) {
                    eprintln!(
                        "Failed to select crew member {} for owner {}: {err}",
                        object_id, owner
                    );
                } else if let Err(err) = self.engine.set_crew_cursor(owner, Some(object_id)) {
                    eprintln!(
                        "Failed to set crew cursor to {} for owner {}: {err}",
                        object_id, owner
                    );
                }
            }
        } else {
            self.focus_id = None;
        }
        self.focus_snapshot = None;
    }
}

fn extract_start_action(
    state: &MenuState,
    actions: Vec<StartupMenuAction>,
) -> Option<FrontendScenario> {
    for action in actions {
        match action {
            StartupMenuAction::StartScenario(summary) => {
                if let Some(entry) = state
                    .entries
                    .iter()
                    .find(|entry| entry.identifier == summary.identifier && entry.is_playable)
                {
                    return Some(entry.clone());
                }
            }
            StartupMenuAction::OpenEntry(ScenarioSummary { identifier, .. }) => {
                eprintln!(
                    "Opening folders is not yet implemented for Rust menu items: {identifier}"
                );
            }
            StartupMenuAction::EditEntry(ScenarioSummary { identifier, .. }) => {
                eprintln!(
                    "Editing entries is not yet implemented for Rust menu items: {identifier}"
                );
            }
            StartupMenuAction::SelectionChanged(_) => {}
        }
    }
    None
}

fn render_menu_frame(graphics: &mut GraphicsSystem, menu: &mut StartupMenu, frame: &mut [u8]) {
    {
        let surface = graphics.surface_mut();
        surface.fill(Color::opaque(16, 28, 52));
        menu.render(surface);
    }
    let surface = graphics.surface();
    let pixels = surface.pixels();
    if pixels.len() == frame.len() {
        frame.copy_from_slice(pixels);
    } else {
        copy_surface(pixels, surface.width(), surface.height(), frame);
    }
}

fn copy_surface(src: &[u8], width: u32, height: u32, dest: &mut [u8]) {
    const BYTES_PER_PIXEL: usize = 4;
    if width == 0 || height == 0 {
        return;
    }
    let stride = width as usize * BYTES_PER_PIXEL;
    for row in 0..height as usize {
        let src_offset = row * stride;
        let dest_offset = row * stride;
        let end = src_offset + stride;
        if end <= src.len() && dest_offset + stride <= dest.len() {
            dest[dest_offset..dest_offset + stride].copy_from_slice(&src[src_offset..end]);
        }
    }
}

fn select_focus_candidate(snapshot: &SimulationSnapshot) -> Option<(ObjectId, i32, bool)> {
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner == PLAYER_OWNER {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner >= 0 {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.owner >= 0 {
            return Some((object.id, object.owner, object.crew_member));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) {
            return Some((object.id, object.owner, object.crew_member));
        }
    }
    snapshot
        .objects
        .first()
        .map(|object| (object.id, object.owner, object.crew_member))
}

fn is_focusable(object: &ObjectSnapshot) -> bool {
    object.alive && object.status.is_active()
}

fn map_key_code(code: VirtualKeyCode) -> Option<KeyCode> {
    match code {
        VirtualKeyCode::Return => Some(KeyCode::Enter),
        VirtualKeyCode::Escape => Some(KeyCode::Escape),
        VirtualKeyCode::Space => Some(KeyCode::Space),
        VirtualKeyCode::Tab => Some(KeyCode::Tab),
        VirtualKeyCode::Up => Some(KeyCode::Up),
        VirtualKeyCode::Down => Some(KeyCode::Down),
        VirtualKeyCode::Left => Some(KeyCode::Left),
        VirtualKeyCode::Right => Some(KeyCode::Right),
        _ => None,
    }
}

fn gui_point_from_position(position: PhysicalPosition<f64>) -> GuiPoint {
    GuiPoint::new(position.x as f32, position.y as f32)
}

fn load_frontend_scenarios() -> Vec<FrontendScenario> {
    let mut scenarios = Vec::new();

    if let Ok(paths) = AppPaths::discover() {
        let roots = scenario_roots(&paths);
        let existing_roots: Vec<_> = roots.into_iter().filter(|path| path.exists()).collect();
        if !existing_roots.is_empty() {
            match resource_scenario::discover_many(existing_roots.iter()) {
                Ok(entries) => {
                    let mut seen = HashSet::new();
                    collect_scenarios(&entries, &mut seen, &mut scenarios);
                }
                Err(err) => {
                    eprintln!("failed to discover scenarios from install roots: {err}");
                }
            }
        }
    } else {
        eprintln!("App paths discovery failed; falling back to built-in sandbox scenario");
    }

    if scenarios.is_empty() {
        scenarios.push(FrontendScenario::fallback());
    } else {
        scenarios.sort_by(|a, b| a.title.cmp(&b.title));
    }
    scenarios
}

fn scenario_roots(paths: &AppPaths) -> Vec<PathBuf> {
    let mut roots = vec![
        paths.scenario_dir(),
        paths.install_root().join("Scenarios"),
        paths.install_root().join("scenarios"),
        paths.planet_dir().to_path_buf(),
        paths.system_group_path().to_path_buf(),
    ];
    roots.sort();
    roots.dedup();
    roots
}

fn collect_scenarios(
    entries: &[resource_scenario::ScenarioEntry],
    seen: &mut HashSet<String>,
    out: &mut Vec<FrontendScenario>,
) {
    for entry in entries {
        if matches!(entry.kind, resource_scenario::ScenarioEntryKind::Scenario)
            && seen.insert(entry.identifier.clone())
        {
            out.push(FrontendScenario::from_resource(entry));
        }
        collect_scenarios(&entry.children, seen, out);
    }
}

fn walker_script() -> &'static str {
    r#"
global func Initialize(state, random) { return nil; }
global func Step(state, frame, random) { return nil; }
"#
}
