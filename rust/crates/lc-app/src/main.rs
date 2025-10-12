use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use lc_engine::{
    ActionSpec, ActionState, ControlButton, ControlEvent, Definition, Engine, EngineError,
    EnvironmentSettings, Landscape, MovementProfile, ObjectId, SimulationSnapshot, SpawnConfig,
    Vector2,
};
use lc_frontend::{GraphicsOverlay, GraphicsSystem, InputDispatcher};
use pixels::{Pixels, SurfaceTexture};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

const WINDOW_WIDTH: u32 = 960;
const WINDOW_HEIGHT: u32 = 540;
const PLAYER_OWNER: i32 = 1;
const FRAME_INTERVAL: Duration = Duration::from_micros(16_666); // ~60 FPS

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
        WindowEvent::KeyboardInput {
            input:
                KeyboardInput {
                    state,
                    virtual_keycode: Some(keycode),
                    ..
                },
            ..
        } => {
            if keycode == VirtualKeyCode::Escape && state == ElementState::Pressed {
                control_flow.set_exit();
                return Ok(());
            }
            app.handle_key(keycode, state)
                .context("failed to process input")?;
        }
        WindowEvent::Focused(focused) => {
            if focused {
                window.request_redraw();
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
}

impl GameApp {
    fn new(width: u32, height: u32) -> Result<Self> {
        let mut engine = Engine::new();
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
        engine.register_definition(definition)?;

        engine.set_environment(EnvironmentSettings::default());
        engine.set_landscape(Landscape::flat(2048, 360));

        let spawn = SpawnConfig::new("Walker")
            .with_owner(PLAYER_OWNER)
            .with_position(Vector2::new(240, 180))
            .with_energy(100)
            .with_action(ActionState::new("Walk"))
            .with_crew_member(true);
        let object_id = engine.spawn_object(spawn)?;
        engine
            .select_crew(PLAYER_OWNER, vec![object_id])
            .context("failed to select spawned crew")?;
        engine
            .set_crew_cursor(PLAYER_OWNER, Some(object_id))
            .context("failed to set crew cursor")?;

        let snapshot = engine.snapshot();

        let scenario_label = "Rust Sandbox".to_string();
        let fallback_ground = 360;
        let mut graphics = GraphicsSystem::new(width, height, fallback_ground, &scenario_label);
        graphics
            .surface_mut()
            .fill(lc_graphics::Color::transparent());

        let mut app = Self {
            engine,
            graphics,
            input: InputDispatcher::new(),
            snapshot,
            focus_id: Some(object_id),
            focus_snapshot: None,
            frame_text: String::new(),
            status_text: String::new(),
            energy_fraction: 1.0,
            scenario_label,
            fallback_ground,
        };
        app.refresh_focus();
        Ok(app)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let mut graphics =
            GraphicsSystem::new(width, height, self.fallback_ground, &self.scenario_label);
        graphics
            .surface_mut()
            .fill(lc_graphics::Color::transparent());
        self.graphics = graphics;
        Ok(())
    }

    fn handle_key(&mut self, key: VirtualKeyCode, state: ElementState) -> Result<(), EngineError> {
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

    fn update(&mut self) -> Result<(), EngineError> {
        self.snapshot = self.engine.tick()?;
        self.refresh_focus();
        Ok(())
    }

    fn refresh_focus(&mut self) {
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
        let Some(focus) = self.focus_snapshot.as_ref() else {
            return Ok(());
        };
        let overlay = GraphicsOverlay {
            frame_text: &self.frame_text,
            status_text: &self.status_text,
            energy_fraction: self.energy_fraction,
        };
        self.graphics
            .update_overlay(&overlay)
            .context("failed to update overlay")?;
        self.graphics.render_frame(&self.snapshot, focus);

        let surface = self.graphics.surface();
        let pixels = surface.pixels();
        if pixels.len() == frame.len() {
            frame.copy_from_slice(pixels);
        } else {
            copy_surface(pixels, surface.width(), surface.height(), frame);
        }

        Ok(())
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

fn walker_script() -> &'static str {
    r#"
global func Initialize(state, random) { return nil; }
global func Step(state, frame, random) { return nil; }
"#
}
