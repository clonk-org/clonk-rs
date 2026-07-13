const CONTROL_SET_NONE: i32 = -1;
const KEYBOARD_SET_FIRST: i32 = 0;
const KEYBOARD_SET_LAST: i32 = 3;
const GAMEPAD_SET_FIRST: i32 = 4;
const GAMEPAD_SET_LAST: i32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalControlInit {
    pub(crate) owner: i32,
    pub(crate) preferred_set: i32,
    pub(crate) prefers_mouse: bool,
    pub(crate) gamepads_enabled: bool,
    pub(crate) replay: bool,
    pub(crate) disable_mouse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalControlAssignment {
    pub(crate) owner: i32,
    pub(crate) set: i32,
    pub(crate) mouse: bool,
}

#[derive(Debug, Default)]
pub(crate) struct LocalControlRegistry {
    assignments: Vec<LocalControlAssignment>,
}

impl LocalControlRegistry {
    pub(crate) fn initialize(&mut self, init: LocalControlInit) -> LocalControlAssignment {
        let assignment = LocalControlAssignment {
            owner: init.owner,
            set: CONTROL_SET_NONE,
            mouse: false,
        };
        self.assignments.push(assignment);
        let index = self.assignments.len() - 1;

        let mut preferred_set = init.preferred_set;
        if !init.gamepads_enabled && (GAMEPAD_SET_FIRST..=GAMEPAD_SET_LAST).contains(&preferred_set)
        {
            preferred_set = KEYBOARD_SET_FIRST;
        }
        let set = if self
            .assignments
            .iter()
            .any(|assignment| assignment.set == preferred_set)
        {
            (KEYBOARD_SET_FIRST..=KEYBOARD_SET_LAST)
                .find(|set| {
                    !self
                        .assignments
                        .iter()
                        .any(|assignment| assignment.set == *set)
                })
                .unwrap_or(CONTROL_SET_NONE)
        } else {
            preferred_set
        };
        self.assignments[index].set = set;
        self.assignments[index].mouse = init.prefers_mouse
            && !init.replay
            && !init.disable_mouse
            && (KEYBOARD_SET_FIRST..=GAMEPAD_SET_LAST).contains(&set)
            && !self.assignments.iter().any(|assignment| assignment.mouse);
        self.assignments[index]
    }

    pub(crate) fn owner_for_set(&self, set: i32) -> Option<i32> {
        self.assignments
            .iter()
            .find(|assignment| assignment.set == set)
            .map(|assignment| assignment.owner)
    }

    pub(crate) fn mouse_owner(&self) -> Option<i32> {
        self.assignments
            .iter()
            .find(|assignment| assignment.mouse)
            .map(|assignment| assignment.owner)
    }

    pub(crate) fn remove(&mut self, owner: i32) -> Option<LocalControlAssignment> {
        let index = self
            .assignments
            .iter()
            .position(|assignment| assignment.owner == owner)?;
        Some(self.assignments.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_preference_self_collides_and_fifth_player_keeps_none() {
        // C4PlayerList appends the provisional player before InitControl;
        // InitControl sets its local Control to -1 before ControlTaken(-1), so
        // the player collides with itself and scans keyboard sets 0..3. The
        // fifth such player finds none free and returns to -1 (pristine
        // 9ffa0a5d src/C4PlayerList.cpp:122-128,271-317;
        // src/C4Player.cpp:1718-1728,1871-1899).
        let mut controls = LocalControlRegistry::default();
        let assigned = (0..5)
            .map(|owner| {
                controls
                    .initialize(LocalControlInit {
                        owner,
                        preferred_set: CONTROL_SET_NONE,
                        prefers_mouse: false,
                        gamepads_enabled: true,
                        replay: false,
                        disable_mouse: false,
                    })
                    .set
            })
            .collect::<Vec<_>>();

        assert_eq!(assigned, vec![0, 1, 2, 3, CONTROL_SET_NONE]);
    }

    #[test]
    fn unique_invalid_set_is_retained_but_duplicates_scan_keyboards() {
        // InitControl range-checks only gamepad rewriting and mouse control.
        // A unique invalid preference therefore survives, while ControlTaken
        // sends later duplicates through the first-free keyboard scan
        // (pristine 9ffa0a5d src/C4Player.cpp:1881-1899;
        // src/C4PlayerList.cpp:122-128).
        let mut controls = LocalControlRegistry::default();
        let initialize = |controls: &mut LocalControlRegistry, owner| {
            controls.initialize(LocalControlInit {
                owner,
                preferred_set: 99,
                prefers_mouse: false,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            })
        };

        assert_eq!(initialize(&mut controls, 10).set, 99);
        assert_eq!(initialize(&mut controls, 11).set, 0);
        assert_eq!(initialize(&mut controls, 12).set, 1);
    }

    #[test]
    fn disabled_gamepads_rewrite_to_keyboard_one_before_collision_scan() {
        // C4P_Control_GamePad1..GamePadMax are 4..7. When gamepads are
        // disabled, InitControl rewrites any preference in that range to
        // Keyboard1 (0) before checking whether the set is already occupied
        // (pristine 9ffa0a5d src/C4Constants.h:84-93;
        // src/C4Player.cpp:1881-1897).
        let mut controls = LocalControlRegistry::default();
        let initialize =
            |controls: &mut LocalControlRegistry, owner, preferred_set, gamepads_enabled| {
                controls.initialize(LocalControlInit {
                    owner,
                    preferred_set,
                    prefers_mouse: false,
                    gamepads_enabled,
                    replay: false,
                    disable_mouse: false,
                })
            };

        assert_eq!(initialize(&mut controls, 20, 7, false).set, 0);
        assert_eq!(initialize(&mut controls, 21, 4, true).set, 4);
        assert_eq!(initialize(&mut controls, 22, 6, false).set, 1);
    }

    #[test]
    fn only_first_fully_eligible_player_gets_mouse_control() {
        // Mouse control requires PrefMouse, a non-replay game, enabled
        // scenario mouse, a final set in Keyboard1..GamePadMax, and no earlier
        // local mouse owner. These gates run after set fallback (pristine
        // 9ffa0a5d src/C4Player.cpp:1898-1912;
        // src/C4PlayerList.cpp:556-562).
        let mut controls = LocalControlRegistry::default();
        let initialize = |controls: &mut LocalControlRegistry,
                          owner,
                          preferred_set,
                          prefers_mouse,
                          replay,
                          disable_mouse| {
            controls.initialize(LocalControlInit {
                owner,
                preferred_set,
                prefers_mouse,
                gamepads_enabled: true,
                replay,
                disable_mouse,
            })
        };

        assert!(!initialize(&mut controls, 30, 8, true, false, false).mouse);
        assert!(!initialize(&mut controls, 31, 0, false, false, false).mouse);
        assert!(!initialize(&mut controls, 32, 1, true, true, false).mouse);
        assert!(!initialize(&mut controls, 33, 2, true, false, true).mouse);
        assert!(initialize(&mut controls, 34, 3, true, false, false).mouse);
        assert!(!initialize(&mut controls, 35, 4, true, false, false).mouse);
    }

    #[test]
    fn owner_lookup_returns_the_first_local_player_with_the_final_set() {
        // C4PlayerList::GetLocalByKbdSet walks list order and compares the
        // final Control value directly, without range validation (pristine
        // 9ffa0a5d src/C4PlayerList.cpp:156-162).
        let mut controls = LocalControlRegistry::default();
        let initialize = |controls: &mut LocalControlRegistry, owner, preferred_set| {
            controls.initialize(LocalControlInit {
                owner,
                preferred_set,
                prefers_mouse: false,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            })
        };
        assert_eq!(initialize(&mut controls, 40, 2).set, 2);
        assert_eq!(initialize(&mut controls, 41, 2).set, 0);
        assert_eq!(initialize(&mut controls, 42, 99).set, 99);

        assert_eq!(controls.owner_for_set(2), Some(40));
        assert_eq!(controls.owner_for_set(0), Some(41));
        assert_eq!(controls.owner_for_set(99), Some(42));
        assert_eq!(controls.owner_for_set(7), None);
    }

    #[test]
    fn mouse_owner_reports_the_first_eligible_local_player() {
        // MouseControlTaken scans the local player list for the first true
        // MouseControl flag (pristine 9ffa0a5d
        // src/C4PlayerList.cpp:556-562).
        let mut controls = LocalControlRegistry::default();
        let initialize = |controls: &mut LocalControlRegistry, owner, prefers_mouse| {
            controls.initialize(LocalControlInit {
                owner,
                preferred_set: owner - 50,
                prefers_mouse,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            })
        };

        assert!(!initialize(&mut controls, 50, false).mouse);
        assert!(initialize(&mut controls, 51, true).mouse);
        assert!(!initialize(&mut controls, 52, true).mouse);
        assert_eq!(controls.mouse_owner(), Some(51));
    }

    #[test]
    fn removing_a_player_releases_its_set_and_mouse_for_later_joins() {
        // C4PlayerList::Remove unlinks the player; subsequent ControlTaken and
        // MouseControlTaken scans therefore immediately reuse its resources
        // without reassigning surviving players (pristine 9ffa0a5d
        // src/C4PlayerList.cpp:122-128,219-268,556-562).
        let mut controls = LocalControlRegistry::default();
        let initialize = |controls: &mut LocalControlRegistry, owner| {
            controls.initialize(LocalControlInit {
                owner,
                preferred_set: 0,
                prefers_mouse: true,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            })
        };
        let first = initialize(&mut controls, 60);
        let second = initialize(&mut controls, 61);
        assert_eq!((first.set, first.mouse), (0, true));
        assert_eq!((second.set, second.mouse), (1, false));

        assert_eq!(controls.remove(999), None);
        assert_eq!(controls.remove(60), Some(first));
        assert_eq!(controls.owner_for_set(0), None);
        assert_eq!(controls.owner_for_set(1), Some(61));
        assert_eq!(controls.mouse_owner(), None);

        let replacement = initialize(&mut controls, 62);
        assert_eq!((replacement.set, replacement.mouse), (0, true));
    }
}
