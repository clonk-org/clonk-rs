//! `lib` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocalVariables(Arc<HashMap<String, Value>>);

impl LocalVariables {
    pub fn snapshot(&self) -> HashMap<String, Value> {
        self.0.as_ref().clone()
    }
}

impl std::ops::Deref for LocalVariables {
    type Target = HashMap<String, Value>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl std::ops::DerefMut for LocalVariables {
    fn deref_mut(&mut self) -> &mut Self::Target {
        #[cfg(test)]
        if Arc::strong_count(&self.0) > 1 {
            SCRIPT_STATE_LOCAL_VAR_DEEP_CLONES.with(|count| count.set(count.get() + 1));
        }
        Arc::make_mut(&mut self.0)
    }
}

impl From<HashMap<String, Value>> for LocalVariables {
    fn from(values: HashMap<String, Value>) -> Self {
        Self(Arc::new(values))
    }
}

impl Serialize for LocalVariables {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LocalVariables {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        HashMap::<String, Value>::deserialize(deserializer).map(Into::into)
    }
}

/// `C4ViewDelay` (C4Constants.h:35): how many object ticks the cursor's
/// energy/magic/breath bars stay visible after a relevant change.
pub const C4_VIEW_DELAY: i32 = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectState {
    /// C4Object::CustomName: Some overrides the crew-info/definition name;
    /// None uses the fallback chain (C4Object.cpp:2103-2116).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub custom_name: Option<String>,
    pub position: Vector2,
    pub velocity: Vector2,
    /// Transient script-call mirror of fix_x/fix_y. Like
    /// `script_fixed_velocity`, this is never persisted.
    #[serde(skip)]
    pub script_fixed_position: Option<FixedVec2>,
    /// Transient script-call mirror of the object's TRUE sub-pixel dirs:
    /// C++ Fn(Get|Set)XDir/YDir read/write the LIVE C4Fixed xdir/ydir
    /// (C4Script.cpp:697-732, :1160-1180) — an INT-seeded scope reads a
    /// 0.4 px/f drift as 0 and mis-takes script guards (the GoldRush
    /// fish's `if (GetXDir() > 0) SetXDir(0)`, f100 wall). Set only on
    /// script-call snapshots (`Object::script_state_snapshot`); never
    /// persisted.
    #[serde(skip)]
    pub script_fixed_velocity: Option<FixedVec2>,
    /// Transient script-call mirror of the object's TRUE angular velocity.
    /// Like the fixed position/velocity mirrors, this is never persisted.
    #[serde(skip)]
    pub script_rotation_velocity: Option<C4Fixed>,
    /// Transient script-call mirror of the raw 16.16 rotation accumulator
    /// (`C4Object::fix_r`). GetObjectVal reflects this independently of the
    /// whole-degree `Rotation` field.
    #[serde(skip)]
    pub script_fixed_rotation: Option<C4Fixed>,
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    /// `C4Object::ViewEnergy`, the `// NoSave //` presentation timer that
    /// makes the cursor's energy/magic/breath bars visible for `C4ViewDelay`
    /// object ticks after a relevant change (C4Object.h:145;
    /// C4Constants.h:35). It is neither saved nor synchronized.
    #[serde(skip)]
    pub view_energy: i32,
    /// C4Object::NeedEnergy: the persistent insufficient-power marker set
    /// by EnergyCheck and energy-consuming actions (C4Object.cpp:118,
    /// 1478, 4739-4752; persisted at :2805).
    #[serde(default)]
    pub need_energy: bool,
    #[serde(default)]
    pub damage: i32,
    #[serde(default)]
    pub magic_energy: i32,
    #[serde(default)]
    pub magic_capacity: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
    pub action: ActionState,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    /// Complete C4Shape slot storage. Public snapshots expose only the active
    /// `vertices` prefix; engine persistence carries this separately.
    #[serde(skip)]
    pub(crate) shape_vertices: ShapeVertexBuffer,
    /// Live C4Shape::ContactDensity. SetContactDensity mutates this
    /// per-object value and C4Shape::CompileFunc persists it.
    #[serde(
        default = "default_contact_density",
        skip_serializing_if = "is_default_contact_density"
    )]
    pub contact_density: i32,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub layer: Option<ObjectId>,
    /// C4Object::Visibility (`Visibility=` in Objects.txt). Zero is
    /// `VIS_All`; nonzero values are the VIS_* bitmask consumed by
    /// C4Object::IsVisible (C4Object.cpp:5600-5629).
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub visibility: i32,
    /// C4Object::BlitMode, distinct from SetGraphics base/overlay modes.
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub blit_mode: u32,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    /// Runtime identity of this object's current link in its container's
    /// `C4ObjectList`. Every successful Enter allocates a fresh link, even
    /// when an Exit+Enter callback restores the same final container/slot.
    #[serde(skip)]
    pub(crate) contents_link_generation: u64,
    #[serde(default)]
    pub components: ComponentList,
    /// C4Object::Component is a C4IDList: indexed access follows insertion
    /// order independently of the count map, and zero-count entries remain
    /// present (C4IDList.cpp:38-45,85-103).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_order: Vec<DefinitionId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    /// C4Object::Controller (C4Object.h:127): the player whose action
    /// chain caused this object — kill/cause tracing. NO_OWNER before
    /// Init (C4Object.cpp:86) and as the Objects.txt compile default
    /// (C4Object.cpp:2739); Init seeds it from the explicit controller or
    /// the owner (C4Object.cpp:162).
    #[serde(default = "default_owner")]
    pub controller: i32,
    #[serde(default = "default_category")]
    pub category: i32,
    #[serde(default)]
    pub crew_member: bool,
    /// C4Object::PlrViewRange, persisted by Objects.txt and initialized to
    /// zero. Joining a player crew raises a zero range to the classic 500.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub plr_view_range: i32,
    /// C4Object::Select: the authoritative crew-selection bit persisted as
    /// `Selected` independently of C4Player::Cursor (C4Object.h:153;
    /// C4Object.cpp:2800).
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    /// C4Object::CrewDisabled (FnSetCrewEnabled, C4Script.cpp:4814-4836).
    #[serde(default)]
    pub crew_disabled: bool,
    #[serde(default = "default_alive")]
    pub alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default)]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    /// Per-object storage for script-level local variables
    /// These are initialized to nil in Construction() and persist across all function calls
    #[serde(default)]
    pub local_vars: LocalVariables,
    /// Cached liquid flag (C4Object::InLiquid, C4Object.h:156): loaded from
    /// Objects.txt (default false, C4Object.cpp:2775), updated inside
    /// movement (DoMovement, C4Movement.cpp:443-460), cleared on container
    /// Exit (C4Object.cpp:1528). FnInLiquid reads THIS flag, not the
    /// landscape (C4Script.cpp:1864-1868).
    #[serde(default)]
    pub in_liquid: bool,
    /// C4Object::Mobile (C4Object.h:152): set at Init for nonzero-dir
    /// non-StaticBack spawns (C4Object.cpp:183-185), by SetXDir/SetYDir/
    /// SetRDir and the action procedures, cleared by ExecMovement when all
    /// dirs reach zero (C4Movement.cpp:572) and re-set by the Tick10
    /// gravity-mobilization pulse (:581-586). Only Mobile objects run
    /// DoMovement or idle gravity. Loaded from Objects.txt `Mobile=`
    /// (default false, C4Object.cpp:2772).
    #[serde(default)]
    pub mobile: bool,
    /// Per-object SolidMask rect (C4Object::SolidMask; SetSolidMask,
    /// C4Script.cpp:271-278). None = the definition's mask; a zero-area
    /// rect = mask OFF (opened gates save SolidMask=0,0,0,0,0,0).
    #[serde(default)]
    pub solid_mask_override: Option<DefinitionTargetRect>,
    /// The Def TimerCall counter (C4Object::Timer, C4Object.cpp:1085-1091):
    /// ++ every Execute, fires Def->TimerCall and resets at Def->Timer.
    /// Saved mid-cycle in Objects.txt (default 0, C4Object.cpp:2738).
    #[serde(default)]
    pub timer: i32,
    /// Script-set extra mass (C4Object::OwnMass, C4Object.cpp:94): SetMass
    /// stores iValue - Def->Mass here; Mass = max((Def->Mass + OwnMass) *
    /// Con / FullCon, 1) (UpdateMass, C4Object.cpp:497-500).
    #[serde(default)]
    pub own_mass: i32,
    /// Burning state (C4Object::OnFire, C4Object.h:205). Set by Incinerate
    /// via the fire effect start (C4Effect.cpp:633); drives OCF_OnFire and
    /// the per-frame ExecFire burning.
    #[serde(default)]
    pub on_fire: bool,
    /// Fire animation phase 0..MaxFirePhase (C4Object::FirePhase; initialized
    /// to Random(MaxFirePhase) at fire start, C4Effect.cpp:634 — a synced
    /// draw).
    #[serde(default)]
    pub fire_phase: i32,
    /// Player that caused the fire (the fire effect's CausedBy var, read by
    /// C4Object::GetFireCausePlr for contact-incineration attribution).
    #[serde(default = "default_owner")]
    pub fire_caused_by: i32,
    /// `C4ObjectInfo::Physical` surrogate for crew members until the info
    /// model lands — cloned lazily from the definition physicals on first
    /// training/permanent write. None = read the definition physicals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_physical: Option<PhysicalInfo>,
    /// `PhysicalTemporary`/`TemporaryPhysical` (C4Object.h): the script-set
    /// temporary physicals, taking precedence in GetPhysical
    /// (C4Object.cpp:2118-2134). None = temporary mode off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_physical: Option<PhysicalInfo>,
    /// `C4TempPhysicalInfo::Changes` (C4InfoCore.h:113): PHYS_StackTemporary
    /// registrations as (physical name, previous value), newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_changes: Vec<(String, i32)>,
    /// `C4Object::Breath` on the raw 0..=Physical.Breath scale, filled from
    /// the physicals at birth (C4Object.cpp:193).
    #[serde(default)]
    pub breath: i32,
    /// EntranceStatus flag toggled by SetEntrance (C4Script.cpp:690-695).
    #[serde(default)]
    pub entrance_status: bool,
    /// The object's script menu (C4Object::Menu; FnCreateMenu,
    /// C4Script.cpp:1426-1459). None = no menu. Runtime-only in C++ too
    /// (Objects.txt has no menu section).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<ObjectMenuState>,
    /// Object color from SetColorDw (C4Script.cpp:3661-3668, C4Object Color).
    #[serde(default)]
    pub color: u32,
    /// Object drawing modulation (`C4Object::ColorMod`). Zero disables
    /// modulation; otherwise the raw C4 color is applied at draw time.
    #[serde(default)]
    pub color_modulation: u32,
    /// Per-object inventory/menu picture facet (`C4Object::PictureRect`). A
    /// zero width selects the definition picture, but the raw zero rect still
    /// participates in picture-stack equality (C4Object.cpp:3123-3127,
    /// 6173-6191).
    #[serde(default)]
    pub picture_rect: DefinitionRect,
    /// Per-object shape rectangle from SetShape (C4Script.cpp:5182-5196);
    /// None means the definition shape applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_override: Option<DefinitionRect>,
    /// The CACHED object character flags, like C++ `obj->OCF`: refreshed by
    /// the SetOCF events (spawn Init, updates, container changes, fire) and
    /// once per frame at Execute-start (C4Object.cpp:215,1058; C4Object.h:361)
    /// — readers consume this field, never a fresh compute.
    #[serde(default)]
    pub ocf: u32,
    /// C4Shape attach bookkeeping — see [`ShapeAttachRecord`]. Objects.txt
    /// persists `iAttachX`/`iAttachY`/`iAttachVtx`; `AttachMat` itself is
    /// runtime-only (C4Shape.cpp:511-514).
    #[serde(default, skip_serializing_if = "ShapeAttachRecord::is_unattached")]
    pub shape_attach: ShapeAttachRecord,
    /// `C4Object::Action.t_attach` as latched by ExecAction this frame
    /// (C4Object.cpp:4692 + the per-procedure ORs), mirrored from
    /// `frame_t_attach` for the script host seam (FnAdjustWalkRotation
    /// reads it, C4Script.cpp:5444).
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub t_attach: u32,
    /// `C4Object::NoCollectDelay` (C4Object.h:134): armed to 2 by
    /// ObjectComDrop (C4ObjectCom.cpp:668-669), counted down by DirectCom
    /// (C4Object.cpp:3371-3374) and SetCommand (C4Object.cpp:3939-3940);
    /// while nonzero it vetoes OCF_Collection (SetOCF, C4Object.cpp:598).
    /// Objects.txt "NoCollectDelay" (default 0, C4Object.cpp:2773).
    #[serde(default)]
    pub no_collect_delay: i32,
    /// `C4Object::Base` (C4Object.h:135): the player number this object is
    /// a home base for, NO_OWNER otherwise. Assigned by ExecBase's Tick10
    /// flag check (C4Object.cpp:1000-1018), cleared by its Tick35 lost-flag
    /// arm (:1024-1031); invalid players clear on load (C4Object.cpp:3161).
    #[serde(default = "default_owner")]
    pub base: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ActionChange {
    pub(crate) previous: ActionState,
    requested_name_change: bool,
}

impl ActionChange {
    fn should_record(&self, current: &ActionState) -> bool {
        self.requested_name_change
            || self.previous.name != current.name
            || self.previous.act_map_index != current.act_map_index
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ApplyDeltaOutcome {
    pub(crate) container_change: Option<(Option<ObjectId>, Option<ObjectId>)>,
    pub(crate) action_change: Option<ActionChange>,
    /// A nonzero energy reached 0 while alive — the caller runs
    /// AssignDeath (C4Object::DoEnergy, C4Object.cpp:1363).
    pub(crate) energy_died: bool,
}

/// The state a pending mid-call spawn would have once created — C++
/// CreateObject fully creates objects DURING the call (Game.CreateObject
/// -> NewObject), so nested `obj->Method()` calls on them need a callable
/// scope before the copy-out spawn happens. Mirrors `spawn_single`'s
/// defaults; the authoritative state is still built by the spawn, with
/// the nested outcome folding only the touched fields on top.
pub(crate) fn preview_spawn_state(
    position: Vector2,
    owner: i32,
    controller: i32,
    category: i32,
    construction: i32,
    contact_density: i32,
    vertices: Vec<ObjectVertex>,
) -> ObjectState {
    let shape_vertices = ShapeVertexBuffer::from_active(&vertices);
    ObjectState {
        view_energy: 0,
        custom_name: None,
        script_fixed_position: None,
        script_fixed_velocity: None,
        script_rotation_velocity: None,
        script_fixed_rotation: None,
        position,
        velocity: Vector2::ZERO,
        rotation: 0,
        shape_attach: ShapeAttachRecord::default(),
        t_attach: 0,
        no_collect_delay: 0,
        base: OWNER_NONE,
        energy: 0,
        need_energy: false,
        damage: 0,
        magic_energy: 0,
        magic_capacity: 0,
        construction: construction.clamp(0, FULL_CON),
        action: ActionState::new("Idle"),
        direction: Direction::default(),
        command_direction: CommandDirection::default(),
        effects: Vec::new(),
        vertices,
        shape_vertices,
        contact_density,
        container: None,
        layer: None,
        visibility: 0,
        blit_mode: 0,
        contents: Vec::new(),
        contents_link_generation: 0,
        components: ComponentList::new(),
        component_order: Vec::new(),
        status: ObjectStatus::Normal,
        owner,
        controller,
        category,
        crew_member: false,
        plr_view_range: 0,
        selected: false,
        crew_disabled: false,
        alive: true,
        base_graphics: None,
        graphics_overlays: Vec::new(),
        draw_transform: None,
        local_vars: LocalVariables::default(),
        in_liquid: false,
        mobile: false,
        solid_mask_override: None,
        timer: 0,
        own_mass: 0,
        on_fire: false,
        fire_phase: 0,
        fire_caused_by: OWNER_NONE,
        info_physical: None,
        temporary_physical: None,
        physical_changes: Vec::new(),
        breath: 0,
        entrance_status: false,
        menu: None,
        color: 0,
        color_modulation: 0,
        picture_rect: DefinitionRect::default(),
        shape_override: None,
        ocf: OCF_NORMAL,
    }
}

pub(crate) fn preview_spawn_state_with_components(
    position: Vector2,
    owner: i32,
    controller: i32,
    category: i32,
    construction: i32,
    contact_density: i32,
    vertices: Vec<ObjectVertex>,
    definition_components: &[(DefinitionId, i32)],
) -> ObjectState {
    let mut state = preview_spawn_state(
        position,
        owner,
        controller,
        category,
        construction,
        contact_density,
        vertices,
    );
    state.component_order = definition_components
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    state.components = definition_component_counts(definition_components, construction);
    state
}

impl ObjectState {
    /// `C4ActionDef::FlipDir` of the action this object currently holds.
    pub(crate) fn action_flip_dir(&self, library: &ActionLibrary) -> i32 {
        library.flip_dir_for_entry(&self.action.name, self.action.act_map_index)
    }

    /// `C4Object::UpdateFlipDir` (C4Object.cpp:410-442). C++ keeps the mirror
    /// in `pDrawTransform` itself — `C4Object::Draw` applies no mirror of its
    /// own — so entering the mirrored direction range folds the sign into
    /// mat[0], and leaving it unfolds the sign and deletes a transform that
    /// has become the identity.
    pub(crate) fn update_flip_dir(&mut self, flip_dir: i32) {
        self.draw_transform = DrawTransform::updated_flip_dir(
            self.draw_transform,
            self.direction.to_script_value(),
            flip_dir,
        );
    }

    /// The trailing half of `C4Object::SetDir` (C4Object.cpp:4275-4279): the
    /// direction write, then the FlipDir refresh it triggers. The
    /// `else Action.DrawDir = iDir` branch leaves the transform alone, so an
    /// action without a FlipDir must not unfold a stale mirror.
    pub(crate) fn write_direction(&mut self, direction: Direction, flip_dir: i32) {
        self.direction = direction;
        if flip_dir != 0 {
            self.update_flip_dir(flip_dir);
        }
    }

    pub(crate) fn apply_delta(
        &mut self,
        delta: &ObjectDelta,
        library: &ActionLibrary,
    ) -> ApplyDeltaOutcome {
        let previous_container = self.container;
        let mut container_change = None;
        let mut action_change = None;
        if let Some(position) = delta.position {
            self.position = position;
        }
        if let Some(velocity) = delta.velocity {
            self.velocity = velocity;
        }
        if let Some(in_liquid) = delta.in_liquid {
            self.in_liquid = in_liquid;
        }
        if let Some(rotation) = delta.rotation {
            self.rotation = rotation.rem_euclid(360);
        }
        if let Some(custom_name) = &delta.custom_name {
            self.custom_name = custom_name.clone();
        }
        if let Some(layer) = delta.layer {
            self.layer = layer;
        }
        if let Some(visibility) = delta.visibility {
            self.visibility = visibility;
        }
        if let Some(blit_mode) = delta.blit_mode {
            self.blit_mode = blit_mode;
        }
        if let Some(picture_rect) = delta.picture_rect {
            self.picture_rect = picture_rect;
        }
        if let Some(color) = delta.color {
            self.color = color;
        }
        if let Some(color_modulation) = delta.color_modulation {
            self.color_modulation = color_modulation;
        }
        let mut energy_died = false;
        if let Some(energy) = delta.energy {
            energy_died =
                !delta.host_energy_death_checked && self.alive && self.energy != 0 && energy == 0;
            self.energy = energy;
            // `DoEnergy` refreshes the bar timer after the change, whatever the
            // outcome (C4Object.cpp:1398).
            self.view_energy = C4_VIEW_DELAY;
        }
        if let Some(breath) = delta.breath {
            self.breath = breath;
            // `DoBreath` (C4Object.cpp:1419) and the asphyxiation arm
            // (C4Object.cpp:914) both refresh it.
            self.view_energy = C4_VIEW_DELAY;
        }
        if let Some(need_energy) = delta.need_energy {
            self.need_energy = need_energy;
        }
        if let Some(damage) = delta.damage {
            self.damage = damage.max(0);
        }
        if let Some((fire_caused_by, fire_phase)) = delta.fire {
            // Staged incinerate outcome — see ObjectUpdate::fire.
            self.on_fire = true;
            self.fire_caused_by = fire_caused_by;
            self.fire_phase = fire_phase;
        }
        if let Some(flag) = delta.fire_flag {
            // Bare SetOnFire write — see ObjectUpdate::fire_flag.
            self.on_fire = flag;
        }
        if let Some(magic_energy) = delta.magic_energy {
            self.magic_energy = magic_energy.max(0);
            // `FnDoMagicEnergy` refreshes it on a successful change
            // (C4Script.cpp:552).
            self.view_energy = C4_VIEW_DELAY;
        }
        if let Some(magic_capacity) = delta.magic_capacity {
            self.magic_capacity = magic_capacity.max(0);
        }
        if let Some(construction) = delta.construction {
            self.construction = if delta.construction_via_docon {
                construction.max(0)
            } else {
                construction.clamp(0, FULL_CON)
            };
        }
        if let Some(contact_density) = delta.contact_density {
            self.contact_density = contact_density;
        }
        if let Some(direction) = delta.direction {
            self.direction = direction;
        }
        if let Some(command_direction) = delta.command_direction {
            self.command_direction = command_direction;
        }
        let raw_change_def_idle = delta.change_def.is_some()
            && delta.action.as_ref().is_some_and(|action| {
                action.force
                    && action.callbacks_dispatched
                    && action.name.as_deref() == Some("Idle")
            });
        if let Some(action) = &delta.action {
            let requested_name_change = action.name.is_some();
            let previous_action = self.action.clone();
            let result = if raw_change_def_idle {
                // C4Object::ChangeDef's post-callback `Action.Act=ActIdle`
                // writes the action slot only. Apply the already-staged
                // callback payload without ActionState's ordinary implicit
                // name-change resets or new-library reconciliation.
                if let Some(name) = &action.name {
                    self.action.name = name.clone();
                    self.action.act_map_index = None;
                }
                if delta.change_def_reset_action_time {
                    self.action.time = 0;
                }
                if let Some(phase) = action.phase {
                    self.action.phase = phase;
                    if action.ticks.is_none() {
                        self.action.ticks = 0;
                    }
                }
                if let Some(ticks) = action.ticks {
                    self.action.ticks = ticks;
                }
                if let Some(data) = action.data {
                    self.action.data = data;
                }
                if let Some(target) = action.target {
                    self.action.target = target;
                }
                if let Some(target2) = action.target2 {
                    self.action.target2 = target2;
                }
                ActionUpdateResult::Applied
            } else {
                self.action.apply_update_with_library(action, library)
            };
            if matches!(result, ActionUpdateResult::Applied) {
                action_change = Some(ActionChange {
                    previous: previous_action,
                    requested_name_change,
                });
            }
        } else {
            self.action.reconcile_with_library(library);
        }
        if let Some(vertices) = &delta.vertices {
            self.vertices = vertices.clone();
        }
        // C4ObjectList::ShiftContents (C4ObjectList.cpp:815-833): cyclic
        // rotation so the target becomes First — relative order preserved.
        // An id not (or no longer) in the list is a no-op.
        if let Some(new_front) = delta.contents_front {
            if let Some(index) = self.contents.iter().position(|id| *id == new_front) {
                self.contents.rotate_left(index);
            }
        }
        if let Some(overlays) = &delta.graphics_overlays {
            self.graphics_overlays = overlays.clone();
        }
        if let Some(transform) = &delta.draw_transform {
            self.draw_transform = *transform;
        }
        if let Some(base_graphics) = &delta.base_graphics {
            self.base_graphics = base_graphics.clone();
        }
        if let Some(owner) = delta.owner {
            self.owner = owner;
            // SetOwner "automatically updates controller"
            // (C4Object.cpp:5499-5500); an explicit SetController in the
            // same batch still wins below.
            self.controller = delta.controller.unwrap_or(owner);
        } else if let Some(controller) = delta.controller {
            self.controller = controller;
        }
        if let Some(base) = delta.base {
            self.base = base;
        }
        if let Some(category) = delta.category {
            self.category = category;
        }
        if let Some(own_mass) = delta.own_mass {
            self.own_mass = own_mass;
        }
        if let Some(crew_member) = delta.crew_member {
            self.crew_member = crew_member;
        }
        if let Some(plr_view_range) = delta.plr_view_range {
            self.plr_view_range = plr_view_range;
        }
        if let Some(selected) = delta.selected {
            self.selected = selected;
        }
        if let Some(crew_disabled) = delta.crew_disabled {
            self.crew_disabled = crew_disabled;
        }
        if let Some(rect) = delta.solid_mask_override {
            self.solid_mask_override = Some(rect);
        }
        if let Some(menu) = &delta.menu {
            self.menu = menu.clone();
        }
        if let Some(shape_override) = delta.shape_override {
            self.shape_override = shape_override;
        }
        if let Some(alive) = delta.alive {
            self.alive = alive;
        }
        if let Some(entrance_status) = delta.entrance_status {
            self.entrance_status = entrance_status;
        }
        if let Some(status) = delta.status {
            self.status = status;
        }
        if let Some(container) = delta.container {
            if self.container != container {
                self.container = container;
                container_change = Some((previous_container, self.container));
                // C4Object::Enter/Exit force-close the moving object's menu
                // (CloseMenu(true), C4Object.cpp:1555/:1594). A delta that
                // carries its OWN menu write already folded the correct
                // post-Enter/Exit state above (script scopes stage the close
                // at Enter/Exit call time) — only menu-less deltas (the
                // engine-internal collect/grab/enter movers) close here.
                if delta.menu.is_none() {
                    self.menu = None;
                }
            }
        }
        if let Some(components) = &delta.components {
            self.component_order = normalized_component_order(
                components,
                delta
                    .component_order
                    .clone()
                    .unwrap_or_else(|| self.component_order.clone()),
                &[],
            );
            self.components = components.clone();
        } else if let Some(component_order) = &delta.component_order {
            self.component_order =
                normalized_component_order(&self.components, component_order.clone(), &[]);
        }
        if let Some(local_vars) = &delta.local_vars {
            self.local_vars = local_vars.clone().into();
        }
        if let Some(physicals) = &delta.physicals {
            self.info_physical = physicals.info;
            self.temporary_physical = physicals.temporary;
            self.physical_changes = physicals.changes.clone();
        }
        if let Some(ocf) = delta.ocf_override {
            self.ocf = ocf;
        }

        if !raw_change_def_idle {
            self.action.reconcile_with_library(library);
        }
        // A delta batches direction, action and draw transform, so the
        // C4Object::SetDir / SetAction folds (C4Object.cpp:4183-4184,
        // 4276-4279) collapse into one refresh after every input has landed —
        // notably after `delta.draw_transform`, which would otherwise clobber
        // an earlier fold. ChangeDef's raw `Action.Act = ActIdle` is excluded
        // because C++ writes it without one, and its trailing SetDir(0) is a
        // no-op on an idle object (C4Object.cpp:1224-1225,4264-4265).
        if !raw_change_def_idle && (delta.direction.is_some() || delta.action.is_some()) {
            self.update_flip_dir(self.action_flip_dir(library));
        }
        ApplyDeltaOutcome {
            energy_died,
            container_change,
            action_change: action_change.filter(|change| change.should_record(&self.action)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ChangeDefContentsSort {
    pub container: ObjectId,
    pub category: i32,
    pub definition_id: DefinitionId,
    pub unsorted: bool,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ObjectDelta {
    /// Non-field operation carried through callback/command aggregation.
    pub(crate) change_def: Option<String>,
    pub(crate) change_def_reinsert: bool,
    change_def_contents_sort: Option<ChangeDefContentsSort>,
    change_def_reset_action_time: bool,
    /// Some(Some(name)) sets C4Object::CustomName; Some(None) clears it.
    custom_name: Option<Option<String>>,
    /// Some(Some(object)) sets C4Object::pLayer; Some(None) clears it.
    layer: Option<Option<ObjectId>>,
    /// Exact C4Object compiler-cache overwrite. Typed pointer assignment does
    /// not touch this; literal-null resets and enumeration do.
    compiler_cache: Option<ObjectCompilerCache>,
    /// C4Object::Visibility overwrite.
    visibility: Option<i32>,
    /// C4Object::BlitMode overwrite.
    blit_mode: Option<u32>,
    /// C4Object::PictureRect overwrite (FnSetPicture).
    picture_rect: Option<DefinitionRect>,
    /// C4Object::Color overwrite (FnSetColorDw).
    color: Option<u32>,
    /// C4Object::ColorMod overwrite (FnSetClrModulation).
    color_modulation: Option<u32>,
    solid_mask_override: Option<DefinitionTargetRect>,
    /// Host-time C4SolidMask construction token. Callback copy-out may fold
    /// object updates and spawns in a different order than C++; carrying the
    /// token preserves the native linked-list age across that boundary.
    solid_mask_instance_sequence: Option<u64>,
    /// Script menu write-through (FnCreateMenu/FnCloseMenu et al.):
    /// Some(None) = closed, Some(Some(_)) = open/replaced.
    menu: Option<Option<ObjectMenuState>>,
    /// Some(Some(rect)) installs SetShape; Some(None) is UpdateShape's
    /// explicit restoration of the definition shape.
    shape_override: Option<Option<DefinitionRect>>,
    pub(crate) position: Option<Vector2>,
    pub(crate) velocity: Option<Vector2>,
    /// Runtime C4Object::InLiquid overwrite from a synchronous host call.
    in_liquid: Option<bool>,
    /// Sub-pixel velocity in 16.16 fixed-point. When present, this takes
    /// precedence over the whole-pixel `velocity` mirror so that script
    /// surfaces (e.g. `SetXDir`) can express fractional `C4Fixed` velocity
    /// exactly, matching C++ `pObj->xdir = itofix(n, prec)` (`C4Script.cpp:697`).
    fixed_velocity: Option<FixedVec2>,
    /// Component-only dir writes (FnSetXDir/FnSetYDir, C4Script.cpp:
    /// 697-732): C++ assigns ONE of xdir/ydir — the other keeps its
    /// full sub-pixel value (a whole-vector write from a script scope
    /// would quantize it through the int mirror).
    fixed_velocity_x: Option<C4Fixed>,
    fixed_velocity_y: Option<C4Fixed>,
    /// Explicit `C4Object::Mobile` overwrite for native object helpers whose
    /// velocity assignment does not share the script Set*Dir mobilization
    /// rule (notably C4Object::Fling's Tumble branch).
    mobile: Option<bool>,
    /// Explicit `C4Object::Action.t_attach` overwrite. Native Fling clears
    /// either CNAT_Bottom or the complete mask in the same frame.
    t_attach: Option<u32>,
    /// Live per-object C4Shape::ContactDensity overwrite.
    contact_density: Option<i32>,
    rotation: Option<i32>,
    /// Sub-pixel angular velocity (16.16 fixed-point degrees/frame) set by
    /// `SetRDir`. Mirrors C++ `pObj->rdir = itofix(n, prec)` (`C4Script.cpp:710`).
    rotation_velocity: Option<C4Fixed>,
    /// Native rdir write that must not arm `Mobile`; see the update field.
    rotation_velocity_raw: Option<C4Fixed>,
    pub(crate) energy: Option<i32>,
    /// The energy write already evaluated C4Object::DoEnergy's synchronous
    /// zero-crossing predicate at its host-call site. This marker belongs to
    /// this specific write: a later merged energy overwrite replaces it.
    host_energy_death_checked: bool,
    /// C4Object::Breath overwrite staged by FnDoBreath.
    breath: Option<i32>,
    /// C4Object::NeedEnergy overwrite.
    need_energy: Option<bool>,
    /// Kill-trace mark riding an energy write (C4Object.cpp:1351-1353).
    energy_loss_cause: Option<i32>,
    /// Staged incinerate outcome `(caused_by, fire_phase)` — see
    /// `ObjectUpdate::fire`.
    fire: Option<(i32, i32)>,
    /// Bare OnFire flag write — see `ObjectUpdate::fire_flag`.
    fire_flag: Option<bool>,
    damage: Option<i32>,
    magic_energy: Option<i32>,
    magic_capacity: Option<i32>,
    construction: Option<i32>,
    /// The construction write came from C4Object::DoCon and therefore keeps
    /// DoCon-specific component/lifecycle semantics.
    construction_via_docon: bool,
    /// No later SetAction/position write resynchronized fix_x/fix_y after the
    /// staged DoCon bottom adjustment.
    construction_preserves_fixed_position: bool,
    resolved_docon_position: Option<Vector2>,
    resolved_docon_fixed_position: Option<FixedVec2>,
    pub(crate) direction: Option<Direction>,
    pub(crate) command_direction: Option<CommandDirection>,
    pub(crate) action: Option<ActionUpdate>,
    status: Option<ObjectStatus>,
    /// The native host already executed this Enter/Exit synchronously. The
    /// engine fold must reconcile only the deferred Contents link rather
    /// than replaying motion, shape, controller, and cached-OCF effects.
    pub(crate) host_container_change: bool,
    /// Final cached C4Object::OCF after the host's ordered native calls.
    ocf_override: Option<u32>,
    pub(crate) owner: Option<i32>,
    /// C4Object::Base overwrite used by FLAG/FlyBase SetOwner propagation.
    base: Option<i32>,
    /// FnSetController (C4Script.cpp:1322-1331) / SetOwner's automatic
    /// controller update (C4Object.cpp:5499-5500).
    controller: Option<i32>,
    category: Option<i32>,
    own_mass: Option<i32>,
    crew_member: Option<bool>,
    plr_view_range: Option<i32>,
    selected: Option<bool>,
    crew_disabled: Option<bool>,
    alive: Option<bool>,
    entrance_status: Option<bool>,
    container: Option<Option<ObjectId>>,
    /// Direct overwrite of the current `C4Object::Shape` vertex list. Unlike
    /// `vertices`, this does not enable `fOwnVertices` or replace the
    /// untransformed shape base.
    live_vertices: Option<Vec<ObjectVertex>>,
    /// Exact fixed-slot C4Shape overwrite, including dormant slots beyond
    /// VtxNum. This wins over `live_vertices` when both are present.
    shape_vertices: Option<ShapeVertexBuffer>,
    /// Permanent own-vertex base used by SetVertex's own-vertex modes.
    vertices: Option<Vec<ObjectVertex>>,
    /// `VTX_Set` installs the base without the `UpdateShape` that
    /// `VTX_SetPermanentUpd` performs (C4Script.cpp:1324-1325).
    vertices_defer_shape_update: bool,
    graphics_overlays: Option<Vec<ObjectGraphicsOverlay>>,
    draw_transform: Option<Option<DrawTransform>>,
    base_graphics: Option<Option<ObjectBaseGraphics>>,
    components: Option<ComponentList>,
    component_order: Option<Vec<DefinitionId>>,
    material_contents: Option<Vec<i32>>,
    pub(crate) local_vars: Option<HashMap<String, Value>>,
    physicals: Option<PhysicalsUpdate>,
    /// Rotate `contents` cyclically so this id becomes the front —
    /// C4ObjectList::ShiftContents (C4ObjectList.cpp:815-833).
    contents_front: Option<ObjectId>,
}

impl ObjectDelta {
    pub(crate) fn ocf_override(&self) -> Option<u32> {
        self.ocf_override
    }

    pub(crate) fn merge_update(&mut self, update: ObjectUpdate) {
        self.host_container_change |= update.host_container_change;
        if let Some(ocf) = update.ocf_override {
            self.ocf_override = Some(ocf);
        }
        let changes_definition = update.change_def.is_some();
        if let Some(change_def) = update.change_def.as_ref() {
            self.change_def = Some(change_def.clone());
        }
        if changes_definition {
            self.change_def_reinsert = update.change_def_reinsert;
            self.change_def_contents_sort = update.change_def_contents_sort.clone();
            self.change_def_reset_action_time = update.change_def_reset_action_time;
        } else if update.change_def_reinsert {
            self.change_def_reinsert = true;
        }
        let writes_construction = update.construction.is_some();
        let construction_via_docon = update.construction_via_docon;
        let construction_preserves_fixed_position = update.construction_preserves_fixed_position;
        let resynchronizes_fixed_position = update.position.is_some()
            || update
                .action
                .as_ref()
                .is_some_and(|action| action.name.is_some());
        let replaces_docon_position =
            update.position.is_some() || (writes_construction && !construction_via_docon);
        if replaces_docon_position && update.resolved_docon_position.is_none() {
            self.resolved_docon_position = None;
        }
        if (resynchronizes_fixed_position || replaces_docon_position)
            && update.resolved_docon_fixed_position.is_none()
        {
            self.resolved_docon_fixed_position = None;
        }
        if let Some(custom_name) = update.custom_name {
            self.custom_name = Some(custom_name);
        }
        if let Some(layer) = update.layer {
            self.layer = Some(layer);
        }
        if let Some(compiler_cache) = update.compiler_cache {
            self.compiler_cache = Some(compiler_cache);
        }
        if let Some(visibility) = update.visibility {
            self.visibility = Some(visibility);
        }
        if let Some(blit_mode) = update.blit_mode {
            self.blit_mode = Some(blit_mode);
        }
        if let Some(picture_rect) = update.picture_rect {
            self.picture_rect = Some(picture_rect);
        }
        if let Some(color) = update.color {
            self.color = Some(color);
        }
        if let Some(color_modulation) = update.color_modulation {
            self.color_modulation = Some(color_modulation);
        }
        if let Some(position) = update.position {
            self.position = Some(position);
        }
        if let Some(in_liquid) = update.in_liquid {
            self.in_liquid = Some(in_liquid);
        }
        if let Some(position) = update.resolved_docon_position {
            self.resolved_docon_position = Some(position);
        }
        if let Some(position) = update.resolved_docon_fixed_position {
            self.resolved_docon_fixed_position = Some(position);
        }
        if let Some(own_mass) = update.own_mass {
            self.own_mass = Some(own_mass);
        }
        if let Some(velocity) = update.velocity {
            self.velocity = Some(velocity);
        }
        if let Some(fixed_velocity) = update.fixed_velocity {
            self.fixed_velocity = Some(fixed_velocity);
        }
        if let Some(x) = update.fixed_velocity_x {
            self.fixed_velocity_x = Some(x);
        }
        if let Some(y) = update.fixed_velocity_y {
            self.fixed_velocity_y = Some(y);
        }
        if let Some(mobile) = update.mobile {
            self.mobile = Some(mobile);
        }
        if let Some(t_attach) = update.t_attach {
            self.t_attach = Some(t_attach);
        }
        if let Some(contact_density) = update.contact_density {
            self.contact_density = Some(contact_density);
        }
        if let Some(rotation) = update.rotation {
            self.rotation = Some(rotation);
        }
        if let Some(rotation_velocity) = update.rotation_velocity {
            self.rotation_velocity = Some(rotation_velocity);
        }
        if let Some(rotation_velocity) = update.rotation_velocity_raw {
            self.rotation_velocity_raw = Some(rotation_velocity);
        }
        if let Some(energy) = update.energy {
            self.energy = Some(energy);
            self.host_energy_death_checked = update.host_energy_death_checked;
        }
        if let Some(breath) = update.breath {
            self.breath = Some(breath);
        }
        if let Some(need_energy) = update.need_energy {
            self.need_energy = Some(need_energy);
        }
        if let Some(cause) = update.energy_loss_cause {
            self.energy_loss_cause = Some(cause);
        }
        if let Some(fire) = update.fire {
            self.fire = Some(fire);
            // stage_ignite is the later write when no bare flag accompanies
            // this update, so it supersedes a previously merged flag.
            self.fire_flag = update.fire_flag;
        } else if let Some(flag) = update.fire_flag {
            // A bare SetOnFire changes only the flag and retains any phase
            // and attribution payload staged earlier in the batch.
            self.fire_flag = Some(flag);
        }
        if let Some(construction) = update.construction {
            self.construction = Some(construction);
        }
        if writes_construction {
            self.construction_via_docon = construction_via_docon;
            self.construction_preserves_fixed_position = construction_preserves_fixed_position;
        } else if resynchronizes_fixed_position {
            self.construction_preserves_fixed_position = false;
        }
        if let Some(damage) = update.damage {
            self.damage = Some(damage);
        }
        if let Some(magic_energy) = update.magic_energy {
            self.magic_energy = Some(magic_energy);
        }
        if let Some(magic_capacity) = update.magic_capacity {
            self.magic_capacity = Some(magic_capacity);
        }
        if let Some(direction) = update.direction {
            self.direction = Some(direction);
        }
        if let Some(command_direction) = update.command_direction {
            self.command_direction = Some(command_direction);
        }
        if let Some(owner) = update.owner {
            self.owner = Some(owner);
        }
        if let Some(base) = update.base {
            self.base = Some(base);
        }
        if let Some(controller) = update.controller {
            self.controller = Some(controller);
        }
        if let Some(category) = update.category {
            self.category = Some(category);
        }
        if let Some(crew_member) = update.crew_member {
            self.crew_member = Some(crew_member);
        }
        if let Some(plr_view_range) = update.plr_view_range {
            self.plr_view_range = Some(plr_view_range);
        }
        if let Some(selected) = update.selected {
            self.selected = Some(selected);
        }
        if let Some(crew_disabled) = update.crew_disabled {
            self.crew_disabled = Some(crew_disabled);
        }
        if let Some(rect) = update.solid_mask_override {
            self.solid_mask_override = Some(rect);
        }
        if let Some(sequence) = update.solid_mask_instance_sequence {
            self.solid_mask_instance_sequence = Some(sequence);
        }
        if let Some(menu) = update.menu {
            self.menu = Some(menu);
        }
        if let Some(shape_override) = update.shape_override {
            self.shape_override = Some(shape_override);
        }

        if let Some(alive) = update.alive {
            self.alive = Some(alive);
        }
        if let Some(entrance_status) = update.entrance_status {
            self.entrance_status = Some(entrance_status);
        }
        if let Some(container) = update.container {
            self.container = Some(container);
        }
        if let Some(status) = update.status {
            self.status = Some(status);
        }
        if let Some(vertices) = update.live_vertices {
            self.live_vertices = Some(vertices);
        }
        if let Some(vertices) = update.shape_vertices {
            self.shape_vertices = Some(vertices);
        }
        if let Some(vertices) = update.vertices {
            self.vertices = Some(vertices);
            self.vertices_defer_shape_update = update.vertices_defer_shape_update;
        }
        if let Some(overlays) = update.graphics_overlays {
            self.graphics_overlays = Some(overlays);
        }
        if let Some(transform) = update.draw_transform {
            self.draw_transform = Some(transform);
        }
        if let Some(base_graphics) = update.base_graphics {
            self.base_graphics = Some(base_graphics);
        }
        if let Some(components) = update.components {
            self.components = Some(components);
        }
        if let Some(component_order) = update.component_order {
            self.component_order = Some(component_order);
        }
        if let Some(material_contents) = update.material_contents {
            self.material_contents = Some(material_contents);
        }
        if let Some(physicals) = update.physicals {
            self.physicals = Some(physicals);
        }
        if let Some(contents_front) = update.contents_front {
            self.contents_front = Some(contents_front);
        }
        if let Some(action) = update.action {
            match &mut self.action {
                Some(existing) => existing.merge(action),
                None => self.action = Some(action),
            }
        }
    }
}

impl From<ObjectUpdate> for ObjectDelta {
    fn from(update: ObjectUpdate) -> Self {
        Self {
            change_def: update.change_def,
            change_def_reinsert: update.change_def_reinsert,
            change_def_contents_sort: update.change_def_contents_sort,
            change_def_reset_action_time: update.change_def_reset_action_time,
            custom_name: update.custom_name,
            layer: update.layer,
            compiler_cache: update.compiler_cache,
            visibility: update.visibility,
            blit_mode: update.blit_mode,
            picture_rect: update.picture_rect,
            color: update.color,
            color_modulation: update.color_modulation,
            fixed_velocity_x: update.fixed_velocity_x,
            fixed_velocity_y: update.fixed_velocity_y,
            position: update.position,
            in_liquid: update.in_liquid,
            own_mass: update.own_mass,
            velocity: update.velocity,
            fixed_velocity: update.fixed_velocity,
            mobile: update.mobile,
            t_attach: update.t_attach,
            contact_density: update.contact_density,
            rotation: update.rotation,
            rotation_velocity: update.rotation_velocity,
            rotation_velocity_raw: update.rotation_velocity_raw,
            energy: update.energy,
            host_energy_death_checked: update.host_energy_death_checked,
            breath: update.breath,
            need_energy: update.need_energy,
            energy_loss_cause: update.energy_loss_cause,
            fire: update.fire,
            fire_flag: update.fire_flag,
            construction: update.construction,
            construction_via_docon: update.construction_via_docon,
            construction_preserves_fixed_position: update.construction_preserves_fixed_position,
            resolved_docon_position: update.resolved_docon_position,
            resolved_docon_fixed_position: update.resolved_docon_fixed_position,
            damage: update.damage,
            magic_energy: update.magic_energy,
            magic_capacity: update.magic_capacity,
            direction: update.direction,
            command_direction: update.command_direction,
            action: update.action,
            status: update.status,
            host_container_change: update.host_container_change,
            ocf_override: update.ocf_override,
            owner: update.owner,
            base: update.base,
            controller: update.controller,
            category: update.category,
            crew_member: update.crew_member,
            plr_view_range: update.plr_view_range,
            selected: update.selected,
            crew_disabled: update.crew_disabled,
            solid_mask_override: update.solid_mask_override,
            solid_mask_instance_sequence: update.solid_mask_instance_sequence,
            menu: update.menu,
            shape_override: update.shape_override,
            alive: update.alive,
            entrance_status: update.entrance_status,
            container: update.container,
            live_vertices: update.live_vertices,
            shape_vertices: update.shape_vertices,
            vertices: update.vertices,
            vertices_defer_shape_update: update.vertices_defer_shape_update,
            graphics_overlays: update.graphics_overlays,
            draw_transform: update.draw_transform,
            base_graphics: update.base_graphics,
            components: update.components,
            component_order: update.component_order,
            material_contents: update.material_contents,
            local_vars: update.local_vars,
            physicals: update.physicals,
            contents_front: update.contents_front,
        }
    }
}

/// Serde's default nested-Option handling collapses an explicit JSON `null`
/// into the same outer `None` as a missing field. Update fields need all three
/// states, so a present value always wraps the decoded inner option.
fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn serialize_double_optional_c4_string<S>(
    value: &Option<Option<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    struct C4StringRef<'a>(&'a str);

    impl serde::Serialize for C4StringRef<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            clonk_script::c4_string_serde::serialize_ref(self.0, serializer)
        }
    }

    match value {
        Some(Some(value)) => serializer.serialize_some(&C4StringRef(value)),
        Some(None) | None => serializer.serialize_none(),
    }
}

fn deserialize_double_optional_c4_string<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct C4String(#[serde(with = "clonk_script::c4_string_serde")] String);

    Option::<C4String>::deserialize(deserializer).map(|value| Some(value.map(|value| value.0)))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectUpdate {
    /// C4Object::CustomName write: Some(Some(name)) sets; Some(None) clears.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_double_optional_c4_string",
        deserialize_with = "deserialize_double_optional_c4_string"
    )]
    pub custom_name: Option<Option<String>>,
    /// C4Object::pLayer write: Some(Some(object)) sets; Some(None) clears.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub layer: Option<Option<ObjectId>>,
    /// Runtime-visible compiler caches. Kept separate from the associated live
    /// pointers because C4EnumeratedObjectPtr only clears its number for a
    /// literal-null assignment and only refreshes it during enumeration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub compiler_cache: Option<ObjectCompilerCache>,
    /// SetVisibility (C4Script.cpp:3860-3869).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<i32>,
    /// C4Object::BlitMode overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blit_mode: Option<u32>,
    /// FnSetPicture's raw C4Object::PictureRect overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_rect: Option<DefinitionRect>,
    /// SetSolidMask's rect update (Some = set; zero-area = mask OFF).
    #[serde(default)]
    pub solid_mask_override: Option<DefinitionTargetRect>,
    /// Runtime-only C4SolidMask construction order reserved while a script
    /// callback is still executing. It must not enter save/control data.
    #[serde(skip)]
    #[doc(hidden)]
    pub solid_mask_instance_sequence: Option<u64>,
    /// FnChangeDef's definition swap (C4Object::ChangeDef,
    /// C4Object.cpp:1180-1231).
    #[serde(default)]
    pub change_def: Option<String>,
    /// A callback removed and re-added this object's contents link. The
    /// final container may equal the pre-call value, so an ordinary Option
    /// delta would otherwise collapse the mandatory remove/re-add into a
    /// no-op. ChangeDef additionally carries its special sort metadata.
    #[serde(default, skip_serializing_if = "is_false")]
    #[doc(hidden)]
    pub change_def_reinsert: bool,
    /// Sort key of a contents link established by an old-definition action
    /// callback before ChangeDef swaps Def. A vetoed final Enter leaves that
    /// exact link in place rather than re-adding it as Unsorted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub change_def_contents_sort: Option<ChangeDefContentsSort>,
    /// The ordinary SetAction(ActIdle) phase of ChangeDef succeeded, so its
    /// action-name transition reset C4Action::Time before the unconditional
    /// raw ActIdle slot overwrite.
    #[serde(default, skip_serializing_if = "is_false")]
    #[doc(hidden)]
    pub change_def_reset_action_time: bool,
    pub position: Option<Vector2>,
    pub velocity: Option<Vector2>,
    /// Runtime C4Object::InLiquid overwrite; it is not serialized into
    /// queued control data.
    #[serde(skip)]
    pub in_liquid: Option<bool>,
    /// Sub-pixel velocity in 16.16 fixed-point, set by precision-aware script
    /// surfaces (`SetXDir`/`SetYDir`). Takes precedence over `velocity` when
    /// applied. Mirrors C++ storing velocity as `C4Fixed` (`C4Object.h` xdir/ydir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    /// See ObjectDelta::fixed_velocity_x — component-only SetXDir/SetYDir.
    pub fixed_velocity_x: Option<C4Fixed>,
    pub fixed_velocity_y: Option<C4Fixed>,
    /// Explicit C4Object::Mobile overwrite. Applied after the generic
    /// velocity-write mobilization so native helpers can preserve or force
    /// the exact C++ branch behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile: Option<bool>,
    /// Explicit C4Object::Action.t_attach overwrite for same-frame native
    /// attachment changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_attach: Option<u32>,
    /// Sub-pixel angular velocity (16.16 fixed degrees/frame) from `SetRDir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    /// The same value written the way `C4Object::AdjustWalkRotation` writes
    /// it — straight onto `rdir` with **no** `Mobile` arming
    /// (C4Object.cpp:6085-6088). `FnSetRDir` is the path that mobilises
    /// (C4Script.cpp:732); a native rdir write must not, or the staged field
    /// re-mobilises the object in a later fold, after ExecMovement has
    /// demobilised it (clonk-org/clonk-rs#1157).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity_raw: Option<C4Fixed>,
    #[serde(default)]
    pub rotation: Option<i32>,
    pub energy: Option<i32>,
    /// Runtime-only marker that this exact energy write already evaluated
    /// the native DoEnergy death predicate synchronously. It prevents the
    /// later callback fold from replaying AssignDeath after a revival or a
    /// SetAlive change, and never enters save/control data.
    #[serde(skip)]
    #[doc(hidden)]
    pub host_energy_death_checked: bool,
    /// C4Object::Breath overwrite on the raw physical scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breath: Option<i32>,
    /// C4Object::NeedEnergy overwrite (FnEnergyCheck).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_energy: Option<bool>,
    /// C4Object::DoEnergy's kill-trace mark (C4Object.cpp:1351-1353),
    /// staged with the UpdatLastEnergyLossCause guard (:1369-1378) already
    /// applied at call time — the fold writes it through unconditionally
    /// (Punch's post-fling write is unguarded too, C4ObjectCom.cpp:755,762).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_loss_cause: Option<i32>,
    /// Staged C4Object::Incinerate outcome `(caused_by, fire_phase)` from
    /// the host seam — the RNG draw and the Incineration callback already
    /// ran mid-call; the fold only writes the fire state bits
    /// (fxFireStart core, C4Effect.cpp:632-634).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire: Option<(i32, i32)>,
    /// Bare OnFire flag write — the engine-internal FnFxFireStop /
    /// FnFxFireStart temp arms (C4Effect.cpp:563-565, 775-791). A preceding
    /// `fire` payload stays staged so SetOnFire can change only the flag
    /// without discarding the already-written cause and phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_flag: Option<bool>,
    #[serde(default)]
    pub damage: Option<i32>,
    #[serde(default)]
    pub magic_energy: Option<i32>,
    #[serde(default)]
    pub magic_capacity: Option<i32>,
    #[serde(default)]
    pub construction: Option<i32>,
    /// Internal provenance for a staged C4Object::DoCon write. Generic
    /// construction assignments still resynchronize fixed position.
    #[serde(default, skip_serializing_if = "is_false")]
    pub construction_via_docon: bool,
    /// Final fixed-position ordering for a staged DoCon. SetAction and
    /// explicit position writes after DoCon clear this bit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub construction_preserves_fixed_position: bool,
    /// Runtime-only host fold of DoCon's sequential integer/fixed position.
    #[serde(skip)]
    pub resolved_docon_position: Option<Vector2>,
    #[serde(skip)]
    pub resolved_docon_fixed_position: Option<FixedVec2>,
    pub action: Option<ActionUpdate>,
    #[serde(default)]
    pub direction: Option<Direction>,
    #[serde(default)]
    pub command_direction: Option<CommandDirection>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    /// Final cached C4Object::OCF overwrite for native helpers whose
    /// temporary state transition intentionally leaves the cache stale.
    /// FnCollect restores NoCollectDelay without a second UpdateOCF
    /// (C4Script.cpp:397-413), so its temporary Collection bit survives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocf_override: Option<u32>,
    #[serde(default)]
    pub owner: Option<i32>,
    /// C4Object::Base overwrite used by FLAG/FlyBase SetOwner propagation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<i32>,
    /// FnSetController (C4Script.cpp:1322-1331); also recorded by
    /// SetOwner's automatic controller update (C4Object.cpp:5499-5500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<i32>,
    #[serde(default)]
    pub category: Option<i32>,
    /// SetMass (C4Script.cpp:3620-3626): OwnMass = value - Def->Mass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_mass: Option<i32>,
    #[serde(default)]
    pub crew_member: Option<bool>,
    /// FnSetPlrViewRange / MakeCrewMember's zero-to-default update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plr_view_range: Option<i32>,
    /// SetObjectCrewStatus changes the player's roster without running
    /// AdjustCursorCommand; suppress the generic crew-bit cursor repair.
    #[serde(default, skip_serializing_if = "is_false")]
    pub crew_status_change: bool,
    /// Live C4Object::Info rank write. Some(Some(rank)) attaches/updates
    /// rank data; Some(None) clears the linked info rank.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub info_rank: Option<Option<i32>>,
    /// Player whose CrewInfoList owns the live C4Object::Info pointer.
    /// Some(None) clears that pointer ownership alongside info_rank.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub info_link: Option<Option<CrewInfoLink>>,
    /// C4Object::Select overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    pub crew_disabled: Option<bool>,
    #[serde(default)]
    pub alive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Option<ObjectId>>,
    /// Runtime provenance for Enter/Exit performed synchronously by the
    /// script host. Save/control data carries only the resulting fields.
    #[serde(skip)]
    #[doc(hidden)]
    pub host_container_change: bool,
    /// Direct current-shape vertex overwrite. This deliberately leaves the
    /// object's own-vertex mode unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_vertices: Option<Vec<ObjectVertex>>,
    /// Exact current C4Shape slot overwrite, including dormant slots beyond
    /// the active vertex count. Internal host mutations use this so delayed
    /// command/state serialization cannot discard RemoveVertex metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub shape_vertices: Option<ShapeVertexBuffer>,
    /// FnSetContactDensity's live C4Shape::ContactDensity overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_density: Option<i32>,
    /// Permanent own-vertex base overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<ObjectVertex>>,
    /// `SetVertex`'s plain own-vertex mode (`VTX_Set`) writes only the shape's
    /// backup half; the live shape keeps its vertices until some later
    /// `UpdateShape` restores them (C4Script.cpp:1297-1325). Set this so the
    /// accompanying `vertices` base does not run `UpdateShape` right away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[doc(hidden)]
    pub vertices_defer_shape_update: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_overlays: Option<Vec<ObjectGraphicsOverlay>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<Option<DrawTransform>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<Option<ObjectBaseGraphics>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<ComponentList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_order: Option<Vec<DefinitionId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_vars: Option<HashMap<String, Value>>,
    /// Full physical-state overwrite from the physicals host functions
    /// (SetPhysical/TrainPhysical/ResetPhysical, C4Script.cpp:552-636).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physicals: Option<PhysicalsUpdate>,
    /// SetEntrance (C4Script.cpp:690-695).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrance_status: Option<bool>,
    /// Script menu write: Some(Some(_)) = CreateMenu/AddMenuItem/
    /// SelectMenuItem left this state, Some(None) = CloseMenu
    /// (C4Object::CloseMenu, C4Object.cpp:2009-2017).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Option<ObjectMenuState>>,
    /// SetColorDw (C4Script.cpp:3661-3668).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    /// SetClrModulation's raw C4Object::ColorMod overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_modulation: Option<u32>,
    /// SetShape / UpdateShape: Some(Some(rect)) installs an override;
    /// Some(None) clears it when UpdateFace(true) rebuilds a non-line shape.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub shape_override: Option<Option<DefinitionRect>>,
    /// Runtime-only C4Object::MaterialContents overwrite produced by the
    /// synchronous DigFree host preview. It never enters save/control data.
    #[serde(skip)]
    #[doc(hidden)]
    pub material_contents: Option<Vec<i32>>,
    /// Rotate the contents list cyclically so this id becomes the front —
    /// C4ObjectList::ShiftContents (C4ObjectList.cpp:815-833), the
    /// FnShiftContents/DirectComContents write path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_front: Option<ObjectId>,
}

/// The complete per-object physical state as left by a script callback —
/// applied wholesale (a cleared temporary set must overwrite engine state).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhysicalsUpdate {
    pub info: Option<PhysicalInfo>,
    pub temporary: Option<PhysicalInfo>,
    pub changes: Vec<(String, i32)>,
}

impl ObjectUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    /// The staged OnFire outcome, if any fire write is pending.
    pub fn staged_on_fire(&self) -> Option<bool> {
        self.fire_flag.or_else(|| self.fire.map(|_| true))
    }

    /// Stage the full fxFireStart outcome (C4Effect.cpp:632-634).
    pub fn stage_ignite(&mut self, caused_by: i32, phase: i32) {
        self.fire = Some((caused_by, phase));
        self.fire_flag = None;
    }

    /// Stage a bare SetOnFire write (FnFxFireStop / temp arms,
    /// C4Effect.cpp:563-565, 775-791).
    pub fn stage_fire_flag(&mut self, flag: bool) {
        self.fire_flag = Some(flag);
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_contents_front(mut self, contents_front: ObjectId) -> Self {
        self.contents_front = Some(contents_front);
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = Some(velocity);
        self
    }

    pub fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = Some(in_liquid);
        self
    }

    pub fn with_rotation(mut self, rotation: i32) -> Self {
        self.rotation = Some(rotation);
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = Some(energy);
        self
    }

    pub fn with_breath(mut self, breath: i32) -> Self {
        self.breath = Some(breath);
        self
    }

    pub fn with_need_energy(mut self, need_energy: bool) -> Self {
        self.need_energy = Some(need_energy);
        self
    }

    pub fn with_damage(mut self, damage: i32) -> Self {
        self.damage = Some(damage);
        self
    }

    pub fn with_construction(mut self, construction: i32) -> Self {
        self.construction = Some(construction.clamp(0, FULL_CON));
        self
    }

    pub fn with_contact_density(mut self, contact_density: i32) -> Self {
        self.contact_density = Some(contact_density);
        self
    }

    pub fn with_magic_energy(mut self, magic_energy: i32) -> Self {
        self.magic_energy = Some(magic_energy);
        self
    }

    pub fn with_magic_capacity(mut self, magic_capacity: i32) -> Self {
        self.magic_capacity = Some(magic_capacity);
        self
    }

    pub fn set_damage(&mut self, damage: i32) {
        self.damage = Some(damage);
    }

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn with_command_direction(mut self, command_direction: CommandDirection) -> Self {
        self.command_direction = Some(command_direction);
        self
    }

    pub fn with_action(mut self, name: impl Into<String>) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_name(name);
        self.action = Some(update);
        self
    }

    pub fn with_action_phase(mut self, phase: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_phase(phase);
        self.action = Some(update);
        self
    }

    pub fn with_action_ticks(mut self, ticks: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_ticks(ticks);
        self.action = Some(update);
        self
    }

    pub fn with_action_data(mut self, data: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_data(data);
        self.action = Some(update);
        self
    }

    pub fn with_action_update(mut self, update: ActionUpdate) -> Self {
        self.action = Some(update);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn with_base(mut self, base: i32) -> Self {
        self.base = Some(base);
        self
    }

    pub fn with_category(mut self, category: i32) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_status(mut self, status: ObjectStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_container(mut self, container: ObjectId) -> Self {
        self.container = Some(Some(container));
        self
    }

    pub fn with_layer(mut self, layer: ObjectId) -> Self {
        self.layer = Some(Some(layer));
        self
    }

    pub fn clear_layer(mut self) -> Self {
        self.layer = Some(None);
        self
    }

    pub fn with_blit_mode(mut self, blit_mode: u32) -> Self {
        self.blit_mode = Some(blit_mode);
        self
    }

    pub fn clear_container(mut self) -> Self {
        self.container = Some(None);
        self
    }

    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = Some(crew_member);
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = Some(alive);
        self
    }

    /// Whether the staged host operations call `C4Object::SetOCF` at the
    /// mutation site. Plain callback completion and velocity/direction writes
    /// leave Execute's cached OCF untouched (C4Object.cpp:1082-1093).
    pub(crate) fn refreshes_ocf_like_cpp(&self) -> bool {
        self.change_def.is_some()
            || self.construction.is_some()
            || self.container.is_some()
            || self
                .action
                .as_ref()
                .is_some_and(|action| action.name.is_some())
            || self.category.is_some()
            || self.alive.is_some()
            || self.fire.is_some()
            || self.fire_flag.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.custom_name.is_none()
            && self.layer.is_none()
            && self.compiler_cache.is_none()
            && self.visibility.is_none()
            && self.blit_mode.is_none()
            && self.picture_rect.is_none()
            && self.color.is_none()
            && self.color_modulation.is_none()
            && self.solid_mask_override.is_none()
            && self.solid_mask_instance_sequence.is_none()
            && self.shape_override.is_none()
            && self.material_contents.is_none()
            && self.change_def.is_none()
            && !self.change_def_reinsert
            && self.change_def_contents_sort.is_none()
            && !self.change_def_reset_action_time
            && self.position.is_none()
            && self.resolved_docon_position.is_none()
            && self.resolved_docon_fixed_position.is_none()
            && self.velocity.is_none()
            && self.in_liquid.is_none()
            && self.fixed_velocity.is_none()
            && self.fixed_velocity_x.is_none()
            && self.fixed_velocity_y.is_none()
            && self.mobile.is_none()
            && self.t_attach.is_none()
            && self.rotation.is_none()
            && self.rotation_velocity.is_none()
            && self.rotation_velocity_raw.is_none()
            && self.energy.is_none()
            && self.breath.is_none()
            && self.need_energy.is_none()
            && self.energy_loss_cause.is_none()
            && self.fire.is_none()
            && self.fire_flag.is_none()
            && self.construction.is_none()
            && self.damage.is_none()
            && self.magic_energy.is_none()
            && self.magic_capacity.is_none()
            && self.direction.is_none()
            && self.command_direction.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.ocf_override.is_none()
            && self.owner.is_none()
            && self.base.is_none()
            && self.controller.is_none()
            && self.category.is_none()
            && self.crew_member.is_none()
            && self.plr_view_range.is_none()
            && !self.crew_status_change
            && self.info_rank.is_none()
            && self.info_link.is_none()
            && self.selected.is_none()
            && self.alive.is_none()
            && self.entrance_status.is_none()
            && self.container.is_none()
            && !self.host_container_change
            && self.live_vertices.is_none()
            && self.shape_vertices.is_none()
            && self.contact_density.is_none()
            && self.vertices.is_none()
            && self.graphics_overlays.is_none()
            && self.draw_transform.is_none()
            && self.base_graphics.is_none()
            && self.components.is_none()
            && self.component_order.is_none()
            && self.physicals.is_none()
            && self.contents_front.is_none()
            && self.menu.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub delay: u32,
    pub update: ObjectUpdate,
    pub effects: Vec<EffectCommand>,
    #[serde(default)]
    pub events: Vec<CommandEvent>,
    pub destroy: bool,
    pub spawns: Vec<SpawnConfig>,
    #[serde(default)]
    pub landscape: Vec<LandscapeCommand>,
    #[serde(default)]
    pub particles: Vec<ParticleCommand>,
}

impl QueuedCommand {
    pub fn new(delay: u32, update: ObjectUpdate) -> Self {
        Self {
            delay,
            update,
            effects: Vec::new(),
            events: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
        }
    }

    pub fn immediate(update: ObjectUpdate) -> Self {
        Self {
            delay: 0,
            update,
            effects: Vec::new(),
            events: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
        }
    }

    pub fn with_delay(mut self, delay: u32) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_effects(mut self, effects: Vec<EffectCommand>) -> Self {
        self.effects = effects;
        self
    }

    pub fn with_events(mut self, events: Vec<CommandEvent>) -> Self {
        self.events = events;
        self
    }

    pub fn with_destroy(mut self, destroy: bool) -> Self {
        self.destroy = destroy;
        self
    }

    pub fn with_spawns(mut self, spawns: Vec<SpawnConfig>) -> Self {
        self.spawns = spawns;
        self
    }

    pub fn with_landscape(mut self, commands: Vec<LandscapeCommand>) -> Self {
        self.landscape = commands;
        self
    }

    pub fn with_particles(mut self, particles: Vec<ParticleCommand>) -> Self {
        self.particles = particles;
        self
    }

    pub fn update(&self) -> &ObjectUpdate {
        &self.update
    }

    pub fn effects(&self) -> &[EffectCommand] {
        &self.effects
    }

    pub fn landscape(&self) -> &[LandscapeCommand] {
        &self.landscape
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CrewSelection {
    pub(crate) cursor: Option<ObjectId>,
}

impl CrewSelection {
    pub(crate) fn prune(&mut self, alive: &HashSet<ObjectId>) {
        if let Some(cursor) = self.cursor {
            if !alive.contains(&cursor) {
                self.cursor = None;
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.cursor.is_none()
    }

    pub(crate) fn cursor(&self) -> Option<ObjectId> {
        self.cursor
    }

    pub(crate) fn set_cursor(&mut self, cursor: Option<ObjectId>) {
        self.cursor = cursor;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrewSelectionState {
    #[serde(default)]
    pub selected: Vec<ObjectId>,
    #[serde(default)]
    pub cursor: Option<ObjectId>,
}

impl From<CrewSelectionState> for CrewSelection {
    fn from(state: CrewSelectionState) -> Self {
        Self {
            cursor: state.cursor,
        }
    }
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct Object {
    #[doc(hidden)]
    pub id: ObjectId,
    /// Runtime identity of this C4Object allocation. Object numbers can be
    /// reused by a section load, while a suspended callback still refers to
    /// the departing allocation. The engine assigns this transient token
    /// when the object enters its live list; it is deliberately absent from
    /// snapshots and savegame state.
    pub(crate) instance_token: u64,
    #[doc(hidden)]
    pub definition_id: DefinitionId,
    #[doc(hidden)]
    pub state: ObjectState,
    /// C4Object::Mass is a compiled cache, not a derived getter. Objects.txt
    /// restores it verbatim (including zero) until a native UpdateMass path
    /// runs. The contents snapshot lets runtime Enter/Exit invalidate a
    /// parent's cache without disturbing load-time link denumeration.
    pub(crate) compiled_mass: Option<i32>,
    pub(crate) compiled_mass_contents: Vec<ObjectId>,
    /// C4Object::Unsorted is a transient, non-savegame flag. ChangeDef sets
    /// it without requesting a sweep; a later C4Object::Resort request (or
    /// Objects.Synchronize) clears it while remove/re-adding the main-list
    /// link. C4ObjectList::Add also treats the flag as a sort override for
    /// contents links.
    #[doc(hidden)]
    pub unsorted: bool,
    /// Deferred contents-link key for a link established during ChangeDef's
    /// old-action callbacks. It is consumed by the engine-side list fold and
    /// is transient like `Unsorted`.
    pub(crate) change_def_contents_sort: Option<ChangeDefContentsSort>,
    #[doc(hidden)]
    pub fixed_position: FixedVec2,
    #[doc(hidden)]
    pub fixed_velocity: FixedVec2,
    /// C4Object::motion_x/motion_y: whole-pixel displacement accumulated by
    /// DoMotion during the most recent DoMovement invocation.
    #[doc(hidden)]
    pub motion_x: i32,
    #[doc(hidden)]
    pub motion_y: i32,
    /// Raw fields consumed by C4Object::CompileFunc rather than reconstructed
    /// from their associated live pointers. C4EnumeratedObjectPtr keeps its
    /// signed enumeration word independent of the resolved Object pointer.
    pub(crate) compiler_cache: ObjectCompilerCache,
    /// 16.16 fixed-point rotation accumulator (C++ `fix_r`, `C4Object.h:149`).
    /// `state.rotation` (whole degrees) is its `fixtoi` projection.
    #[doc(hidden)]
    pub fixed_rotation: C4Fixed,
    /// 16.16 fixed-point angular velocity in degrees/frame (C++ `rdir`,
    /// `C4Object.h:150`). Set by `SetRDir`; applied as `fix_r += rdir * 5` each
    /// frame (`C4Movement.cpp:376`).
    #[doc(hidden)]
    pub rotation_velocity: C4Fixed,
    #[doc(hidden)]
    pub destroyed: bool,
    /// Last `Info->Physical` storage after AssignRemoval clears the object's
    /// Info pointer. A native caller may still hold the original C++ pointer
    /// for the remainder of the stack frame (ObjectComDigDouble does).
    pub(crate) retired_info_physical: Option<PhysicalInfo>,
    /// The baked solid mask (grid worlds only; C4Object::pSolidMaskData).
    #[doc(hidden)]
    pub solid_mask_bake: Option<SolidMaskBake>,
    /// C4SolidMask::MaskPut for an eligible mask whose landscape-clipped
    /// rectangle is empty. Such a put has no raster bake, but it must remain
    /// logically put so movement can restore riders and a later Remove can
    /// clear the lifecycle state (C4SolidMask.cpp:75-79,176-195,231-262).
    pub(crate) solid_mask_empty_put: bool,
    /// Construction order of the live C4SolidMask instance. This survives
    /// ordinary Remove/Put cycles (including fully off-landscape puts) and is
    /// cleared only when C++ would delete pSolidMaskData.
    pub(crate) solid_mask_instance_sequence: Option<u64>,
    /// This frame's latched Action.t_attach (C4Object.cpp:4692 + the
    /// per-procedure assignments): ExecAction computes it BEFORE the
    /// phase-wrap SetAction — the movement that follows runs with the
    /// OLD action's attach (the wrapping HeadUp bison free-falls its
    /// wrap frame instead of snapping to a floor 5px down).
    #[doc(hidden)]
    pub frame_t_attach: u32,
    /// C4Object::t_contact latched by the most recent ContactCheck. Command
    /// execution precedes this frame's movement and therefore reads the
    /// previous movement frame's value (C4Movement.cpp:166-182,470).
    #[doc(hidden)]
    pub frame_t_contact: u32,
    /// Live C4Shape::VtxContactCNAT aligned with the current Shape vertices.
    /// Collision response reads this again after synchronous Contact* calls.
    pub(crate) frame_vertex_contacts: Vec<u32>,
    /// Live C4Shape::ContactCNAT/ContactCount. Unlike `frame_t_contact`, these
    /// may be replaced by a shape refresh inside a synchronous Contact*
    /// callback before ContactCheck returns.
    pub(crate) frame_shape_contact_cnat: u32,
    pub(crate) frame_shape_contact_count: i32,
    /// This frame's UprightAttach bits (C4Object.cpp:4698-4705): the
    /// per-frame `Action.t_attach |= Def->UprightAttach` OR that feeds the
    /// movement config. Transient — recomputed at every ExecAction, never
    /// serialized.
    pub(crate) upright_t_attach: u32,
    /// C4Object::iLastAttachMovementFrame: prevents two moving masks from
    /// carrying the same object twice in one frame (C4SolidMask.cpp:187-193).
    pub(crate) last_attach_movement_frame: i32,
    /// Last energy-loss causing player (C4Object::LastEnergyLossCausePlayer,
    /// read by AssignDeath for kill attribution).
    #[doc(hidden)]
    pub last_energy_loss_cause: i32,
    /// Transient C4Object::InMat cache. SetOCF/UpdateOCF sample it; ExecLife
    /// deliberately reads that cached, normally pre-movement material.
    pub(crate) in_mat: Option<MaterialId>,
    pub(crate) command_queue: VecDeque<QueuedCommand>,
    #[doc(hidden)]
    pub commands: CommandStack,
    pub(crate) pending_action_events: VecDeque<ActionTransitionEvent>,
    /// Ordered action-slot changes awaiting client-local ActMap-sound
    /// reconciliation. This is runtime presentation state, not synchronized
    /// or save-persisted object state.
    pub(crate) pending_action_sound_events: VecDeque<ActionSoundTransition>,
    /// ActMap `Sound=` most recently attempted for this object. Rejected starts
    /// are remembered too: C++ does not retry `NewInstance` later merely
    /// because mixer state changed.
    pub(crate) active_action_sound: Option<String>,
    /// Whether creation or a SetAction transition has made the initial
    /// ActMap-sound decision. `None` cannot encode this because an action may
    /// legitimately have no Sound=.
    pub(crate) action_sound_initialized: bool,
    /// The DFA_SWIM free-fall exit `return`ed out of ExecAction this
    /// frame BEFORE any t_attach assignment — the frame's movement runs
    /// unattached (C4Object.cpp:4692,4956).
    pub(crate) swim_exit_this_frame: bool,
    pub(crate) material_contents: Vec<i32>,
    pub(crate) shape_template: ObjectShapeTemplate,
    /// Exact live C4Shape rectangle. Con/r may change without UpdateShape,
    /// so this cannot be reconstructed from the current scalar fields.
    pub(crate) shape_rect: Option<DefinitionRect>,
    /// Live C4Shape::FireTop. Unlike the definition template this survives
    /// SetShape and loaded-object shape data until UpdateShape rebuilds the
    /// complete shape from DefCore (C4Shape.cpp:495-510).
    pub(crate) shape_fire_top: i32,
    #[doc(hidden)]
    pub own_shape_vertices: Option<Vec<ObjectVertex>>,
}

/// Private C4Object compiler caches exposed verbatim by GetObjectVal. Pointer
/// assignment and pointer denumeration do not refresh these words; an object
/// enumeration pass does. Keeping them independent is observable for freshly
/// assigned, stale, legacy-offset, and unresolved references.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ObjectCompilerCache {
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub contained: i32,
    #[serde(default)]
    pub action_target1: i32,
    #[serde(default)]
    pub action_target2: i32,
    #[serde(default)]
    pub layer: i32,
}

impl ObjectCompilerCache {
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Stable position of C4Effect::Execute's live-list cursor. Effect numbers
/// identify the current node even when callbacks insert into the prefix; the
/// priority is only a fallback for legacy command paths that still unlink the
/// current node immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EffectFrameCursor {
    pub(crate) number: i32,
    pub(crate) priority: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionTransitionKind {
    Natural,
    Forced,
}

#[derive(Debug, Clone)]
pub(crate) struct ActionTransitionEvent {
    pub(crate) previous_action: ActionState,
    pub(crate) kind: ActionTransitionKind,
}

#[derive(Debug, Clone)]
pub(crate) struct ActionSoundTransition {
    /// Concrete old-slot name captured before incomplete construction can
    /// coerce the requested slot (C4Object.cpp:4121-4130).
    pub(crate) stop: Option<String>,
    /// Concrete final-slot name captured after coercion
    /// (C4Object.cpp:4159-4163).
    pub(crate) start: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ContainerUpdateRecord {
    pub(crate) object_id: ObjectId,
    pub(crate) previous: Option<ObjectId>,
    pub(crate) new: Option<ObjectId>,
    pub(crate) host_executed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ObjectShapeTemplate {
    vertices: Vec<ObjectVertex>,
    pub(crate) rect: Option<DefinitionRect>,
    fire_top: i32,
    pub(crate) stretch_growth: bool,
    pub(crate) rotateable: i32,
    /// DefCore Line type: Line objects skip every shape/vertex refresh
    /// (C4Object::UpdateShape early return, C4Object.cpp:322-324) — the
    /// CONNECT exec owns their vertices.
    pub(crate) line: i32,
}

impl ObjectShapeTemplate {
    pub(crate) fn new(
        vertices: Vec<ObjectVertex>,
        rect: Option<DefinitionRect>,
        fire_top: i32,
        stretch_growth: bool,
        rotateable: i32,
    ) -> Self {
        Self {
            vertices,
            rect,
            fire_top,
            stretch_growth,
            rotateable,
            line: 0,
        }
    }

    pub(crate) fn with_line(mut self, line: i32) -> Self {
        self.line = line;
        self
    }
}

#[derive(Debug, Default)]
pub(crate) struct CommandQueueOutcome {
    pub(crate) spawns: Vec<SpawnConfig>,
    pub(crate) destroy: bool,
    pub(crate) effect_events: Vec<EffectEvent>,
    pub(crate) container_updates: Vec<ContainerUpdateRecord>,
    pub(crate) command_events: Vec<CommandEvent>,
    particles: Vec<ParticleCommand>,
    pub(crate) definition_changed: bool,
    pub(crate) change_def_reinsert: bool,
}

/// Record a fixed-point vector in a snapshot only when it carries sub-pixel
/// detail beyond its whole-pixel projection — i.e. when `fixtoi(fixed)` would
/// not round-trip back to `fixed`. Returns `None` for whole-pixel values so the
/// snapshot stays minimal and reconstructs losslessly via `itofix(pixels)`.
fn subpixel_or_none(fixed: FixedVec2, pixels: Vector2) -> Option<FixedVec2> {
    if fixed == FixedVec2::from_ints(pixels.x, pixels.y) {
        None
    } else {
        Some(fixed)
    }
}

impl Object {
    /// C++ APIs that explicitly test Status accept every nonzero value,
    /// including C4OS_INACTIVE. Raw pointers may remain dereferenceable
    /// after Status reaches zero; callers must not use this as a pointer gate.
    pub(crate) fn has_nonzero_status(&self) -> bool {
        !self.destroyed && self.state.status != ObjectStatus::Deleted
    }

    pub(crate) fn new(
        id: ObjectId,
        definition_id: DefinitionId,
        state: ObjectState,
        shape_template: ObjectShapeTemplate,
        own_shape_vertices: Option<Vec<ObjectVertex>>,
    ) -> Self {
        let frame_vertex_contacts = vec![0; state.vertices.len()];
        let fixed_position = FixedVec2::from_ints(state.position.x, state.position.y);
        let fixed_velocity = FixedVec2::from_ints(state.velocity.x, state.velocity.y);
        let fixed_rotation = itofix(state.rotation);
        let shape_fire_top = scaled_shape_fire_top(
            shape_template.fire_top,
            state.construction,
            shape_template.line,
        );
        let shape_rect = if shape_template.line != 0 {
            shape_template.rect
        } else {
            transformed_shape_rect(
                shape_template.rect,
                state.construction,
                shape_template.stretch_growth,
                shape_template.rotateable,
                state.rotation,
            )
        };
        Self {
            id,
            instance_token: 0,
            definition_id,
            compiled_mass: None,
            compiled_mass_contents: Vec::new(),
            unsorted: false,
            change_def_contents_sort: None,
            fixed_position,
            fixed_velocity,
            motion_x: 0,
            motion_y: 0,
            compiler_cache: ObjectCompilerCache::default(),
            fixed_rotation,
            rotation_velocity: C4Fixed::ZERO,
            destroyed: matches!(state.status, ObjectStatus::Deleted),
            retired_info_physical: None,
            frame_t_attach: 0,
            frame_t_contact: 0,
            frame_vertex_contacts,
            frame_shape_contact_cnat: 0,
            frame_shape_contact_count: 0,
            solid_mask_bake: None,
            solid_mask_empty_put: false,
            solid_mask_instance_sequence: None,
            state,
            upright_t_attach: 0,
            last_attach_movement_frame: -1,
            last_energy_loss_cause: OWNER_NONE,
            in_mat: None,
            command_queue: VecDeque::new(),
            commands: CommandStack::new(),
            pending_action_events: VecDeque::new(),
            pending_action_sound_events: VecDeque::new(),
            active_action_sound: None,
            action_sound_initialized: false,
            swim_exit_this_frame: false,
            material_contents: Vec::new(),
            shape_template,
            shape_rect,
            shape_fire_top,
            own_shape_vertices,
        }
    }

    fn fixed_vec_to_pixels(value: FixedVec2) -> Vector2 {
        Vector2::new(value.int_x(), value.int_y())
    }

    pub(crate) fn position_pixels(&self) -> Vector2 {
        Self::fixed_vec_to_pixels(self.fixed_position)
    }

    pub(crate) fn velocity_pixels(&self) -> Vector2 {
        Self::fixed_vec_to_pixels(self.fixed_velocity)
    }

    #[doc(hidden)]
    pub fn set_position(&mut self, position: Vector2) {
        self.state.position = position;
        self.fixed_position = FixedVec2::from_ints(position.x, position.y);
    }

    pub(crate) fn set_velocity(&mut self, velocity: Vector2) {
        self.state.velocity = velocity;
        self.fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
    }

    pub(crate) fn refresh_velocity_from_fixed(&mut self) {
        self.state.velocity = self.velocity_pixels();
    }

    fn shape_base_vertices(&self) -> &[ObjectVertex] {
        self.own_shape_vertices
            .as_deref()
            .unwrap_or(&self.shape_template.vertices)
    }

    pub(crate) fn unrotated_shape_vertices(&self) -> Vec<ObjectVertex> {
        if self.shape_template.line != 0 {
            return self.state.shape_vertices.active_vec();
        }
        transformed_shape_vertices(
            self.shape_base_vertices(),
            self.state.construction,
            self.shape_template.stretch_growth,
            0,
            0,
        )
    }

    #[doc(hidden)]
    pub fn current_shape_rect(&self) -> Option<DefinitionRect> {
        self.state.shape_override.or(self.shape_rect)
    }

    fn definition_derived_shape_rect(&self) -> Option<DefinitionRect> {
        if self.shape_template.line != 0 {
            return self.shape_template.rect;
        }
        transformed_shape_rect(
            self.shape_template.rect,
            self.state.construction,
            self.shape_template.stretch_growth,
            self.shape_template.rotateable,
            self.state.rotation,
        )
    }

    fn definition_derived_fire_top(&self) -> i32 {
        scaled_shape_fire_top(
            self.shape_template.fire_top,
            self.state.construction,
            self.shape_template.line,
        )
    }

    pub(crate) fn refresh_shape_after_state_change(
        &mut self,
        previous_construction: i32,
        previous_rect: Option<DefinitionRect>,
        preserve_bottom: bool,
    ) {
        self.refresh_shape_geometry();
        if self.shape_template.line != 0 {
            return;
        }

        let step_size = FULL_CON / 100;
        let previous_step = previous_construction / step_size;
        let current_step = self.state.construction / step_size;
        let step_diff = current_step - previous_step;
        let current_rect = self.current_shape_rect();
        let adjusted_y = if preserve_bottom {
            docon_adjusted_position_y(
                self.state.position.y,
                previous_rect,
                self.state.position.y,
                current_rect,
                self.state.rotation,
                self.state.category,
                previous_step,
                step_diff,
                self.shape_template.rect.map_or(0, |rect| rect.height),
            )
        } else {
            self.state.position.y
        };
        if adjusted_y != self.state.position.y {
            self.set_position(Vector2::new(self.state.position.x, adjusted_y));
        }
    }

    pub(crate) fn refresh_shape_from_template(
        &mut self,
        template: ObjectShapeTemplate,
        previous_construction: i32,
        previous_rect: Option<DefinitionRect>,
    ) {
        self.shape_template = template;
        self.refresh_shape_after_state_change(previous_construction, previous_rect, false);
    }

    pub(crate) fn refresh_shape_geometry(&mut self) {
        if self.shape_template.line != 0 {
            // Line shape independent (C4Object.cpp:322-324).
            return;
        }
        // C4Shape::CopyFrom replaces AttachMat from the definition shape
        // while deliberately retaining iAttachX/Y/Vtx
        // (C4Shape.cpp:421-443). Definition shapes start unattached.
        self.state.shape_attach.mat_valid = false;
        self.state.shape_attach.mat_vehicle = false;
        self.state.shape_override = None;
        self.shape_rect = self.definition_derived_shape_rect();
        self.shape_fire_top = self.definition_derived_fire_top();
        let vertices = transformed_shape_vertices(
            self.shape_base_vertices(),
            self.state.construction,
            self.shape_template.stretch_growth,
            self.shape_template.rotateable,
            self.state.rotation,
        );
        self.state.shape_vertices.replace_active(&vertices);
        self.state.vertices = vertices;
        self.frame_vertex_contacts = vec![0; self.state.vertices.len()];
        self.frame_shape_contact_cnat = CNAT_NONE;
        self.frame_shape_contact_count = 0;
    }

    pub(crate) fn latch_shape_contact(&mut self, contact: &ShapeContact) {
        self.frame_t_contact = contact.contact_cnat;
        self.frame_shape_contact_cnat = contact.contact_cnat;
        self.frame_shape_contact_count = contact.count();
        self.frame_vertex_contacts
            .resize(self.state.vertices.len(), 0);
        for index in 0..self.state.vertices.len() {
            // Native CheckContact skips CNAT_NoCollision vertices without
            // touching their retained VtxContactCNAT slot.
            if self.state.vertices[index].cnat & CNAT_NO_COLLISION == 0 {
                self.frame_vertex_contacts[index] =
                    contact.vertex_contacts.get(index).copied().unwrap_or(0);
            }
        }
    }

    pub(crate) fn movement_attach(&self) -> u32 {
        if self.swim_exit_this_frame {
            CNAT_NONE
        } else {
            self.frame_t_attach
        }
    }

    pub(crate) fn live_contact_has_vertex_cnat(&self, cnat: u32) -> bool {
        self.frame_vertex_contacts
            .iter()
            .take(self.state.vertices.len())
            .any(|contact| contact & cnat != 0)
    }

    pub(crate) fn live_contact_first_friction(&self) -> i32 {
        self.state
            .vertices
            .iter()
            .zip(&self.frame_vertex_contacts)
            .find_map(|(vertex, &contact)| (contact != 0).then_some(vertex.friction))
            .unwrap_or(0)
    }

    pub(crate) fn live_contact_first_weight(&self) -> i32 {
        self.state
            .vertices
            .iter()
            .zip(&self.frame_vertex_contacts)
            .filter_map(|(vertex, &contact)| (contact != 0).then_some(vertex.x.signum()))
            .find(|&weight| weight != 0)
            .unwrap_or(0)
    }

    pub(crate) fn set_owned_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.own_shape_vertices = Some(vertices);
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.refresh_shape_after_state_change(previous_construction, previous_rect, false);
    }

    pub(crate) fn set_live_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.state.shape_vertices.replace_active(&vertices);
        self.state.vertices = vertices;
        self.frame_vertex_contacts
            .resize(self.state.vertices.len(), 0);
    }

    pub(crate) fn set_shape_vertex_buffer(&mut self, vertices: ShapeVertexBuffer) {
        self.state.vertices = vertices.active_vec();
        self.state.shape_vertices = vertices;
        self.frame_vertex_contacts
            .resize(self.state.vertices.len(), 0);
    }

    pub(crate) fn set_construction(&mut self, construction: i32) {
        self.compiled_mass = None;
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.state.construction = construction.clamp(0, FULL_CON);
        self.refresh_shape_after_state_change(previous_construction, previous_rect, true);
    }

    pub(crate) fn set_construction_from_docon(&mut self, construction: i32) {
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.state.construction = construction.max(0);
        if !docon_refreshes_construction(previous_construction, self.state.construction) {
            return;
        }
        self.compiled_mass = None;
        self.refresh_shape_after_state_change(previous_construction, previous_rect, true);
    }

    pub(crate) fn remember_compiled_mass_contents(&mut self) {
        if self.compiled_mass.is_some() {
            self.compiled_mass_contents = self.state.contents.clone();
        }
    }

    #[doc(hidden)]
    pub fn set_fixed_velocity(&mut self, velocity: FixedVec2) {
        self.fixed_velocity = velocity;
        self.state.velocity = self.velocity_pixels();
    }

    /// Script/host-call snapshot: carries the TRUE sub-pixel dirs so the
    /// scope's GetXDir/GetYDir read the live C4Fixed values like C++
    /// (C4Script.cpp:1160-1180) instead of the int-quantized mirror.
    pub(crate) fn script_state_snapshot(&self) -> ObjectState {
        #[cfg(test)]
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let mut state = self.state.clone();
        // A handful of engine-internal temporary shape probes replace the
        // active Vec directly before dispatching callbacks. Preserve the raw
        // dormant tail, but make the script-visible prefix match that exact
        // live shape at call time.
        state.shape_vertices.replace_active(&state.vertices);
        state.script_fixed_position = Some(self.fixed_position);
        state.script_fixed_velocity = Some(self.fixed_velocity);
        state.script_rotation_velocity = Some(self.rotation_velocity);
        state.script_fixed_rotation = Some(self.fixed_rotation);
        state.shape_override = self.current_shape_rect();
        state
    }

    pub(crate) fn clamp_velocity(&mut self, physics: &PhysicsSettings) {
        physics.clamp_fixed_velocity(&mut self.fixed_velocity);
        self.refresh_velocity_from_fixed();
    }

    pub(crate) fn apply_delta(
        &mut self,
        delta: &ObjectDelta,
        action_library: &ActionLibrary,
    ) -> ApplyDeltaOutcome {
        if delta.change_def.is_some() {
            self.change_def_contents_sort = delta.change_def_contents_sort.clone();
        }
        if let Some(sequence) = delta.solid_mask_instance_sequence {
            self.solid_mask_instance_sequence = Some(sequence);
        } else if delta.change_def.is_some()
            || delta.solid_mask_override.is_some()
            || delta
                .base_graphics
                .as_ref()
                .is_some_and(|graphics| self.state.base_graphics.as_ref() != graphics.as_ref())
        {
            // ChangeDef, SetSolidMask and a real SetGraphics change delete
            // pSolidMaskData before UpdateSolidMask constructs a new tail
            // instance (C4Object.cpp:382-400,1207-1244,3809-3817).
            self.solid_mask_instance_sequence = None;
        }
        if let Some(compiler_cache) = &delta.compiler_cache {
            self.compiler_cache = compiler_cache.clone();
        }
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        let construction_refreshes_shape = delta.construction.is_some_and(|construction| {
            !delta.construction_via_docon
                || docon_refreshes_construction(previous_construction, construction.max(0))
        });
        if delta.own_mass.is_some() || delta.change_def.is_some() || construction_refreshes_shape {
            self.compiled_mass = None;
        }
        let shape_changed = construction_refreshes_shape
            || delta.rotation.is_some()
            || (delta.vertices.is_some() && !delta.vertices_defer_shape_update);
        // Kill-trace mark BEFORE the energy write (C4Object.cpp:1351-1361)
        // so AssignDeath credits the new cause.
        if let Some(cause) = delta.energy_loss_cause {
            self.last_energy_loss_cause = cause;
        }
        let outcome = self.state.apply_delta(delta, action_library);
        if outcome.action_change.is_some()
            && delta
                .action
                .as_ref()
                .is_some_and(|action| action.action_sound_dispatched)
        {
            // The host-call seam already ran SetAction's StopSoundEffect /
            // StartSoundEffect pair. Remember the selected slot even when its
            // NewInstance was rejected: C++ does not retry that start at the
            // frame boundary, and a retry could consume wildcard RNG or become
            // spuriously successful after mixer state changes.
            self.pending_action_sound_events.clear();
            if let Some(selection) = delta
                .action
                .as_ref()
                .and_then(|action| action.action_sound_selection.clone())
            {
                self.active_action_sound = selection;
            }
            self.action_sound_initialized = true;
        }
        if let Some(position) = delta.position {
            self.fixed_position = FixedVec2::from_ints(position.x, position.y);
        }
        // A DoCon integer-y result is part of the live state seen by a later
        // merged SetAction. Install it before SetAction resynchronizes the
        // fixed coordinates; an explicit same-DoCon fixed result still wins
        // below for the inverse (SetAction-before-bottom-adjust) ordering.
        if let Some(position) = delta.resolved_docon_position {
            self.state.position = position;
        }
        // C4Object::SetAction resyncs the fixed coords to the integer
        // position once past its early returns (C4Object.cpp:4144).
        if outcome
            .action_change
            .as_ref()
            .map(|change| change.requested_name_change)
            .unwrap_or(false)
        {
            self.fixed_position =
                FixedVec2::from_ints(self.state.position.x, self.state.position.y);
        }
        if let Some(position) = delta.resolved_docon_fixed_position {
            self.fixed_position = position;
        }
        // No reprojection from the fixed coords otherwise: C++ x/y only
        // change via explicit assignment or movement — DoCon's initial
        // adjust legitimately leaves y and fixtoi(fix_y) split.
        if let Some(fixed_velocity) = delta.fixed_velocity {
            // Sub-pixel velocity is authoritative; derive the whole-pixel mirror
            // from it (matches C++ where xdir/ydir as C4Fixed are the source of
            // truth and the integer view is `fixtoi`).
            self.fixed_velocity = fixed_velocity;
            self.state.velocity = self.velocity_pixels();
        } else if let Some(velocity) = delta.velocity {
            self.fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
        } else {
            self.state.velocity = self.velocity_pixels();
        }
        // Component dir writes land on the TRUE fixed velocity — the
        // untouched component keeps its sub-pixel value (FnSetXDir only
        // assigns xdir, C4Script.cpp:697-705).
        if let Some(x) = delta.fixed_velocity_x {
            self.fixed_velocity.x = x;
            self.state.velocity = self.velocity_pixels();
        }
        if let Some(y) = delta.fixed_velocity_y {
            self.fixed_velocity.y = y;
            self.state.velocity = self.velocity_pixels();
        }

        // Script dir writes mobilize unconditionally: FnSetXDir/FnSetYDir/
        // FnSetRDir all end in `pObj->Mobile = 1` (C4Script.cpp:705,718,732).
        if delta.fixed_velocity.is_some()
            || delta.fixed_velocity_x.is_some()
            || delta.fixed_velocity_y.is_some()
            || delta.velocity.is_some()
            || delta.rotation_velocity.is_some()
        {
            self.state.mobile = true;
        }
        // Native object helpers can assign velocity without sharing the
        // script Set*Dir mobilization rule. Their explicit result wins over
        // the generic rule above (C4Object::Fling's Tumble branch preserves
        // Mobile, while Jump/raw fallback set it).
        if let Some(mobile) = delta.mobile {
            self.state.mobile = mobile;
        }
        if let Some(t_attach) = delta.t_attach {
            // Movement consumes the already-latched frame value after an
            // action callback; keep the serializable mirror in sync too.
            self.state.t_attach = t_attach;
            self.frame_t_attach = t_attach;
        }
        if delta.rotation.is_some() {
            // An explicit rotation re-seeds the fixed accumulator, mirroring C++
            // forcing `fix_r = itofix(r)` (`C4Movement.cpp:418`).
            self.fixed_rotation = itofix(self.state.rotation);
        }
        if let Some(rotation_velocity) = delta.rotation_velocity_raw {
            self.rotation_velocity = rotation_velocity;
        }
        if let Some(rotation_velocity) = delta.rotation_velocity {
            self.rotation_velocity = rotation_velocity;
        }
        if let Some(vertices) = &delta.vertices {
            self.own_shape_vertices = Some(vertices.clone());
        }
        if shape_changed {
            let fixed_position = self.fixed_position;
            self.refresh_shape_after_state_change(
                previous_construction,
                previous_rect,
                delta.construction.is_some() && delta.resolved_docon_position.is_none(),
            );
            if delta.construction_preserves_fixed_position {
                // DoCon's straight-con bottom adjustment calls UpdatePos,
                // which updates sectors but never fix_x/fix_y
                // (C4Object.cpp:1462-1495, C4Object::UpdatePos at :346-354).
                self.fixed_position = fixed_position;
            }
        }
        if let Some(shape_override) = delta.shape_override {
            self.state.shape_override = shape_override;
            match shape_override {
                Some(rect) => self.shape_rect = Some(rect),
                None if !shape_changed => self.refresh_shape_geometry(),
                None => {}
            }
        }
        if let Some(material_contents) = &delta.material_contents {
            self.material_contents = material_contents.clone();
        }
        if let Some(vertices) = &delta.live_vertices {
            self.set_live_shape_vertices(vertices.clone());
        }
        if let Some(vertices) = &delta.shape_vertices {
            self.set_shape_vertex_buffer(vertices.clone());
        }
        outcome
    }

    pub(crate) fn advance_fixed_position(&mut self) {
        self.fixed_position += self.fixed_velocity;
        self.state.position = self.position_pixels();
        self.state.velocity = self.velocity_pixels();
    }

    pub(crate) fn apply_command_operations<I>(&mut self, operations: I)
    where
        I: IntoIterator<Item = CommandOperation>,
    {
        for operation in operations {
            match operation {
                // C4Object::ClearCommands removes the complete native
                // command chain. The compatibility queue contains deferred
                // continuations for that same chain, so entries already
                // present at this chronological Clear must disappear too.
                CommandOperation::Clear => {
                    self.commands.clear();
                    self.command_queue.clear();
                }
                CommandOperation::PushFront(request) => {
                    let _ = self.commands.push_front(request);
                }
                CommandOperation::PushBack(request) => {
                    let _ = self.commands.push_back(request);
                }
                CommandOperation::Finish { index, success } => {
                    self.commands.finish_entry_public(index, success);
                }
                CommandOperation::DecrementNoCollectDelay => {
                    // C4Object::SetCommand entry (C4Object.cpp:3941-3942).
                    if self.state.no_collect_delay > 0 {
                        self.state.no_collect_delay -= 1;
                    }
                }
                CommandOperation::SetNoCollectDelay { value, ocf } => {
                    self.state.no_collect_delay = value;
                    self.state.ocf = ocf;
                }
                CommandOperation::Restore(snapshot) => {
                    self.commands.restore_from_snapshot(&snapshot);
                }
            }
        }
    }

    pub(crate) fn step_command_stack(
        &mut self,
        ctx: CommandRuntimeContext<'_>,
        gravity: C4Fixed,
    ) -> Option<CommandStepResult> {
        self.commands.execute_front_with_gravity(&ctx, gravity)
    }

    #[doc(hidden)]
    pub fn mark_destroyed(&mut self) -> Vec<EffectEvent> {
        if self.destroyed {
            return Vec::new();
        }
        self.destroyed = true;
        self.state.status = ObjectStatus::Deleted;
        self.drain_effects_with_reason(EffectStopReason::Destroyed)
    }

    pub(crate) fn snapshot(&self, library: Option<&ActionLibrary>) -> ObjectSnapshot {
        let procedure = library
            .and_then(|lib| {
                lib.procedure_name_for_entry(
                    &self.state.action.name,
                    self.state.action.act_map_index,
                )
            })
            .map(|name| name.to_string());
        // The INT position is the sim-state x/y (C++ exports object->x/y);
        // it may legitimately differ from fixtoi(fix) after the DoCon
        // initial split — the fixed coords travel separately below.
        let position = self.state.position;
        let velocity = self.velocity_pixels();
        let rotation_velocity = self
            .rotation_velocity
            .is_nonzero()
            .then_some(self.rotation_velocity);
        // C++ always persists fix_r. Omit it only when reconstruction from
        // raw `r` is lossless; a stopped object may still retain a fractional
        // accumulator from its last rotation step.
        let fixed_rotation =
            (self.fixed_rotation != itofix(self.state.rotation)).then_some(self.fixed_rotation);
        let current_shape = self.current_shape_rect();
        let current_shape = (current_shape != self.definition_derived_shape_rect())
            .then_some(current_shape)
            .flatten();
        let current_fire_top = (self.shape_fire_top != self.definition_derived_fire_top())
            .then_some(self.shape_fire_top);
        ObjectSnapshot {
            id: self.id,
            definition_id: self.definition_id.clone(),
            custom_name: self.state.custom_name.clone(),
            position,
            velocity,
            // C++ persists `r` verbatim; DoMovement keeps active rotation in
            // (-180, 180], so a left lean remains negative across a save.
            rotation: self.state.rotation,
            energy: self.state.energy,
            need_energy: self.state.need_energy,
            damage: self.state.damage,
            magic_energy: self.state.magic_energy,
            magic_capacity: self.state.magic_capacity,
            construction: self.state.construction,
            action: self.state.action.clone(),
            direction: self.state.direction,
            command_direction: self.state.command_direction,
            action_procedure: procedure,
            effects: self.state.effects.clone(),
            vertices: self.state.vertices.clone(),
            current_shape,
            current_fire_top,
            contact_density: self.state.contact_density,
            own_vertices: self.own_shape_vertices.clone(),
            vertex_contacts: self.frame_vertex_contacts.clone(),
            solid_mask_override: self.state.solid_mask_override,
            container: self.state.container,
            layer: self.state.layer,
            visibility: self.state.visibility,
            blit_mode: self.state.blit_mode,
            color: self.state.color,
            color_modulation: self.state.color_modulation,
            picture_rect: self.state.picture_rect,
            contents: self.state.contents.clone(),
            components: self.state.components.clone(),
            component_order: self.state.component_order.clone(),
            status: self.state.status,
            owner: self.state.owner,
            base: self.state.base,
            controller: self.state.controller,
            category: self.state.category,
            crew_member: self.state.crew_member,
            plr_view_range: self.state.plr_view_range,
            selected: self.state.selected,
            alive: self.state.alive,
            base_graphics: self.state.base_graphics.clone(),
            graphics_overlays: self.state.graphics_overlays.clone(),
            draw_transform: self.state.draw_transform,
            command_queue: self.command_queue.iter().cloned().collect(),
            command_stack: self.commands.snapshot(),
            local_vars: self.state.local_vars.snapshot(),
            in_liquid: self.state.in_liquid,
            mobile: self.state.mobile,
            ocf: self.state.ocf,
            timer: self.state.timer,
            own_mass: self.state.own_mass,
            on_fire: self.state.on_fire,
            fire_phase: self.state.fire_phase,
            fire_caused_by: self.state.fire_caused_by,
            info_physical: self.state.info_physical,
            temporary_physical: self.state.temporary_physical,
            physical_changes: self.state.physical_changes.clone(),
            breath: self.state.breath,
            last_energy_loss_cause: self.last_energy_loss_cause,
            fixed_position: subpixel_or_none(self.fixed_position, position),
            fixed_velocity: subpixel_or_none(self.fixed_velocity, velocity),
            rotation_velocity,
            fixed_rotation,
        }
    }

    pub(crate) fn apply_effect_commands(&mut self, commands: &[EffectCommand]) -> Vec<EffectEvent> {
        let mut events = Vec::new();
        for command in commands {
            match command {
                EffectCommand::Add {
                    effect,
                    constructor_values,
                } => {
                    let is_update = effect.number > 0
                        && self
                            .state
                            .effects
                            .iter()
                            .any(|existing| existing.number == effect.number);
                    let (inserted, _) = self.insert_effect(effect.clone());
                    if !is_update {
                        events.push(EffectEvent::started(
                            inserted,
                            constructor_values
                                .clone()
                                .unwrap_or_else(|| std::array::from_fn(|_| Value::Nil)),
                        ));
                    }
                }
                EffectCommand::Update(effect) => {
                    if let Some(existing) = self
                        .state
                        .effects
                        .iter_mut()
                        .find(|existing| existing.number == effect.number)
                    {
                        *existing = effect.clone();
                    }
                }
                EffectCommand::Remove { name, no_callbacks } => {
                    if let Some(removed) = self.mark_effect_dead(name) {
                        if !no_callbacks {
                            events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
                        }
                    }
                }
                EffectCommand::RemoveNumber {
                    number,
                    no_callbacks,
                } => {
                    if let Some(removed) = self.mark_effect_dead_by_number(*number) {
                        if !no_callbacks {
                            events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
                        }
                    }
                }
                EffectCommand::UnlinkNumber { number } => {
                    self.remove_effect_by_number(*number);
                }
                EffectCommand::Clear => {
                    events.extend(self.drain_effects_with_reason(EffectStopReason::Cleared));
                }
            }
        }
        events
    }

    pub(crate) fn ensure_material_capacity(&mut self, count: usize) {
        if self.material_contents.len() < count {
            self.material_contents.resize(count, 0);
        }
    }

    pub(crate) fn material_content(&self, material: MaterialId) -> i32 {
        let index = material.index();
        self.material_contents.get(index).copied().unwrap_or(0)
    }

    pub(crate) fn set_material_content(&mut self, material: MaterialId, amount: i32) {
        let index = material.index();
        if self.material_contents.len() <= index {
            self.material_contents.resize(index + 1, 0);
        }
        self.material_contents[index] = amount.max(0);
    }

    pub(crate) fn add_material_content(&mut self, material: MaterialId, amount: i32) {
        if amount <= 0 {
            return;
        }
        let index = material.index();
        if self.material_contents.len() <= index {
            self.material_contents.resize(index + 1, 0);
        }
        let slot = &mut self.material_contents[index];
        *slot = slot.saturating_add(amount);
    }

    // iIntervall/iTime are stored verbatim (C4Effect.cpp:66-67) - a zero
    // interval means the timer never fires.
    /// C4Effect::New semantics: same-name effects COEXIST; each gets a
    /// per-object monotonic number (max existing + 1, C4Effect.cpp:76-78).
    /// A carried nonzero number matching an existing effect is an UPDATE
    /// (EffectVar writes fold back through the add command).
    pub(crate) fn insert_effect(
        &mut self,
        mut effect: EffectState,
    ) -> (EffectState, Option<EffectState>) {
        if effect.timer < 0 {
            effect.timer = 0;
        }
        if effect.number > 0 {
            if let Some(existing) = self
                .state
                .effects
                .iter_mut()
                .find(|existing| existing.number == effect.number)
            {
                *existing = effect.clone();
                return (effect, None);
            }
        }
        if effect.number == 0 {
            effect.number = self
                .state
                .effects
                .iter()
                .map(|existing| existing.number)
                .max()
                .unwrap_or(0)
                .saturating_add(1)
                .max(1);
        }
        // Equal-priority nodes are newest-first. A fresh max+1 number still
        // inserts at the front, while a carried number (for example a denied
        // Kill victim) returns to its original position among its peers.
        let mut insert_pos = 0;
        if effect.priority > 0 {
            let priority = effect.priority as u32;
            while insert_pos < self.state.effects.len() {
                let existing = &self.state.effects[insert_pos];
                let existing_priority = existing.priority.unsigned_abs();
                if existing_priority > priority
                    || (existing_priority == priority && existing.number < effect.number)
                {
                    break;
                }
                insert_pos += 1;
            }
        }
        let inserted = effect.clone();
        self.state.effects.insert(insert_pos, effect);
        (inserted, None)
    }

    fn mark_effect_dead(&mut self, name: &str) -> Option<EffectState> {
        let effect = self
            .state
            .effects
            .iter_mut()
            .find(|effect| effect.name == name && effect.priority != 0)?;
        let stopped = effect.clone();
        effect.priority = 0;
        Some(stopped)
    }

    fn mark_effect_dead_by_number(&mut self, number: i32) -> Option<EffectState> {
        let effect = self
            .state
            .effects
            .iter_mut()
            .find(|effect| effect.number == number && effect.priority != 0)?;
        let stopped = effect.clone();
        effect.priority = 0;
        Some(stopped)
    }

    /// Remove THE effect (by its C++ identity, iNumber — names may repeat).
    pub(crate) fn remove_effect_by_number(&mut self, number: i32) -> Option<EffectState> {
        self.state
            .effects
            .iter()
            .position(|existing| existing.number == number)
            .map(|index| self.state.effects.remove(index))
    }

    fn drain_effects_with_reason(&mut self, reason: EffectStopReason) -> Vec<EffectEvent> {
        self.state
            .effects
            .drain(..)
            // C4Effect::ClearAll recurses through pNext before stopping its
            // own node, so every whole-list clear runs tail-to-head.
            .rev()
            .map(|effect| EffectEvent::stopped(effect, reason))
            .collect()
    }

    pub(crate) fn advance_effect_frame_cursor(
        &mut self,
        cursor: Option<EffectFrameCursor>,
    ) -> Option<(EffectFrameCursor, Option<EffectEvent>)> {
        advance_effect_frame_cursor(&mut self.state.effects, cursor)
    }

    pub(crate) fn enqueue_commands<I>(&mut self, commands: I)
    where
        I: IntoIterator<Item = QueuedCommand>,
    {
        self.command_queue.extend(commands);
    }

    pub(crate) fn apply_status(&mut self, status: ObjectStatus) -> Vec<EffectEvent> {
        if self.state.status == status {
            return Vec::new();
        }

        self.state.status = status;
        match status {
            ObjectStatus::Deleted => self.mark_destroyed(),
            _ => {
                if status.is_active() {
                    self.destroyed = false;
                }
                Vec::new()
            }
        }
    }

    pub(crate) fn record_action_event(
        &mut self,
        previous: ActionState,
        kind: ActionTransitionKind,
        action_library: &ActionLibrary,
    ) {
        let current = self.state.action.clone();
        let changed =
            previous.name != current.name || previous.act_map_index != current.act_map_index;
        self.record_action_sound_transition(&previous, &current, action_library, changed);
        self.pending_action_events.push_back(ActionTransitionEvent {
            previous_action: previous,
            kind,
        });
    }

    pub(crate) fn record_action_event_with_sound_stop(
        &mut self,
        previous: ActionState,
        kind: ActionTransitionKind,
        action_library: &ActionLibrary,
        stop_previous: bool,
    ) {
        let current = self.state.action.clone();
        self.record_action_sound_transition(&previous, &current, action_library, stop_previous);
        self.pending_action_events.push_back(ActionTransitionEvent {
            previous_action: previous,
            kind,
        });
    }

    pub(crate) fn record_action_sound_transition(
        &mut self,
        previous: &ActionState,
        current: &ActionState,
        action_library: &ActionLibrary,
        stop_previous: bool,
    ) {
        let start_current =
            previous.name != current.name || previous.act_map_index != current.act_map_index;
        if !stop_previous && !start_current {
            return;
        }
        let sound_for = |action: &ActionState| {
            action_library
                .spec_for_state(action)
                .and_then(|spec| spec.sound.clone())
                .filter(|sound| !sound.is_empty())
        };
        self.pending_action_sound_events
            .push_back(ActionSoundTransition {
                stop: stop_previous.then(|| sound_for(previous)).flatten(),
                start: start_current.then(|| sound_for(current)).flatten(),
            });
    }

    pub(crate) fn execute_command_queue(
        &mut self,
        physics: &PhysicsSettings,
        materials: &MaterialSet,
        mut landscape: Option<&mut Landscape>,
        action_library: &ActionLibrary,
        definitions: &rustc_hash::FxHashMap<DefinitionId, Definition>,
        players: &HashMap<i32, Player>,
    ) -> CommandQueueOutcome {
        #[cfg(test)]
        if self.command_queue.is_empty() {
            EMPTY_COMMAND_QUEUE_EXECUTIONS.with(|count| count.set(count.get() + 1));
        }
        let mut outcome = CommandQueueOutcome::default();
        loop {
            let execute_now = match self.command_queue.front_mut() {
                Some(command) if command.delay == 0 => true,
                Some(command) => {
                    command.delay -= 1;
                    false
                }
                None => break,
            };

            if !execute_now {
                break;
            }

            let command = self.command_queue.pop_front().expect("front exists");
            let delta: ObjectDelta = command.update.into();
            if let Some(new_def) = delta.change_def.as_deref() {
                if let Some(definition) = definitions.get(new_def) {
                    let owner_color =
                        players
                            .get(&self.state.owner)
                            .and_then(Player::color)
                            .map(|color| {
                                u32::from(color.r) << 16
                                    | u32::from(color.g) << 8
                                    | u32::from(color.b)
                            });
                    Engine::apply_change_object_def_to_object(
                        self,
                        new_def,
                        definition,
                        materials.len(),
                        owner_color,
                    );
                    outcome.definition_changed = true;
                }
            }
            if delta.change_def.is_some() {
                outcome.change_def_reinsert = delta.change_def_reinsert;
            }
            let current_action_library = definitions
                .get(&self.definition_id)
                .map(Definition::action_library)
                .unwrap_or(action_library);
            let delta_outcome = self.apply_delta(&delta, current_action_library);
            let callbacks_dispatched = delta
                .action
                .as_ref()
                .map(|action| action.callbacks_dispatched)
                .unwrap_or(false);
            if let Some(change) = delta_outcome.action_change {
                if !callbacks_dispatched {
                    self.record_action_event(
                        change.previous,
                        ActionTransitionKind::Forced,
                        current_action_library,
                    );
                }
            }
            if let Some((previous, new)) = delta_outcome.container_change {
                outcome.container_updates.push(ContainerUpdateRecord {
                    object_id: self.id,
                    previous,
                    new,
                    host_executed: delta.host_container_change,
                });
            }
            let mut effect_events = self.apply_effect_commands(&command.effects);
            if let Some(status) = delta.status {
                let mut status_events = self.apply_status(status);
                if !status_events.is_empty() {
                    effect_events.append(&mut status_events);
                }
                if matches!(status, ObjectStatus::Deleted) {
                    outcome.destroy = true;
                }
            }
            self.clamp_velocity(physics);
            if command.destroy {
                if !matches!(self.state.status, ObjectStatus::Deleted) {
                    effect_events.extend(self.mark_destroyed());
                }
                outcome.destroy = true;
            }
            if !effect_events.is_empty() {
                outcome.effect_events.extend(effect_events);
            }
            if !command.spawns.is_empty() {
                outcome.spawns.extend(command.spawns);
            }
            if !command.events.is_empty() {
                outcome.command_events.extend(command.events);
            }
            if !command.particles.is_empty() {
                outcome.particles.extend(command.particles);
            }
            if let Some(landscape_ref) = &mut landscape {
                for op in command.landscape.iter() {
                    op.apply(landscape_ref);
                }
            }
            if outcome.destroy {
                self.command_queue.clear();
                break;
            }
        }
        outcome
    }
}

/// Converts a script error from an engine-initiated callback into a logged
/// no-op. C++ runs lifecycle/game calls with `fPassErrors=false`: the error
/// shows in the log, the call yields nil, and the game continues
/// (C4AulExec.cpp:1318-1342). Non-script engine errors stay fatal.
/// `C4RankSystem::RankByExperience` with the default curve
/// Experience(rank) = rank^1.5 * RankBase(=1000) (C4RankSystem.cpp:226-237).
fn fair_crew_rank(experience: i32, rank_base: i32) -> i32 {
    let mut rank = 0;
    loop {
        let next = ((rank + 1) as f64).powf(1.5) * f64::from(rank_base);
        if next as i32 <= experience {
            rank += 1;
        } else {
            return rank;
        }
    }
}

/// `C4RankSystem::Experience` for the game-global rank system initialized
/// with `RankBase=1000` (C4RankSystem.cpp:226-229; C4Game.cpp:3518-3524).
/// `C4Object::DoExperience` deliberately uses this system for the promotion
/// threshold even when the crew definition supplies custom rank names.
pub(crate) fn rank_experience(rank: i32, rank_base: i32) -> i32 {
    if rank < 0 {
        return 0;
    }
    ((rank as f64).powf(1.5) * f64::from(rank_base)) as i32
}

pub(crate) fn crew_rank_experience(rank: i32) -> i32 {
    rank_experience(rank, 1_000)
}

/// `C4ObjectInfoCore::UpdateCustomRanks`' finite-table projection. `None`
/// means the definition has no custom rank system and therefore stores the
/// zero tag; an exhausted custom table stores `EXP_NoPromotion` (-1).
pub(crate) fn custom_next_rank_info(
    rank_names: Option<&RankNameTable>,
    rank_base: Option<i32>,
    rank: i32,
) -> (String, i32) {
    let Some(rank_names) = rank_names else {
        return (String::new(), 0);
    };
    let Some(next_rank) = rank
        .checked_add(1)
        .and_then(|rank| usize::try_from(rank).ok())
    else {
        return (String::new(), -1);
    };
    let Some(next_name) = rank_names.get(next_rank).filter(|name| !name.is_empty()) else {
        return (String::new(), -1);
    };
    let rank_base = rank_base.filter(|base| *base != 0).unwrap_or(1_000);
    let experience = ((next_rank as f64).powf(1.5) * f64::from(rank_base)) as i32;
    (next_name.into_owned(), experience)
}

/// Refresh the current custom rank name and stored next-rank fields. Callers
/// deliberately invoke this only at C++'s creation/save seams, never on load
/// or promotion.
pub(crate) fn update_custom_rank_fields(
    rank_name: &mut String,
    core: &mut CrewInfoCoreFields,
    rank: i32,
    rank_names: Option<&RankNameTable>,
    rank_base: Option<i32>,
) {
    if let Some(current_name) = rank_names
        .and_then(|names| usize::try_from(rank).ok().and_then(|rank| names.get(rank)))
        .filter(|name| !name.is_empty())
    {
        rank_name.clear();
        rank_name.push_str(&current_name);
    }
    (core.next_rank_name, core.next_rank_exp) = custom_next_rank_info(rank_names, rank_base, rank);
}

/// The state-changing half of `C4Object::DoExperience`
/// (C4Object.cpp:1518-1529). Returns whether this call promoted the info.
/// Promotion is intentionally limited to one rank per call, and hitting the
/// exact maximum suppresses promotion just like the native `< MaxExperience`
/// guard. The legacy executable performs an unchecked signed addition before
/// `BoundBy`; wrapping preserves that two's-complement runtime behavior.
pub(crate) fn adjust_crew_experience(info: &mut CrewObjectInfo, change: i32) -> bool {
    const MAX_EXPERIENCE: i32 = 100_000_000;

    info.experience = info
        .experience
        .wrapping_add(change)
        .clamp(0, MAX_EXPERIENCE);
    let next_rank = info.rank.saturating_add(1);
    if info.experience < MAX_EXPERIENCE && info.experience >= crew_rank_experience(next_rank) {
        info.rank = next_rank;
        true
    } else {
        false
    }
}

/// Native field updates in `C4PhysicalInfo::PromotionUpdate`
/// (C4InfoCore.cpp:207-222), before its optional definition-script
/// `GetFairCrewPhysical` overrides. Fair crew additionally trains Scale,
/// Hangle, Swim and Fight linearly toward `C4MaxPhysical` by rank 20.
pub(crate) fn promotion_updated_physical(
    mut physical: PhysicalInfo,
    rank: i32,
    training_definition: Option<PhysicalInfo>,
) -> PhysicalInfo {
    if rank >= 0 {
        physical.can_dig = 1;
        physical.can_chop = 1;
        physical.can_construct = 1;
        physical.can_scale = 1;
        physical.can_hangle = 1;
    }
    physical.energy = physical
        .energy
        .max((50 + 5 * rank.clamp(0, 10)) * (C4_MAX_PHYSICAL / 100));
    if let Some(definition) = training_definition {
        let train_rank = rank.clamp(0, 20);
        let train = |value: i32| value + (C4_MAX_PHYSICAL - value) * train_rank / 20;
        physical.scale = train(definition.scale);
        physical.hangle = train(definition.hangle);
        physical.swim = train(definition.swim);
        physical.fight = train(definition.fight);
    }
    physical
}

/// The persistent `C4ObjectInfo::Physical` installed on a joined/recruited
/// crew member. Fair crew never overwrites this training: GetPhysical selects
/// the definition's fair-crew projection live while the round option is on.
pub(crate) fn crew_info_physical(definition: PhysicalInfo, info_rank: i32) -> PhysicalInfo {
    promotion_updated_physical(definition, info_rank, None)
}

/// Numeric half of `C4Def::GetFairCrewPhysicals`; cache ownership and script
/// callbacks live at the definition-resolution seams below.
pub(crate) fn fair_crew_physical(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
) -> PhysicalInfo {
    let rank = fair_crew_rank(strength, rank_base);
    promotion_updated_physical(definition, rank, Some(definition))
}

pub(crate) const FAIR_CREW_PHYSICAL_NAMES: [&str; 21] = [
    "Energy",
    "Breath",
    "Walk",
    "Jump",
    "Scale",
    "Hangle",
    "Dig",
    "Swim",
    "Throw",
    "Push",
    "Fight",
    "Magic",
    "Float",
    "CanScale",
    "CanHangle",
    "CanDig",
    "CanConstruct",
    "CanChop",
    "CanFly",
    "CorrosionResist",
    "BreatheWater",
];

pub(crate) type FairCrewPhysicalCache = Rc<RefCell<HashMap<DefinitionId, PhysicalInfo>>>;

pub(crate) enum FairCrewProjectionStart {
    Cached(PhysicalInfo),
    New { physical: PhysicalInfo, rank: i32 },
}

pub(crate) fn begin_fair_crew_projection(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
    definition_id: &DefinitionId,
    cache: &FairCrewPhysicalCache,
) -> FairCrewProjectionStart {
    if let Some(physical) = cache.borrow().get(definition_id).copied() {
        return FairCrewProjectionStart::Cached(physical);
    }
    let rank = fair_crew_rank(strength, rank_base);
    let physical = promotion_updated_physical(definition, rank, Some(definition));
    // Native publishes pFairCrewPhysical before PromotionUpdate invokes any
    // hooks. Re-entrant GetPhysical calls therefore observe this in-progress
    // projection instead of recursively filling another cache entry.
    cache.borrow_mut().insert(definition_id.clone(), physical);
    FairCrewProjectionStart::New { physical, rank }
}

pub(crate) fn fair_crew_physical_cached(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
    definition_id: &DefinitionId,
    cache: &FairCrewPhysicalCache,
) -> PhysicalInfo {
    match begin_fair_crew_projection(definition, strength, rank_base, definition_id, cache) {
        FairCrewProjectionStart::Cached(physical)
        | FairCrewProjectionStart::New { physical, .. } => physical,
    }
}

/// C4PhysicalInfo::PromotionUpdate's definition callback. Every field is
/// passed by reference after the numeric promotion; the callback's result is
/// only the commit gate for the final reference value.
fn apply_fair_crew_physical_script(
    definition_physical: PhysicalInfo,
    mut physical: PhysicalInfo,
    rank: i32,
    definition_id: &DefinitionId,
    script: &ScriptEngine,
    cache: &FairCrewPhysicalCache,
) -> PhysicalInfo {
    if !script.has_function("GetFairCrewPhysical") {
        return physical;
    }
    compat::with_fair_crew_definition_context(definition_id.clone(), definition_physical, || {
        for name in FAIR_CREW_PHYSICAL_NAMES {
            let current = physical.value_by_name(name).unwrap_or_default();
            let args = [Value::from(name), Value::Int(rank), Value::Int(current)];
            match script.call_with_ref_args("GetFairCrewPhysical", &args) {
                Ok((commit, final_args)) if compat::value_raw_truthy(&commit) => {
                    let value = final_args
                        .get(2)
                        .and_then(Value::as_c4_int)
                        .unwrap_or_default();
                    physical.set_by_name(name, value);
                    cache.borrow_mut().insert(definition_id.clone(), physical);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        definition = %definition_id,
                        function = "GetFairCrewPhysical",
                        field = name,
                        error = %error,
                        "script error in fair-crew physical callback; retaining numeric value"
                    );
                    log_runtime_call_frames(definition_id, error.call_frames());
                }
            }
        }
        physical
    })
}

pub(crate) fn fair_crew_physical_with_script(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
    definition_id: &DefinitionId,
    script: &ScriptEngine,
    cache: &FairCrewPhysicalCache,
) -> PhysicalInfo {
    match begin_fair_crew_projection(definition, strength, rank_base, definition_id, cache) {
        FairCrewProjectionStart::Cached(physical) => physical,
        FairCrewProjectionStart::New { physical, rank } => apply_fair_crew_physical_script(
            definition,
            physical,
            rank,
            definition_id,
            script,
            cache,
        ),
    }
}

/// Dump one ` by: ` line per active script context, innermost first, the way
/// `C4AulExec::Exec` traces a tolerated runtime error
/// (`src/C4AulExec.cpp:1335-1346`, `C4AulScriptContext::dump` at info).
///
/// The caller reports the error itself *first* and *above* this — `err` in
/// `C4AulError::show` (`src/C4Aul.cpp:32-37`). That ordering is the contract:
/// the engine's default filter is `info`, so a message logged below its own
/// frames is dropped and the trace reads as orphan ` by: ` lines with no error
/// to explain them.
pub(crate) fn log_runtime_call_frames(definition: &str, frames: &[clonk_script::RuntimeCallFrame]) {
    for frame in frames {
        if let Some(dump) = frame.direct_exec_display() {
            tracing::info!(" by: {dump}");
            continue;
        }
        let mut dump = format!("{}({})", frame.function(), frame.arguments());
        if let Some(object) = frame.object_context() {
            dump.push_str(&format!(" (obj {object})"));
        } else if let Some(definition) = frame.definition_context() {
            dump.push_str(&format!(" (def {definition})"));
        }
        let source_name = frame.source_name().unwrap_or(definition);
        if !source_name.is_empty() {
            dump.push_str(&format!(" ({source_name}:{})", frame.source_line()));
        }
        tracing::info!(" by: {dump}");
    }
}

/// Trace a tolerated engine call whose failure came from script; anything else
/// the engine folded has no C4Aul context to dump.
pub(crate) fn log_engine_error_call_frames(error: &EngineError) {
    if let EngineError::Script { source, .. } = error {
        log_runtime_call_frames("", source.call_frames());
    }
}

pub(crate) fn tolerate_script_error<T>(
    result: Result<T, EngineError>,
) -> Result<Option<T>, EngineError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(EngineError::Script {
            definition,
            function,
            source,
            // Funnels consume the recovery payload before tolerating;
            // errors reaching here without a funnel carry none.
            recovery: _,
        }) => {
            tracing::error!(
                %definition,
                function,
                error = %source,
                "script error in engine callback; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames(&definition, source.call_frames());
            Ok(None)
        }
        Err(other) => Err(other),
    }
}

pub(crate) fn script_execution_error(
    definition: String,
    function: String,
    source: ScriptError,
    recovery: Option<Box<ScriptCallRecovery>>,
) -> EngineError {
    match source.into_diagnostic() {
        Ok(source) => EngineError::Script {
            definition,
            function,
            source,
            recovery,
        },
        Err(_) => EngineError::invalid_script_output(
            definition,
            function,
            "unsupported host continuation escaped the script error boundary".into(),
        ),
    }
}

/// Everything a failed outer script call mutated BEFORE the error: the
/// partial host outcome plus the advanced RNG and audio state. C++ keeps
/// all of it (mutations land on the live objects as they happen); the
/// engine funnels apply it before handling the error.
#[derive(Debug)]
pub struct ScriptCallRecovery {
    pub(crate) outcome: compat::EffectContextOutcome,
    pub(crate) audio: AudioRegistry,
    pub(crate) rng: LcgRng,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("definition `{0}` is already registered")]
    DefinitionAlreadyExists(String),
    #[error("unknown definition `{0}`")]
    UnknownDefinition(String),
    #[error("player {0} already exists")]
    PlayerAlreadyExists(i32),
    #[error("player resource {resource_id} is unavailable: {detail}")]
    MissingPlayerResource { resource_id: i32, detail: String },
    #[error("This scenario is designed for a maximum of {maximum} players.")]
    TooManyPlayers { maximum: i32 },
    #[error("unknown player {0}")]
    UnknownPlayer(i32),
    #[error("unknown object `{0}`")]
    UnknownObject(ObjectId),
    #[error("container error for object {object}: {detail}")]
    Container { object: ObjectId, detail: String },
    #[error("crew selection error for owner {owner}: {detail}")]
    CrewSelection { owner: i32, detail: String },
    #[error("crew role error for owner {owner}: {detail}")]
    CrewRole { owner: i32, detail: String },
    #[error("script error in {function} of `{definition}`")]
    Script {
        definition: String,
        function: String,
        #[source]
        source: clonk_script::ScriptErrorDiagnostic,
        /// The failed call's PRE-ERROR outcome. C4AulExec errors abort the
        /// call but roll nothing back (C4AulExec.cpp:1318-1342) — C++
        /// already mutated the live objects — so the engine funnel applies
        /// this before surfacing the error.
        recovery: Option<Box<ScriptCallRecovery>>,
    },
    #[error("invalid script output in {function} of `{definition}`: {detail}")]
    InvalidScriptOutput {
        definition: String,
        function: String,
        detail: String,
    },
    /// An app-owned classic UI path reached an unported presentation or
    /// action boundary. This is deliberately distinct from script output:
    /// the control fail-safe must never downgrade it into a status line.
    #[error("classic menu parity boundary: {detail}")]
    ClassicMenuParityBoundary { detail: String },
    #[error("object id `{0}` is already in use")]
    DuplicateObjectId(ObjectId),
    #[error("invalid scenario-section landscape: {0}")]
    InvalidScenarioSectionLandscape(String),
    #[error("invalid PXS component in engine state: {0}")]
    InvalidPxsComponent(String),
    #[error(transparent)]
    RuntimeJoinPlayerRestore(#[from] RuntimeJoinPlayerRestoreError),
    #[error("failed to persist scenario section `{section}`: {detail}")]
    ScenarioSectionSave { section: String, detail: String },
    #[error("failed to load objects for scenario section `{section}`: {detail}")]
    ScenarioSectionObjects { section: String, detail: String },
}

impl EngineError {
    pub(crate) fn invalid_script_output(
        definition: impl Into<String>,
        function: impl Into<String>,
        detail: String,
    ) -> Self {
        Self::InvalidScriptOutput {
            definition: definition.into(),
            function: function.into(),
            detail,
        }
    }
}

#[derive(Debug, Error)]
pub enum EngineStateIoError {
    #[error("I/O error while handling engine state")]
    Io(#[from] io::Error),
    #[error("failed to (de)serialize engine state as JSON")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnConfig {
    #[serde(default)]
    pub id: Option<ObjectId>,
    pub definition_id: DefinitionId,
    /// Saved or explicitly assigned C4Object::CustomName.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub custom_name: Option<String>,
    pub position: Vector2,
    pub velocity: Vector2,
    /// Saved C4Object::motion_x/motion_y frame caches.
    #[serde(default)]
    pub motion_x: i32,
    #[serde(default)]
    pub motion_y: i32,
    /// Exact stale/raw C4Object compiler caches. Loaded Objects.txt records
    /// populate these before pointer denumeration; fresh objects leave them
    /// at the native zero/empty defaults.
    #[serde(default, skip_serializing_if = "ObjectCompilerCache::is_default")]
    #[doc(hidden)]
    pub compiler_cache: ObjectCompilerCache,
    /// Exact sub-pixel velocity: savegame `XDir`/`YDir` are serialized
    /// C4Fixed values (C4Object.cpp:2765-2766), not whole pixels. Takes
    /// precedence over `velocity` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    #[serde(default)]
    pub rotation: i32,
    /// None = the C4Object::Init rule (alive -> GetPhysical()->Energy,
    /// else 0; C4Object.cpp:191-192), unless native compiled-object defaults
    /// are selected. Some = explicit raw value (loader).
    pub energy: Option<i32>,
    /// Saved C4Object::Damage. None uses the fresh-object zero default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<i32>,
    /// Saved C4Object::NeedEnergy. New objects default to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_energy: Option<bool>,
    /// C4Object::MagicEnergy, compiled verbatim from saves with default 0
    /// (C4Object.cpp:2768). MagicPhysicalFactor raw scale.
    #[serde(default)]
    pub magic_energy: Option<i32>,
    #[serde(default = "default_construction")]
    pub construction: i32,
    pub action: Option<ActionState>,
    /// Construction already dispatched the selected action sound through the
    /// client-local host before this deferred spawn materialized.
    #[serde(skip)]
    #[doc(hidden)]
    pub action_sound_dispatched: bool,
    /// Final attempted action-loop selection already applied by Construction.
    #[serde(skip)]
    #[doc(hidden)]
    pub action_sound_selection: Option<Option<String>>,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    /// Saved temporary physical block and its ordered stack history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_physical: Option<PhysicalInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_changes: Vec<(String, i32)>,
    /// Saved raw breath counter; fresh and generic programmatic loaded objects
    /// seed this from Physical.Breath, while native compiled records default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breath: Option<i32>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    /// Exact saved C4Shape slot storage. Loaded Objects.txt may carry
    /// non-default values beyond `Vertices`; fresh spawns leave this None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub shape_vertices: Option<ShapeVertexBuffer>,
    /// Saved C4Object::fOwnVertices flag. Its untransformed backup lives in
    /// raw shape slots 15.. and is reconstructed during loaded spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub owns_shape_vertices: Option<bool>,
    /// Exact saved live C4Shape rectangle (Width/Height/Offset). Loaded
    /// objects install it verbatim; future UpdateShape rebuilds from DefCore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_rect: Option<DefinitionRect>,
    /// Saved live C4Shape::ContactDensity. None means a fresh object copies
    /// its definition; a loaded object defaults to C4M_Solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_density: Option<i32>,
    /// Saved live C4Shape::FireTop. Fresh objects derive it from DefCore and
    /// construction; loaded objects compile the embedded shape verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_fire_top: Option<i32>,
    /// Saved C4Shape attachment coordinates. AttachMat itself is not compiled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_attach: Option<ShapeAttachRecord>,
    /// Explicit per-object `C4Object::Component` list. Loaded Objects.txt
    /// entries compile this verbatim (C4Object.cpp:2811); fresh objects use
    /// their definition components scaled to initial Con when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<ComponentList>,
    /// Explicit C4IDList order for loaded/runtime component lists. None uses
    /// definition order for fresh objects and a deterministic key fallback
    /// for legacy Rust states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_order: Option<Vec<DefinitionId>>,
    pub owner: i32,
    /// Explicit C4Object::Controller: Objects.txt `Controller=` on loads
    /// (compile default NO_OWNER, C4Object.cpp:2739) or the creating
    /// controller from script CreateObject/CreateConstruction
    /// (C4Script.cpp:1905-1906, 1932-1933). None/NO_OWNER = the Init rule
    /// (owner) for fresh spawns (C4Object.cpp:162).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<i32>,
    #[serde(default)]
    pub crew_member: Option<bool>,
    /// Saved C4Object::CrewDisabled bit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crew_disabled: Option<bool>,
    /// Saved C4Object::PlrViewRange (`PlrViewRange=` in Objects.txt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plr_view_range: Option<i32>,
    /// Saved C4Object::Select bit (`Selected=` in Objects.txt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub layer: Option<ObjectId>,
    /// Saved C4Object::Visibility. None/zero is VIS_All.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<i32>,
    /// Saved C4Object::BlitMode. None uses the definition default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blit_mode: Option<u32>,
    /// Saved raw C4Object::PictureRect (`Picture=` in Objects.txt). A zero
    /// rect is meaningful: Picture2Facet then falls back to the def picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_rect: Option<DefinitionRect>,
    /// Saved C4Object::Color (`ColorDw=` in Objects.txt). Fresh
    /// ColorByOwner objects derive it from their owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    /// Saved C4Object::ColorMod (`ColorMod=` in Objects.txt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_modulation: Option<u32>,
    /// Saved object graphics selection, draw transform, and overlay chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    #[serde(default)]
    pub alive: Option<bool>,
    /// Saved C4Object::Category. Native compiled records default to zero;
    /// fresh and generic programmatic loaded objects use the definition.
    #[serde(default)]
    pub category: Option<i32>,
    /// `InLiquid` from Objects.txt (C4Object.cpp:2775, default false).
    #[serde(default)]
    pub in_liquid: Option<bool>,
    /// Saved C4Object::EntranceStatus. Loaded objects skip Initialize, so
    /// Objects.txt must restore this independently (C4Object.cpp:2803).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrance_status: Option<bool>,
    /// Exact sub-pixel position: savegame `FixX`/`FixY` are serialized
    /// C4Fixed values (C4Object.cpp:2762-2763). C++ never reconciles them
    /// with the integer X/Y after load — the override leaves `position`
    /// untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_position: Option<FixedVec2>,
    /// Exact fixed rotation accumulator (`FixR`, C4Object.cpp:2764);
    /// independent of the whole-degree `rotation` like FixX/FixY.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_rotation: Option<C4Fixed>,
    /// Angular velocity (`RDir`, C4Object.cpp:2767).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    /// Explicit Mobile flag (Objects.txt `Mobile=`, C4Object.cpp:2772).
    /// None = the C4Object::Init rule for fresh spawns.
    #[serde(default)]
    pub mobile: Option<bool>,
    /// Mid-cycle Def TimerCall counter (Objects.txt `Timer=`, default 0,
    /// C4Object.cpp:2738).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<i32>,
    /// Saved script mass override and the compiled Mass cache. The cache is
    /// authoritative after Objects.txt load until a native UpdateMass path;
    /// a marked native compiled record installs zero when this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_mass: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub compiled_mass: Option<i32>,
    /// Saved burning state. fire_caused_by is recovered from the Fire effect
    /// because native Objects.txt does not carry a separate scalar field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_fire: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_phase: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_caused_by: Option<i32>,
    /// Saved private C4Object bookkeeping fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attach_movement_frame: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_energy_loss_cause: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_collect_delay: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<i32>,
    /// Parsed OCF is retained for format completeness, but loaded objects run
    /// SetOCF after compilation and therefore replace it from restored state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub compiled_ocf: Option<u32>,
    /// Exact top-first C4Command linked-list state from [Commands].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_stack: Option<CommandStackSnapshot>,
    /// Per-object script locals (Objects.txt `LocalNamed=`,
    /// C4Object.cpp:2788) — loaded verbatim into ObjectState.local_vars.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub local_vars: HashMap<String, Value>,
    /// The object is LOADED (Objects.txt / savegame), not created:
    /// C4GameObjects::Load (C4GameObjects.cpp:535-618) only compiles and
    /// denumerates — Construction/Initialize fire for new objects only
    /// (C4Object::Init). This selects the loaded-object lifecycle; omitted
    /// scalar values keep the programmatic fixture fallbacks unless
    /// [`Self::native_compiled_object_defaults`] is also set.
    #[serde(default)]
    pub loaded: bool,
    /// Apply C4Object::CompileFunc's literal defaults for omitted Objects.txt
    /// scalar fields: Category, Energy, Breath, and Mass all start at zero.
    /// Scenario loading sets this together with [`Self::loaded`]; keeping the
    /// marker separate lets programmatic callback-suppressed fixtures retain
    /// definition-derived values when they do not model a compiled record.
    #[serde(default, skip_serializing_if = "is_false")]
    pub native_compiled_object_defaults: bool,
    /// Saved per-object SolidMask rect (Objects.txt SolidMask=; default
    /// keeps the definition mask, C4Object.cpp:2770).
    #[serde(default)]
    pub solid_mask: Option<DefinitionTargetRect>,
    /// Runtime-only C4SolidMask construction order reserved by synchronous
    /// script creation before this deferred spawn materializes.
    #[serde(skip)]
    #[doc(hidden)]
    pub solid_mask_instance_sequence: Option<u64>,
    /// Runtime-only allocation identity reserved when C4Game::NewObject
    /// links a callback-created object before deferred copy-out.
    #[serde(skip)]
    #[doc(hidden)]
    pub instance_token: Option<u64>,
    /// Construction/Initialize already ran synchronously inside the
    /// creating host call (C4Game::NewObject semantics,
    /// C4Game.cpp:1117-1127) - materialization must not repeat them.
    #[serde(default)]
    pub initialized: bool,
    /// The DoCon bottom-growth y-adjust already happened at the creation
    /// seam (CreateObject computed the preview from the FINAL center) —
    /// materialization must not re-apply it.
    #[serde(default)]
    pub position_adjusted: bool,
}

impl SpawnConfig {
    pub fn new(definition_id: impl Into<String>) -> Self {
        Self {
            id: None,
            definition_id: definition_id.into(),
            custom_name: None,
            position: Vector2::ZERO,
            velocity: Vector2::ZERO,
            motion_x: 0,
            motion_y: 0,
            compiler_cache: ObjectCompilerCache::default(),
            fixed_velocity: None,
            rotation: 0,
            energy: None,
            damage: None,
            need_energy: None,
            magic_energy: None,
            construction: FULL_CON,
            action: None,
            action_sound_dispatched: false,
            action_sound_selection: None,
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            effects: Vec::new(),
            temporary_physical: None,
            physical_changes: Vec::new(),
            breath: None,
            vertices: Vec::new(),
            shape_vertices: None,
            owns_shape_vertices: None,
            shape_rect: None,
            contact_density: None,
            shape_fire_top: None,
            shape_attach: None,
            components: None,
            component_order: None,
            owner: OWNER_NONE,
            controller: None,
            crew_member: None,
            crew_disabled: None,
            plr_view_range: None,
            selected: None,
            status: None,
            container: None,
            layer: None,
            visibility: None,
            blit_mode: None,
            picture_rect: None,
            color: None,
            color_modulation: None,
            base_graphics: None,
            graphics_overlays: Vec::new(),
            draw_transform: None,
            alive: None,
            category: None,
            in_liquid: None,
            entrance_status: None,
            fixed_position: None,
            fixed_rotation: None,
            rotation_velocity: None,
            mobile: None,
            timer: None,
            own_mass: None,
            compiled_mass: None,
            on_fire: None,
            fire_phase: None,
            fire_caused_by: None,
            last_attach_movement_frame: None,
            last_energy_loss_cause: None,
            no_collect_delay: None,
            base: None,
            compiled_ocf: None,
            command_stack: None,
            local_vars: HashMap::new(),
            loaded: false,
            native_compiled_object_defaults: false,
            solid_mask: None,
            solid_mask_instance_sequence: None,
            instance_token: None,
            initialized: false,
            position_adjusted: false,
        }
    }

    pub fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = Some(in_liquid);
        self
    }

    pub fn with_entrance_status(mut self, entrance_status: bool) -> Self {
        self.entrance_status = Some(entrance_status);
        self
    }

    pub fn with_custom_name(mut self, name: impl Into<String>) -> Self {
        self.custom_name = Some(name.into());
        self
    }

    pub fn with_fixed_position(mut self, position: FixedVec2) -> Self {
        self.fixed_position = Some(position);
        self
    }

    pub fn with_fixed_rotation(mut self, rotation: C4Fixed) -> Self {
        self.fixed_rotation = Some(rotation);
        self
    }

    pub fn with_rotation_velocity(mut self, velocity: C4Fixed) -> Self {
        self.rotation_velocity = Some(velocity);
        self
    }

    pub fn with_mobile(mut self, mobile: bool) -> Self {
        self.mobile = Some(mobile);
        self
    }

    pub fn with_timer(mut self, timer: i32) -> Self {
        self.timer = Some(timer);
        self
    }

    pub fn with_local_vars(mut self, local_vars: HashMap<String, Value>) -> Self {
        self.local_vars = local_vars;
        self
    }

    pub fn with_solid_mask(mut self, rect: DefinitionTargetRect) -> Self {
        self.solid_mask = Some(rect);
        self
    }

    pub fn with_loaded(mut self, loaded: bool) -> Self {
        self.loaded = loaded;
        self
    }

    /// Mark this configuration as a native compiled Objects.txt record.
    /// Call together with [`Self::with_loaded`] to reproduce the complete
    /// C4GameObjects::Load lifecycle.
    pub fn with_native_compiled_object_defaults(mut self) -> Self {
        self.native_compiled_object_defaults = true;
        self
    }

    pub fn with_fixed_velocity(mut self, velocity: FixedVec2) -> Self {
        self.fixed_velocity = Some(velocity);
        self
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = position;
        self
    }

    pub fn with_rotation(mut self, rotation: i32) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = velocity;
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = Some(energy);
        self
    }

    pub fn with_need_energy(mut self, need_energy: bool) -> Self {
        self.need_energy = Some(need_energy);
        self
    }

    pub fn with_magic_energy(mut self, magic_energy: i32) -> Self {
        self.magic_energy = Some(magic_energy);
        self
    }

    pub fn with_construction(mut self, construction: i32) -> Self {
        // Final clamping needs the target definition and the loaded-object
        // flag, both resolved by spawn_single. Preserve the raw C4Object::Con
        // value here (Objects.txt may contain signed or over-100% values).
        self.construction = construction;
        self
    }

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn with_command_direction(mut self, command_direction: CommandDirection) -> Self {
        self.command_direction = command_direction;
        self
    }

    pub fn with_action(mut self, action: ActionState) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_effects(mut self, effects: Vec<EffectState>) -> Self {
        self.effects = effects;
        self
    }

    pub fn add_effect(mut self, effect: EffectState) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn with_vertices(mut self, vertices: Vec<ObjectVertex>) -> Self {
        self.vertices = vertices;
        self
    }

    pub(crate) fn with_shape_vertex_slots(
        mut self,
        active_count: usize,
        slots: Vec<ObjectVertex>,
    ) -> Self {
        self.shape_vertices = Some(ShapeVertexBuffer::from_slots(active_count, &slots));
        self
    }

    pub(crate) fn with_owns_shape_vertices(mut self, owns_vertices: bool) -> Self {
        self.owns_shape_vertices = Some(owns_vertices);
        self
    }

    pub fn with_shape_rect(mut self, rect: DefinitionRect) -> Self {
        self.shape_rect = Some(rect);
        self
    }

    pub fn with_contact_density(mut self, contact_density: i32) -> Self {
        self.contact_density = Some(contact_density);
        self
    }

    pub fn with_shape_fire_top(mut self, fire_top: i32) -> Self {
        self.shape_fire_top = Some(fire_top);
        self
    }

    pub fn with_components(mut self, components: ComponentList) -> Self {
        self.components = Some(components);
        // The list carries its own order, so no separate order vector is set
        // here; spawn resolves known entries in definition order and appends
        // unknown extras deterministically.
        self.component_order = None;
        self
    }

    pub fn with_ordered_components(mut self, components: Vec<(DefinitionId, i32)>) -> Self {
        let mut counts = ComponentList::new();
        let mut order = Vec::new();
        for (id, count) in components {
            order.push(id.clone());
            // Append rather than merge: C4IDList keeps a repeated ID as its own
            // entry (`C4IDList.cpp:33-36`), and the shipped Bazooka DefCore has
            // one. A map-shaped build kept only the first of each.
            counts.push(id, count);
        }
        self.components = Some(counts);
        self.component_order = Some(order);
        self
    }

    pub fn add_vertex(mut self, vertex: ObjectVertex) -> Self {
        self.vertices.push(vertex);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = owner;
        self
    }

    pub fn with_controller(mut self, controller: i32) -> Self {
        self.controller = Some(controller);
        self
    }

    pub fn with_category(mut self, category: i32) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_id(mut self, id: ObjectId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_status(mut self, status: ObjectStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = Some(crew_member);
        self
    }

    pub fn with_plr_view_range(mut self, plr_view_range: i32) -> Self {
        self.plr_view_range = Some(plr_view_range);
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = Some(alive);
        self
    }

    pub fn with_container(mut self, container: ObjectId) -> Self {
        self.container = Some(container);
        self
    }

    pub fn with_layer(mut self, layer: ObjectId) -> Self {
        self.layer = Some(layer);
        self
    }

    pub fn with_visibility(mut self, visibility: i32) -> Self {
        self.visibility = Some(visibility);
        self
    }

    pub fn with_blit_mode(mut self, blit_mode: u32) -> Self {
        self.blit_mode = Some(blit_mode);
        self
    }

    pub fn with_picture_rect(mut self, picture_rect: DefinitionRect) -> Self {
        self.picture_rect = Some(picture_rect);
        self
    }

    pub fn with_color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    pub fn with_color_modulation(mut self, color_modulation: u32) -> Self {
        self.color_modulation = Some(color_modulation);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: ObjectId,
    pub definition_id: DefinitionId,
    /// C4Object::CustomName; None falls back to crew-info/definition name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub custom_name: Option<String>,
    pub position: Vector2,
    pub velocity: Vector2,
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    /// C4Object::NeedEnergy, persisted by C++ as `NeedEnergy=`
    /// (C4Object.cpp:2805).
    #[serde(default, skip_serializing_if = "is_false")]
    pub need_energy: bool,
    #[serde(default)]
    pub damage: i32,
    #[serde(default)]
    pub magic_energy: i32,
    #[serde(default)]
    pub magic_capacity: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
    #[serde(default)]
    pub action: ActionState,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    #[serde(default)]
    pub action_procedure: Option<String>,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    /// Exceptional live C4Object::Shape rectangle after SetShape or a loaded
    /// embedded shape. None means the exact rectangle is reconstructible from
    /// DefCore, Con and rotation. The sparse sidecar preserves C++ runtime
    /// shape state in renderer snapshots and saved recordings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_shape: Option<DefinitionRect>,
    /// Exceptional live C4Shape::FireTop loaded with the embedded shape.
    /// None means DefCore FireTop scaled by Con (C4Shape.cpp:103-127,495-510).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_fire_top: Option<i32>,
    /// Saved live C4Shape::ContactDensity (C4Shape.cpp:495-510).
    #[serde(
        default = "default_contact_density",
        skip_serializing_if = "is_default_contact_density"
    )]
    pub contact_density: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_vertices: Option<Vec<ObjectVertex>>,
    /// `C4Shape::VtxContactCNAT` values aligned with `vertices`. These are
    /// presentation diagnostics, not collision input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vertex_contacts: Vec<u32>,
    /// Per-object `C4Object::SolidMask` override. `None` selects DefCore;
    /// a zero-area rectangle explicitly disables the definition mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_mask_override: Option<DefinitionTargetRect>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    /// C4Object::pLayer (Objects.txt `Layer=` / SetObjectLayer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<ObjectId>,
    /// C4Object::Visibility; zero is VIS_All.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub visibility: i32,
    /// C4Object::BlitMode, including the C4GFXBLIT_CUSTOM marker.
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub blit_mode: u32,
    /// C4Object::Color (owner color for ColorByOwner definitions, or an
    /// explicit SetColorDw/Objects.txt value).
    #[serde(default)]
    pub color: u32,
    /// C4Object::ColorMod, persisted as `ColorMod=` in Objects.txt.
    #[serde(default)]
    pub color_modulation: u32,
    /// Raw per-object C4Object::PictureRect. Width zero selects the
    /// definition-default facet at draw time.
    #[serde(default)]
    pub picture_rect: DefinitionRect,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub components: ComponentList,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_order: Vec<DefinitionId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    /// C4Object::Controller (persisted like the savegame `Controller`
    /// field, default NO_OWNER, C4Object.cpp:2739).
    #[serde(default = "default_owner")]
    pub controller: i32,
    #[serde(default = "default_category")]
    pub category: i32,
    #[serde(default)]
    pub crew_member: bool,
    /// Saved C4Object::PlrViewRange.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub plr_view_range: i32,
    /// C4Object::Select, persisted independently of the player's cursor
    /// (C4Object.cpp:2800).
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    #[serde(default = "default_alive")]
    pub alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default)]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    #[serde(default)]
    pub command_queue: Vec<QueuedCommand>,
    #[serde(default)]
    pub command_stack: CommandStackSnapshot,
    #[serde(default)]
    pub local_vars: HashMap<String, Value>,
    /// Cached liquid flag (C4Object::InLiquid, persisted like the
    /// savegame `InLiquid` field, C4Object.cpp:2775).
    #[serde(default)]
    pub in_liquid: bool,
    /// C4Object::Mobile (persisted like the savegame `Mobile` field,
    /// C4Object.cpp:2772).
    #[serde(default)]
    pub mobile: bool,
    /// The object's current OCF bits (C4Object::OCF) — broadcast world
    /// feeds need them for OCF-filtered searches.
    #[serde(default)]
    pub ocf: u32,
    /// The Def TimerCall counter (persisted like the savegame `Timer`
    /// field, C4Object.cpp:2738).
    #[serde(default)]
    pub timer: i32,
    /// C4Object::OwnMass (SetMass leftovers; persisted like the savegame).
    #[serde(default)]
    pub own_mass: i32,
    /// Burning state (C4Object::OnFire) with its animation phase and the
    /// causing player (the fire effect's CausedBy var).
    #[serde(default)]
    pub on_fire: bool,
    #[serde(default)]
    pub fire_phase: i32,
    #[serde(default = "default_owner")]
    pub fire_caused_by: i32,
    /// `C4ObjectInfo::Physical` surrogate (crew training); C++ persists info
    /// physicals with the crew file — carried on the object until the
    /// C4ObjectInfo model lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_physical: Option<PhysicalInfo>,
    /// `PhysicalTemporary`/`TemporaryPhysical` (C4Object.cpp:2777,2798-2801).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_physical: Option<PhysicalInfo>,
    /// `C4TempPhysicalInfo::Changes` (C4InfoCore.cpp:306) as
    /// (physical name, previous value) pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_changes: Vec<(String, i32)>,
    /// `C4Object::Breath` (C4Object.cpp:2738 compile list).
    #[serde(default)]
    pub breath: i32,
    /// `LastEngLossPlr` (C4Object.cpp:2740) — kill attribution.
    #[serde(default = "default_owner")]
    pub last_energy_loss_cause: i32,
    /// `C4Object::Base` (C4Object.h:135): home-base player or NO_OWNER
    /// (ExecBase flag assignment, C4Object.cpp:1000-1031).
    #[serde(default = "default_owner")]
    pub base: i32,
    /// Raw 16.16 fixed-point position, recorded only when it carries sub-pixel
    /// detail beyond the whole-pixel `position` (i.e. `position != fixtoi(fix)`).
    /// `None` ⇒ reconstruct losslessly via `itofix(position)`. Mirrors C++
    /// persisting both `x` and `fix_x` (`C4Object.cpp:2742`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_position: Option<FixedVec2>,
    /// Raw 16.16 fixed-point velocity, recorded only when it carries sub-pixel
    /// detail beyond the whole-pixel `velocity`. `None` ⇒ `itofix(velocity)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    /// Raw 16.16 fixed-point angular velocity (`rdir`), present when nonzero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    /// Raw rotation accumulator (`fix_r`), omitted only when it equals
    /// `itofix(rotation)` and can be reconstructed exactly. C++ persists raw
    /// signed `r`, `fix_r`, and `rdir` independently (C4Object.cpp:2769,
    /// 2789,2792).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_rotation: Option<C4Fixed>,
}

/// Persisted runtime values owned by `C4AulScriptEngine`: numbered
/// `Global(index)` slots and declared `GlobalNamed` statics
/// (C4Aul.cpp:506-520). Ordered maps keep JSON byte-stable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptGlobalState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub numbered: BTreeMap<i32, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub named: BTreeMap<String, Value>,
}

/// Presentation metadata for definitions whose `Line` DefCore field turns
/// their live shape vertices into a polyline (`C4Object::DrawLine`). Kept on
/// the snapshot because the renderer deliberately has no mutable `Engine`
/// access.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionLineMetadata {
    #[serde(default)]
    pub line: i32,
    #[serde(default)]
    pub line_intersect: i32,
}

#[cfg(test)]
mod walk_rotation_mobility_tests {
    use super::*;

    /// `FnSetRDir` mobilises (`C4Script.cpp:732`), so a staged
    /// `rotation_velocity` must keep arming `Mobile`.
    #[test]
    fn a_script_rdir_write_still_mobilizes() {
        let mut delta = ObjectDelta::default();
        delta.rotation_velocity = Some(C4Fixed::from_raw(4096));

        let mut object = test_object();
        object.state.mobile = false;
        object.apply_delta(&delta, &ActionLibrary::default());

        assert!(object.state.mobile, "SetRDir must mobilize like C++");
        assert_eq!(object.rotation_velocity, C4Fixed::from_raw(4096));
    }

    /// `C4Object::AdjustWalkRotation` writes `rdir` directly and never
    /// touches `Mobile` (`C4Object.cpp:6085-6088`). The staged field outlives
    /// the procedure that set it, so a walking object that dies in the same
    /// frame carried it into `ChangeDef`'s fold and was re-mobilised after
    /// `ExecMovement` had demobilised it — a corpse the oracle had at rest
    /// (clonk-org/clonk-rs#1157).
    #[test]
    fn a_native_rdir_write_leaves_mobile_alone() {
        let mut delta = ObjectDelta::default();
        delta.rotation_velocity_raw = Some(C4Fixed::from_raw(4096));

        let mut object = test_object();
        object.state.mobile = false;
        object.apply_delta(&delta, &ActionLibrary::default());

        assert!(
            !object.state.mobile,
            "a native rdir write must not mobilize; C++ leaves Mobile untouched"
        );
        assert_eq!(object.rotation_velocity, C4Fixed::from_raw(4096));
    }

    /// And it must not *clear* the flag either: C++ leaves it exactly as it
    /// found it.
    #[test]
    fn a_native_rdir_write_preserves_an_already_mobile_object() {
        let mut delta = ObjectDelta::default();
        delta.rotation_velocity_raw = Some(C4Fixed::from_raw(-2048));

        let mut object = test_object();
        object.state.mobile = true;
        object.apply_delta(&delta, &ActionLibrary::default());

        assert!(object.state.mobile, "an already-mobile object stays mobile");
    }

    fn test_object() -> Object {
        Object::new(
            ObjectId::new(1),
            "FISH".to_string(),
            serde_json::from_value(serde_json::json!({
                "position": {"x": 0, "y": 0},
                "velocity": {"x": 0, "y": 0},
                "rotation": 0,
                "energy": 0,
                "action": {
                    "name": "Walk",
                    "phase": 0,
                    "ticks": 0,
                    "time": 0,
                    "data": 0
                },
                "vertices": [],
                "contents": [],
                "effects": [],
            }))
            .expect("object state deserializes"),
            ObjectShapeTemplate::new(Vec::new(), None, 0, false, 0),
            None,
        )
    }
}
