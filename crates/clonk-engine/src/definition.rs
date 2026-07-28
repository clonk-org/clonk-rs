//! `lib` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[derive(Clone)]
pub struct DefinitionSpriteImage {
    #[doc(hidden)]
    pub width: u32,
    #[doc(hidden)]
    pub height: u32,
    #[doc(hidden)]
    pub pixels: Arc<[u8]>,
    #[doc(hidden)]
    pub color_mask: Option<Arc<[u8]>>,
}

impl DefinitionSpriteImage {
    pub(crate) fn from_resource(
        image: &clonk_resources::GraphicsImage,
        mask: Option<&clonk_resources::ColorByOwnerMask>,
    ) -> Self {
        let color_mask = mask.and_then(|mask| {
            if mask.width != image.width() || mask.height != image.height() {
                return None;
            }
            let channels =
                definition_color_mask_channels(&mask.pixels, image.width(), image.height())?;
            if !definition_color_mask_has_coverage(&mask.pixels, channels) {
                return None;
            }
            Some(Arc::from(mask.pixels.clone().into_boxed_slice()))
        });
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.clone_pixels(),
            color_mask,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn into_pixels(self) -> Arc<[u8]> {
        self.pixels
    }

    pub fn color_mask(&self) -> Option<Arc<[u8]>> {
        self.color_mask.as_ref().map(Arc::clone)
    }

    pub(crate) fn solid_mask_source_pixels(&self) -> Arc<[u8]> {
        let Some(overlay) = self
            .color_mask
            .as_ref()
            .filter(|overlay| overlay.len() == self.pixels.len())
        else {
            return Arc::clone(&self.pixels);
        };
        let mut pixels = self.pixels.to_vec();
        for (base, overlay) in pixels.chunks_exact_mut(4).zip(overlay.chunks_exact(4)) {
            // C4Surface::GetPixDw composites the owner surface before
            // IsPixTransparent samples a solid mask. With C4's inverse-alpha
            // BltAlpha helper this is a saturating sum of conventional
            // opacities, not just the base PNG's alpha.
            base[3] = base[3].saturating_add(overlay[3]);
        }
        Arc::from(pixels.into_boxed_slice())
    }

    fn colorize_by_material(&mut self, colors: &[[u8; 4]; 3]) {
        colorize_definition_pixels(&mut self.pixels, colors);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionActionFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DefinitionActionGraphics {
    pub facet: Option<DefinitionActionFacet>,
    pub directions: i32,
    pub flip_dir: Option<i32>,
    pub reverse: bool,
    pub facet_base: bool,
    pub facet_top_face: bool,
    pub facet_target_stretch: bool,
    pub length: Option<i32>,
}

/// Reserved metadata key indicating that an action-graphics map also carries
/// physical ActMap slots. Legacy/synthetic maps omit it and remain name-only.
#[doc(hidden)]
pub const PHYSICAL_ACTION_GRAPHICS_MARKER: &str = "\0lc:actmap:physical";

#[doc(hidden)]
pub fn physical_action_graphics_key(index: u32) -> String {
    format!("\0lc:actmap:{index}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionComponent {
    pub id: DefinitionId,
    pub count: i32,
}

#[derive(Clone)]
pub struct Definition {
    pub(crate) id: DefinitionId,
    pub(crate) name: String,
    /// DefCore `Version` / C4Def::rC4XVer (src/C4Def.h:190).
    pub(crate) version: [i32; 5],
    /// DefCore `RequireDef`; reflected as an ID-only C4IDList.
    pub(crate) require_defs: Vec<String>,
    /// Exact signed compiler values for DefCore fields whose gameplay
    /// projection is intentionally normalized to bool/option/nonnegative.
    pub(crate) def_core_reflected_ints: HashMap<String, i32>,
    /// Trimmed localized C4Def description (`C4Def::GetDesc`).
    description: Option<String>,
    /// Shared compiled script: `host_world_context()` hands clones of this
    /// `Arc` to host functions so nested script calls (Find_Func, GameCall)
    /// can execute another definition's functions mid-VM-call.
    pub(crate) script: Arc<ScriptEngine>,
    /// The preparsed, unlinked script owned by this definition. Relinking
    /// rebuilds `script` from this pristine copy so include/append copies
    /// and global-function links cannot accumulate.
    pub(crate) base_script: clonk_script::Script,
    /// The raw Script.c text — read-only presentation support and declaration
    /// ordering for C4ScriptHost `[..|Image=..]` descriptors
    /// (C4AulParse.cpp:301-380).
    pub(crate) script_source: String,
    includes: Vec<String>,
    /// Mirrors C4AulScript::IncludesResolved: definitions already linked in a
    /// prior resolve pass must not copy the same parent functions again when
    /// a later definition is registered.
    pub(crate) includes_resolved: bool,
    /// `#appendto` targets of this definition's script
    /// (C4AulScript::Appends; resolved by Engine::resolve_appends).
    pub(crate) appends: Vec<clonk_script::AppendTo>,
    pub(crate) has_construction: bool,
    pub(crate) has_initialize: bool,
    pub(crate) has_step: bool,
    action_library: ActionLibrary,
    /// Whether `C4DefScriptHost::AfterLink` populated action and TimerCall
    /// function pointers for the current linked script tree.
    callbacks_linked: bool,
    action_graphics: HashMap<String, DefinitionActionGraphics>,
    crew_member: bool,
    /// Literal signed DefCore `CrewMember` value returned by FnCrewMember.
    /// `crew_member` remains the derived nonzero gameplay capability.
    pub(crate) crew_member_value: i32,
    pub(crate) no_standard_crew: i32,
    /// DefCore `SilentCommands`: suppresses C4Command::Fail's common
    /// message/sound/ComDir-stop tail, but not command-specific callbacks.
    pub(crate) silent_commands: bool,
    /// DefCore `CanBeBase` (C4Def.cpp; FirstBase detection in
    /// PlaceReadyBase, C4Player.cpp:596-599).
    pub(crate) can_be_base: bool,
    /// The def's ClonkNames list content (C4Def.cpp:645-652,
    /// C4CFN_ClonkNames): overrides Game.Names for new crew infos.
    clonk_names: Option<String>,
    /// C4Def::fClonkNamesOwned: inherited include data must be cleared by
    /// ResetIncludeDependencies while a definition's own list survives.
    clonk_names_owned: bool,
    movement: MovementProfile,
    pub(crate) category: i32,
    pub(crate) max_user_select: i32,
    /// DefCore `BlitMode`, copied into C4Object::BlitMode at Init/reset.
    pub(crate) blit_mode: u32,
    /// DefCore `ColorByOwner`, used by picture-stack equality.
    pub(crate) color_by_owner: bool,
    pub(crate) color_by_material: String,
    /// DefCore `AllowPictureStack` APS_* exception bits.
    pub(crate) allow_picture_stack: i32,
    /// C4Def graphics scale (`C4DefCore::Scale / 100.0f`).
    pub(crate) graphics_scale: f32,
    ocf_base: u32,
    pub(crate) value: i32,
    /// DefCore `NoSell`; any nonzero value prevents SellFromBase from
    /// selecting this definition as the root sale object.
    pub(crate) no_sell: i32,
    /// DefCore `Rebuy` (C4Def.cpp:359): permits sold objects to introduce
    /// their definition into home-base stock.
    pub(crate) rebuyable: bool,
    /// DefCore `BaseAutoSell` (C4Def.cpp:457): sold automatically by a base
    /// while BASEFUNC_AutoSellContents is active.
    pub(crate) base_auto_sell: bool,
    pub(crate) mass: i32,
    pub(crate) move_to_range: i32,
    /// DefCore `Pathfinder` (C4Def.cpp:399): nonzero enables MoveTo path
    /// search for non-crew objects and supplies the clamped search level.
    pub(crate) pathfinder: i32,
    /// DefCore `NoTransferZones` (C4Def.cpp:415): exclude transfer-zone
    /// edges from this definition's MoveTo path searches.
    pub(crate) no_transfer_zones: i32,
    /// DefCore `NoPushEnter` (C4Def.cpp:396): any nonzero value prevents
    /// this definition from executing Enter commands.
    pub(crate) no_push_enter: i32,
    pub(crate) drag_image_picture: i32,
    pub(crate) picture: Option<DefinitionPicture>,
    picture_image: Option<DefinitionPictureImage>,
    /// First def portrait (C4CFN_Portraits, src/C4Components.h:88) — HUD
    /// cursor info only (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-320).
    portrait_image: Option<DefinitionPictureImage>,
    /// ColorByOwner-aware portrait surface retained for
    /// C4Game::DrawTextSpecImage portrait specifications.
    portrait_graphics_image: Option<DefinitionPictureImage>,
    /// All named `Portrait*.*` variants, keyed by lowercase portrait name.
    portrait_graphics: Vec<(String, DefinitionPictureImage)>,
    /// Def rank symbols (C4Def::pRankSymbols from Rank.png,
    /// src/C4Def.cpp:684-691) — HUD cursor info only.
    rank_symbols_image: Option<DefinitionPictureImage>,
    /// Finite localized `C4Def::pRankNames` table used by Promote. This is
    /// independent of the rank-symbol strip and may be inherited.
    rank_names: Option<RankNameTable>,
    /// `C4RankSystem::Base` paired with `rank_names`. Like the native
    /// `pRankNames` pointer, an inherited rank table carries its curve.
    rank_base: Option<i32>,
    rank_names_owned: bool,
    /// Base rank-cell count after localized extension cells are removed.
    rank_symbol_count: Option<u32>,
    /// Whether this definition loaded its own strip. Non-owned pointers are
    /// overwritten by each linked include just like C4Def's
    /// `fRankSymbolsOwned`/`IncludeDefinition` path.
    rank_symbols_owned: bool,
    sprite_image: Option<DefinitionSpriteImage>,
    sprite_variants: HashMap<String, DefinitionSpriteImage>,
    pub(crate) shape: Option<DefinitionRect>,
    /// C4Shape::FireTop (C4Shape.cpp:509).
    pub(crate) fire_top: i32,
    /// DefCore `LiftTop` (C4Def.cpp:385), used by DFA_LIFT's target-height
    /// callback gate (C4Object.cpp:5281-5286).
    pub(crate) lift_top: i32,
    solid_mask: Option<DefinitionTargetRect>,
    pub(crate) def_core_solid_mask: Option<DefinitionTargetRect>,
    /// DefCore `TopFace` (C4Def.cpp:306), drawn in the second object pass.
    top_face: Option<DefinitionTargetRect>,
    pub(crate) def_core_top_face: Option<DefinitionTargetRect>,
    shape_vertices: Vec<ObjectVertex>,
    /// Complete fixed C4Shape slots from DefCore. Fresh C4Object::Init copies
    /// the whole shape, not just its active VtxNum prefix.
    pub(crate) shape_vertex_slots: ShapeVertexBuffer,
    pub(crate) contact_density: i32,
    pub(crate) contact_function_calls: bool,
    collection_rect: Option<DefinitionRect>,
    pub(crate) def_core_collection_rect: Option<DefinitionRect>,
    pub(crate) collection_limit: i32,
    /// DefCore `Fragile`; outdoor Put must not throw these objects into a
    /// target's collection area.
    pub(crate) fragile: bool,
    /// Raw DefCore `Projectile`; any nonzero value lets Attack select the
    /// object from the attacker's contents.
    pub(crate) projectile: i32,
    pub(crate) explosive: i32,
    pub(crate) collectible: bool,
    /// DefCore `NoGet` (src/C4Def.cpp:412): omit this definition from
    /// manual get/activate menus when set to any nonzero value.
    pub(crate) no_get: bool,
    /// `GrabPutGet` DefCore bitfield (src/C4Def.cpp:364-373) — read by the
    /// viewport command-row presentation (C4Object::DrawCommands).
    pub(crate) grab_put_get: i32,
    /// DefCore `VehicleControl` (src/C4Def.cpp:398):
    /// C4D_VehicleControl_Outside=1 | C4D_VehicleControl_Inside=2, the
    /// SetCommand ControlCommand overloads (C4Object.cpp:3944-3969).
    pub(crate) vehicle_control: i32,
    pub(crate) constructable: bool,
    pub(crate) construction_offset: i32,
    pub(crate) stretch_growth: bool,
    /// DefCore `Oversize`: DoCon may grow beyond FullCon.
    pub(crate) oversize: bool,
    /// `Placement=` (C4Def.cpp:312): 0 surface, 1 liquid, 2 air.
    pub(crate) placement: i32,
    /// `Growth=` (C4Def.cpp:358): PlaceVegetation's random-growth gate.
    pub(crate) growth: i32,
    pub(crate) basement: i32,
    pub(crate) rotateable: i32,
    pub(crate) border_bound: i32,
    pub(crate) upright_attach: i32,
    /// RotatedSolidmasks (C4Def.cpp:414): the solid mask stays put while
    /// the object is rotated (C4Object::UpdateSolidMask gate,
    /// C4Object.cpp:5655) and bakes through the rotated branch of
    /// C4SolidMask::Put (C4SolidMask.cpp:108-174).
    pub(crate) rotated_solid_masks: bool,
    /// DefCore `AutoContextMenu` (C4Def.cpp:416): entering this container
    /// may automatically open its context menu (C4Object.cpp:2049-2056).
    pub(crate) auto_context_menu: bool,
    pub(crate) needed_gfx_mode: i32,
    pub(crate) no_component_mass: bool,
    /// NoStabilize=1 opts out of the small-tilt upright snap
    /// (C4Object::Stabilize, C4Movement.cpp:491).
    pub(crate) no_stabilize: bool,
    pub(crate) hide_hud_bars: i32,
    pub(crate) hide_hud_elements: i32,
    /// DefCore Timer= interval in frames (default 35, C4Def.cpp:298).
    pub(crate) timer: i32,
    /// DefCore TimerCall= function name (C4Def.cpp:299), fired every
    /// Timer-th Execute per object (C4Object.cpp:1085-1091). None when
    /// the def names no callback (C++ links to nullptr).
    pub(crate) timer_call: Option<String>,
    /// Runtime-only `C4Def::TimerCall` function pointer cache.
    timer_call_link: ScriptCallbackLink,
    /// Runtime-only `C4DefScriptHost::SFn_ControlTransfer` pointer cache.
    control_transfer_link: ScriptCallbackLink,
    pub(crate) components: Vec<DefinitionComponent>,
    pub(crate) line_connect: u32,
    /// ContactIncinerate=N: 1-in-N contact-fire chance (0 = not inflammable).
    pub(crate) contact_incinerate: i32,
    /// BlastIncinerate=N: incinerate once accumulated Damage reaches N after
    /// a blast (C4Object::Blast, C4Object.cpp:1421-1423); 0 = off.
    pub(crate) blast_incinerate: i32,
    /// ContainBlast=1: shields contents from explosions (the DoExplosion
    /// container walk, C4Effect.cpp:884).
    pub(crate) contain_blast: i32,
    /// Any nonzero `ClosedContainer` prevents contained objects from
    /// inheriting this object's cached `InMat`.
    pub(crate) closed_container: i32,
    /// HorizontalFix=1 (C4Def::NoHorizontalMove, C4Def.cpp:383): exempt
    /// from shockwave flings.
    pub(crate) no_horizontal_move: i32,
    pub(crate) no_burn_decay: bool,
    pub(crate) no_burn_damage: bool,
    /// NoBreath=1: exempt from the ExecLife breathing check (C4Object.cpp:880).
    pub(crate) no_breath: bool,
    pub(crate) temporary_crew: i32,
    pub(crate) smoke_rate: i32,
    /// `Float` DefCore value (C4Def.cpp:379): IsInLiquidCheck buoyancy line
    /// offset (C4Object.cpp:5609-5612).
    pub(crate) float_line: i32,
    pub(crate) line: i32,
    pub(crate) line_intersect: i32,
    /// Grab DefCore value: 0 none, 1 grab+push, 2 grab-only (C4Object.cpp:1763).
    pub(crate) grab: i32,
    pub(crate) burn_turn_to: Option<String>,
    /// DefCore `ConstructTo` / C4Def::BuildTurnTo: successful Build ticks
    /// change the construction target after DoCon.
    pub(crate) build_turn_to: Option<String>,
    pub(crate) incomplete_activity: bool,
    /// `Exclusive` DefCore flag (C4Def.cpp:313): OCF_Exclusive — no action
    /// through this, no construction in front of it (SetOCF,
    /// C4Object.cpp:581-583).
    pub(crate) exclusive: bool,
    /// `Edible` DefCore flag (C4Def.cpp:355): OCF_Edible (SetOCF,
    /// C4Object.cpp:630-632).
    pub(crate) edible: bool,
    /// `Prey` DefCore flag (C4Def.cpp:354): OCF_Prey while alive (SetOCF,
    /// C4Object.cpp:615-618).
    pub(crate) prey: bool,
    /// `AttractLightning` DefCore flag (C4Def.cpp:391): OCF_AttractLightning
    /// at FullCon (SetOCF, C4Object.cpp:623-626).
    pub(crate) attract_lightning: bool,
    /// `Entrance` DefCore rect (C4Def.cpp:309): the enter/activate area
    /// (OCF_Entrance, SetOCF C4Object.cpp:584-587; area check in
    /// GetOCFForPos, C4Object.cpp:1149-1153).
    pub(crate) entrance_rect: Option<DefinitionRect>,
    /// `RotatedEntrance` (C4Def.cpp:377): 0 = upright only, 1 = any
    /// rotation, N = rotations up to N degrees (SetOCF, C4Object.cpp:586).
    pub(crate) rotated_entrance: i32,
    /// `NoFight` DefCore flag (C4Def.cpp:413): suppresses OCF_FightReady
    /// (SetOCF, C4Object.cpp:606-610).
    pub(crate) no_fight: bool,
    /// `Chop` DefCore flag (C4Def::Chopable, C4Def.cpp:378): OCF_Chop
    /// candidate (SetOCF, C4Object.cpp:570-575).
    pub(crate) chopable: bool,
    /// The [Physical] DefCore section (C4Def::Physical).
    pub(crate) physical: PhysicalInfo,
    /// Real C4Script content gets the C++ callback arguments — no parameters
    /// for StartCall/EndCall/PhaseCall, the last phase for AbortCall
    /// (C4Object.cpp:4154-4182) — while synthetic command-DSL fixtures keep
    /// the additive (state, action) convention.
    pub(crate) c4_callback_args: bool,
    /// SolidMask alpha pixels extracted from the sprite once at set time —
    /// the movement loop reads them per masked object per moving object per
    /// tick, and re-scanning the sprite there dominated the loop.
    pub(crate) solid_mask_pixels: SolidMaskPixels,
    /// Per-active-graphics/override-rect pixel cache (Objects.txt SolidMask=
    /// picks its own sprite region, while SetGraphics picks another bitmap).
    solid_mask_rect_cache:
        std::cell::RefCell<HashMap<(Option<String>, i32, i32, i32, i32), SolidMaskPixels>>,
}

/// Precomputed solid-mask pixel data for `solid_masks_for_movement`.
#[derive(Debug, Clone, Default)]
pub(crate) enum SolidMaskPixels {
    /// No sprite image: the whole mask rect is solid.
    #[default]
    Rectangle,
    /// Per-pixel alpha mask (1 = solid).
    Alpha(Arc<Vec<u8>>),
    /// Invalid rect or unavailable named bitmap: ignore the mask entirely.
    OutOfBounds,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ActionCallbackKind {
    Start,
    End,
    Phase,
    Abort,
}

impl ActionCallbackKind {
    fn context(self) -> &'static str {
        match self {
            ActionCallbackKind::Start => "action start",
            ActionCallbackKind::End => "action end",
            ActionCallbackKind::Phase => "action phase",
            ActionCallbackKind::Abort => "action abort",
        }
    }
}

/// Context labels used by actual C4AulScript::DirectExec call sites. Several
/// Rust compatibility adapters reuse the expression evaluator for native C++
/// calls; those must not appear in C4Aul's DirectExec trace/profiler totals.
pub(crate) fn is_cpp_direct_exec_context(context: &str) -> bool {
    matches!(
        context,
        "console script" | "internal script" | "MenuCommand"
    )
}

impl Definition {
    pub fn from_script(
        id: impl Into<String>,
        name: impl Into<String>,
        source: &str,
    ) -> Result<Self, EngineError> {
        let id = id.into();
        let name = name.into();

        // Compile the script to extract includes before adding to engine
        let compiled_script =
            clonk_script::Script::compile_c4_string(source).map_err(|parse_error| {
                EngineError::Script {
                    definition: id.clone(),
                    function: "load".to_string(),
                    source: parse_error.into(),
                    recovery: None,
                }
            })?;
        for diagnostic in compiled_script.parse_diagnostics() {
            tracing::warn!(
                definition = %id,
                %diagnostic,
                "definition script parse error quarantined; continuing like C++"
            );
        }

        let includes = compiled_script.includes().to_vec();
        let appends = compiled_script.appends().to_vec();

        let mut script = ScriptEngine::new();
        script.set_script_name(id.clone());
        script.set_definition_name(name.clone());
        script.set_definition_context(true);
        script.add_script(compiled_script.clone());
        compat::register_host_functions(&mut script);
        // Synthetic command-DSL fixtures historically declare these as
        // `global func`; real C4 content switches to local-only callback
        // gates in set_c4_callback_convention below.
        let has_construction = script.has_function("Construction");
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        let base_auto_sell = id.eq_ignore_ascii_case("GOLD");
        // The engine is single-threaded; Arc is shared ownership for host
        // contexts, not cross-thread transport (the script engine holds the
        // Rc-based GlobalNamed table).
        #[allow(clippy::arc_with_non_send_sync)]
        Ok(Self {
            id,
            name,
            version: DEFAULT_DEFINITION_VERSION,
            require_defs: Vec::new(),
            def_core_reflected_ints: HashMap::new(),
            description: None,
            script: Arc::new(script),
            base_script: compiled_script,
            script_source: source.to_string(),
            includes,
            includes_resolved: false,
            appends,
            has_construction,
            has_initialize,
            has_step,
            action_library: ActionLibrary::default(),
            callbacks_linked: false,
            action_graphics: HashMap::new(),
            crew_member: false,
            crew_member_value: 0,
            no_standard_crew: 0,
            silent_commands: false,
            can_be_base: false,
            clonk_names: None,
            clonk_names_owned: false,
            movement: MovementProfile::default(),
            category: DEFAULT_CATEGORY,
            max_user_select: 0,
            blit_mode: 0,
            color_by_owner: false,
            color_by_material: String::new(),
            allow_picture_stack: 0,
            graphics_scale: 1.0,
            ocf_base: OCF_NORMAL,
            value: 0,
            no_sell: 0,
            rebuyable: false,
            base_auto_sell,
            mass: 0,
            move_to_range: 0,
            pathfinder: 0,
            no_transfer_zones: 0,
            no_push_enter: 0,
            drag_image_picture: 0,
            picture: None,
            picture_image: None,
            portrait_image: None,
            portrait_graphics_image: None,
            portrait_graphics: Vec::new(),
            rank_symbols_image: None,
            rank_names: None,
            rank_base: None,
            rank_names_owned: false,
            rank_symbol_count: None,
            rank_symbols_owned: false,
            sprite_image: None,
            sprite_variants: HashMap::new(),
            shape: None,
            fire_top: 0,
            lift_top: 0,
            solid_mask: None,
            def_core_solid_mask: None,
            top_face: None,
            def_core_top_face: None,
            shape_vertices: Vec::new(),
            shape_vertex_slots: ShapeVertexBuffer::default(),
            contact_density: CONTACT_DENSITY_SOLID,
            contact_function_calls: false,
            collection_rect: None,
            def_core_collection_rect: None,
            collection_limit: 0,
            fragile: false,
            projectile: 0,
            explosive: 0,
            collectible: false,
            no_get: false,
            grab_put_get: 0,
            vehicle_control: 0,
            constructable: false,
            construction_offset: 0,
            stretch_growth: false,
            oversize: false,
            placement: 0,
            growth: 0,
            basement: 0,
            rotateable: 0,
            border_bound: 0,
            upright_attach: 0,
            rotated_solid_masks: false,
            auto_context_menu: false,
            needed_gfx_mode: 0,
            no_component_mass: false,
            no_stabilize: false,
            hide_hud_bars: 0,
            hide_hud_elements: 0,
            timer: 35,
            timer_call: None,
            timer_call_link: ScriptCallbackLink::default(),
            control_transfer_link: ScriptCallbackLink::default(),
            components: Vec::new(),
            line_connect: 0,
            contact_incinerate: 0,
            blast_incinerate: 0,
            contain_blast: 0,
            closed_container: 0,
            no_horizontal_move: 0,
            no_burn_decay: false,
            no_breath: false,
            temporary_crew: 0,
            smoke_rate: 100,
            float_line: 0,
            line: 0,
            line_intersect: 0,
            grab: 0,
            no_burn_damage: false,
            burn_turn_to: None,
            build_turn_to: None,
            incomplete_activity: false,
            exclusive: false,
            edible: false,
            prey: false,
            attract_lightning: false,
            entrance_rect: None,
            rotated_entrance: 0,
            no_fight: false,
            chopable: false,
            physical: PhysicalInfo::default(),
            c4_callback_args: false,
            solid_mask_pixels: SolidMaskPixels::default(),
            solid_mask_rect_cache: std::cell::RefCell::new(HashMap::new()),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_name(&mut self, name: String) {
        Arc::make_mut(&mut self.script).set_definition_name(name.clone());
        self.name = name;
    }

    pub fn version(&self) -> [i32; 5] {
        self.version
    }

    pub fn set_version(&mut self, version: [i32; 5]) {
        self.version = if definition_version_at_least(version, [4, 0, 0, 0]) {
            version
        } else {
            DEFAULT_DEFINITION_VERSION
        };
    }

    pub(crate) fn version_at_least(&self, required: [i32; 4]) -> bool {
        definition_version_at_least(self.version, required)
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Effective target `Context*` functions with their C4Aul description
    /// metadata. C++ enumerates the linked script list from `FuncL` backward
    /// and parses caption/Image/Condition/Desc from the raw description
    /// segments (C4Aul.cpp:357-379; C4AulParse.cpp:309-380).
    pub(crate) fn script_context_functions(&self) -> Vec<ScriptContextFunction> {
        self.script_menu_functions("Context")
            .into_iter()
            .filter(|function| function.has_description)
            .collect()
    }

    /// Effective script functions whose names start with `prefix`, in the
    /// reverse declaration order used by C4AulScript::GetSFunc(index,
    /// prefix). Unlike the public Context-menu projection above, native
    /// AddContextFunctions also enumerates functions with no description.
    pub(crate) fn script_menu_functions(&self, prefix: &str) -> Vec<ScriptContextFunction> {
        self.script
            .local_functions_in_get_sfunc_order()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, function)| {
                let mut metadata = script_context_function_metadata(function);
                if metadata.condition.as_ref().is_some_and(|condition| {
                    self.script.resolve_function(condition, true).is_none()
                }) {
                    // C4Aul stores the resolved condition pointer in the
                    // annotation. An unresolved name is therefore equivalent
                    // to omitting Condition, not a deferred failing call.
                    metadata.condition = None;
                }
                metadata
            })
            .collect()
    }

    pub(crate) fn script_menu_function(&self, name: &str) -> Option<ScriptContextFunction> {
        self.script.resolve_function(name, false).map(|resolution| {
            let mut metadata = script_context_function_metadata(resolution.function.as_ref());
            if metadata
                .condition
                .as_ref()
                .is_some_and(|condition| self.script.resolve_function(condition, true).is_none())
            {
                metadata.condition = None;
            }
            metadata
        })
    }

    pub fn set_description(&mut self, description: Option<String>) {
        self.description = description.filter(|text| !text.is_empty());
    }

    pub(crate) fn set_script_name(&mut self, script_name: impl Into<String>) {
        Arc::make_mut(&mut self.script).set_script_name(script_name);
    }

    pub(crate) fn set_game_script_name(&mut self, script_name: impl Into<String>) {
        Arc::make_mut(&mut self.script).set_game_script_name(script_name);
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.script.has_function(name)
    }

    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    pub fn appends(&self) -> &[clonk_script::AppendTo] {
        &self.appends
    }

    /// Recomputes the lifecycle-callback gates after script linking
    /// (appends/includes can introduce Construction/Initialize/Step).
    pub(crate) fn refresh_script_flags(&mut self) {
        let (has_construction, has_initialize, has_step) = if self.c4_callback_args {
            (
                self.script.has_local_function("Construction"),
                self.script.has_local_function("Initialize"),
                self.script.has_local_function("Step"),
            )
        } else {
            (
                self.script.has_function("Construction"),
                self.script.has_function("Initialize"),
                self.script.has_function("Step"),
            )
        };
        self.has_construction = has_construction;
        self.has_initialize = has_initialize;
        self.has_step = has_step;
    }

    pub(crate) fn mark_callbacks_unlinked(&mut self) {
        self.callbacks_linked = false;
        self.action_library.reset_callback_links();
        self.timer_call_link.reset();
        self.control_transfer_link.reset();
    }

    /// Cache ActMap, TimerCall and ControlTransfer functions after
    /// appends/includes are final. C++'s two GetSFunc layers each consume
    /// one leading failsafe marker.
    pub(crate) fn link_callbacks(&mut self) {
        if self.callbacks_linked {
            return;
        }

        let script = Arc::clone(&self.script);
        let definition = self.id.clone();
        let resolve = |configured: &str| {
            if configured.is_empty() {
                return None;
            }
            let function_name = configured.strip_prefix('~').unwrap_or(configured);
            let function_name = function_name.strip_prefix('~').unwrap_or(function_name);
            script
                .resolve_function(function_name, false)
                .map(|resolution| ScriptCallbackTarget::linked(function_name, resolution))
        };

        self.action_library
            .link_callbacks(|action, slot, configured| {
                let target = resolve(configured);
                if target.is_none() && !configured.is_empty() {
                    tracing::warn!(
                        definition = %definition,
                        "Error getting Action {}: {} function '{}'",
                        action,
                        slot,
                        configured
                    );
                }
                target
            });

        let timer_target = self.timer_call.as_deref().and_then(|configured| {
            let target = resolve(configured);
            if target.is_none() && !configured.is_empty() {
                tracing::warn!(
                    definition = %definition,
                    "Error getting TimerCall function '{}'",
                    configured
                );
            }
            target
        });
        self.timer_call_link.set_linked(timer_target);
        self.control_transfer_link
            .set_linked(resolve("~ControlTransfer"));
        self.callbacks_linked = true;
    }

    /// Restores the host to its own preparsed functions. C++ UnLink deletes
    /// linked include/append copies but retains original script functions and
    /// the engine-owned global value cells.
    pub(crate) fn reset_script_links(&mut self) {
        self.mark_callbacks_unlinked();
        Arc::make_mut(&mut self.script).replace_script_deferred(self.base_script.clone(), false);
        self.includes_resolved = false;
        if !self.rank_names_owned {
            self.rank_names = None;
            self.rank_base = None;
        }
        if !self.clonk_names_owned {
            self.clonk_names = None;
        }
        if !self.rank_symbols_owned {
            self.rank_symbols_image = None;
            self.rank_symbol_count = None;
        }
        self.refresh_script_flags();
    }

    pub(crate) fn replace_base_script(&mut self, source: &str, script: clonk_script::Script) {
        self.mark_callbacks_unlinked();
        self.includes = script.includes().to_vec();
        self.appends = script.appends().to_vec();
        self.script_source = source.to_owned();
        self.base_script = script;
        self.includes_resolved = false;
    }

    pub(crate) fn include_definition_metadata(&mut self, parent: &Definition) {
        if !self.clonk_names_owned {
            self.clonk_names = parent.clonk_names.clone();
        }
        if !self.rank_names_owned {
            self.rank_names = parent.rank_names.clone();
            self.rank_base = parent.rank_base;
        }
        if !self.rank_symbols_owned {
            self.rank_symbols_image = parent.rank_symbols_image.clone();
            self.rank_symbol_count = parent.rank_symbol_count;
        }
    }

    pub fn merge_from(&mut self, parent: &Definition) {
        self.mark_callbacks_unlinked();
        Arc::make_mut(&mut self.script).merge_from(&parent.script);
        self.include_definition_metadata(parent);
        self.refresh_script_flags();
    }

    pub fn function_count(&self) -> usize {
        self.script.function_count()
    }

    /// Distinct function roots plus all inherited overload nodes. Unlike
    /// [`Self::function_count`], this detects duplicate link copies.
    pub fn linked_function_count(&self) -> usize {
        self.script.linked_function_count()
    }

    pub fn from_resource(resource: &ResourceDefinitionData) -> Result<Self, EngineError> {
        let name = resource
            .core
            .name
            .clone()
            .unwrap_or_else(|| "Undefined".to_string());
        let mut definition =
            Definition::from_script(resource.core.id.clone(), name, resource.script.combined())?;
        definition.description = resource.description().map(str::to_owned);
        definition.set_clonk_names(resource.clonk_names.clone());
        // Real content gets the C++ callback arguments (no parameters;
        // AbortCall gets the last phase — C4Object.cpp:4154-4182).
        definition.set_c4_callback_convention(true);
        definition.set_version(resource.core.version);
        definition.require_defs = resource.core.require_defs.clone();

        if let Some(action_map) = &resource.action_map {
            let mut specs = HashMap::new();
            let mut physical_actions = Vec::with_capacity(action_map.actions.len());
            let mut visuals = HashMap::new();
            let mut reflections = HashMap::new();
            visuals.insert(
                PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
                DefinitionActionGraphics::default(),
            );
            for (index, (action_name, action_def)) in action_map.actions.iter().enumerate() {
                let (spec, graphics) = Self::convert_action_definition(action_def);
                physical_actions.push((action_name.clone(), spec.clone()));
                // SetActionByName and FnGetActMapVal both scan the physical
                // ActMap forward, so the first duplicate name wins.
                specs.entry(action_name.clone()).or_insert(spec);
                visuals
                    .entry(action_name.clone())
                    .or_insert_with(|| graphics.clone());
                visuals.insert(
                    physical_action_graphics_key(index.min(u32::MAX as usize) as u32),
                    graphics,
                );
                reflections.entry(action_name.clone()).or_insert_with(|| {
                    crate::action::C4ActionReflection::from_resource(action_name, action_def)
                });
            }
            // Real ActMaps carry no default action: C++ objects start
            // ActIdle (C4Object::Init). Only DSL manifests set one.
            let default_action = action_map.default_action.clone();
            definition.configure_actions(default_action.clone(), specs);
            definition.configure_physical_actions(physical_actions);
            definition.configure_action_reflections(reflections);
            definition.configure_action_graphics(visuals);
        }

        definition.set_crew_member_value(resource.core.crew_member);
        definition.no_standard_crew = resource.core.no_standard_crew;
        definition.set_silent_commands(resource.core.silent_commands);
        definition.set_category(resource.core.category);
        definition.max_user_select = resource.core.max_user_select;
        definition.set_blit_mode(resource.core.blit_mode);
        definition.set_color_by_owner(resource.core.color_by_owner);
        definition.color_by_material = resource.core.color_by_material.clone();
        definition.set_allow_picture_stack(resource.core.allow_picture_stack);
        definition.set_graphics_scale(resource.core.graphics_scale as f32 / 100.0);
        definition.set_value(resource.core.value);
        definition.set_no_sell(resource.core.no_sell);
        definition.set_rebuyable(resource.core.rebuyable);
        definition.set_base_auto_sell(resource.core.base_auto_sell);
        definition.set_mass(resource.core.mass);
        definition.set_picture(resource.core.picture.map(DefinitionPicture::from));
        definition.set_solid_mask(resource.core.solid_mask.map(DefinitionTargetRect::from));
        definition.set_top_face(resource.core.top_face.map(DefinitionTargetRect::from));
        if let Some(image) = resource.picture_image.as_ref() {
            definition.set_picture_image(Some(DefinitionPictureImage::from_resource(
                image,
                resource.picture_color_by_owner_mask.as_ref(),
            )));
        }
        if let Some(image) = resource.portrait_image.as_ref() {
            definition.set_portrait_image(Some(DefinitionPictureImage::from_resource(image, None)));
        }
        if let Some(image) = resource.portrait_graphics_image.as_ref() {
            definition.set_portrait_graphics_image(Some(DefinitionPictureImage::from_resource(
                image,
                resource.portrait_color_by_owner_mask.as_ref(),
            )));
        }
        definition.set_portrait_graphics(
            resource
                .portrait_graphics
                .iter()
                .map(|portrait| {
                    (
                        portrait.name.clone(),
                        DefinitionPictureImage::from_resource(
                            &portrait.image,
                            portrait.color_by_owner_mask.as_ref(),
                        ),
                    )
                })
                .collect(),
        );
        if let Some(image) = resource.rank_symbols_image.as_ref() {
            definition
                .set_rank_symbols_image(Some(DefinitionPictureImage::from_resource(image, None)));
        }
        definition.set_rank_name_table(resource.rank_names.clone(), resource.rank_base);
        definition.set_rank_symbol_count(resource.rank_symbol_count);
        if let Some(image) = resource.graphics_image.as_ref() {
            let mask = resource.color_by_owner_mask.as_ref();
            definition.set_sprite_image(Some(DefinitionSpriteImage::from_resource(image, mask)));
        }
        definition.validate_base_graphics_rects();
        if !resource.additional_graphics.is_empty() {
            let mut variants = HashMap::with_capacity(resource.additional_graphics.len());
            for (key, variant) in &resource.additional_graphics {
                let mask = variant.color_by_owner_mask.as_ref();
                variants.insert(
                    key.clone(),
                    DefinitionSpriteImage::from_resource(&variant.image, mask),
                );
            }
            definition.set_sprite_variants(variants);
        }
        definition.set_shape_rect(resource.core.shape.map(DefinitionRect::from));
        definition.set_fire_top(resource.core.fire_top);
        definition.set_lift_top(resource.core.lift_top);
        definition.set_shape_vertex_slots(
            resource.core.vertices.len(),
            &resource
                .core
                .vertex_slots
                .iter()
                .map(|vertex| {
                    ObjectVertex::new(vertex.x, vertex.y)
                        .with_cnat(vertex.cnat)
                        .with_friction(vertex.friction)
                })
                .collect::<Vec<_>>(),
        );
        definition.set_contact_density(resource.core.contact_density);
        definition.set_contact_function_calls(resource.core.contact_function_calls);
        definition.set_collection_rect(resource.core.collection.map(DefinitionRect::from));
        definition.set_collection_limit(resource.core.collection_limit);
        definition.set_fragile(resource.core.fragile);
        definition.set_projectile(resource.core.projectile);
        definition.explosive = resource.core.explosive;
        definition.set_fire_properties(
            resource.core.contact_incinerate,
            resource.core.no_burn_decay,
            resource.core.no_burn_damage,
        );
        definition.set_blast_incinerate(resource.core.blast_incinerate);
        definition.set_contain_blast(resource.core.contain_blast);
        definition.set_closed_container(resource.core.closed_container);
        definition.set_no_horizontal_move(resource.core.no_horizontal_move);
        definition.set_burn_turn_to(resource.core.burn_turn_to.clone());
        definition.set_build_turn_to(resource.core.build_turn_to.clone());
        definition.set_incomplete_activity(resource.core.incomplete_activity);
        definition.set_no_breath(resource.core.no_breath);
        definition.temporary_crew = resource.core.temporary_crew;
        definition.smoke_rate = resource.core.smoke_rate;
        definition.set_grab(resource.core.grab);
        definition.set_move_to_range(resource.core.move_to_range);
        definition.set_pathfinder(resource.core.pathfinder);
        definition.set_no_transfer_zones(resource.core.no_transfer_zones);
        definition.set_no_push_enter(resource.core.no_push_enter);
        definition.drag_image_picture = resource.core.drag_image_picture;
        definition.float_line = resource.core.float_line;
        definition.set_line(resource.core.line);
        definition.set_line_intersect(resource.core.line_intersect);
        definition.set_physical(resource.core.physical);
        definition.set_collectible(resource.core.collectible);
        definition.set_no_get(resource.core.no_get != 0);
        definition.set_grab_put_get(resource.core.grab_put_get);
        definition.set_vehicle_control(resource.core.vehicle_control);
        definition.set_constructable(resource.core.constructable);
        definition.set_can_be_base(resource.core.can_be_base);
        definition.set_construction_offset(resource.core.con_size_off);
        definition.set_stretch_growth(resource.core.stretch_growth);
        definition.set_oversize(resource.core.oversize);
        definition.set_placement(resource.core.placement);
        definition.set_growth(resource.core.growth);
        definition.set_basement(resource.core.basement);
        definition.set_rotateable(resource.core.rotateable);
        definition.set_border_bound(resource.core.border_bound);
        definition.set_upright_attach(resource.core.upright_attach);
        definition.set_rotated_solid_masks(resource.core.rotated_solid_masks);
        definition.set_auto_context_menu(resource.core.auto_context_menu);
        definition.needed_gfx_mode = resource.core.needed_gfx_mode;
        definition.set_no_component_mass(resource.core.no_component_mass);
        definition.set_no_stabilize(resource.core.no_stabilize);
        definition.hide_hud_bars = resource.core.hide_hud_bars;
        definition.hide_hud_elements = resource.core.hide_hud_elements;
        definition.set_timer(resource.core.timer);
        definition.set_timer_call(resource.core.timer_call.clone());
        if !resource.core.components.is_empty() {
            let components = resource
                .core
                .components
                .iter()
                .map(|component| DefinitionComponent {
                    id: component.id.clone(),
                    count: component.count,
                })
                .collect();
            definition.set_components(components);
        }
        definition.set_line_connect(resource.core.line_connect);
        definition.set_exclusive(resource.core.exclusive);
        definition.set_edible(resource.core.edible);
        definition.set_prey(resource.core.prey);
        definition.set_attract_lightning(resource.core.attract_lightning);
        definition.set_entrance_rect(resource.core.entrance.map(DefinitionRect::from));
        definition.set_rotated_entrance(resource.core.rotated_entrance);
        definition.set_no_fight(resource.core.no_fight);
        definition.set_chopable(resource.core.chopable);
        definition.def_core_reflected_ints = resource.core.reflected_ints.clone();
        Ok(definition)
    }

    fn convert_action_definition(
        action: &ResourceActionDefinition,
    ) -> (ActionSpec, DefinitionActionGraphics) {
        let mut spec = ActionSpec::default();
        if let Some(procedure) = action.procedure.as_deref().and_then(|procedure| {
            clonk_resources::definition::PROCEDURE_NAMES
                .iter()
                .find(|candidate| **candidate == procedure)
        }) {
            spec = spec.with_procedure(*procedure);
        }
        if let Some(length) = action.length {
            spec = spec.with_length(length);
        }
        if let Some(next) = &action.next_action {
            spec = spec.with_next(next.clone());
        }
        spec = spec.with_next_index(action.next_action_index);
        if let Some(delay) = action.delay {
            spec = spec.with_delay(delay);
        }
        if let Some(step) = action.step {
            spec = spec.with_step(step);
        }
        if let Some(phase) = &action.phase_call {
            spec = spec.with_phase_call(phase.clone());
        }
        if let Some(start) = &action.start_call {
            spec = spec.with_start_call(start.clone());
        }
        if let Some(end) = &action.end_call {
            spec = spec.with_end_call(end.clone());
        }
        if let Some(abort) = &action.abort_call {
            spec = spec.with_abort_call(abort.clone());
        }
        if action.no_other_action {
            spec = spec.with_no_other_action(true);
        }
        if action.disabled {
            spec = spec.with_disabled(true);
        }
        if action.energy_usage != 0 {
            spec = spec.with_energy_usage(action.energy_usage);
        }
        if let Some(in_liquid_action) = &action.in_liquid_action {
            spec = spec.with_in_liquid_action(in_liquid_action.clone());
        }
        if let Some(directions) = action.directions {
            spec = spec.with_directions(directions);
        }
        if let Some(turn_action) = &action.turn_action {
            spec = spec.with_turn_action(turn_action.clone());
        }
        if let Some(sound) = &action.sound {
            spec = spec.with_sound(sound.clone());
        }
        if let Some(dig_free) = action.dig_free {
            spec = spec.with_dig_free(dig_free);
        }
        if action.attach != 0 {
            spec = spec.with_attach(action.attach);
        }
        let mut graphics = DefinitionActionGraphics::default();
        graphics.length = action.length;
        graphics.directions = action.directions.unwrap_or(1);
        graphics.flip_dir = action.flip_dir;
        graphics.reverse = action.reverse;
        graphics.facet_base = action.facet_base;
        graphics.facet_top_face = action.facet_top_face;
        graphics.facet_target_stretch = action.facet_target_stretch;
        graphics.facet = action.facet.as_ref().map(Self::convert_action_facet);
        (spec, graphics)
    }

    fn convert_action_facet(facet: &ResourceActionFacet) -> DefinitionActionFacet {
        DefinitionActionFacet {
            x: facet.x,
            y: facet.y,
            width: facet.width,
            height: facet.height,
            target_x: facet.target_x,
            target_y: facet.target_y,
        }
    }

    pub fn set_debugger_hooks(&mut self, hooks: DebuggerHooks) {
        Arc::make_mut(&mut self.script).set_debugger_hooks(hooks);
    }

    /// Switch this definition's engine callbacks to the C++ argument
    /// convention (C4Object.cpp:4154-4182). Real content loaded from
    /// resources runs this way; synthetic command-DSL fixtures do not.
    pub fn set_c4_callback_convention(&mut self, enabled: bool) {
        self.c4_callback_args = enabled;
        self.refresh_script_flags();
    }

    /// Shared handle to the compiled script for nested script calls
    /// (Find_Func/GameCall targets resolve functions on the target object's
    /// own definition script, C4Aul.cpp:130-148).
    pub(crate) fn script_arc(&self) -> Arc<ScriptEngine> {
        Arc::clone(&self.script)
    }

    /// Shares the System.c4g global-function table into this script host.
    pub(crate) fn set_global_functions(
        &mut self,
        functions: Option<Arc<HashMap<String, clonk_script::Function>>>,
    ) {
        Arc::make_mut(&mut self.script).set_global_functions(functions);
    }

    pub fn configure_actions(
        &mut self,
        default_action: Option<String>,
        specs: HashMap<String, ActionSpec>,
    ) {
        self.action_library = ActionLibrary::new(default_action, specs);
        self.mark_callbacks_unlinked();
    }

    pub(crate) fn configure_physical_actions(&mut self, actions: Vec<(String, ActionSpec)>) {
        self.action_library.set_physical_actions(actions);
        self.mark_callbacks_unlinked();
    }

    pub(crate) fn configure_action_reflections(
        &mut self,
        reflections: HashMap<String, action::C4ActionReflection>,
    ) {
        self.action_library.set_reflections(reflections);
    }

    pub fn configure_action_graphics(
        &mut self,
        graphics: HashMap<String, DefinitionActionGraphics>,
    ) {
        self.action_graphics = graphics;
    }

    pub fn action_library(&self) -> &ActionLibrary {
        &self.action_library
    }

    fn shared_action_library(&self, world: &HostWorldContext) -> SharedActionLibrary {
        world
            .definition_metadata(self.id.as_str())
            .map(|metadata| metadata.action_library.clone())
            .unwrap_or_else(|| self.action_library.clone().into())
    }

    pub fn action_graphics(&self) -> &HashMap<String, DefinitionActionGraphics> {
        &self.action_graphics
    }

    pub fn graphics_for_action(&self, action: &str) -> Option<&DefinitionActionGraphics> {
        self.action_graphics.get(action)
    }

    pub fn default_action_state(&self) -> ActionState {
        ActionState::new(self.action_library.default_action())
    }

    pub fn is_crew(&self) -> bool {
        self.crew_member
    }

    pub fn crew_member_value(&self) -> i32 {
        self.crew_member_value
    }

    pub fn can_be_base(&self) -> bool {
        self.can_be_base
    }

    pub fn set_can_be_base(&mut self, can_be_base: bool) {
        self.can_be_base = can_be_base;
    }

    pub fn clonk_names(&self) -> Option<&str> {
        self.clonk_names.as_deref()
    }

    pub fn set_clonk_names(&mut self, clonk_names: Option<String>) {
        self.clonk_names_owned = clonk_names.is_some();
        self.clonk_names = clonk_names;
    }

    pub fn set_crew_member(&mut self, crew_member: bool) {
        self.crew_member = crew_member;
        self.crew_member_value = i32::from(crew_member);
    }

    /// Preserve C4DefCore's raw signed integer while deriving the boolean
    /// capability used by OCF and player-crew behavior.
    pub fn set_crew_member_value(&mut self, crew_member: i32) {
        self.crew_member = crew_member != 0;
        self.crew_member_value = crew_member;
    }

    pub fn silent_commands(&self) -> bool {
        self.silent_commands
    }

    pub fn set_silent_commands(&mut self, silent_commands: bool) {
        self.silent_commands = silent_commands;
    }

    pub fn movement_profile(&self) -> MovementProfile {
        self.movement
    }

    pub fn set_movement_profile(&mut self, movement: MovementProfile) {
        self.movement = movement;
    }

    pub fn category(&self) -> i32 {
        self.category
    }

    pub fn set_category(&mut self, category: i32) {
        self.category = normalize_category(category, DEFAULT_CATEGORY);
    }

    pub fn blit_mode(&self) -> u32 {
        self.blit_mode
    }

    pub fn set_blit_mode(&mut self, blit_mode: u32) {
        self.blit_mode = blit_mode;
    }

    pub fn color_by_owner(&self) -> bool {
        self.color_by_owner
    }

    pub fn set_color_by_owner(&mut self, color_by_owner: bool) {
        self.color_by_owner = color_by_owner;
    }

    pub fn allow_picture_stack(&self) -> i32 {
        self.allow_picture_stack
    }

    pub fn set_allow_picture_stack(&mut self, allow_picture_stack: i32) {
        self.allow_picture_stack = allow_picture_stack;
    }

    pub fn graphics_scale(&self) -> f32 {
        self.graphics_scale
    }

    pub fn set_graphics_scale(&mut self, graphics_scale: f32) {
        self.graphics_scale = graphics_scale.max(0.0);
    }

    pub fn ocf_base(&self) -> u32 {
        let mut ocf = self.ocf_base;
        if self.rotateable != 0 {
            ocf |= crate::ocf::ROTATE;
        }
        // OCF_Grab from the Grab DefCore value (C4Object SetOCF).
        if self.grab != 0 {
            ocf |= crate::ocf::GRAB;
        }
        ocf
    }

    pub fn set_ocf_base(&mut self, ocf: u32) {
        self.ocf_base = ocf | OCF_NORMAL;
    }

    pub fn rotateable(&self) -> i32 {
        self.rotateable
    }

    pub fn set_rotateable(&mut self, rotateable: i32) {
        self.rotateable = rotateable;
    }

    /// The def+state arm of C4Object::SetOCF (C4Object.cpp:526-666).
    /// Context-dependent bits (HitSpeeds, Chop, InSolid, InFree, Available)
    /// join in `Engine::compute_object_ocf`. The raw `ocf_base` seed is the
    /// fixture shortcut for def flags this model does not carry.
    pub fn compute_ocf(&self, state: &ObjectState) -> u32 {
        self.compute_ocf_with_contents_count(state, state.contents.len())
    }

    /// Live SetOCF variant. `C4ObjectList::ObjectCount` ignores removed list
    /// holes but retains C4OS_INACTIVE entries, so Engine callers supply the
    /// count resolved against the authoritative object table.
    pub(crate) fn compute_ocf_with_contents_count(
        &self,
        state: &ObjectState,
        contents_count: usize,
    ) -> u32 {
        // OCF_Normal: the OCF is never zero (SetOCF, C4Object.cpp:547-548)
        let mut ocf = self.ocf_base | OCF_NORMAL;
        // OCF_NotContained (SetOCF, C4Object.cpp:627-629); OCF_Available
        // joins in Engine::compute_object_ocf with its container and
        // landscape clauses (C4Object.cpp:645-648).
        if state.container.is_none() {
            ocf |= crate::ocf::NOT_CONTAINED;
        }
        // OCF_FullCon (SetOCF, C4Object.cpp:567-569)
        if state.construction >= FULL_CON {
            ocf |= crate::ocf::FULL_CON;
        }
        // OCF_Living/OCF_Alive (SetOCF, C4Object.cpp:600-605)
        if state.category & CATEGORY_LIVING != 0 {
            ocf |= crate::ocf::LIVING;
            if state.alive {
                ocf |= crate::ocf::ALIVE;
            }
        }
        // OCF_Prey: Def->Prey && the RAW Alive flag (SetOCF,
        // C4Object.cpp:615-618)
        if self.prey && state.alive {
            ocf |= crate::ocf::PREY;
        }
        // OCF_CrewMember: Def->CrewMember && the RAW Alive flag
        // (SetOCF, C4Object.cpp:619-622)
        if self.crew_member && state.alive {
            ocf |= crate::ocf::CREW_MEMBER;
        }
        // OCF_AttractLightning at FullCon (SetOCF, C4Object.cpp:623-626)
        if self.attract_lightning && ocf & crate::ocf::FULL_CON != 0 {
            ocf |= crate::ocf::ATTRACT_LIGHTNING;
        }
        // OCF_Edible (SetOCF, C4Object.cpp:630-632)
        if self.edible {
            ocf |= crate::ocf::EDIBLE;
        }
        // OCF_FightReady: the OCF_Alive BIT, an action without
        // ObjectDisabled, and !Def->NoFight (SetOCF, C4Object.cpp:606-610).
        if ocf & crate::ocf::ALIVE != 0
            && !self
                .action_library
                .disables_object_for_entry(&state.action.name, state.action.act_map_index)
            && !self.no_fight
        {
            ocf |= crate::ocf::FIGHT_READY;
        }
        // OCF_Construct: can be built outside (SetOCF, C4Object.cpp:549-552)
        if self.constructable
            && state.construction < FULL_CON
            && state.rotation == 0
            && !state.on_fire
        {
            ocf |= crate::ocf::CONSTRUCT;
        }
        // OCF_Rotate: rotateable, but not a minimum (invisible)
        // construction site (SetOCF, C4Object.cpp:576-580)
        if self.rotateable != 0 && state.construction > 100 {
            ocf |= crate::ocf::ROTATE;
        }
        // OCF_Exclusive (SetOCF, C4Object.cpp:581-583)
        if self.exclusive {
            ocf |= crate::ocf::EXCLUSIVE;
        }
        // OCF_Entrance: positive entrance area, FullCon, and the
        // RotatedEntrance rotation gate (SetOCF, C4Object.cpp:584-587)
        if self
            .entrance_rect
            .is_some_and(|rect| rect.width > 0 && rect.height > 0)
            && ocf & crate::ocf::FULL_CON != 0
            && (self.rotated_entrance == 1 || state.rotation <= self.rotated_entrance)
        {
            ocf |= crate::ocf::ENTRANCE;
        }
        // OCF_Grab: Grab DefCore value, never on StaticBack objects
        // (SetOCF, C4Object.cpp:553-555)
        if self.grab != 0 && state.category & CATEGORY_STATIC_BACK == 0 {
            ocf |= crate::ocf::GRAB;
        }
        if self.collectible {
            ocf |= crate::ocf::CARRYABLE;
        }
        // OCF_OnFire (SetOCF, C4Object.cpp:559-561)
        if state.on_fire {
            ocf |= crate::ocf::ON_FIRE;
        }
        // OCF_InLiquid: the cached flag, uncontained only (SetOCF
        // C4Object.cpp:633-636, UpdateOCF :729-732).
        if state.in_liquid && state.container.is_none() {
            ocf |= crate::ocf::IN_LIQUID;
        }
        // OCF_Inflammable: not burning, ContactIncinerate set, not a dead
        // living (SetOCF, C4Object.cpp:562-566)
        if !state.on_fire
            && self.contact_incinerate > 0
            && (state.category & CATEGORY_LIVING == 0 || state.alive)
        {
            ocf |= crate::ocf::INFLAMMABLE;
        }
        if self.collection_ocf_enabled(state, contents_count, state.no_collect_delay) {
            ocf |= crate::ocf::COLLECTION;
        }
        // OCF_LineConstruct: FullCon + any LineConnect bit besides
        // C4D_EnergyHolder (SetOCF, C4Object.cpp:611-614)
        if ocf & crate::ocf::FULL_CON != 0 && self.line_connect & !LINE_CONNECT_ENERGY_HOLDER != 0 {
            ocf |= crate::ocf::LINE_CONSTRUCT;
        }
        // OCF_PowerConsumer (SetOCF, C4Object.cpp:649-652)
        if self.line_connect & LINE_CONNECT_POWER_CONSUMER != 0 && ocf & crate::ocf::FULL_CON != 0 {
            ocf |= crate::ocf::POWER_CONSUMER;
        }
        // OCF_PowerSupply: a generator, or an energized power output
        // (SetOCF, C4Object.cpp:653-657)
        if (self.line_connect & LINE_CONNECT_POWER_GENERATOR != 0
            || (self.line_connect & LINE_CONNECT_POWER_OUTPUT != 0 && state.energy > 0))
            && ocf & crate::ocf::FULL_CON != 0
        {
            ocf |= crate::ocf::POWER_SUPPLY;
        }
        // OCF_Container: Grab_Put, Grab_Get or an open entrance (SetOCF,
        // C4Object.cpp:658-660)
        if self.grab_put_get & (GRAB_PUT_GET_PUT | GRAB_PUT_GET_GET) != 0
            || ocf & crate::ocf::ENTRANCE != 0
        {
            ocf |= crate::ocf::CONTAINER;
        }
        ocf
    }

    /// The OCF_Collection bit decision (SetOCF, C4Object.cpp:593-599), including
    /// the raw fixture seed and explicit Contents and NoCollectDelay values.
    /// Host-context previews can therefore substitute those scalars without
    /// cloning the full state.
    pub(crate) fn collection_ocf_enabled(
        &self,
        state: &ObjectState,
        contents_len: usize,
        no_collect_delay: i32,
    ) -> bool {
        if self.ocf_base & crate::ocf::COLLECTION != 0 {
            return true;
        }
        (self.ocf_base & crate::ocf::FULL_CON != 0
            || state.construction >= FULL_CON
            || self.incomplete_activity)
            && self.collection_rect.is_some_and(|rect| rect.is_positive())
            && !collection_limit_reached(self.collection_limit, contents_len)
            && !self
                .action_library
                .disables_object_for_entry(&state.action.name, state.action.act_map_index)
            && no_collect_delay == 0
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, value: i32) {
        self.def_core_reflected_ints
            .insert("Value".to_string(), value);
        self.value = value;
    }

    pub fn no_sell(&self) -> i32 {
        self.no_sell
    }

    pub fn set_no_sell(&mut self, no_sell: i32) {
        self.no_sell = no_sell;
    }

    pub fn rebuyable(&self) -> bool {
        self.rebuyable
    }

    pub fn set_rebuyable(&mut self, rebuyable: bool) {
        self.rebuyable = rebuyable;
    }

    pub fn base_auto_sell(&self) -> bool {
        self.base_auto_sell
    }

    pub fn set_base_auto_sell(&mut self, base_auto_sell: bool) {
        self.base_auto_sell = base_auto_sell;
    }

    pub fn mass(&self) -> i32 {
        self.mass
    }

    pub fn set_mass(&mut self, mass: i32) {
        self.mass = mass.max(0);
    }

    pub fn move_to_range(&self) -> i32 {
        self.move_to_range
    }

    pub fn set_move_to_range(&mut self, move_to_range: i32) {
        self.move_to_range = move_to_range;
    }

    pub fn pathfinder(&self) -> i32 {
        self.pathfinder
    }

    pub fn set_pathfinder(&mut self, pathfinder: i32) {
        self.pathfinder = pathfinder;
    }

    pub fn no_transfer_zones(&self) -> i32 {
        self.no_transfer_zones
    }

    pub fn set_no_transfer_zones(&mut self, no_transfer_zones: i32) {
        self.no_transfer_zones = no_transfer_zones;
    }

    pub fn no_push_enter(&self) -> i32 {
        self.no_push_enter
    }

    pub fn set_no_push_enter(&mut self, no_push_enter: i32) {
        self.no_push_enter = no_push_enter;
    }

    /// `Grab` DefCore value (C4Def.h): 0 = not grabbable, 1 = grab and
    /// push, 2 = grab only (C4Object.cpp:1763).
    pub fn grab(&self) -> i32 {
        self.grab
    }

    pub fn set_grab(&mut self, grab: i32) {
        self.grab = grab;
    }

    pub fn picture(&self) -> Option<DefinitionPicture> {
        self.picture
    }

    pub fn set_picture(&mut self, picture: Option<DefinitionPicture>) {
        self.picture = picture;
    }

    pub fn picture_image(&self) -> Option<&DefinitionPictureImage> {
        self.picture_image.as_ref()
    }

    pub fn set_picture_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.picture_image = image;
    }

    /// First def portrait (C4CFN_Portraits, src/C4Components.h:88).
    pub fn portrait_image(&self) -> Option<&DefinitionPictureImage> {
        self.portrait_image.as_ref()
    }

    pub fn set_portrait_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.portrait_image = image;
    }

    pub fn portrait_graphics_image(&self) -> Option<&DefinitionPictureImage> {
        self.portrait_graphics_image.as_ref()
    }

    pub fn set_portrait_graphics_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.portrait_graphics_image = image;
    }

    pub fn portrait_graphics(&self, name: &str) -> Option<&DefinitionPictureImage> {
        self.portrait_graphics
            .iter()
            .find(|(candidate, _)| clonk_resources::material::c4_names_equal(candidate, name))
            .map(|(_, image)| image)
    }

    pub(crate) fn portrait_graphics_names(&self) -> impl Iterator<Item = &str> {
        self.portrait_graphics.iter().map(|(name, _)| name.as_str())
    }

    pub fn set_portrait_graphics(&mut self, portraits: Vec<(String, DefinitionPictureImage)>) {
        self.portrait_graphics = portraits;
    }

    /// Def rank symbols (C4Def::pRankSymbols, src/C4Def.cpp:684-691).
    pub fn rank_symbols_image(&self) -> Option<&DefinitionPictureImage> {
        self.rank_symbols_image.as_ref()
    }

    pub fn set_rank_symbols_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.rank_symbols_owned = image.is_some();
        self.rank_symbols_image = image;
    }

    pub fn rank_names(&self) -> Option<&RankNameTable> {
        self.rank_names.as_ref()
    }

    pub fn set_rank_names(&mut self, names: Option<Vec<String>>) {
        let base = names.as_ref().map(|_| 1_000);
        self.set_rank_system(names, base);
    }

    pub fn rank_base(&self) -> Option<i32> {
        self.rank_base
    }

    pub fn set_rank_system(&mut self, names: Option<Vec<String>>, rank_base: Option<i32>) {
        self.set_rank_name_table(names.map(RankNameTable::from_resolved_names), rank_base);
    }

    pub(crate) fn set_rank_name_table(
        &mut self,
        names: Option<RankNameTable>,
        rank_base: Option<i32>,
    ) {
        self.rank_names_owned = names.is_some();
        self.rank_names = names;
        self.rank_base = self.rank_names.as_ref().map(|_| match rank_base {
            Some(0) | None => 1_000,
            Some(base) => base,
        });
    }

    pub fn rank_symbol_count(&self) -> Option<u32> {
        self.rank_symbol_count
    }

    pub fn set_rank_symbol_count(&mut self, count: Option<u32>) {
        self.rank_symbol_count = count;
    }

    /// C4Def::ColorizeByMaterial / C4DefGraphics::ColorizeByMaterial:
    /// recolor every surface in the base/additional/portrait graphics chain.
    /// Rust retains separate cropped presentation images, so recolor those
    /// copies too; otherwise C4Def::Picture and the first portrait would keep
    /// showing the pre-colorization bitmap.
    pub(crate) fn colorize_by_material(&mut self, materials: &MaterialSet) {
        if self.color_by_material.is_empty() {
            return;
        }
        let Some(material) = materials.get(&self.color_by_material) else {
            tracing::error!(
                "C4Def::ColorizeByMaterial: mat {} not defined",
                self.color_by_material
            );
            return;
        };
        let colors = c4_material_definition_colors(material);

        if let Some(image) = self.sprite_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        for image in self.sprite_variants.values_mut() {
            image.colorize_by_material(&colors);
        }
        if let Some(image) = self.picture_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        if let Some(image) = self.portrait_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        if let Some(image) = self.portrait_graphics_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        for (_, image) in &mut self.portrait_graphics {
            image.colorize_by_material(&colors);
        }

        // Material alpha can change the collision mask derived from the base
        // sprite. Rebuilding also drops every lazily cached named mask.
        self.rebuild_solid_mask_pixels();
    }

    pub fn sprite_image(&self) -> Option<&DefinitionSpriteImage> {
        self.sprite_image.as_ref()
    }

    pub fn set_sprite_image(&mut self, image: Option<DefinitionSpriteImage>) {
        self.sprite_image = image;
        self.rebuild_solid_mask_pixels();
    }

    /// C4Def::Load clears invalid BASE DefCore graphics rectangles before
    /// objects copy them or the renderer sees TopFace (C4Def.cpp:727-741).
    pub(crate) fn validate_base_graphics_rects(&mut self) {
        let Some(image) = self.sprite_image.as_ref() else {
            return;
        };
        let image_width = i64::from(image.width);
        let image_height = i64::from(image.height);

        let invalid_solid_mask = self.def_core_solid_mask.is_some_and(|solid_mask| {
            solid_mask.x < 0
                || solid_mask.y < 0
                || i64::from(solid_mask.x) + i64::from(solid_mask.width) > image_width
                || i64::from(solid_mask.y) + i64::from(solid_mask.height) > image_height
        });
        if invalid_solid_mask {
            self.set_solid_mask(None);
        }

        let logical_width = image_width as f32 / self.graphics_scale;
        let logical_height = image_height as f32 / self.graphics_scale;
        let invalid_top_face = self.def_core_top_face.is_some_and(|top_face| {
            top_face.x < 0
                || top_face.y < 0
                || (i64::from(top_face.x) + i64::from(top_face.width)) as f32 > logical_width
                || (i64::from(top_face.y) + i64::from(top_face.height)) as f32 > logical_height
        });
        if invalid_top_face {
            tracing::warn!(
                definition = %self.id,
                name = %self.name,
                "invalid TopFace; cleared"
            );
            self.set_top_face(None);
        }
    }

    pub fn sprite_image_variant(
        &self,
        graphics_name: Option<&str>,
    ) -> Option<&DefinitionSpriteImage> {
        match graphics_name {
            None | Some("") => self.sprite_image.as_ref(),
            Some(name) => {
                let key = clonk_resources::material::c4_name_key(name);
                self.sprite_variants.get(&key)
            }
        }
    }

    pub fn set_sprite_variants(&mut self, variants: HashMap<String, DefinitionSpriteImage>) {
        self.sprite_variants = variants;
        self.solid_mask_rect_cache.borrow_mut().clear();
    }

    pub fn sprite_variant_keys(&self) -> Vec<String> {
        self.sprite_variants.keys().cloned().collect()
    }

    pub fn shape_rect(&self) -> Option<DefinitionRect> {
        self.shape
    }

    /// `Float` DefCore value (C4Def.cpp:379) — the buoyancy line.
    pub fn line(&self) -> i32 {
        self.line
    }

    pub fn set_line(&mut self, line: i32) {
        self.line = line;
    }

    pub fn line_intersect(&self) -> i32 {
        self.line_intersect
    }

    pub fn set_line_intersect(&mut self, line_intersect: i32) {
        self.line_intersect = line_intersect;
    }

    pub fn set_float_line(&mut self, float_line: i32) {
        self.float_line = float_line;
    }

    pub fn set_shape_rect(&mut self, rect: Option<DefinitionRect>) {
        self.shape = rect;
    }

    pub fn solid_mask(&self) -> Option<DefinitionTargetRect> {
        self.solid_mask
    }

    pub fn set_solid_mask(&mut self, rect: Option<DefinitionTargetRect>) {
        self.def_core_solid_mask = rect;
        self.solid_mask = rect.filter(DefinitionTargetRect::is_positive);
        self.rebuild_solid_mask_pixels();
    }

    pub fn top_face(&self) -> Option<DefinitionTargetRect> {
        self.top_face
    }

    pub fn set_top_face(&mut self, rect: Option<DefinitionTargetRect>) {
        self.def_core_top_face = rect;
        self.top_face = rect.filter(DefinitionTargetRect::is_positive);
    }

    /// Per-pixel decode for an ARBITRARY mask rect — Objects.txt
    /// SolidMask= overrides pick a different sprite region per object
    /// (C4Object::SolidMask; the CTWR platform has NO DefCore mask at
    /// all). Cached per rect.
    pub(crate) fn solid_mask_pixels_for_rect(
        &self,
        mask: DefinitionTargetRect,
        graphics_name: Option<&str>,
    ) -> SolidMaskPixels {
        let graphics_key = graphics_name
            .filter(|name| !name.is_empty())
            .map(clonk_resources::material::c4_name_key);
        if graphics_key.is_none() && Some(mask) == self.solid_mask {
            return self.solid_mask_pixels.clone();
        }
        let key = (
            graphics_key.clone(),
            mask.x,
            mask.y,
            mask.width,
            mask.height,
        );
        if let Some(cached) = self.solid_mask_rect_cache.borrow().get(&key) {
            return cached.clone();
        }
        let computed = self.compute_solid_mask_pixels(mask, graphics_key.as_deref());
        self.solid_mask_rect_cache
            .borrow_mut()
            .insert(key, computed.clone());
        computed
    }

    fn compute_solid_mask_pixels(
        &self,
        mask: DefinitionTargetRect,
        graphics_name: Option<&str>,
    ) -> SolidMaskPixels {
        let Some(image) = self.sprite_image_variant(graphics_name) else {
            return if graphics_name.is_none() {
                SolidMaskPixels::Rectangle
            } else {
                // C++ SetGraphics rejects an unknown named graphic. Never
                // silently substitute the owning definition's default bitmap.
                SolidMaskPixels::OutOfBounds
            };
        };
        let image_width = i32::try_from(image.width).unwrap_or(i32::MAX);
        let image_height = i32::try_from(image.height).unwrap_or(i32::MAX);
        let pixels = image.solid_mask_source_pixels();
        solid_mask_pixels_for_checked_bitmap(mask, image_width, image_height, pixels.as_ref())
            .map(SolidMaskPixels::Alpha)
            .unwrap_or(SolidMaskPixels::OutOfBounds)
    }

    /// Extract the SolidMask alpha pixels from the sprite (alpha != 0 =
    /// solid), mirroring the per-tick scan this replaces.
    fn rebuild_solid_mask_pixels(&mut self) {
        self.solid_mask_rect_cache.borrow_mut().clear();
        let Some(mask) = self.solid_mask else {
            self.solid_mask_pixels = SolidMaskPixels::default();
            return;
        };
        if let Some(image) = self.sprite_image.as_ref() {
            let right = i64::from(mask.x) + i64::from(mask.width);
            let bottom = i64::from(mask.y) + i64::from(mask.height);
            if mask.x < 0
                || mask.y < 0
                || right > i64::from(image.width)
                || bottom > i64::from(image.height)
            {
                // The definition-level cache holds the RAW DefCore rect.
                // Object Init/runtime checks persist a distinct checked rect
                // and therefore bypass this entry. Keep malformed raw sizes
                // from allocating before C4Def::Load validation runs.
                self.solid_mask_pixels = SolidMaskPixels::OutOfBounds;
                return;
            }
        }
        self.solid_mask_pixels = self.compute_solid_mask_pixels(mask, None);
    }

    pub fn shape_vertices(&self) -> &[ObjectVertex] {
        &self.shape_vertices
    }

    pub fn set_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.shape_vertex_slots = ShapeVertexBuffer::from_active(&vertices);
        self.shape_vertices = vertices;
    }

    pub(crate) fn set_shape_vertex_slots(&mut self, active_count: usize, slots: &[ObjectVertex]) {
        self.shape_vertex_slots = ShapeVertexBuffer::from_slots(active_count, slots);
        self.shape_vertices = self.shape_vertex_slots.active_vec();
    }

    pub(crate) fn shape_vertex_buffer(&self) -> &ShapeVertexBuffer {
        &self.shape_vertex_slots
    }

    pub fn contact_density(&self) -> i32 {
        self.contact_density
    }

    pub fn set_contact_density(&mut self, contact_density: i32) {
        self.contact_density = contact_density;
    }

    pub fn contact_function_calls(&self) -> bool {
        self.contact_function_calls
    }

    pub fn set_contact_function_calls(&mut self, contact_function_calls: bool) {
        self.contact_function_calls = contact_function_calls;
    }

    pub fn border_bound(&self) -> i32 {
        self.border_bound
    }

    pub fn set_border_bound(&mut self, border_bound: i32) {
        self.border_bound = border_bound;
    }

    pub fn upright_attach(&self) -> i32 {
        self.upright_attach
    }

    pub fn no_stabilize(&self) -> bool {
        self.no_stabilize
    }

    pub fn set_no_stabilize(&mut self, no_stabilize: bool) {
        self.no_stabilize = no_stabilize;
    }

    pub fn timer(&self) -> i32 {
        self.timer
    }

    pub fn set_timer(&mut self, timer: i32) {
        self.timer = timer;
    }

    pub fn timer_call(&self) -> Option<&str> {
        self.timer_call.as_deref()
    }

    pub(crate) fn timer_callback(&self) -> Option<ScriptCallbackTarget> {
        self.timer_call_link.target(self.timer_call.as_deref())
    }

    pub(crate) fn control_transfer_callback(&self) -> Option<ScriptCallbackTarget> {
        self.control_transfer_link.target(Some("ControlTransfer"))
    }

    pub fn set_timer_call(&mut self, timer_call: Option<String>) {
        self.timer_call = timer_call;
        self.mark_callbacks_unlinked();
    }

    pub fn set_upright_attach(&mut self, upright_attach: i32) {
        self.upright_attach = upright_attach;
    }

    /// RotatedSolidmasks (C4Def.cpp:414): rotation does not disable the
    /// solid mask (C4Object.cpp:5655).
    pub fn rotated_solid_masks(&self) -> bool {
        self.rotated_solid_masks
    }

    pub fn auto_context_menu(&self) -> bool {
        self.auto_context_menu
    }

    pub fn no_component_mass(&self) -> bool {
        self.no_component_mass
    }

    pub fn set_no_component_mass(&mut self, no_component_mass: bool) {
        self.no_component_mass = no_component_mass;
    }

    pub fn set_rotated_solid_masks(&mut self, rotated_solid_masks: bool) {
        self.rotated_solid_masks = rotated_solid_masks;
    }

    pub fn set_auto_context_menu(&mut self, auto_context_menu: bool) {
        self.auto_context_menu = auto_context_menu;
    }

    pub fn collection_rect(&self) -> Option<DefinitionRect> {
        self.collection_rect
    }

    pub fn fire_top(&self) -> i32 {
        self.fire_top
    }

    pub fn set_fire_top(&mut self, fire_top: i32) {
        self.fire_top = fire_top;
    }

    pub fn lift_top(&self) -> i32 {
        self.lift_top
    }

    pub fn set_lift_top(&mut self, lift_top: i32) {
        self.lift_top = lift_top;
    }

    pub fn set_collection_rect(&mut self, rect: Option<DefinitionRect>) {
        self.def_core_collection_rect = rect;
        self.collection_rect = rect.filter(DefinitionRect::is_positive);
    }

    pub fn collection_limit(&self) -> i32 {
        self.collection_limit
    }

    pub fn contact_incinerate(&self) -> i32 {
        self.contact_incinerate
    }

    pub fn blast_incinerate(&self) -> i32 {
        self.blast_incinerate
    }

    pub fn set_blast_incinerate(&mut self, blast_incinerate: i32) {
        self.blast_incinerate = blast_incinerate;
    }

    pub fn contain_blast(&self) -> i32 {
        self.contain_blast
    }

    pub fn set_contain_blast(&mut self, contain_blast: i32) {
        self.contain_blast = contain_blast;
    }

    pub fn closed_container(&self) -> i32 {
        self.closed_container
    }

    pub fn set_closed_container(&mut self, closed_container: i32) {
        self.closed_container = closed_container;
    }

    pub fn no_horizontal_move(&self) -> i32 {
        self.no_horizontal_move
    }

    pub fn set_no_horizontal_move(&mut self, no_horizontal_move: i32) {
        self.no_horizontal_move = no_horizontal_move;
    }

    pub fn no_burn_decay(&self) -> bool {
        self.no_burn_decay
    }

    pub fn no_burn_damage(&self) -> bool {
        self.no_burn_damage
    }

    pub fn no_breath(&self) -> bool {
        self.no_breath
    }

    pub fn set_no_breath(&mut self, no_breath: bool) {
        self.no_breath = no_breath;
    }

    pub fn set_fire_properties(
        &mut self,
        contact_incinerate: i32,
        no_burn_decay: bool,
        no_burn_damage: bool,
    ) {
        self.contact_incinerate = contact_incinerate;
        self.no_burn_decay = no_burn_decay;
        self.no_burn_damage = no_burn_damage;
    }

    pub fn burn_turn_to(&self) -> Option<&str> {
        self.burn_turn_to.as_deref()
    }

    pub fn build_turn_to(&self) -> Option<&str> {
        self.build_turn_to.as_deref()
    }

    pub fn incomplete_activity(&self) -> bool {
        self.incomplete_activity
    }

    pub fn set_burn_turn_to(&mut self, target: Option<String>) {
        self.burn_turn_to = target;
    }

    pub fn set_build_turn_to(&mut self, target: Option<String>) {
        self.build_turn_to = target;
    }

    pub fn set_incomplete_activity(&mut self, enabled: bool) {
        self.incomplete_activity = enabled;
    }

    pub fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    pub fn set_exclusive(&mut self, exclusive: bool) {
        self.exclusive = exclusive;
    }

    pub fn set_edible(&mut self, edible: bool) {
        self.edible = edible;
    }

    pub fn set_prey(&mut self, prey: bool) {
        self.prey = prey;
    }

    pub fn set_attract_lightning(&mut self, attract_lightning: bool) {
        self.attract_lightning = attract_lightning;
    }

    pub fn entrance_rect(&self) -> Option<DefinitionRect> {
        self.entrance_rect
    }

    pub fn set_entrance_rect(&mut self, entrance_rect: Option<DefinitionRect>) {
        self.entrance_rect = entrance_rect;
    }

    pub fn set_rotated_entrance(&mut self, rotated_entrance: i32) {
        self.rotated_entrance = rotated_entrance;
    }

    pub fn set_no_fight(&mut self, no_fight: bool) {
        self.no_fight = no_fight;
    }

    pub fn is_chopable(&self) -> bool {
        self.chopable
    }

    pub fn set_chopable(&mut self, chopable: bool) {
        self.chopable = chopable;
    }

    pub fn physical(&self) -> &PhysicalInfo {
        &self.physical
    }

    /// Seed for the FnAdjustWalkRotation host seam (C4Script.cpp:5439-5448):
    /// Def->Rotateable, the frame's Action.t_attach, the shape attach record
    /// and Def->Shape.VtxX[Shape.iAttachVtx] (C4Object.cpp:6023).
    pub(crate) fn walk_rotation_seed(&self, state: &ObjectState) -> compat::WalkRotationSeed {
        compat::WalkRotationSeed {
            rotateable: self.rotateable,
            t_attach: state.t_attach,
            attach: state.shape_attach,
            def_attach_vtx_x: usize::try_from(state.shape_attach.vtx)
                .ok()
                .and_then(|vtx| self.shape_vertices.get(vtx))
                .map(|vertex| vertex.x)
                .unwrap_or(0),
        }
    }

    /// The object scope every object-context script call publishes.
    ///
    /// C++ hands each callback the same `C4Object *`, so every entry point
    /// here has to expose the same channels. The setters below assign
    /// independent fields, so the call sites that ordered them differently
    /// were already building an identical context.
    pub(crate) fn host_object_context<'a>(
        &self,
        state: &'a ObjectState,
        object_id: ObjectId,
        world: &HostWorldContext,
    ) -> compat::HostObjectContext<'a> {
        compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.time,
            state.action.data,
            state.action.phase,
            self.shared_action_library(world),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_action_index(state.action.act_map_index)
        .with_shape_vertices(&state.shape_vertices)
        .with_definition_id(self.id.as_str())
        // The scope publishes its whole overlay list, so it must start
        // from the object's real overlays: C4Object::GetGraphicsOverlay
        // splices a single node (src/C4Object.cpp:5962-5977).
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_base_graphics(state.base_graphics.clone())
        .with_alive(state.alive)
        .with_controller(state.controller)
        .with_in_liquid(state.in_liquid)
        .with_own_mass(state.own_mass)
        .with_physicals(
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            *self.physical(),
        )
        .with_walk_rotation(self.walk_rotation_seed(state))
        .with_script_fixed_position(state.script_fixed_position)
        .with_script_fixed_velocity(state.script_fixed_velocity)
        .with_script_rotation_velocity(state.script_rotation_velocity)
        .with_script_fixed_rotation(state.script_fixed_rotation)
        .with_magic_energy(state.magic_energy)
        .with_breath(state.breath)
        .with_need_energy(state.need_energy)
        .with_ocf(state.ocf)
    }

    pub fn set_physical(&mut self, physical: PhysicalInfo) {
        self.physical = physical;
    }

    pub fn set_collection_limit(&mut self, limit: i32) {
        self.collection_limit = limit;
    }

    pub fn fragile(&self) -> bool {
        self.fragile
    }

    pub fn set_fragile(&mut self, fragile: bool) {
        self.fragile = fragile;
    }

    pub fn projectile(&self) -> i32 {
        self.projectile
    }

    pub fn set_projectile(&mut self, projectile: i32) {
        self.projectile = projectile;
    }

    pub fn is_collectible(&self) -> bool {
        self.collectible
    }

    pub fn no_get(&self) -> bool {
        self.no_get
    }

    pub fn set_no_get(&mut self, no_get: bool) {
        self.no_get = no_get;
    }

    pub fn hide_hud_bars(&self) -> i32 {
        self.hide_hud_bars
    }

    pub fn set_hide_hud_bars(&mut self, hide_hud_bars: i32) {
        self.hide_hud_bars = hide_hud_bars;
    }

    pub fn hide_hud_elements(&self) -> i32 {
        self.hide_hud_elements
    }

    pub fn set_hide_hud_elements(&mut self, hide_hud_elements: i32) {
        self.hide_hud_elements = hide_hud_elements;
    }

    pub fn grab_put_get(&self) -> i32 {
        self.grab_put_get
    }

    pub fn vehicle_control(&self) -> i32 {
        self.vehicle_control
    }

    pub fn set_vehicle_control(&mut self, vehicle_control: i32) {
        self.vehicle_control = vehicle_control;
    }

    pub fn set_grab_put_get(&mut self, grab_put_get: i32) {
        self.grab_put_get = grab_put_get;
    }

    pub fn set_collectible(&mut self, collectible: bool) {
        self.collectible = collectible;
    }

    pub fn is_constructable(&self) -> bool {
        self.constructable
    }

    pub fn set_constructable(&mut self, constructable: bool) {
        self.constructable = constructable;
    }

    pub fn construction_offset(&self) -> i32 {
        self.construction_offset
    }

    pub fn set_construction_offset(&mut self, offset: i32) {
        self.construction_offset = offset;
    }

    pub fn stretch_growth(&self) -> bool {
        self.stretch_growth
    }

    pub fn set_stretch_growth(&mut self, stretch_growth: bool) {
        self.stretch_growth = stretch_growth;
    }

    pub fn oversize(&self) -> bool {
        self.oversize
    }

    pub fn set_oversize(&mut self, oversize: bool) {
        self.oversize = oversize;
    }

    pub fn placement(&self) -> i32 {
        self.placement
    }

    pub fn set_placement(&mut self, placement: i32) {
        self.placement = placement;
    }

    pub fn growth(&self) -> i32 {
        self.growth
    }

    pub fn set_growth(&mut self, growth: i32) {
        self.growth = growth;
    }

    pub fn basement(&self) -> i32 {
        self.basement
    }

    pub fn set_basement(&mut self, basement: i32) {
        self.basement = basement;
    }

    pub fn components(&self) -> &[DefinitionComponent] {
        &self.components
    }

    pub fn set_components(&mut self, components: Vec<DefinitionComponent>) {
        self.components = components;
    }

    pub fn line_connect(&self) -> u32 {
        self.line_connect
    }

    pub fn set_line_connect(&mut self, line_connect: u32) {
        self.line_connect = line_connect;
    }

    pub(crate) fn call_initialize(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        random: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            CommandBatch,
            AudioRegistry,
            LcgRng,
            u64,
            Option<EngineError>,
        ),
        EngineError,
    > {
        if !self.has_initialize {
            return Ok((
                CommandBatch::default(),
                audio,
                rng,
                world.next_object_id(),
                None,
            ));
        }
        // C++ Initialize runs with no parameters (PSF_Initialize broadcast);
        // (state, random) is the synthetic command-DSL convention.
        let args = if self.c4_callback_args {
            Vec::new()
        } else {
            vec![
                build_state_value(&self.id, object_id, state, &self.action_library),
                Value::Int(random),
            ]
        };
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        // Cells live OUTSIDE the session so a mid-call error keeps the
        // pre-error local writes (C4AulExec aborts the call but rolls
        // nothing back, C4AulExec.cpp:1318-1342).
        let cells = clonk_script::LocalCells::from_local_vars(&state.local_vars);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(self.host_object_context(state, object_id, &world)),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                compat::register_session_local_cells(object_id, cells.clone());
                self.script.call_with_cells_and_this(
                    "Initialize",
                    &args,
                    &cells,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        // A script error aborts the call but the partial outcome below
        // still folds — C++'s CreateObject/SetTransferZone/Random side
        // effects all landed live before the unwind and NewObj's
        // `Number = ++ObjectEnumerationIndex` (C4Game.cpp:1119) stays
        // advanced; the error surfaces to the caller as a value.
        let (returned, script_error) = match result {
            Ok(value) => (Some(value), None),
            Err(source) => (
                None,
                Some(script_execution_error(
                    self.id.clone(),
                    "Initialize".to_string(),
                    source,
                    None,
                )),
            ),
        };
        let mut batch = match returned {
            Some(value) => parse_command(&self.id, "Initialize", value)?,
            None => CommandBatch::default(),
        };
        // Store updated local variables in the delta so they persist
        batch.delta.local_vars = Some(cells.snapshot());
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;
        batch.other_objects.extend(other_objects);
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);

        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if let Some(update) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &update);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        if !object_commands.is_empty() {
            batch.commands.extend(object_commands);
        }
        if !command_operations.is_empty() {
            batch.command_ops.extend(command_operations);
        }
        if destroy_object {
            batch.destroy = true;
        }
        if !host_object_effects.is_empty() {
            batch.effects.extend(host_object_effects);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
        }
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview = host_raster_preview;
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id, script_error))
    }

    pub(crate) fn call_construction(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            CommandBatch,
            AudioRegistry,
            LcgRng,
            u64,
            Option<EngineError>,
        ),
        EngineError,
    > {
        if !self.has_construction {
            return Ok((
                CommandBatch::default(),
                audio,
                rng,
                world.next_object_id(),
                None,
            ));
        }
        // Construction() takes no arguments
        let args: [Value; 0] = [];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        // Cells live OUTSIDE the session so a mid-call error keeps the
        // pre-error local writes (C4AulExec.cpp:1318-1342, no rollback).
        let cells = clonk_script::LocalCells::from_local_vars(&state.local_vars);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(self.host_object_context(state, object_id, &world)),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                compat::register_session_local_cells(object_id, cells.clone());
                self.script.call_with_cells_and_this(
                    "Construction",
                    &args,
                    &cells,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        // A script error aborts the call but the partial outcome below
        // still folds (C4AulExec.cpp:1318-1342, no rollback) — the error
        // surfaces to the caller as a value.
        let script_error = result.err().map(|source| {
            script_execution_error(self.id.clone(), "Construction".to_string(), source, None)
        });
        // Construction() return value is not used (it just returns 0 or nil)
        // We only care about side effects (initializing local variables, etc.)
        // But we DO need to capture updated local variable values
        let mut batch = CommandBatch::default();
        // Store updated local variables in the delta so they persist
        batch.delta.local_vars = Some(cells.snapshot());
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;
        batch.other_objects.extend(other_objects);
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);

        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if let Some(update) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &update);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        batch.spawns.extend(host_spawns);
        batch.landscape_ops.extend(host_landscape_ops);
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview = host_raster_preview;
        batch.particles.extend(host_particles);
        batch.transfer_zones.extend(host_transfer_zones);
        batch.messages.extend(host_messages);
        batch.commands.extend(object_commands);
        batch.command_ops.extend(command_operations);
        batch.effects.extend(host_object_effects);
        batch.global_effects.extend(host_global_effects);
        batch.destroy = destroy_object;
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id, script_error))
    }

    pub(crate) fn call_step(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        frame: u64,
        random: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, LcgRng, u64), EngineError> {
        if !self.has_step {
            return Ok((CommandBatch::default(), audio, rng, world.next_object_id()));
        }
        let frame_value = if frame > i32::MAX as u64 {
            i32::MAX
        } else {
            frame as i32
        };
        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::Int(frame_value),
            Value::Int(random),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(self.host_object_context(state, object_id, &world)),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_session("Step", &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let (result, updated_local_vars) = result.map_err(|source| {
            script_execution_error(self.id.clone(), "Step".to_string(), source, None)
        })?;
        let mut batch = parse_command(&self.id, "Step", result)?;
        batch.delta.local_vars = Some(updated_local_vars);
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;
        batch.other_objects.extend(other_objects);
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);

        if let Some(delta) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &delta);
        }
        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        if !object_commands.is_empty() {
            batch.commands.extend(object_commands);
        }
        if !command_operations.is_empty() {
            batch.command_ops.extend(command_operations);
        }
        if destroy_object {
            batch.destroy = true;
        }
        if !host_object_effects.is_empty() {
            batch.effects.extend(host_object_effects);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
        }
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview = host_raster_preview;
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id))
    }

    pub(crate) fn call_action_callback(
        &self,
        object_definition: &Definition,
        callback: &ScriptCallbackTarget,
        kind: ActionCallbackKind,
        state: &ObjectState,
        object_id: ObjectId,
        action_name: &str,
        abort_phase: Option<i32>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        // Linked definitions arrive with an exact retained function body.
        // Unlinked synthetic fixtures keep their historical name-based path.

        // C++ ActMap callbacks pass no parameters, except AbortCall which
        // gets the last phase (C4Object.cpp:4154,4168,4182). The (state,
        // action) pair is the synthetic command-DSL convention.
        let args = if self.c4_callback_args {
            match kind {
                ActionCallbackKind::Abort => {
                    vec![Value::Int(abort_phase.unwrap_or(state.action.phase))]
                }
                _ => Vec::new(),
            }
        } else {
            vec![
                build_state_value(
                    &object_definition.id,
                    object_id,
                    state,
                    &object_definition.action_library,
                ),
                Value::String(action_name.to_string().into()),
            ]
        };
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(object_definition.host_object_context(state, object_id, &world)),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_callback_session(callback, &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let mut host_effects = host_effects;
        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }
        let audio_state = audio_guard.finish();
        let (value, updated_local_vars) = match result {
            Ok(value) => value,
            Err(source) => {
                return Err(script_execution_error(
                    self.id.clone(),
                    kind.context().to_string(),
                    source,
                    Some(Box::new(ScriptCallRecovery {
                        outcome: host_effects,
                        audio: audio_state,
                        rng,
                    })),
                ));
            }
        };

        // Action callbacks can return any value in C4Script.
        // The return value is typically used to indicate success/failure (e.g., return 1).
        // Unlike some other callback types, we don't validate or use the return value here.
        // This matches the C++ engine behavior where callbacks like Scaling() return int.
        drop(value);

        // Store updated local variables so they persist
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        Ok((host_effects, audio_state, rng))
    }

    pub(crate) fn call_menu_entries(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Vec<ContextMenuEntry>, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function("MenuEntries") {
            return Ok((Vec::new(), audio, rng));
        }

        let args = [build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        )];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = self.host_object_context(state, object_id, &world);
        let (result, outcome) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_session("MenuEntries", &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, _updated_local_vars) = result.map_err(|source| {
            script_execution_error(self.id.clone(), "MenuEntries".to_string(), source, None)
        })?;
        // MenuEntries shouldn't modify local vars, so we discard them
        let entries = self.parse_context_menu_entries(value)?;

        if !outcome.object.is_empty()
            || !outcome.global.is_empty()
            || outcome.object_update.is_some()
            || !outcome.object_commands.is_empty()
            || outcome.destroy_object
            || outcome.environment.is_some()
            || outcome.physics.is_some()
            || !outcome.spawns.is_empty()
            || !outcome.particles.is_empty()
            || !outcome.transfer_zones.is_empty()
            || !outcome.messages.is_empty()
            || !outcome.audio.events.is_empty()
            || outcome.trigger_game_over
        {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries".to_string(),
                detail: "callback must not modify game state".to_string(),
            });
        }
        if !environment_delta.is_empty() || !physics_delta.is_empty() {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries".to_string(),
                detail: "callback must not modify global state".to_string(),
            });
        }

        let audio_state = audio_guard.finish();
        Ok((entries, audio_state, rng))
    }

    pub(crate) fn call_menu_command(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        kind: MenuCommandKind,
        selection: &MenuCommandSelection,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function("MenuCommand") {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::String(kind.as_str().to_string().into()),
            build_menu_selection_value(selection),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = self.host_object_context(state, object_id, &world);
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_session("MenuCommand", &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| {
            script_execution_error(self.id.clone(), "MenuCommand".to_string(), source, None)
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        // Same bool cast as C4Object::MenuCommand (C4Object.cpp:3736):
        // script results coerce by raw truthiness, never a type error.
        let handled = compat::value_raw_truthy(&value);

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((handled, host_effects, audio_state, rng))
    }

    pub(crate) fn call_control(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args: [Value; 0] = [];
        let (value, host_effects, audio_state, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Control",
            |script, cells, this| script.call_with_cells_and_this(function, &args, cells, this),
        )?;
        // C4Object::CallControl (C4Object.cpp:3300) evaluates the script
        // result with `static_cast<bool>(...)` — C4Value raw-data truthiness
        // (C4Value.h:76,183-185): only nil, 0 and false are unhandled. Real
        // control scripts return ints (`return(1)`), never bools.
        Ok((
            compat::value_raw_truthy(&value),
            host_effects,
            audio_state,
            rng,
        ))
    }

    /// Run one OUTER object VM call on LIVE local cells registered as the
    /// object's session with the host context: nested calls the host
    /// routes back onto this object (and cross-object Local/LocalN
    /// references) share the storage, so mid-call local writes are visible
    /// in both directions — C++ mutates the ONE live C4Object. Must run
    /// inside `with_effect_context_with_state`; the caller folds the final
    /// locals via the returned snapshot.
    fn run_live_object_session(
        &self,
        function: &str,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
        object_id: ObjectId,
    ) -> Result<(Value, HashMap<String, Value>), clonk_script::ScriptError> {
        self.run_live_object_callback_session(
            &ScriptCallbackTarget::unlinked(function),
            args,
            local_vars,
            object_id,
        )
    }

    fn run_live_object_callback_session(
        &self,
        callback: &ScriptCallbackTarget,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
        object_id: ObjectId,
    ) -> Result<(Value, HashMap<String, Value>), clonk_script::ScriptError> {
        let cells = clonk_script::LocalCells::from_local_vars(local_vars);
        compat::register_session_local_cells(object_id, cells.clone());
        let result = match callback.resolution() {
            Some(resolution) => self.script.call_pinned_with_cells_and_this(
                resolution.function.as_ref(),
                resolution.scope == clonk_script::ScriptFunctionScope::Global,
                args,
                &cells,
                compat::object_reference_value(object_id),
            ),
            None => self.script.call_with_cells_and_this(
                callback.function_name(),
                args,
                &cells,
                compat::object_reference_value(object_id),
            ),
        };
        result.map(|value| (value, cells.snapshot()))
    }

    #[doc(hidden)]
    pub fn call_object_function(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        args: &[Value],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        self.call_object_callback(
            self,
            state,
            object_id,
            &ScriptCallbackTarget::unlinked(function),
            args,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_object_callback(
        &self,
        object_definition: &Definition,
        state: &ObjectState,
        object_id: ObjectId,
        callback: &ScriptCallbackTarget,
        args: &[Value],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if callback.resolution().is_none() && !self.script.has_function(callback.function_name()) {
            let next_object_id = world.next_object_id();
            return Ok((
                Value::Nil,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let arg_values: Vec<Value> = args.to_vec();
        let function = callback.function_name();
        self.exec_in_object_context_for_definition(
            object_definition,
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            function,
            |script, cells, this| match callback.resolution() {
                Some(resolution) => script.call_pinned_with_cells_and_this(
                    resolution.function.as_ref(),
                    resolution.scope == clonk_script::ScriptFunctionScope::Global,
                    &arg_values,
                    cells,
                    this,
                ),
                None => script.call_with_cells_and_this(function, &arg_values, cells, this),
            },
        )
    }

    /// Run C4Object::Incinerate inside the carrier's live host context so its
    /// C4Effect constructor shares the canonical AddEffect check/start path.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn incinerate_object(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        caused_by: i32,
        blasted: bool,
        incinerating: Option<ObjectId>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Incinerate",
            |_script, _cells, _this| {
                compat::incinerate_target(object_id, caused_by, blasted, incinerating)
                    .map(Value::Bool)
                    .map_err(Into::into)
            },
        )?;
        Ok((matches!(value, Value::Bool(true)), outcome, audio, rng))
    }

    /// Run C4Object::GetValue for C4Player::UpdateValue without exposing the
    /// host helper to script-name shadowing. CalcValue/CalcDefValue and
    /// construction scaling execute in the object's complete live context.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn player_asset_object_value(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        player: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(i32, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let args = [
            compat::object_reference_value(object_id),
            Value::Nil,
            Value::Nil,
            Value::Int(player),
        ];
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "GetValue",
            |_script, _cells, _this| compat::get_value(&args).map_err(Into::into),
        )?;
        Ok((value.as_c4_int().unwrap_or(0), outcome, audio, rng))
    }

    /// Run C4Def::GetValue for C4Command::Buy without exposing the host
    /// helper to script-name shadowing. CalcDefValue and the base's
    /// CalcBuyValue still execute in the actor's complete live host context.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn command_buy_value(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        item_definition: &str,
        base_id: ObjectId,
        buyer: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            Option<i32>,
            compat::EffectContextOutcome,
            AudioRegistry,
            LcgRng,
        ),
        EngineError,
    > {
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "GetValue",
            |_script, _cells, _this| {
                compat::calculated_definition_value(item_definition, Some(base_id), buyer)
                    .map(|price| price.map(Value::Int).unwrap_or(Value::Nil))
                    .map_err(Into::into)
            },
        )?;
        let price = match value {
            Value::Int(value) => Some(value),
            _ => None,
        };
        Ok((price, outcome, audio, rng))
    }

    /// Invoke the native FnBuy/C4Player::Buy path directly so a script
    /// function named Buy cannot shadow the engine operation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn command_buy_item(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        item_definition: &str,
        buyer: i32,
        payer: i32,
        base_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let args = [
            Value::C4Id(item_definition.to_string()),
            Value::Int(buyer),
            Value::Int(payer),
            compat::object_reference_value(base_id),
            Value::Bool(false),
        ];
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Buy",
            |_script, _cells, _this| compat::buy(&args).map_err(Into::into),
        )?;
        Ok((matches!(value, Value::Object(_)), outcome, audio, rng))
    }

    /// Invoke C4Player::Sell2Home directly so a script function named Sell
    /// cannot shadow the command's recursive native transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn command_sell_item(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        sold_object: ObjectId,
        base_owner: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Sell",
            |_script, _cells, _this| {
                compat::sell_object_to_home_live(sold_object, base_owner)
                    .map(Value::Bool)
                    .map_err(Into::into)
            },
        )?;
        Ok((matches!(value, Value::Bool(true)), outcome, audio, rng))
    }

    /// The shared object-context execution seam: installs the physics/
    /// environment/random/audio guards and the HostObjectContext, runs
    /// `invoke` on the definition script against LIVE local cells
    /// registered as the object's session — nested calls the host routes
    /// back onto the same object share the storage, so mid-call local
    /// writes are visible in both directions (C++ mutates the ONE live
    /// C4Object; its object locals are by-reference) — and folds the
    /// outcome. The body every `call_object_function`-style entry shares.
    #[allow(clippy::too_many_arguments)]
    fn exec_in_object_context<F>(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
        label: &str,
        invoke: F,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError>
    where
        F: FnOnce(
            &clonk_script::Engine,
            &clonk_script::LocalCells,
            Value,
        ) -> Result<Value, clonk_script::ScriptError>,
    {
        self.exec_in_object_context_for_definition(
            self,
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            label,
            invoke,
        )
    }

    /// Execute a callback owned by `self` against an object whose live
    /// definition may differ. Native retained function pointers preserve
    /// their original script host while `Exec(pObj, ...)` derives `this`,
    /// object locals and object metadata from the supplied receiver.
    #[allow(clippy::too_many_arguments)]
    fn exec_in_object_context_for_definition<F>(
        &self,
        object_definition: &Definition,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
        label: &str,
        invoke: F,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError>
    where
        F: FnOnce(
            &clonk_script::Engine,
            &clonk_script::LocalCells,
            Value,
        ) -> Result<Value, clonk_script::ScriptError>,
    {
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = object_definition.host_object_context(state, object_id, &world);
        let cells = clonk_script::LocalCells::from_local_vars(&state.local_vars);
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                compat::register_session_local_cells(object_id, cells.clone());
                invoke(
                    &self.script,
                    &cells,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        // The outcome folds for BOTH exits: on a script error the partial
        // outcome — everything mutated before the unwind, including the
        // cells' local writes — travels as the error's recovery payload
        // (C4AulExec aborts the call but rolls nothing back,
        // C4AulExec.cpp:1318-1342).
        // Store updated local variables — the final cell snapshot carries
        // nested-call writes too (shared live storage).
        let updated_local_vars = cells.snapshot();
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        let value = match result {
            Ok(value) => value,
            Err(source) => {
                return Err(script_execution_error(
                    self.id.clone(),
                    label.to_string(),
                    source,
                    Some(Box::new(ScriptCallRecovery {
                        outcome: host_effects,
                        audio: audio_state,
                        rng,
                    })),
                ));
            }
        };
        Ok((value, host_effects, audio_state, rng))
    }

    /// `C4Object::GetInfoString`'s effect walk, executed in the target's
    /// live object context so `Fx*Info` callbacks retain their normal side
    /// effects, RNG, audio, local-variable, and nested-object semantics.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_object_effect_info(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            Vec<String>,
            compat::EffectContextOutcome,
            AudioRegistry,
            LcgRng,
        ),
        EngineError,
    > {
        let effects = state.effects.clone();
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "GetInfoString",
            |_script, _cells, _this| {
                Ok(Value::Array(
                    compat::object_effect_info_lines(object_id, &effects)
                        .into_iter()
                        .map(Value::from)
                        .collect(),
                ))
            },
        )?;
        let lines = match value {
            Value::Array(values) => values
                .into_iter()
                .filter_map(|value| match value {
                    Value::String(line) => Some(line.into_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok((lines, outcome, audio, rng))
    }

    /// C4AulScript::DirectExec in this definition's object context — the
    /// C4Object::MenuCommand seam (C4Object.cpp:3756-3760): `source` runs
    /// as ONE expression with the object's locals and `this`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn direct_exec_object_expression(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        source: &str,
        label: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            label,
            |script, cells, this| {
                script.direct_exec_with_cells_and_this_in_context_diagnostics(
                    source,
                    cells,
                    this,
                    label,
                    is_cpp_direct_exec_context(label),
                )
            },
        )
    }

    /// Synchronized-control DirectExec variant. Unlike object-menu commands,
    /// the temporary expression script carries strictness on the packet.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn direct_exec_object_expression_at_strict(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        source: &str,
        label: &str,
        strict_level: Option<u8>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            label,
            |script, cells, this| {
                script.direct_exec_with_cells_and_this_at_strict_in_context_diagnostics(
                    source,
                    cells,
                    this,
                    strict_level,
                    label,
                    is_cpp_direct_exec_context(label),
                )
            },
        )
    }

    pub(crate) fn call_menu_callback(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args = [build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        )];
        let args_call = args.clone();
        let function_name = function.to_string();
        let function_call = function_name.clone();
        let local_vars_call = state.local_vars.clone();
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = self.host_object_context(state, object_id, &world);
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            move || {
                self.run_live_object_session(
                    &function_call,
                    &args_call,
                    &local_vars_call,
                    object_id,
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| {
            script_execution_error(
                format!("{}::{}", self.id, function),
                "MenuCallback".to_string(),
                source,
                None,
            )
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        // C4Object::MenuCommand (C4Object.cpp:3732-3736): the executed menu
        // function's result is `static_cast<bool>(DirectExec(...))` — raw
        // C4Value truthiness; real context functions return ints.
        let handled = compat::value_raw_truthy(&value);

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((handled, host_effects, audio_state, rng))
    }

    fn parse_context_menu_entries(
        &self,
        value: Value,
    ) -> Result<Vec<ContextMenuEntry>, EngineError> {
        let Value::Array(entries) = value else {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries".to_string(),
                detail: format!("expected array (got {})", value.type_name()),
            });
        };

        let mut result = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            let Value::Proplist(props) = entry else {
                return Err(EngineError::InvalidScriptOutput {
                    definition: self.id.clone(),
                    function: "MenuEntries".to_string(),
                    detail: format!(
                        "entry {index} must be a proplist (got {})",
                        entry.type_name()
                    ),
                });
            };

            let label = match props.get("label") {
                Some(Value::String(text)) if !text.is_empty() => text.to_string(),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!(
                            "entry {index} field `label` must be non-empty string (got {})",
                            other.type_name()
                        ),
                    });
                }
                None => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!("entry {index} missing required field `label`"),
                    });
                }
            };

            let function = match props.get("callback") {
                Some(Value::String(name)) if !name.is_empty() => name.to_string(),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!(
                            "entry {index} field `callback` must be non-empty string (got {})",
                            other.type_name()
                        ),
                    });
                }
                None => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!("entry {index} missing required field `callback`"),
                    });
                }
            };

            let description = match props.get("description") {
                Some(Value::String(text)) if text.is_empty() => None,
                Some(Value::String(text)) => Some(text.to_string()),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!(
                            "entry {index} field `description` must be string (got {})",
                            other.type_name()
                        ),
                    });
                }
                None => None,
            };

            result.push(ContextMenuEntry {
                function,
                label,
                description,
            });
        }

        Ok(result)
    }

    pub(crate) fn call_effect_start(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        constructor_values: &[Value; 4],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        // C4Effect's constructor calls Fx*Start with iTemp=0 followed by
        // the four rVal arguments supplied to AddEffect
        // (C4Effect.cpp:118-129). Deferred object starts must retain the
        // same argument list as the synchronous priority-one/global path.
        let mut extras = vec![Value::Int(0)];
        extras.extend(constructor_values.iter().cloned());
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Start",
            "FxStart",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    pub(crate) fn call_effect_timer(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        frame: u64,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Timer",
            "FxTimer",
            vec![Value::Int(effect.timer)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// `Fx<Name>Damage` (C4Effect.cpp:312-322): the effect is asked to
    /// modify a damage/energy change before it applies.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_effect_damage(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        change: i32,
        cause: i32,
        caused_by: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Damage",
            "FxDamage",
            vec![Value::Int(change), Value::Int(cause), Value::Int(caused_by)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// `Fx<Name>Effect` check call (C4Effect.cpp:280-282): the checker
    /// effect is asked about a pending new effect — the callback receives
    /// the new name plus the AddEffect rVal1-4 (C++ passes them at
    /// positions 5-8; the deferred convention appends them after the name).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_effect_effect(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        checker: &EffectState,
        pending: &EffectState,
        constructor_values: &[Value; 4],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let mut extras = vec![Value::String(pending.name.clone().into())];
        // rVal1-4 (C4Effect.cpp:282): always four slots, missing = nil.
        extras.extend(constructor_values.iter().cloned());
        self.dispatch_effect_callback(
            carrier,
            checker,
            "Effect",
            "FxEffect",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// `Fx<Name>Add` merge seam (DoCall PSFS_FxAdd, C4Effect.cpp:300-301):
    /// the ACCEPTOR effect receives the annulled new effect's name, timer
    /// interval and the first four AddEffect parameters.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_effect_add(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        acceptor: &EffectState,
        pending: &EffectState,
        constructor_values: &[Value; 4],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let mut extras = vec![
            Value::String(pending.name.clone().into()),
            Value::Int(pending.interval),
        ];
        // rVal1-4 (C4Effect.cpp:301): always four slots, missing = nil.
        extras.extend(constructor_values.iter().cloned());
        self.dispatch_effect_callback(
            carrier,
            acceptor,
            "Add",
            "FxAdd",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    pub(crate) fn call_effect_stop(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        reason: EffectStopReason,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let mut extras = vec![effect_stop_reason_value(reason)];
        if matches!(reason, EffectStopReason::Temp) {
            // fTemp = true (TempRemoveUpperEffects, C4Effect.cpp:489).
            extras.push(Value::Bool(true));
        }
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Stop",
            "FxStop",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// Reactivate a temp-removed effect (TempReaddUpperEffects,
    /// C4Effect.cpp:505): Fx*Start with iTemp = C4FxCall_Temp
    /// (C4Effects.h:47); the result is ignored like C++.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_effect_temp_readd(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Start",
            "FxStart",
            vec![Value::Int(1)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    fn dispatch_effect_callback(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        event: &'static str,
        function_label: &'static str,
        mut extras: Vec<Value>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let next_object_id = world.next_object_id();
        let callback_name = format!("Fx{}{}", effect.name, event);
        let callback = resolve_effect_script_callback(effect, &callback_name, &world);
        // AddFunc registers the engine's FxFireStart below script functions.
        // C4Effect therefore calls the native body only when script lookup did
        // not find an override; inherited() from an override already reaches
        // the registered host through the ordinary VM path.
        let native_fire_start = callback.is_none() && effect.name == C4FX_FIRE && event == "Start";
        if callback.is_none() && !native_fire_start {
            return Ok((
                EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
                None,
            ));
        }
        // Fx callbacks resolve CODE on the effect command target/id, but
        // pForObj remains the affected object's real C4Object. Its ActMap,
        // physicals, OCF metadata and definition id therefore come from the
        // carrier, never from `self` merely because `self` owns the callback
        // script (C4Effect.cpp:42-57,128-129,342-345).
        let carrier_definition_id = carrier.and_then(|(_, object_id)| {
            world
                .get(object_id)
                .map(|object| object.definition_id().to_string())
        });
        let carrier_metadata = carrier_definition_id
            .as_deref()
            .and_then(|id| world.definition_metadata(id))
            .cloned();

        let mut args = Vec::with_capacity(2 + extras.len());
        // The affected object is the first argument — C++ passes nullptr
        // (nil) for GLOBAL effects (C4Effect::Execute, C4Effect.cpp:345).
        args.push(
            carrier
                .map(|(state, object_id)| {
                    if self.c4_callback_args {
                        compat::object_reference_value(object_id)
                    } else {
                        build_state_value(
                            carrier_definition_id.as_deref().unwrap_or(&self.id),
                            object_id,
                            state,
                            carrier_metadata
                                .as_ref()
                                .map(|metadata| &*metadata.action_library)
                                .unwrap_or(&self.action_library),
                        )
                    }
                })
                .unwrap_or(Value::Nil),
        );
        args.push(build_effect_value(effect));
        args.append(&mut extras);

        // Effect callbacks execute on the effect's command target
        // (pFn->Exec(pCommandTarget, ...), C4Effect.cpp:129,345,392,456):
        // `this()` is the command target and its object locals are live.
        // The affected carrier and command target may be different objects,
        // so copy locals from the command target's world snapshot unless it
        // is the carrier whose threaded event snapshot is newer.
        let context_object = callback
            .as_ref()
            .and_then(|callback| callback.command_object)
            .or_else(|| {
                native_fire_start
                    .then(|| {
                        effect
                            .command_target
                            .map(|target| ObjectId::new(target as u64))
                    })
                    .flatten()
                    .filter(|target| world.get(*target).is_some())
            });
        let context_is_self =
            carrier.is_some_and(|(_, object_id)| context_object == Some(object_id));
        let context_this = context_object
            .map(compat::object_reference_value)
            .unwrap_or(Value::Nil);
        let context_locals = if context_is_self {
            carrier
                .map(|(state, _)| state.local_vars.clone())
                .unwrap_or_default()
        } else {
            context_object
                .and_then(|object_id| world.get(object_id))
                .and_then(|object| object.full_state().map(|state| state.local_vars.clone()))
                .unwrap_or_default()
        };
        // The callback's LIVE local cells: registered as the object's
        // session so nested calls / cross-object references back onto it
        // share the storage (C++ mutates the one live C4Object).
        let context_cells = clonk_script::LocalCells::from_local_vars(&context_locals);

        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let audio_guard = enter_audio_context(audio);
        let callback_definition_context = callback
            .as_ref()
            .and_then(|callback| callback.definition_context.clone())
            .or_else(|| {
                native_fire_start
                    .then(|| {
                        context_object.and_then(|object| {
                            world
                                .get(object)
                                .map(|object| DefinitionId::from(object.definition_id()))
                        })
                    })
                    .flatten()
            });
        let (result, mut commands) = compat::with_effect_context_with_state_and_definition(
            carrier.map(|(state, object_id)| {
                let carrier_walk_rotation = carrier_metadata
                    .as_ref()
                    .map(|metadata| compat::WalkRotationSeed {
                        rotateable: metadata.rotateable,
                        t_attach: state.t_attach,
                        attach: state.shape_attach,
                        def_attach_vtx_x: usize::try_from(state.shape_attach.vtx)
                            .ok()
                            .and_then(|vtx| metadata.vertices.get(vtx))
                            .map(|vertex| vertex.x)
                            .unwrap_or(0),
                    })
                    .unwrap_or_else(|| self.walk_rotation_seed(state));
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.time,
                    state.action.data,
                    state.action.phase,
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.action_library.clone())
                        .unwrap_or_else(|| self.shared_action_library(&world)),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.ocf_base)
                        .unwrap_or(self.ocf_base),
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.crew_member)
                        .unwrap_or(self.crew_member),
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_action_index(state.action.act_map_index)
                .with_shape_vertices(&state.shape_vertices)
                .with_definition_id(carrier_definition_id.as_deref().unwrap_or(self.id.as_str()))
                .with_alive(state.alive)
                .with_controller(state.controller)
                .with_in_liquid(state.in_liquid)
                .with_own_mass(state.own_mass)
                .with_physicals(
                    state.info_physical,
                    state.temporary_physical,
                    state.physical_changes.clone(),
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.physical)
                        .unwrap_or(*self.physical()),
                )
                .with_base_graphics(state.base_graphics.clone())
                // C4Object::GetGraphicsOverlay splices a single node into the
                // live pGfxOverlay list (src/C4Object.cpp:5962-5977), so an
                // effect callback that writes one overlay leaves the object's
                // other overlays alone. The scope publishes its WHOLE overlay
                // list (compat/contexts.rs:8892), so it must start from the
                // carrier's real overlays or the write deletes the rest.
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_walk_rotation(carrier_walk_rotation)
                .with_script_fixed_position(state.script_fixed_position)
                .with_script_fixed_velocity(state.script_fixed_velocity)
                .with_script_rotation_velocity(state.script_rotation_velocity)
                .with_script_fixed_rotation(state.script_fixed_rotation)
                .with_magic_energy(state.magic_energy)
                .with_breath(state.breath)
                .with_need_energy(state.need_energy)
                .with_ocf(state.ocf)
            }),
            callback_definition_context,
            context_object,
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                if let Some(session_id) = context_object {
                    compat::register_session_local_cells(session_id, context_cells.clone());
                }
                if native_fire_start {
                    return compat::fx_fire_start(&args)
                        .map(|value| Some((value, context_cells.snapshot())))
                        .map_err(ScriptError::from);
                }
                let callback = callback
                    .as_ref()
                    .expect("script callback exists when native fallback is inactive");
                if callback.engine_global_entry {
                    if context_object.is_some() {
                        return callback
                            .script
                            .call_pinned_with_cells_and_this(
                                &callback.resolution.function,
                                true,
                                &args,
                                &context_cells,
                                context_this,
                            )
                            .map(|value| Some((value, context_cells.snapshot())));
                    }
                    return callback
                        .script
                        .call_pinned_with_ref_args(&callback.resolution.function, true, &args)
                        .map(|(value, _)| Some((value, HashMap::new())));
                }
                if context_object.is_some() {
                    return callback.script.call_effect_callback_in_context_with_cells(
                        &effect.name,
                        event,
                        &args,
                        &context_cells,
                        context_this,
                    );
                }
                callback.script.call_effect_callback_in_context(
                    &effect.name,
                    event,
                    &args,
                    &context_locals,
                    context_this,
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let audio_state = audio_guard.finish();

        let callback_result = recover_effect_callback_error(
            result,
            &context_cells,
            format!("{}::{}::{}", self.id, effect.name, function_label),
        )?;
        if !environment_delta.is_empty() {
            commands.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            commands.physics = Some(physics_delta);
        }
        let callback_result = callback_result.map(|(value, updated_locals)| {
            if context_is_self {
                commands.context_locals = Some(updated_locals);
            } else if let Some(context_object) = context_object {
                append_effect_command_target_locals(&mut commands, context_object, updated_locals);
            }
            value
        });
        Ok((commands, audio_state, rng, callback_result))
    }
}
