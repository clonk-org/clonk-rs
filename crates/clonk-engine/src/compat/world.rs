use super::*;

#[derive(Debug, Clone)]
pub(crate) struct HostWorldObject {
    pub id: ObjectId,
    pub(crate) definition_id: DefinitionId,
    /// Runtime-only C4Object::Unsorted flag. It is intentionally absent
    /// from saves, but live Enter/Add calls must still see it.
    pub(crate) unsorted: bool,
    pub(crate) status: ObjectStatus,
    pub(crate) alive: bool,
    /// C4Object::InLiquid (the cached flag FnInLiquid reads).
    in_liquid: bool,
    pub action_name: String,
    pub action_index: Option<u32>,
    /// Action.Dir as a script value (0=Left, 1=Right) — read by foreign
    /// GetDir (FnGetDir reads any pObj).
    pub direction: i32,
    pub action_target: Option<ObjectId>,
    pub action_target2: Option<ObjectId>,
    pub action_procedure: Option<String>,
    pub owner: i32,
    /// Same-call scope overlay for C4Object::Controller.
    pub(crate) controller: Option<i32>,
    /// C4Object::Select at call entry, overlaid from active scopes.
    pub selected: bool,
    /// C4Object::CrewDisabled at call entry, overlaid from active scopes.
    pub crew_disabled: bool,
    pub category: i32,
    pub collectible: bool,
    /// Whether SetOCF would expose OCF_Collection with NoCollectDelay
    /// temporarily cleared. FnCollect performs exactly that temporary
    /// recompute before its gate (C4Script.cpp:397-406).
    collection_available_ignoring_delay: bool,
    /// Whether the definition/action/construction gates allow collection
    /// with an empty inventory and no delay. Live Enter/Exit combines this
    /// with the current limit and delay before callback-visible SetOCF.
    collection_enabled: bool,
    pub(crate) no_collect_delay: i32,
    /// Raw signed DefCore CollectionLimit; zero alone means unlimited.
    collection_limit: i32,
    pub energy: i32,
    /// C4Object::NeedEnergy at call entry, overlaid from active scopes.
    pub need_energy: bool,
    pub construction: i32,
    /// Current C4Shape::ContactDensity. Dynamic SetContactDensity is not yet
    /// modeled, so engine contexts seed this from the parsed definition.
    contact_density: i32,
    #[allow(dead_code)]
    pub damage: i32,
    pub ocf: u32,
    pub(crate) move_to_range: i32,
    pub(crate) pathfinder: i32,
    pub(crate) no_transfer_zones: i32,
    pub(crate) no_push_enter: i32,
    pub position: Vector2,
    pub(crate) fixed_position: FixedVec2,
    #[allow(dead_code)]
    pub velocity: Vector2,
    pub(crate) fixed_velocity: FixedVec2,
    pub(crate) motion_x: i32,
    pub(crate) motion_y: i32,
    pub(crate) last_attach_movement_frame: i32,
    pub(crate) compiler_cache: crate::ObjectCompilerCache,
    /// Raw 16.16 fixed-point rotation accumulator (`C4Object::fix_r`).
    pub(crate) fixed_rotation: C4Fixed,
    /// Raw 16.16 fixed-point angular velocity (`C4Object::rdir`).
    pub(crate) rotation_velocity: C4Fixed,
    pub rotation: i32,
    pub vertices: Vec<ObjectVertex>,
    pub(crate) own_vertices: bool,
    #[allow(dead_code)]
    pub action_data: i32,
    pub action_ticks: i32,
    pub action_phase: i32,
    pub(crate) container: Option<ObjectId>,
    pub(crate) contents: Vec<ObjectId>,
    #[allow(dead_code)]
    pub draw_transform: Option<DrawTransform>,
    /// FnGetCommand views of the object's command stack, top first
    /// (C4Script.cpp:918-945). A frame-start snapshot — mid-frame command
    /// changes are not re-read (C++ reads live).
    pub commands: Vec<CommandView>,
    /// Full mutable command state for synchronous ExecuteCommand calls.
    pub command_stack: CommandStackSnapshot,
    /// Full object-state snapshot for nested script calls (Find_Func,
    /// GameCall): lets host functions build a complete object scope for
    /// another object mid-VM-call. `None` in legacy fixture contexts.
    pub(crate) state: Option<Rc<ObjectState>>,
    /// C4Object::MaterialContents at callback entry. This runtime-only
    /// accumulation is not part of ObjectState, but DigFree must update and
    /// inspect it synchronously before the engine outcome folds.
    pub(crate) material_contents: Vec<i32>,
    /// C4Object::LastEnergyLossCausePlayer (kill trace) — carried beside
    /// the state snapshot because it lives on the engine object wrapper.
    pub last_energy_loss_cause: i32,
}

/// The DefCore fields the blast/fire chain consults: the host-path
/// incinerate (C4Object::Blast, C4Object.cpp:1420-1423 + the fxFireStart
/// core, C4Effect.cpp:560-641) and the GetDefCoreVal reflection entries
/// System.c4g's DoExplosion/BlastObjectsShockwaveCheck read (GetXVal.c).
#[derive(Debug, Clone, Default)]
pub(crate) struct DefinitionFireMetadata {
    /// Complete compiler-shaped DefCore/Physical reflection surface. It
    /// lives in this already-aggregated definition payload so legacy test
    /// fixtures can keep using `DefinitionMetadata` struct literals.
    pub def_core_values: DefCoreValueStore,
    /// C4Shape::FireTop, reflected through GetDefCoreVal.
    pub fire_top: i32,
    /// DefCore LiftTop, reflected for System.c4g's GetDefLiftTop helper.
    pub lift_top: i32,
    /// BlastIncinerate threshold (0 = off).
    pub blast_incinerate: i32,
    /// BurnTurnTo changedef target (C4Effect.cpp:579-585).
    pub burn_turn_to: Option<String>,
    /// IncompleteActivity/NoBurnDecay gate the burning contents ejection
    /// (C4Effect.cpp:586-594).
    pub incomplete_activity: bool,
    pub no_burn_decay: bool,
    /// NoBurnDamage skips the Tick10 fire damage (C4Object.cpp:780).
    pub no_burn_damage: bool,
    /// ContactIncinerate 1-in-N contact-fire chance (0 = not inflammable).
    pub contact_incinerate: i32,
    /// ContainBlast=1 shields contents from explosions (C4Effect.cpp:884).
    pub contain_blast: i32,
    /// ClosedContainer mode (0 open, 1 closed/no view, 2 closed/view).
    pub closed_container: i32,
    /// HorizontalFix (C4Def::NoHorizontalMove): no shockwave flings.
    pub no_horizontal_move: i32,
    /// Grab (0 none, 1 grab+push, 2 grab-only) — the shockwave check's
    /// vehicle/FLOAT exemption reads it (Explode.c BlastObjectsShockwaveCheck).
    pub grab: i32,
    /// DefCore NoPushEnter; any nonzero value rejects C4Command::Enter for
    /// objects of this definition.
    pub no_push_enter: i32,
    /// DefCore NoGet; contained objects of this definition cannot be taken
    /// by C4Command::Get.
    pub no_get: bool,
    /// DefCore Oversize removes DoCon's upper FullCon clamp.
    pub oversize: bool,
    /// Positive Collection rect enables OCF_Collection when construction,
    /// capacity, action and delay gates pass.
    pub collection_rect: Option<DefinitionRect>,
    /// DefCore `Fragile`; Put does not throw these items into collection
    /// areas.
    pub fragile: bool,
    /// Raw DefCore `Projectile`; nonzero contents are preferred by Attack.
    pub projectile: i32,
    /// Positive Entrance rect enables OCF_Entrance at FullCon.
    pub entrance_rect: Option<DefinitionRect>,
    /// DefCore RotatedEntrance rotation cutoff.
    pub rotated_entrance: i32,
    /// DefCore AttractLightning is gated by FullCon.
    pub attract_lightning: bool,
    /// DefCore NoFight suppresses OCF_FightReady.
    pub no_fight: bool,
}

/// Primitive values emitted by the C++ `C4ValueGetCompiler`. DefCore
/// reflection is deliberately compiler-shaped: compound entries are exposed
/// one primitive at a time through `entry_nr`, never as script arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DefCorePrimitive {
    Int(i32),
    Bool(bool),
    String(String),
    C4Id(String),
    Nil,
}

impl DefCorePrimitive {
    pub(crate) fn into_value(self) -> Value {
        match self {
            Self::Int(value) => Value::Int(value),
            Self::Bool(value) => Value::Bool(value),
            Self::String(value) => Value::String(value.into()),
            Self::C4Id(value) => Value::C4Id(value),
            Self::Nil => Value::Nil,
        }
    }
}

/// Fully defaulted `C4DefCore::CompileFunc` + `C4Shape::CompileFunc(false)`
/// view, followed by the sibling `Physical` section. Keeping the sections
/// separate preserves both exact section matching and the C++ no-section
/// search where duplicate `Float`/`Scale` names are indexed in traversal
/// order (DefCore first, Physical second).
#[derive(Debug, Clone, Default)]
pub(crate) struct DefCoreValueStore {
    pub(crate) def_core: HashMap<&'static str, Vec<DefCorePrimitive>>,
    pub(crate) physical: HashMap<&'static str, Vec<DefCorePrimitive>>,
}

impl DefCoreValueStore {
    fn int(value: i32) -> Vec<DefCorePrimitive> {
        vec![DefCorePrimitive::Int(value)]
    }

    fn ints(values: impl IntoIterator<Item = i32>) -> Vec<DefCorePrimitive> {
        values.into_iter().map(DefCorePrimitive::Int).collect()
    }

    fn trimmed_ints(values: impl IntoIterator<Item = i32>) -> Vec<DefCorePrimitive> {
        let mut values = values.into_iter().collect::<Vec<_>>();
        while values.last() == Some(&0) {
            values.pop();
        }
        Self::ints(values)
    }

    pub(crate) fn c4id(value: Option<&str>) -> Vec<DefCorePrimitive> {
        vec![match value.map(clonk_script::c4_id_raw) {
            Some(raw) if raw != 0 => DefCorePrimitive::C4Id(clonk_script::c4_id_from_raw(raw)),
            _ => DefCorePrimitive::Nil,
        }]
    }

    fn rect(rect: Option<DefinitionRect>) -> Vec<DefCorePrimitive> {
        let rect = rect.unwrap_or_default();
        Self::ints([rect.x, rect.y, rect.width, rect.height])
    }

    fn target_rect(rect: Option<crate::DefinitionTargetRect>) -> Vec<DefCorePrimitive> {
        let rect = rect.unwrap_or_else(|| crate::DefinitionTargetRect::new(0, 0, 0, 0, 0, 0));
        Self::ints([
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.target_x,
            rect.target_y,
        ])
    }

    pub(crate) fn from_definition(definition: &crate::Definition) -> Self {
        let mut def_core = HashMap::new();
        let mut physical = HashMap::new();
        let shape = definition.shape.unwrap_or_default();
        let picture = definition
            .picture
            .map(|picture| DefinitionRect::new(picture.x, picture.y, picture.width, picture.height))
            .filter(|picture| picture.width != 0 && picture.height != 0)
            .unwrap_or(shape);
        let mut category = definition.category;
        if definition.crew_member_value != 0 {
            category |= 1 << 18;
        }
        if category & CATEGORY_SORT_LIMIT == 0 {
            category = (category & !CATEGORY_SORT_LIMIT) | 1;
        }
        let reflected_int = |entry: &str, fallback: i32| {
            Self::int(
                definition
                    .def_core_reflected_ints
                    .get(entry)
                    .copied()
                    .unwrap_or(fallback),
            )
        };

        def_core.insert("id", Self::c4id(Some(definition.id.as_str())));
        def_core.insert("Version", Self::trimmed_ints(definition.version));
        def_core.insert(
            "Name",
            vec![DefCorePrimitive::String(definition.name.clone())],
        );
        def_core.insert(
            "RequireDef",
            definition
                .require_defs
                .iter()
                .map(|id| DefCorePrimitive::C4Id(id.clone()))
                .collect(),
        );
        def_core.insert("Category", Self::int(category));
        def_core.insert("MaxUserSelect", Self::int(definition.max_user_select));
        def_core.insert("Timer", Self::int(definition.timer));
        def_core.insert(
            "TimerCall",
            vec![DefCorePrimitive::String(
                definition.timer_call.clone().unwrap_or_default(),
            )],
        );
        def_core.insert(
            "ContactCalls",
            reflected_int("ContactCalls", i32::from(definition.contact_function_calls)),
        );
        def_core.insert("Width", Self::int(shape.width));
        def_core.insert("Height", Self::int(shape.height));
        def_core.insert("Offset", Self::trimmed_ints([shape.x, shape.y]));
        def_core.insert(
            "Vertices",
            reflected_int(
                "Vertices",
                definition.shape_vertex_slots.active_count() as i32,
            ),
        );
        def_core.insert(
            "VertexX",
            Self::trimmed_ints(
                definition
                    .shape_vertex_slots
                    .slots
                    .iter()
                    .map(|vertex| vertex.x),
            ),
        );
        def_core.insert(
            "VertexY",
            Self::trimmed_ints(
                definition
                    .shape_vertex_slots
                    .slots
                    .iter()
                    .map(|vertex| vertex.y),
            ),
        );
        def_core.insert(
            "VertexCNAT",
            Self::trimmed_ints(
                definition
                    .shape_vertex_slots
                    .slots
                    .iter()
                    .map(|vertex| vertex.cnat as i32),
            ),
        );
        def_core.insert(
            "VertexFriction",
            Self::trimmed_ints(
                definition
                    .shape_vertex_slots
                    .slots
                    .iter()
                    .map(|vertex| vertex.friction),
            ),
        );
        def_core.insert("ContactDensity", Self::int(definition.contact_density));
        def_core.insert("FireTop", Self::int(definition.fire_top));
        def_core.insert("Value", reflected_int("Value", definition.value));
        def_core.insert("Mass", Self::int(definition.mass));
        def_core.insert(
            "Components",
            definition
                .components
                .iter()
                .flat_map(|component| {
                    [
                        DefCorePrimitive::C4Id(component.id.as_str().to_string()),
                        DefCorePrimitive::Int(component.count),
                    ]
                })
                .collect(),
        );
        def_core.insert(
            "SolidMask",
            Self::target_rect(definition.def_core_solid_mask),
        );
        def_core.insert("TopFace", Self::target_rect(definition.def_core_top_face));
        def_core.insert("Picture", Self::rect(Some(picture)));
        def_core.insert("PictureFE", Vec::new());
        def_core.insert("Entrance", Self::rect(definition.entrance_rect));
        def_core.insert(
            "Collection",
            Self::rect(definition.def_core_collection_rect),
        );
        def_core.insert(
            "CollectionLimit",
            reflected_int("CollectionLimit", definition.collection_limit),
        );
        def_core.insert("Placement", Self::int(definition.placement));
        def_core.insert(
            "Exclusive",
            reflected_int("Exclusive", i32::from(definition.exclusive)),
        );
        def_core.insert(
            "ContactIncinerate",
            reflected_int("ContactIncinerate", definition.contact_incinerate),
        );
        def_core.insert("BlastIncinerate", Self::int(definition.blast_incinerate));
        def_core.insert("BurnTo", Self::c4id(definition.burn_turn_to.as_deref()));
        def_core.insert(
            "Base",
            reflected_int("Base", i32::from(definition.can_be_base)),
        );
        def_core.insert("Line", Self::int(definition.line));
        def_core.insert("LineConnect", Self::int(definition.line_connect as i32));
        def_core.insert("LineIntersect", Self::int(definition.line_intersect));
        def_core.insert("Prey", reflected_int("Prey", i32::from(definition.prey)));
        def_core.insert(
            "Edible",
            reflected_int("Edible", i32::from(definition.edible)),
        );
        def_core.insert("CrewMember", Self::int(definition.crew_member_value));
        def_core.insert("NoStandardCrew", Self::int(definition.no_standard_crew));
        def_core.insert("Growth", Self::int(definition.growth));
        def_core.insert(
            "Rebuy",
            reflected_int("Rebuy", i32::from(definition.rebuyable)),
        );
        def_core.insert(
            "Construction",
            reflected_int("Construction", i32::from(definition.constructable)),
        );
        def_core.insert(
            "ConstructTo",
            Self::c4id(definition.build_turn_to.as_deref()),
        );
        def_core.insert("Grab", reflected_int("Grab", definition.grab));
        def_core.insert("GrabPutGet", Self::int(definition.grab_put_get));
        def_core.insert(
            "Collectible",
            reflected_int("Collectible", i32::from(definition.collectible)),
        );
        def_core.insert("Rotate", reflected_int("Rotate", definition.rotateable));
        def_core.insert("RotatedEntrance", Self::int(definition.rotated_entrance));
        def_core.insert(
            "Chop",
            reflected_int("Chop", i32::from(definition.chopable)),
        );
        def_core.insert("Float", Self::int(definition.float_line));
        def_core.insert("ContainBlast", Self::int(definition.contain_blast));
        def_core.insert(
            "ColorByOwner",
            reflected_int("ColorByOwner", i32::from(definition.color_by_owner)),
        );
        def_core.insert(
            "ColorByMaterial",
            vec![DefCorePrimitive::String(
                definition.color_by_material.clone(),
            )],
        );
        def_core.insert("HorizontalFix", Self::int(definition.no_horizontal_move));
        def_core.insert(
            "BorderBound",
            reflected_int("BorderBound", definition.border_bound),
        );
        def_core.insert("LiftTop", Self::int(definition.lift_top));
        def_core.insert(
            "UprightAttach",
            reflected_int("UprightAttach", definition.upright_attach),
        );
        def_core.insert(
            "StretchGrowth",
            reflected_int("StretchGrowth", i32::from(definition.stretch_growth)),
        );
        def_core.insert("Basement", reflected_int("Basement", definition.basement));
        def_core.insert(
            "NoBurnDecay",
            reflected_int("NoBurnDecay", i32::from(definition.no_burn_decay)),
        );
        def_core.insert(
            "IncompleteActivity",
            reflected_int(
                "IncompleteActivity",
                i32::from(definition.incomplete_activity),
            ),
        );
        def_core.insert(
            "AttractLightning",
            reflected_int("AttractLightning", i32::from(definition.attract_lightning)),
        );
        def_core.insert(
            "Oversize",
            reflected_int("Oversize", i32::from(definition.oversize)),
        );
        def_core.insert(
            "Fragile",
            reflected_int("Fragile", i32::from(definition.fragile)),
        );
        def_core.insert("Explosive", Self::int(definition.explosive));
        def_core.insert("Projectile", Self::int(definition.projectile));
        def_core.insert("NoPushEnter", Self::int(definition.no_push_enter));
        def_core.insert("DragImagePicture", Self::int(definition.drag_image_picture));
        def_core.insert("VehicleControl", Self::int(definition.vehicle_control));
        def_core.insert("Pathfinder", Self::int(definition.pathfinder));
        def_core.insert("MoveToRange", Self::int(definition.move_to_range));
        def_core.insert(
            "NoComponentMass",
            reflected_int("NoComponentMass", i32::from(definition.no_component_mass)),
        );
        def_core.insert(
            "NoStabilize",
            reflected_int("NoStabilize", i32::from(definition.no_stabilize)),
        );
        def_core.insert("ClosedContainer", Self::int(definition.closed_container));
        def_core.insert(
            "SilentCommands",
            reflected_int("SilentCommands", i32::from(definition.silent_commands)),
        );
        def_core.insert(
            "NoBurnDamage",
            reflected_int("NoBurnDamage", i32::from(definition.no_burn_damage)),
        );
        def_core.insert("TemporaryCrew", Self::int(definition.temporary_crew));
        def_core.insert("SmokeRate", Self::int(definition.smoke_rate));
        def_core.insert("BlitMode", Self::int(definition.blit_mode as i32));
        def_core.insert(
            "NoBreath",
            reflected_int("NoBreath", i32::from(definition.no_breath)),
        );
        def_core.insert(
            "ConSizeOff",
            reflected_int("ConSizeOff", definition.construction_offset),
        );
        def_core.insert("NoSell", Self::int(definition.no_sell));
        def_core.insert(
            "NoGet",
            reflected_int("NoGet", i32::from(definition.no_get)),
        );
        def_core.insert(
            "NoFight",
            reflected_int("NoFight", i32::from(definition.no_fight)),
        );
        def_core.insert(
            "RotatedSolidmasks",
            reflected_int(
                "RotatedSolidmasks",
                i32::from(definition.rotated_solid_masks),
            ),
        );
        def_core.insert("NoTransferZones", Self::int(definition.no_transfer_zones));
        def_core.insert(
            "AutoContextMenu",
            reflected_int("AutoContextMenu", i32::from(definition.auto_context_menu)),
        );
        def_core.insert("NeededGfxMode", Self::int(definition.needed_gfx_mode));
        def_core.insert(
            "AllowPictureStack",
            Self::int(definition.allow_picture_stack),
        );
        def_core.insert("HideHUDBars", Self::int(definition.hide_hud_bars));
        def_core.insert("HideHUDElements", Self::int(definition.hide_hud_elements));
        def_core.insert(
            "Scale",
            reflected_int(
                "Scale",
                (definition.graphics_scale * 100.0).round() as u32 as i32,
            ),
        );
        def_core.insert(
            "BaseAutoSell",
            vec![DefCorePrimitive::Bool(definition.base_auto_sell)],
        );

        let info = definition.physical;
        for (name, value) in [
            ("Energy", info.energy),
            ("Breath", info.breath),
            ("Walk", info.walk),
            ("Jump", info.jump),
            ("Scale", info.scale),
            ("Hangle", info.hangle),
            ("Dig", info.dig),
            ("Swim", info.swim),
            ("Throw", info.throw),
            ("Push", info.push),
            ("Fight", info.fight),
            ("Magic", info.magic),
            ("Float", info.float),
            ("CanScale", info.can_scale),
            ("CanHangle", info.can_hangle),
            ("CanDig", info.can_dig),
            ("CanConstruct", info.can_construct),
            ("CanChop", info.can_chop),
            ("CanFly", info.can_fly),
            ("CorrosionResist", info.corrosion_resist),
            ("BreatheWater", info.breathe_water),
        ] {
            physical.insert(name, Self::int(value));
        }

        Self { def_core, physical }
    }

    fn get(&self, entry: &str, section: Option<&str>, entry_nr: i32) -> Option<Value> {
        let mut index = usize::try_from(entry_nr).ok()?;
        let sections: &[&HashMap<&'static str, Vec<DefCorePrimitive>>] = match section {
            Some("DefCore") => &[&self.def_core],
            Some("Physical") => &[&self.physical],
            Some(_) => return None,
            None => &[&self.def_core, &self.physical],
        };
        for entries in sections {
            let Some(values) = entries.get(entry) else {
                continue;
            };
            if index < values.len() {
                return values.get(index).cloned().map(DefCorePrimitive::into_value);
            }
            index -= values.len();
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.def_core.is_empty() && self.physical.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DefinitionMetadata {
    /// DefCore `Name` (FnGetName's def form, C4Script.cpp:992-1005).
    pub name: String,
    /// Canonical names of successfully decoded `Portrait*.*` graphics.
    /// C4PortraitGraphics::Get compares these case-insensitively.
    pub portrait_names: Vec<String>,
    pub category: i32,
    /// DefCore `BorderBound`, consulted synchronously by Exit's BoundsCheck
    /// before the requested position is installed.
    pub border_bound: i32,
    /// DefCore `ContactCalls`; TargetBounds invokes Contact* only when this
    /// legacy switch is enabled on the definition live at the call site.
    pub contact_function_calls: bool,
    /// DefCore `BlitMode`, used by SetObjectBlitMode(0).
    pub blit_mode: u32,
    pub ocf_base: u32,
    pub crew_member: bool,
    /// Literal signed C4DefCore::CrewMember value. Gameplay uses the
    /// derived boolean above; FnCrewMember exposes this raw value.
    pub crew_member_value: i32,
    /// DefCore `SilentCommands`, read after a failed-command script callback
    /// from the actor's current definition.
    pub silent_commands: bool,
    /// DefCore `VehicleControl`: SetCommand's unconditional contained/pushed
    /// ControlCommand routing bits (C4Object.cpp:3957-3983).
    pub vehicle_control: i32,
    /// ActMap for building nested object scopes (Find_Func targets).
    pub action_library: SharedActionLibrary,
    /// AfterLink-pinned `C4DefScriptHost::SFn_ControlTransfer`. Keeping the
    /// cached target (including cached null) lets script ExecuteCommand use
    /// the same direct callback as ordinary engine command execution.
    pub control_transfer_callback: Option<ScriptCallbackTarget>,
    /// Presentation facets used by FrameDecoration::SetByDef.
    pub action_graphics: HashMap<String, crate::DefinitionActionGraphics>,
    #[allow(dead_code)]
    pub value: i32,
    /// DefCore AllowPictureStack APS_* exception bits, used by the live
    /// internal object-menu row grouping path.
    pub allow_picture_stack: i32,
    #[allow(dead_code)]
    pub mass: i32,
    /// DefCore NoComponentMass suppresses the contents contribution to the
    /// live cached C4Object::Mass.
    pub no_component_mass: bool,
    pub constructable: bool,
    pub shape: Option<DefinitionRect>,
    /// DefCore `Placement` (C4Def.cpp:312): PlaceVegetation dispatches
    /// surface/liquid placement from this value.
    pub placement: i32,
    /// DefCore `Growth` (C4Def.cpp:358): the optional random-growth gate
    /// in C4Game::PlaceVegetation.
    pub growth: i32,
    pub construction_offset: i32,
    #[allow(dead_code)]
    pub basement: i32,
    /// The `[Physical]` section (GetPhysical's def form, C4Script.cpp:652).
    pub physical: PhysicalInfo,
    /// DefCore `Components` in list order (C4IDList; GetComponent's
    /// count/index forms, C4Script.cpp:2685-2709).
    pub components: Vec<(String, i32)>,
    /// Raw DefCore `CollectionLimit` reflected by GetDefCoreVal. Zero is
    /// the C++ unlimited/default value, not a missing value.
    pub collection_limit: i32,
    /// DefCore `GrabPutGet` bitfield reflected by GetDefCoreVal
    /// (`C4D_GrabPut=1 | C4D_GrabGet=2`).
    pub grab_put_get: i32,
    /// DefCore `LineConnect` bits (C4D_Power_Consumer etc.;
    /// FnEnergyCheck, C4Script.cpp:1845-1856).
    pub line_connect: u32,
    /// ClonkNames newline count (C4ObjectInfoList::New's name draw range,
    /// C4InfoCore.cpp:411); None = use the game standard names.
    pub clonk_name_newlines: Option<i32>,
    /// DefCore StretchGrowth (the con-scaling mode; DoCon's bottom
    /// adjust shape math).
    pub stretch_growth: bool,
    /// DefCore Rotateable; C4Object::Init clears initial rotation/rdir
    /// when this is zero (C4Object.cpp:169-170).
    pub rotateable: i32,
    /// DefCore Line type (C4D_Line*; nonzero skips con-scaling and the
    /// DoCon bottom adjust — C4Object::UpdateShape's early return).
    pub line: i32,
    /// The definition's shape vertices (full-Con). Pending-spawn preview
    /// scopes seed from these so creation callbacks (AdjustSeatVertex,
    /// CHBM Connect) mutate the REAL vertex list, not an empty one.
    pub vertices: Vec<ObjectVertex>,
    /// Def->Shape.ContactDensity copied by fresh C4Object::Init. None is
    /// reserved for synthetic host fixtures and means C4M_Solid.
    pub contact_density: Option<i32>,
    /// Fire fields for the host-path incinerate (C4Object::Blast).
    pub fire: DefinitionFireMetadata,
}

impl DefinitionMetadata {
    pub(crate) fn contact_density(&self) -> i32 {
        self.contact_density.unwrap_or(crate::CONTACT_DENSITY_SOLID)
    }
}

/// The immutable graphics data needed to expose a freshly created object's
/// C4SolidMask before its Initialize callback returns. Real grid landscapes
/// cannot bake that pending object yet, so the script host samples this
/// transient descriptor exactly like C4SolidMaskBitmap::MaskPixel.
#[derive(Debug, Clone)]
pub(crate) struct HostSolidMaskImage {
    width: i32,
    height: i32,
    pixels: Arc<[u8]>,
}

impl HostSolidMaskImage {
    pub(crate) fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        Self {
            width: i32::try_from(width).unwrap_or(i32::MAX),
            height: i32::try_from(height).unwrap_or(i32::MAX),
            pixels,
        }
    }

    fn check_mask_rect(&self, mask: crate::DefinitionTargetRect) -> crate::DefinitionTargetRect {
        mask.checked_for_solid_mask_bitmap(self.width, self.height)
    }

    fn pixels_for_checked_mask(&self, mask: crate::DefinitionTargetRect) -> Option<Arc<Vec<u8>>> {
        crate::solid_mask_pixels_for_checked_bitmap(
            mask,
            self.width,
            self.height,
            self.pixels.as_ref(),
        )
    }

    /// Clamp once like CheckSolidMaskRect, then copy the effective bitmap
    /// pixels. Lifecycle callers that persist the checked object rectangle
    /// use `pixels_for_checked_mask` directly to avoid clamping it twice.
    pub(crate) fn mask_pixels(
        &self,
        mask: crate::DefinitionTargetRect,
    ) -> Option<(crate::DefinitionTargetRect, Arc<Vec<u8>>)> {
        let mask = self.check_mask_rect(mask);
        let pixels = self.pixels_for_checked_mask(mask)?;
        Some((mask, pixels))
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HostSolidMaskMetadata {
    pub(crate) shape: Option<DefinitionRect>,
    pub(crate) default_mask: Option<crate::DefinitionTargetRect>,
    pub(crate) rotated_solid_masks: bool,
    default_image: Option<HostSolidMaskImage>,
    named_images: HashMap<String, HostSolidMaskImage>,
}

impl HostSolidMaskMetadata {
    pub(crate) fn new(
        shape: Option<DefinitionRect>,
        default_mask: Option<crate::DefinitionTargetRect>,
        rotated_solid_masks: bool,
        default_image: Option<HostSolidMaskImage>,
        named_images: HashMap<String, HostSolidMaskImage>,
    ) -> Self {
        Self {
            shape,
            default_mask,
            rotated_solid_masks,
            default_image,
            named_images,
        }
    }

    pub(crate) fn check_mask_rect(
        &self,
        mask: crate::DefinitionTargetRect,
        name: Option<&str>,
    ) -> Option<crate::DefinitionTargetRect> {
        match name.filter(|name| !name.is_empty()) {
            Some(name) => self
                .named_images
                .get(&clonk_resources::material::c4_name_key(name))
                .map(|image| image.check_mask_rect(mask)),
            None => Some(
                self.default_image
                    .as_ref()
                    .map_or(mask, |image| image.check_mask_rect(mask)),
            ),
        }
    }

    pub(crate) fn pixels_for_checked_mask(
        &self,
        mask: crate::DefinitionTargetRect,
        name: Option<&str>,
    ) -> Option<Option<Arc<Vec<u8>>>> {
        if !mask.is_positive() {
            return None;
        }
        match name.filter(|name| !name.is_empty()) {
            Some(name) => self
                .named_images
                .get(&clonk_resources::material::c4_name_key(name))?
                .pixels_for_checked_mask(mask)
                .map(Some),
            None => match self.default_image.as_ref() {
                Some(image) => image.pixels_for_checked_mask(mask).map(Some),
                None => Some(None),
            },
        }
    }
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub enum PlayerCommand {
    /// `FnSetName`'s definition branch writes the mutable `C4Def::Name`.
    /// This is engine-global; the player-command outcome is the existing
    /// ordered synchronous script-to-engine transport.
    SetDefinitionName {
        definition_id: DefinitionId,
        name: String,
    },
    /// Persist one `C4ObjectInfo::Name` write. `link` identifies the owning
    /// roster entry when the live info came from a player's CrewInfoList;
    /// link-less fixture/import infos still update the live object payload.
    SetCrewInfoName {
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        name: String,
    },
    /// FnSetPortrait mutates the C4ObjectInfo itself. Keep both the live
    /// projection and its pointer-equivalent roster entry in lockstep.
    SetCrewInfoPortrait {
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        portraits: CrewPortraitState,
    },
    /// `FnSetCrewExtraData`: an incremental named-slot write on the exact
    /// C4ObjectInfo pointer and, when linked, its persistent roster entry.
    SetCrewExtraData {
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        name: String,
        value: Value,
    },
    /// Persist one `C4ObjectInfo::Physical` training write through the exact
    /// owning roster pointer. The object-scope copy is carried separately by
    /// `ObjectUpdate::physicals`.
    SetCrewInfoPhysical {
        link: CrewInfoLink,
        physical: PhysicalInfo,
    },
    /// Engine-global `C4Game::LoadScenarioSection` request. This uses the
    /// player-command outcome channel only as the existing synchronous
    /// script-to-engine transport; it is not scoped to any player.
    LoadScenarioSection {
        name: String,
        flags: i32,
        /// Inactive objects survive C++'s section teardown. Capture their
        /// effective identities at the exact host-call point so the engine
        /// can preserve them while replacing the active section world.
        preserve_ids: Vec<ObjectId>,
    },
    /// `C4RoundResults::AddCustomEvaluationString`, keyed by persistent
    /// C4PlayerInfo ID (zero is the global evaluation text).
    AddEvaluationData { player_info_id: i32, text: String },
    /// `C4RoundResults::HideSettlementScore`; this engine-global flag is
    /// consumed when the evaluation player rows are built.
    HideSettlementScore { hide: bool },
    /// `C4RoundResults::SetLeaguePerformance`, keyed by persistent
    /// C4PlayerInfo ID (zero is the independent global slot).
    SetLeaguePerformance { score: i32, player_info_id: i32 },
    /// `C4PlayerInfo::SetLeagueProgressData`; `None` clears the underlying
    /// StdStrBuf while `Some([])` retains an allocated empty string.
    SetLeagueProgressData {
        player_info_id: i32,
        data: Option<Vec<u8>>,
    },
    /// `FnSetRestoreInfos`'s raw `C4NetworkRestartInfos::Infos::What`
    /// replacement. Unknown and negative bits are deliberately retained.
    SetRestoreInfos { what: i32 },
    /// `FnSetMaxPlayer`'s direct write to
    /// `C4GameParameters::MaxPlayers`. This is engine-global; like
    /// `LoadScenarioSection`, it uses the existing ordered player-command
    /// outcome channel only as script-to-engine transport.
    SetMaxPlayer { max_players: i32 },
    /// Register one `C4MessageInput` custom command. The host function always
    /// reports success after this request, while the authoritative registry
    /// silently preserves its first entry for a duplicate name.
    AddMessageBoardCommand {
        command: crate::InitialNetworkMessageBoardCommand,
    },
    /// Append a `C4MessageBoardQuery` after removing the first query for the
    /// same callback object from this player.
    CallMessageBoard {
        player_id: i32,
        query: crate::MessageBoardQuery,
    },
    /// Close an exact active script input and remove the first matching
    /// player query. Emitted for AbortMessageBoard even when no query exists,
    /// because the presentation line may still be open for an answered query.
    AbortMessageBoard {
        player_id: i32,
        target: Option<ObjectId>,
    },
    /// Remove one query while executing OnMessageBoardAnswer.
    RemoveMessageBoardQuery {
        player_id: i32,
        target: Option<ObjectId>,
    },
    /// `FnActivateGameGoalMenu`: evaluate the live goals on every peer and
    /// build presentation state only for a locally controlled player.
    ActivateGameGoalMenu { player_id: i32, open_menu: bool },
    /// `FnSetPlayerTeam`'s complete, callback-approved team transition.
    /// The host has already made every field visible to the still-running
    /// VM; this payload lets the authoritative engine repeat the transition
    /// after the copied script context returns.
    SetPlayerTeam {
        player_id: i32,
        team: Option<i32>,
        generated_team: Option<TeamInfo>,
        color: Option<u32>,
        home_base_material_entries: Option<Vec<(DefinitionId, i32)>>,
        synchronize_hostility: bool,
    },
    /// `FnInitScenarioPlayer`'s synchronous
    /// `C4Player::ScenarioAndTeamInit` request. The copied host context
    /// preflights the return value; the authoritative engine performs the
    /// scenario/team initialization after the VM call returns.
    InitScenarioPlayer { player_id: i32, team: i32 },
    /// Final live `C4Player::Crew` lists after a synchronous crew mutation.
    /// Membership is per player and independent of C4Object::Owner; one
    /// object may therefore occur in more than one roster.
    SetCrewRosters { rosters: Vec<(i32, Vec<ObjectId>)> },
    /// `C4ObjectInfo::Retire` for an info owned by this player's
    /// `CrewInfoList`. The object pointer itself is cleared by ObjectUpdate;
    /// this command keeps the persistent roster entry idle for later reuse.
    RetireCrewInfo {
        object_id: ObjectId,
        link: CrewInfoLink,
    },
    /// `C4Object::AssignDeath` marks the still-linked object info dead,
    /// increments its death count and retires its active stint. Unlike
    /// `RetireCrewInfo`, the object keeps its Info pointer.
    AssignDeathCrewInfo {
        object_id: ObjectId,
        link: CrewInfoLink,
    },
    /// Link an exact persistent CrewInfo entry to a live object. A newly
    /// created entry is appended before the link is installed; an existing
    /// entry is merely recruited. The full payload moves with GrabInfo.
    LinkCrewInfo {
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        info: CrewObjectInfo,
        created_entry: Option<crate::player_file::CrewInfo>,
        recruit: bool,
        has_died: bool,
    },
    /// One ordered `C4Object::DoExperience` call. Keep the change
    /// incremental so independently produced callback outcomes compound in
    /// engine order instead of replacing one another with stale snapshots.
    /// `link` is the pointer-equivalent identity of the persistent info that
    /// was attached at call time; removal or GrabInfo later in the same
    /// callback must not lose the mutation.
    AdjustCrewExperience {
        object_id: ObjectId,
        link: Option<CrewInfoLink>,
        change: i32,
    },
    /// Runtime-only `C4ObjectInfo::ControlCount` delta produced by a native
    /// command finish inside synchronous script-host execution. Experience
    /// calls crossed by this delta are transported as ordered
    /// `AdjustCrewExperience` commands.
    AdjustCrewControlCount { link: CrewInfoLink, gain: i32 },
    AdjustHomeBaseMaterial {
        player_id: i32,
        definition_id: DefinitionId,
        delta: i32,
    },
    /// `C4Player::SyncHomebaseMaterialToTeam` without a preceding list
    /// mutation. Sell2Home still performs this sync for a valid mapped
    /// non-Rebuyable definition that has no existing material entry.
    SyncHomeBaseMaterialToTeam { player_id: i32 },
    AdjustHomeBaseProduction {
        player_id: i32,
        definition_id: DefinitionId,
        delta: i32,
    },
    GrantKnowledge {
        player_id: i32,
        definition_id: DefinitionId,
    },
    RevokeKnowledge {
        player_id: i32,
        definition_id: DefinitionId,
    },
    GrantMagic {
        player_id: i32,
        definition_id: DefinitionId,
    },
    RevokeMagic {
        player_id: i32,
        definition_id: DefinitionId,
    },
    /// `FnSetWealth` (C4Script.cpp:2761-2766), already clamped.
    SetWealth {
        player_id: i32,
        value: i32,
        /// True for a C4Player::DoWealth path (buy/sell), which always arms
        /// ViewWealth; false for FnSetWealth's direct assignment.
        show_change: bool,
    },
    /// `FnDoScore` -> `C4Player::DoPoints` (C4Script.cpp:2762-2766;
    /// C4Player.cpp:1824-1828). Keep this incremental so independently
    /// batched script outcomes compound in their original order.
    AdjustPoints { player_id: i32, delta: i32 },
    /// `FnEliminatePlayer`'s regular path: mark the player eliminated and
    /// start C4RetireDelay before C4PlayerList retires them.
    Eliminate { player_id: i32 },
    /// `FnSurrenderPlayer`: mark the player surrendered/eliminated and start
    /// the same C4RetireDelay used by the synchronized surrender control.
    Surrender { player_id: i32 },
    /// `FnEliminatePlayer(..., true)` asks the control host to remove the
    /// player directly, without the regular elimination fate.
    Remove { player_id: i32 },
    /// `FnSetFoW` (C4Script.cpp:3671-3678): persist the explicit fog of
    /// war setting and its forced override on the validated player.
    SetFogOfWar { player_id: i32, enabled: bool },
    /// FnSetPlrShowControlPos's validated C4Player::ShowControlPos write.
    SetShowControlPosition { player_id: i32, position: i32 },
    /// FnSetPlrShowControl's validated, StringBitEval-encoded ShowControl write.
    SetShowControl { player_id: i32, mask: i32 },
    /// FnSetPlrShowCommand's runtime-only C4Player::FlashCom write.
    SetShowCommand { player_id: i32, command: i32 },
    /// `FnSetHostility`'s validated `C4Player::Hostility` update. Callback
    /// rejection and same-call visibility are resolved before this command is
    /// emitted; the engine applies the surviving declaration afterward.
    SetHostility {
        player_id: i32,
        opponent: i32,
        hostile: bool,
    },
    /// `FnSetPlrExtraData` (C4Script.cpp:4692-4732): a validated named
    /// slot write on C4Player::ExtraData.
    SetExtraData {
        player_id: i32,
        name: String,
        value: Value,
    },
    /// FnSetCursor (C4Script.cpp:2951-2958): pPlr->SetCursor(pObj) and,
    /// unless fNoSelectCrew, SelectCrew(pObj, true).
    SetCursor {
        player_id: i32,
        object: Option<ObjectId>,
        control: PlayerControlState,
    },
    /// FnSetViewCursor (C4Script.cpp:2954-2963): assign the player's
    /// independent camera-follow pointer without changing Cursor.
    SetViewCursor {
        player_id: i32,
        object: Option<ObjectId>,
    },
    /// FnSetPlrView (C4Script.cpp:2545-2550): switch to C4PVM_Target and
    /// follow ViewTarget without changing the independent ViewCursor.
    SetPlrView {
        player_id: i32,
        object: Option<ObjectId>,
    },
    /// C4Player::ClearPointers for an object removed during the same script
    /// call. Ordered after earlier SetPlrView/SetViewCursor commands.
    ClearObjectPointers { object: ObjectId },
    /// The prefix of one `C4Player::ClearPointers` step: clear Captain/Crew
    /// and write Cursor=null before callbackful AdjustCursorCommand.
    ClearPlayerObjectPointersBeforeAdjust { player_id: i32, object: ObjectId },
    /// The suffix of one `C4Player::ClearPointers` step, after
    /// AdjustCursorCommand: clear ViewCursor/ViewTarget/menu/query pointers.
    ClearPlayerObjectPointersAfterAdjust { player_id: i32, object: ObjectId },
    /// `C4Player::AdjustCursorCommand` starts with ResetCursorView while the
    /// old ViewCursor is still live.
    ResetCursorView { player_id: i32 },
    /// Its conditional UpdateView call, with the focus position resolved at
    /// the exact host-call point before selection callbacks.
    UpdatePlayerView {
        player_id: i32,
        position: Option<Vector2>,
    },
    /// FnClearLastPlrCom (C4Script.cpp:2624-2635): clear the pending
    /// single/double-click command latches, preserving LastComDelay.
    ClearLastPlrCom { player_id: i32 },
}

impl HostWorldObject {
    pub(crate) fn with_move_to_range(mut self, move_to_range: i32) -> Self {
        self.move_to_range = move_to_range;
        self
    }

    pub(crate) fn with_pathfinder(mut self, pathfinder: i32) -> Self {
        self.pathfinder = pathfinder;
        self
    }

    pub(crate) fn with_no_transfer_zones(mut self, no_transfer_zones: i32) -> Self {
        self.no_transfer_zones = no_transfer_zones;
        self
    }

    pub(crate) fn with_no_push_enter(mut self, no_push_enter: i32) -> Self {
        self.no_push_enter = no_push_enter;
        self
    }

    pub(crate) fn with_fixed_motion(mut self, position: FixedVec2, velocity: FixedVec2) -> Self {
        self.fixed_position = position;
        self.fixed_velocity = velocity;
        self
    }

    pub(crate) fn with_compiler_fields(
        mut self,
        motion_x: i32,
        motion_y: i32,
        last_attach_movement_frame: i32,
        compiler_cache: crate::ObjectCompilerCache,
    ) -> Self {
        self.motion_x = motion_x;
        self.motion_y = motion_y;
        self.last_attach_movement_frame = last_attach_movement_frame;
        self.compiler_cache = compiler_cache;
        self
    }

    pub(crate) fn with_rotation_velocity(mut self, rotation_velocity: C4Fixed) -> Self {
        self.rotation_velocity = rotation_velocity;
        self
    }

    pub(crate) fn with_fixed_rotation(mut self, fixed_rotation: C4Fixed) -> Self {
        self.fixed_rotation = fixed_rotation;
        self
    }

    pub(crate) fn with_direction(mut self, direction: i32) -> Self {
        self.direction = direction;
        self
    }

    #[cfg(test)]
    pub(crate) fn new(
        id: ObjectId,
        definition_id: impl Into<String>,
        status: ObjectStatus,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        energy: i32,
        construction: i32,
        position: Vector2,
        velocity: Vector2,
        vertices: Vec<ObjectVertex>,
        action_data: i32,
        action_ticks: i32,
        container: Option<ObjectId>,
    ) -> Self {
        Self::with_category(
            id,
            definition_id,
            status,
            action_name,
            action_target,
            action_target2,
            action_procedure,
            owner,
            DEFAULT_CATEGORY,
            energy,
            construction,
            0,
            position,
            velocity,
            0,
            vertices,
            action_data,
            action_ticks,
            0,
            container,
            None,
        )
    }

    pub(crate) fn with_category(
        id: ObjectId,
        definition_id: impl Into<String>,
        status: ObjectStatus,
        action_name: impl Into<String>,
        action_target: Option<ObjectId>,
        action_target2: Option<ObjectId>,
        action_procedure: Option<String>,
        owner: i32,
        category: i32,
        energy: i32,
        construction: i32,
        damage: i32,
        position: Vector2,
        velocity: Vector2,
        rotation: i32,
        vertices: Vec<ObjectVertex>,
        action_data: i32,
        action_ticks: i32,
        action_phase: i32,
        container: Option<ObjectId>,
        draw_transform: Option<DrawTransform>,
    ) -> Self {
        Self {
            id,
            definition_id: definition_id.into(),
            unsorted: false,
            status,
            alive: true,
            in_liquid: false,
            action_name: action_name.into(),
            action_index: None,
            direction: 0,
            action_target,
            action_target2,
            action_procedure,
            owner,
            controller: None,
            selected: false,
            crew_disabled: false,
            category,
            collectible: false,
            collection_available_ignoring_delay: false,
            collection_enabled: false,
            no_collect_delay: 0,
            collection_limit: 0,
            energy,
            need_energy: false,
            construction: construction.max(0),
            contact_density: crate::CONTACT_DENSITY_SOLID,
            damage,
            ocf: ocf::NORMAL,
            move_to_range: 0,
            pathfinder: 0,
            no_transfer_zones: 0,
            no_push_enter: 0,
            position,
            fixed_position: FixedVec2::from_ints(position.x, position.y),
            velocity,
            fixed_velocity: FixedVec2::from_ints(velocity.x, velocity.y),
            motion_x: 0,
            motion_y: 0,
            last_attach_movement_frame: -1,
            compiler_cache: crate::ObjectCompilerCache::default(),
            fixed_rotation: itofix(rotation),
            rotation_velocity: C4Fixed::ZERO,
            rotation,
            vertices,
            own_vertices: false,
            action_data,
            action_ticks,
            action_phase,
            container,
            contents: Vec::new(),
            draw_transform,
            commands: Vec::new(),
            command_stack: CommandStackSnapshot::default(),
            state: None,
            material_contents: Vec::new(),
            last_energy_loss_cause: OWNER_NONE,
        }
    }

    pub(crate) fn with_commands(mut self, commands: Vec<CommandView>) -> Self {
        self.commands = commands;
        self
    }

    pub(crate) fn with_unsorted(mut self, unsorted: bool) -> Self {
        self.unsorted = unsorted;
        self
    }

    pub(crate) fn with_action_index(mut self, action_index: Option<u32>) -> Self {
        self.action_index = action_index;
        self
    }

    pub(crate) fn with_own_vertices(mut self, own_vertices: bool) -> Self {
        self.own_vertices = own_vertices;
        self
    }

    pub(crate) fn with_command_stack(mut self, command_stack: CommandStackSnapshot) -> Self {
        self.command_stack = command_stack;
        self
    }

    pub(crate) fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub(crate) fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn with_crew_disabled(mut self, crew_disabled: bool) -> Self {
        self.crew_disabled = crew_disabled;
        self
    }

    pub(crate) fn with_collectible(mut self, collectible: bool) -> Self {
        self.collectible = collectible;
        self
    }

    pub(crate) fn with_collection_available_ignoring_delay(mut self, available: bool) -> Self {
        self.collection_available_ignoring_delay = available;
        self
    }

    pub(crate) fn with_collection_enabled(mut self, enabled: bool) -> Self {
        self.collection_enabled = enabled;
        self
    }

    pub(crate) fn with_no_collect_delay(mut self, delay: i32) -> Self {
        self.no_collect_delay = delay;
        self
    }

    pub(crate) fn with_collection_limit(mut self, limit: i32) -> Self {
        self.collection_limit = limit;
        self
    }

    pub(crate) fn with_need_energy(mut self, need_energy: bool) -> Self {
        self.need_energy = need_energy;
        self
    }

    pub(crate) fn with_contact_density(mut self, contact_density: i32) -> Self {
        self.contact_density = contact_density;
        self
    }

    pub(crate) fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = in_liquid;
        self
    }

    pub(crate) fn with_ocf(mut self, ocf: u32) -> Self {
        self.ocf = ocf;
        self
    }

    pub(crate) fn with_full_state(mut self, state: Rc<ObjectState>) -> Self {
        self.state = Some(state);
        self
    }

    pub(crate) fn with_material_contents(mut self, material_contents: Vec<i32>) -> Self {
        self.material_contents = material_contents;
        self
    }

    pub(crate) fn with_last_energy_loss_cause(mut self, cause: i32) -> Self {
        self.last_energy_loss_cause = cause;
        self
    }

    /// The full state snapshot, when the context was built by the engine
    /// (`Engine::host_world_context`). See the `state` field docs.
    pub(crate) fn full_state(&self) -> Option<&Rc<ObjectState>> {
        self.state.as_ref()
    }

    pub fn alive(&self) -> bool {
        self.alive
    }

    pub fn in_liquid(&self) -> bool {
        self.in_liquid
    }

    pub fn definition_id(&self) -> &str {
        &self.definition_id
    }

    pub fn status(&self) -> ObjectStatus {
        self.status
    }

    pub fn ocf(&self) -> u32 {
        self.ocf
    }

    fn collection_available_ignoring_delay(&self) -> bool {
        self.collection_available_ignoring_delay || self.ocf & ocf::COLLECTION != 0
    }

    pub fn action_target(&self, index: usize) -> Option<ObjectId> {
        match index {
            0 => self.action_target,
            1 => self.action_target2,
            _ => None,
        }
    }

    pub fn procedure_name(&self) -> Option<&str> {
        self.action_procedure.as_deref()
    }

    pub fn owner(&self) -> i32 {
        self.owner
    }

    /// C4Object::Controller — carried on the full-state snapshot; legacy
    /// fixture snapshots without one fall back to the owner (the Init
    /// default, C4Object.cpp:162).
    pub fn controller(&self) -> i32 {
        self.controller.unwrap_or_else(|| {
            self.state
                .as_ref()
                .map(|state| state.controller)
                .unwrap_or(self.owner)
        })
    }

    pub fn category(&self) -> i32 {
        self.category
    }

    pub fn energy(&self) -> i32 {
        self.energy
    }

    pub fn construction(&self) -> i32 {
        self.construction
    }

    pub fn contact_density(&self) -> i32 {
        self.contact_density
    }

    #[allow(dead_code)]
    pub fn damage(&self) -> i32 {
        self.damage
    }

    pub fn action_name(&self) -> &str {
        &self.action_name
    }

    pub fn container(&self) -> Option<ObjectId> {
        self.container
    }

    pub fn contents(&self) -> &[ObjectId] {
        &self.contents
    }

    pub fn with_contents(mut self, contents: Vec<ObjectId>) -> Self {
        self.contents = contents;
        self
    }

    pub fn is_present(&self) -> bool {
        !matches!(self.status, ObjectStatus::Deleted)
    }

    pub fn position(&self) -> Vector2 {
        self.position
    }

    pub fn velocity(&self) -> Vector2 {
        self.velocity
    }

    pub fn fixed_velocity(&self) -> FixedVec2 {
        self.fixed_velocity
    }

    pub fn vertices(&self) -> &[ObjectVertex] {
        &self.vertices
    }

    pub fn action_ticks(&self) -> i32 {
        self.action_ticks
    }

    #[allow(dead_code)]
    pub fn action_data(&self) -> i32 {
        self.action_data
    }

    pub fn action_phase(&self) -> i32 {
        self.action_phase
    }

    pub fn set_action_phase(&mut self, phase: i32) {
        self.action_phase = phase;
    }
}

#[derive(Clone, Default)]
pub(crate) struct HostCrewInfoState {
    pub(crate) idle: HashMap<(i32, String), Vec<(CrewInfoLink, crate::player_file::CrewInfo)>>,
    pub(crate) entries: HashMap<CrewInfoLink, crate::player_file::CrewInfo>,
    pub(crate) order: HashMap<i32, Vec<CrewInfoLink>>,
    pub(crate) next_indices: HashMap<i32, usize>,
    pub(crate) roster_names: HashMap<i32, Vec<String>>,
    /// Runtime-only `C4ObjectInfo::ControlCount`, sharing the stable roster
    /// identity used by the authoritative engine.
    pub(crate) control_counts: HashMap<CrewInfoLink, i32>,
}

/// Definition- and script-derived host data that is immutable between load
/// or relink boundaries. Host callbacks are frequent, so keep these tables
/// shared instead of rebuilding and immediately wrapping them in `Rc` for
/// every copied world context.
#[derive(Clone, Default)]
pub(crate) struct HostDefinitionTables {
    color_by_owner: Rc<HashSet<DefinitionId>>,
    base_auto_sell: Rc<HashSet<DefinitionId>>,
    rebuyable: Rc<HashSet<DefinitionId>>,
    no_sell: Rc<HashSet<DefinitionId>>,
    descriptions: Rc<HashMap<DefinitionId, String>>,
    rank_names: Rc<HashMap<DefinitionId, RankNameTable>>,
    rank_bases: Rc<HashMap<DefinitionId, i32>>,
    scripts: Rc<HashMap<DefinitionId, Arc<ScriptEngine>>>,
    linked_script_hosts: Rc<Vec<(String, Arc<ScriptEngine>)>>,
    standard_crew_names: Option<String>,
    definition_crew_names: Rc<HashMap<String, String>>,
    reference_parameter_slots: Rc<HashMap<String, u32>>,
}

impl HostDefinitionTables {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        color_by_owner: HashSet<DefinitionId>,
        base_auto_sell: HashSet<DefinitionId>,
        rebuyable: HashSet<DefinitionId>,
        no_sell: HashSet<DefinitionId>,
        descriptions: HashMap<DefinitionId, String>,
        rank_names: HashMap<DefinitionId, RankNameTable>,
        rank_bases: HashMap<DefinitionId, i32>,
        scripts: HashMap<DefinitionId, Arc<ScriptEngine>>,
        linked_script_hosts: Vec<(String, Arc<ScriptEngine>)>,
        standard_crew_names: Option<String>,
        definition_crew_names: HashMap<String, String>,
    ) -> Self {
        Self {
            color_by_owner: Rc::new(color_by_owner),
            base_auto_sell: Rc::new(base_auto_sell),
            rebuyable: Rc::new(rebuyable),
            no_sell: Rc::new(no_sell),
            descriptions: Rc::new(descriptions),
            rank_names: Rc::new(rank_names),
            rank_bases: Rc::new(rank_bases),
            reference_parameter_slots: Rc::new(reference_parameter_slots(
                &scripts,
                &linked_script_hosts,
            )),
            scripts: Rc::new(scripts),
            linked_script_hosts: Rc::new(linked_script_hosts),
            standard_crew_names,
            definition_crew_names: Rc::new(definition_crew_names),
        }
    }
}

/// C4AulParse resolves `&` parameters through the engine-wide same-name chain
/// (`GetFirstFunc`/`GetNextSNFunc`, C4AulParse.cpp:2318-2331,3225). Fold that
/// chain into one `name -> slot bitmask` table when the definition tables are
/// built, so an arrow call can answer it without walking every script host.
fn reference_parameter_slots(
    scripts: &HashMap<DefinitionId, Arc<ScriptEngine>>,
    linked_script_hosts: &[(String, Arc<ScriptEngine>)],
) -> HashMap<String, u32> {
    let mut slots: HashMap<String, u32> = HashMap::new();
    let mut collect = |script: &ScriptEngine| {
        for (name, function) in script.functions() {
            let mask = function
                .params
                .iter()
                .enumerate()
                .filter(|(index, parameter)| parameter.is_reference && *index < u32::BITS as usize)
                .fold(0u32, |mask, (index, _)| mask | (1 << index));
            if mask != 0 {
                *slots.entry(name.clone()).or_default() |= mask;
            }
        }
    };
    for script in scripts.values() {
        collect(script);
    }
    for (_, script) in linked_script_hosts {
        collect(script);
    }
    slots
}

// Not `derive(Debug)`: `ScriptEngine` (in `definition_scripts`) has no Debug.
#[derive(Clone, Copy)]
pub(crate) struct LazyHostWorldProvider {
    source: *const (),
    object: unsafe fn(*const (), ObjectId) -> Option<(usize, HostWorldObject)>,
    objects: unsafe fn(*const (), &HashSet<usize>) -> Vec<(usize, HostWorldObject)>,
    landscape: unsafe fn(*const ()) -> Option<Landscape>,
    legacy_find_object: Option<unsafe fn(*const (), ObjectId, &FindObjectParams) -> Option<bool>>,
}

impl LazyHostWorldProvider {
    /// Create a provider whose source is borrowed for the complete lifetime
    /// of every `HostWorldContext` clone carrying it.
    ///
    /// # Safety
    ///
    /// `source` must remain valid and at a stable address until those
    /// contexts are dropped. Provider callbacks run only inside the
    /// synchronous script invocation that created the context. While a
    /// callback runs, the source objects and landscape may not be moved or
    /// mutated through another path. An object already exclusively borrowed
    /// by the caller must be seeded into the context; `objects` receives its
    /// index in `excluded` and must not dereference that entry.
    pub(crate) unsafe fn new(
        source: *const (),
        object: unsafe fn(*const (), ObjectId) -> Option<(usize, HostWorldObject)>,
        objects: unsafe fn(*const (), &HashSet<usize>) -> Vec<(usize, HostWorldObject)>,
        landscape: unsafe fn(*const ()) -> Option<Landscape>,
    ) -> Self {
        Self {
            source,
            object,
            objects,
            landscape,
            legacy_find_object: None,
        }
    }

    pub(crate) fn with_legacy_find_object(
        mut self,
        legacy_find_object: unsafe fn(*const (), ObjectId, &FindObjectParams) -> Option<bool>,
    ) -> Self {
        self.legacy_find_object = Some(legacy_find_object);
        self
    }

    fn object(self, id: ObjectId) -> Option<(usize, HostWorldObject)> {
        // SAFETY: the constructor's source-lifetime and aliasing contract is
        // upheld by the engine's synchronous callback wrappers.
        unsafe { (self.object)(self.source, id) }
    }

    fn objects(self, excluded: &HashSet<usize>) -> Vec<(usize, HostWorldObject)> {
        // SAFETY: see `object`; excluded indices are never dereferenced by a
        // conforming provider.
        unsafe { (self.objects)(self.source, excluded) }
    }

    fn landscape(self) -> Option<Landscape> {
        // SAFETY: see `object`.
        unsafe { (self.landscape)(self.source) }
    }
}

#[derive(Clone, Default)]
pub(crate) struct HostWorldObjectStore {
    objects: HashMap<ObjectId, HostWorldObject>,
    pub(crate) order: Vec<ObjectId>,
    indices: HashMap<ObjectId, usize>,
    removed: HashSet<ObjectId>,
    complete: bool,
}

impl HostWorldObjectStore {
    fn insert_ordered_by_index(&mut self, id: ObjectId, index: usize) {
        // `get` materializes a previously absent object, so the id is not
        // already in `order`. Insert after equal indices to match appending
        // followed by stable `sort_by_key`, without re-sorting every prior
        // materialization.
        let indices = &self.indices;
        let insert_at = self.order.partition_point(|object_id| {
            indices.get(object_id).copied().unwrap_or(usize::MAX) <= index
        });
        self.order.insert(insert_at, id);
    }
}

#[derive(Clone)]
#[doc(hidden)]
pub struct HostWorldContext {
    /// Callback-local COW object view. The engine seeds the executing object;
    /// an id-specific lookup materializes only that object, while enumeration
    /// fills the complete map on demand.
    pub(crate) object_store: RefCell<Rc<HostWorldObjectStore>>,
    lazy_world: Option<LazyHostWorldProvider>,
    /// `Game.Objects` from First -> Next. The engine's `exec_list` is this
    /// order reversed; only APIs such as C4Game::FindBase explicitly walk
    /// the forward master list (C4Game.cpp:3732-3744).
    pub(crate) master_order: Rc<Vec<ObjectId>>,
    /// Uninitialized until a host API actually reads or mutates terrain.
    landscape: OnceCell<Option<Rc<Landscape>>>,
    /// Fully defaulted, post-load `Game.C4S` reflection data. This remains
    /// separate from the evaluated runtime landscape: GetScenarioVal reads
    /// the scenario core, not C4Landscape's mutable state.
    scenario_values: Rc<ScenarioValueStore>,
    /// Scenario-section group names available to `LoadScenarioSection`,
    /// normalized to ASCII lowercase like C++'s `SEqualNoCase` lookup.
    scenario_sections: Rc<HashSet<String>>,
    /// C4SolidMask pixels not already baked into the landscape plane.
    /// Grid worlds bake MCVehic directly; column fixtures retain the same
    /// overlay used by movement/contact checks.
    pub(crate) movement_solid_masks: Rc<Vec<crate::SolidMaskRect>>,
    pub(crate) definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
    /// Solid-mask geometry and sprite alpha shared into synchronous
    /// CreateObject callbacks. Only same-call pending objects consume it;
    /// committed grid-world masks are already baked into `landscape`.
    pub(crate) solid_mask_metadata: Rc<HashMap<DefinitionId, HostSolidMaskMetadata>>,
    /// Active grid-world C4SolidMask bakes, including each mask's saved
    /// background buffer. A synchronous callback clones these alongside the
    /// landscape so native operations such as DoCon can remove/re-put masks
    /// before nested script callbacks inspect GBack*.
    pub(crate) solid_mask_bakes: Rc<Vec<(ObjectId, crate::SolidMaskBake)>>,
    /// Live C4SolidMask instance ages, including eligible off-landscape
    /// masks that have no raster bake.
    pub(crate) solid_mask_instance_sequences: Rc<RefCell<HashMap<ObjectId, u64>>>,
    /// First unused instance age at callback entry.
    pub(crate) next_solid_mask_instance_sequence: Rc<Cell<u64>>,
    /// Definitions whose default graphics carry a ColorByOwner surface.
    /// This drives SetGraphics/ChangeDef's immediate Color reset.
    color_by_owner_definitions: Rc<HashSet<DefinitionId>>,
    base_auto_sell_definitions: Rc<HashSet<DefinitionId>>,
    rebuyable_definitions: Rc<HashSet<DefinitionId>>,
    no_sell_definitions: Rc<HashSet<DefinitionId>>,
    /// Localized `C4Def::GetDesc` text, kept separate from simulation
    /// metadata so presentation lookup does not enlarge every fixture.
    definition_descriptions: Rc<HashMap<DefinitionId, String>>,
    /// Finite localized `C4Def::pRankNames` lookup tables. Absence is
    /// distinct from an empty custom table: absent definitions fall back to
    /// the game-global rank system during `C4Object::Promote`.
    pub(crate) definition_rank_names: Rc<HashMap<DefinitionId, RankNameTable>>,
    /// Process-local `Game.Rank` names frozen from IDS_GAME_DEFRANKS when
    /// this game initialized.
    pub(crate) default_rank_names: Rc<Vec<String>>,
    /// `C4RankSystem::Base` paired with each custom definition rank table.
    definition_rank_bases: Rc<HashMap<DefinitionId, i32>>,
    /// Runtime `Game.Defs` order after C4DefList::SortByID. Definition-indexing
    /// APIs must never observe the nondeterministic order of `definitions`.
    definition_order: Rc<Vec<DefinitionId>>,
    /// Lazily built on the first sector query: most callbacks never run
    /// one, and an eager build per host context made every tick quadratic
    /// in the object count.
    sectors: RefCell<Option<Rc<SectorMap>>>,
    pub(crate) transfer_zones: Rc<Vec<TransferZoneState>>,
    /// Last values written to the game-global C4PathFinder by an obstructed
    /// MoveTo search. FnGetPath reuses them instead of resetting defaults.
    pathfinder_level: i32,
    pathfinder_transfer_zones_enabled: bool,
    /// Shared process-presentation sink for the global Game.PathFinder graph.
    pub(crate) pathfinder_debug: Rc<RefCell<PathfinderDebugSnapshot>>,
    pub(crate) players: Rc<HashMap<i32, PlayerState>>,
    /// Runtime-only `C4Player::FoWViewObjs` membership. PlayerState omits
    /// this list, but AssignDeath needs it before Death is called to decide
    /// whether a dead living object retains its view range.
    player_fow_view_objects: Rc<HashMap<i32, HashSet<ObjectId>>>,
    /// Process-local display names for configured keyboard/gamepad controls,
    /// keyed by the player's effective control-set number.
    control_key_names: Rc<HashMap<i32, Vec<crate::ControlKeyName>>>,
    /// IDs present in `Game.PlayerInfos`, including retained infos whose
    /// runtime C4Player has already retired. ID zero is the global-results
    /// sentinel and is never stored here.
    player_info_ids: Rc<HashSet<i32>>,
    player_order: Rc<Vec<i32>>,
    teams: Rc<Vec<TeamInfo>>,
    pub(crate) local_players: Rc<HashSet<i32>>,
    active_message_board_input: Option<crate::ActiveMessageBoardInput>,
    /// Legacy selection projection retained only for fixture/FFI contexts
    /// that name crew ids without providing corresponding world objects.
    pub(crate) crew_selection: Rc<HashMap<i32, CrewSelectionState>>,
    next_object_id: u64,
    team_home_base_rule: bool,
    pub(crate) needed_material_strings: Rc<crate::NeededMaterialStrings>,
    /// Process-local ConstructionCheck feedback templates
    /// (C4Landscape.cpp:2131-2163).
    pub(crate) construction_check_strings: Rc<crate::ConstructionCheckStrings>,
    /// Process-local `IDS_OBJ_NODIG` template used by synchronous queued-Dig
    /// execution inside a script callback.
    pub(crate) object_no_dig_resource_string: Rc<String>,
    /// `C4GameParameters::isLeague`: league games forbid every scripted
    /// team switch, including an otherwise successful same-team no-op.
    pub(crate) league_game: bool,
    /// Process-local `Application.iGameTickDelay`. Script callbacks share
    /// the live cell so SetGameSpeed takes effect synchronously like C++.
    game_tick_delay_ms: Rc<Cell<u64>>,
    /// Replaced with a fresh token on every successful SetGameSpeed, including
    /// equal-delay calls, because C++ unconditionally restarts its timer.
    game_tick_delay_revision: Rc<Cell<u64>>,
    /// Exact `Game.Parameters.League` bytes. This is a different parameter
    /// from `isLeague()`/LeagueAddress and gates the progress-data API.
    pub(crate) league_name: Rc<Vec<u8>>,
    /// Persistent `Game.PlayerInfos` projection keyed by C4PlayerInfo::ID.
    /// A missing key is an unknown info, `None` is a null StdStrBuf, and
    /// `Some([])` is its distinct allocated-empty state.
    player_info_league_progress_data: Rc<BTreeMap<i32, Option<Vec<u8>>>>,
    /// Sparse C4PlayerInfo::iLeagueScore overrides keyed by player-info ID.
    /// Known infos absent from this map retain the native default score zero.
    pub(crate) player_info_league_scores: Rc<BTreeMap<i32, i32>>,
    /// Complete live `Game.Teams` configuration. Empty team lists cannot be
    /// used to infer these flags because present-empty and missing Teams.txt
    /// take different C++ paths.
    team_configuration: TeamConfiguration,
    /// `Game.Parameters.IsNetworkGame`, copied from the active
    /// `Game.NetworkActive` session during parameter setup
    /// (C4GameParameters.cpp:429-434).
    network_game: bool,
    /// `C4GameControl::isNetwork()`: unlike the persisted network-game
    /// parameter this becomes false after ChangeToLocal.
    network_control_mode: bool,
    /// `C4GameControl::SyncMode()`: network/replay control or an attached
    /// recording hides process-local view state from synchronized scripts.
    pub(crate) control_sync_mode: bool,
    /// Process-local `Console.EditCursor.Target`; absent when no developer
    /// console/edit cursor exists.
    pub(crate) edit_cursor_target: Option<ObjectId>,
    /// Process-local `Game.Control.isReplay()` state. Unlike SyncMode this
    /// excludes ordinary network and recording sessions.
    pub(crate) replay_control: bool,
    /// App-owned `Game.GraphicsSystem.GetViewportCount() > 0` projection.
    pub(crate) film_viewport_available: bool,
    /// App-owned console pause requests produced by `PauseGame`.
    pause_game_requests: Rc<RefCell<Vec<PauseGameRequest>>>,
    /// App-owned local pacing requests produced by `SetPreSend`.
    network_target_fps_requests: Rc<RefCell<Vec<crate::NetworkTargetFpsRequest>>>,
    /// App-owned physical viewport mutations in exact script-call order.
    pub(crate) viewport_presentation_requests: Rc<RefCell<Vec<crate::ViewportPresentationRequest>>>,
    /// Effective `GetSmokeLevel` for sync-relevant FXU1 creation: 150 in
    /// network/recording sync mode, otherwise Config.Graphics.SmokeLevel.
    smoke_level: i32,
    /// Live `Game.Parameters.MaxPlayers`. Successful script writes update
    /// this preview before their deferred engine command is folded back.
    max_players: i32,
    /// Live fair-crew round parameters used by C4Object::GetPhysical in
    /// synchronous and nested script callbacks.
    pub(crate) use_fair_crew: bool,
    pub(crate) fair_crew_strength: i32,
    pub(crate) fair_crew_physical_cache: crate::FairCrewPhysicalCache,
    /// `Game.Control.isCtrlHost()`, independent from network-game state.
    pub(crate) control_host: bool,
    /// Ordered player-info updates produced inside copied script contexts.
    pub(crate) player_info_updates: Rc<RefCell<Vec<crate::PlayerInfoUpdateRequest>>>,
    /// Live `Game.Script.Counter`. C4GameScriptHost::Execute increments it
    /// before entering ScriptN, and ScriptCounter() observes that increment.
    scenario_script_counter: i32,
    /// C4RULE_StructuresNeedEnergy (Game.Rules; FnEnergyCheck gates on
    /// it, C4Script.cpp:1845-1856).
    pub(crate) structures_need_energy: bool,
    /// Cached `Game.Rules & C4RULE_FlagRemoveable`, refreshed by the engine
    /// on InitRules/frame one/Tick255 like C++ UpdateRules.
    flag_removeable: bool,
    /// Exact Game.Names text and per-definition ClonkNames sources used by
    /// C4ObjectInfoList::New inside a synchronous MakeCrewMember call.
    pub(crate) standard_crew_names: Option<String>,
    pub(crate) definition_crew_names: Rc<HashMap<String, String>>,
    /// Mutable projection of the players' C4ObjectInfoList state. A host
    /// callback can recruit/create several infos before its outcome is folded
    /// into Engine, so consumed entries and newly allocated indices live here.
    pub(crate) crew_info_state: Rc<RefCell<HostCrewInfoState>>,
    /// Names of loaded particle defs (C4ParticleSystem::GetDef,
    /// C4Particles.cpp:465-473). `None` = no registry attached (legacy
    /// fixture contexts): name lookups behave permissively. `Some` = engine
    /// attached its registry: unknown names make the particle host functions
    /// return false exactly like the C++ GetDef-failure paths
    /// (C4Script.cpp:4874,4893,4917,4932).
    particle_defs: Option<Rc<std::collections::HashSet<String>>>,
    /// Compiled definition scripts, shared from `Engine.definitions`, so host
    /// functions can run script functions on other objects mid-VM-call
    /// (Find_Func/Sort_Func, GameCall). Empty in legacy fixture contexts.
    definition_scripts: Rc<HashMap<DefinitionId, Arc<ScriptEngine>>>,
    /// Engine-wide `&`-parameter slots per function name (C4AulParse's
    /// `anyfunctakesref` chain, folded once when the tables are installed).
    reference_parameter_slots: Rc<HashMap<String, u32>>,
    /// Retained System.c4g hosts. Their global functions live in the shared
    /// engine table, but `Func->LinkedTo` still resolves local functions on
    /// the declaring System script (for example an OrderFunc comparator).
    linked_script_hosts: Rc<Vec<(String, Arc<ScriptEngine>)>>,
    /// The material table (Game.Material): name lookups for FnMaterial
    /// (C4Script.cpp:2488-2491). `None` in legacy fixture contexts.
    pub(crate) materials: Option<Rc<MaterialSet>>,
    /// Crew object ranks from the engine's crew infos (`pObj->Info->Rank`;
    /// GetHiRank reads them, C4Player.cpp:1012). Objects without an entry
    /// behave like info-less crew (rank -1).
    crew_ranks: Rc<HashMap<u64, i32>>,
    /// Full modeled C4ObjectInfoCore values for GetObjectInfoCoreVal.
    pub(crate) crew_infos: Rc<HashMap<ObjectId, CrewObjectInfo>>,
    /// Player whose `CrewInfoList` owns each live C4Object::Info pointer.
    /// This is intentionally independent of both object Owner and crew-list
    /// membership (C4Player::SetObjectCrewStatus may change either alone).
    crew_info_links: Rc<HashMap<ObjectId, CrewInfoLink>>,
    /// The scenario script, shared from `Engine.scenario_script`, for
    /// GameCall/GameCallEx mid-VM-call resolution (C++ resolves on
    /// `Game.Script`, C4Script.cpp:3483). `None` when no scenario script is
    /// installed (and in fixture contexts).
    scenario_script: Option<Arc<ScriptEngine>>,
    pub(crate) frame: u64,
    /// Live `Game.Time`, used by C4ObjectInfo::Retire during AssignDeath.
    game_time: i32,
    pub(crate) base_buy_enabled: bool,
    pub(crate) base_sell_enabled: bool,
    pub(crate) base_auto_sell_enabled: bool,
    pub(crate) base_reject_entrance_enabled: bool,
    pub(crate) base_extinguish_enabled: bool,
    /// Raw `C4Sky::Modulation`/`BackClr` at callback entry.
    sky_adjustment: SkyAdjustment,
    /// `C4Sky::FadeClr1`/`FadeClr2` at callback entry. GetSkyColor reads
    /// these independently of the mutable sky-adjustment preview.
    pub(crate) sky_fade: [RgbColor; 2],
    /// Engine-owned surrogate for process config `MissionAccess`, shared so
    /// grants are visible immediately to later and nested script calls.
    pub(crate) mission_access: Rc<RefCell<String>>,
    pub(crate) scoreboard: Rc<RefCell<ScoreboardState>>,
    pub(crate) scoreboard_presentations: Rc<RefCell<ScoreboardPresentationSink>>,
}

/// Exact callback-final raster state threaded into the next callback phase.
/// The authoritative engine still replays ordered operations separately.
#[derive(Debug, Clone)]
pub(crate) struct HostRasterPreview {
    pub(crate) landscape: Option<Landscape>,
    pub(crate) solid_mask_bakes: Vec<(ObjectId, crate::SolidMaskBake)>,
    pub(crate) solid_mask_instance_sequences: HashMap<ObjectId, u64>,
    pub(crate) next_solid_mask_instance_sequence: u64,
}

impl Default for HostWorldContext {
    fn default() -> Self {
        Self {
            object_store: RefCell::new(Rc::new(HostWorldObjectStore {
                complete: true,
                ..HostWorldObjectStore::default()
            })),
            lazy_world: None,
            master_order: Rc::new(Vec::new()),
            landscape: OnceCell::new(),
            scenario_values: Rc::new(ScenarioValueStore::default()),
            scenario_sections: Rc::new(HashSet::new()),
            movement_solid_masks: Rc::new(Vec::new()),
            definitions: Rc::new(HashMap::new()),
            solid_mask_metadata: Rc::new(HashMap::new()),
            solid_mask_bakes: Rc::new(Vec::new()),
            solid_mask_instance_sequences: Rc::new(RefCell::new(HashMap::new())),
            next_solid_mask_instance_sequence: Rc::new(Cell::new(1)),
            color_by_owner_definitions: Rc::new(HashSet::new()),
            base_auto_sell_definitions: Rc::new(HashSet::new()),
            rebuyable_definitions: Rc::new(HashSet::new()),
            no_sell_definitions: Rc::new(HashSet::new()),
            definition_descriptions: Rc::new(HashMap::new()),
            definition_rank_names: Rc::new(HashMap::new()),
            default_rank_names: Rc::new(crate::us_default_rank_names()),
            definition_rank_bases: Rc::new(HashMap::new()),
            definition_order: Rc::new(Vec::new()),
            sectors: RefCell::new(None),
            transfer_zones: Rc::new(Vec::new()),
            pathfinder_level: 1,
            pathfinder_transfer_zones_enabled: true,
            pathfinder_debug: Rc::new(RefCell::new(PathfinderDebugSnapshot::default())),
            players: Rc::new(HashMap::new()),
            player_fow_view_objects: Rc::new(HashMap::new()),
            control_key_names: Rc::new(HashMap::new()),
            player_info_ids: Rc::new(HashSet::new()),
            player_order: Rc::new(Vec::new()),
            teams: Rc::new(Vec::new()),
            local_players: Rc::new(HashSet::new()),
            active_message_board_input: None,
            crew_selection: Rc::new(HashMap::new()),
            next_object_id: 1,
            league_game: false,
            game_tick_delay_ms: Rc::new(Cell::new(crate::DEFAULT_GAME_TICK_DELAY_MS)),
            game_tick_delay_revision: Rc::new(Cell::new(0)),
            league_name: Rc::new(Vec::new()),
            player_info_league_progress_data: Rc::new(BTreeMap::new()),
            player_info_league_scores: Rc::new(BTreeMap::new()),
            team_configuration: TeamConfiguration::default(),
            network_game: false,
            network_control_mode: false,
            control_sync_mode: false,
            edit_cursor_target: None,
            replay_control: false,
            film_viewport_available: false,
            pause_game_requests: Rc::new(RefCell::new(Vec::new())),
            network_target_fps_requests: Rc::new(RefCell::new(Vec::new())),
            viewport_presentation_requests: Rc::new(RefCell::new(Vec::new())),
            smoke_level: crate::DEFAULT_SMOKE_LEVEL,
            max_players: 0,
            use_fair_crew: false,
            fair_crew_strength: 1_000,
            fair_crew_physical_cache: Rc::new(RefCell::new(HashMap::new())),
            control_host: true,
            player_info_updates: Rc::new(RefCell::new(Vec::new())),
            scenario_script_counter: 0,
            structures_need_energy: false,
            flag_removeable: false,
            standard_crew_names: None,
            definition_crew_names: Rc::new(HashMap::new()),
            crew_info_state: Rc::new(RefCell::new(HostCrewInfoState::default())),
            team_home_base_rule: false,
            needed_material_strings: Rc::new(crate::NeededMaterialStrings::default()),
            construction_check_strings: Rc::new(crate::ConstructionCheckStrings::default()),
            object_no_dig_resource_string: Rc::new("%s cannot dig.".to_string()),
            particle_defs: None,
            definition_scripts: Rc::new(HashMap::new()),
            reference_parameter_slots: Rc::new(HashMap::new()),
            linked_script_hosts: Rc::new(Vec::new()),
            scenario_script: None,
            crew_ranks: Rc::new(HashMap::new()),
            crew_infos: Rc::new(HashMap::new()),
            crew_info_links: Rc::new(HashMap::new()),
            materials: None,
            frame: 0,
            game_time: 0,
            base_buy_enabled: true,
            base_sell_enabled: true,
            base_auto_sell_enabled: true,
            base_reject_entrance_enabled: true,
            base_extinguish_enabled: true,
            sky_adjustment: SkyAdjustment::default(),
            sky_fade: default_sky_fade(),
            mission_access: Rc::new(RefCell::new(String::new())),
            scoreboard: Rc::new(RefCell::new(ScoreboardState::default())),
            scoreboard_presentations: Rc::new(RefCell::new(ScoreboardPresentationSink::default())),
        }
    }
}

impl HostWorldContext {
    /// Replace the live object/landscape portion of a partially built host
    /// context. Engine movement uses this to defer the expensive object-state
    /// materialization until a Contact* callback actually fires.
    pub(crate) fn with_objects_and_landscape<I>(
        mut self,
        objects: I,
        landscape: Option<Landscape>,
    ) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        let objects = objects.into_iter().collect::<Vec<HostWorldObject>>();
        let mut store = HostWorldObjectStore {
            objects: HashMap::with_capacity(objects.len()),
            order: Vec::with_capacity(objects.len()),
            indices: HashMap::with_capacity(objects.len()),
            removed: HashSet::new(),
            complete: true,
        };
        for (index, object) in objects.into_iter().enumerate() {
            let id = object.id;
            store.order.push(id);
            store.indices.insert(id, index);
            store.objects.insert(id, object);
        }
        self.object_store = RefCell::new(Rc::new(store));
        self.lazy_world = None;
        let replace_landscape = match self.landscape.get() {
            None => true,
            Some(current) => current.is_none() && landscape.is_some(),
        };
        if replace_landscape {
            self.landscape = OnceCell::from(landscape.map(Rc::new));
        }
        self.sectors = RefCell::new(None);
        self
    }

    /// Attach the engine's synchronous lazy source. Empty fixture contexts
    /// remain complete; only engine callback contexts opt into this state.
    pub(crate) fn with_lazy_world_provider(mut self, provider: LazyHostWorldProvider) -> Self {
        self.lazy_world = Some(provider);
        Rc::make_mut(self.object_store.get_mut()).complete = false;
        if self.landscape.get().is_some_and(Option::is_none) {
            self.landscape = OnceCell::new();
        }
        self
    }

    /// Seed an object that the callback already owns. This is both the fast
    /// path for ordinary self-only callbacks and the aliasing boundary for a
    /// live engine object held through `&mut` during the script call.
    pub(crate) fn with_seeded_object(mut self, index: usize, object: HostWorldObject) -> Self {
        self.seed_object(index, object);
        self
    }

    pub(crate) fn seed_object(&mut self, index: usize, object: HostWorldObject) {
        let id = object.id;
        let store = Rc::make_mut(self.object_store.get_mut());
        store.removed.remove(&id);
        store.indices.insert(id, index);
        store.objects.insert(id, object);
        if !store.order.contains(&id) {
            store.order.push(id);
            store.order.sort_by_key(|object_id| {
                store.indices.get(object_id).copied().unwrap_or(usize::MAX)
            });
        }
        self.sectors = RefCell::new(None);
    }

    pub(crate) fn with_definition_tables(
        mut self,
        tables: Rc<HostDefinitionTables>,
        base_auto_sell_enabled: bool,
        crew_info_state: HostCrewInfoState,
    ) -> Self {
        self.color_by_owner_definitions = Rc::clone(&tables.color_by_owner);
        self.base_auto_sell_definitions = Rc::clone(&tables.base_auto_sell);
        self.rebuyable_definitions = Rc::clone(&tables.rebuyable);
        self.no_sell_definitions = Rc::clone(&tables.no_sell);
        self.definition_descriptions = Rc::clone(&tables.descriptions);
        self.definition_rank_names = Rc::clone(&tables.rank_names);
        self.definition_rank_bases = Rc::clone(&tables.rank_bases);
        self.definition_scripts = Rc::clone(&tables.scripts);
        self.reference_parameter_slots = Rc::clone(&tables.reference_parameter_slots);
        self.linked_script_hosts = Rc::clone(&tables.linked_script_hosts);
        self.standard_crew_names = tables.standard_crew_names.clone();
        self.definition_crew_names = Rc::clone(&tables.definition_crew_names);
        self.base_auto_sell_enabled = base_auto_sell_enabled;
        self.crew_info_state = Rc::new(RefCell::new(crew_info_state));
        self
    }

    pub(crate) fn with_needed_material_strings(
        mut self,
        strings: Rc<crate::NeededMaterialStrings>,
    ) -> Self {
        self.needed_material_strings = strings;
        self
    }

    pub(crate) fn with_construction_check_strings(
        mut self,
        strings: Rc<crate::ConstructionCheckStrings>,
    ) -> Self {
        self.construction_check_strings = strings;
        self
    }

    pub(crate) fn with_object_no_dig_resource_string(mut self, template: Rc<String>) -> Self {
        self.object_no_dig_resource_string = template;
        self
    }

    pub(crate) fn with_command_settings(
        mut self,
        frame: u64,
        base_buy_enabled: bool,
        base_sell_enabled: bool,
        base_reject_entrance_enabled: bool,
        base_extinguish_enabled: bool,
    ) -> Self {
        self.frame = frame;
        self.base_buy_enabled = base_buy_enabled;
        self.base_sell_enabled = base_sell_enabled;
        self.base_reject_entrance_enabled = base_reject_entrance_enabled;
        self.base_extinguish_enabled = base_extinguish_enabled;
        self
    }

    pub(crate) fn with_structures_need_energy(mut self, value: bool) -> Self {
        self.structures_need_energy = value;
        self
    }

    pub(crate) fn with_flag_removeable(mut self, value: bool) -> Self {
        self.flag_removeable = value;
        self
    }

    pub(crate) fn structures_need_energy(&self) -> bool {
        self.structures_need_energy
    }

    pub(crate) fn flag_removeable(&self) -> bool {
        self.flag_removeable
    }

    #[cfg(test)]
    pub(crate) fn from_objects<I>(objects: I) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        Self::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        )
    }

    #[cfg(test)]
    pub(crate) fn from_objects_with_players<I, P>(objects: I, players: P) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
        P: IntoIterator<Item = PlayerState>,
    {
        let map = players
            .into_iter()
            .map(|state| (state.id, state))
            .collect::<HashMap<_, _>>();
        Self::with_landscape(
            objects,
            None,
            HashMap::new(),
            Vec::new(),
            map,
            HashMap::new(),
            1,
            false,
        )
    }

    pub(crate) fn with_landscape<I>(
        objects: I,
        landscape: Option<Landscape>,
        definitions: HashMap<DefinitionId, DefinitionMetadata>,
        transfer_zones: Vec<TransferZoneState>,
        players: HashMap<i32, PlayerState>,
        crew_selection: HashMap<i32, CrewSelectionState>,
        next_object_id: u64,
        team_home_base_rule: bool,
    ) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        Self::with_landscape_shared(
            objects,
            landscape,
            Rc::new(definitions),
            Rc::new(ScenarioValueStore::default()),
            Rc::new(crate::us_default_rank_names()),
            transfer_zones,
            players,
            crew_selection,
            next_object_id,
            team_home_base_rule,
        )
    }

    /// `with_landscape` with an already-shared metadata table: definitions
    /// are immutable during play, so the engine caches the table instead of
    /// re-cloning every ActionLibrary per host context.
    pub(crate) fn with_landscape_shared<I>(
        objects: I,
        landscape: Option<Landscape>,
        definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
        scenario_values: Rc<ScenarioValueStore>,
        default_rank_names: Rc<Vec<String>>,
        transfer_zones: Vec<TransferZoneState>,
        players: HashMap<i32, PlayerState>,
        crew_selection: HashMap<i32, CrewSelectionState>,
        next_object_id: u64,
        team_home_base_rule: bool,
    ) -> Self
    where
        I: IntoIterator<Item = HostWorldObject>,
    {
        let map = objects.into_iter().collect::<Vec<HostWorldObject>>();
        let sectors = RefCell::new(None);
        let mut order = Vec::with_capacity(map.len());
        let mut lookup = HashMap::with_capacity(map.len());
        for object in map {
            let id = object.id;
            order.push(id);
            lookup.insert(id, object);
        }
        // Fixture/FFI compatibility: older callers carry selection only in
        // CrewSelectionState. Project that legacy view onto the canonical
        // C4Object::Select bit before host queries run.
        for (&owner, selection) in &crew_selection {
            for &id in &selection.selected {
                if let Some(object) = lookup.get_mut(&id) {
                    if object.owner == owner {
                        object.selected = true;
                    }
                }
            }
        }
        let order = Rc::new(order);
        let mut player_ids: Vec<_> = players.keys().copied().collect();
        player_ids.sort_unstable();
        let player_info_ids = players
            .values()
            .map(|player| player.player_info_id)
            .filter(|id| *id != 0)
            .collect();
        Self {
            object_store: RefCell::new(Rc::new(HostWorldObjectStore {
                objects: lookup,
                order: order.as_ref().clone(),
                indices: order
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, id)| (id, index))
                    .collect(),
                removed: HashSet::new(),
                complete: true,
            })),
            lazy_world: None,
            master_order: Rc::clone(&order),
            landscape: OnceCell::from(landscape.map(Rc::new)),
            scenario_values,
            scenario_sections: Rc::new(HashSet::new()),
            movement_solid_masks: Rc::new(Vec::new()),
            definitions,
            solid_mask_metadata: Rc::new(HashMap::new()),
            solid_mask_bakes: Rc::new(Vec::new()),
            solid_mask_instance_sequences: Rc::new(RefCell::new(HashMap::new())),
            next_solid_mask_instance_sequence: Rc::new(Cell::new(1)),
            color_by_owner_definitions: Rc::new(HashSet::new()),
            base_auto_sell_definitions: Rc::new(HashSet::new()),
            rebuyable_definitions: Rc::new(HashSet::new()),
            no_sell_definitions: Rc::new(HashSet::new()),
            definition_descriptions: Rc::new(HashMap::new()),
            definition_rank_names: Rc::new(HashMap::new()),
            default_rank_names,
            definition_rank_bases: Rc::new(HashMap::new()),
            definition_order: Rc::new(Vec::new()),
            sectors,
            transfer_zones: Rc::new(transfer_zones),
            pathfinder_level: 1,
            pathfinder_transfer_zones_enabled: true,
            pathfinder_debug: Rc::new(RefCell::new(PathfinderDebugSnapshot::default())),
            local_players: Rc::new(player_ids.iter().copied().collect()),
            active_message_board_input: None,
            player_order: Rc::new(player_ids),
            player_info_ids: Rc::new(player_info_ids),
            players: Rc::new(players),
            player_fow_view_objects: Rc::new(HashMap::new()),
            control_key_names: Rc::new(HashMap::new()),
            teams: Rc::new(Vec::new()),
            crew_selection: Rc::new(crew_selection),
            next_object_id,
            team_home_base_rule,
            needed_material_strings: Rc::new(crate::NeededMaterialStrings::default()),
            construction_check_strings: Rc::new(crate::ConstructionCheckStrings::default()),
            object_no_dig_resource_string: Rc::new("%s cannot dig.".to_string()),
            league_game: false,
            game_tick_delay_ms: Rc::new(Cell::new(crate::DEFAULT_GAME_TICK_DELAY_MS)),
            game_tick_delay_revision: Rc::new(Cell::new(0)),
            league_name: Rc::new(Vec::new()),
            player_info_league_progress_data: Rc::new(BTreeMap::new()),
            player_info_league_scores: Rc::new(BTreeMap::new()),
            team_configuration: TeamConfiguration::default(),
            network_game: false,
            network_control_mode: false,
            control_sync_mode: false,
            edit_cursor_target: None,
            replay_control: false,
            film_viewport_available: false,
            pause_game_requests: Rc::new(RefCell::new(Vec::new())),
            network_target_fps_requests: Rc::new(RefCell::new(Vec::new())),
            viewport_presentation_requests: Rc::new(RefCell::new(Vec::new())),
            smoke_level: crate::DEFAULT_SMOKE_LEVEL,
            max_players: 0,
            use_fair_crew: false,
            fair_crew_strength: 1_000,
            fair_crew_physical_cache: Rc::new(RefCell::new(HashMap::new())),
            control_host: true,
            player_info_updates: Rc::new(RefCell::new(Vec::new())),
            scenario_script_counter: 0,
            structures_need_energy: false,
            flag_removeable: false,
            standard_crew_names: None,
            definition_crew_names: Rc::new(HashMap::new()),
            crew_info_state: Rc::new(RefCell::new(HostCrewInfoState::default())),
            particle_defs: None,
            definition_scripts: Rc::new(HashMap::new()),
            reference_parameter_slots: Rc::new(HashMap::new()),
            linked_script_hosts: Rc::new(Vec::new()),
            scenario_script: None,
            crew_ranks: Rc::new(HashMap::new()),
            crew_infos: Rc::new(HashMap::new()),
            crew_info_links: Rc::new(HashMap::new()),
            materials: None,
            frame: 0,
            game_time: 0,
            base_buy_enabled: true,
            base_sell_enabled: true,
            base_auto_sell_enabled: true,
            base_reject_entrance_enabled: true,
            base_extinguish_enabled: true,
            sky_adjustment: SkyAdjustment::default(),
            sky_fade: default_sky_fade(),
            mission_access: Rc::new(RefCell::new(String::new())),
            scoreboard: Rc::new(RefCell::new(ScoreboardState::default())),
            scoreboard_presentations: Rc::new(RefCell::new(ScoreboardPresentationSink::default())),
        }
    }

    pub(crate) fn with_sky_adjustment(mut self, adjustment: SkyAdjustment) -> Self {
        self.sky_adjustment = adjustment;
        self
    }

    pub(crate) fn with_player_fow_view_objects<I, O>(mut self, players: I) -> Self
    where
        I: IntoIterator<Item = (i32, O)>,
        O: IntoIterator<Item = ObjectId>,
    {
        self.player_fow_view_objects = Rc::new(
            players
                .into_iter()
                .map(|(player, objects)| (player, objects.into_iter().collect()))
                .collect(),
        );
        self
    }

    pub(crate) fn player_has_fow_view_object(&self, player: i32, object: ObjectId) -> bool {
        self.player_fow_view_objects
            .get(&player)
            .is_some_and(|objects| objects.contains(&object))
    }

    pub(crate) fn remove_player_fow_view_object(&mut self, player: i32, object: ObjectId) {
        if let Some(objects) = Rc::make_mut(&mut self.player_fow_view_objects).get_mut(&player) {
            objects.remove(&object);
        }
    }

    /// `C4Object::PlrFoWActualize`: remove and conditionally re-add the
    /// object in its current owner's list, or every player's list when it
    /// has no valid owner.
    pub(crate) fn actualize_player_fow_view_object(
        &mut self,
        object: ObjectId,
        owner: i32,
        range: i32,
    ) {
        let player_ids = if self.players.contains_key(&owner) {
            vec![owner]
        } else {
            self.players.keys().copied().collect()
        };
        let memberships = Rc::make_mut(&mut self.player_fow_view_objects);
        for player in player_ids {
            let objects = memberships.entry(player).or_default();
            objects.remove(&object);
            if range != 0 {
                objects.insert(object);
            }
        }
    }

    /// The FoW-list half of `C4Object::SetOwner`: remove through the OLD
    /// owner semantics, then actualize the NEW valid owner when non-null.
    pub(crate) fn change_player_fow_view_object_owner(
        &mut self,
        object: ObjectId,
        old_owner: i32,
        new_owner: i32,
        range: i32,
    ) {
        let old_player_ids = if self.players.contains_key(&old_owner) {
            vec![old_owner]
        } else {
            self.players.keys().copied().collect()
        };
        let memberships = Rc::make_mut(&mut self.player_fow_view_objects);
        for player in old_player_ids {
            if let Some(objects) = memberships.get_mut(&player) {
                objects.remove(&object);
            }
        }
        if new_owner != OWNER_NONE {
            self.actualize_player_fow_view_object(object, new_owner, range);
        }
    }

    pub(crate) fn with_game_time(mut self, game_time: i32) -> Self {
        self.game_time = game_time;
        self
    }

    pub(crate) fn game_time(&self) -> i32 {
        self.game_time
    }

    pub(crate) fn with_sky_fade(mut self, top: RgbColor, bottom: RgbColor) -> Self {
        self.sky_fade = [top, bottom];
        self
    }

    pub(crate) fn sky_adjustment(&self) -> SkyAdjustment {
        self.sky_adjustment
    }

    pub(crate) fn with_scoreboard(mut self, scoreboard: Rc<RefCell<ScoreboardState>>) -> Self {
        self.scoreboard = scoreboard;
        self
    }

    pub(crate) fn with_mission_access(mut self, access: Rc<RefCell<String>>) -> Self {
        self.mission_access = access;
        self
    }

    pub(crate) fn with_scoreboard_presentations(
        mut self,
        presentations: Rc<RefCell<ScoreboardPresentationSink>>,
    ) -> Self {
        self.scoreboard_presentations = presentations;
        self
    }

    pub(crate) fn with_local_players<I>(mut self, players: I) -> Self
    where
        I: IntoIterator<Item = i32>,
    {
        self.local_players = Rc::new(players.into_iter().collect());
        self
    }

    /// Override the fixture-compatible numeric player order with the native
    /// C4PlayerList order. Ignore stale/duplicate IDs and append any players
    /// omitted by the caller deterministically so every registered player is
    /// still visible to indexed script functions.
    pub(crate) fn with_player_order<I>(mut self, players: I) -> Self
    where
        I: IntoIterator<Item = i32>,
    {
        let player_order = Rc::make_mut(&mut self.player_order);
        player_order.clear();
        for id in players {
            if self.players.contains_key(&id) && !player_order.contains(&id) {
                player_order.push(id);
            }
        }

        let fallback_start = player_order.len();
        for id in self.players.keys().copied() {
            if !player_order.contains(&id) {
                player_order.push(id);
            }
        }
        player_order[fallback_start..].sort_unstable();
        self
    }

    pub(crate) fn with_active_message_board_input(
        mut self,
        input: Option<crate::ActiveMessageBoardInput>,
    ) -> Self {
        self.active_message_board_input = input;
        self
    }

    pub(crate) fn with_control_key_names(
        mut self,
        names: Rc<HashMap<i32, Vec<crate::ControlKeyName>>>,
    ) -> Self {
        self.control_key_names = names;
        self
    }

    pub(crate) fn active_message_board_input(&self) -> Option<&crate::ActiveMessageBoardInput> {
        self.active_message_board_input.as_ref()
    }

    pub(crate) fn with_player_info_ids<I>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = i32>,
    {
        let mut known = self.player_info_ids.as_ref().clone();
        known.extend(ids.into_iter().filter(|id| *id != 0));
        self.player_info_ids = Rc::new(known);
        self
    }

    pub(crate) fn player_info_id_known(&self, id: i32) -> bool {
        self.player_info_ids.contains(&id)
    }

    pub(crate) fn with_league_progress_data(
        mut self,
        league_name: Rc<Vec<u8>>,
        progress_data: Rc<BTreeMap<i32, Option<Vec<u8>>>>,
    ) -> Self {
        let mut known = self.player_info_ids.as_ref().clone();
        known.extend(progress_data.keys().copied().filter(|id| *id != 0));
        self.player_info_ids = Rc::new(known);
        self.league_name = league_name;
        self.player_info_league_progress_data = progress_data;
        self
    }

    pub(crate) fn league_name_configured(&self) -> bool {
        !self.league_name.is_empty()
    }

    pub(crate) fn player_info_league_progress_data(&self, id: i32) -> Option<&Option<Vec<u8>>> {
        (id != 0)
            .then(|| self.player_info_league_progress_data.get(&id))
            .flatten()
    }

    pub(crate) fn set_player_info_league_progress_data(
        &mut self,
        id: i32,
        data: Option<Vec<u8>>,
    ) -> bool {
        if id == 0 || !self.player_info_ids.contains(&id) {
            return false;
        }
        Rc::make_mut(&mut self.player_info_league_progress_data).insert(id, data);
        true
    }

    pub(crate) fn with_league_scores(mut self, scores: Rc<BTreeMap<i32, i32>>) -> Self {
        let mut known = self.player_info_ids.as_ref().clone();
        known.extend(scores.keys().copied().filter(|id| *id > 0));
        self.player_info_ids = Rc::new(known);
        self.player_info_league_scores = Rc::new(
            scores
                .iter()
                .filter_map(|(&id, &score)| (id > 0 && score != 0).then_some((id, score)))
                .collect(),
        );
        self
    }

    pub(crate) fn player_info_league_score(&self, id: i32) -> Option<i32> {
        (id >= 1 && self.player_info_ids.contains(&id)).then(|| {
            self.player_info_league_scores
                .get(&id)
                .copied()
                .unwrap_or(0)
        })
    }

    pub(crate) fn with_teams(mut self, teams: Rc<Vec<TeamInfo>>) -> Self {
        self.teams = teams;
        self
    }

    pub(crate) fn with_team_runtime_options(
        mut self,
        team_configuration: TeamConfiguration,
        league_game: bool,
    ) -> Self {
        self.team_configuration = team_configuration;
        self.league_game = league_game;
        self
    }

    pub(crate) fn teams(&self) -> &[TeamInfo] {
        self.teams.as_slice()
    }

    pub(crate) fn league_game(&self) -> bool {
        self.league_game
    }

    pub(crate) fn with_game_tick_delay(
        mut self,
        delay: Rc<Cell<u64>>,
        revision: Rc<Cell<u64>>,
    ) -> Self {
        self.game_tick_delay_ms = delay;
        self.game_tick_delay_revision = revision;
        self
    }

    fn restart_game_tick_delay_ms(&self, delay: u64) {
        self.game_tick_delay_ms.set(delay);
        self.game_tick_delay_revision
            .set(crate::next_game_tick_delay_revision());
    }

    pub(crate) fn auto_generate_teams(&self) -> bool {
        self.team_configuration.auto_generate_teams
    }

    pub(crate) fn team_colors(&self) -> bool {
        self.team_configuration.team_colors
    }

    pub(crate) fn team_config_value(&self, query: i32) -> Option<i32> {
        self.team_configuration.script_value(query)
    }

    /// Attach the scenario script for GameCall/GameCallEx resolution.
    pub(crate) fn with_scenario_script(mut self, script: Option<Arc<ScriptEngine>>) -> Self {
        self.scenario_script = script;
        self
    }

    pub(crate) fn with_network_game(mut self, network_game: bool) -> Self {
        self.network_game = network_game;
        self
    }

    pub(crate) fn with_network_control_mode(mut self, network_control_mode: bool) -> Self {
        self.network_control_mode = network_control_mode;
        self
    }

    pub(crate) fn with_control_sync_mode(mut self, control_sync_mode: bool) -> Self {
        self.control_sync_mode = control_sync_mode;
        self
    }

    pub(crate) fn with_edit_cursor_target(mut self, target: Option<ObjectId>) -> Self {
        self.edit_cursor_target = target;
        self
    }

    pub(crate) fn with_pause_game_requests(
        mut self,
        replay_control: bool,
        requests: Rc<RefCell<Vec<PauseGameRequest>>>,
    ) -> Self {
        self.replay_control = replay_control;
        self.pause_game_requests = requests;
        self
    }

    pub(crate) fn with_network_target_fps_requests(
        mut self,
        requests: Rc<RefCell<Vec<crate::NetworkTargetFpsRequest>>>,
    ) -> Self {
        self.network_target_fps_requests = requests;
        self
    }

    pub(crate) fn with_viewport_presentation_requests(
        mut self,
        replay_control: bool,
        requests: Rc<RefCell<Vec<crate::ViewportPresentationRequest>>>,
    ) -> Self {
        self.replay_control = replay_control;
        self.viewport_presentation_requests = requests;
        self
    }

    pub(crate) fn with_film_viewport_available(mut self, available: bool) -> Self {
        self.film_viewport_available = available;
        self
    }

    pub(crate) fn with_smoke_level(mut self, smoke_level: i32) -> Self {
        self.smoke_level = smoke_level;
        self
    }

    pub(crate) fn smoke_level(&self) -> i32 {
        self.smoke_level
    }

    pub(crate) fn with_max_players(mut self, max_players: i32) -> Self {
        self.max_players = max_players;
        self
    }

    pub(crate) fn max_players(&self) -> i32 {
        self.max_players
    }

    pub(crate) fn set_max_players(&mut self, max_players: i32) {
        self.max_players = max_players;
    }

    pub(crate) fn with_fair_crew_parameters(
        mut self,
        use_fair_crew: bool,
        fair_crew_strength: i32,
    ) -> Self {
        self.use_fair_crew = use_fair_crew;
        self.fair_crew_strength = fair_crew_strength;
        self
    }

    pub(crate) fn with_fair_crew_physical_cache(
        mut self,
        cache: crate::FairCrewPhysicalCache,
    ) -> Self {
        self.fair_crew_physical_cache = cache;
        self
    }

    pub(crate) fn with_control_host(
        mut self,
        control_host: bool,
        updates: Rc<RefCell<Vec<crate::PlayerInfoUpdateRequest>>>,
    ) -> Self {
        self.control_host = control_host;
        self.player_info_updates = updates;
        self
    }

    pub(crate) fn with_scenario_script_counter(mut self, counter: i32) -> Self {
        self.scenario_script_counter = counter;
        self
    }

    pub(crate) fn scenario_script_counter(&self) -> i32 {
        self.scenario_script_counter
    }

    pub(crate) fn network_game(&self) -> bool {
        self.network_game
    }

    pub(crate) fn network_control_mode(&self) -> bool {
        self.network_control_mode
    }

    pub(crate) fn scenario_script(&self) -> Option<&Arc<ScriptEngine>> {
        self.scenario_script.as_ref()
    }

    /// Attach the engine's compiled definition scripts for nested script
    /// calls. See the `definition_scripts` field docs.
    pub(crate) fn with_definition_metadata(
        mut self,
        definitions: Rc<HashMap<DefinitionId, DefinitionMetadata>>,
    ) -> Self {
        self.definitions = definitions;
        self
    }

    pub(crate) fn definition_color_by_owner(&self, id: &str) -> bool {
        self.color_by_owner_definitions.contains(id)
    }

    pub(crate) fn definition_base_auto_sell(&self, id: &str) -> bool {
        self.base_auto_sell_definitions.contains(id)
    }

    pub(crate) fn definition_rebuyable(&self, id: &str) -> bool {
        self.rebuyable_definitions.contains(id)
    }

    pub(crate) fn definition_no_sell(&self, id: &str) -> bool {
        self.no_sell_definitions.contains(id)
    }

    pub(crate) fn with_definition_order(mut self, order: Rc<Vec<DefinitionId>>) -> Self {
        self.definition_order = order;
        self
    }

    pub(crate) fn with_definition_scripts(
        mut self,
        scripts: HashMap<DefinitionId, Arc<ScriptEngine>>,
    ) -> Self {
        self.definition_scripts = Rc::new(scripts);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_linked_script_hosts(
        mut self,
        scripts: Vec<(String, Arc<ScriptEngine>)>,
    ) -> Self {
        self.linked_script_hosts = Rc::new(scripts);
        self
    }

    pub(crate) fn definition_script(&self, id: &str) -> Option<&Arc<ScriptEngine>> {
        self.definition_scripts.get(id)
    }

    pub(crate) fn definition_scripts(&self) -> impl Iterator<Item = &Arc<ScriptEngine>> {
        self.definition_scripts.values()
    }

    /// True when any engine script function of this name declares `&` at the
    /// zero-based slot (C4AulParse.cpp:2318-2331 `anyfunctakesref`).
    pub(crate) fn function_takes_reference_at(&self, name: &str, slot: usize) -> bool {
        slot < u32::BITS as usize
            && self
                .reference_parameter_slots
                .get(name)
                .is_some_and(|mask| mask & (1 << slot) != 0)
    }

    /// Resolve the local-lookup script host of the suspended VM frame. This
    /// follows a local function's Owner or a global function's LinkedTo host,
    /// never the mutable definition of `cthr->Obj`.
    pub(crate) fn script_for_host_identity(
        &self,
        identity: clonk_script::ScriptHostIdentity,
    ) -> Option<(String, Option<DefinitionId>, Arc<ScriptEngine>)> {
        if let Some(script) = self
            .scenario_script
            .as_ref()
            .filter(|script| script.host_identity() == identity)
        {
            return Some(("Game.Script".to_string(), None, Arc::clone(script)));
        }
        if let Some((name, script)) = self
            .linked_script_hosts
            .iter()
            .find(|(_, script)| script.host_identity() == identity)
        {
            return Some((name.clone(), None, Arc::clone(script)));
        }
        let mut matches = self
            .definition_scripts
            .iter()
            .filter(|(_, script)| script.host_identity() == identity)
            .collect::<Vec<_>>();
        matches.sort_by_key(|(definition, _)| *definition);
        matches.first().map(|(definition, script)| {
            (
                (*definition).clone(),
                Some((*definition).clone()),
                Arc::clone(script),
            )
        })
    }

    /// Resolve a function strictly through `Game.ScriptEngine`, then return
    /// the retained script host named by the global function's `LinkedTo`
    /// pointer. Calling through an arbitrary host that merely shares the
    /// global table would lose declaring-host local-helper lookup.
    pub(crate) fn resolve_engine_global_script(
        &self,
        name: &str,
    ) -> Option<(Arc<ScriptEngine>, clonk_script::ScriptFunctionResolution)> {
        let resolve = |script: &Arc<ScriptEngine>| {
            let resolution = script.resolve_global_function(name)?;
            let exact_script = self
                .script_for_host_identity(resolution.host_identity)
                .map(|(_, _, script)| script)
                .or_else(|| {
                    (script.host_identity() == resolution.host_identity).then(|| Arc::clone(script))
                })?;
            Some((exact_script, resolution))
        };

        if let Some(resolved) = self.scenario_script.as_ref().and_then(resolve) {
            return Some(resolved);
        }
        for (_, script) in self.linked_script_hosts.iter() {
            if let Some(resolved) = resolve(script) {
                return Some(resolved);
            }
        }
        let mut definitions = self.definition_scripts.iter().collect::<Vec<_>>();
        definitions.sort_by_key(|(definition, _)| *definition);
        definitions
            .into_iter()
            .find_map(|(_, script)| resolve(script))
    }

    /// Native functions owned by `Game.ScriptEngine` are registered on each
    /// live Rust script host. Select one deterministically only after strict
    /// global-script lookup has failed.
    pub(crate) fn resolve_engine_host_script(&self, name: &str) -> Option<Arc<ScriptEngine>> {
        if let Some(script) = self
            .scenario_script
            .as_ref()
            .filter(|script| script.has_host_function(name))
        {
            return Some(Arc::clone(script));
        }
        if let Some((_, script)) = self
            .linked_script_hosts
            .iter()
            .find(|(_, script)| script.has_host_function(name))
        {
            return Some(Arc::clone(script));
        }
        let mut definitions = self.definition_scripts.iter().collect::<Vec<_>>();
        definitions.sort_by_key(|(definition, _)| *definition);
        definitions
            .into_iter()
            .find_map(|(_, script)| script.has_host_function(name).then(|| Arc::clone(script)))
    }

    /// Whether any definition script, global script, or host function knows
    /// `name` — the global-function-map lookup of `GetFirstFunc`
    /// (C4Aul.cpp:545-552).
    pub(crate) fn script_function_known(&self, name: &str) -> bool {
        self.definition_scripts
            .values()
            .chain(self.linked_script_hosts.iter().map(|(_, script)| script))
            .any(|script| {
                script.has_function(name)
                    || script.has_global_function(name)
                    || script.has_host_function(name)
            })
    }

    /// Attach the engine's particle def registry (names from
    /// `C4ParticleSystem` defs). See the `particle_defs` field docs.
    pub(crate) fn with_particle_defs(mut self, defs: std::collections::HashSet<String>) -> Self {
        self.particle_defs = Some(Rc::new(defs));
        self
    }

    /// Attach the material table (FnMaterial name lookups).
    pub(crate) fn with_materials(mut self, materials: Option<Rc<MaterialSet>>) -> Self {
        self.materials = materials;
        self
    }

    pub(crate) fn materials(&self) -> Option<&MaterialSet> {
        self.materials.as_deref()
    }

    /// Attach the engine's crew-info rank table (see `crew_ranks` docs).
    pub(crate) fn with_crew_ranks(mut self, ranks: Rc<HashMap<u64, i32>>) -> Self {
        self.crew_ranks = ranks;
        self
    }

    pub(crate) fn with_crew_infos(mut self, infos: Rc<HashMap<ObjectId, CrewObjectInfo>>) -> Self {
        self.crew_infos = infos;
        self
    }

    pub(crate) fn with_crew_info_links(
        mut self,
        links: Rc<HashMap<ObjectId, CrewInfoLink>>,
    ) -> Self {
        self.crew_info_links = links;
        self
    }

    /// The crew object's Info rank; `None` for info-less objects.
    pub(crate) fn crew_rank(&self, object: u64) -> Option<i32> {
        self.crew_ranks.get(&object).copied()
    }

    pub(crate) fn crew_info_link(&self, object: ObjectId) -> Option<CrewInfoLink> {
        self.crew_info_links.get(&object).copied()
    }

    /// `Some(known?)` when a registry is attached, `None` otherwise.
    pub(crate) fn particle_def_known(&self, name: &str) -> Option<bool> {
        self.particle_defs.as_ref().map(|defs| defs.contains(name))
    }

    /// `C4Id2Def` visibility: `Some(known?)` when the engine attached a
    /// definition table, `None` for legacy fixture contexts (empty table
    /// stays permissive like particle_def_known).
    pub(crate) fn definition_known(&self, id: &str) -> Option<bool> {
        (!self.definitions.is_empty()).then(|| self.definitions.contains_key(id))
    }

    fn materialize_objects(&self) {
        if self.object_store.borrow().complete {
            return;
        }
        let Some(provider) = self.lazy_world else {
            Rc::make_mut(&mut self.object_store.borrow_mut()).complete = true;
            return;
        };
        let excluded = self
            .object_store
            .borrow()
            .indices
            .values()
            .copied()
            .collect::<HashSet<_>>();
        let materialized = provider.objects(&excluded);
        let mut store = self.object_store.borrow_mut();
        let store = Rc::make_mut(&mut store);
        for (index, object) in materialized {
            let id = object.id;
            if store.removed.contains(&id) || store.objects.contains_key(&id) {
                continue;
            }
            store.indices.insert(id, index);
            store.objects.insert(id, object);
        }
        store.order = store.objects.keys().copied().collect();
        store
            .order
            .sort_by_key(|id| store.indices.get(id).copied().unwrap_or(usize::MAX));
        store.complete = true;
    }

    pub(crate) fn get(&self, id: ObjectId) -> Option<HostWorldObject> {
        {
            let store = self.object_store.borrow();
            if store.removed.contains(&id) {
                return None;
            }
            if let Some(object) = store.objects.get(&id) {
                return Some(object.clone());
            }
            if store.complete {
                return None;
            }
        }
        let (index, object) = self.lazy_world?.object(id)?;
        let mut store = self.object_store.borrow_mut();
        let store = Rc::make_mut(&mut store);
        if store.removed.contains(&id) {
            return None;
        }
        store.indices.insert(id, index);
        store.objects.insert(id, object.clone());
        store.insert_ordered_by_index(id, index);
        Some(object)
    }

    pub(crate) fn matches_legacy_find_object_candidate(
        &self,
        id: ObjectId,
        params: &FindObjectParams,
    ) -> Option<bool> {
        {
            let store = self.object_store.borrow();
            if store.removed.contains(&id) {
                return None;
            }
            if let Some(object) = store.objects.get(&id) {
                return Some(params.matches_object(object));
            }
            if store.complete {
                return None;
            }
        }
        let Some(provider) = self.lazy_world else {
            return self.get(id).map(|object| params.matches_object(&object));
        };
        let Some(matches) = provider.legacy_find_object else {
            return self.get(id).map(|object| params.matches_object(&object));
        };
        // SAFETY: the same synchronous source-lifetime and object-storage
        // contract as `LazyHostWorldProvider::object` applies. This callback
        // only reads the scalar fields C4Game::FindObject itself inspects.
        unsafe { matches(provider.source, id, params) }
    }

    /// Update the callback-visible identity of a live command target while
    /// one engine-owned effect batch is still running. C++
    /// OnObjectChangedDef refreshes effect callback functions immediately;
    /// a cloned HostWorldContext otherwise keeps resolving the old script.
    pub(crate) fn preview_object_change_def(&mut self, id: ObjectId, definition_id: &str) {
        let _ = self.get(id);
        let store = Rc::make_mut(self.object_store.get_mut());
        if let Some(object) = store.objects.get_mut(&id) {
            object.definition_id = definition_id.to_string();
            object.unsorted = true;
        }
    }

    /// Carry mask-driving foreign-object writes between callbacks in one
    /// deferred effect batch. C++ mutates the live object immediately; a
    /// cloned host world must therefore seed the next callback from the
    /// preceding callback's final geometry and graphics state.
    pub(crate) fn preview_object_update(&mut self, id: ObjectId, update: &ObjectUpdate) {
        if let Some(definition_id) = update.change_def.as_deref() {
            self.preview_object_change_def(id, definition_id);
        }
        let _ = self.get(id);
        let store = Rc::make_mut(self.object_store.get_mut());
        let Some(object) = store.objects.get_mut(&id) else {
            return;
        };
        if let Some(position) = update.position {
            object.position = position;
            object.fixed_position = FixedVec2::from_ints(position.x, position.y);
        }
        if let Some(position) = update.resolved_docon_position {
            object.position = position;
        }
        if let Some(position) = update.resolved_docon_fixed_position {
            object.fixed_position = position;
        }
        if let Some(rotation) = update.rotation {
            object.rotation = rotation;
            object.fixed_rotation = itofix(rotation);
        }
        if let Some(construction) = update.construction {
            object.construction = construction.max(0);
        }
        if let Some(container) = update.container {
            object.container = container;
        }
        if let Some(status) = update.status {
            object.status = status;
        }
        if let Some(material_contents) = update.material_contents.as_ref() {
            object.material_contents = material_contents.clone();
        }

        if let Some(state) = object.state.as_mut() {
            let state = Rc::make_mut(state);
            if update.change_def.is_some() {
                state.solid_mask_override = None;
            }
            if let Some(position) = update.position {
                state.position = position;
            }
            if let Some(position) = update.resolved_docon_position {
                state.position = position;
            }
            if let Some(rotation) = update.rotation {
                state.rotation = rotation;
            }
            if let Some(construction) = update.construction {
                state.construction = construction.max(0);
            }
            if let Some(container) = update.container {
                state.container = container;
            }
            if let Some(layer) = update.layer {
                state.layer = layer;
            }
            if let Some(status) = update.status {
                state.status = status;
            }
            if let Some(mask) = update.solid_mask_override {
                state.solid_mask_override = Some(mask);
            }
            if let Some(base_graphics) = update.base_graphics.clone() {
                state.base_graphics = base_graphics;
            }
            if let Some(shape_override) = update.shape_override {
                state.shape_override = shape_override;
            }
            if let Some(local_vars) = update.local_vars.as_ref() {
                state.local_vars = local_vars.clone();
            }
        }
    }

    /// Carry one callback-final raw contents list into the threaded preview
    /// used by a later callback in the same effect batch. C++ mutates these
    /// links synchronously; the copied host world otherwise sees only the
    /// child's updated `Contained` pointer.
    pub(crate) fn preview_contents_order(&mut self, container: ObjectId, contents: &[ObjectId]) {
        let _ = self.get(container);
        let store = Rc::make_mut(self.object_store.get_mut());
        let Some(object) = store.objects.get_mut(&container) else {
            return;
        };
        object.contents = contents.to_vec();
        if let Some(state) = object.state.as_mut() {
            Rc::make_mut(state).contents = contents.to_vec();
        }
    }

    pub(crate) fn preview_object_destroyed(&mut self, id: ObjectId) {
        let store = Rc::make_mut(self.object_store.get_mut());
        store.objects.remove(&id);
        // Keep the storage index as a provider exclusion even after the
        // callback-private object is tombstoned. The engine may still hold
        // this entry through an exclusive borrow for the rest of the call.
        store.order.retain(|object_id| *object_id != id);
        store.removed.insert(id);
        Rc::make_mut(&mut self.master_order).retain(|object_id| *object_id != id);
        self.solid_mask_instance_sequences.borrow_mut().remove(&id);
    }

    pub(crate) fn object_ids(&self) -> Vec<ObjectId> {
        self.materialize_objects();
        self.object_store.borrow().order.clone()
    }

    pub(crate) fn master_object_ids(&self) -> &[ObjectId] {
        self.master_order.as_ref().as_slice()
    }

    pub(crate) fn with_master_order<I>(mut self, order: I) -> Self
    where
        I: IntoIterator<Item = ObjectId>,
    {
        self.master_order = Rc::new(order.into_iter().collect());
        self
    }

    /// Attach an exact callback-entry snapshot of `C4LSectors`. Its rank
    /// oracle and physical per-sector vectors can legitimately disagree
    /// after a native SortByCategory, so reconstructing it from ids loses
    /// observable ordering state.
    pub(crate) fn with_sector_map(mut self, sectors: Option<SectorMap>) -> Self {
        self.sectors = RefCell::new(sectors.map(Rc::new));
        self
    }

    fn landscape_slot(&self) -> &Option<Rc<Landscape>> {
        self.landscape.get_or_init(|| {
            self.lazy_world
                .and_then(LazyHostWorldProvider::landscape)
                .map(Rc::new)
        })
    }

    fn ensure_landscape_initialized(&mut self) {
        if self.landscape.get().is_none() {
            let landscape = self
                .lazy_world
                .and_then(LazyHostWorldProvider::landscape)
                .map(Rc::new);
            let _ = self.landscape.set(landscape);
        }
    }

    fn landscape_slot_mut(&mut self) -> &mut Option<Rc<Landscape>> {
        self.ensure_landscape_initialized();
        self.landscape
            .get_mut()
            .expect("landscape slot initialized above")
    }

    pub(crate) fn landscape_ref(&self) -> Option<&Landscape> {
        self.landscape_slot().as_deref()
    }

    pub(crate) fn landscape_shared(&self) -> Option<Rc<Landscape>> {
        self.landscape_slot().clone()
    }

    pub(crate) fn landscape_mut(&mut self) -> Option<&mut Landscape> {
        self.landscape_slot_mut().as_mut().map(Rc::make_mut)
    }

    /// Thread state-bearing landscape operations across effect callbacks
    /// that execute before the authoritative Engine fold. Pixel mutations,
    /// texture-map allocations, and retained map-creator state are all live.
    pub(crate) fn preview_runtime_landscape_operation(&mut self, operation: &LandscapeOperation) {
        match operation {
            LandscapeOperation::DrawMap {
                origin,
                bitmap,
                map_width,
                map_height,
                texmap,
                map_creator,
            } => {
                self.ensure_landscape_initialized();
                let Some(landscape) = self
                    .landscape
                    .get_mut()
                    .and_then(Option::as_mut)
                    .map(Rc::make_mut)
                else {
                    return;
                };
                let bakes = Rc::make_mut(&mut self.solid_mask_bakes);
                let _ = landscape.preview_draw_indexed_map_with_masks(
                    bakes,
                    *origin,
                    bitmap,
                    *map_width,
                    *map_height,
                    texmap.clone(),
                );
                if let Some(map_creator) = map_creator {
                    let _ = landscape.replace_runtime_map_creator_state(map_creator.0.clone());
                }
            }
            LandscapeOperation::SyncRuntimeTexMap { texmap } => {
                let Some(landscape) = self.landscape_slot_mut().as_mut().map(Rc::make_mut) else {
                    return;
                };
                let _ = landscape.replace_runtime_texmap_state(texmap.clone());
            }
            LandscapeOperation::SetTextureIndex {
                texmap,
                old_index,
                new_index,
            } => {
                let Some(landscape) = self.landscape_slot_mut().as_mut().map(Rc::make_mut) else {
                    return;
                };
                let _ = landscape.apply_runtime_texture_index_move(
                    texmap.clone(),
                    *old_index,
                    *new_index,
                );
            }
            LandscapeOperation::RemoveUnusedTexMapEntries { cleared_slots } => {
                let Some(landscape) = self.landscape_slot_mut().as_mut().map(Rc::make_mut) else {
                    return;
                };
                let _ = landscape.clear_runtime_texmap_entries(cleared_slots);
            }
            LandscapeOperation::ClearRect {
                origin,
                width,
                height,
            } => {
                let materials = self.materials.clone().unwrap_or_default();
                self.ensure_landscape_initialized();
                let Some(landscape) = self
                    .landscape
                    .get_mut()
                    .and_then(Option::as_mut)
                    .map(Rc::make_mut)
                else {
                    return;
                };
                let bounds =
                    crate::landscape::RasterChangeRect::new(origin.x, origin.y, *width, *height);
                if landscape.pixel_grid().is_some() {
                    let bakes = Rc::make_mut(&mut self.solid_mask_bakes);
                    landscape.preview_raster_transaction_with_masks(bakes, bounds, |landscape| {
                        landscape.clear_rect_pixels(bounds)
                    });
                } else {
                    let landscape_height = landscape.estimated_height();
                    for row in origin.y..origin.y.saturating_add(*height) {
                        crate::Engine::mutate_clear_rect_landscape_row(
                            landscape,
                            materials.as_ref(),
                            origin.x,
                            row,
                            *width,
                            None,
                            landscape_height,
                        );
                    }
                }
            }
            LandscapeOperation::ClearRectDensity {
                origin,
                width,
                height,
                density,
            } => {
                let materials = self.materials.clone().unwrap_or_default();
                let Some(landscape) = self.landscape_slot_mut().as_mut().map(Rc::make_mut) else {
                    return;
                };
                let landscape_height = landscape.estimated_height();
                for row in origin.y..origin.y.saturating_add(*height) {
                    crate::Engine::mutate_clear_rect_landscape_row(
                        landscape,
                        materials.as_ref(),
                        origin.x,
                        row,
                        *width,
                        Some(*density),
                        landscape_height,
                    );
                }
            }
            LandscapeOperation::DrawMaterialQuad {
                material_texture,
                vertices,
                ift,
            } => {
                self.ensure_landscape_initialized();
                let Some(landscape) = self
                    .landscape
                    .get_mut()
                    .and_then(Option::as_mut)
                    .map(Rc::make_mut)
                else {
                    return;
                };
                let bakes = Rc::make_mut(&mut self.solid_mask_bakes);
                let _ = landscape.preview_draw_material_quad_with_masks(
                    bakes,
                    material_texture,
                    *vertices,
                    *ift,
                );
            }
            LandscapeOperation::DrawMatChunks {
                origin,
                width,
                height,
                count_x,
                count_y,
                material,
                byte,
                map_seed,
                random_offsets,
                texmap,
            } => {
                self.ensure_landscape_initialized();
                let Some(landscape) = self
                    .landscape
                    .get_mut()
                    .and_then(Option::as_mut)
                    .map(Rc::make_mut)
                else {
                    return;
                };
                let bakes = Rc::make_mut(&mut self.solid_mask_bakes);
                let _ = landscape.preview_draw_material_chunks_with_masks(
                    bakes,
                    *origin,
                    *width,
                    *height,
                    *count_x,
                    *count_y,
                    material,
                    *byte,
                    *map_seed,
                    random_offsets,
                    texmap.clone(),
                );
            }
            LandscapeOperation::DrawVolcanoBranch {
                from,
                to,
                size,
                material_byte,
            } => {
                let Some(landscape) = self.landscape_slot_mut().as_mut().map(Rc::make_mut) else {
                    return;
                };
                let _ = landscape.draw_volcano_branch(*from, *to, *size, *material_byte);
            }
            LandscapeOperation::DrawDefMap {
                origin,
                bitmap,
                map_width,
                map_height,
                texmap,
                map_creator,
            } => {
                self.ensure_landscape_initialized();
                let Some(landscape) = self
                    .landscape
                    .get_mut()
                    .and_then(Option::as_mut)
                    .map(Rc::make_mut)
                else {
                    return;
                };
                let bakes = Rc::make_mut(&mut self.solid_mask_bakes);
                let _ = landscape.preview_draw_indexed_map_with_masks(
                    bakes,
                    *origin,
                    bitmap,
                    *map_width,
                    *map_height,
                    texmap.clone(),
                );
                let _ = landscape.replace_runtime_map_creator_state(map_creator.0.clone());
            }
            LandscapeOperation::DigCircle { center, radius, .. } => {
                let materials = self.materials.clone().unwrap_or_default();
                if let Some(landscape) = self.landscape_mut() {
                    preview_dig_circle_pixels(landscape, materials.as_ref(), *center, *radius);
                }
            }
            LandscapeOperation::DigCirclePreviewed { center, radius } => {
                let materials = self.materials.clone().unwrap_or_default();
                if let Some(landscape) = self.landscape_mut() {
                    preview_dig_circle_pixels(landscape, materials.as_ref(), *center, *radius);
                }
            }
            LandscapeOperation::DigRect {
                origin,
                width,
                height,
                ..
            } => {
                let materials = self.materials.clone().unwrap_or_default();
                if let Some(landscape) = self.landscape_mut() {
                    preview_dig_rect_pixels(
                        landscape,
                        materials.as_ref(),
                        *origin,
                        *width,
                        *height,
                    );
                }
            }
            LandscapeOperation::DigRectPreviewed {
                origin,
                width,
                height,
            } => {
                let materials = self.materials.clone().unwrap_or_default();
                if let Some(landscape) = self.landscape_mut() {
                    preview_dig_rect_pixels(
                        landscape,
                        materials.as_ref(),
                        *origin,
                        *width,
                        *height,
                    );
                }
            }
            LandscapeOperation::ShakeCircle { center, radius } => {
                let materials = self.materials.clone().unwrap_or_default();
                if let Some(landscape) = self.landscape_mut() {
                    preview_shake_circle_pixels(landscape, materials.as_ref(), *center, *radius);
                }
            }
            LandscapeOperation::BlastCirclePreviewed {
                center,
                radius,
                replay,
            } => {
                let materials = self.materials.clone().unwrap_or_default();
                if let Some(landscape) = self.landscape_mut() {
                    preview_captured_blast_pixels(
                        landscape,
                        materials.as_ref(),
                        *center,
                        *radius,
                        &replay.pixels,
                    );
                }
            }
            LandscapeOperation::MatAdjust { modulation } => {
                if let Some(landscape) = self.landscape_slot_mut().as_mut().map(Rc::make_mut) {
                    landscape.set_modulation(*modulation);
                }
            }
            _ => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn with_scenario_values(mut self, values: Rc<ScenarioValueStore>) -> Self {
        self.scenario_values = values;
        self
    }

    pub(crate) fn with_scenario_sections<I, S>(mut self, sections: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.scenario_sections = Rc::new(
            sections
                .into_iter()
                .map(|name| name.as_ref().to_ascii_lowercase())
                .collect(),
        );
        self
    }

    pub(crate) fn scenario_section_known(&self, name: &str) -> bool {
        self.scenario_sections
            .contains(name.to_ascii_lowercase().as_str())
    }

    pub(crate) fn scenario_value(
        &self,
        entry: &str,
        section: Option<&str>,
        entry_nr: i32,
    ) -> Option<&ScenarioValue> {
        self.scenario_values.get(entry, section, entry_nr)
    }

    pub(crate) fn landscape_push_pull(&self) -> bool {
        self.scenario_values.landscape_push_pull()
    }

    pub(crate) fn with_movement_solid_masks(mut self, masks: Vec<crate::SolidMaskRect>) -> Self {
        self.movement_solid_masks = Rc::new(masks);
        self
    }

    pub(crate) fn with_solid_mask_metadata(
        mut self,
        metadata: Rc<HashMap<DefinitionId, HostSolidMaskMetadata>>,
    ) -> Self {
        self.solid_mask_metadata = metadata;
        self
    }

    pub(crate) fn with_solid_mask_bakes(
        mut self,
        bakes: Vec<(ObjectId, crate::SolidMaskBake)>,
    ) -> Self {
        self.solid_mask_bakes = Rc::new(bakes);
        self
    }

    pub(crate) fn with_solid_mask_instance_sequences(
        mut self,
        sequences: HashMap<ObjectId, u64>,
        next_sequence: u64,
    ) -> Self {
        self.solid_mask_instance_sequences = Rc::new(RefCell::new(sequences));
        self.next_solid_mask_instance_sequence = Rc::new(Cell::new(next_sequence));
        self
    }

    /// Thread synchronous mask raster state between callbacks that otherwise
    /// receive independent copy-on-write HostWorldContext clones.
    pub(crate) fn preview_solid_mask_operations(
        &mut self,
        operations: &[crate::HostSolidMaskOperation],
    ) {
        if operations.is_empty() {
            return;
        }
        for operation in operations {
            if let crate::HostSolidMaskOperation::Landscape { operation } = operation {
                self.preview_runtime_landscape_operation(operation);
                continue;
            }
            let object_id = match operation {
                crate::HostSolidMaskOperation::Remove { object_id }
                | crate::HostSolidMaskOperation::Put { object_id, .. } => *object_id,
                crate::HostSolidMaskOperation::Landscape { .. } => unreachable!(),
            };
            self.ensure_landscape_initialized();
            let Some(landscape) = self
                .landscape
                .get_mut()
                .and_then(Option::as_mut)
                .map(Rc::make_mut)
            else {
                continue;
            };
            let bakes = Rc::make_mut(&mut self.solid_mask_bakes);
            let previous = remove_host_solid_mask_raster(landscape, bakes, object_id);
            match operation {
                crate::HostSolidMaskOperation::Remove { .. } => {
                    self.solid_mask_instance_sequences
                        .borrow_mut()
                        .remove(&object_id);
                }
                crate::HostSolidMaskOperation::Put {
                    spec,
                    position,
                    instance_sequence,
                    ..
                } => {
                    self.solid_mask_instance_sequences
                        .borrow_mut()
                        .insert(object_id, *instance_sequence);
                    self.next_solid_mask_instance_sequence.set(
                        self.next_solid_mask_instance_sequence.get().max(
                            instance_sequence
                                .checked_add(1)
                                .expect("C4SolidMask instance sequence overflow"),
                        ),
                    );
                    if let Some(bake) = crate::put_solid_mask_raster(
                        landscape,
                        spec.clone(),
                        *position,
                        *instance_sequence,
                    ) {
                        let insert_at = previous
                            .map(|(index, _)| index)
                            .unwrap_or(bakes.len())
                            .min(bakes.len());
                        bakes.insert(insert_at, (object_id, bake));
                    }
                }
                crate::HostSolidMaskOperation::Landscape { .. } => unreachable!(),
            }
        }
    }

    pub(crate) fn apply_host_raster_preview(&mut self, preview: HostRasterPreview) {
        self.landscape = OnceCell::from(preview.landscape.map(Rc::new));
        self.solid_mask_bakes = Rc::new(preview.solid_mask_bakes);
        self.solid_mask_instance_sequences =
            Rc::new(RefCell::new(preview.solid_mask_instance_sequences));
        self.next_solid_mask_instance_sequence =
            Rc::new(Cell::new(preview.next_solid_mask_instance_sequence));
    }

    pub(crate) fn host_raster_preview(&self) -> HostRasterPreview {
        HostRasterPreview {
            landscape: self.landscape_ref().cloned(),
            solid_mask_bakes: self.solid_mask_bakes.as_ref().clone(),
            solid_mask_instance_sequences: self.solid_mask_instance_sequences.borrow().clone(),
            next_solid_mask_instance_sequence: self.next_solid_mask_instance_sequence.get(),
        }
    }

    /// Refresh the parts of a movement callback's world view changed by
    /// C4Object::DoMotion removing its solid mask. Contact callbacks after
    /// the first committed pixel must query the restored landscape rather
    /// than the snapshot from DoMovement entry.
    pub(crate) fn refresh_after_do_motion(
        &mut self,
        mover: ObjectId,
        landscape: &Landscape,
        bakes: Vec<(ObjectId, crate::SolidMaskBake)>,
    ) {
        // If no terrain host call ran yet, the movement provider already
        // points at this live landscape; preserve laziness instead of cloning
        // it merely because DoMotion advanced.
        if self.landscape.get().is_some() {
            self.landscape = OnceCell::from(Some(Rc::new(landscape.clone())));
        }
        Rc::make_mut(&mut self.movement_solid_masks).retain(|mask| mask.object_id != mover);
        self.solid_mask_bakes = Rc::new(bakes);
    }

    pub(crate) fn movement_density_at(&self, x: i32, y: i32) -> Option<i32> {
        Some(crate::movement_density_at(
            self.landscape_ref()?,
            self.materials()?,
            self.movement_solid_masks.as_slice(),
            None,
            x,
            y,
        ))
    }

    pub(crate) fn transfer_zones(&self) -> &[TransferZoneState] {
        self.transfer_zones.as_ref()
    }

    pub(crate) fn preview_transfer_zone_command(&mut self, command: &TransferZoneCommand) {
        let mut zones = TransferZoneTable::from_states(self.transfer_zones.as_ref());
        match command {
            TransferZoneCommand::Set { owner, rect } => zones.set(*owner, *rect),
            TransferZoneCommand::Clear { owner } => zones.clear(*owner),
        }
        self.transfer_zones = Rc::new(zones.states());
    }

    pub(crate) fn with_pathfinder_settings(
        mut self,
        level: i32,
        transfer_zones_enabled: bool,
    ) -> Self {
        self.set_pathfinder_settings(level, transfer_zones_enabled);
        self
    }

    pub(crate) fn with_pathfinder_debug_sink(
        mut self,
        sink: Rc<RefCell<PathfinderDebugSnapshot>>,
    ) -> Self {
        self.pathfinder_debug = sink;
        self
    }

    pub(crate) fn set_pathfinder_settings(&mut self, level: i32, transfer_zones_enabled: bool) {
        self.pathfinder_level = level.clamp(1, 10);
        self.pathfinder_transfer_zones_enabled = transfer_zones_enabled;
    }

    pub(crate) fn pathfinder_settings(&self) -> (i32, bool) {
        (
            self.pathfinder_level,
            self.pathfinder_transfer_zones_enabled,
        )
    }

    pub(crate) fn next_object_id(&self) -> u64 {
        self.next_object_id
    }

    pub(crate) fn with_next_object_id(mut self, next_object_id: u64) -> Self {
        self.next_object_id = next_object_id;
        self
    }

    pub(crate) fn team_home_base_rule(&self) -> bool {
        self.team_home_base_rule
    }

    pub(crate) fn definition_category(&self, id: &str) -> Option<i32> {
        self.definitions.get(id).map(|meta| meta.category)
    }

    pub(crate) fn definition_id_by_index(
        &self,
        index: i32,
        category: i32,
    ) -> Option<&DefinitionId> {
        let index = usize::try_from(index).ok()?;
        let category = if category == 0 { -1 } else { category };
        if category == -1 {
            return self.definition_order.get(index);
        }
        self.definition_order
            .iter()
            .filter(|id| {
                self.definitions
                    .get(*id)
                    .is_some_and(|metadata| metadata.category & category != 0)
            })
            .nth(index)
    }

    pub(crate) fn definition_metadata(&self, id: &str) -> Option<&DefinitionMetadata> {
        self.definitions.get(id)
    }

    pub(crate) fn definition_rank_base(&self, id: &str) -> Option<i32> {
        self.definition_rank_bases.get(id).copied()
    }

    pub(crate) fn definition_description(&self, id: &str) -> Option<&str> {
        self.definition_descriptions.get(id).map(String::as_str)
    }

    pub(crate) fn object_live_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        host_object_live_shape_rect(object, &self.definitions)
    }

    pub(crate) fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        sector_shape_rect(self.object_live_shape_rect(object))
    }

    /// The sector map over this context's objects, built on first use.
    fn sector_map(&self) -> Option<Rc<SectorMap>> {
        self.materialize_objects();
        let landscape = self.landscape_ref()?;
        let mut cache = self.sectors.borrow_mut();
        if cache.is_none() {
            let store = self.object_store.borrow();
            *cache = Some(Rc::new(build_host_sector_map(
                store.order.iter().filter_map(|id| store.objects.get(id)),
                &self.definitions,
                landscape,
            )));
        }
        cache.clone()
    }

    /// Callback-local C4GameObjects::UpdatePos preview used by
    /// StatusActivate/StatusDeactivate. The HostWorldContext is a copied
    /// call snapshot, so copy-on-write keeps this synchronous list/sector
    /// mutation private until the authoritative ObjectUpdate folds.
    pub(crate) fn preview_object_status_sector(
        &self,
        object: &HostWorldObject,
        master_order: &[ObjectId],
    ) {
        if self.sector_map().is_none() {
            return;
        }
        let mut cache = self.sectors.borrow_mut();
        let Some(sectors) = cache.as_mut() else {
            return;
        };
        let sectors = Rc::make_mut(sectors);
        sectors.remove(object.id);
        sectors.set_master_order(master_order.to_vec());
        if object.status().is_active() {
            if let Some(record) = host_sector_record(object, self.definitions.as_ref()) {
                sectors.add(record);
            }
        }
    }

    /// Callback-local `C4GameObjects::UpdatePos` for a live position/shape
    /// change. Unlike a status transition this retains the object's existing
    /// sector-list links wherever its covered area did not change.
    pub(crate) fn preview_object_sector_update(
        &self,
        record: SectorObject,
        master_order: &[ObjectId],
    ) {
        if self.sector_map().is_none() {
            return;
        }
        let mut cache = self.sectors.borrow_mut();
        let Some(sectors) = cache.as_mut() else {
            return;
        };
        let sectors = Rc::make_mut(sectors);
        sectors.set_master_order(master_order.iter().copied());
        sectors.update(record);
    }

    pub(crate) fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.sector_map().map(|sectors| {
            let area = sectors.area(rect);
            sectors.object_ids_in_area(&area)
        })
    }

    pub(crate) fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.sector_map().map(|sectors| {
            let area = sectors.area(rect);
            sectors.shape_ids_in_area(&area)
        })
    }

    pub(crate) fn object_sector_id_lists_in_rect(
        &self,
        rect: DefinitionRect,
    ) -> Option<Vec<Vec<ObjectId>>> {
        self.sector_map().map(|sectors| {
            let area = sectors.area(rect);
            sectors.object_id_lists_in_area(&area)
        })
    }

    pub(crate) fn shape_sector_id_lists_in_rect(
        &self,
        rect: DefinitionRect,
    ) -> Option<Vec<Vec<ObjectId>>> {
        self.sector_map().map(|sectors| {
            let area = sectors.area(rect);
            sectors.shape_id_lists_in_area(&area)
        })
    }

    pub(crate) fn player_ids(&self) -> &[i32] {
        self.player_order.as_ref()
    }

    pub(crate) fn player(&self, id: i32) -> Option<&PlayerState> {
        self.players.get(&id)
    }

    pub(crate) fn control_key_name(
        &self,
        control_set: i32,
        control: i32,
        short: bool,
    ) -> Option<&str> {
        let control = usize::try_from(control).ok()?;
        self.control_key_names
            .get(&control_set)?
            .get(control)
            .map(|name| name.display(short))
    }
}

pub(crate) fn build_host_sector_map<'a, I>(
    objects: I,
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
    landscape: &Landscape,
) -> SectorMap
where
    I: IntoIterator<Item = &'a HostWorldObject>,
{
    let width = i32::try_from(landscape.width()).unwrap_or(i32::MAX);
    let height = landscape.estimated_height();
    let mut sectors = SectorMap::new(width, height);
    sectors.rebuild(
        objects
            .into_iter()
            .filter_map(|object| host_sector_record(object, definitions)),
    );
    sectors
}

fn host_sector_record(
    object: &HostWorldObject,
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
) -> Option<SectorObject> {
    if !object.status().is_active() {
        return None;
    }
    Some(SectorObject {
        id: object.id,
        position: object.position(),
        shape_rect: sector_shape_rect(host_object_live_shape_rect(object, definitions)),
    })
}

/// Exact world-space `C4Object::Shape` rectangle at callback entry.
/// Engine-created host snapshots carry `Object::current_shape_rect()` in
/// `ObjectState::shape_override`; the derivation and vertex fallback keep
/// synthetic fixture contexts useful without overriding that authoritative
/// live value.
fn host_object_live_shape_rect(
    object: &HostWorldObject,
    definitions: &HashMap<DefinitionId, DefinitionMetadata>,
) -> DefinitionRect {
    let metadata = definitions.get(object.definition_id());
    object
        .full_state()
        .and_then(|state| state.shape_override)
        .or_else(|| {
            let metadata = metadata?;
            if metadata.line != 0 {
                return metadata.shape;
            }
            crate::transformed_shape_rect(
                metadata.shape,
                object.construction(),
                metadata.stretch_growth,
                metadata.rotateable,
                object.rotation,
            )
        })
        .map(|rect| {
            DefinitionRect::new(
                object.position().x.saturating_add(rect.x),
                object.position().y.saturating_add(rect.y),
                rect.width,
                rect.height,
            )
        })
        .or_else(|| host_vertex_bounds_rect(object.position(), object.vertices()))
        .unwrap_or_else(|| DefinitionRect::new(object.position().x, object.position().y, 1, 1))
}

/// `C4Object::Left/Top/Width/Height`, used by `C4LArea(Object)` and the
/// legacy `C4Object::At` query, expand short shapes upward to an 18-pixel
/// action area. Array Find predicates themselves use the raw shape above.
pub(crate) fn sector_shape_rect(mut rect: DefinitionRect) -> DefinitionRect {
    let add_top = (18 - rect.height).max(0);
    rect.y = rect.y.saturating_sub(add_top);
    rect.height = rect.height.saturating_add(add_top);
    rect
}

pub(crate) fn host_vertex_bounds_rect(
    position: Vector2,
    vertices: &[ObjectVertex],
) -> Option<DefinitionRect> {
    let first = vertices.first()?;
    let mut min_x = first.x;
    let mut max_x = first.x;
    let mut min_y = first.y;
    let mut max_y = first.y;
    for vertex in &vertices[1..] {
        min_x = min_x.min(vertex.x);
        max_x = max_x.max(vertex.x);
        min_y = min_y.min(vertex.y);
        max_y = max_y.max(vertex.y);
    }
    Some(DefinitionRect::new(
        position.x.saturating_add(min_x),
        position.y.saturating_add(min_y),
        max_x.saturating_sub(min_x).saturating_add(1),
        max_y.saturating_sub(min_y).saturating_add(1),
    ))
}

pub(crate) trait WorldAccessor {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject>;
    fn matches_legacy_find_object_candidate(
        &self,
        id: ObjectId,
        params: &FindObjectParams,
    ) -> Option<bool> {
        self.get_object(id)
            .map(|object| params.matches_object(&object))
    }
    fn object_ids(&self) -> Vec<ObjectId>;
    fn master_object_ids(&self) -> Vec<ObjectId>;
    /// Exact world-space `C4Object::Shape`; no `C4Object::addtop` expansion.
    fn object_live_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect;
    /// Sector/legacy-`At` bounds, including `C4Object::addtop`.
    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect;
    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>>;
    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>>;
    /// The bounded Find-with-sort walk needs the per-sector lists kept
    /// separate (C4FindObject.cpp:283-307); shape lists arrive without the
    /// Marker dedup the flat variants apply.
    fn object_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>>;
    fn shape_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>>;
    /// Definition mass/value for the C4SO_Mass/C4SO_Value sorts.
    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata>;
    /// Whether any definition script (or host function) knows `name` —
    /// the `Game.ScriptEngine.GetFirstFunc` lookup C4FindObjectFunc does at
    /// construction (C4Aul.cpp:545-552).
    fn script_function_known(&self, name: &str) -> bool;
}

/// Snapshot fallback for C4Object::Mass. Live host calls use the scoped
/// variant below, while standalone Find fixtures still need the complete
/// cached-mass formula, including recursively contained objects.
fn world_object_mass(
    world: &impl WorldAccessor,
    target: ObjectId,
    visited: &mut HashSet<ObjectId>,
) -> i32 {
    if !visited.insert(target) {
        return 0;
    }
    let Some(object) = world.get_object(target) else {
        visited.remove(&target);
        return 0;
    };
    let Some(metadata) = world.definition_metadata(object.definition_id()) else {
        visited.remove(&target);
        return 1;
    };
    let state = object.full_state();
    let own_mass = state.map(|state| state.own_mass).unwrap_or(0);
    let mut mass = metadata
        .mass
        .saturating_add(own_mass)
        .saturating_mul(object.construction())
        / FULL_CON;
    mass = mass.max(1);
    if !metadata.no_component_mass {
        for child in object.contents() {
            if world
                .get_object(*child)
                .is_some_and(|child| child.is_present())
            {
                mass = mass.saturating_add(world_object_mass(world, *child, visited));
            }
        }
    }
    visited.remove(&target);
    mass
}

/// C4SO_Mass reads the live cached `pFor->Mass`. Prefer the current host scope
/// so contents and own-mass changes made by an earlier Find_Func or Sort_Func
/// callback in the same search are reflected in the key.
pub(crate) fn sort_object_mass(world: &impl WorldAccessor, target: ObjectId) -> i32 {
    let live = HOST_CONTEXT.with(|cell| {
        let borrow = cell.try_borrow().ok()?;
        let context = borrow.as_ref()?;
        context.get_world_object(target)?;
        Some(reflected_object_mass(context, target, &mut HashSet::new()))
    });
    live.unwrap_or_else(|| world_object_mass(world, target, &mut HashSet::new()))
}

impl WorldAccessor for HostWorldContext {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.get(id)
    }

    fn matches_legacy_find_object_candidate(
        &self,
        id: ObjectId,
        params: &FindObjectParams,
    ) -> Option<bool> {
        self.matches_legacy_find_object_candidate(id, params)
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.object_ids()
    }

    fn master_object_ids(&self) -> Vec<ObjectId> {
        HostWorldContext::master_object_ids(self).to_vec()
    }

    fn object_live_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        self.object_live_shape_rect(object)
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        self.object_shape_rect(object)
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.object_sector_ids_in_rect(rect)
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.shape_sector_ids_in_rect(rect)
    }

    fn object_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>> {
        self.object_sector_id_lists_in_rect(rect)
    }

    fn shape_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>> {
        self.shape_sector_id_lists_in_rect(rect)
    }

    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata> {
        HostWorldContext::definition_metadata(self, id).cloned()
    }

    fn script_function_known(&self, name: &str) -> bool {
        HostWorldContext::script_function_known(self, name)
    }
}

/// Borrow-free handle for callback-backed Find/Sort evaluation. Every method
/// takes and releases one short immutable HOST_CONTEXT borrow and returns an
/// owned value; nested script dispatch therefore remains free to take the
/// mutable borrow while all later observations see its completed scope.
#[derive(Clone, Copy)]
pub(crate) struct LiveFuncFindView;

impl LiveFuncFindView {
    pub(crate) fn new() -> Option<Self> {
        HOST_CONTEXT.with(|cell| cell.borrow().as_ref().map(|_| Self))
    }

    fn read<T>(&self, f: impl FnOnce(&EffectHostContext) -> T) -> T {
        HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow
                .as_ref()
                .expect("live Func-find view outlived its host context");
            f(context)
        })
    }
}

impl WorldAccessor for LiveFuncFindView {
    fn get_object(&self, id: ObjectId) -> Option<HostWorldObject> {
        self.read(|context| context.get_world_object(id))
    }

    fn object_ids(&self) -> Vec<ObjectId> {
        self.read(EffectHostContext::world_object_ids)
    }

    fn master_object_ids(&self) -> Vec<ObjectId> {
        self.read(EffectHostContext::master_object_ids)
    }

    fn object_live_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        self.read(|context| match context.get_world_object(object.id) {
            Some(live) => effect_object_live_shape_rect(context, &live),
            None => effect_object_live_shape_rect(context, object),
        })
    }

    fn object_shape_rect(&self, object: &HostWorldObject) -> DefinitionRect {
        sector_shape_rect(self.object_live_shape_rect(object))
    }

    fn object_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.read(|context| {
            <EffectHostContext as WorldAccessor>::object_sector_ids_in_rect(context, rect)
        })
    }

    fn shape_sector_ids_in_rect(&self, rect: DefinitionRect) -> Option<Vec<ObjectId>> {
        self.read(|context| {
            <EffectHostContext as WorldAccessor>::shape_sector_ids_in_rect(context, rect)
        })
    }

    fn object_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>> {
        self.read(|context| {
            <EffectHostContext as WorldAccessor>::object_sector_id_lists_in_rect(context, rect)
        })
    }

    fn shape_sector_id_lists_in_rect(&self, rect: DefinitionRect) -> Option<Vec<Vec<ObjectId>>> {
        self.read(|context| {
            <EffectHostContext as WorldAccessor>::shape_sector_id_lists_in_rect(context, rect)
        })
    }

    fn definition_metadata(&self, id: &str) -> Option<DefinitionMetadata> {
        self.read(|context| <EffectHostContext as WorldAccessor>::definition_metadata(context, id))
    }

    fn script_function_known(&self, name: &str) -> bool {
        self.read(|context| {
            <EffectHostContext as WorldAccessor>::script_function_known(context, name)
        })
    }
}

/// `CheckObjectStatus` erases only `Status == 0`; an object deactivated by a
/// callback has nonzero C4OS_INACTIVE and remains in a result already being
/// built even though it has left the active master list.
pub(crate) fn object_present_after_callback(world: &impl WorldAccessor, id: ObjectId) -> bool {
    world
        .get_object(id)
        .is_some_and(|object| object.status() != ObjectStatus::Deleted)
}

pub(crate) fn retain_present_after_callback(world: &impl WorldAccessor, ids: &mut Vec<ObjectId>) {
    ids.retain(|id| object_present_after_callback(world, *id));
}

/// `FnIsNetwork` reads `Game.Parameters.IsNetworkGame`
/// (C4Script.cpp:3554). Parameter setup copies that flag from the active
/// `Game.NetworkActive` session (C4GameParameters.cpp:429-434).
pub(crate) fn is_network(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Bool(
            cell.borrow()
                .as_ref()
                .is_some_and(|context| context.world.network_game()),
        ))
    })
}

/// FnReloadDef (C4Script.cpp:4974-4990). The engine has a source-backed
/// script relink core, but no runtime resource path from which this native can
/// safely reload graphics/script data. Preserve the typed tooling surface and
/// report the unsupported reload as C4ValueInt false.
pub(crate) fn reload_def(args: &[Value]) -> Result<Value, RuntimeError> {
    let _definition = parse_native_c4id_argument(args.first(), "ReloadDef")?;
    Ok(Value::Int(0))
}

/// FnPauseGame (C4Script.cpp:6042-6051). Console pausing is process-local:
/// suppress it during replay and otherwise hand the halt/toggle action to the
/// embedding app without mutating synchronized engine state.
pub(crate) fn pause_game(args: &[Value]) -> Result<Value, RuntimeError> {
    let toggle = value_to_bool(args.first().unwrap_or(&Value::Nil), "PauseGame", "toggle")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        if let Some(context) = borrow.as_ref() {
            if !context.world.replay_control {
                context
                    .world
                    .pause_game_requests
                    .borrow_mut()
                    .push(if toggle {
                        PauseGameRequest::Toggle
                    } else {
                        PauseGameRequest::Halt
                    });
            }
        }
    });
    Ok(Value::Nil)
}

/// FnFrameCounter (C4Script.cpp): Game.FrameCounter — the current
/// simulation frame.
pub(crate) fn frame_counter(_args: &[Value]) -> Result<Value, RuntimeError> {
    ENVIRONMENT_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let frame = borrow
            .as_ref()
            .map(|context| context.frame as i32)
            .unwrap_or(0);
        Ok(Value::Int(frame))
    })
}

/// `FnGetSystemTime` (C4Script.cpp:4654-4684): expose one local wall-clock
/// field only outside network, replay, and recording synchronization modes.
pub(crate) fn get_system_time(args: &[Value]) -> Result<Value, RuntimeError> {
    let field = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetSystemTime",
        "field",
    )?;
    let sync_mode = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.world.control_sync_mode)
    });
    if sync_mode || !(0..=7).contains(&field) {
        return Ok(Value::Nil);
    }

    let now = Local::now();
    let value = match field {
        0 => now.year(),
        1 => now.month() as i32,
        2 => now.weekday().num_days_from_sunday() as i32,
        3 => now.day() as i32,
        4 => now.hour() as i32,
        5 => now.minute() as i32,
        6 => now.second() as i32,
        7 => (now.nanosecond() / 1_000_000) as i32,
        _ => unreachable!("field range checked above"),
    };
    Ok(Value::Int(value))
}

/// FnGetTime (C4Script.cpp:4647-4652): expose the process-local
/// `timeGetTime()` clock only in local, non-recording control mode. A missing
/// host context corresponds to C4GameControl::CM_None and is synchronized.
pub(crate) fn get_time(_args: &[Value]) -> Result<Value, RuntimeError> {
    with_host_context(Ok(Value::Nil), |context| {
        if context.world.control_sync_mode {
            return Ok(Value::Nil);
        }
        Ok(Value::Int(clonk_core::chrono_util::time_get_time() as i32))
    })
}

/// FnGetDefCoreVal (C4Script.cpp:4170-4183): compiler-shaped reflection over
/// the complete DefCore/Shape surface and sibling Physical section.
pub(crate) fn get_def_core_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let entry = parse_optional_string(args.first(), "GetDefCoreVal", "entry")?;
    let section = parse_optional_string(args.get(1), "GetDefCoreVal", "section")?
        .filter(|section| !section.is_empty());
    let requested = parse_native_c4id_argument(args.get(2), "GetDefCoreVal")?;
    let entry_index = parse_optional_i32(args.get(3), "GetDefCoreVal", "entry_nr")?.unwrap_or(0);
    let Some(entry) = entry else {
        return Ok(Value::Nil);
    };
    with_host_context(Ok(Value::Nil), |context| {
        let definition_id = match requested {
            Some(id) => Some(id),
            // `if (!idDef && cthr->Def) idDef = cthr->Def->id` — the
            // executing function's definition, even without an object.
            None => context
                .current_definition_id()
                .map(|definition| definition.to_string()),
        };
        let Some(definition_id) = definition_id else {
            return Ok(Value::Nil);
        };
        let Some(metadata) = context.definition_metadata(definition_id.as_str()) else {
            return Ok(Value::Nil);
        };
        if !metadata.fire.def_core_values.is_empty() {
            return Ok(metadata
                .fire
                .def_core_values
                .get(entry.as_str(), section.as_deref(), entry_index)
                .unwrap_or(Value::Nil));
        }
        // Snapshot-only and synthetic host fixtures predating the complete
        // reflection store retain the small modeled fallback below.
        if section
            .as_deref()
            .is_some_and(|section| section != "DefCore")
        {
            return Ok(Value::Nil);
        }
        let shape = metadata.shape.unwrap_or(DefinitionRect::new(0, 0, 0, 0));
        if entry == "Offset" {
            return Ok(match entry_index {
                0 => Value::Int(shape.x),
                1 => Value::Int(shape.y),
                _ => Value::Nil,
            });
        }
        if entry_index != 0 {
            return Ok(Value::Nil);
        }
        Ok(match entry.as_str() {
            "Width" => Value::Int(shape.width),
            "Height" => Value::Int(shape.height),
            "CollectionLimit" => Value::Int(metadata.collection_limit),
            "GrabPutGet" => Value::Int(metadata.grab_put_get),
            "FireTop" => Value::Int(metadata.fire.fire_top),
            "LiftTop" => Value::Int(metadata.fire.lift_top),
            "Value" => Value::Int(metadata.value),
            "Mass" => Value::Int(metadata.mass),
            // C4Def::CompileFunc's line bitfields (C4Def.cpp:333-351).
            "Line" => Value::Int(metadata.line),
            "LineConnect" => Value::Int(metadata.line_connect as i32),
            // The blast-chain entries System.c4g reads through the GetXVal
            // wrappers (GetDefGrab/GetDefHorizontalFix/GetDefContainBlast,
            // BlastObjectsShockwaveCheck + DoExplosion).
            "Grab" => Value::Int(metadata.fire.grab),
            "NoPushEnter" => Value::Int(metadata.fire.no_push_enter),
            "HorizontalFix" => Value::Int(metadata.fire.no_horizontal_move),
            "ContainBlast" => Value::Int(metadata.fire.contain_blast),
            "ClosedContainer" => Value::Int(metadata.fire.closed_container),
            "BlastIncinerate" => Value::Int(metadata.fire.blast_incinerate),
            "ContactIncinerate" => Value::Int(metadata.fire.contact_incinerate),
            other => {
                tracing::debug!(entry = other, "GetDefCoreVal entry not modeled; nil");
                Value::Nil
            }
        })
    })
}

/// FnGetScenarioVal (C4Script.cpp:4244-4250): exact StdCompiler reflection
/// over the retained, post-load `Game.C4S`. `entry_nr` counts primitive
/// callbacks inside the matched named value: C4SVal is Std/Rnd/Min/Max and
/// ID lists are ID/count pairs (C4Script.cpp:3997-4006).
pub(crate) fn get_scenario_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(entry) = parse_optional_string(args.first(), "GetScenarioVal", "entry")? else {
        return Ok(Value::Nil);
    };
    let section = parse_optional_string(args.get(1), "GetScenarioVal", "section")?
        .filter(|section| !section.is_empty());
    let entry_index = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "GetScenarioVal",
        "entry_nr",
    )?;
    with_host_context(Ok(Value::Nil), |context| {
        if let Some(value) =
            context
                .world
                .scenario_value(entry.as_str(), section.as_deref(), entry_index)
        {
            return Ok(match value {
                ScenarioValue::Int(value) => Value::Int(*value),
                ScenarioValue::Bool(value) => Value::Bool(*value),
                ScenarioValue::String(value) => Value::from(value.clone()),
                // C4Value(C4ID_None) has C4V_Any type, i.e. nil rather than
                // a typed zero ID (C4Value.h:113,306).
                ScenarioValue::C4Id(value) if value.is_empty() => Value::Nil,
                ScenarioValue::C4Id(value) => {
                    let raw = clonk_script::c4_id_parse(value);
                    if raw == 0 {
                        Value::Nil
                    } else {
                        Value::C4Id(clonk_script::c4_id_from_raw(raw))
                    }
                }
            });
        }
        tracing::debug!(
            entry = entry.as_str(),
            section = section.as_deref().unwrap_or(""),
            entry_index,
            "GetScenarioVal entry not found; nil"
        );
        Ok(Value::Nil)
    })
}

/// `FnLoadScenarioSection` (C4Script.cpp:5401-5408): reject a null/empty
/// name, resolve the section case-insensitively, and hand the engine an
/// ordered request. C++ removes every active object but deliberately keeps
/// inactive objects, so their effective identities travel with the request.
pub(crate) fn load_scenario_section(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "LoadScenarioSection expects at most 2 arguments: name, flags",
        ));
    }
    let Some(name) = parse_optional_string(args.first(), "LoadScenarioSection", "name")? else {
        return Ok(Value::Int(0));
    };
    if name.is_empty() {
        return Ok(Value::Int(0));
    }
    let flags = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "LoadScenarioSection",
        "flags",
    )?;

    with_host_context_mut(Ok(Value::Int(0)), |context| {
        if !context.world.scenario_section_known(&name) {
            return Ok(Value::Int(0));
        }
        let preserve_ids = context
            .all_world_object_ids()
            .into_iter()
            .filter(|id| {
                context
                    .get_world_object(*id)
                    .is_some_and(|object| object.status() == ObjectStatus::Inactive)
            })
            .collect();
        context.record_player_command(PlayerCommand::LoadScenarioSection {
            name,
            flags,
            preserve_ids,
        });
        Ok(Value::Int(1))
    })
}

pub(crate) fn game_over(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GameOver expects at most 1 argument: game over state",
        ));
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow
            .as_mut()
            .ok_or_else(|| RuntimeError::new("GameOver requires an active engine context"))?;
        let triggered = context.request_game_over();
        Ok(Value::Bool(triggered))
    })
}

/// FnFreeRect (C4Script.cpp:3125-3131): clears the landscape rect in
/// GLOBAL coordinates (no caller offset, unlike DigFree*) without
/// producing dug-out material. A nonzero fifth argument selects C++'s
/// density-filtered ClearRectDensity arm.
/// FnScriptGo (C4Script.cpp:2782-2786): switches the scenario script
/// counter gate (Game.Script.Go) that drives the timed Script%d
/// sections (C4GameScriptHost::Execute, C4ScriptHost.cpp:222-232).
pub(crate) fn script_go(args: &[Value]) -> Result<Value, RuntimeError> {
    let go = match args.first() {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Int(value)) => *value != 0,
        _ => false,
    };
    with_host_context_mut(Ok(Value::Nil), |context| {
        context.script_go_request = Some(go);
        Ok(Value::Nil)
    })
}

/// FnScriptCounter (C4Script.cpp:3616-3619): zero-argument getter for the
/// live scenario counter. C4GameScriptHost::Execute has already incremented
/// it before entering ScriptN (C4ScriptHost.cpp:222-232), and a preceding
/// goto() in this same VM call is immediately visible.
pub(crate) fn script_counter(_args: &[Value]) -> Result<Value, RuntimeError> {
    with_host_context(Ok(Value::Int(0)), |context| {
        Ok(Value::Int(
            context
                .script_counter_request
                .unwrap_or(context.scenario_script_counter),
        ))
    })
}

/// Fn_goto (C4Script.cpp:225-229): synchronously replaces
/// `Game.Script.Counter` and returns the assigned integer. The current
/// C4GameScriptHost pulse has already post-incremented the counter before it
/// calls Script%d (C4ScriptHost.cpp:222-232), so this redirects the next pulse.
pub(crate) fn script_goto(args: &[Value]) -> Result<Value, RuntimeError> {
    let counter = value_to_i32(args.first().unwrap_or(&Value::Nil), "goto", "counter")?;
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.script_counter_request = Some(counter);
        }
        Ok(Value::Int(counter))
    })
}

/// `Game.OverlapObject` (C4Game.cpp:1298-1313) over the host world: any
/// active, uncontained object whose category intersects `category` within
/// C4D_SortLimit and whose shape rect overlaps the given rect
/// (C4Rect::Overlap, C4Rect.cpp:92-99).
pub(crate) fn host_overlap_object(
    context: &EffectHostContext,
    x: i32,
    y: i32,
    wdt: i32,
    hgt: i32,
    category: i32,
) -> bool {
    context.world_object_ids().into_iter().any(|id| {
        let Some(object) = context.get_world_object(id) else {
            return false;
        };
        if !object.is_present() || !object.status().is_active() {
            return false;
        }
        if object.container().is_some() {
            return false;
        }
        if object.category() & category & CATEGORY_SORT_LIMIT == 0 {
            return false;
        }
        rects_overlap_cpp(
            DefinitionRect::new(x, y, wdt, hgt),
            effect_object_live_shape_rect(context, &object),
        )
    })
}

/// FnSetGameSpeed (C4Script.cpp:5219-5231): a zero/unfilled speed restores
/// the legacy 38 FPS default, nonzero values must be in 1..=1000, and league
/// games reject calls whose caller is a DirectExec temporary script.
pub(crate) fn set_game_speed(args: &[Value]) -> Result<Value, RuntimeError> {
    let requested = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetGameSpeed", "speed")?;

    with_host_context(Ok(Value::Bool(false)), |context| {
        if context.world.league_game() && clonk_script::caller_is_temporary_script() != Some(false)
        {
            return Ok(Value::Bool(false));
        }
        let speed = if requested == 0 {
            38
        } else if (1..=1000).contains(&requested) {
            requested
        } else {
            return Ok(Value::Bool(false));
        };
        context
            .world
            .restart_game_tick_delay_ms((1000 / speed) as u64);
        Ok(Value::Bool(true))
    })
}

/// FnSetPreSend (C4Script.cpp:5695-5707): typed arguments are converted before
/// the native body. Negative values fail, nonnegative offline calls are no-op
/// successes, and network calls enqueue a process-local target request. Each
/// app matches the optional wildcard against its own client name.
pub(crate) fn set_pre_send(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_fps = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPreSend",
        "target FPS",
    )?;
    let client_pattern = parse_native_c4_string_argument(args.get(1), "SetPreSend", "client name")?;
    if target_fps < 0 {
        return Ok(Value::Bool(false));
    }

    let network_control_mode = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.world.network_control_mode())
    });
    if !network_control_mode {
        return Ok(Value::Bool(true));
    }

    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.world.network_target_fps_requests.borrow_mut().push(
                crate::NetworkTargetFpsRequest {
                    target_fps: if target_fps == 0 { 38 } else { target_fps },
                    client_pattern,
                },
            );
        }
    });
    Ok(Value::Bool(true))
}

/// Raster-only C4SolidMask::Remove shared by a live callback context and the
/// threaded preview between successive callbacks in one effect batch.
pub(crate) fn remove_host_solid_mask_raster(
    landscape: &mut Landscape,
    bakes: &mut Vec<(ObjectId, crate::SolidMaskBake)>,
    id: ObjectId,
) -> Option<(usize, u64)> {
    let index = bakes.iter().position(|(object_id, _)| *object_id == id)?;
    let (_, bake) = bakes.remove(index);
    let instance_sequence = bake.instance_sequence;
    let vehicle = landscape.grid_vehicle_byte()?;
    bake.restore_background(landscape, vehicle);
    let mut overlapping_masks = bakes
        .iter()
        .enumerate()
        .filter_map(|(index, (_, other))| other.overlaps(&bake).then_some(index))
        .collect::<Vec<_>>();
    overlapping_masks.sort_unstable_by(|&left, &right| {
        bakes[right]
            .1
            .instance_sequence
            .cmp(&bakes[left].1.instance_sequence)
    });
    for other_index in overlapping_masks {
        let (_, other) = &mut bakes[other_index];
        other.reput_after_removal(&bake, landscape, vehicle);
    }
    Some((index, instance_sequence))
}
