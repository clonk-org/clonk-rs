//! Port of the C4Particles "newgfx" particle system (src/C4Particles.{h,cpp}).
//!
//! IMPORTANT parity note: the C++ header (C4Particles.h:18-27) declares this
//! system to hold "everything, that is not sync-relevant". Every random draw
//! in C4Particles.cpp goes through `SafeRandom` — libc `rand()` seeded from
//! wall-clock time (C4Random.h:35,71-75) — never the synced `Random()` LCG,
//! and particle counts scale with the *local* `Config.Graphics.SmokeLevel`.
//! Particles therefore cannot desync the simulation; what must match C++ is
//! the script-visible surface (host-function return values and side-effect
//! structure), not the random streams themselves.

use crate::math::{fixtof, C4Fixed};
use crate::{ObjectId, ParticleLayer};
use clonk_resources::GraphicsImage;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use thiserror::Error;

/// Engine state read by the particle exec procs: GravAccel,
/// `Game.FrameCounter`, landscape bounds, `GBackSolid`, and `GBackWind`.
pub struct ParticleEnv<'a> {
    pub gravity: C4Fixed,
    pub frame_counter: i32,
    pub back_wdt: i32,
    pub back_hgt: i32,
    pub solid: &'a dyn Fn(i32, i32) -> bool,
    pub wind: &'a dyn Fn(i32, i32) -> i32,
}

/// Position/velocity of the object a particle list is attached to, used by
/// fxStdExec for Attach defs (C4Particles.cpp:619-625).
#[derive(Debug, Clone, Copy)]
pub struct ParticleTarget {
    pub x: i32,
    pub y: i32,
    pub xdir: C4Fixed,
    pub ydir: C4Fixed,
}

/// Maximum number of particles of one type. C4Constants.h:71.
pub const C4PX_MAX_PARTICLE: i32 = 256;

/// Mirror of `C4ParticleDefCore` (C4Particles.h:53-85). Field defaults follow
/// the C++ constructor (C4Particles.cpp:58-74) plus the `CompileFunc` INI
/// defaults (C4Particles.cpp:30-56); `WindDrift` is only initialized by
/// `CompileFunc` (default 0).
pub use clonk_resources::ParticleDefinitionCore as ParticleDefCore;

/// `LightenClrBy` (StdColors.h:216-223): per-RGB-byte saturating add, alpha
/// byte unchanged.
fn lighten_clr_by(clr: u32, by: u8) -> u32 {
    let lighten = |byte: u32| (byte + by as u32).min(0xff);
    (clr & 0xff000000)
        | (lighten((clr >> 16) & 0xff) << 16)
        | (lighten((clr >> 8) & 0xff) << 8)
        | lighten(clr & 0xff)
}

/// Rust stand-in for `SafeRandom` (C4Random.h:71-75).
///
/// C++ uses libc `rand()` seeded from wall-clock time (C4Random.h:35), making
/// the stream deliberately unsynced across clients — only the call structure
/// is parity-relevant, never the values. This port uses the classic POSIX
/// example LCG so tests can pin a deterministic stream via an explicit seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeRng {
    state: u32,
}

impl SafeRng {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// `SafeRandom(range)`: `if (!range) return 0; return rand() % range;`.
    pub fn random(&mut self, range: i32) -> i32 {
        if range == 0 {
            return 0;
        }
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        (((self.state >> 16) & 0x7fff) as i32) % range
    }
}

impl Default for SafeRng {
    fn default() -> Self {
        Self::new(1)
    }
}

/// Init/exec/collision procedures from `C4ParticleProcMap`
/// (C4Particles.cpp:790-801). C++ stores raw function pointers; the Rust port
/// dispatches on this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticleProc {
    SmokeInit,
    SmokeExec,
    StdInit,
    StdExec,
    Bounce,
    BounceY,
    Stop,
    Die,
}

/// Drawing procedures from `C4ParticleDrawProcMap`
/// (C4Particles.cpp:803-808). Keeping this separate from [`ParticleProc`]
/// prevents an init/exec procedure name from being accepted as a draw proc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticleDrawProc {
    Smoke,
    Std,
}

impl ParticleDrawProc {
    /// `C4ParticleSystem::GetDrawProc` (C4Particles.cpp:455-463).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Smoke" => Some(Self::Smoke),
            "Std" => Some(Self::Std),
            _ => None,
        }
    }
}

/// Source facet retained from a particle group's `Face=` setting. Target
/// offsets are preserved even though the two built-in C++ draw procs do not
/// consult them; exposing the normalized value keeps the render catalog a
/// faithful description of the loaded resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticleGraphicsFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

/// Geometry of the phase grid derived by `C4Facet::GetPhaseNum`
/// (C4Facet.cpp:391-398). Std draws horizontal phases from row zero; Smoke
/// addresses its native 4x4 grid explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticleSourcePhaseGeometry {
    pub width: i32,
    pub height: i32,
    pub columns: i32,
    pub rows: i32,
}

/// Runtime-only graphics payload exposed to the frontend. Particle
/// definitions are rebuilt from definition resources on startup, so the
/// decoded RGBA payload is intentionally omitted from save-state serde.
#[derive(Debug, Clone)]
pub struct ParticleGraphics {
    pub image: GraphicsImage,
    pub facet: ParticleGraphicsFacet,
    pub phases: ParticleSourcePhaseGeometry,
}

impl PartialEq for ParticleGraphics {
    fn eq(&self, other: &Self) -> bool {
        self.facet == other.facet
            && self.phases == other.phases
            && self.image.width() == other.image.width()
            && self.image.height() == other.image.height()
            && self.image.pixels() == other.image.pixels()
    }
}

impl ParticleProc {
    /// `C4ParticleSystem::GetProc` (C4Particles.cpp:445-453): nullptr when the
    /// name is not in the map.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "SmokeInit" => Some(Self::SmokeInit),
            "SmokeExec" => Some(Self::SmokeExec),
            "StdInit" => Some(Self::StdInit),
            "StdExec" => Some(Self::StdExec),
            "Bounce" => Some(Self::Bounce),
            "BounceY" => Some(Self::BounceY),
            "Stop" => Some(Self::Stop),
            "Die" => Some(Self::Die),
            _ => None,
        }
    }
}

/// A loaded particle definition: core + values derived during
/// `C4ParticleDef::Load` (C4Particles.cpp:118-192).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleDef {
    pub core: ParticleDefCore,
    /// Number of animation phases in the graphics (C4Particles.cpp:141), after
    /// the FadeOutLen adjustment (C4Particles.cpp:148-152).
    pub length: i32,
    /// Native's literal facet width/height ratio (C4Particles.cpp:156).
    pub aspect: f32,
    pub init_proc: ParticleProc,
    pub exec_proc: ParticleProc,
    pub collision_proc: Option<ParticleProc>,
    pub draw_proc: ParticleDrawProc,
    /// Decoded image and phase-grid metadata for resource-backed defs.
    /// Manually registered simulation-only defs retain `None`.
    #[serde(default, skip)]
    pub graphics: Option<ParticleGraphics>,
    /// Number of live particles of this kind (C4Particles.h:104).
    pub count: i32,
    /// `C4ParticleDef::Filename` — the group this def was loaded from.
    /// `Reload` re-opens exactly this path (`C4Particles.cpp:196-205`), and a
    /// def with no filename refuses to reload at all (`:197`). Manually
    /// registered simulation-only defs have none.
    #[serde(default, skip)]
    pub source_path: Option<std::path::PathBuf>,
}

/// Load failure reasons (C4Particles.cpp:142-177 logs + returns false).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParticleDefError {
    #[error("particle graphics have no horizontal phases")]
    InvalidLength,
    #[error("particle facet dimensions must be positive, got {width}x{height}")]
    InvalidFacetDimensions { width: i32, height: i32 },
    #[error("particle image dimensions exceed the supported i32 range")]
    ImageDimensionsOutOfRange,
    #[error("unknown particle init procedure `{0}`")]
    UnknownInitProc(String),
    #[error("unknown particle exec procedure `{0}`")]
    UnknownExecProc(String),
    #[error("unknown particle collision procedure `{0}`")]
    UnknownCollisionProc(String),
    #[error("unknown particle draw procedure `{0}`")]
    UnknownDrawProc(String),
}

/// One live particle, mirroring `C4Particle` (C4Particles.h:118-134). The C++
/// chunk/free-list storage is an allocation strategy; the Rust port keeps a
/// flat Vec tagged with the list (`layer`) each particle belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Particle {
    pub def_name: String,
    pub x: f32,
    pub y: f32,
    pub xdir: f32,
    pub ydir: f32,
    pub life: i32,
    pub a: f32,
    pub b: i32,
    pub layer: ParticleLayer,
}

/// Mirror of `C4ParticleSystem` (C4Particles.h:172-219). Simulation remains
/// here; decoded draw metadata is exposed read-only for the frontend.
#[derive(Debug, Clone)]
pub struct ParticleSystem {
    /// Insertion-ordered def list (C++ keeps a linked list, pDef0..pDefL).
    defs: Vec<ParticleDef>,
    def_names: RefCell<Rc<HashSet<String>>>,
    reloadable_def_names: RefCell<Rc<HashSet<String>>>,
    reloadable_def_io_success: RefCell<Rc<HashMap<String, bool>>>,
    def_name_caches_dirty: Cell<bool>,
    particles: Vec<Particle>,
    pub safe_rng: SafeRng,
    /// Local `Config.Graphics.SmokeLevel` (default 200, C4Config.cpp:452).
    /// Scales every def's MaxCount in `create` — a per-client setting, which
    /// is why particle counts are not sync-relevant in C++ either.
    pub smoke_level: i32,
    /// Local `Config.Graphics.FireParticles` (default true, C4Config.cpp:484).
    /// `SetDefParticles` leaves pFire1/pFire2 null when it is off
    /// (C4Particles.cpp:483-489), which is what `is_fire_particle_loaded`
    /// reports — also a per-client setting.
    pub fire_particles: bool,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        Self {
            defs: Vec::new(),
            def_names: RefCell::new(Rc::new(HashSet::new())),
            reloadable_def_names: RefCell::new(Rc::new(HashSet::new())),
            reloadable_def_io_success: RefCell::new(Rc::new(HashMap::new())),
            def_name_caches_dirty: Cell::new(false),
            particles: Vec::new(),
            safe_rng: SafeRng::default(),
            smoke_level: crate::DEFAULT_SMOKE_LEVEL,
            fire_particles: DEFAULT_FIRE_PARTICLES,
        }
    }
}

/// `Config.Graphics.FireParticles` default (C4Config.cpp:484).
pub const DEFAULT_FIRE_PARTICLES: bool = true;

/// The stock fire particle def names `SetDefParticles` resolves into
/// pFire1/pFire2 (C4Particles.cpp:485-486).
pub const FIRE_DEF_NAME: &str = "Fire";
pub const FIRE2_DEF_NAME: &str = "Fire2";

/// One burning object's state, snapshotted where `FnFxFireTimer` reads it
/// (C4Effect.cpp:679-701) so the emitter can run at the particle system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectFireEmission {
    /// The burning object; owns the back/front particle lists C++ deals to.
    pub object: ObjectId,
    /// `C4Fx_FireMode_*`, read back from the effect's Var 0.
    pub fire_mode: i32,
    /// `Def->Shape.Wdt` / `Def->Shape.Hgt` / `Def->Shape.FireTop`.
    pub def_width: i32,
    pub def_height: i32,
    pub fire_top: i32,
    /// `GetCon()`, in `FullCon` units.
    pub con: i32,
    /// `Def->GrowthType` — false pins `iWdtCon` at 100 below full con.
    pub growth_type: bool,
    /// `pObj->x` / `pObj->y`, also the Attach offset origin.
    pub x: i32,
    pub y: i32,
    /// The live instance `Shape` rect, object-relative.
    pub shape_x: i32,
    pub shape_y: i32,
    pub shape_width: i32,
    pub shape_height: i32,
    /// `pObj->r` in degrees and `Def->Rotateable`.
    pub rotation: i32,
    pub rotateable: bool,
    /// `fixtof(pObj->xdir)` / `fixtof(pObj->ydir)` — the raw fixed velocity,
    /// which C++ scales by 3 and truncates toward zero.
    pub xdir: f32,
    pub ydir: f32,
}

impl ParticleSystem {
    fn refresh_def_name_caches(&self) {
        *self.def_names.borrow_mut() =
            Rc::new(self.defs.iter().map(|def| def.core.name.clone()).collect());
        *self.reloadable_def_names.borrow_mut() = Rc::new(
            self.defs
                .iter()
                .filter(|def| def.source_path.is_some())
                .map(|def| def.core.name.clone())
                .collect(),
        );
        // `C4ParticleDef::Reload` opens `Filename` and loads the complete
        // definition before returning (`C4Particles.cpp:194-205`). Seed that
        // complete preflight result alongside the names so
        // `FnReloadParticle` can report an I/O/load failure before its
        // deferred request is applied.
        *self.reloadable_def_io_success.borrow_mut() = Rc::new(
            self.defs
                .iter()
                .filter_map(|def| {
                    let path = def.source_path.as_ref()?;
                    Some((
                        def.core.name.clone(),
                        clonk_resources::Group::open(path)
                            .ok()
                            .and_then(|group| {
                                clonk_resources::ParticleDefinition::load(&group).ok()
                            })
                            .is_some(),
                    ))
                })
                .collect(),
        );
        self.def_name_caches_dirty.set(false);
    }

    fn refresh_def_name_caches_if_dirty(&self) {
        if self.def_name_caches_dirty.get() {
            self.refresh_def_name_caches();
        }
    }

    /// `C4ParticleSystem::GetDef` (C4Particles.cpp:465-473).
    pub fn get_def(&self, name: &str) -> Option<&ParticleDef> {
        self.defs.iter().find(|def| def.core.name == name)
    }

    pub fn get_def_mut(&mut self, name: &str) -> Option<&mut ParticleDef> {
        let index = self.defs.iter().position(|def| def.core.name == name)?;
        // Name and source path are public fields, so arbitrary mutation may
        // invalidate both script-host snapshots. Their next reader rebuilds
        // from the live definition list after this mutable borrow ends.
        self.def_name_caches_dirty.set(true);
        self.defs.get_mut(index)
    }

    pub fn def_count(&self) -> usize {
        self.defs.len()
    }

    /// Definitions in native linked-list order (`pDef0` through `pDefL`).
    /// Successful overloads remove the first exact-case match and append the
    /// replacement at the tail.
    pub fn definitions(&self) -> &[ParticleDef] {
        &self.defs
    }

    /// `C4ParticleDef::Load` (C4Particles.cpp:118-192): derive length/aspect,
    /// resolve procs, then replace any older def of the same name
    /// ("particle overloading", C4Particles.cpp:178-187).
    pub fn register_def(
        &mut self,
        core: ParticleDefCore,
        gfx_length: i32,
        aspect: f32,
    ) -> Result<(), ParticleDefError> {
        self.register_def_with_graphics(core, gfx_length, aspect, None, None)
    }

    fn register_def_with_graphics(
        &mut self,
        core: ParticleDefCore,
        gfx_length: i32,
        aspect: f32,
        graphics: Option<ParticleGraphics>,
        source_path: Option<std::path::PathBuf>,
    ) -> Result<(), ParticleDefError> {
        if gfx_length <= 0 {
            return Err(ParticleDefError::InvalidLength);
        }
        let mut core = core;
        // case fadeout from length (C4Particles.cpp:148-152)
        let mut length = gfx_length;
        if core.fade_out_len != 0 {
            length = (length - core.fade_out_len).max(1);
            if core.fade_out_delay == 0 {
                core.fade_out_delay = 1;
            }
        }
        // if phase num is 1, no reverse is allowed (C4Particles.cpp:154)
        if length == 1 {
            core.reverse = 0;
        }
        let init_proc = ParticleProc::from_name(&core.init_fn)
            .ok_or_else(|| ParticleDefError::UnknownInitProc(core.init_fn.clone()))?;
        let exec_proc = ParticleProc::from_name(&core.exec_fn)
            .ok_or_else(|| ParticleDefError::UnknownExecProc(core.exec_fn.clone()))?;
        let collision_proc = (!core.collision_fn.is_empty())
            .then(|| {
                ParticleProc::from_name(&core.collision_fn).ok_or_else(|| {
                    ParticleDefError::UnknownCollisionProc(core.collision_fn.clone())
                })
            })
            .transpose()?;
        let draw_proc = ParticleDrawProc::from_name(&core.draw_fn)
            .ok_or_else(|| ParticleDefError::UnknownDrawProc(core.draw_fn.clone()))?;

        // Resolve every procedure before mutating the list. In C++ the old
        // same-name definition is deleted only after the new definition has
        // loaded its graphics and resolved all function pointers.
        let name = core.name.clone();
        if let Some(index) = self.defs.iter().position(|def| def.core.name == name) {
            self.defs.remove(index);
        }
        self.defs.push(ParticleDef {
            source_path,
            core,
            length,
            aspect,
            init_proc,
            exec_proc,
            collision_proc,
            draw_proc,
            graphics,
            count: 0,
        });
        self.refresh_def_name_caches();
        Ok(())
    }

    /// Register a decoded `Particle.txt` resource. Graphics-derived values
    /// follow `C4ParticleDef::Load`: horizontal image/facet division yields
    /// the raw phase count and Aspect is literally facet width/height. C++
    /// accepts many out-of-bounds facets; Rust preserves that weak contract
    /// while rejecting non-positive divisors so arithmetic remains safe.
    pub fn register_resource(
        &mut self,
        resource: &clonk_resources::ParticleDefinition,
    ) -> Result<(), ParticleDefError> {
        self.register_resource_from(resource, None)
    }

    /// Register a decoded resource, remembering the group it came from so
    /// `C4ParticleDef::Reload` can re-open exactly that path
    /// (`C4Particles.cpp:196-205`).
    pub fn register_resource_from(
        &mut self,
        resource: &clonk_resources::ParticleDefinition,
        source_path: Option<std::path::PathBuf>,
    ) -> Result<(), ParticleDefError> {
        let facet = ParticleGraphicsFacet {
            x: resource.facet.x,
            y: resource.facet.y,
            width: resource.facet.width,
            height: resource.facet.height,
            target_x: resource.facet.target_x,
            target_y: resource.facet.target_y,
        };
        if facet.width <= 0 || facet.height <= 0 {
            return Err(ParticleDefError::InvalidFacetDimensions {
                width: facet.width,
                height: facet.height,
            });
        }
        let image_width = i32::try_from(resource.image.width())
            .map_err(|_| ParticleDefError::ImageDimensionsOutOfRange)?;
        let image_height = i32::try_from(resource.image.height())
            .map_err(|_| ParticleDefError::ImageDimensionsOutOfRange)?;
        let columns = image_width / facet.width;
        let rows = image_height / facet.height;
        let aspect = facet.width as f32 / facet.height as f32;
        let graphics = ParticleGraphics {
            image: resource.image.clone(),
            facet,
            phases: ParticleSourcePhaseGeometry {
                width: facet.width,
                height: facet.height,
                columns,
                rows,
            },
        };
        self.register_def_with_graphics(
            resource.core.clone(),
            columns,
            aspect,
            Some(graphics),
            source_path,
        )
    }

    /// The definition list in registration order.
    pub fn defs(&self) -> &[ParticleDef] {
        &self.defs
    }

    /// Move the most recently registered definition back to `index`.
    ///
    /// `C4ParticleDef::Reload` mutates the definition **in place**, so its
    /// position in `pDef0..pDefL` is unchanged. Rebuilding it by remove and
    /// re-register would move it to the tail and reorder every later
    /// definition, which changes what `GetDef` finds for a duplicate name.
    pub fn restore_def_order(&mut self, index: usize) {
        if index < self.defs.len() {
            let last = self.defs.len() - 1;
            self.defs[index..=last].rotate_right(1);
        }
    }

    /// `delete pDef` — unlink one definition from the list
    /// (`C4Particles.cpp:104-111`).
    ///
    /// C++ leaves `pSmoke`/`pBlast`/`pFSpark`/`pFire1`/`pFire2` dangling here
    /// because it never re-runs `SetDefParticles`; the port drops the entry
    /// cleanly instead, since reproducing a dangling pointer is not parity.
    pub fn remove_def(&mut self, name: &str) -> bool {
        let before = self.defs.len();
        self.defs.retain(|def| def.core.name != name);
        let removed = self.defs.len() != before;
        if removed {
            self.refresh_def_name_caches();
        }
        removed
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    /// Names of all loaded defs, for the host-function GetDef checks.
    pub fn def_names(&self) -> std::collections::HashSet<String> {
        #[cfg(test)]
        crate::PARTICLE_DEF_NAME_REBUILDS.with(|count| count.set(count.get() + 1));
        self.refresh_def_name_caches_if_dirty();
        self.def_names.borrow().as_ref().clone()
    }

    pub(crate) fn shared_def_names(&self) -> Rc<HashSet<String>> {
        self.refresh_def_name_caches_if_dirty();
        Rc::clone(&self.def_names.borrow())
    }

    /// Attach the group a definition was loaded from after the fact.
    ///
    /// Production registers the path with the definition
    /// (`register_resource_from`); this exists for callers that build a
    /// definition first and learn its group second.
    pub fn set_def_source_path(&mut self, name: &str, path: Option<std::path::PathBuf>) -> bool {
        let found = match self.defs.iter_mut().find(|def| def.core.name == name) {
            Some(def) => {
                def.source_path = path;
                true
            }
            None => false,
        };
        if found {
            self.refresh_def_name_caches();
        }
        found
    }

    /// The definitions a reload could actually re-open — those carrying a
    /// `Filename`. `C4ParticleDef::Reload` refuses without one
    /// (`C4Particles.cpp:197`), so a manually registered simulation-only def
    /// can never reload however it is named.
    pub fn reloadable_def_names(&self) -> std::collections::HashSet<String> {
        self.refresh_def_name_caches_if_dirty();
        self.reloadable_def_names.borrow().as_ref().clone()
    }

    pub(crate) fn shared_reloadable_def_names(&self) -> Rc<HashSet<String>> {
        self.refresh_def_name_caches_if_dirty();
        Rc::clone(&self.reloadable_def_names.borrow())
    }

    pub(crate) fn shared_reloadable_def_io_success(&self) -> Rc<HashMap<String, bool>> {
        self.refresh_def_name_caches_if_dirty();
        Rc::clone(&self.reloadable_def_io_success.borrow())
    }

    /// `C4ParticleList::Remove` via FnClearParticles (C4Script.cpp:4925-4944):
    /// the global scope clears the global list; an object scope clears that
    /// object's front and back lists. Def counts are released
    /// (C4Particles.cpp:291-310).
    pub fn remove(&mut self, def_name: Option<&str>, scope: &crate::ParticleScope) {
        let mut removed: Vec<String> = Vec::new();
        self.particles.retain(|particle| {
            let scope_matches = match scope {
                crate::ParticleScope::Global => particle.layer == ParticleLayer::Global,
                crate::ParticleScope::Object(id) => matches!(
                    particle.layer,
                    ParticleLayer::ObjectFront(layer_id) | ParticleLayer::ObjectBack(layer_id)
                        if layer_id == *id
                ),
            };
            let def_matches = def_name.is_none_or(|name| particle.def_name == name);
            if scope_matches && def_matches {
                removed.push(particle.def_name.clone());
                false
            } else {
                true
            }
        });
        for name in removed {
            if let Some(def) = self.defs.iter_mut().find(|def| def.core.name == name) {
                def.count -= 1;
            }
        }
    }

    /// Reinstate a particle from a saved snapshot, bumping its def count.
    /// Bypasses the init proc and creation limits (load path, not Create).
    pub fn restore_particle(&mut self, particle: Particle) {
        if let Some(def) = self
            .defs
            .iter_mut()
            .find(|def| def.core.name == particle.def_name)
        {
            def.count += 1;
        }
        self.particles.push(particle);
    }

    /// `C4ParticleSystem::ClearParticles` (C4Particles.cpp:343-365): drop all
    /// particles and reset def counts (defs stay loaded).
    pub fn clear_particles(&mut self) {
        self.particles.clear();
        for def in &mut self.defs {
            def.count = 0;
        }
    }

    /// `C4ParticleSystem::Create` (C4Particles.cpp:378-419). `attach_origin`
    /// carries the creating object's position for the Attach offset
    /// (C4Particles.cpp:404-408). Returns whether a particle was created.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        def_name: &str,
        x: f32,
        y: f32,
        xdir: f32,
        ydir: f32,
        a: f32,
        b: i32,
        layer: ParticleLayer,
        attach_origin: Option<(i32, i32)>,
    ) -> bool {
        let Some(def_index) = self.defs.iter().position(|def| def.core.name == def_name) else {
            return false;
        };
        // check count (C4Particles.cpp:389-394)
        let max_count = self.defs[def_index].core.max_count * (self.smoke_level + 20) / 150;
        let room = max_count - self.defs[def_index].count;
        if room <= 0 {
            return false;
        }
        // reduce creation if limit is nearly reached
        if room < (max_count >> 1) && self.safe_rng.random(room) < self.safe_rng.random(max_count) {
            return false;
        }
        let mut particle = Particle {
            def_name: def_name.to_string(),
            x,
            y,
            xdir,
            ydir,
            life: 0,
            a,
            b,
            layer,
        };
        if self.defs[def_index].core.attach != 0 {
            if let Some((origin_x, origin_y)) = attach_origin {
                particle.x -= origin_x as f32;
                particle.y -= origin_y as f32;
            }
        }
        // call initialization (C4Particles.cpp:410-412)
        let init_proc = self.defs[def_index].init_proc;
        if !self.run_init_proc(init_proc, def_index, &mut particle) {
            return false;
        }
        self.defs[def_index].count += 1;
        self.particles.push(particle);
        true
    }

    /// `C4ParticleSystem::Cast` (C4Particles.cpp:421-443): create `amount`
    /// particles with SafeRandom-spread velocity (±level/2, tenths), an
    /// a-value uniform in [a0,a1] (hundredths), and a b-value composed from
    /// per-byte deltas between b0 and b1.
    #[allow(clippy::too_many_arguments)]
    pub fn cast(
        &mut self,
        def_name: &str,
        amount: i32,
        x: f32,
        y: f32,
        level: i32,
        a0: f32,
        b0: u32,
        a1: f32,
        b1: u32,
        layer: ParticleLayer,
        attach_origin: Option<(i32, i32)>,
    ) -> bool {
        // safety (C4Particles.cpp:426)
        if self.get_def(def_name).is_none() {
            return false;
        }
        // get range for a and b (C4Particles.cpp:428-433)
        let (mut i_a0, mut i_a1) = ((a0 * 100.0) as i32, (a1 * 100.0) as i32);
        if i_a1 < i_a0 {
            std::mem::swap(&mut i_a0, &mut i_a1);
        }
        let i_ad = i_a1 - i_a0 + 1;
        let (b0, b1) = if b1 < b0 { (b1, b0) } else { (b0, b1) };
        let db = b1 - b0;
        let (db1, db2, db3, db4) = (
            (db >> 24) as u8,
            (db >> 16) as u8,
            (db >> 8) as u8,
            db as u8,
        );
        // create them (C4Particles.cpp:435-440)
        for _ in 0..amount {
            let xdir = (self.safe_rng.random(level + 1) - level / 2) as f32 / 10.0;
            let ydir = (self.safe_rng.random(level + 1) - level / 2) as f32 / 10.0;
            let a = (i_a0 + self.safe_rng.random(i_ad)) as f32 / 100.0;
            let b = b0
                .wrapping_add((self.safe_rng.random(db1 as i32) as u32) << 24)
                .wrapping_add((self.safe_rng.random(db2 as i32) as u32) << 16)
                .wrapping_add((self.safe_rng.random(db3 as i32) as u32) << 8)
                .wrapping_add(self.safe_rng.random(db4 as i32) as u32);
            self.create(
                def_name,
                x,
                y,
                xdir,
                ydir,
                a,
                b as i32,
                layer.clone(),
                attach_origin,
            );
        }
        true
    }

    /// `C4ParticleSystem::IsFireParticleLoaded` (C4Particles.h:214):
    /// `pFire1 && pFire2`. `SetDefParticles` (C4Particles.cpp:475-492) only
    /// resolves those two when `Config.Graphics.FireParticles` is set, so the
    /// per-client switch folds into the same answer.
    pub fn is_fire_particle_loaded(&self) -> bool {
        self.fire_particles
            && self.get_def(FIRE_DEF_NAME).is_some()
            && self.get_def(FIRE2_DEF_NAME).is_some()
    }

    /// The particle half of `FnFxFireTimer` (C4Effect.cpp:660-769): a double
    /// set of particles per execution, the first quarter the normal `Fire`
    /// def and the remaining three quarters the additive `Fire2`, dealt to
    /// the object's back list three times out of four. Returns how many were
    /// actually created — `create`'s MaxCount arm can refuse some.
    ///
    /// The loop lives here rather than beside the ported effect body because
    /// C++ draws it from the process-global `SafeRandom` stream that
    /// `C4ParticleSystem::Create` also consumes (C4Particles.cpp:394);
    /// keeping both on one `SafeRng` preserves that interleaving. C++ is
    /// explicit that this stream is deliberately unsynchronized
    /// (C4Effect.cpp:725-726), so it must never be the synced `Random`.
    pub fn create_object_fire(&mut self, emission: &ObjectFireEmission) -> i32 {
        // some constant effect parameters for this object (C4Effect.cpp:679-687)
        let width = emission.def_width.max(1);
        let height = emission.def_height;
        let mut y_off = height / 2 - emission.fire_top;
        const BASE_PARTICLE_SIZE: i32 = 30;
        const PARTICLE_SIZE_DIFF: i32 = 10;
        const REL_PARTICLE_SIZE: i32 = 12;

        // get remainign size (%) (C4Effect.cpp:693-697)
        // C++ is plain int32_t throughout this block (C4Effect.cpp:694,716,
        // 719,727,749-751,757-758). An Oversize object's Con is unbounded
        // above FullCon, so these products can overflow; C++ wraps and keeps
        // drawing a garbage particle, and this stream is presentation-only,
        // so the port wraps too rather than trapping on a script-reachable
        // path.
        let con = (100i32.wrapping_mul(emission.con) / crate::FULL_CON).max(1);
        let mut wdt_con = con;
        // fixed width for not-stretched-objects
        if !emission.growth_type && wdt_con < 100 {
            wdt_con = 100;
        }

        // regard non-center object offsets (C4Effect.cpp:699-701)
        let x = emission.x + emission.shape_x + emission.shape_width / 2;
        let y = emission.y + emission.shape_y + emission.shape_height / 2;

        // apply rotation (C4Effect.cpp:703-713)
        let mut rot = [1.0f32, 0.0, 0.0, 1.0];
        if emission.rotation != 0 && emission.rotateable {
            // `cosf(static_cast<float>(r * pi_v<float> / 180.0))`: the
            // multiply is float, the divide widens to double, and the cast
            // narrows back before the call.
            let radians = ((emission.rotation as f32 * std::f32::consts::PI) as f64 / 180.0) as f32;
            rot[0] = radians.cos();
            rot[1] = -radians.sin();
            rot[2] = -rot[1];
            rot[3] = rot[0];
            // rotated objects usually better burn from the center
            if y_off > 0 {
                y_off = 0;
            }
        }

        // Adjust particle number by con (C4Effect.cpp:715-716)
        let count = (f64::from(width.wrapping_mul(height)).sqrt() / 4.0) as i32;
        let count = (count.wrapping_mul(wdt_con) / 100).max(2);

        // calc base for particle size parameter (C4Effect.cpp:718-719)
        let size_base = ((f64::from(width.wrapping_mul(height)).sqrt()
            * f64::from(con.wrapping_add(20))
            / 120.0)
            .sqrt()
            * f64::from(REL_PARTICLE_SIZE)) as i32;

        let attach_origin = Some((emission.x, emission.y));
        let mut created = 0;
        for index in 0..count * 2 {
            // calc actual size to be used in this frame (C4Effect.cpp:724-727)
            let size = self
                .safe_rng
                .random(PARTICLE_SIZE_DIFF + 1)
                .wrapping_add(BASE_PARTICLE_SIZE - PARTICLE_SIZE_DIFF / 2 - 1)
                .wrapping_add(size_base);

            // get particle target list (C4Effect.cpp:729-730)
            let layer = if self.safe_rng.random(4) != 0 {
                ParticleLayer::ObjectBack(emission.object)
            } else {
                ParticleLayer::ObjectFront(emission.object)
            };

            // get particle def and color (C4Effect.cpp:732-744)
            let (def_name, mut color) = if index < count / 2 {
                (
                    FIRE_DEF_NAME,
                    0x3200_4000u32.wrapping_add((self.safe_rng.random(59) as u32 + 196) << 16),
                )
            } else {
                (FIRE2_DEF_NAME, 0x00ff_ffffu32)
            };
            if emission.fire_mode == crate::C4FX_FIRE_MODE_OBJECT {
                color = color.wrapping_add(0x6200_0000);
            }

            // get particle creation pos... (C4Effect.cpp:746-751)
            let rand_x = self.safe_rng.random(width + 1) - width / 2 - 1;
            let px = rand_x.wrapping_mul(wdt_con) / 100;
            let mut py = y_off.wrapping_mul(con) / 100;
            if emission.fire_mode == crate::C4FX_FIRE_MODE_LIVING_VEG {
                // parable form particle pos on livings
                py = py.wrapping_sub(px.wrapping_mul(px).wrapping_mul(100) / width / wdt_con);
            }

            // ...and movement speed (C4Effect.cpp:753-766)
            let (x_dir, y_dir) = if emission.fire_mode != crate::C4FX_FIRE_MODE_OBJECT {
                // ...for normal fire proc
                (
                    (rand_x.wrapping_mul(con) / 400)
                        .wrapping_sub(px / 3)
                        .wrapping_sub((emission.xdir * 3.0) as i32),
                    (-self
                        .safe_rng
                        .random(15i32.wrapping_add(height.wrapping_mul(con) / 300))
                        - 1)
                    .wrapping_sub((emission.ydir * 3.0) as i32),
                )
            } else {
                // ...for objects
                let x_dir = -((emission.xdir * 3.0) as i32);
                let mut y_dir = -((emission.ydir * 3.0) as i32);
                if y_dir == 0 {
                    y_dir = -self.safe_rng.random(13 + height / 4) - 1;
                }
                (x_dir, y_dir)
            };

            // OK; create it! (C4Effect.cpp:768-769)
            if self.create(
                def_name,
                x as f32 + rot[0] * px as f32 + rot[1] * py as f32,
                y as f32 + rot[2] * px as f32 + rot[3] * py as f32,
                x_dir as f32 / 10.0,
                y_dir as f32 / 10.0,
                size as f32 / 10.0,
                color as i32,
                layer,
                attach_origin,
            ) {
                created += 1;
            }
        }
        created
    }

    /// `C4ParticleSystem::Push` (C4Particles.cpp:494-519): add a velocity
    /// delta to every live particle, optionally filtered by def. Returns the
    /// number of particles pushed.
    pub fn push(&mut self, def_name: Option<&str>, dxdir: f32, dydir: f32) -> i32 {
        self.particles
            .iter_mut()
            .filter(|particle| def_name.is_none_or(|name| particle.def_name == name))
            .map(|particle| {
                particle.xdir += dxdir;
                particle.ydir += dydir;
            })
            .count() as i32
    }

    /// `C4ParticleList::Exec` (C4Particles.cpp:250-267) for one layer: run
    /// each particle's exec proc; a false return removes the particle and
    /// releases its def count. C++ iterates newest-first (MoveList prepends),
    /// so we walk the Vec in reverse insertion order.
    pub fn exec_layer(
        &mut self,
        layer: &ParticleLayer,
        target: Option<ParticleTarget>,
        env: &ParticleEnv,
    ) {
        let mut index = self.particles.len();
        while index > 0 {
            index -= 1;
            if self.particles[index].layer != *layer {
                continue;
            }
            let def_index = self
                .defs
                .iter()
                .position(|def| def.core.name == self.particles[index].def_name);
            let Some(def_index) = def_index else {
                // No def (legacy snapshot path): keep the particle untouched.
                continue;
            };
            let mut particle = self.particles[index].clone();
            let exec_proc = self.defs[def_index].exec_proc;
            let keep = self.run_exec_proc(exec_proc, def_index, &mut particle, target, env);
            if keep {
                self.particles[index] = particle;
            } else {
                self.defs[def_index].count -= 1;
                self.particles.remove(index);
            }
        }
    }

    /// Exec proc dispatch (C4Particles.h:46: returns whether the particle
    /// survives — C++ returns "whether particle died" inverted at call site;
    /// here true = keep, matching the call sites' use).
    fn run_exec_proc(
        &mut self,
        proc: ParticleProc,
        def_index: usize,
        particle: &mut Particle,
        target: Option<ParticleTarget>,
        env: &ParticleEnv,
    ) -> bool {
        match proc {
            ParticleProc::StdExec => self.fx_std_exec(def_index, particle, target, env),
            ParticleProc::SmokeExec => self.fx_smoke_exec(def_index, particle, env),
            ParticleProc::Bounce => {
                particle.xdir = -particle.xdir;
                particle.ydir = -particle.ydir;
                true
            }
            ParticleProc::BounceY => {
                particle.ydir = -particle.ydir;
                true
            }
            ParticleProc::Stop => {
                particle.xdir = 0.0;
                particle.ydir = 0.0;
                true
            }
            ParticleProc::Die => false,
            // Init procs run as exec keep the particle alive untouched.
            ParticleProc::StdInit | ParticleProc::SmokeInit => true,
        }
    }

    /// fxStdExec (C4Particles.cpp:614-697).
    fn fx_std_exec(
        &mut self,
        def_index: usize,
        particle: &mut Particle,
        target: Option<ParticleTarget>,
        env: &ParticleEnv,
    ) -> bool {
        let def = self.defs[def_index].clone();
        let mut dx = particle.x;
        let mut dy = particle.y;
        let mut dxdir = particle.xdir;
        let mut dydir = particle.ydir;
        // rel. position & movement (C4Particles.cpp:619-625)
        if def.core.attach != 0 {
            if let Some(target) = target {
                dx += target.x as f32;
                dy += target.y as f32;
                dxdir += fixtof(target.xdir);
                dydir += fixtof(target.ydir);
            }
        }
        // move (C4Particles.cpp:628-645)
        if particle.xdir != 0.0 || particle.ydir != 0.0 {
            let vertex_hit = def.core.vertex_count != 0
                && (env.solid)(
                    (dx + particle.xdir) as i32,
                    (dy + particle.ydir + def.core.vertex_y as f32 * particle.a / 100.0) as i32,
                );
            if vertex_hit {
                // collision (C4Particles.cpp:632-634)
                if let Some(collision_proc) = def.collision_proc {
                    if !self.run_exec_proc(collision_proc, def_index, particle, target, env) {
                        return false;
                    }
                }
            } else if def.core.r_by_v != 2 {
                particle.x += particle.xdir;
                particle.y += particle.ydir;
            }
            // RByV == 2: velocity is rotation-only, no movement
        }
        // apply gravity (C4Particles.cpp:647)
        if def.core.gravity_acc != 0 {
            particle.ydir += fixtof(C4Fixed::from_raw(
                env.gravity.val().wrapping_mul(def.core.gravity_acc),
            )) / 100.0;
        }
        // apply WindDrift (C4Particles.cpp:649-660)
        if def.core.wind_drift != 0 && !(env.solid)(dx as i32, dy as i32) {
            let wind = (env.wind)(dx as i32, dy as i32);
            let txdir = wind as f32 / 15.0;
            let tydir = 0.0f32;
            let wind_drift = (def.core.wind_drift - 20).max(0) as f32;
            particle.xdir += ((txdir - dxdir) * wind_drift) / 800.0;
            particle.ydir += ((tydir - dydir) * wind_drift) / 800.0;
        }
        // fade out (C4Particles.cpp:662-671)
        let mut fade = def.core.alpha_fade;
        if fade < 0 {
            fade = i32::from(env.frame_counter % -fade == 0);
        }
        if fade != 0 {
            let clr = particle.b as u32;
            let alpha = (clr >> 24) as i32 + def.core.alpha_fade;
            if alpha >= 0xff {
                return false;
            }
            particle.b = ((clr & 0xffffff) | ((alpha as u32) << 24)) as i32;
        }
        // if delay is given, advance lifetime (C4Particles.cpp:673-691)
        if def.core.delay != 0 {
            if particle.life < 0 {
                // decay: post-decrement compare (C4Particles.cpp:678)
                let keep = particle.life >= -def.core.fade_out_len * def.core.fade_out_delay;
                particle.life -= 1;
                return keep;
            }
            particle.life += 1;
            let phase = particle.life / def.core.delay;
            let length = def.length - def.core.reverse;
            if phase >= length * def.core.repeats + def.core.reverse {
                if def.core.fade_out_len == 0 {
                    return false;
                }
                particle.life = -1;
            }
            return true;
        }
        // outside landscape range? (C4Particles.cpp:692-696)
        let mut keep = if dxdir > 0.0 {
            dx - particle.a < env.back_wdt as f32
        } else {
            dx + particle.a > 0.0
        };
        keep = keep
            && if dydir > 0.0 {
                dy - particle.a < env.back_hgt as f32
            } else {
                dy + particle.a > def.core.y_off as f32
            };
        keep
    }

    /// fxSmokeExec (C4Particles.cpp:537-576).
    fn fx_smoke_exec(
        &mut self,
        _def_index: usize,
        particle: &mut Particle,
        env: &ParticleEnv,
    ) -> bool {
        // lifetime: pre-decrement, die at exactly 0 (C4Particles.cpp:540)
        particle.life -= 1;
        if particle.life == 0 {
            return false;
        }
        let building = particle.life & 0x7fff0000 != 0;
        if building {
            // decrease init-time, increase color value (C4Particles.cpp:543-552)
            particle.life -= 0x010000;
            particle.b = particle.b.wrapping_sub(0x10000000_u32 as i32);
            if particle.life & 0x7fff0000 == 0 {
                particle.b = (particle.b & 0xffffff) | ((255 - particle.life) << 24);
            }
        }
        // color change (C4Particles.cpp:553-555): lighten RGB by 1, alpha+1
        let clr = particle.b as u32;
        let lightened = lighten_clr_by(clr, 1);
        particle.b = ((lightened & 0xffffff) | (((clr >> 24) + 1).min(255) << 24)) as i32;
        // wind to float (C4Particles.cpp:556-562)
        if particle.b % 12 == 0 || building {
            particle.xdir = (0.025f32 * (env.wind)(particle.x as i32, particle.y as i32) as f32)
                .clamp(-2.0, 2.0);
            particle.xdir += 0.1 * self.safe_rng.random(41) as f32 - 2.0;
        }
        // float (C4Particles.cpp:563-570)
        if (env.solid)(particle.x as i32, (particle.y - particle.a) as i32) {
            // if stuck, decay; otherwise, move down
            if !(env.solid)(particle.x as i32, particle.y as i32) {
                particle.y += 0.4;
            } else {
                particle.a -= 2.0;
            }
        } else {
            particle.y -= 1.0;
        }
        particle.x += particle.xdir;
        // increase in size (C4Particles.cpp:572-573)
        particle.a *= 1.01;
        true
    }

    /// Init proc dispatch (C4Particles.h:45: init returns whether the
    /// particle could be created).
    fn run_init_proc(
        &mut self,
        proc: ParticleProc,
        def_index: usize,
        particle: &mut Particle,
    ) -> bool {
        match proc {
            ParticleProc::StdInit => {
                // fxStdInit (C4Particles.cpp:600-612)
                let def = &self.defs[def_index];
                particle.life = if def.core.delay == 0 {
                    self.safe_rng.random(def.length)
                } else {
                    0
                };
                if particle.b == 0 {
                    particle.b = 0xffffff;
                }
                true
            }
            ParticleProc::SmokeInit => {
                // fxSmokeInit (C4Particles.cpp:521-535)
                let def = &self.defs[def_index];
                particle.life = def.core.min_lifetime;
                let spread = def.core.max_lifetime - def.core.min_lifetime;
                if spread != 0 {
                    particle.life += self.safe_rng.random(spread);
                }
                // use high-word of life to store init-status
                particle.life |= (particle.life / 17) << 16;
                // set kind in ydir — int division: only SafeRandom(300)==299
                // contributes the +1 ("set last kind reeeaaally seldom")
                particle.ydir =
                    self.safe_rng.random(15) as f32 + (self.safe_rng.random(300) / 299) as f32;
                if particle.b == 0 {
                    particle.b = 0xff4b4b4b_u32 as i32;
                } else {
                    particle.b = (particle.b as u32 | 0xff000000) as i32;
                }
                true
            }
            // Non-init procs used as InitFn behave like their exec/collision
            // counterparts in C++ (raw function pointers); none of them reject
            // creation except Die (returns false, C4Particles.cpp:721-725).
            ParticleProc::Die => false,
            _ => true,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_core_is_the_resource_core_type() {
        let resource_core = clonk_resources::ParticleDefinitionCore::default();
        let _: ParticleDefCore = resource_core;
    }

    /// Every live newgfx particle is projected into `SimulationSnapshot`.
    ///
    /// This is the contract clonk-org/clonk-rs#290 turns on. That issue asks
    /// whether newgfx state could become GPU-authoritative, and observes that
    /// it could not "also feed a byte-identical per-tick `SimulationSnapshot`
    /// … unless that contract changes or the CPU shadow-simulates". So the
    /// first thing to establish is whether the projection exists at all —
    /// stated in the issue, pinned nowhere.
    ///
    /// It does: `Engine::snapshot` appends `particle_system.particles()` after
    /// the object particles and the PXS slots. A change that moved this state
    /// to the GPU without a readback would have to delete or hollow out this
    /// projection, which fails here rather than silently emptying a field the
    /// frontend, recordings and dev replay all read.
    #[test]
    fn every_live_newgfx_particle_is_projected_into_the_snapshot() {
        let mut engine = crate::Engine::new();
        let particle = Particle {
            def_name: "Smoke".to_string(),
            x: 12.5,
            y: -3.25,
            xdir: 0.5,
            ydir: -1.5,
            life: 42,
            a: 7.5,
            b: 3,
            layer: ParticleLayer::Global,
        };
        engine.particle_system.restore_particle(particle.clone());

        let projected = engine.snapshot().particles;
        let found = projected
            .iter()
            .find(|entry| entry.definition_id == "Smoke")
            .expect("the live particle reaches the snapshot");

        // The values travel, not just the count: a projection that dropped to
        // a placeholder would still satisfy a length check.
        assert_eq!(found.position.x, particle.x);
        assert_eq!(found.position.y, particle.y);
        assert_eq!(found.life, particle.life);
    }

    // C4Game.cpp:2369-2394 — ReloadParticle's exact refusal and failure policy.
    #[test]
    fn reload_particle_refuses_network_and_clears_everything_on_failure() {
        let mut engine = crate::Engine::new();
        engine
            .particle_system
            .register_def(std_core("Smoke"), 4, 1.0)
            .expect("register a simulation-only particle def");

        // The network refusal is the FIRST line — before the name check and
        // before any lookup, so nothing is touched.
        assert!(!engine.reload_particle("Smoke", true));
        assert!(engine.particle_system.get_def("Smoke").is_some());

        // An unknown name reloads nothing and clears nothing: a plain false,
        // not a failure, so the known def survives it.
        assert!(!engine.reload_particle("NoSuchParticle", false));
        assert!(engine.particle_system.get_def("Smoke").is_some());

        // A def that exists but cannot reload takes the destructive arm:
        // `C4ParticleDef::Reload` refuses without a filename
        // (C4Particles.cpp:197), and `ReloadParticle` treats that like any
        // other failure — every particle in the system goes, then the def.
        assert!(!engine.reload_particle("Smoke", false));
        assert!(
            engine.particle_system.get_def("Smoke").is_none(),
            "a failed reload removes the definition"
        );
    }

    fn std_core(name: &str) -> ParticleDefCore {
        ParticleDefCore {
            name: name.to_string(),
            init_fn: "StdInit".to_string(),
            exec_fn: "StdExec".to_string(),
            draw_fn: "Std".to_string(),
            ..ParticleDefCore::default()
        }
    }

    #[test]
    fn def_registry_lookup_overload_and_proc_resolution_match_cpp() {
        // GetDef seeks the def list by name (C4Particles.cpp:465-473); loading
        // a def whose name already exists deletes the older def ("particle
        // overloading", C4Particles.cpp:178-187); unresolvable init/exec procs
        // abort the load (C4Particles.cpp:157-167).
        let mut system = ParticleSystem::default();
        assert!(system.get_def("Smoke").is_none());

        system
            .register_def(std_core("Smoke"), 4, 1.0)
            .expect("valid def registers");
        assert!(system.get_def("Smoke").is_some());
        assert!(system.get_def("Blast").is_none());

        // Overload: second "Smoke" def replaces the first.
        let mut overload = std_core("Smoke");
        overload.gravity_acc = 77;
        system
            .register_def(overload, 4, 1.0)
            .expect("overload registers");
        assert_eq!(system.def_count(), 1);
        assert_eq!(system.get_def("Smoke").unwrap().core.gravity_acc, 77);

        // Unknown exec proc → load fails, def not registered.
        let mut bad = std_core("Bad");
        bad.exec_fn = "NoSuchProc".to_string();
        assert!(system.register_def(bad, 4, 1.0).is_err());
        assert!(system.get_def("Bad").is_none());

        // Draw proc resolution happens after the generic proc lookups but
        // before overload deletion. A bad newer same-name def must leave the
        // previously loaded winner intact.
        let mut bad_draw = std_core("Smoke");
        bad_draw.gravity_acc = 99;
        bad_draw.draw_fn = "NoSuchDrawProc".to_string();
        assert_eq!(
            system.register_def(bad_draw, 4, 1.0),
            Err(ParticleDefError::UnknownDrawProc(
                "NoSuchDrawProc".to_string()
            ))
        );
        assert_eq!(system.get_def("Smoke").unwrap().core.gravity_acc, 77);
        assert_eq!(
            system.get_def("Smoke").unwrap().draw_proc,
            ParticleDrawProc::Std
        );
    }

    #[test]
    fn mutable_definition_access_refreshes_name_and_reloadability_snapshots() {
        // GetDef and Reload inspect the live linked definition, so cached host
        // snapshots must follow its current Name/Filename fields
        // (C4Particles.cpp:197-205,465-473).
        let mut system = ParticleSystem::default();
        system.register_def(std_core("Old"), 1, 1.0).unwrap();

        let def = system.get_def_mut("Old").expect("definition exists");
        def.core.name = "New".to_owned();
        def.source_path = Some(std::path::PathBuf::from("New.c4p"));

        assert_eq!(system.def_names(), HashSet::from(["New".to_owned()]));
        assert_eq!(
            system.reloadable_def_names(),
            HashSet::from(["New".to_owned()])
        );
        assert_eq!(
            system.shared_def_names().as_ref(),
            &HashSet::from(["New".to_owned()])
        );
        assert_eq!(
            system.shared_reloadable_def_names().as_ref(),
            &HashSet::from(["New".to_owned()])
        );
    }

    #[test]
    fn def_registry_exposes_native_order_and_appends_successful_overloads() {
        let mut system = ParticleSystem::default();
        system.register_def(std_core("First"), 1, 1.0).unwrap();
        system.register_def(std_core("Second"), 1, 1.0).unwrap();

        let mut first_overload = std_core("First");
        first_overload.gravity_acc = 42;
        system.register_def(first_overload, 1, 1.0).unwrap();

        assert_eq!(
            system
                .definitions()
                .iter()
                .map(|def| def.core.name.as_str())
                .collect::<Vec<_>>(),
            ["Second", "First"]
        );
        assert_eq!(system.get_def("First").unwrap().core.gravity_acc, 42);

        // Native overload matching is exact-case.
        system.register_def(std_core("first"), 1, 1.0).unwrap();
        assert_eq!(
            system
                .definitions()
                .iter()
                .map(|def| def.core.name.as_str())
                .collect::<Vec<_>>(),
            ["Second", "First", "first"]
        );
    }

    #[test]
    fn resource_registration_retains_image_facet_and_raw_phase_geometry() {
        let resource = clonk_resources::ParticleDefinition {
            core: clonk_resources::ParticleDefinitionCore {
                name: "Sheet".to_string(),
                init_fn: "StdInit".to_string(),
                exec_fn: "StdExec".to_string(),
                draw_fn: "Std".to_string(),
                fade_out_len: 1,
                ..clonk_resources::ParticleDefinitionCore::default()
            },
            image: GraphicsImage::new(96, 64, vec![0xff; 96 * 64 * 4]),
            facet: clonk_resources::ParticleFacet {
                x: 4,
                y: 8,
                width: 32,
                height: 16,
                target_x: -16,
                target_y: -8,
            },
        };
        let mut system = ParticleSystem::default();
        system.register_resource(&resource).unwrap();

        let def = system.get_def("Sheet").unwrap();
        assert_eq!(def.length, 2, "FadeOutLen adjusts the live phase length");
        assert_eq!(def.aspect.to_bits(), 2.0f32.to_bits());
        assert_eq!(def.draw_proc, ParticleDrawProc::Std);
        let graphics = def.graphics.as_ref().expect("resource graphics retained");
        assert_eq!(graphics.image.width(), 96);
        assert_eq!(graphics.image.height(), 64);
        assert_eq!(
            graphics.facet,
            ParticleGraphicsFacet {
                x: 4,
                y: 8,
                width: 32,
                height: 16,
                target_x: -16,
                target_y: -8,
            }
        );
        assert_eq!(
            graphics.phases,
            ParticleSourcePhaseGeometry {
                width: 32,
                height: 16,
                columns: 3,
                rows: 4,
            }
        );
    }

    #[test]
    fn def_load_adjusts_length_fadeout_and_reverse_like_cpp() {
        // C4Particles.cpp:148-154: Length = max(Length - FadeOutLen, 1) with
        // FadeOutDelay defaulting to 1; a single-phase def forces Reverse = 0.
        let mut system = ParticleSystem::default();
        let mut core = std_core("Fade");
        core.fade_out_len = 6;
        core.reverse = 1;
        system.register_def(core, 7, 2.0).expect("registers");
        let def = system.get_def("Fade").unwrap();
        assert_eq!(def.length, 1);
        assert_eq!(def.core.fade_out_delay, 1);
        assert_eq!(def.core.reverse, 0, "single phase disallows reverse");
        assert_eq!(def.aspect.to_bits(), 2.0f32.to_bits());
    }

    #[test]
    fn create_inits_particle_counts_and_respects_max_count_like_cpp() {
        // C4ParticleSystem::Create (C4Particles.cpp:378-419): the def's
        // MaxCount is scaled by (SmokeLevel+20)/150 (line 389; SmokeLevel
        // defaults to 200, C4Config.cpp:452); no room → no particle; fxStdInit
        // (C4Particles.cpp:600-612) sets life = SafeRandom(Length) when Delay
        // is 0 and defaults b to 0xffffff; created particles bump def Count.
        let mut system = ParticleSystem::default();
        let mut core = std_core("Spark");
        core.max_count = 1; // scaled: 1*220/150 = 1
        system.register_def(core, 8, 1.0).expect("registers");

        assert!(system.create(
            "Spark",
            10.0,
            20.0,
            1.5,
            -0.5,
            3.0,
            0,
            crate::ParticleLayer::Global,
            None,
        ));
        assert_eq!(system.get_def("Spark").unwrap().count, 1);
        let particle = system.particles().last().unwrap();
        assert_eq!(particle.x.to_bits(), 10.0f32.to_bits());
        assert_eq!(particle.y.to_bits(), 20.0f32.to_bits());
        assert_eq!(particle.b, 0xffffff, "fxStdInit defaults b to 0xffffff");
        assert!(
            (0..8).contains(&particle.life),
            "fxStdInit: life = SafeRandom(Length)"
        );

        // Room exhausted (count == scaled MaxCount) → not created.
        assert!(!system.create(
            "Spark",
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0,
            crate::ParticleLayer::Global,
            None,
        ));
        assert_eq!(system.get_def("Spark").unwrap().count, 1);

        // Unknown def → not created (C4Particles.cpp:385).
        assert!(!system.create(
            "Nope",
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0,
            crate::ParticleLayer::Global,
            None,
        ));
    }

    #[test]
    fn create_applies_attach_offset_like_cpp() {
        // C4Particles.cpp:404-408: when the def has Attach set and an object
        // is supplied, the particle position becomes relative to the object.
        let mut system = ParticleSystem::default();
        let mut core = std_core("Trail");
        core.attach = 1;
        system.register_def(core, 1, 1.0).expect("registers");
        assert!(system.create(
            "Trail",
            100.0,
            60.0,
            0.0,
            0.0,
            0.0,
            7,
            crate::ParticleLayer::ObjectFront(crate::ObjectId::new(5)),
            Some((30, 40)),
        ));
        let particle = system.particles().last().unwrap();
        assert_eq!(particle.x.to_bits(), 70.0f32.to_bits());
        assert_eq!(particle.y.to_bits(), 20.0f32.to_bits());
        assert_eq!(particle.b, 7, "non-zero b is preserved");
    }

    #[test]
    fn cast_consumes_safe_random_draws_in_cpp_order() {
        // C4ParticleSystem::Cast (C4Particles.cpp:421-443): per particle the
        // draws are xdir, ydir, a, then the four b-delta bytes — followed by
        // fxStdInit's life draw inside Create. a-range is scaled ×100 and
        // swapped if needed; the b range is split into per-byte deltas.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Mist"), 10, 1.0)
            .expect("registers");
        system.safe_rng = SafeRng::new(99);

        // Independent mirror of the expected draw sequence.
        let mut mirror = SafeRng::new(99);
        let mut expected = Vec::new();
        let (a0, a1) = (1.0f32, 2.0f32);
        let (b0, b1) = (0x11223344u32, 0x55667788u32);
        let (i_a0, i_a1) = ((a0 * 100.0) as i32, (a1 * 100.0) as i32);
        let i_ad = i_a1 - i_a0 + 1;
        let db = b1 - b0;
        let (db1, db2, db3, db4) = (
            (db >> 24) as u8,
            (db >> 16) as u8,
            (db >> 8) as u8,
            db as u8,
        );
        for _ in 0..2 {
            let xdir = (mirror.random(20 + 1) - 10) as f32 / 10.0;
            let ydir = (mirror.random(20 + 1) - 10) as f32 / 10.0;
            let a = (i_a0 + mirror.random(i_ad)) as f32 / 100.0;
            let b = b0
                .wrapping_add((mirror.random(db1 as i32) as u32) << 24)
                .wrapping_add((mirror.random(db2 as i32) as u32) << 16)
                .wrapping_add((mirror.random(db3 as i32) as u32) << 8)
                .wrapping_add(mirror.random(db4 as i32) as u32);
            let life = mirror.random(10); // fxStdInit inside Create
            expected.push((xdir, ydir, a, b as i32, life));
        }

        assert!(system.cast(
            "Mist",
            2,
            5.0,
            6.0,
            20,
            a0,
            b0,
            a1,
            b1,
            crate::ParticleLayer::Global,
            None,
        ));
        assert_eq!(system.particles().len(), 2);
        for (particle, (xdir, ydir, a, b, life)) in system.particles().iter().zip(expected.iter()) {
            assert_eq!(particle.x.to_bits(), 5.0f32.to_bits());
            assert_eq!(particle.y.to_bits(), 6.0f32.to_bits());
            assert_eq!(particle.xdir.to_bits(), xdir.to_bits());
            assert_eq!(particle.ydir.to_bits(), ydir.to_bits());
            assert_eq!(particle.a.to_bits(), a.to_bits());
            assert_eq!(particle.b, *b);
            assert_eq!(particle.life, *life);
        }

        // Unknown def → false (C4Particles.cpp:426).
        assert!(!system.cast(
            "Nope",
            1,
            0.0,
            0.0,
            0,
            0.0,
            0,
            0.0,
            0,
            crate::ParticleLayer::Global,
            None,
        ));
    }

    #[test]
    fn object_fire_emits_a_double_set_split_one_quarter_fire_and_three_quarters_fire2() {
        // FnFxFireTimer's emitter loop (C4Effect.cpp:721-770) runs
        // `iCount * 2` times; `i < iCount / 2` picks pFire1 ("Fire"), the
        // rest pFire2 ("Fire2"). For a 16x16 full-con object iCount is
        // `int(sqrt(16*16) / 4)` = 4, so 8 particles: 2 Fire, 6 Fire2.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");

        let created = system.create_object_fire(&ObjectFireEmission {
            object: crate::ObjectId::new(7),
            fire_mode: crate::C4FX_FIRE_MODE_STRUCT_VEH,
            def_width: 16,
            def_height: 16,
            fire_top: 0,
            con: crate::FULL_CON,
            growth_type: false,
            x: 100,
            y: 200,
            shape_x: -8,
            shape_y: -8,
            shape_width: 16,
            shape_height: 16,
            rotation: 0,
            rotateable: false,
            xdir: 0.0,
            ydir: 0.0,
        });

        assert_eq!(created, 8, "iCount * 2 particles per execution");
        let names: Vec<&str> = system
            .particles()
            .iter()
            .map(|particle| particle.def_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Fire", "Fire", "Fire2", "Fire2", "Fire2", "Fire2", "Fire2", "Fire2"],
        );
    }

    #[test]
    fn object_fire_consumes_safe_random_draws_in_cpp_order() {
        // C4Effect.cpp:724-758 draws, per particle: size, target list,
        // then the Fire-only color draw, then the x offset, then (outside
        // C4Fx_FireMode_Object) the upward speed — followed by fxStdInit's
        // life draw inside Create.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        system.safe_rng = SafeRng::new(4242);

        // Independent mirror of the expected draw sequence. For a 16x16
        // full-con object: count = 4, wdt_con = con = 100, y_off = 8, and
        // size_base = int(sqrt(sqrt(256) * 120 / 120) * 12) = 48.
        let mut mirror = SafeRng::new(4242);
        let mut expected = Vec::new();
        for index in 0..8 {
            let size = mirror.random(11) + 30 - 5 - 1 + 48;
            let layer = if mirror.random(4) != 0 {
                ParticleLayer::ObjectBack(crate::ObjectId::new(7))
            } else {
                ParticleLayer::ObjectFront(crate::ObjectId::new(7))
            };
            let color = if index < 2 {
                0x3200_4000u32 + ((mirror.random(59) as u32 + 196) << 16)
            } else {
                0x00ff_ffff
            };
            let rand_x = mirror.random(17) - 8 - 1;
            let px = rand_x;
            let py = 8;
            let x_dir = rand_x * 100 / 400 - px / 3;
            let y_dir = -mirror.random(15 + 16 * 100 / 300) - 1;
            let life = mirror.random(10); // fxStdInit inside Create
            expected.push((size, layer, color, px, py, x_dir, y_dir, life));
        }

        assert_eq!(
            system.create_object_fire(&ObjectFireEmission {
                object: crate::ObjectId::new(7),
                fire_mode: crate::C4FX_FIRE_MODE_STRUCT_VEH,
                def_width: 16,
                def_height: 16,
                fire_top: 0,
                con: crate::FULL_CON,
                growth_type: false,
                x: 100,
                y: 200,
                shape_x: -8,
                shape_y: -8,
                shape_width: 16,
                shape_height: 16,
                rotation: 0,
                rotateable: false,
                xdir: 0.0,
                ydir: 0.0,
            }),
            8,
        );

        for (particle, (size, layer, color, px, py, x_dir, y_dir, life)) in
            system.particles().iter().zip(expected.iter())
        {
            assert_eq!(particle.x.to_bits(), (100.0 + *px as f32).to_bits());
            assert_eq!(particle.y.to_bits(), (200.0 + *py as f32).to_bits());
            assert_eq!(particle.xdir.to_bits(), (*x_dir as f32 / 10.0).to_bits());
            assert_eq!(particle.ydir.to_bits(), (*y_dir as f32 / 10.0).to_bits());
            assert_eq!(particle.a.to_bits(), (*size as f32 / 10.0).to_bits());
            assert_eq!(particle.b, *color as i32);
            assert_eq!(particle.life, *life);
            assert_eq!(&particle.layer, layer);
        }
    }

    #[test]
    fn object_fire_mode_object_bumps_alpha_and_trails_the_objects_own_velocity() {
        // C4Effect.cpp:751 adds 0x62000000 to every color in
        // C4Fx_FireMode_Object, and :787-793 replaces the spread velocity
        // with the object's own, only drawing an upward speed when that
        // leaves iYDir at zero.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        system.safe_rng = SafeRng::new(11);

        let mut mirror = SafeRng::new(11);
        let mut expected = Vec::new();
        for index in 0..8 {
            let _size = mirror.random(11);
            let _layer = mirror.random(4);
            let color = if index < 2 {
                0x3200_4000u32 + ((mirror.random(59) as u32 + 196) << 16)
            } else {
                0x00ff_ffff
            } + 0x6200_0000;
            let _rand_x = mirror.random(17);
            let _life = mirror.random(10);
            expected.push(color);
        }

        assert_eq!(
            system.create_object_fire(&ObjectFireEmission {
                object: crate::ObjectId::new(3),
                fire_mode: crate::C4FX_FIRE_MODE_OBJECT,
                def_width: 16,
                def_height: 16,
                fire_top: 0,
                con: crate::FULL_CON,
                growth_type: false,
                x: 0,
                y: 0,
                shape_x: 0,
                shape_y: 0,
                shape_width: 0,
                shape_height: 0,
                rotation: 0,
                rotateable: false,
                xdir: 1.0,
                ydir: 2.0,
            }),
            8,
        );

        for (particle, color) in system.particles().iter().zip(expected.iter()) {
            assert_eq!(particle.b, *color as i32, "alpha-bumped object-mode color");
            // -int(fixtof(xdir) * 3) / 10, -int(fixtof(ydir) * 3) / 10
            assert_eq!(particle.xdir.to_bits(), (-3.0f32 / 10.0).to_bits());
            assert_eq!(particle.ydir.to_bits(), (-6.0f32 / 10.0).to_bits());
        }
    }

    #[test]
    fn object_fire_mode_object_draws_an_upward_speed_only_when_the_object_is_still() {
        // C4Effect.cpp:764-765: a resting object leaves iYDir at zero, which
        // is the one case that consumes SafeRandom(13 + iHeight / 4).
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        system.safe_rng = SafeRng::new(11);

        let mut mirror = SafeRng::new(11);
        let mut expected = Vec::new();
        for index in 0..8 {
            let _size = mirror.random(11);
            let _layer = mirror.random(4);
            if index < 2 {
                let _color = mirror.random(59);
            }
            let _rand_x = mirror.random(17);
            let y_dir = -mirror.random(13 + 16 / 4) - 1;
            let _life = mirror.random(10);
            expected.push(y_dir);
        }

        assert_eq!(
            system.create_object_fire(&ObjectFireEmission {
                object: crate::ObjectId::new(3),
                fire_mode: crate::C4FX_FIRE_MODE_OBJECT,
                def_width: 16,
                def_height: 16,
                fire_top: 0,
                con: crate::FULL_CON,
                growth_type: false,
                x: 0,
                y: 0,
                shape_x: 0,
                shape_y: 0,
                shape_width: 0,
                shape_height: 0,
                rotation: 0,
                rotateable: false,
                xdir: 0.0,
                ydir: 0.0,
            }),
            8,
        );

        for (particle, y_dir) in system.particles().iter().zip(expected.iter()) {
            assert_eq!(particle.xdir.to_bits(), 0.0f32.to_bits());
            assert_eq!(particle.ydir.to_bits(), (*y_dir as f32 / 10.0).to_bits());
        }
    }

    #[test]
    fn object_fire_mode_living_veg_bends_the_emission_row_into_a_parabola() {
        // C4Effect.cpp:750-751: livings emit along a downward parabola,
        // `iPy -= iPx * iPx * 100 / iWidth / iWdtCon`, so the row sags away
        // from the center by the square of the horizontal offset.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        system.safe_rng = SafeRng::new(2024);

        let mut mirror = SafeRng::new(2024);
        let mut expected = Vec::new();
        for index in 0..8 {
            let _size = mirror.random(11);
            let _layer = mirror.random(4);
            if index < 2 {
                let _color = mirror.random(59);
            }
            let rand_x = mirror.random(17) - 8 - 1;
            let px = rand_x;
            let py = 8 - px * px * 100 / 16 / 100;
            let _y_dir = mirror.random(15 + 16 * 100 / 300);
            let _life = mirror.random(10);
            expected.push((px, py));
        }

        assert_eq!(
            system.create_object_fire(&ObjectFireEmission {
                object: crate::ObjectId::new(9),
                fire_mode: crate::C4FX_FIRE_MODE_LIVING_VEG,
                def_width: 16,
                def_height: 16,
                fire_top: 0,
                con: crate::FULL_CON,
                growth_type: false,
                x: 100,
                y: 200,
                shape_x: 0,
                shape_y: 0,
                shape_width: 0,
                shape_height: 0,
                rotation: 0,
                rotateable: false,
                xdir: 0.0,
                ydir: 0.0,
            }),
            8,
        );

        assert!(
            expected.iter().any(|(px, _)| *px != 0),
            "the sample must include off-center offsets to exercise the term",
        );
        for (particle, (px, py)) in system.particles().iter().zip(expected.iter()) {
            assert_eq!(particle.x.to_bits(), (100.0 + *px as f32).to_bits());
            assert_eq!(particle.y.to_bits(), (200.0 + *py as f32).to_bits());
        }
    }

    #[test]
    fn object_fire_rotates_the_offsets_and_recenters_a_rotateable_object() {
        // C4Effect.cpp:703-713: a rotated Rotateable object spins the (px,py)
        // offset through the r matrix, and "rotated objects usually better
        // burn from the center" clamps a positive iYOff to zero.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        system.safe_rng = SafeRng::new(77);

        let radians = ((90.0f32 * std::f32::consts::PI) as f64 / 180.0) as f32;
        let rot = [radians.cos(), -radians.sin(), radians.sin(), radians.cos()];
        let mut mirror = SafeRng::new(77);
        let mut expected = Vec::new();
        for index in 0..8 {
            let _size = mirror.random(11);
            let _layer = mirror.random(4);
            if index < 2 {
                let _color = mirror.random(59);
            }
            let rand_x = mirror.random(17) - 8 - 1;
            let _y_dir = mirror.random(15 + 16 * 100 / 300);
            let _life = mirror.random(10);
            // iYOff would be 8, but rotation clamps it to 0.
            expected.push((rand_x, 0));
        }

        assert_eq!(
            system.create_object_fire(&ObjectFireEmission {
                object: crate::ObjectId::new(4),
                fire_mode: crate::C4FX_FIRE_MODE_STRUCT_VEH,
                def_width: 16,
                def_height: 16,
                fire_top: 0,
                con: crate::FULL_CON,
                growth_type: false,
                x: 50,
                y: 60,
                shape_x: 0,
                shape_y: 0,
                shape_width: 0,
                shape_height: 0,
                rotation: 90,
                rotateable: true,
                xdir: 0.0,
                ydir: 0.0,
            }),
            8,
        );

        for (particle, (px, py)) in system.particles().iter().zip(expected.iter()) {
            let expected_x = 50.0f32 + rot[0] * *px as f32 + rot[1] * *py as f32;
            let expected_y = 60.0f32 + rot[2] * *px as f32 + rot[3] * *py as f32;
            assert_eq!(particle.x.to_bits(), expected_x.to_bits());
            assert_eq!(particle.y.to_bits(), expected_y.to_bits());
        }
    }

    #[test]
    fn object_fire_raises_the_emission_row_by_the_defs_fire_top() {
        // `iYOff = iHeight / 2 - Def->Shape.FireTop` (C4Effect.cpp:682): a
        // FireTop moves the row up from the shape's vertical centre, which is
        // how tall definitions burn at their top rather than their middle.
        let emit = |fire_top: i32| {
            let mut system = ParticleSystem::default();
            system
                .register_def(std_core("Fire"), 10, 1.0)
                .expect("registers");
            system
                .register_def(std_core("Fire2"), 10, 1.0)
                .expect("registers");
            system.safe_rng = SafeRng::new(5);
            system.create_object_fire(&ObjectFireEmission {
                object: crate::ObjectId::new(1),
                fire_mode: crate::C4FX_FIRE_MODE_STRUCT_VEH,
                def_width: 16,
                def_height: 40,
                fire_top,
                con: crate::FULL_CON,
                growth_type: false,
                x: 0,
                y: 0,
                shape_x: 0,
                shape_y: 0,
                shape_width: 0,
                shape_height: 0,
                rotation: 0,
                rotateable: false,
                xdir: 0.0,
                ydir: 0.0,
            });
            system
                .particles()
                .iter()
                .map(|particle| particle.y)
                .collect::<Vec<_>>()
        };

        // iYOff is 20 without FireTop and 20 - 15 = 5 with it, and con is
        // full, so every particle sits exactly 15 higher.
        let base = emit(0);
        let raised = emit(15);
        assert_eq!(base.len(), raised.len());
        assert!(!base.is_empty());
        for (base_y, raised_y) in base.iter().zip(raised.iter()) {
            assert_eq!(
                raised_y.to_bits(),
                (base_y - 15.0).to_bits(),
                "FireTop subtracts from iYOff before the con scale",
            );
        }
    }

    #[test]
    fn object_fire_wraps_like_cpp_int32_instead_of_trapping_on_a_huge_con() {
        // C4Effect.cpp:694,716,719,727,749-751,757-758 are plain `int32_t`.
        // An `Oversize` definition's Con is bounded only below (C4Object.cpp
        // DoCon), so `100 * GetCon()` and the LivingVeg `iPx * iPx * 100`
        // term overflow for a large burning object — C++ wraps and draws a
        // garbage particle. Con is reachable from script and from a loaded
        // Objects.txt, so the port must not trap here either.
        let mut system = ParticleSystem::default();
        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");

        let emission = ObjectFireEmission {
            object: crate::ObjectId::new(1),
            fire_mode: crate::C4FX_FIRE_MODE_LIVING_VEG,
            def_width: 200,
            def_height: 200,
            fire_top: 0,
            con: i32::MAX,
            growth_type: true,
            x: 0,
            y: 0,
            shape_x: 0,
            shape_y: 0,
            shape_width: 0,
            shape_height: 0,
            rotation: 0,
            rotateable: false,
            xdir: 0.0,
            ydir: 0.0,
        };
        assert!(
            system.create_object_fire(&emission) > 0,
            "the emission still runs; only the arithmetic wraps",
        );
    }

    #[test]
    fn object_fire_holds_width_at_full_con_unless_the_def_stretches() {
        // C4Effect.cpp:693-697: iCon follows GetCon(), but a def without
        // GrowthType keeps iWdtCon pinned at 100 so a half-built structure
        // still burns across its full width — and :740 floors the count at 2.
        let emission = |growth_type: bool| ObjectFireEmission {
            object: crate::ObjectId::new(1),
            fire_mode: crate::C4FX_FIRE_MODE_STRUCT_VEH,
            def_width: 16,
            def_height: 16,
            fire_top: 0,
            con: crate::FULL_CON / 10, // 10%
            growth_type,
            x: 0,
            y: 0,
            shape_x: 0,
            shape_y: 0,
            shape_width: 0,
            shape_height: 0,
            rotation: 0,
            rotateable: false,
            xdir: 0.0,
            ydir: 0.0,
        };

        let mut fixed_width = ParticleSystem::default();
        fixed_width
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        fixed_width
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        // iWdtCon pinned to 100 → count stays int(sqrt(256) / 4) = 4.
        assert_eq!(fixed_width.create_object_fire(&emission(false)), 8);

        let mut stretched = ParticleSystem::default();
        stretched
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        stretched
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        // iWdtCon = 10 → 4 * 10 / 100 = 0, floored to the minimum 2.
        assert_eq!(stretched.create_object_fire(&emission(true)), 4);
    }

    #[test]
    fn fire_particles_are_loaded_only_when_both_defs_and_the_local_switch_are_present() {
        // IsFireParticleLoaded is `pFire1 && pFire2` (C4Particles.h:214), and
        // SetDefParticles only resolves the pair when
        // Config.Graphics.FireParticles is set (C4Particles.cpp:483-489).
        let mut system = ParticleSystem::default();
        assert!(!system.is_fire_particle_loaded(), "no defs registered yet");

        system
            .register_def(std_core("Fire"), 10, 1.0)
            .expect("registers");
        assert!(!system.is_fire_particle_loaded(), "Fire2 still missing");

        system
            .register_def(std_core("Fire2"), 10, 1.0)
            .expect("registers");
        assert!(system.is_fire_particle_loaded());

        system.fire_particles = false;
        assert!(
            !system.is_fire_particle_loaded(),
            "the local FireParticles switch leaves pFire1/pFire2 null",
        );
    }

    #[test]
    fn push_adds_velocity_to_matching_particles_like_cpp() {
        // C4ParticleSystem::Push (C4Particles.cpp:494-519): with a def, only
        // particles of that def are pushed; with no def, every live particle
        // is. (C++ also touches free-list slots when no def is given, but
        // those are fully re-initialized by Create, so that is unobservable.)
        let mut system = ParticleSystem::default();
        let mut spark = std_core("Spark");
        spark.delay = 1; // fxStdInit: Delay set → life starts at 0, no draw
        let mut mist = std_core("Mist");
        mist.delay = 1;
        system.register_def(spark, 4, 1.0).expect("registers");
        system.register_def(mist, 4, 1.0).expect("registers");
        let layer = crate::ParticleLayer::Global;
        assert!(system.create("Spark", 0.0, 0.0, 1.0, 1.0, 0.0, 1, layer.clone(), None));
        assert!(system.create("Mist", 0.0, 0.0, 2.0, 2.0, 0.0, 1, layer, None));

        let pushed = system.push(Some("Spark"), 0.5, -0.25);
        assert_eq!(pushed, 1);
        assert_eq!(system.particles()[0].xdir.to_bits(), 1.5f32.to_bits());
        assert_eq!(system.particles()[0].ydir.to_bits(), 0.75f32.to_bits());
        assert_eq!(system.particles()[1].xdir.to_bits(), 2.0f32.to_bits());

        let pushed_all = system.push(None, 1.0, 0.0);
        assert_eq!(pushed_all, 2);
        assert_eq!(system.particles()[0].xdir.to_bits(), 2.5f32.to_bits());
        assert_eq!(system.particles()[1].xdir.to_bits(), 3.0f32.to_bits());
    }

    fn open_env(env_gravity: crate::math::C4Fixed) -> ParticleEnv<'static> {
        ParticleEnv {
            gravity: env_gravity,
            frame_counter: 0,
            back_wdt: 320,
            back_hgt: 200,
            solid: &|_, _| false,
            wind: &|_, _| 0,
        }
    }

    #[test]
    fn std_exec_moves_applies_gravity_and_kills_offscreen_like_cpp() {
        // fxStdExec (C4Particles.cpp:614-697): position += dir each frame;
        // GravityAcc adds fixtof(GravAccel * GravityAcc) / 100 to ydir
        // (line 647); a particle moving right dies once x - a >= GBackWdt
        // (lines 694-696). With Delay = 0 the life counter is never
        // decremented — death is purely positional/alpha.
        let mut system = ParticleSystem::default();
        let mut core = std_core("Spark");
        core.gravity_acc = 100;
        // Delay-based lifetime: life starts 0 (fxStdInit) and must stay below
        // (Length - Reverse) * Repeats + Reverse or the particle dies on its
        // first exec (C4Particles.cpp:682-689) — Repeats = 0 means instant
        // death, so any real delayed def carries Repeats > 0.
        core.delay = 1;
        core.repeats = 1000;
        system.register_def(core, 4, 1.0).expect("registers");
        let layer = crate::ParticleLayer::Global;
        assert!(system.create("Spark", 10.0, 20.0, 2.0, -1.0, 1.0, 1, layer.clone(), None));

        let gravity = crate::math::fixed100(20); // GravAccel-style value
        let env = open_env(gravity);
        system.exec_layer(&layer, None, &env);

        let particle = &system.particles()[0];
        assert_eq!(particle.x.to_bits(), 12.0f32.to_bits());
        assert_eq!(particle.y.to_bits(), 19.0f32.to_bits());
        // ydir: -1.0 + fixtof(GravAccel*100)/100 = -1.0 + fixtof(GravAccel)
        let expected_ydir = -1.0f32 + crate::math::fixtof(gravity);
        assert_eq!(particle.ydir.to_bits(), expected_ydir.to_bits());
        // delay-based lifetime advanced (C4Particles.cpp:672-691)
        assert_eq!(particle.life, 1);

        // Particle moving right beyond GBackWdt dies and releases its count.
        // The off-screen rules only run for Delay == 0 (C4Particles.cpp:692).
        let mut runaway = ParticleSystem::default();
        let core = std_core("Spark");
        runaway.register_def(core, 4, 1.0).expect("registers");
        assert!(runaway.create("Spark", 500.0, 50.0, 1.0, 0.0, 2.0, 1, layer.clone(), None));
        runaway.exec_layer(&layer, None, &env);
        assert!(runaway.particles().is_empty(), "x - a >= GBackWdt kills");
        assert_eq!(runaway.get_def("Spark").unwrap().count, 0);
    }

    #[test]
    fn std_exec_wind_drift_and_alpha_fade_match_cpp() {
        // WindDrift (C4Particles.cpp:649-660): air speed is wind/15, drift
        // strength max(WindDrift-20, 0), and both dirs relax toward the air
        // speed by (target-dir)*drift/800. AlphaFade (lines 662-671) adds to
        // the alpha byte; alpha >= 0xff kills; a negative AlphaFade applies
        // one step every -AlphaFade frames.
        let mut system = ParticleSystem::default();
        let mut core = std_core("Leaf");
        core.wind_drift = 100;
        core.alpha_fade = 16;
        core.delay = 1;
        core.repeats = 1000;
        system.register_def(core, 4, 1.0).expect("registers");
        let layer = crate::ParticleLayer::Global;
        assert!(system.create(
            "Leaf",
            50.0,
            50.0,
            1.0,
            0.5,
            0.0,
            0x40123456,
            layer.clone(),
            None
        ));
        let env = ParticleEnv {
            gravity: crate::math::C4Fixed::ZERO,
            frame_counter: 0,
            back_wdt: 320,
            back_hgt: 200,
            solid: &|_, _| false,
            wind: &|_, _| 30,
        };
        system.exec_layer(&layer, None, &env);
        let particle = &system.particles()[0];
        // moved by the pre-drift velocity first
        assert_eq!(particle.x.to_bits(), 51.0f32.to_bits());
        assert_eq!(particle.y.to_bits(), 50.5f32.to_bits());
        // xdir: 1.0 + ((30/15 - 1.0) * 80) / 800 = 1.1
        let expected_xdir = 1.0f32 + ((2.0 - 1.0) * 80.0) / 800.0;
        let expected_ydir = 0.5f32 + ((0.0 - 0.5) * 80.0) / 800.0;
        assert_eq!(particle.xdir.to_bits(), expected_xdir.to_bits());
        assert_eq!(particle.ydir.to_bits(), expected_ydir.to_bits());
        // alpha 0x40 + 16 = 0x50
        assert_eq!(particle.b as u32, 0x50123456);

        // alpha saturation kills (C4Particles.cpp:669)
        let mut dying = ParticleSystem::default();
        let mut core = std_core("Leaf");
        core.alpha_fade = 16;
        core.delay = 1;
        core.repeats = 1000;
        dying.register_def(core, 4, 1.0).expect("registers");
        assert!(dying.create(
            "Leaf",
            50.0,
            50.0,
            0.0,
            0.0,
            0.0,
            0xf8000000u32 as i32,
            crate::ParticleLayer::Global,
            None
        ));
        dying.exec_layer(&crate::ParticleLayer::Global, None, &env);
        assert!(dying.particles().is_empty());

        // negative AlphaFade fires only on matching frames (line 663) and
        // adds the (negative) AlphaFade itself to alpha (line 668).
        let mut periodic = ParticleSystem::default();
        let mut core = std_core("Leaf");
        core.alpha_fade = -3;
        core.delay = 1;
        core.repeats = 1000;
        periodic.register_def(core, 4, 1.0).expect("registers");
        assert!(periodic.create(
            "Leaf",
            50.0,
            50.0,
            0.0,
            0.0,
            0.0,
            0x40000000,
            crate::ParticleLayer::Global,
            None
        ));
        let mut frame5 = ParticleEnv {
            frame_counter: 5,
            ..env
        };
        periodic.exec_layer(&crate::ParticleLayer::Global, None, &frame5);
        assert_eq!(
            periodic.particles()[0].b as u32,
            0x40000000,
            "5 % 3 != 0 → no fade step"
        );
        frame5.frame_counter = 6;
        periodic.exec_layer(&crate::ParticleLayer::Global, None, &frame5);
        assert_eq!(
            periodic.particles()[0].b as u32,
            0x3d000000,
            "6 % 3 == 0 → alpha += -3"
        );
    }

    #[test]
    fn std_exec_collision_dispatches_collision_proc_like_cpp() {
        // C4Particles.cpp:628-645: with VertexCount set, a solid pixel at the
        // movement target triggers the collision proc — Bounce reverses both
        // dirs (lines 699-705) and blocks the move; Die removes the particle
        // (721-725); without a collision proc the particle just does not move.
        let solid_env = ParticleEnv {
            gravity: crate::math::C4Fixed::ZERO,
            frame_counter: 0,
            back_wdt: 320,
            back_hgt: 200,
            solid: &|_, y| y >= 60,
            wind: &|_, _| 0,
        };
        let layer = crate::ParticleLayer::Global;

        let mut bouncing = ParticleSystem::default();
        let mut core = std_core("Drop");
        core.vertex_count = 1;
        core.collision_fn = "Bounce".to_string();
        core.delay = 1;
        core.repeats = 1000;
        bouncing.register_def(core, 4, 1.0).expect("registers");
        assert!(bouncing.create("Drop", 50.0, 59.0, 0.0, 2.0, 0.0, 1, layer.clone(), None));
        bouncing.exec_layer(&layer, None, &solid_env);
        let particle = &bouncing.particles()[0];
        assert_eq!(
            particle.y.to_bits(),
            59.0f32.to_bits(),
            "collision blocks move"
        );
        assert_eq!(particle.xdir.to_bits(), (-0.0f32).to_bits());
        assert_eq!(particle.ydir.to_bits(), (-2.0f32).to_bits());

        let mut dying = ParticleSystem::default();
        let mut core = std_core("Drop");
        core.vertex_count = 1;
        core.collision_fn = "Die".to_string();
        dying.register_def(core, 4, 1.0).expect("registers");
        assert!(dying.create("Drop", 50.0, 59.0, 0.0, 2.0, 0.0, 1, layer.clone(), None));
        dying.exec_layer(&layer, None, &solid_env);
        assert!(dying.particles().is_empty());

        let mut stuck = ParticleSystem::default();
        let mut core = std_core("Drop");
        core.vertex_count = 1;
        core.delay = 1;
        core.repeats = 1000;
        stuck.register_def(core, 4, 1.0).expect("registers");
        assert!(stuck.create("Drop", 50.0, 59.0, 0.0, 2.0, 0.0, 1, layer.clone(), None));
        stuck.exec_layer(&layer, None, &solid_env);
        let particle = &stuck.particles()[0];
        assert_eq!(particle.y.to_bits(), 59.0f32.to_bits());
        assert_eq!(particle.ydir.to_bits(), 2.0f32.to_bits());
    }

    #[test]
    fn std_exec_delay_lifetime_fadeout_decay_matches_cpp() {
        // C4Particles.cpp:672-691: with Delay set, life counts up; once
        // phase = life/Delay reaches (Length-Reverse)*Repeats + Reverse the
        // particle either dies (no FadeOutLen) or enters decay at life = -1.
        // Decay keeps the particle while life >= -FadeOutLen*FadeOutDelay,
        // post-decrementing each exec (line 678).
        let mut system = ParticleSystem::default();
        let mut core = std_core("Puff");
        core.delay = 1;
        core.repeats = 1;
        core.fade_out_len = 2;
        core.fade_out_delay = 3;
        // gfx length 6 → Length = max(6-2, 1) = 4 (C4Particles.cpp:148-152)
        system.register_def(core, 6, 1.0).expect("registers");
        let layer = crate::ParticleLayer::Global;
        assert!(system.create("Puff", 50.0, 50.0, 0.0, 0.0, 0.0, 1, layer.clone(), None));
        let env = open_env(crate::math::C4Fixed::ZERO);

        // life 0 → 1 → 2 → 3, then phase 4 >= (4-0)*1+0 → life = -1
        for expected_life in [1, 2, 3, -1] {
            system.exec_layer(&layer, None, &env);
            assert_eq!(system.particles()[0].life, expected_life);
        }
        // decay: survives while life >= -6, post-decrementing
        for expected_life in [-2, -3, -4, -5, -6, -7] {
            system.exec_layer(&layer, None, &env);
            assert_eq!(system.particles()[0].life, expected_life);
        }
        // life == -7 < -6 → dies
        system.exec_layer(&layer, None, &env);
        assert!(system.particles().is_empty());
        assert_eq!(system.get_def("Puff").unwrap().count, 0);
    }

    #[test]
    fn smoke_init_and_exec_match_cpp_trace() {
        // fxSmokeInit (C4Particles.cpp:521-535): life = MinLifetime
        // (+SafeRandom spread), with life/17 stored in the high word as
        // init-status; ydir holds the drawing "kind"; b defaults 0xff4b4b4b.
        // fxSmokeExec (C4Particles.cpp:537-576): pre-decrements life (dies at
        // exactly 0), burns down the high word while "building" (b loses
        // 0x10000000 per building tick, then alpha snaps to 255-life),
        // lightens RGB by 1 + bumps alpha by 1 each tick, floats up one pixel
        // when unobstructed, and grows by ×1.01.
        let mut system = ParticleSystem::default();
        let core = ParticleDefCore {
            name: "Smoke".to_string(),
            init_fn: "SmokeInit".to_string(),
            exec_fn: "SmokeExec".to_string(),
            draw_fn: "Smoke".to_string(),
            min_lifetime: 34,
            max_lifetime: 34, // iLD = 0 → no SafeRandom draw for life
            ..ParticleDefCore::default()
        };
        system.register_def(core, 4, 1.0).expect("registers");
        system.safe_rng = SafeRng::new(7);
        let mut mirror = SafeRng::new(7);
        let expected_kind = mirror.random(15) as f32 + (mirror.random(300) / 299) as f32;

        let layer = crate::ParticleLayer::Global;
        assert!(system.create("Smoke", 50.0, 100.0, 0.0, 0.0, 4.0, 0, layer.clone(), None));
        let particle = &system.particles()[0];
        assert_eq!(particle.life, 34 | (2 << 16), "init-status in high word");
        assert_eq!(particle.ydir.to_bits(), expected_kind.to_bits());
        assert_eq!(particle.b as u32, 0xff4b4b4b);

        let env = ParticleEnv {
            gravity: crate::math::C4Fixed::ZERO,
            frame_counter: 0,
            back_wdt: 320,
            back_hgt: 200,
            solid: &|_, _| false,
            wind: &|_, _| 0,
        };

        // exec 1: building → life 0x20022-1-0x10000 = 0x10021,
        // b: 0xff4b4b4b - 0x10000000 = 0xef4b4b4b → lighten+alpha → 0xf04c4c4c
        system.exec_layer(&layer, None, &env);
        let xdir1 = 0.1f32 * mirror.random(41) as f32 - 2.0; // wind draw
        let particle = &system.particles()[0];
        assert_eq!(particle.life, 0x10021);
        assert_eq!(particle.b as u32, 0xf04c4c4c);
        assert_eq!(particle.xdir.to_bits(), xdir1.to_bits());
        assert_eq!(particle.y.to_bits(), 99.0f32.to_bits(), "floats up");
        assert_eq!(particle.x.to_bits(), (50.0f32 + xdir1).to_bits());
        assert_eq!(particle.a.to_bits(), (4.0f32 * 1.01).to_bits());

        // exec 2: building ends → alpha snaps to 255-life = 0xdf, then the
        // per-tick lighten/alpha-bump gives 0xe04d4d4d.
        system.exec_layer(&layer, None, &env);
        let xdir2 = 0.1f32 * mirror.random(41) as f32 - 2.0;
        let particle = &system.particles()[0];
        assert_eq!(particle.life, 0x20);
        assert_eq!(particle.b as u32, 0xe04d4d4d);
        assert_eq!(particle.xdir.to_bits(), xdir2.to_bits());

        // exec 3: no longer building; b%12 != 0 → no wind draw this tick.
        system.exec_layer(&layer, None, &env);
        let particle = &system.particles()[0];
        assert_eq!(particle.life, 31);
        assert_eq!(particle.b as u32, 0xe14e4e4e);
        assert_eq!(particle.xdir.to_bits(), xdir2.to_bits(), "xdir unchanged");

        // life pre-decrements to exactly 0 → dies after 31 more execs.
        for _ in 0..30 {
            system.exec_layer(&layer, None, &env);
            assert_eq!(system.particles().len(), 1);
        }
        system.exec_layer(&layer, None, &env);
        assert!(system.particles().is_empty());
        assert_eq!(system.get_def("Smoke").unwrap().count, 0);
    }

    #[test]
    fn safe_rng_zero_range_returns_zero_and_is_deterministic_per_seed() {
        // SafeRandom (C4Random.h:71-75): `if (!range) return 0; return
        // rand() % range;`. C++ seeds libc rand() from wall-clock time
        // (C4Random.h:35), so the stream is explicitly unsynced; the Rust
        // port only has to honor the structure: range 0 → 0, results in
        // [0, range), deterministic for a fixed seed.
        let mut rng = SafeRng::new(1);
        assert_eq!(rng.random(0), 0);
        for range in [1, 2, 3, 10, 100, 256] {
            let value = rng.random(range);
            assert!((0..range).contains(&value), "range={range} got {value}");
        }
        let mut a = SafeRng::new(42);
        let mut b = SafeRng::new(42);
        for range in [5, 7, 100, 33, 256, 1000] {
            assert_eq!(a.random(range), b.random(range));
        }
    }

    #[test]
    fn particle_proc_map_matches_cpp() {
        // C4ParticleProcMap (C4Particles.cpp:790-801): name → proc, with an
        // empty-name terminator; GetProc returns nullptr for unknown names
        // (C4Particles.cpp:445-453).
        assert_eq!(
            ParticleProc::from_name("SmokeInit"),
            Some(ParticleProc::SmokeInit)
        );
        assert_eq!(
            ParticleProc::from_name("SmokeExec"),
            Some(ParticleProc::SmokeExec)
        );
        assert_eq!(
            ParticleProc::from_name("StdInit"),
            Some(ParticleProc::StdInit)
        );
        assert_eq!(
            ParticleProc::from_name("StdExec"),
            Some(ParticleProc::StdExec)
        );
        assert_eq!(
            ParticleProc::from_name("Bounce"),
            Some(ParticleProc::Bounce)
        );
        assert_eq!(
            ParticleProc::from_name("BounceY"),
            Some(ParticleProc::BounceY)
        );
        assert_eq!(ParticleProc::from_name("Stop"), Some(ParticleProc::Stop));
        assert_eq!(ParticleProc::from_name("Die"), Some(ParticleProc::Die));
        assert_eq!(ParticleProc::from_name("NoSuchProc"), None);
        assert_eq!(ParticleProc::from_name(""), None);

        assert_eq!(
            ParticleDrawProc::from_name("Smoke"),
            Some(ParticleDrawProc::Smoke)
        );
        assert_eq!(
            ParticleDrawProc::from_name("Std"),
            Some(ParticleDrawProc::Std)
        );
        assert_eq!(ParticleDrawProc::from_name("std"), None);
        assert_eq!(ParticleDrawProc::from_name(""), None);
    }

    #[test]
    fn particle_def_core_defaults_match_cpp() {
        // C4ParticleDefCore ctor (C4Particles.cpp:58-74) + CompileFunc
        // defaults (C4Particles.cpp:30-56): MaxCount = C4Px_MaxParticle (256,
        // C4Constants.h:71), Parallaxity = {100, 100}, everything else 0/"".
        let core = ParticleDefCore::default();
        assert_eq!(core.name, "");
        assert_eq!(core.max_count, 256);
        assert_eq!(core.min_lifetime, 0);
        assert_eq!(core.max_lifetime, 0);
        assert_eq!(core.y_off, 0);
        assert_eq!(core.delay, 0);
        assert_eq!(core.repeats, 0);
        assert_eq!(core.reverse, 0);
        assert_eq!(core.fade_out_len, 0);
        assert_eq!(core.fade_out_delay, 0);
        assert_eq!(core.r_by_v, 0);
        assert_eq!(core.placement, 0);
        assert_eq!(core.gravity_acc, 0);
        assert_eq!(core.wind_drift, 0);
        assert_eq!(core.vertex_count, 0);
        assert_eq!(core.vertex_y, 0);
        assert_eq!(core.additive, 0);
        assert_eq!(core.attach, 0);
        assert_eq!(core.alpha_fade, 0);
        assert_eq!(core.parallaxity, [100, 100]);
        assert_eq!(core.init_fn, "");
        assert_eq!(core.exec_fn, "");
        assert_eq!(core.draw_fn, "");
        assert_eq!(core.collision_fn, "");
    }
}
