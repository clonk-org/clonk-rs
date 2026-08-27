//! The container-backed menus: construction, base buy and sell, activate
//! and container contents.

use super::*;

impl Engine {
    /// C4Object::ActivateMenu(C4MN_Construction): install the classic
    /// structure-knowledge menu directly on the owning crew
    /// (C4Object.cpp:1982-2005).
    pub(crate) fn open_construction_menu(&mut self, crew_index: usize) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;

        let crew_id = self.objects[crew_index].id;
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        let owner = self.objects[crew_index].state.owner;
        let Some(player) = self.players.get(&owner) else {
            return Ok(());
        };
        let player_name = player.name().to_string();
        let knowledge = player
            .knowledge_entries()
            .iter()
            .map(|(definition_id, _)| definition_id.clone())
            .collect::<Vec<_>>();
        let symbol_id = if self.definitions.contains_key("CXCN") {
            "CXCN".to_string()
        } else if self.definitions.contains_key("WKS1") {
            "WKS1".to_string()
        } else {
            String::new()
        };
        // C4Object::ActivateMenu initializes the shell before AddRefSym
        // resolves each row's potentially callback-driven components.
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: format!("Player {player_name}|has no construction plans."),
            symbol_id,
            title_symbol: crate::ObjectMenuSymbol::default(),
            identification: Value::Int(1),
            style: 0,
            equal_item_height: false,
            permanent: false,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::Components,
            extra_data: 0,
            internal_refill_token: 0,
            selection: -1,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: None,
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items: Vec::new(),
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });

        for definition_id in knowledge {
            let Some((name, description)) = self
                .definitions
                .get(&definition_id)
                .filter(|definition| definition.category() & crate::CATEGORY_STRUCTURE != 0)
                .map(|definition| {
                    (
                        definition.name().to_string(),
                        definition.description().unwrap_or_default().to_string(),
                    )
                })
            else {
                continue;
            };
            let components = self
                .build_required_components(&definition_id, crew_id)?
                .into_iter()
                .map(|component| crate::ObjectMenuComponent {
                    definition_id: component.id,
                    count: component.count,
                })
                .collect();
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(1) && menu.command_object == Some(crew_id)
            }) else {
                return Ok(());
            };
            let select = menu.selection == -1;
            menu.items.push(crate::ObjectMenuItem {
                caption: format!("Construction: {name}"),
                info_caption: crate::normalize_menu_info_caption(description),
                command: format!("SetCommand(this, \"Construct\",,0,0,,{definition_id})"),
                command2: String::new(),
                count: C4MN_ITEM_NO_COUNT,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: None,
                components,
                selectable: true,
                value: None,
                text_display_progress: -1,
            });
            if select {
                menu.selection = 0;
            }
        }
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Buy) plus the immediate
    /// C4ObjectMenu::SetRefillObject/Refill pass (C4Object.cpp:1919-1930;
    /// C4ObjectMenu.cpp:207-237).
    pub(crate) fn open_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_buy_menu(crew_index, base_index, false)
    }

    pub(in crate::direct_com) fn refill_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_buy_menu(crew_index, base_index, true)
    }

    pub(in crate::direct_com) fn build_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        continue_existing: bool,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let base_id = self.objects[base_index].id;
        let previous_selection = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| {
                menu.identification == Value::Int(4)
                    && (!continue_existing || menu.refill_object == Some(base_id))
            })
            .map(|menu| menu.selection);
        let base_player = self.objects[base_index].state.base;
        let base_owner = self.objects[base_index].state.owner;
        let material = self
            .players
            .get(&base_player)
            .map(|player| {
                player
                    .home_base_material_entries()
                    .iter()
                    .map(|(definition_id, count)| (definition_id.clone(), *count))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut items = Vec::new();
        for (definition_id, count) in material {
            let Some((definition_name, definition_description)) =
                self.definitions.get(&definition_id).map(|definition| {
                    (
                        definition.name().to_string(),
                        definition.description().unwrap_or_default().to_string(),
                    )
                })
            else {
                continue;
            };
            let command = format!(
                "AppendCommand(this,\"Buy\",Object({}),1,0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                definition_id
            );
            let command2 = format!(
                "AppendCommand(this,\"Buy\",Object({}),{},0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                count,
                definition_id
            );
            let value = self.definition_value_in_container_for_menu(
                crew_id,
                &definition_id,
                base_id,
                base_player,
            )?;
            items.push(crate::ObjectMenuItem {
                caption: format!("Buy {definition_name}"),
                info_caption: crate::normalize_menu_info_caption(&definition_description),
                command,
                command2,
                count,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: None,
                components: Vec::new(),
                selectable: true,
                value: Some(value),
                text_display_progress: -1,
            });
        }
        // C4ObjectMenu rebuilds Buy rows with ClearItems(false), preserving
        // the numeric slot. C4Menu::AdjustSelection keeps it when valid and
        // otherwise walks backward to the final selectable row
        // (C4ObjectMenu.cpp:207-237; C4Menu.cpp:947-988,1014-1038).
        let selection = if items.is_empty() {
            -1
        } else {
            let last = i32::try_from(items.len() - 1).unwrap_or(i32::MAX);
            previous_selection.unwrap_or(0).clamp(0, last)
        };

        if continue_existing {
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(4) && menu.refill_object == Some(base_id)
            }) {
                menu.items = items;
                menu.selection = selection;
            }
            return Ok(());
        }

        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: "There is nothing to buy.".to_string(),
            symbol_id: String::new(),
            title_symbol: crate::ObjectMenuSymbol::Buy { owner: base_owner },
            identification: Value::Int(4),
            style: 0,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::Value,
            extra_data: 0,
            internal_refill_token: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: Some(base_id),
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
        Ok(())
    }

    /// The row partition produced by `C4ObjectListIterator::GetNext` for an
    /// object menu: category-ineligible entries are invisible, same-ID chunks
    /// retain list order, and only `CanConcatPictureWith`-equal objects share
    /// a row (C4ObjectList.cpp:849-903).
    pub(in crate::direct_com) fn object_menu_picture_groups(
        &mut self,
        contents: &[ObjectId],
        category_mask: i32,
    ) -> Vec<(ObjectId, i32)> {
        internal_object_menu_picture_groups(
            &EngineInternalObjectMenuSource(self),
            contents,
            category_mask,
        )
        .into_iter()
        .map(|group| (group.representative, group.count))
        .collect()
    }

    /// The native C4ObjectMenu refill owns each internal row's picture at
    /// construction time. Keep the effective object graphics as immutable
    /// presentation inputs so a later tick cannot blank the symbol
    /// (`C4ObjectMenu.cpp:194-199,311-313,350-372`).
    pub(in crate::direct_com) fn native_object_menu_picture_snapshot(
        &self,
        object_id: ObjectId,
    ) -> Option<crate::ObjectMenuPictureSnapshot> {
        let index = self.find_object_index(object_id)?;
        let object = &self.objects[index];
        Some(crate::ObjectMenuPictureSnapshot {
            definition_id: object.definition_id.clone(),
            symbol_size: 35,
            base_graphics: object.state.base_graphics.clone(),
            graphics_overlays: object.state.graphics_overlays.clone(),
            blit_mode: object.state.blit_mode,
            color: object.state.color,
            color_modulation: object.state.color_modulation,
            picture_rect: object.state.picture_rect,
            rank: None,
        })
    }

    /// `C4ObjectList::ObjectCount(id)` counts every live same-ID content,
    /// independently of the category/picture group used for the visible row
    /// (C4ObjectList.cpp:320-329; C4ObjectMenu.cpp:266-267,317-319).
    pub(in crate::direct_com) fn live_contents_definition_count(
        &self,
        contents: &[ObjectId],
        definition_id: &str,
    ) -> i32 {
        let count = contents
            .iter()
            .filter_map(|candidate| self.find_object_index(*candidate))
            .filter(|&candidate| {
                let candidate = &self.objects[candidate];
                candidate.has_nonzero_status() && candidate.definition_id == definition_id
            })
            .count();
        i32::try_from(count).unwrap_or(i32::MAX)
    }

    pub(in crate::direct_com) fn live_contents_count(&self, contents: &[ObjectId]) -> i32 {
        let count = contents
            .iter()
            .filter_map(|candidate| self.find_object_index(*candidate))
            .filter(|&candidate| self.objects[candidate].has_nonzero_status())
            .count();
        i32::try_from(count).unwrap_or(i32::MAX)
    }

    /// `C4Object::GetValue(pInBase, iForPlr)` for the value cached on an
    /// Activate/Sell-menu row. Running the public host expression preserves
    /// CalcValue/CalcDefValue, construction scaling, CalcSellValue, side
    /// effects, and fail-safe script errors (C4ObjectMenu.cpp:190-201,
    /// 268-271).
    pub(in crate::direct_com) fn object_value_in_container_for_menu(
        &mut self,
        command_object: ObjectId,
        object_id: ObjectId,
        container_id: ObjectId,
        for_player: i32,
    ) -> Result<i32, EngineError> {
        let expression = format!(
            "GetValue(Object({}),0,Object({}),{})",
            object_id.as_u64(),
            container_id.as_u64(),
            for_player
        );
        let Some(command_index) = self.find_object_index(command_object) else {
            return Ok(0);
        };
        Ok(crate::tolerate_script_error(self.direct_exec_on_object(
            command_index,
            &expression,
            "ObjectMenuValue",
        ))?
        .and_then(|value| value.as_c4_int())
        .unwrap_or(0))
    }

    /// `C4Def::GetValue(pInBase, iForPlr)` for a Buy-menu row
    /// (C4ObjectMenu.cpp:230-233). The definition and base hooks execute on
    /// every refill before the row is appended.
    pub(in crate::direct_com) fn definition_value_in_container_for_menu(
        &mut self,
        command_object: ObjectId,
        definition_id: &str,
        container_id: ObjectId,
        for_player: i32,
    ) -> Result<i32, EngineError> {
        let expression = format!(
            "GetValue(0,{},Object({}),{})",
            definition_id,
            container_id.as_u64(),
            for_player
        );
        let Some(command_index) = self.find_object_index(command_object) else {
            return Ok(0);
        };
        Ok(crate::tolerate_script_error(self.direct_exec_on_object(
            command_index,
            &expression,
            "BuyMenuValue",
        ))?
        .and_then(|value| value.as_c4_int())
        .unwrap_or(0))
    }

    /// `ClearItems(false)` keeps the numeric slot. `checkIDSelection` first
    /// accepts that slot when its C4ID survived, otherwise finds the first row
    /// carrying the old C4ID; `AdjustSelection` supplies the numeric fallback
    /// (C4ObjectMenu.cpp:147-164; C4Menu.cpp:975-1017).
    pub(in crate::direct_com) fn refilled_object_menu_selection(
        items: &[crate::ObjectMenuItem],
        previous_selection: Option<i32>,
        selected_definition: Option<&str>,
    ) -> i32 {
        if items.is_empty() {
            return -1;
        }
        if let (Some(previous), Some(selected)) = (previous_selection, selected_definition) {
            if usize::try_from(previous)
                .ok()
                .and_then(|selection| items.get(selection))
                .is_some_and(|item| item.item_id == selected)
            {
                return previous;
            }
        }
        if let Some(selection) = selected_definition
            .and_then(|selected| items.iter().position(|item| item.item_id == selected))
            .and_then(|selection| i32::try_from(selection).ok())
        {
            return selection;
        }
        let last = i32::try_from(items.len() - 1).unwrap_or(i32::MAX);
        previous_selection.unwrap_or(0).clamp(0, last)
    }

    /// C4Object::ActivateMenu(C4MN_Sell) plus C4ObjectMenu's immediate
    /// refill over the base's stContents list (C4Object.cpp:1932-1943;
    /// C4ObjectMenu.cpp:238-277).
    pub(crate) fn open_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_sell_menu(crew_index, base_index, false)
    }

    pub(in crate::direct_com) fn refill_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        self.build_base_sell_menu(crew_index, base_index, true)
    }

    pub(in crate::direct_com) fn build_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        continue_existing: bool,
    ) -> Result<(), EngineError> {
        const CATEGORY_TRADE_LIVING: i32 = 1 << 16;
        let crew_id = self.objects[crew_index].id;
        let base_id = self.objects[base_index].id;
        let (previous_selection, selected_definition) = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| {
                menu.identification == Value::Int(5)
                    && (!continue_existing || menu.refill_object == Some(base_id))
            })
            .map(|menu| {
                let selected_definition = usize::try_from(menu.selection)
                    .ok()
                    .and_then(|selection| menu.items.get(selection))
                    .map(|item| item.item_id.clone());
                (Some(menu.selection), selected_definition)
            })
            .unwrap_or((None, None));
        let base_owner = self.objects[base_index].state.owner;
        let base_definition = self.objects[base_index].definition_id.clone();
        let contents = self.objects[base_index].state.contents.clone();
        let sell_category = crate::CATEGORY_STATIC_BACK
            | crate::CATEGORY_STRUCTURE
            | crate::CATEGORY_VEHICLE
            | crate::CATEGORY_OBJECT
            | CATEGORY_TRADE_LIVING;
        let mut items = Vec::new();

        for (item_id, count) in self.object_menu_picture_groups(&contents, sell_category) {
            let Some(item_index) = self.find_object_index(item_id) else {
                continue;
            };
            let definition_id = self.objects[item_index].definition_id.clone();
            let Some((definition_name, definition_description, no_sell)) =
                self.definitions.get(&definition_id).map(|definition| {
                    (
                        definition.name().to_string(),
                        definition.description().unwrap_or_default().to_string(),
                        definition.no_sell(),
                    )
                })
            else {
                continue;
            };
            if no_sell != 0 {
                continue;
            }
            let all_count = self.live_contents_definition_count(&contents, &definition_id);
            let command = format!(
                "AppendCommand(this,\"Sell\",Object({}),1,0,Object({}),0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                item_id.as_u64(),
                definition_id
            );
            let command2 = format!(
                "AppendCommand(this,\"Sell\",Object({}),{},0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                all_count,
                definition_id
            );
            let for_player = self
                .find_object_index(crew_id)
                .map(|index| self.objects[index].state.owner)
                .unwrap_or(-1);
            // C4ObjectMenu.cpp:258-263 renders Picture2Facet before
            // GetValue; capture it before the callback below.
            let picture_snapshot = self.native_object_menu_picture_snapshot(item_id);
            let value =
                self.object_value_in_container_for_menu(crew_id, item_id, base_id, for_player)?;
            items.push(crate::ObjectMenuItem {
                caption: format!("Sell {definition_name}"),
                info_caption: crate::normalize_menu_info_caption(&definition_description),
                command,
                command2,
                count,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot,
                picture_object: Some(item_id),
                components: Vec::new(),
                selectable: true,
                value: Some(value),
                text_display_progress: -1,
            });
        }

        // ClearItems(false) leaves C++'s numeric selection in place while
        // checkIDSelection restores the selected C4ID after refill. If that
        // C4ID vanished, AdjustSelection keeps the old slot when valid and
        // otherwise walks backward to the final row (C4ObjectMenu.cpp:
        // 147-164,238-275; C4Menu.cpp:975-1017).
        let selection = Self::refilled_object_menu_selection(
            &items,
            previous_selection,
            selected_definition.as_deref(),
        );
        let base_name = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| base_definition.clone());
        if continue_existing {
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(5) && menu.refill_object == Some(base_id)
            }) {
                menu.items = items;
                menu.selection = selection;
            }
            return Ok(());
        }
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: format!("{} is empty.", base_name),
            symbol_id: String::new(),
            title_symbol: crate::ObjectMenuSymbol::Sell { owner: base_owner },
            identification: Value::Int(5),
            style: 0,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::Value,
            extra_data: 0,
            internal_refill_token: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: Some(base_id),
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Activate) plus its immediate contents
    /// refill (C4Object.cpp:1884-1918; C4ObjectMenu.cpp:170-205).
    pub(crate) fn open_activate_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
    ) -> Result<(), EngineError> {
        self.set_activate_menu(crew_index, container_index, true, None)
    }

    pub(crate) fn initialize_activate_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
    ) -> Result<(), EngineError> {
        self.set_activate_menu(crew_index, container_index, false, None)
    }

    pub(crate) fn set_activate_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        continue_existing: bool,
        reused_menu_identity: Option<u64>,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let container_id = self.objects[container_index].id;
        let menu = build_activate_menu_state(
            &mut EngineInternalObjectMenuSource(self),
            crew_id,
            container_id,
            continue_existing,
            reused_menu_identity,
        )?;
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        if self.find_object_index(container_id).is_none() {
            return Ok(());
        }
        self.objects[crew_index].state.menu = menu;
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Get/C4MN_Contents) plus the immediate
    /// contents refill (C4Object.cpp:1945-1959; C4ObjectMenu.cpp:279-326).
    pub(crate) fn open_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
    ) -> Result<(), EngineError> {
        self.set_container_contents_menu(crew_index, container_index, identification, true, None)
    }

    pub(crate) fn initialize_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
    ) -> Result<(), EngineError> {
        self.set_container_contents_menu(crew_index, container_index, identification, false, None)
    }

    pub(crate) fn set_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
        continue_existing: bool,
        reused_menu_identity: Option<u64>,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let container_id = self.objects[container_index].id;
        let menu = build_container_contents_menu_state(
            &mut EngineInternalObjectMenuSource(self),
            crew_id,
            container_id,
            identification,
            continue_existing,
            reused_menu_identity,
        )?;
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = menu;
        Ok(())
    }
}
