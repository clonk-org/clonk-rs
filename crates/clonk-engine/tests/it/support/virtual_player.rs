use super::dev_feedback::{DevFeedbackCapture, ReplayInputV1, ScenarioReplayV1};
use clonk_engine::{
    Engine, EngineError, ObjectId, ObjectMenuState, COM_CURSOR_TOGGLE, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_THROW, COM_UP,
};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

const RECENT_STATE_LIMIT: usize = 6;
const RECENT_ID_LIMIT: usize = 8;
const DOUBLE_CLICK_TICKS: u32 = 10;

/// Headless input driver for parity tests.
///
/// Every control goes through `C4Player::InCom`'s Rust counterpart. In
/// particular, this never edits an object snapshot, command direction, menu
/// selection, or script local directly. The press/release range and modifier
/// bytes come from `src/C4Constants.h:156,173-235`; buffering and double-click
/// synthesis come from `src/C4Player.cpp:1490-1554`.
pub struct VirtualPlayer<'engine> {
    engine: &'engine mut Engine,
    owner: i32,
    feedback: Option<DevFeedbackCapture>,
}

#[derive(Debug)]
pub enum VirtualPlayerError {
    Engine(EngineError),
    InvalidControl {
        control: u8,
    },
    MilestoneNotReached {
        milestone: String,
        frame: u64,
        diagnostics: String,
    },
    Timeout {
        milestone: String,
        max_ticks: u32,
        frame: u64,
        diagnostics: String,
        artifacts: Option<PathBuf>,
        artifact_warning: Option<String>,
    },
    MenuClosed {
        owner: i32,
        frame: u64,
    },
    MenuIndexOutOfBounds {
        index: usize,
        item_count: usize,
    },
    MenuItemMissing {
        caption: String,
    },
    MenuItemUnselectable {
        index: usize,
        caption: String,
    },
    MenuNavigationStalled {
        target: usize,
        selection: i32,
    },
}

impl fmt::Display for VirtualPlayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine(error) => write!(formatter, "engine input failed: {error}"),
            Self::InvalidControl { control } => {
                write!(
                    formatter,
                    "control {control} is not a press/release control"
                )
            }
            Self::MilestoneNotReached {
                milestone,
                frame,
                diagnostics,
            } => {
                write!(
                    formatter,
                    "milestone `{milestone}` was not reached at frame {frame}; {diagnostics}"
                )
            }
            Self::Timeout {
                milestone,
                max_ticks,
                frame,
                diagnostics,
                artifacts,
                artifact_warning,
            } => {
                write!(
                    formatter,
                    "timed out after {max_ticks} ticks waiting for `{milestone}` at frame {frame}; \
                     {diagnostics}"
                )?;
                if let Some(path) = artifacts {
                    write!(formatter, "; artifacts: {}", path.display())?;
                }
                if let Some(warning) = artifact_warning {
                    write!(formatter, "; artifact capture failed: {warning}")?;
                }
                Ok(())
            }
            Self::MenuClosed { owner, frame } => {
                write!(
                    formatter,
                    "player {owner}'s cursor menu is closed at frame {frame}"
                )
            }
            Self::MenuIndexOutOfBounds { index, item_count } => write!(
                formatter,
                "menu index {index} is outside its {item_count} items"
            ),
            Self::MenuItemMissing { caption } => {
                write!(formatter, "menu has no item captioned `{caption}`")
            }
            Self::MenuItemUnselectable { index, caption } => write!(
                formatter,
                "menu item {index} (`{caption}`) is not selectable"
            ),
            Self::MenuNavigationStalled { target, selection } => write!(
                formatter,
                "menu navigation did not reach item {target}; selection remained {selection}"
            ),
        }
    }
}

impl Error for VirtualPlayerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Engine(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EngineError> for VirtualPlayerError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl VirtualPlayerError {
    pub fn artifact_dir(&self) -> Option<&Path> {
        match self {
            Self::Timeout { artifacts, .. } => artifacts.as_deref(),
            _ => None,
        }
    }
}

impl<'engine> VirtualPlayer<'engine> {
    pub fn new(engine: &'engine mut Engine, owner: i32) -> Self {
        Self {
            engine,
            owner,
            feedback: None,
        }
    }

    /// Enable deterministic input recording and timeout artifact capture.
    /// The ordinary constructor remains allocation-free for tests that do
    /// not need replay forensics.
    pub fn with_dev_feedback(
        engine: &'engine mut Engine,
        owner: i32,
        replay: ScenarioReplayV1,
        artifact_label: impl Into<String>,
    ) -> Self {
        let feedback = DevFeedbackCapture::new(replay, artifact_label, engine);
        Self {
            engine,
            owner,
            feedback: Some(feedback),
        }
    }

    pub fn recorded_inputs(&self) -> Option<&[ReplayInputV1]> {
        self.feedback
            .as_ref()
            .map(DevFeedbackCapture::recorded_inputs)
    }

    /// Read-only access for milestone predicates and assertions. The harness
    /// intentionally does not expose mutable engine access.
    pub fn engine(&self) -> &Engine {
        self.engine
    }

    /// Start a route checkpoint with a clean physical-key ledger under the
    /// selected C4Player control style. Scenario state is untouched; future
    /// controls still enter through `C4Player::InCom`.
    pub fn reset_input_ledger_with_control_style(
        &mut self,
        control_style: bool,
    ) -> Result<(), VirtualPlayerError> {
        let control = &mut self.engine.player_mut(self.owner)?.control;
        control.last_com = 0;
        control.last_com_delay = 0;
        control.last_com_down_double = 0;
        control.pressed_coms = 0;
        control.control_style = control_style;
        Ok(())
    }

    /// Send a physical key-down control through `C4Player::InCom`.
    pub fn press(&mut self, control: u8) -> Result<(), VirtualPlayerError> {
        self.validate_control(control)?;
        self.engine.player_in_com(self.owner, control, 0)?;
        self.record_input(control, 0);
        Ok(())
    }

    /// Send the matching key-up byte (`base + 16`). C++ ignores a release
    /// unless the key-down set its bit (`src/C4Player.cpp:1540-1548`).
    pub fn release(&mut self, control: u8) -> Result<(), VirtualPlayerError> {
        self.validate_control(control)?;
        self.engine
            .player_in_com(self.owner, control + COM_RELEASE_OFFSET, 0)?;
        self.record_input(control + COM_RELEASE_OFFSET, 0);
        Ok(())
    }

    /// A physical press followed immediately by its release. C++ retains the
    /// press in `LastCom`, so its `COM_Single` callback is still delayed.
    pub fn tap(&mut self, control: u8) -> Result<(), VirtualPlayerError> {
        self.press(control)?;
        self.release(control)
    }

    /// Two taps in the same input window. The repeated second press becomes
    /// `COM_Double` in `C4Player::InCom` (`src/C4Player.cpp:1532-1536`).
    pub fn double_tap(&mut self, control: u8) -> Result<(), VirtualPlayerError> {
        self.tap(control)?;
        self.tap(control)
    }

    pub fn ticks(&mut self, count: u32) -> Result<(), VirtualPlayerError> {
        for _ in 0..count {
            self.tick_once()?;
        }
        Ok(())
    }

    /// Let C++'s buffered `LastCom` become `COM_Single` before repeating the
    /// same physical control. The buffer flushes only after `C4DoubleClick`
    /// (10) frames (`src/C4Constants.h:156`; `src/C4Player.cpp:1217-1228`).
    pub fn wait_out_double_click(&mut self) -> Result<(), VirtualPlayerError> {
        self.ticks(DOUBLE_CLICK_TICKS + 1)
    }

    /// Hold a physical control while ticking toward a milestone, then emit
    /// its matching release even when the milestone times out. This is the
    /// headless equivalent of walking/flying until a visible landmark; both
    /// edges still pass through C4Player::InCom (src/C4Player.cpp:1490-1554).
    pub fn hold_until(
        &mut self,
        control: u8,
        milestone: impl Into<String>,
        max_ticks: u32,
        reached: impl FnMut(&Engine) -> bool,
    ) -> Result<u32, VirtualPlayerError> {
        self.press(control)?;
        let outcome = self.wait_until_without_artifacts(milestone.into(), max_ticks, reached);
        let release = self.release(control);
        match outcome {
            Ok(elapsed) => release.map(|()| elapsed),
            Err(mut error) => {
                // Preserve the milestone failure as the primary result, just
                // as before, but defer capture until the matching key-up has
                // traversed `player_in_com` and entered the replay tape.
                let _ = release;
                self.capture_timeout_artifact(&mut error);
                Err(error)
            }
        }
    }

    /// Tick until a read-only milestone is true, checking before the first
    /// tick and after each subsequent tick.
    pub fn wait_until(
        &mut self,
        milestone: impl Into<String>,
        max_ticks: u32,
        mut reached: impl FnMut(&Engine) -> bool,
    ) -> Result<u32, VirtualPlayerError> {
        let milestone = milestone.into();
        match self.wait_until_without_artifacts(milestone, max_ticks, &mut reached) {
            Ok(elapsed) => Ok(elapsed),
            Err(mut error) => {
                self.capture_timeout_artifact(&mut error);
                Err(error)
            }
        }
    }

    fn wait_until_without_artifacts(
        &mut self,
        milestone: String,
        max_ticks: u32,
        mut reached: impl FnMut(&Engine) -> bool,
    ) -> Result<u32, VirtualPlayerError> {
        if reached(self.engine) {
            return Ok(0);
        }
        let mut recent = VecDeque::with_capacity(RECENT_STATE_LIMIT);
        // Timeout diagnostics retain only the final few observations. Avoid
        // eagerly cloning and formatting object/menu state on every successful
        // route tick; start collecting exactly where the bounded queue would
        // otherwise have discarded all earlier entries.
        let diagnostics_start =
            max_ticks.saturating_sub(u32::try_from(RECENT_STATE_LIMIT - 1).unwrap_or(u32::MAX));
        if diagnostics_start == 0 {
            self.remember_observable_state(&mut recent);
        }
        for elapsed in 1..=max_ticks {
            self.tick_once()?;
            if elapsed >= diagnostics_start {
                self.remember_observable_state(&mut recent);
            }
            if reached(self.engine) {
                return Ok(elapsed);
            }
        }
        Err(VirtualPlayerError::Timeout {
            milestone,
            max_ticks,
            frame: self.engine.frame(),
            diagnostics: self.timeout_diagnostics(recent),
            artifacts: None,
            artifact_warning: None,
        })
    }

    pub fn assert_milestone(
        &self,
        milestone: impl Into<String>,
        reached: impl FnOnce(&Engine) -> bool,
    ) -> Result<(), VirtualPlayerError> {
        if reached(self.engine) {
            Ok(())
        } else {
            // A route that ends a bounded recovery loop without reaching its
            // goal fails here rather than in a `wait_until`, so carry the same
            // observable state a timeout would have reported.
            let mut recent = VecDeque::with_capacity(1);
            self.remember_observable_state(&mut recent);
            Err(VirtualPlayerError::MilestoneNotReached {
                milestone: milestone.into(),
                frame: self.engine.frame(),
                diagnostics: self.timeout_diagnostics(recent),
            })
        }
    }

    /// Menu controls deliberately use the player's ordinary directional
    /// controls. C++ converts them only while the cursor menu is active
    /// (`src/C4Player.cpp:1502-1513`, `src/C4Menu.cpp:1040-1069`).
    pub fn menu_left(&mut self) -> Result<(), VirtualPlayerError> {
        self.require_menu()?;
        self.tap(COM_LEFT)
    }

    pub fn menu_right(&mut self) -> Result<(), VirtualPlayerError> {
        self.require_menu()?;
        self.tap(COM_RIGHT)
    }

    pub fn menu_up(&mut self) -> Result<(), VirtualPlayerError> {
        self.require_menu()?;
        self.tap(COM_UP)
    }

    pub fn menu_down(&mut self) -> Result<(), VirtualPlayerError> {
        self.require_menu()?;
        self.tap(COM_DOWN)
    }

    pub fn menu_enter(&mut self) -> Result<(), VirtualPlayerError> {
        self.require_menu()?;
        self.tap(COM_THROW)
    }

    pub fn menu_close(&mut self) -> Result<(), VirtualPlayerError> {
        self.require_menu()?;
        self.tap(COM_DIG)
    }

    /// Reach an item using repeated real Right controls, including C++ menu
    /// wrap-around and unselectable-item skipping (`src/C4Menu.cpp:433-480`).
    pub fn menu_navigate_to_caption(&mut self, caption: &str) -> Result<(), VirtualPlayerError> {
        let target = self
            .require_menu()?
            .items
            .iter()
            .position(|item| item.caption == caption)
            .ok_or_else(|| VirtualPlayerError::MenuItemMissing {
                caption: caption.to_owned(),
            })?;
        self.menu_navigate_to_index(target)
    }

    pub fn menu_navigate_to_index(&mut self, target: usize) -> Result<(), VirtualPlayerError> {
        let (item_count, selectable, caption) = {
            let menu = self.require_menu()?;
            let Some(item) = menu.items.get(target) else {
                return Err(VirtualPlayerError::MenuIndexOutOfBounds {
                    index: target,
                    item_count: menu.items.len(),
                });
            };
            (menu.items.len(), item.selectable, item.caption.clone())
        };
        if !selectable {
            return Err(VirtualPlayerError::MenuItemUnselectable {
                index: target,
                caption,
            });
        }

        for _ in 0..=item_count {
            let selection = self.require_menu()?.selection;
            if selection == target as i32 {
                return Ok(());
            }
            self.menu_right()?;
        }
        Err(VirtualPlayerError::MenuNavigationStalled {
            target,
            selection: self.require_menu()?.selection,
        })
    }

    fn require_menu(&self) -> Result<&ObjectMenuState, VirtualPlayerError> {
        self.engine
            .cursor_object_menu(self.owner)
            .map(|(_, menu)| menu)
            .ok_or(VirtualPlayerError::MenuClosed {
                owner: self.owner,
                frame: self.engine.frame(),
            })
    }

    fn validate_control(&self, control: u8) -> Result<(), VirtualPlayerError> {
        if (COM_LEFT..=COM_CURSOR_TOGGLE).contains(&control) {
            Ok(())
        } else {
            Err(VirtualPlayerError::InvalidControl { control })
        }
    }

    fn record_input(&mut self, command: u8, data: i32) {
        if let Some(feedback) = &mut self.feedback {
            feedback.record_input(self.engine.frame(), self.owner, command, data);
        }
    }

    fn tick_once(&mut self) -> Result<(), VirtualPlayerError> {
        if let Some(feedback) = &mut self.feedback {
            feedback.before_tick(self.engine);
        }
        self.engine.tick_without_snapshot()?;
        Ok(())
    }

    fn capture_timeout_artifact(&self, error: &mut VirtualPlayerError) {
        let VirtualPlayerError::Timeout {
            milestone,
            max_ticks,
            diagnostics,
            artifacts,
            artifact_warning,
            ..
        } = error
        else {
            return;
        };
        let Some(feedback) = &self.feedback else {
            return;
        };
        match feedback.capture_timeout(self.engine, milestone, *max_ticks, diagnostics) {
            Ok(path) => *artifacts = path,
            Err(error) => *artifact_warning = Some(error.to_string()),
        }
    }

    fn remember_observable_state(&self, recent: &mut VecDeque<String>) {
        if recent.len() == RECENT_STATE_LIMIT {
            recent.pop_front();
        }
        recent.push_back(self.observable_state());
    }

    fn observable_state(&self) -> String {
        let frame = self.engine.frame();
        let menu = self.menu_diagnostics();
        self.engine
            .crew_cursor(self.owner)
            .and_then(|cursor| {
                self.engine.object_snapshot(cursor).map(|object| {
                    let container = object
                        .container
                        .map_or_else(|| "-".to_owned(), |id| id.to_string());
                    let target = object
                        .action
                        .target
                        .map_or_else(|| "-".to_owned(), |id| id.to_string());
                    format!(
                        "frame={frame} cursor={}:{} pos=({},{}) vel=({},{}) \
                         action={}:{}@{} target={} comdir={} container={} contents={} menu={}",
                        object.id,
                        object.definition_id,
                        object.position.x,
                        object.position.y,
                        object.velocity.x,
                        object.velocity.y,
                        object.action.name,
                        object.action.phase,
                        object.action.time,
                        target,
                        object.command_direction.to_script_value(),
                        container,
                        compact_ids(&object.contents),
                        menu,
                    )
                })
            })
            .unwrap_or_else(|| format!("frame={frame} cursor=none menu={menu}"))
    }

    fn menu_diagnostics(&self) -> String {
        self.engine
            .cursor_object_menu(self.owner)
            .map(|(object, menu)| {
                let selected = usize::try_from(menu.selection)
                    .ok()
                    .and_then(|index| menu.items.get(index))
                    .map(|item| format!("{:?}", item.caption))
                    .unwrap_or_else(|| "-".to_owned());
                format!(
                    "open@{} caption={:?} selection={}/{} selected={}",
                    object,
                    menu.caption,
                    menu.selection,
                    menu.items.len(),
                    selected,
                )
            })
            .unwrap_or_else(|| "closed".to_owned())
    }

    fn timeout_diagnostics(&self, recent: VecDeque<String>) -> String {
        let snapshot = self.engine.snapshot();
        let hud = snapshot
            .hud
            .players
            .iter()
            .find(|player| player.owner == self.owner)
            .map(|player| {
                let focus = player
                    .focus
                    .map_or_else(|| "-".to_owned(), |id| id.to_string());
                format!(
                    "owner={} focus={} crew={} wealth={} score={} eliminated={} messages={} \
                     scoreboard={}x{}/show={}",
                    player.owner,
                    focus,
                    compact_ids(&player.crew),
                    player.wealth,
                    player.score,
                    player.eliminated,
                    snapshot.hud.messages.len(),
                    snapshot.hud.scoreboard.row_count(),
                    snapshot.hud.scoreboard.column_count(),
                    snapshot.hud.scoreboard.show_count(),
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "owner={} missing messages={} scoreboard={}x{}/show={}",
                    self.owner,
                    snapshot.hud.messages.len(),
                    snapshot.hud.scoreboard.row_count(),
                    snapshot.hud.scoreboard.column_count(),
                    snapshot.hud.scoreboard.show_count(),
                )
            });
        let effects = snapshot
            .global_effects
            .iter()
            .take(RECENT_ID_LIMIT)
            .map(|effect| {
                format!(
                    "{:?}#{}(timer={},interval={},priority={})",
                    effect.name, effect.number, effect.timer, effect.interval, effect.priority,
                )
            })
            .collect::<Vec<_>>();
        let omitted_effects = snapshot.global_effects.len().saturating_sub(effects.len());
        let effect_suffix = if omitted_effects != 0 {
            format!(",...+{omitted_effects}")
        } else {
            String::new()
        };
        format!(
            "recent=[{}]; hud={{{hud}}}; global-effects=[{}{}]",
            recent.into_iter().collect::<Vec<_>>().join(" | "),
            effects.join(","),
            effect_suffix,
        )
    }
}

fn compact_ids(ids: &[ObjectId]) -> String {
    let shown = ids
        .iter()
        .take(RECENT_ID_LIMIT)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let omitted = ids.len().saturating_sub(shown.len());
    let suffix = if omitted != 0 {
        format!(",...+{omitted}")
    } else {
        String::new()
    };
    format!("[{}{}]", shown.join(","), suffix)
}
