//! `impl Engine` — custom commands, direct script execution and environment.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn custom_command_integer(argument: &[u8]) -> i32 {
        // std::from_chars accepts a leading '-' but not '+'. The oracle
        // strips exactly one '+' itself, ignores the returned end pointer,
        // and leaves the initialized result at zero on invalid/overflow.
        let argument = argument.strip_prefix(b"+").unwrap_or(argument);
        let (negative, digits) = match argument.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, argument),
        };
        let digit_count = digits
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digit_count == 0 {
            return 0;
        }
        let mut magnitude = 0_u64;
        for byte in &digits[..digit_count] {
            let Some(value) = magnitude
                .checked_mul(10)
                .and_then(|value| value.checked_add(u64::from(byte - b'0')))
            else {
                return 0;
            };
            magnitude = value;
        }
        if negative {
            if magnitude == i32::MAX as u64 + 1 {
                i32::MIN
            } else {
                i32::try_from(magnitude).map_or(0, |value| -value)
            }
        } else {
            i32::try_from(magnitude).unwrap_or(0)
        }
    }

    fn sprintf_custom_command(format: &str, argument: &str, specifier: char) -> Option<String> {
        // The registered command supplies one printf argument. Preserve %%
        // and consume exactly one literal matching %d/%s. Fail closed for
        // other, repeated, mixed, or malformed conversions instead of
        // DirectExecing a source C++ would not have composed this way.
        let mut argument = Some(argument);
        let mut output = String::with_capacity(format.len() + argument.unwrap_or("").len());
        let mut chars = format.chars().peekable();
        while let Some(current) = chars.next() {
            if current != '%' {
                output.push(current);
                continue;
            }
            match chars.peek().copied() {
                Some('%') => {
                    chars.next();
                    output.push('%');
                }
                Some(next) if next == specifier && argument.is_some() => {
                    chars.next();
                    output.push_str(argument.take().expect("argument was checked above"));
                }
                _ => return None,
            }
        }
        Some(output)
    }

    fn format_custom_command_script(
        command: &InitialNetworkMessageBoardCommand,
        argument: &[u8],
        player: i32,
    ) -> Option<String> {
        let script = command.script.replace("%player%", &player.to_string());
        if script.contains("%d") {
            let value = Self::custom_command_integer(argument).to_string();
            return Self::sprintf_custom_command(&script, &value, 'd');
        }
        if !script.contains("%s") {
            return Some(script);
        }

        let argument = match command.restriction {
            MessageBoardCommandRestriction::Escaped => {
                let mut escaped = Vec::with_capacity(argument.len());
                for byte in argument {
                    if matches!(*byte, b'\\' | b'"') {
                        escaped.push(b'\\');
                    }
                    escaped.push(*byte);
                }
                escaped
            }
            MessageBoardCommandRestriction::Plain => argument.to_vec(),
            MessageBoardCommandRestriction::Identifier => argument
                .iter()
                .copied()
                .take_while(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(*byte, b'_' | b'~' | b'+' | b'-')
                        || byte.is_ascii_whitespace()
                })
                .collect(),
        };
        let argument = clonk_script::c4_string_from_bytes(&argument);
        Self::sprintf_custom_command(&script, &argument, 's')
    }

    /// Execute one synchronized `CID_EMMoveObj` packet. The editor's local
    /// selection/property-dialog updates are intentionally absent; every
    /// simulation mutation and callback remains ordered exactly as carried
    /// by the packet's object-number array.
    pub fn execute_em_move_object_control(
        &mut self,
        control: &EmMoveObjectControlData,
        script_policy: ScriptControlPolicy,
    ) -> Result<bool, EngineError> {
        if self.league_game {
            return Ok(false);
        }

        let object_id = |number: i32| u64::try_from(number).ok().map(ObjectId::new);

        match control.action {
            EMMO_MOVE => {
                for &number in &control.objects {
                    let Some(index) = object_id(number)
                        .and_then(|id| self.find_object_index(id))
                        .filter(|&index| {
                            !self.objects[index].destroyed
                                && self.objects[index].state.status != ObjectStatus::Deleted
                        })
                    else {
                        continue;
                    };
                    let position = self.objects[index].state.position;
                    let target = Vector2::new(
                        position.x.wrapping_add(control.tx),
                        position.y.wrapping_add(control.ty),
                    );
                    self.force_object_position(index, target);
                    let object = &mut self.objects[index];
                    object.fixed_velocity = FixedVec2::ZERO;
                    object.state.velocity = Vector2::ZERO;
                    object.state.mobile = false;
                }
            }
            EMMO_ENTER => {
                // C++ resolves the target exactly once before iterating. The
                // retained object remains addressable if an earlier callback
                // marks it deleted; Enter performs its own live status gate.
                let Some(target_id) = object_id(control.target_object).filter(|&id| {
                    self.find_object_index(id).is_some_and(|index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != ObjectStatus::Deleted
                    })
                }) else {
                    return Ok(true);
                };
                for &number in &control.objects {
                    let Some(source_id) = object_id(number).filter(|&id| {
                        self.find_object_index(id).is_some_and(|index| {
                            !self.objects[index].destroyed
                                && self.objects[index].state.status != ObjectStatus::Deleted
                        })
                    }) else {
                        continue;
                    };
                    let _ = self.try_object_enter(source_id, target_id)?;
                }
            }
            EMMO_DUPLICATE => {
                for &number in &control.objects {
                    let Some(source_id) = object_id(number) else {
                        continue;
                    };
                    let Some((definition_id, owner, position, layer)) = self
                        .find_object_index(source_id)
                        .filter(|&index| {
                            !self.objects[index].destroyed
                                && self.objects[index].state.status != ObjectStatus::Deleted
                        })
                        .map(|index| {
                            let source = &self.objects[index];
                            (
                                source.definition_id.clone(),
                                source.state.owner,
                                source.state.position,
                                source.state.layer,
                            )
                        })
                    else {
                        continue;
                    };
                    let mut spawn = SpawnConfig::new(definition_id)
                        .with_position(position)
                        .with_owner(owner);
                    if let Some(layer) = layer {
                        spawn = spawn.with_layer(layer);
                    }
                    let _ = self.spawn_object_with_initial_lifecycle(spawn, Some(source_id))?;
                }
            }
            EMMO_SCRIPT => {
                // Unlike every other action, C4ControlScript receives each
                // raw number without a SafeObjectPointer prefilter. Missing
                // and deleted object numbers therefore execute in global
                // fallback scope, and duplicate entries execute repeatedly.
                for &target_object in &control.objects {
                    let nested = ScriptControlData {
                        target_object,
                        strictness: control.strictness,
                        script: control.script.clone(),
                        by_client: control.by_client,
                    };
                    let _ = self.execute_script_control(&nested, script_policy)?;
                }
            }
            EMMO_REMOVE => {
                for &number in &control.objects {
                    let Some(id) = object_id(number).filter(|&id| {
                        self.find_object_index(id).is_some_and(|index| {
                            !self.objects[index].destroyed
                                && self.objects[index].state.status != ObjectStatus::Deleted
                        })
                    }) else {
                        continue;
                    };
                    let _ = self.assign_object_removal(id)?;
                }
            }
            EMMO_EXIT => {
                for &number in &control.objects {
                    let Some(id) = object_id(number).filter(|&id| {
                        self.find_object_index(id).is_some_and(|index| {
                            !self.objects[index].destroyed
                                && self.objects[index].state.status != ObjectStatus::Deleted
                        })
                    }) else {
                        continue;
                    };
                    let _ = self.exit_object_at_current_transform(id)?;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// Execute one synchronized `CID_EMDrawTool` packet. The process-local
    /// tools dialog is intentionally absent; landscape mode, raster writes,
    /// and Fill's synchronized RNG/InsertMaterial sequence are authoritative.
    pub fn execute_em_draw_tool_control(&mut self, control: &EmDrawToolControlData) -> bool {
        if self.league_game {
            return false;
        }
        if control.action == EMDT_SET_MODE {
            self.set_editor_landscape_mode(control.mode);
            return true;
        }
        if self.landscape.as_ref().map(Landscape::mode) != Some(control.mode)
            || control.material.is_empty()
        {
            return false;
        }

        let material = clonk_script::c4_string_from_bytes(control.material.as_bytes());
        match control.action {
            EMDT_BRUSH | EMDT_LINE | EMDT_RECT => {
                if control.texture.is_empty() {
                    return false;
                }
                let texture = clonk_script::c4_string_from_bytes(control.texture.as_bytes());
                self.draw_editor_landscape(
                    control.action,
                    control.x,
                    control.y,
                    control.x2,
                    control.y2,
                    control.grade,
                    &material,
                    &texture,
                    control.ift,
                )
            }
            EMDT_FILL => {
                let Some(material) = self.materials.id_of(&material) else {
                    return false;
                };
                for _ in 0..control.grade {
                    // C++ pins evaluation order explicitly: the Y draw is
                    // first, then X, before each InsertMaterial call.
                    let r2 = control
                        .y
                        .wrapping_add(self.rng.random(control.grade))
                        .wrapping_sub(control.grade / 2);
                    let r1 = control
                        .x
                        .wrapping_add(self.rng.random(control.grade))
                        .wrapping_sub(control.grade / 2);
                    let _ = self.insert_material(material, r1, r2, 0, 0);
                }
                true
            }
            _ => true,
        }
    }

    /// Execute one synchronized `CID_EMDropDef` packet through the same
    /// strict global internal-script path as `C4ControlEMDropDef`.
    pub fn execute_em_drop_def_control(
        &mut self,
        control: &EmDropDefControlData,
    ) -> Result<bool, EngineError> {
        if self.league_game {
            return Ok(false);
        }

        let encoded = clonk_script::c4_string_from_bytes(&control.id);
        let raw_id = clonk_script::c4_id_parse(&encoded);
        if raw_id == 0 {
            return Ok(false);
        }
        let definition_id = clonk_script::c4_id_from_raw(raw_id);
        let Some(category) = self.definition_category(&definition_id) else {
            return Ok(false);
        };
        let definition_text = clonk_script::c4_id_text(&definition_id);
        // C++'s lexer accepts the numeric-to-C4ID transition in e.g. 1_AA;
        // clonk-script's numeric lexer does not yet consume that underscore.
        // An explicit C4Id conversion is behaviorally identical for this
        // otherwise-valid four-byte ID and keeps this control executable.
        let numeric_underscore_id = control
            .id
            .iter()
            .position(|byte| !byte.is_ascii_digit())
            .is_some_and(|index| {
                index > 0
                    && control.id[index] == b'_'
                    && control.id[index + 1..]
                        .iter()
                        .all(u8::is_ascii_alphanumeric)
            });
        let definition_expression = if numeric_underscore_id {
            format!("C4Id(\"{definition_text}\")")
        } else {
            definition_text
        };
        // `-2147483648` is a unary minus plus an out-of-range positive token
        // in clonk-script. Spell the same i32 value as an in-range expression.
        let script_i32 = |value: i32| {
            if value == i32::MIN {
                "(-2147483647-1)".to_string()
            } else {
                value.to_string()
            }
        };
        let x = script_i32(control.x);
        let y = script_i32(control.y);
        let source = if category & CATEGORY_STRUCTURE != 0 {
            format!(
                "CreateConstruction({},{},{},-1,{},true)",
                definition_expression, x, y, FULL_CON
            )
        } else {
            format!("CreateObject({},{},{},-1)", definition_expression, x, y)
        };
        self.execute_internal_script_at_scope(SCRIPT_SCOPE_GLOBAL, &source)?;
        Ok(true)
    }

    /// Execute the local presentation half of a `C4CMT_Say` message control.
    /// The message uses the raw C++ `ViewTarget`-then-`Cursor` lookup rather
    /// than the resolved viewport mode, and rechecks the packet's player
    /// ownership before exposing any text (`src/C4Control.cpp:1075-1079,
    /// 1139-1155`).
    pub fn execute_message_control_say(&mut self, control: &MessageControlData) -> bool {
        if control.message_type != MESSAGE_TYPE_SAY {
            return false;
        }

        let Some((view_object, cursor, player_name, player_color)) = self
            .player(control.player)
            .filter(|player| player.at_client() == PlayerAtClient::new(control.by_client))
            .and_then(|player| {
                let view_object = player.raw_view_target_or_cursor()?;
                let color = player
                    .color()
                    .map(|color| {
                        u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                    })
                    .unwrap_or(0);
                Some((
                    view_object,
                    player.cursor(),
                    player.name().to_string(),
                    color,
                ))
            })
        else {
            return false;
        };

        let Some(view_index) = self.find_object_index(view_object) else {
            return false;
        };
        if self.objects[view_index].destroyed
            || self.objects[view_index].state.status == ObjectStatus::Deleted
        {
            return false;
        }

        let cinematic = matches!(
            self.scenario_values.get("Film", Some("Head"), 0),
            Some(scenario::ScenarioValue::Int(2))
        );
        let raw_message = clonk_script::c4_string_from_bytes(control.message.as_bytes());
        let (text, color) = if cinematic {
            let cursor_presentation = cursor.and_then(|cursor| {
                let index = self.find_object_index(cursor)?;
                let object = &self.objects[index];
                if object.destroyed || object.state.status == ObjectStatus::Deleted {
                    return None;
                }
                let name = object
                    .state
                    .custom_name
                    .as_ref()
                    .filter(|name| !name.is_empty())
                    .cloned()
                    .or_else(|| {
                        self.crew_object_infos
                            .get(&cursor)
                            .map(|info| info.name.clone())
                    })
                    .or_else(|| {
                        self.definitions
                            .get(&object.definition_id)
                            .map(|definition| definition.name().to_string())
                    })
                    .unwrap_or_else(|| object.definition_id.clone());
                Some((name, object.state.color))
            });
            let (speaker, color) = cursor_presentation.unwrap_or((player_name, player_color));
            (
                format!("<{speaker}> {raw_message}"),
                if color == 0 { 0xff } else { color },
            )
        } else {
            (raw_message, player_color)
        };

        self.messages.add_message(MessageSpec {
            kind: message::MessageKind::Target,
            text,
            target: Some(view_object),
            player: None,
            offset: Vector2::ZERO,
            color: color | 0xff00_0000,
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        });
        true
    }

    /// `C4S.Head.Film == C4SFilm_Cinematic`, used while classifying outgoing
    /// quote-prefixed Say messages before the private control is queued.
    pub fn cinematic_film(&self) -> bool {
        matches!(
            self.scenario_values.get("Film", Some("Head"), 0),
            Some(scenario::ScenarioValue::Int(2))
        )
    }

    /// C++ truthiness of `C4S.Head.Film`. Replays suppress ordinary viewport
    /// overlays when this is set, including the local mouse-button stack.
    pub fn film(&self) -> bool {
        matches!(
            self.scenario_values.get("Film", Some("Head"), 0),
            Some(scenario::ScenarioValue::Int(value)) if *value != 0
        )
    }

    /// C++ truthiness of the persistent `C4S.Head.Replay` scenario flag.
    pub fn replay(&self) -> bool {
        matches!(
            self.scenario_values.get("Replay", Some("Head"), 0),
            Some(scenario::ScenarioValue::Int(value)) if *value != 0
        )
    }

    /// C++ truthiness of the persistent `C4S.Game.ValueGain` scenario value.
    pub fn scenario_value_gain_enabled(&self) -> bool {
        matches!(
            self.scenario_values.get("ValueGain", Some("Game"), 0),
            Some(scenario::ScenarioValue::Int(value)) if *value != 0
        )
    }

    /// Whether the current scenario selects C++'s film-view keyboard scope.
    /// Both normal and cinematic films count; the raw replay flag remains
    /// authoritative even after control playback reaches its end marker.
    pub fn film_replay(&self) -> bool {
        self.replay() && self.film()
    }

    /// Whether fullscreen viewport reconciliation uses replay-film fallback:
    /// after the sole viewport closes, C++ recreates it for the first player
    /// instead of creating the silent ownerless observer viewport.
    pub fn is_replay_film(&self) -> bool {
        self.film_replay()
    }

    /// Execute one synchronized `CID_CustomCommand` packet. Player ownership
    /// is checked first, followed by the running/registration gates; accepted
    /// packets execute the currently registered template in global scope.
    pub fn execute_custom_command_control(
        &mut self,
        control: &CustomCommandControlData,
        game_running: bool,
    ) -> Result<bool, EngineError> {
        const NO_OWNER: i32 = -1;
        if control.player != NO_OWNER
            && !self
                .player(control.player)
                .is_some_and(|player| player.at_client() == PlayerAtClient::new(control.by_client))
        {
            return Ok(false);
        }
        if !game_running {
            return Ok(false);
        }
        let Some(command) = self
            .message_board_commands
            .iter()
            .find(|command| {
                clonk_script::c4_string_bytes(&command.name) == control.command.as_bytes()
            })
            .cloned()
        else {
            return Ok(false);
        };

        let Some(source) = Self::format_custom_command_script(
            &command,
            control.argument.as_bytes(),
            control.player,
        ) else {
            return Ok(true);
        };
        let _ = self.direct_exec_script_control_global(&source, "internal script", Some(3))?;
        Ok(true)
    }

    fn internal_player_script_allowed(&self, player: i32, by_client: i32) -> bool {
        player == OWNER_NONE
            || self
                .player(player)
                .is_some_and(|player| player.at_client() == PlayerAtClient::new(by_client))
    }

    fn execute_internal_script_at_scope(
        &mut self,
        scope: i32,
        source: &str,
    ) -> Result<(), EngineError> {
        if scope == SCRIPT_SCOPE_CONSOLE {
            let _ = self.direct_exec_scenario_script(source, "internal script", Some(3))?;
            return Ok(());
        }
        if scope == SCRIPT_SCOPE_GLOBAL {
            let _ = self.direct_exec_script_control_global(source, "internal script", Some(3))?;
            return Ok(());
        }

        let object_index = u64::try_from(scope)
            .ok()
            .and_then(|number| self.find_object_index(ObjectId::new(number)))
            .filter(|&index| self.objects[index].state.status != ObjectStatus::Deleted);
        if let Some(index) = object_index {
            let _ = tolerate_script_error(self.direct_exec_on_object_at_strict(
                index,
                source,
                "internal script",
                Some(3),
            ))?;
        } else {
            let _ = self.direct_exec_script_control_global(source, "internal script", Some(3))?;
        }
        Ok(())
    }

    /// Execute `CID_ActivateGameGoalMenu` through the strict internal-script
    /// path after applying the inherited player/author gate.
    pub fn execute_activate_game_goal_menu_control(
        &mut self,
        control: &ActivateGameGoalMenuControlData,
    ) -> Result<bool, EngineError> {
        if !self.internal_player_script_allowed(control.player, control.by_client) {
            return Ok(false);
        }
        self.execute_internal_script_at_scope(
            SCRIPT_SCOPE_GLOBAL,
            &format!("ActivateGameGoalMenu({})", control.player),
        )?;
        Ok(true)
    }

    /// Execute `CID_ToggleHostility`. The one-way hostility declaration is
    /// read by the script at execution time, so adjacent packets flip it
    /// independently in their synchronized order.
    pub fn execute_toggle_hostility_control(
        &mut self,
        control: &ToggleHostilityControlData,
    ) -> Result<bool, EngineError> {
        if !self.internal_player_script_allowed(control.player, control.by_client) {
            return Ok(false);
        }
        self.execute_internal_script_at_scope(
            SCRIPT_SCOPE_GLOBAL,
            &format!(
                "SetHostility({},{},!Hostile({},{},true))",
                control.player, control.opponent, control.player, control.opponent
            ),
        )?;
        Ok(true)
    }

    /// Execute object-scoped `CID_ActivateGameGoalRule`, including the C++
    /// fallback to global scope when the object pointer is not safe.
    pub fn execute_activate_game_goal_rule_control(
        &mut self,
        control: &ActivateGameGoalRuleControlData,
    ) -> Result<bool, EngineError> {
        if !self.internal_player_script_allowed(control.player, control.by_client) {
            return Ok(false);
        }
        self.execute_internal_script_at_scope(
            control.object,
            &format!("Activate({})", control.player),
        )?;
        Ok(true)
    }

    /// Execute `CID_SetPlayerTeam`; team/league/callback validation remains
    /// inside the ordinary `SetPlayerTeam` host function.
    pub fn execute_set_player_team_control(
        &mut self,
        control: &SetPlayerTeamControlData,
    ) -> Result<bool, EngineError> {
        if !self.internal_player_script_allowed(control.player, control.by_client) {
            return Ok(false);
        }
        self.execute_internal_script_at_scope(
            SCRIPT_SCOPE_GLOBAL,
            &format!("SetPlayerTeam({},{})", control.player, control.team),
        )?;
        Ok(true)
    }

    /// Execute host-only `CID_EliminatePlayer`. This override intentionally
    /// does not apply the inherited player ownership gate.
    pub fn execute_eliminate_player_control(
        &mut self,
        control: &EliminatePlayerControlData,
    ) -> Result<bool, EngineError> {
        if control.by_client != 0 {
            return Ok(false);
        }
        self.execute_internal_script_at_scope(
            SCRIPT_SCOPE_GLOBAL,
            &format!("EliminatePlayer({})", control.player),
        )?;
        Ok(true)
    }

    /// Execute one synchronized `CID_MessageBoardAnswer` packet. The packet
    /// is accepted only for the client that owns the addressed player;
    /// `NO_OWNER` remains the C++ control's explicit ownerless exception.
    ///
    /// C++ formats and parses a strict-3 internal
    /// `OnMessageBoardAnswer(Object(...), ...)` script. Keeping that source
    /// path preserves its quote/backslash escaping as well as parse failures
    /// for line breaks and overlong string literals.
    pub fn execute_message_board_answer_control(
        &mut self,
        control: &MessageBoardAnswerControlData,
    ) -> Result<bool, EngineError> {
        const NO_OWNER: i32 = -1;
        let allowed = control.player == NO_OWNER
            || self
                .player(control.player)
                .is_some_and(|player| player.at_client() == PlayerAtClient::new(control.by_client));
        if !allowed {
            return Ok(false);
        }

        let source = if control.answer.is_empty() {
            format!(
                "OnMessageBoardAnswer(Object({}),{},)",
                control.object, control.player
            )
        } else {
            // C4AUL_MAX_String is 1024 bytes. EscapeString does not replace
            // CR/LF, so either condition makes DirectExec fail before the
            // native function can consume the query.
            if control.answer.as_bytes().len() > 1024
                || control.answer.as_bytes().contains(&b'\n')
                || control.answer.as_bytes().contains(&b'\r')
            {
                return Ok(true);
            }
            let mut escaped = Vec::with_capacity(control.answer.as_bytes().len());
            for byte in control.answer.as_bytes() {
                if matches!(*byte, b'\\' | b'"') {
                    escaped.push(b'\\');
                }
                escaped.push(*byte);
            }
            let escaped = clonk_script::c4_string_from_bytes(&escaped);
            format!(
                "OnMessageBoardAnswer(Object({}),{},\"{}\")",
                control.object, control.player, escaped
            )
        };

        let _ = self.direct_exec_script_control_global(&source, "console script", Some(3))?;
        Ok(true)
    }

    /// Execute one synchronized `CID_Script` packet. `Ok(None)` denotes a
    /// packet rejected by the league/sender policy; an allowed packet always
    /// returns a value, with parse or fail-safe runtime errors represented as
    /// script `nil` after committing any side effects made before the error.
    pub fn execute_script_control(
        &mut self,
        control: &ScriptControlData,
        policy: ScriptControlPolicy,
    ) -> Result<Option<Value>, EngineError> {
        if self.league_game {
            return Ok(None);
        }
        if control.by_client != 0 {
            let permitted = if policy.is_replay {
                policy.allow_scripting_in_replays
            } else {
                policy.console_active
            };
            if !permitted {
                return Ok(None);
            }
        }

        let source = clonk_script::c4_string_from_bytes(control.script.as_bytes());
        let strict_level = control.strictness.level();

        if control.target_object == SCRIPT_SCOPE_CONSOLE {
            return self
                .direct_exec_script_control_console(&source, strict_level)
                .map(Some);
        }

        let object_index = u64::try_from(control.target_object)
            .ok()
            .and_then(|number| self.find_object_index(ObjectId::new(number)))
            .filter(|&index| self.objects[index].state.status != ObjectStatus::Deleted);
        if let Some(index) = object_index {
            let value = tolerate_script_error(self.direct_exec_on_object_at_strict(
                index,
                &source,
                "console script",
                strict_level,
            ))?
            .unwrap_or(Value::Nil);
            return Ok(Some(value));
        }

        self.direct_exec_script_control_global(&source, "console script", strict_level)
            .map(Some)
    }

    /// Rebuild `C4Console::UpdateInputCtrl`'s two function groups from the
    /// live script engine. Native functions honor C++ `GetPublic`; scenario
    /// functions use the local `GetSFunc(index)` view regardless of script
    /// access. The groups remain distinct for platform-specific combo-box
    /// layout, and scenario declarations retain `GetSFunc`'s reverse-source
    /// traversal so Win32 can reverse them again when inserting at index zero.
    pub fn console_script_completion_catalog(&self) -> ConsoleScriptCompletionCatalog {
        let mut engine_functions =
            compat::public_console_host_function_names(&self.script_control_global_host());
        if let Some(functions) = self.global_script_functions.as_deref() {
            engine_functions.extend(functions.keys().cloned());
        }
        engine_functions.sort();
        engine_functions.dedup();

        let scenario_functions = self
            .scenario_script
            .as_ref()
            .map(ScenarioScript::local_function_names_in_get_sfunc_order)
            .unwrap_or_default();

        ConsoleScriptCompletionCatalog {
            engine_functions,
            scenario_functions,
        }
    }

    /// Build the script-engine scope used by `SCOPE_Global`. It deliberately
    /// has no scenario/definition-local functions, but shares every engine
    /// global cell and function and carries the normal native host surface.
    pub(crate) fn script_control_global_host(&self) -> ScriptEngine {
        let mut script = ScriptEngine::new();
        script.set_script_name("System.c4g");
        script.set_game_script_name(
            self.scenario_script
                .as_ref()
                .map(|scenario| scenario.script.script_name())
                .unwrap_or("Script.c"),
        );
        script.set_global_variables(self.script_globals.clone());
        script.set_global_slots(self.script_global_slots.clone());
        script.set_global_constants(self.script_global_consts.clone());
        script.set_string_registrations(self.script_string_registrations.clone());
        script.set_global_functions(self.global_script_functions.clone());
        compat::register_host_functions(&mut script);
        script
    }

    fn direct_exec_script_control_console(
        &mut self,
        source: &str,
        strict_level: Option<u8>,
    ) -> Result<Value, EngineError> {
        self.direct_exec_scenario_script(source, "console script", strict_level)
    }

    pub(crate) fn direct_exec_scenario_script(
        &mut self,
        source: &str,
        function_label: &str,
        strict_level: Option<u8>,
    ) -> Result<Value, EngineError> {
        let Some((name, script)) = self
            .scenario_script
            .as_ref()
            .map(|scenario| (scenario.name.clone(), scenario.script_arc()))
        else {
            // Game.Script exists even when the scenario supplied no Script.c;
            // its empty host still resolves through Game.ScriptEngine.
            let mut script = self.script_control_global_host();
            script.set_script_name("Script.c");
            return self.direct_exec_script_control_host(
                "Game.Script",
                &script,
                source,
                function_label,
                strict_level,
            );
        };
        self.direct_exec_script_control_host(
            &name,
            script.as_ref(),
            source,
            function_label,
            strict_level,
        )
    }

    pub(crate) fn direct_exec_script_control_global(
        &mut self,
        source: &str,
        function_label: &str,
        strict_level: Option<u8>,
    ) -> Result<Value, EngineError> {
        let script = self.script_control_global_host();
        self.direct_exec_script_control_host(
            "Game.ScriptEngine",
            &script,
            source,
            function_label,
            strict_level,
        )
    }

    /// `Game.ScriptEngine.GetFuncRecursive(...)->Exec(nullptr, args)`: call
    /// strictly in engine-global scope (script globals before the native host
    /// function), with no object `this`. This is observably different from
    /// `C4Object::Call`, where a same-named local definition function wins.
    pub(crate) fn call_engine_global_function(
        &mut self,
        function: &str,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        let script = self.script_control_global_host();
        let world = self.host_world_context();
        let (value, _final_args, batch, audio_state, rng, script_error) =
            ScenarioScript::call_value_for_script(
                "Game.ScriptEngine",
                &script,
                None,
                function,
                args,
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            if !matches!(error, EngineError::Script { .. }) {
                return Err(error);
            }
        }
        Ok(value.unwrap_or(Value::Nil))
    }

    /// `Game.Script.Call` with the exact C++ argument list and raw return
    /// value. Unlike [`Self::call_scenario_script_function`], this does not
    /// prepend the fixture-only scenario state and keeps fail-safe callback
    /// errors as a silent miss.
    pub(crate) fn call_scenario_script_value(
        &mut self,
        function: &str,
        args: &[Value],
    ) -> Result<Option<Value>, EngineError> {
        let Some((name, script)) = self.scenario_script.as_ref().and_then(|scenario| {
            scenario
                .script
                .has_local_function(function)
                .then(|| (scenario.name.clone(), scenario.script_arc()))
        }) else {
            return Ok(None);
        };
        let world = self.host_world_context();
        let (value, _final_args, batch, audio_state, rng, script_error) =
            ScenarioScript::call_value_for_script(
                &name,
                script.as_ref(),
                None,
                function,
                args,
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            if !matches!(error, EngineError::Script { .. }) {
                return Err(error);
            }
            return Ok(None);
        }
        Ok(value)
    }

    /// `C4MCOverlay::AlgoScript`: resolve a scenario-local function afresh,
    /// call it with no object context, and keep the complete synchronous call
    /// inside C4Landscape's temporary FixRandom ledger. Script failures are
    /// the native false fallback, but mutations completed before the failure
    /// remain live.
    pub(crate) fn call_map_script_algorithm(
        &mut self,
        rng: &mut LcgRng,
        function: &str,
        args: [i32; 4],
    ) -> bool {
        let Some((name, script)) = self.scenario_script.as_ref().and_then(|scenario| {
            scenario
                .script
                .has_local_function(function)
                .then(|| (scenario.name.clone(), scenario.script_arc()))
        }) else {
            return false;
        };
        let args = args.map(Value::Int);
        let world = self.host_world_context();
        let (value, _final_args, batch, audio_state, map_rng, script_error) =
            ScenarioScript::call_value_for_script(
                &name,
                script.as_ref(),
                None,
                function,
                &args,
                world,
                rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
            );

        // Batch application can synchronously construct objects and invoke
        // further callbacks. Those draws belong to the same fixed map epoch,
        // never to the saved gameplay ledger restored after Landscape::Init.
        let saved_game_rng = std::mem::replace(&mut self.rng, map_rng);
        self.audio_registry = audio_state;
        let batch_ok = self.apply_scenario_batch(batch).is_ok();
        *rng = std::mem::replace(&mut self.rng, saved_game_rng);

        batch_ok && script_error.is_none() && value.is_some_and(|value| value.as_bool())
    }

    pub(crate) fn direct_exec_script_control_host(
        &mut self,
        script_name: &str,
        script: &ScriptEngine,
        source: &str,
        function_label: &str,
        strict_level: Option<u8>,
    ) -> Result<Value, EngineError> {
        let world = self.host_world_context();
        let (value, batch, audio_state, rng, script_error) =
            ScenarioScript::direct_exec_value_for_script(
                script_name,
                script,
                source,
                function_label,
                strict_level,
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            // The raw-value seam records ordinary script failures here after
            // preserving their staged side effects. Fatal engine errors are
            // still surfaced rather than being converted to `nil`.
            if !matches!(error, EngineError::Script { .. }) {
                return Err(error);
            }
        }
        Ok(value.unwrap_or(Value::Nil))
    }

    pub fn set_control_host(&mut self, control_host: bool) {
        self.control_host = control_host;
    }

    pub fn set_replay_control(&mut self, replay_control: bool) {
        self.replay_control = replay_control;
    }

    /// `C4Playback::Finish`: end the replayed round and return control to the
    /// local host after the end marker has been consumed.
    pub fn finish_replay(&mut self) -> Result<(), EngineError> {
        self.request_game_over()?;
        self.replay_control = false;
        self.network_control_mode = false;
        self.control_host = true;
        self.control_rate = 1;
        Ok(())
    }

    /// Set the app-owned physical viewport availability sampled by replay
    /// `SetFilmView`. This is process-local and excluded from EngineState.
    pub fn set_film_viewport_available(&mut self, available: bool) {
        self.film_viewport_available = available;
    }

    /// Set the ordered process-local physical viewport targets, including
    /// OWNER_NONE observer slots. This projection is presentation-only and
    /// excluded from EngineState, like `film_viewport_available`.
    pub fn set_physical_viewport_players<I>(&mut self, players: I)
    where
        I: IntoIterator<Item = i32>,
    {
        *self.physical_viewport_players.borrow_mut() = players.into_iter().collect();
    }

    #[doc(hidden)]
    pub fn is_control_host(&self) -> bool {
        self.control_host
    }

    /// Drains authoritative CreateScriptPlayer requests in call order. The
    /// app feeds them through PlayerInfo admission before any JoinPlayer is
    /// issued, matching C4PlayerInfoList::DoPlayerInfoUpdate.
    pub fn take_script_player_info_updates(&mut self) -> Vec<PlayerInfoUpdateRequest> {
        self.host_requests
            .player_info_updates
            .borrow_mut()
            .drain(..)
            .collect()
    }

    /// Drain `SetLeagueProgressData` writes in script execution order so the
    /// app's retained C4PlayerInfo registry remains the same object graph as
    /// the engine-side script projection.
    pub fn take_player_info_league_progress_updates(&mut self) -> Vec<(i32, Option<Vec<u8>>)> {
        std::mem::take(&mut self.host_requests.player_info_league_progress_updates)
    }

    /// Drain host-authored `CID_ClientUpdate(CUT_Activate, false)` requests
    /// emitted by `C4Player::Eliminate` in execution order. The embedding
    /// control layer assigns them to a synchronized tick.
    pub fn take_pending_client_updates(&mut self) -> Vec<ClientUpdateControlData> {
        std::mem::take(&mut self.host_requests.pending_client_updates)
    }

    /// Drain host-authored `CID_RemovePlr` requests in script call order.
    /// The embedding control layer assigns them to a not-yet-executed tick.
    pub fn take_pending_remove_player_controls(&mut self) -> Vec<RemovePlayerControlData> {
        std::mem::take(&mut self.host_requests.pending_remove_player_controls)
    }

    /// Drain goal evaluations in synchronized control order. Remote/replay
    /// requests remain observable with `open_menu == false` so callers can
    /// discard only the presentation while retaining callback execution.
    pub fn take_game_goal_menu_requests(&mut self) -> Vec<GameGoalMenuRequest> {
        std::mem::take(&mut self.host_requests.pending_game_goal_menu_requests)
    }

    /// Drain process-local pause actions in script call order. Replay calls
    /// are suppressed before they reach this app-owned request channel.
    pub fn take_pause_game_requests(&mut self) -> Vec<PauseGameRequest> {
        std::mem::take(&mut *self.host_requests.pause_game_requests.borrow_mut())
    }

    /// Apply the reloads `FnReloadParticle` accepted during the last script
    /// call (`C4Game::ReloadParticle`, `C4Game.cpp:2369-2394`).
    ///
    /// The builtin answered synchronously from pre-seeded state, so this is
    /// only the work; the script already has its result. A reload that fails
    /// here still clears every particle and drops the definition, exactly as
    /// the direct call does — the script simply saw the optimistic answer,
    /// which is the one narrow divergence this design accepts.
    /// Apply the reloads `FnReloadDef` accepted during the last script call
    /// (`C4Game::ReloadDef`, `C4Game.cpp:2322-2367`).
    pub fn apply_definition_reload_requests(&mut self) -> usize {
        let requests =
            std::mem::take(&mut *self.host_requests.definition_reload_requests.borrow_mut());
        let network_game = self.network_game;
        requests
            .into_iter()
            .filter(|id| self.reload_definition(id, network_game))
            .count()
    }

    pub fn apply_particle_reload_requests(&mut self) -> usize {
        let requests =
            std::mem::take(&mut *self.host_requests.particle_reload_requests.borrow_mut());
        let network_game = self.network_game;
        requests
            .into_iter()
            .filter(|name| self.reload_particle(name, network_game))
            .count()
    }

    /// Drain local `SetPreSend` requests in exact script-call order. This is
    /// runtime-only state and never enters synchronized snapshots or saves.
    pub fn take_network_target_fps_requests(&mut self) -> Vec<NetworkTargetFpsRequest> {
        std::mem::take(&mut *self.host_requests.network_target_fps_requests.borrow_mut())
    }

    /// Drain physical viewport mutations in exact script call order.
    pub fn take_viewport_presentation_requests(&mut self) -> Vec<ViewportPresentationRequest> {
        std::mem::take(
            &mut *self
                .host_requests
                .viewport_presentation_requests
                .borrow_mut(),
        )
    }

    /// Update the process-local developer-console target queried by the
    /// script `EditCursor` builtin. Unknown or removed objects read as nil.
    pub fn set_edit_cursor_target(&mut self, target: Option<ObjectId>) {
        self.edit_cursor_target = target;
    }

    pub fn set_local_players<I>(&mut self, players: I)
    where
        I: IntoIterator<Item = i32>,
    {
        self.local_players = Some(players.into_iter().collect());
    }

    pub fn active_message_board_input(&self) -> Option<&ActiveMessageBoardInput> {
        self.active_message_board_input.as_ref()
    }

    /// Register one `C4MessageInput` custom command. Matching is exact and a
    /// duplicate name leaves the first entry untouched, like C++ AddCommand.
    pub fn add_message_board_command(
        &mut self,
        command: InitialNetworkMessageBoardCommand,
    ) -> bool {
        if self.message_board_commands.iter().any(|registered| {
            clonk_script::c4_string_bytes(&registered.name)
                == clonk_script::c4_string_bytes(&command.name)
        }) {
            return false;
        }
        self.message_board_commands.push(command);
        true
    }

    pub fn message_board_commands(&self) -> &[InitialNetworkMessageBoardCommand] {
        &self.message_board_commands
    }

    /// Whether any active in-game message line contains `needle`, without
    /// cloning the complete simulation snapshot.
    pub fn message_line_contains(&self, needle: &str) -> bool {
        self.messages.line_contains(needle)
    }

    pub fn environment(&self) -> EnvironmentSettings {
        self.environment
    }

    /// The nine raw Game.GraphicsSystem.dwGamma controls. Applying these to
    /// the renderer LUT is deliberately outside this engine-state slice.
    pub fn gamma_controls(&self) -> &GammaControlState {
        &self.gamma
    }

    /// Add the nine controls exactly like C4GraphicsSystem::ApplyGamma
    /// (C4GraphicsSystem.cpp:787-809), returning its three packed RGB points.
    pub fn effective_gamma_control_points(&self) -> [u32; 3] {
        self.gamma.combined_control_points()
    }

    /// C4GraphicsSystem::Default (C4GraphicsSystem.cpp:277-281), used at a
    /// fresh scenario boundary before its Initialize script may set slot 0.
    pub(crate) fn reset_gamma_controls(&mut self) {
        self.gamma = GammaControlState::default();
    }

    /// C4Weather::SetSeasonGamma writes the weather curve into
    /// C4GRI_SEASON (slot 1) and leaves the existing slot untouched when
    /// NoGamma is set (C4Weather.cpp:259-284).
    pub(crate) fn refresh_season_gamma_control(&mut self) {
        if let Some(points) = self.environment.season_gamma_control_points() {
            let _ = self.gamma.set_ramp(1, points);
        }
    }

    /// `C4Weather::Init(false)` for a compiled savegame: no scenario-value
    /// evaluation or weather mutation, only the final season-gamma refresh.
    pub(crate) fn refresh_loaded_weather_gamma_control(&mut self) {
        self.refresh_season_gamma_control();
    }

    pub(crate) fn apply_environment_delta(&mut self, delta: &EnvironmentDelta) {
        let refresh_gamma = delta.requests_season_gamma_refresh();
        delta.apply(&mut self.environment);
        if refresh_gamma {
            self.refresh_season_gamma_control();
        }
    }

    pub fn set_environment(&mut self, environment: EnvironmentSettings) {
        let mut environment = environment;
        environment.refresh_runtime_fields();
        self.environment = environment;
    }

    pub fn set_sky(&mut self, settings: SkySettings) {
        self.sky = Some(SkyState::new(settings));
    }

    pub fn clear_sky(&mut self) {
        self.sky = None;
    }

    pub fn sky_settings(&self) -> Option<&SkySettings> {
        self.sky.as_ref().map(SkyState::settings)
    }

    pub fn team_home_base_rule(&self) -> bool {
        self.team_home_base_rule
    }

    pub fn set_team_home_base_rule(&mut self, enabled: bool) {
        if self.team_home_base_rule == enabled {
            return;
        }
        self.team_home_base_rule = enabled;
        if enabled {
            let ids: Vec<_> = self.players.keys().copied().collect();
            for id in ids {
                self.sync_team_home_base_for(id);
            }
        }
    }
}
