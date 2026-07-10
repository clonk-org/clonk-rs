use lc_engine::{
    Engine, EngineError, ObjectMenuState, COM_CURSOR_TOGGLE, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_THROW, COM_UP,
};
use std::error::Error;
use std::fmt;

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
    },
    Timeout {
        milestone: String,
        max_ticks: u32,
        frame: u64,
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
            Self::MilestoneNotReached { milestone, frame } => {
                write!(
                    formatter,
                    "milestone `{milestone}` was not reached at frame {frame}"
                )
            }
            Self::Timeout {
                milestone,
                max_ticks,
                frame,
            } => write!(
                formatter,
                "timed out after {max_ticks} ticks waiting for `{milestone}` at frame {frame}"
            ),
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

impl<'engine> VirtualPlayer<'engine> {
    pub fn new(engine: &'engine mut Engine, owner: i32) -> Self {
        Self { engine, owner }
    }

    /// Read-only access for milestone predicates and assertions. The harness
    /// intentionally does not expose mutable engine access.
    pub fn engine(&self) -> &Engine {
        self.engine
    }

    /// Send a physical key-down control through `C4Player::InCom`.
    pub fn press(&mut self, control: u8) -> Result<(), VirtualPlayerError> {
        self.validate_control(control)?;
        self.engine.player_in_com(self.owner, control, 0)?;
        Ok(())
    }

    /// Send the matching key-up byte (`base + 16`). C++ ignores a release
    /// unless the key-down set its bit (`src/C4Player.cpp:1540-1548`).
    pub fn release(&mut self, control: u8) -> Result<(), VirtualPlayerError> {
        self.validate_control(control)?;
        self.engine
            .player_in_com(self.owner, control + COM_RELEASE_OFFSET, 0)?;
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
            self.engine.tick()?;
        }
        Ok(())
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
        if reached(self.engine) {
            return Ok(0);
        }
        for elapsed in 1..=max_ticks {
            self.engine.tick()?;
            if reached(self.engine) {
                return Ok(elapsed);
            }
        }
        Err(VirtualPlayerError::Timeout {
            milestone,
            max_ticks,
            frame: self.engine.frame(),
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
            Err(VirtualPlayerError::MilestoneNotReached {
                milestone: milestone.into(),
                frame: self.engine.frame(),
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
}
