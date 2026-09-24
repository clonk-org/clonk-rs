//! Actor-aware route planning for the normal profile's command AI
//! (`sim-navigation-ai`, docs/COMPAT_PROFILE.md).
//!
//! C4PathFinder routes a single pixel through the landscape (`PointFree` is
//! `LandscapeFree`, C4PathFinder.cpp:403-406; C4Game.cpp:2288-2292). A route
//! may therefore squeeze through a gap the actor's body cannot pass, cross open
//! air as if it were floor, or run up a face the actor cannot scale; the
//! waypoint MoveTo then expires as success and its parent retries the same
//! route forever (C4Command.cpp:1544-1552). This planner instead searches the
//! positions the actor can stand at, joined by the moves the engine executes
//! for it: walking over small steps, walking off a ledge, the fixed
//! ObjectComJump arc (C4ObjectCom.cpp:284-296), and scaling a wall until
//! KneelUp puts the actor on top.
//!
//! It is lockstep-safe: integer and `C4Fixed` arithmetic only, a total order on
//! the open list, and a work budget counted in expansions rather than time.

use crate::math::{self, C4Fixed};
use crate::{
    Landscape, ObjectVertex, Vector2, ATTACH_RANGE, CNAT_BOTTOM, CNAT_LEFT, CNAT_RIGHT, CNAT_TOP,
    FULL_CON,
};
use clonk_resources::{PhysicalInfo, C4_MAX_PHYSICAL};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// Vertices a [`NavBody`] keeps. Crew definitions carry far fewer (CLNK has
/// seven); a larger shape keeps its first twelve.
const MAX_BODY_VERTICES: usize = 12;
/// Cost units per frame of movement.
pub const COST_PER_FRAME: i32 = 16;
/// Extra cost for a jump, which misses more easily than a walk.
const JUMP_PENALTY: i32 = 8 * COST_PER_FRAME;
/// Frames KneelUp takes to stand the actor up at the top of a wall.
const KNEEL_UP_FRAMES: i32 = 10;
/// Frames a simulated flight may last before it counts as no landing.
const MAX_FLIGHT_FRAMES: i32 = 240;
/// Pixels a simulated climb may cover.
const MAX_CLIMB: i32 = 600;
/// How far a corner scale reaches across and up (C4Physics.h:24-25).
const CORNER_RANGE: i32 = ATTACH_RANGE + 2;
/// Walls a simulated drop may let go of before it counts as no landing.
const MAX_LET_GOS: usize = 8;
/// OCF_HitSpeed3: at this |xdir| + |ydir| a flier tumbles off a wall instead
/// of grabbing it (C4Movement.cpp:37; C4Object.cpp:593,2095-2101,4428-4433).
const HIT_SPEED3: i32 = 6;
/// How far beyond the start/goal box the search may wander.
const SEARCH_MARGIN_X: i32 = 400;
const SEARCH_MARGIN_Y: i32 = 300;
/// Waypoints a plan hands to the command stack at once; the stack holds at
/// most 35 commands (C4Object.cpp:3909-3913), and the goal MoveTo replans
/// from wherever the prefix ends.
pub const MAX_PLAN_WAYPOINTS: usize = 20;
/// How far a landing may stray from the planned spot and still count; the
/// executor applies the same tolerance to Jump and Drop waypoints.
pub const LANDING_TOLERANCE_X: i32 = 8;
/// Vertical arrival tolerance: WALK cannot correct height.
pub const ARRIVAL_TOLERANCE_Y: i32 = 10;
/// A jump or drop is only planned if taking off this many pixels off the
/// planned spot still lands in the same place: WALK is not exact.
const TAKEOFF_SLACK: i32 = 2;

/// The actor's collision vertices (C4Shape::VtxX/VtxY/VtxCNAT relative to its
/// position), kept in a fixed array so command snapshots stay allocation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavBody {
    len: u8,
    x: [i16; MAX_BODY_VERTICES],
    y: [i16; MAX_BODY_VERTICES],
    cnat: [u8; MAX_BODY_VERTICES],
}

impl NavBody {
    pub fn from_vertices(vertices: &[ObjectVertex]) -> Self {
        let mut body = Self::default();
        for vertex in vertices.iter().take(MAX_BODY_VERTICES) {
            let index = usize::from(body.len);
            body.x[index] = vertex.x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            body.y[index] = vertex.y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            body.cnat[index] = (vertex.cnat & 0xff) as u8;
            body.len += 1;
        }
        body
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn vertices(&self) -> impl Iterator<Item = (i32, i32, u32)> + '_ {
        (0..usize::from(self.len)).map(|index| {
            (
                i32::from(self.x[index]),
                i32::from(self.y[index]),
                u32::from(self.cnat[index]),
            )
        })
    }

    /// The deepest CNAT_Bottom vertex, where the actor meets the floor.
    fn feet(&self) -> i32 {
        self.vertices()
            .filter(|(_, _, cnat)| cnat & CNAT_BOTTOM != 0)
            .map(|(_, y, _)| y)
            .max()
            .or_else(|| self.vertices().map(|(_, y, _)| y).max())
            .unwrap_or(0)
    }

    /// The highest floor step WALK carries the actor over. A step that stays
    /// below every side vertex lifts the walker; one that reaches a side
    /// vertex is a wall instead (SCALE for a scaler), and one exactly at the
    /// side vertex's height stalls it. Measured for CLNK, whose side vertices
    /// sit six pixels above its feet: five pixels walk, six stall.
    fn walk_step(&self) -> i32 {
        self.vertices()
            .filter(|(_, _, cnat)| cnat & (CNAT_LEFT | CNAT_RIGHT) != 0)
            .map(|(_, y, _)| y)
            .max()
            .map_or(4, |side| (self.feet() - side - 1).clamp(1, 8))
    }
}

/// What the planner needs to know about the actor it plans for.
#[derive(Debug, Clone, Copy)]
pub struct NavActor {
    pub body: NavBody,
    /// DFA_WALK's limit and the jump launch's horizontal speed,
    /// ValByPhysical(280, Walk) scaled by construction.
    pub walk_speed: C4Fixed,
    /// The jump launch's vertical speed, ValByPhysical(1000, Jump) scaled by
    /// construction (C4ObjectCom.cpp:284-296).
    pub jump_speed: C4Fixed,
    /// DFA_SCALE's climbing speed, ValByPhysical(200, Scale).
    pub scale_speed: C4Fixed,
    /// DFA_SWIM's limit on each axis, ValByPhysical(160, Swim)
    /// (C4Object.cpp:4939,4976-4978).
    pub swim_speed: C4Fixed,
    /// Frames the actor can go without air before it loses energy: Breath
    /// drains by 2% of C4MaxPhysical every fifth frame (C4Object.cpp:
    /// 880-921). Unlimited for a water-breather, whose breath the planner
    /// does not model: it breathes in water instead (C4Object.cpp:891-894).
    pub breath_frames: i32,
    /// Where the actor draws breath, below its position: half its shape's
    /// top offset (C4Object.cpp:897).
    pub breath_offset: i32,
    pub can_scale: bool,
    /// A flier touching a ceiling hangles instead of falling on
    /// (C4Object.cpp:4382-4421), which no planned move expects.
    pub can_hangle: bool,
    pub gravity: C4Fixed,
}

impl NavActor {
    /// `shape_top` is the actor's shape offset above its position
    /// (C4Shape::y), where its breath point comes from.
    pub fn new(
        body: NavBody,
        physical: &PhysicalInfo,
        construction: i32,
        gravity: C4Fixed,
        shape_top: i32,
    ) -> Self {
        let con = math::itofix_prec(construction, FULL_CON);
        Self {
            body,
            walk_speed: math::val_by_physical(280, physical.walk) * con,
            jump_speed: math::val_by_physical(1000, physical.jump) * con,
            scale_speed: math::val_by_physical(200, physical.scale),
            swim_speed: math::val_by_physical(160, physical.swim),
            breath_frames: if physical.breathe_water != 0 {
                i32::MAX
            } else {
                physical.breath / (2 * C4_MAX_PHYSICAL / 100) * 5
            },
            breath_offset: shape_top / 2,
            can_scale: physical.can_scale != 0,
            can_hangle: physical.can_hangle != 0,
            gravity,
        }
    }
}

/// How the actor reaches a waypoint from the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavMove {
    /// Walk along the floor, over steps no higher than the walk step.
    Walk,
    /// Walk off a ledge and fall to the floor below.
    Drop,
    /// Jump from the previous waypoint along the ObjectComJump arc.
    Jump,
    /// Walk into the wall to start SCALE, climb, and KneelUp on top.
    Climb,
    /// Swim in a straight line through liquid.
    Swim,
    /// Swim along the surface into a shore too shallow to swim on, until
    /// DFA_SWIM's corner scale KneelUps the actor onto it.
    Ashore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavWaypoint {
    pub x: i32,
    pub y: i32,
    pub movement: NavMove,
    /// Facing for Jump and Climb.
    pub right: bool,
    /// Expected frames to reach this waypoint from the previous one.
    pub frames: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavPlan {
    pub waypoints: Vec<NavWaypoint>,
    /// Total cost in [`COST_PER_FRAME`] units.
    pub cost: i32,
}

/// Where the plan may end: any standing position within the ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavGoal {
    pub x: i32,
    pub y: i32,
    pub range_x: i32,
    pub range_y: i32,
}

impl NavGoal {
    fn contains(&self, x: i32, y: i32) -> bool {
        (x - self.x).abs() <= self.range_x && (y - self.y).abs() <= self.range_y
    }
}

/// Plan a route for `actor`, standing at `start`, to `goal`, expanding at
/// most `budget` positions. `None` means no route within the budget.
pub fn plan(
    landscape: &Landscape,
    actor: &NavActor,
    start: Vector2,
    goal: NavGoal,
    budget: usize,
) -> Option<NavPlan> {
    if actor.body.is_empty() {
        return None;
    }
    let search = Search::new(landscape, actor, start, goal);
    let start = search.settle(start.x, start.y)?;
    search.run(start, goal, budget)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Walk,
    Drop,
    Jump { right: bool },
    Climb { right: bool },
    JumpClimb { right: bool, grab: (i32, i32) },
    Swim,
    Ashore { right: bool },
}

#[derive(Debug, Clone, Copy)]
struct Link {
    from: (i32, i32),
    edge: Edge,
    cost: i32,
}

#[derive(Debug)]
enum Landing {
    Stand {
        x: i32,
        y: i32,
        frames: i32,
    },
    Wall {
        x: i32,
        y: i32,
        dir: i32,
        frames: i32,
    },
    /// The flight ends in liquid, where the actor swims.
    Swim {
        x: i32,
        y: i32,
        frames: i32,
    },
}

impl Landing {
    /// Whether two landings end the same planned move.
    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Stand { x, y, .. }, Self::Stand { x: ox, y: oy, .. })
            | (Self::Swim { x, y, .. }, Self::Swim { x: ox, y: oy, .. }) => {
                (x - ox).abs() <= LANDING_TOLERANCE_X && (y - oy).abs() <= ARRIVAL_TOLERANCE_Y
            }
            (
                Self::Wall { x, y, dir, .. },
                Self::Wall {
                    x: ox,
                    y: oy,
                    dir: odir,
                    ..
                },
            ) => dir == odir && (x - ox).abs() <= 2 && (y - oy).abs() <= 2 * ARRIVAL_TOLERANCE_Y,
            _ => false,
        }
    }
}

enum WalkStep {
    Stand(i32),
    Ledge,
    Wall,
}

/// The CNAT of the actor's side that faces `dir`.
fn facing_side(dir: i32) -> u32 {
    if dir > 0 {
        CNAT_RIGHT
    } else {
        CNAT_LEFT
    }
}

struct Search<'a> {
    landscape: &'a Landscape,
    actor: &'a NavActor,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    walk_step: i32,
    walk_cost: i32,
    climb_cost: i32,
    swim_cost: i32,
    /// Cost a route may spend without air in one go.
    breath_budget: i32,
}

impl<'a> Search<'a> {
    fn new(landscape: &'a Landscape, actor: &'a NavActor, start: Vector2, goal: NavGoal) -> Self {
        let width = landscape.width().min(i32::MAX as u32) as i32;
        let height = landscape.estimated_height();
        let per_pixel = |speed: C4Fixed| {
            if speed.val() <= 0 {
                i32::MAX / 4
            } else {
                (COST_PER_FRAME * math::itofix(1).val() / speed.val()).max(1)
            }
        };
        Self {
            landscape,
            actor,
            min_x: (start.x.min(goal.x) - SEARCH_MARGIN_X).max(0),
            max_x: (start.x.max(goal.x) + SEARCH_MARGIN_X).min(width - 1),
            // Stay inside the map: a route over a wall that reaches the top
            // edge would stand the actor in the sky above the landscape.
            min_y: (start.y.min(goal.y) - SEARCH_MARGIN_Y).max(0),
            max_y: (start.y.max(goal.y) + SEARCH_MARGIN_Y).min(height - 1),
            walk_step: actor.body.walk_step(),
            walk_cost: per_pixel(actor.walk_speed),
            climb_cost: per_pixel(actor.scale_speed),
            swim_cost: per_pixel(actor.swim_speed),
            // A fifth in hand: breath drains on every fifth frame, whichever
            // frame the route starts on (C4Object.cpp:881).
            breath_budget: (actor.breath_frames.saturating_mul(4) / 5)
                .saturating_mul(COST_PER_FRAME),
        }
    }

    fn in_bounds(&self, x: i32, y: i32) -> bool {
        (self.min_x..=self.max_x).contains(&x) && (self.min_y..=self.max_y).contains(&y)
    }

    fn fits(&self, x: i32, y: i32) -> bool {
        self.actor
            .body
            .vertices()
            .all(|(vx, vy, _)| !self.landscape.is_solid_at(x + vx, y + vy))
    }

    fn supported(&self, x: i32, y: i32) -> bool {
        self.actor
            .body
            .vertices()
            .filter(|(_, _, cnat)| cnat & CNAT_BOTTOM != 0)
            .any(|(vx, vy, _)| self.landscape.is_solid_at(x + vx, y + vy + 1))
    }

    fn standing(&self, x: i32, y: i32) -> bool {
        self.fits(x, y) && self.supported(x, y) && !self.landscape.is_liquid_at(x, y)
    }

    /// The actor swims where its centre is in liquid (IsInLiquidCheck for
    /// Float=1, C4Object.cpp:5632-5635) and its body is clear of solid.
    fn swimming(&self, x: i32, y: i32) -> bool {
        self.actor.swim_speed.val() > 0 && self.landscape.is_liquid_at(x, y) && self.fits(x, y)
    }

    /// The actor draws breath at (x, y) while nothing semi-solid, liquid or
    /// solid, is at its breath point (C4Object.cpp:895-898). The forcefield
    /// exception, Vehicle material there giving breath (C4Object.cpp:
    /// 886-890), is not modelled.
    fn breathing(&self, x: i32, y: i32) -> bool {
        let by = y + self.actor.breath_offset;
        !self.landscape.is_solid_at(x, by) && !self.landscape.is_liquid_at(x, by)
    }

    /// A side vertex facing `dir` touches solid, the contact that turns a
    /// walker or flier into a scaler (C4Object.cpp:4406-4520).
    fn wall_contact(&self, x: i32, y: i32, dir: i32) -> bool {
        let side = facing_side(dir);
        self.actor
            .body
            .vertices()
            .filter(|(_, _, cnat)| cnat & side != 0)
            .any(|(vx, vy, _)| self.landscape.is_solid_at(x + vx + dir, y + vy))
    }

    /// The side whose vertices alone block a step down from (x, y). The
    /// touching vertex's CNAT is the contact direction (C4Movement.cpp:172),
    /// so the engine reports a side contact there, not a floor.
    fn caught_side(&self, x: i32, y: i32) -> Option<i32> {
        let caught = |side: u32| {
            self.actor
                .body
                .vertices()
                .filter(|(_, _, cnat)| cnat & side != 0)
                .any(|(vx, vy, _)| self.landscape.is_solid_at(x + vx, y + vy + 1))
        };
        match (caught(CNAT_LEFT), caught(CNAT_RIGHT)) {
            (true, false) => Some(-1),
            (false, true) => Some(1),
            _ => None,
        }
    }

    /// Where a plan from (x, y) starts: a standing position within a few
    /// pixels, or the swimming position itself.
    fn settle(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        [0, 1, -1, 2, -2, 3, -3]
            .into_iter()
            .map(|dy| y + dy)
            .find(|&y| self.standing(x, y))
            .or_else(|| self.swimming(x, y).then_some(y))
            .map(|y| (x, y))
    }

    fn walk(&self, x: i32, y: i32, dir: i32) -> WalkStep {
        let nx = x + dir;
        if self.fits(nx, y) {
            for down in 0..=self.walk_step {
                if !self.fits(nx, y + down) {
                    break;
                }
                if self.standing(nx, y + down) {
                    return WalkStep::Stand(y + down);
                }
            }
            return WalkStep::Ledge;
        }
        for up in 1..=self.walk_step {
            if self.fits(nx, y - up) {
                return if self.standing(nx, y - up) {
                    WalkStep::Stand(y - up)
                } else {
                    WalkStep::Wall
                };
            }
        }
        WalkStep::Wall
    }

    /// Integrate the actor's flight the way C4Object::ExecMovement does each
    /// frame: gravity first, then the move, one pixel at a time.
    fn fly(&self, x: i32, y: i32, mut vx: C4Fixed, mut vy: C4Fixed) -> Option<Landing> {
        let (mut fx, mut fy) = (math::itofix(x), math::itofix(y));
        let (mut ix, mut iy) = (x, y);
        let tumbles = |vx: C4Fixed, vy: C4Fixed| vx.abs() + vy.abs() >= math::itofix(HIT_SPEED3);
        for frame in 1..=MAX_FLIGHT_FRAMES {
            vy += self.actor.gravity;
            fx += vx;
            let tx = math::fixtoi(fx);
            while ix != tx {
                let step = (tx - ix).signum();
                if !self.fits(ix + step, iy) {
                    let side_contact = self.wall_contact(ix, iy, step);
                    if side_contact && tumbles(vx, vy) {
                        return None;
                    }
                    if side_contact && self.actor.can_scale {
                        return Some(Landing::Wall {
                            x: ix,
                            y: iy,
                            dir: step,
                            frames: frame,
                        });
                    }
                    if !side_contact {
                        // A head or foot vertex clipped a corner: the
                        // contact the engine reacts to is not predictable.
                        return None;
                    }
                    vx = C4Fixed::ZERO;
                    fx = math::itofix(ix);
                    break;
                }
                ix += step;
            }
            fy += vy;
            let ty = math::fixtoi(fy);
            while iy != ty {
                let step = (ty - iy).signum();
                if !self.fits(ix, iy + step) {
                    if step > 0 {
                        if self.standing(ix, iy) {
                            return Some(Landing::Stand {
                                x: ix,
                                y: iy,
                                frames: frame,
                            });
                        }
                        return self
                            .caught_side(ix, iy)
                            .filter(|_| self.actor.can_scale && !tumbles(vx, vy))
                            .map(|dir| Landing::Wall {
                                x: ix,
                                y: iy,
                                dir,
                                frames: frame,
                            });
                    }
                    if self.actor.can_hangle || self.actor.can_scale {
                        // Touching a ceiling mid-flight turns a hangler into
                        // HANGLE and a side vertex into SCALE
                        // (C4Object.cpp:4382-4520); no planned move follows.
                        return None;
                    }
                    vy = C4Fixed::ZERO;
                    fy = math::itofix(iy);
                    break;
                }
                iy += step;
            }
            if !self.in_bounds(ix, iy) {
                return None;
            }
            if self.landscape.is_liquid_at(ix, iy) {
                // In liquid the flier takes its action's InLiquidAction,
                // Swim for crew (C4Object.cpp:4758-4763).
                return self.swimming(ix, iy).then_some(Landing::Swim {
                    x: ix,
                    y: iy,
                    frames: frame,
                });
            }
        }
        None
    }

    /// Walk off the ledge at (x, y) and fall. A wall met on the way is let go
    /// of again, as the executor does with every grab a Drop did not plan
    /// (ObjectComLetGo: a one pixel per frame xdir away from the wall), so a
    /// drop can bounce down a shaft. Returns where the fall ends, standing or
    /// swimming, and the frames it takes.
    fn drop(&self, x: i32, y: i32, dir: i32) -> Option<(i32, i32, i32)> {
        let (mut x, mut y, mut frames) = (x, y, 0);
        let mut vx = self.actor.walk_speed * dir;
        for _ in 0..=MAX_LET_GOS {
            match self.fly(x, y, vx, C4Fixed::ZERO)? {
                Landing::Stand {
                    x,
                    y,
                    frames: flight,
                }
                | Landing::Swim {
                    x,
                    y,
                    frames: flight,
                } => return Some((x, y, frames + flight)),
                Landing::Wall {
                    x: wall_x,
                    y: wall_y,
                    dir: wall,
                    frames: flight,
                } => {
                    (x, y, frames) = (wall_x, wall_y, frames + flight);
                    vx = math::itofix(-wall);
                }
            }
        }
        None
    }

    /// A drop off the ledge at (x, y), provided walking a pixel or two
    /// further before the fall ends the same move: WALK does not stop the
    /// actor exactly at the edge.
    fn robust_drop(&self, x: i32, y: i32, dir: i32) -> Option<(i32, i32, i32)> {
        let (lx, ly, frames) = self.drop(x, y, dir)?;
        let landing = Landing::Stand {
            x: lx,
            y: ly,
            frames,
        };
        (1..=TAKEOFF_SLACK)
            .all(|late| {
                self.drop(x + dir * late, y, dir)
                    .is_some_and(|(ox, oy, of)| {
                        landing.matches(&Landing::Stand {
                            x: ox,
                            y: oy,
                            frames: of,
                        })
                    })
            })
            .then_some((lx, ly, frames))
    }

    fn jump(&self, x: i32, y: i32, dir: i32) -> Option<Landing> {
        self.fly(x, y, self.actor.walk_speed * dir, -self.actor.jump_speed)
    }

    /// The ObjectComJump arc from (x, y), provided taking off a couple of
    /// pixels early or late ends the same move.
    fn robust_jump(&self, x: i32, y: i32, dir: i32) -> Option<Landing> {
        let landing = self.jump(x, y, dir)?;
        (-TAKEOFF_SLACK..=TAKEOFF_SLACK)
            .filter(|&offset| offset != 0 && self.standing(x + offset, y))
            .all(|offset| {
                self.jump(x + offset, y, dir)
                    .is_some_and(|other| landing.matches(&other))
            })
            .then_some(landing)
    }

    /// C4Shape::Attach (C4Shape.cpp:196-226) towards the vertices on `side`:
    /// each, in vertex order, looks along `(dx, dy)` from AttachRange pixels
    /// short of itself to AttachRange - 1 beyond, and the actor moves to rest
    /// against the first solid pixel it finds; a later vertex overrides an
    /// earlier one. `None` when no vertex finds any: the attachment is lost.
    fn attach(&self, x: i32, y: i32, side: u32, (dx, dy): (i32, i32)) -> Option<(i32, i32)> {
        let width = self.landscape.width().min(i32::MAX as u32) as i32;
        self.actor
            .body
            .vertices()
            .filter(|(_, _, cnat)| cnat & side != 0)
            .fold(None, |attached, (vx, vy, _)| {
                let (cx, cy) = attached.unwrap_or((x, y));
                (-ATTACH_RANGE..ATTACH_RANGE)
                    .find(|&step| {
                        let ax = cx + vx + (step + 1) * dx;
                        (0..width).contains(&ax)
                            && self.landscape.is_solid_at(ax, cy + vy + (step + 1) * dy)
                    })
                    .map(|step| (cx + step * dx, cy + step * dy))
                    .or(attached)
            })
    }

    /// Scale up the wall on `dir` from (x, y) as the engine's attached
    /// movement does (C4Movement.cpp:324-369): every pixel up re-attaches the
    /// actor to the face, so it follows a face that leans. Where the face
    /// ends, a corner scale puts it on top. Returns the standing position
    /// there and the frames it all takes.
    fn climb(&self, x: i32, y: i32, dir: i32) -> Option<(i32, i32, i32)> {
        if !self.actor.can_scale || !self.wall_contact(x, y, dir) {
            return None;
        }
        let side = facing_side(dir);
        let (mut cx, mut cy) = (x, y);
        for pixels in 0..MAX_CLIMB {
            let attached = self.attach(cx, cy - 1, side, (dir, 0));
            let tx = attached.map_or(cx, |(ax, _)| ax);
            if !self.in_bounds(tx, cy - 1) || !self.fits(tx, cy - 1) {
                // A contact aborts the step (C4Movement.cpp:355-362) and
                // ends the climb here. C++ corner-scales a scaler whose foot
                // meets a ledge (C4Object.cpp:4352-4358); leaving that out
                // can only miss a route, never invent one.
                return None;
            }
            (cx, cy) = (tx, cy - 1);
            if attached.is_none() {
                return self.corner_scale(cx, cy, dir, pixels + 1);
            }
        }
        None
    }

    /// ObjectActionCornerScale for a scaler (C4ObjectCom.cpp:167-218): the
    /// actor moves to the first spot up to CornerRange across and up from
    /// (x, y), widest and then highest first, where no vertex with a contact
    /// side is in solid, and KneelUp stands it there. `pixels` is the climb
    /// so far. Returns the standing position and the frames the climb takes.
    fn corner_scale(&self, x: i32, y: i32, dir: i32, pixels: i32) -> Option<(i32, i32, i32)> {
        let (kx, ky) = (1..=CORNER_RANGE)
            .rev()
            .flat_map(|across| {
                (1..=CORNER_RANGE)
                    .rev()
                    .map(move |up| (x + dir * across, y - up))
            })
            .find(|&(kx, ky)| self.in_bounds(kx, ky) && self.corner_free(kx, ky))?;
        let (sx, sy) = self.kneel(kx, ky)?;
        Some((
            sx,
            sy,
            pixels * self.climb_cost / COST_PER_FRAME + KNEEL_UP_FRAMES,
        ))
    }

    /// CornerScaleOkay (C4ObjectCom.cpp:167-180) reads only the contact
    /// sides, so a vertex without one (CLNK's centre) may be in solid.
    fn corner_free(&self, x: i32, y: i32) -> bool {
        const SIDES: u32 = CNAT_LEFT | CNAT_RIGHT | CNAT_TOP | CNAT_BOTTOM;
        self.actor
            .body
            .vertices()
            .filter(|(_, _, cnat)| cnat & SIDES != 0)
            .all(|(vx, vy, _)| !self.landscape.is_solid_at(x + vx, y + vy))
    }

    /// KneelUp at (x, y): DFA_KNEEL attaches to the floor (C4Object.cpp:
    /// 4817-4821). Without a floor in range the engine drops the actor from
    /// there (C4Object.cpp:4277-4315); past a wall too thin to kneel on, it
    /// grabs the far face instead of landing, so that is no route.
    fn kneel(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        self.attach(x, y, CNAT_BOTTOM, (0, 1))
            .map(|(_, floor)| (x, floor))
            .filter(|&(x, floor)| self.standing(x, floor))
    }

    fn heuristic(&self, x: i32, y: i32, goal: NavGoal) -> i32 {
        let dx = ((x - goal.x).abs() - goal.range_x).max(0);
        let dy = ((y - goal.y).abs() - goal.range_y).max(0);
        dx.saturating_mul(self.walk_cost)
            .saturating_add(dy.saturating_mul(2))
    }

    fn successors(&self, x: i32, y: i32, out: &mut Vec<((i32, i32), Edge, i32)>) {
        out.clear();
        if self.swimming(x, y) {
            self.swim_successors(x, y, out);
            return;
        }
        for dir in [-1, 1] {
            let right = dir > 0;
            let mut at_edge = false;
            match self.walk(x, y, dir) {
                WalkStep::Stand(ny) => out.push(((x + dir, ny), Edge::Walk, self.walk_cost)),
                WalkStep::Ledge => {
                    at_edge = true;
                    if let Some((lx, ly, frames)) = self.robust_drop(x, y, dir) {
                        out.push(((lx, ly), Edge::Drop, frames * COST_PER_FRAME));
                    }
                }
                WalkStep::Wall => {
                    at_edge = true;
                    if let Some((tx, ty, frames)) = self.climb(x, y, dir) {
                        out.push(((tx, ty), Edge::Climb { right }, frames * COST_PER_FRAME));
                    }
                }
            }
            if at_edge || x.rem_euclid(4) == 0 {
                match self.robust_jump(x, y, dir) {
                    Some(
                        Landing::Stand {
                            x: lx,
                            y: ly,
                            frames,
                        }
                        | Landing::Swim {
                            x: lx,
                            y: ly,
                            frames,
                        },
                    ) if (lx, ly) != (x, y) => out.push((
                        (lx, ly),
                        Edge::Jump { right },
                        frames * COST_PER_FRAME + JUMP_PENALTY,
                    )),
                    Some(Landing::Wall {
                        x: wx,
                        y: wy,
                        dir: wall_dir,
                        frames,
                    }) => {
                        if let Some((tx, ty, climb)) = self.climb(wx, wy, wall_dir) {
                            out.push((
                                (tx, ty),
                                Edge::JumpClimb {
                                    right: wall_dir > 0,
                                    grab: (wx, wy),
                                },
                                (frames + climb) * COST_PER_FRAME + JUMP_PENALTY,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// From a swimming position: a step through liquid in any of eight
    /// directions, all equally fast because DFA_SWIM limits each axis on its
    /// own (C4Object.cpp:4976-4978), scaling out up a wall the swimmer
    /// pushes against (C4Object.cpp:4458-4467,4516-4526), or going ashore.
    fn swim_successors(&self, x: i32, y: i32, out: &mut Vec<((i32, i32), Edge, i32)>) {
        for (dx, dy) in [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            if self.swimming(x + dx, y + dy) {
                out.push(((x + dx, y + dy), Edge::Swim, self.swim_cost));
            }
        }
        for dir in [-1, 1] {
            if let Some((tx, ty, frames)) = self.climb(x, y, dir) {
                out.push((
                    (tx, ty),
                    Edge::Climb { right: dir > 0 },
                    frames * COST_PER_FRAME,
                ));
            }
            if let Some((tx, ty)) = self.ashore(x, y, dir) {
                out.push((
                    (tx, ty),
                    Edge::Ashore { right: dir > 0 },
                    KNEEL_UP_FRAMES * COST_PER_FRAME,
                ));
            }
        }
    }

    /// A swimmer at the surface, with nothing liquid above its centre, whose
    /// step on in `dir` puts its bottom vertex in solid corner-scales out
    /// (C4Object.cpp:4375-4379): it moves to the first spot two to
    /// CornerRange pixels across and as many up, nearest first, where no
    /// vertex with a contact side is in solid, and KneelUp stands it there
    /// (C4ObjectCom.cpp:191-218). Returns the standing position.
    fn ashore(&self, x: i32, y: i32, dir: i32) -> Option<(i32, i32)> {
        let grounded = self
            .actor
            .body
            .vertices()
            .filter(|(_, _, cnat)| cnat & CNAT_BOTTOM != 0)
            .any(|(vx, vy, _)| self.landscape.is_solid_at(x + dir + vx, y + vy));
        if !grounded || self.landscape.is_liquid_at(x, y - 1) {
            return None;
        }
        let (kx, ky) = (2..=CORNER_RANGE)
            .map(|range| (x + dir * range, y - range))
            .find(|&(kx, ky)| self.in_bounds(kx, ky) && self.corner_free(kx, ky))?;
        self.kneel(kx, ky)
    }

    fn run(&self, start: (i32, i32), goal: NavGoal, budget: usize) -> Option<NavPlan> {
        let mut best: HashMap<(i32, i32), i32> = HashMap::new();
        let mut links: HashMap<(i32, i32), Link> = HashMap::new();
        // Cost each position's best route has spent since its last breath.
        let mut breathless: HashMap<(i32, i32), i32> = HashMap::new();
        let mut open = BinaryHeap::new();
        best.insert(start, 0);
        breathless.insert(start, 0);
        open.push(Reverse((
            self.heuristic(start.0, start.1, goal),
            0,
            start.0,
            start.1,
        )));
        let mut successors = Vec::new();
        let mut expansions = 0usize;
        while let Some(Reverse((_, g, x, y))) = open.pop() {
            if best.get(&(x, y)).is_some_and(|&known| known < g) {
                continue;
            }
            if goal.contains(x, y) {
                return Some(self.reconstruct(start, (x, y), &links, g));
            }
            expansions += 1;
            if expansions > budget {
                return None;
            }
            self.successors(x, y, &mut successors);
            let origin_breathes = self.breathing(x, y);
            let origin_held = breathless.get(&(x, y)).copied().unwrap_or(0);
            for &(next, edge, cost) in &successors {
                if !self.in_bounds(next.0, next.1) {
                    continue;
                }
                // A move between two breathing positions takes no breath; any
                // other counts in full against the budget.
                let next_breathes = self.breathing(next.0, next.1);
                let held = match (origin_breathes, next_breathes) {
                    (true, true) => 0,
                    (true, false) => cost,
                    (false, _) => origin_held.saturating_add(cost),
                };
                if held > self.breath_budget {
                    continue;
                }
                let next_g = g.saturating_add(cost);
                if best.get(&next).is_some_and(|&known| known <= next_g) {
                    continue;
                }
                best.insert(next, next_g);
                breathless.insert(next, if next_breathes { 0 } else { held });
                links.insert(
                    next,
                    Link {
                        from: (x, y),
                        edge,
                        cost,
                    },
                );
                open.push(Reverse((
                    next_g.saturating_add(self.heuristic(next.0, next.1, goal)),
                    next_g,
                    next.0,
                    next.1,
                )));
            }
        }
        None
    }

    fn reconstruct(
        &self,
        start: (i32, i32),
        end: (i32, i32),
        links: &HashMap<(i32, i32), Link>,
        cost: i32,
    ) -> NavPlan {
        let mut edges = Vec::new();
        let mut node = end;
        while node != start {
            let link = links[&node];
            edges.push((link.from, node, link.edge, link.cost));
            node = link.from;
        }
        edges.reverse();

        let mut waypoints: Vec<NavWaypoint> = Vec::new();
        // A walk run's cost is summed before it becomes frames: one pixel
        // costs less than a frame, so converting per edge would truncate.
        let mut walk_cost = 0;
        let mut walking_to: Option<(i32, i32)> = None;
        let flush_walk = |waypoints: &mut Vec<NavWaypoint>,
                          walking_to: &mut Option<(i32, i32)>,
                          walk_cost: &mut i32| {
            if let Some((x, y)) = walking_to.take() {
                waypoints.push(NavWaypoint {
                    x,
                    y,
                    movement: NavMove::Walk,
                    right: false,
                    frames: (*walk_cost + COST_PER_FRAME - 1) / COST_PER_FRAME,
                });
            }
            *walk_cost = 0;
        };
        // A swim run's positions, each with the cost of the step into it.
        let mut swim_path: Vec<((i32, i32), i32)> = Vec::new();
        for (from, to, edge, cost) in edges {
            let frames = cost / COST_PER_FRAME;
            if edge != Edge::Swim && !swim_path.is_empty() {
                waypoints.extend(self.swim_waypoints(&swim_path));
                swim_path.clear();
            }
            match edge {
                Edge::Walk => {
                    walking_to = Some(to);
                    walk_cost += cost;
                }
                Edge::Swim => {
                    flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
                    if swim_path.is_empty() {
                        swim_path.push((from, 0));
                    }
                    swim_path.push((to, cost));
                }
                Edge::Drop => {
                    flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
                    waypoints.push(NavWaypoint {
                        x: to.0,
                        y: to.1,
                        movement: NavMove::Drop,
                        right: to.0 >= from.0,
                        frames,
                    });
                }
                Edge::Jump { right } => {
                    flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
                    waypoints.push(NavWaypoint {
                        x: to.0,
                        y: to.1,
                        movement: NavMove::Jump,
                        right,
                        frames,
                    });
                }
                Edge::Climb { right } => {
                    flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
                    waypoints.push(NavWaypoint {
                        x: to.0,
                        y: to.1,
                        movement: NavMove::Climb,
                        right,
                        frames,
                    });
                }
                Edge::Ashore { right } => {
                    flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
                    waypoints.push(NavWaypoint {
                        x: to.0,
                        y: to.1,
                        movement: NavMove::Ashore,
                        right,
                        frames,
                    });
                }
                Edge::JumpClimb { right, grab } => {
                    flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
                    waypoints.push(NavWaypoint {
                        x: grab.0,
                        y: grab.1,
                        movement: NavMove::Jump,
                        right,
                        frames: frames / 2,
                    });
                    waypoints.push(NavWaypoint {
                        x: to.0,
                        y: to.1,
                        movement: NavMove::Climb,
                        right,
                        frames: frames - frames / 2,
                    });
                }
            }
        }
        waypoints.extend(self.swim_waypoints(&swim_path));
        flush_walk(&mut waypoints, &mut walking_to, &mut walk_cost);
        NavPlan { waypoints, cost }
    }

    /// The fewest Swim waypoints along a swim run, each the furthest of its
    /// positions the swimmer reaches in a straight line from the waypoint
    /// before: the executor steers straight at each one. Consecutive
    /// positions are neighbours, so a line always reaches the next.
    fn swim_waypoints(&self, path: &[((i32, i32), i32)]) -> Vec<NavWaypoint> {
        let mut waypoints = Vec::new();
        let mut anchor = 0;
        while anchor + 1 < path.len() {
            let from = path[anchor].0;
            let reach = (anchor + 2..path.len())
                .rev()
                .find(|&index| self.swim_line(from, path[index].0))
                .unwrap_or(anchor + 1);
            let cost: i32 = path[anchor + 1..=reach].iter().map(|&(_, cost)| cost).sum();
            let (x, y) = path[reach].0;
            waypoints.push(NavWaypoint {
                x,
                y,
                movement: NavMove::Swim,
                right: false,
                frames: (cost + COST_PER_FRAME - 1) / COST_PER_FRAME,
            });
            anchor = reach;
        }
        waypoints
    }

    /// Whether every position on the straight line from `from` to `to` is
    /// one the actor swims at.
    fn swim_line(&self, from: (i32, i32), to: (i32, i32)) -> bool {
        let steps = (to.0 - from.0).abs().max((to.1 - from.1).abs()).max(1);
        (0..=steps).all(|step| {
            self.swimming(
                from.0 + (to.0 - from.0) * step / steps,
                from.1 + (to.1 - from.1) * step / steps,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::PixelGrid;

    const G: i32 = 200;
    const W: usize = 480;
    const H: usize = 300;

    /// Solid ground below `G`, then each (x0, y0, x1, y1, solid) rectangle.
    fn terrain(rects: &[(i32, i32, i32, i32, bool)]) -> Landscape {
        flooded_terrain(rects, &[])
    }

    /// [`terrain`], then each (x0, y0, x1, y1) rectangle of `water` filled
    /// with Water.
    fn flooded_terrain(
        rects: &[(i32, i32, i32, i32, bool)],
        water: &[(i32, i32, i32, i32)],
    ) -> Landscape {
        let mut pixels = vec![0u8; W * H];
        let mut set = |x0: i32, y0: i32, x1: i32, y1: i32, pixel: u8| {
            for y in y0.max(0)..=y1.min(H as i32 - 1) {
                for x in x0.max(0)..=x1.min(W as i32 - 1) {
                    pixels[y as usize * W + x as usize] = pixel;
                }
            }
        };
        set(0, G, W as i32 - 1, H as i32 - 1, 1);
        for &(x0, y0, x1, y1, solid) in rects {
            set(x0, y0, x1, y1, u8::from(solid));
        }
        for &(x0, y0, x1, y1) in water {
            set(x0, y0, x1, y1, 2);
        }
        grid_landscape(W, H, pixels, G)
    }

    /// A `width` x `height` landscape of sky (0), Earth (1) and Water (2)
    /// pixels, every column's surface at `surface`.
    fn grid_landscape(width: usize, height: usize, pixels: Vec<u8>, surface: i32) -> Landscape {
        let mut landscape =
            Landscape::with_default_material(width as u32, vec![surface; width], None)
                .expect("navigation landscape");
        landscape.set_world_height(height as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            width as u32,
            height as u32,
            pixels,
            vec![0, 100, 25],
            vec![None, Some("Earth".to_owned()), Some("Water".to_owned())],
            vec![None; 3],
        ));
        landscape
    }

    /// Frontier's landscape (`Missions.c4f/Frontier.c4s`, seed 0) at x
    /// 926..=1010, y 340..=400: a basin at the foot of a slope that rises in
    /// short faces and ledges (clonk-org/clonk-rs#1726). `#` is solid, `~`
    /// water and `.` sky.
    const FRONTIER_SLOPE_LEFT: usize = 926;
    const FRONTIER_SLOPE_TOP: usize = 340;
    const FRONTIER_SLOPE: [&str; 61] = [
        ".....................................................................................", // 340
        ".....................................................................................", // 341
        ".....................................................................................", // 342
        ".....................................................................................", // 343
        ".....................................................................................", // 344
        ".....................................................................................", // 345
        ".....................................................................................", // 346
        ".....................................................................................", // 347
        ".....................................................................................", // 348
        "..........................................................................###########", // 349
        ".........................................................................############", // 350
        "........................................................................#############", // 351
        "......................................................................###############", // 352
        ".....................................................................################", // 353
        "....................................................................#################", // 354
        "....................................................................#################", // 355
        "...................................................................##################", // 356
        "..................................................................###################", // 357
        ".................................................................####################", // 358
        "................................................................#####################", // 359
        "..............................................................#######################", // 360
        ".............................................................########################", // 361
        "............................................................#########################", // 362
        "...........................................................##########################", // 363
        "..........................................................###########################", // 364
        "..........................................................###########################", // 365
        ".........................................................############################", // 366
        "................................................###.....#############################", // 367
        "..............................................########.##############################", // 368
        "............................................#########################################", // 369
        "...........................................##########################################", // 370
        "..........................................###########################################", // 371
        "........................................#############################################", // 372
        ".......................................##############################################", // 373
        "......................................###############################################", // 374
        "......................................###############################################", // 375
        ".....................................################################################", // 376
        "...........................####.....#################################################", // 377
        ".........................########..##################################################", // 378
        "........................#############################################################", // 379
        "........................#############################################################", // 380
        "........................#############################################################", // 381
        "........................#############################################################", // 382
        ".......................##############################################################", // 383
        ".......................##############################################################", // 384
        ".......................##############################################################", // 385
        ".......................##############################################################", // 386
        "......................###############################################################", // 387
        "......................###############################################################", // 388
        ".....................################################################################", // 389
        ".....................################################################################", // 390
        "......................###############################################################", // 391
        "....................#################################################################", // 392
        "...................##################################################################", // 393
        "..................###################################################################", // 394
        "..................###################################################################", // 395
        ".................####################################################################", // 396
        "................#####################################################################", // 397
        "...............######################################################################", // 398
        "....#####.....#######################################################################", // 399
        "~~###################################################################################", // 400
    ];

    /// [`FRONTIER_SLOPE`] at its Frontier coordinates. A pixel outside the
    /// cut repeats the nearest one inside, so the cut's edges add no wall or
    /// ledge the real map lacks.
    fn frontier_slope() -> Landscape {
        let (columns, rows) = (FRONTIER_SLOPE[0].len(), FRONTIER_SLOPE.len());
        let width = FRONTIER_SLOPE_LEFT + columns + 10;
        let height = FRONTIER_SLOPE_TOP + rows + 20;
        let pixel = |x: usize, y: usize| {
            let row = y.saturating_sub(FRONTIER_SLOPE_TOP).min(rows - 1);
            let column = x.saturating_sub(FRONTIER_SLOPE_LEFT).min(columns - 1);
            match FRONTIER_SLOPE[row].as_bytes()[column] {
                b'#' => 1,
                b'~' => 2,
                _ => 0,
            }
        };
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| pixel(x, y)))
            .collect();
        grid_landscape(width, height, pixels, FRONTIER_SLOPE_TOP as i32)
    }

    /// CLNK's DefCore vertices, shape and physicals (Clonk.c4d/DefCore.txt) under
    /// the default 0.2 px/frame² gravity.
    fn clonk(can_scale: bool) -> NavActor {
        let vertices = [
            (0, 2, 0),
            (0, -7, 4),
            (0, 9, 8),
            (-2, -3, 1),
            (2, -3, 2),
            (-4, 3, 1),
            (4, 3, 2),
        ]
        .map(|(x, y, cnat)| ObjectVertex {
            x,
            y,
            cnat,
            friction: 0,
        });
        let physical = PhysicalInfo {
            breath: 50_000,
            walk: 70_000,
            jump: 40_000,
            scale: 30_000,
            swim: 60_000,
            can_scale: i32::from(can_scale),
            ..PhysicalInfo::default()
        };
        NavActor::new(
            NavBody::from_vertices(&vertices),
            &physical,
            FULL_CON,
            C4Fixed::from_raw(13_107),
            -10,
        )
    }

    fn goal(x: i32, y: i32) -> NavGoal {
        NavGoal {
            x,
            y,
            range_x: 3,
            range_y: 6,
        }
    }

    fn moves(plan: &NavPlan) -> Vec<NavMove> {
        plan.waypoints
            .iter()
            .map(|waypoint| waypoint.movement)
            .collect()
    }

    #[test]
    fn clonk_walks_five_pixel_steps_and_scales_from_seven() {
        let body = clonk(true).body;
        assert_eq!(body.feet(), 9);
        assert_eq!(
            body.walk_step(),
            5,
            "measured: five walk, six stall, seven scale"
        );
    }

    #[test]
    fn walks_across_flat_ground() {
        let landscape = terrain(&[]);
        let plan = plan(
            &landscape,
            &clonk(true),
            Vector2::new(100, G - 10),
            goal(300, G - 10),
            20_000,
        )
        .expect("flat route");
        assert_eq!(moves(&plan), vec![NavMove::Walk]);
        let last = plan.waypoints.last().expect("one waypoint");
        assert!((last.x - 300).abs() <= 3 && last.y == G - 10, "{last:?}");
        // About 200px at 1.96 px/frame: the executor's lifetime is built on
        // this, so per-pixel rounding must not truncate it away.
        assert!(
            (95..=110).contains(&last.frames),
            "walk duration {} frames",
            last.frames
        );
    }

    #[test]
    fn walks_up_a_five_pixel_step_but_jumps_a_six_pixel_one() {
        let five = terrain(&[(200, G - 5, W as i32 - 1, G - 1, true)]);
        let plan5 = plan(
            &five,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(300, G - 15),
            20_000,
        )
        .expect("five-pixel step");
        assert_eq!(moves(&plan5), vec![NavMove::Walk]);

        let six = terrain(&[(200, G - 6, W as i32 - 1, G - 1, true)]);
        let plan6 = plan(
            &six,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(300, G - 16),
            20_000,
        )
        .expect("six-pixel step");
        assert!(moves(&plan6).contains(&NavMove::Jump), "{plan6:?}");
    }

    #[test]
    fn scales_a_sixty_pixel_cliff_only_with_can_scale() {
        let cliff = terrain(&[(200, G - 60, W as i32 - 1, G - 1, true)]);
        let scaled = plan(
            &cliff,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(300, G - 70),
            20_000,
        )
        .expect("scalable cliff");
        assert!(moves(&scaled).contains(&NavMove::Climb), "{scaled:?}");
        let top = scaled
            .waypoints
            .iter()
            .find(|waypoint| waypoint.movement == NavMove::Climb)
            .expect("climb");
        assert_eq!((top.x, top.y), (202, G - 70), "measured KneelUp position");

        assert!(
            plan(
                &cliff,
                &clonk(false),
                Vector2::new(150, G - 10),
                goal(300, G - 70),
                20_000
            )
            .is_none(),
            "a 60px face is out of jump reach (38px) for a non-scaler"
        );
    }

    #[test]
    fn climbs_out_of_a_frontier_basin_up_a_face_that_leans_back() {
        // Measured with the engine on Frontier (seed 0), ComDir Right held
        // from the basin floor: SCALE from (942,387) up a face that leans
        // back, a corner scale to (955,366), KneelUp settling a pixel lower,
        // then WALK on up the slope through (970,359).
        let slope = frontier_slope();
        let plan = plan(
            &slope,
            &clonk(true),
            Vector2::new(936, 390),
            goal(970, 359),
            20_000,
        )
        .expect("out of the basin");
        let climb = plan
            .waypoints
            .iter()
            .find(|waypoint| waypoint.movement == NavMove::Climb)
            .expect("a climb");
        assert_eq!((climb.x, climb.y), (955, 367), "{plan:?}");
    }

    #[test]
    fn a_corner_scale_takes_the_widest_spot_the_top_leaves_free() {
        // CheckCornerScale starts CornerRange across and narrows until the
        // body fits (C4ObjectCom.cpp:182-189). A bump five pixels past the
        // edge leaves five: measured with the engine, KneelUp at (200,G-61)
        // settles on the top at (200,G-60).
        let bumped = terrain(&[
            (200, G - 50, W as i32 - 1, G - 1, true),
            (205, G - 60, 215, G - 51, true),
        ]);
        let plan = plan(
            &bumped,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(200, G - 60),
            20_000,
        )
        .expect("onto the top beside the bump");
        let climb = plan
            .waypoints
            .iter()
            .find(|waypoint| waypoint.movement == NavMove::Climb)
            .expect("a climb");
        assert_eq!((climb.x, climb.y), (200, G - 60), "{plan:?}");
    }

    #[test]
    fn reaches_the_bottom_of_a_pit_and_climbs_back_out() {
        let pit = terrain(&[(220, G, 280, G + 59, false)]);
        let down = plan(
            &pit,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(250, G + 50),
            20_000,
        )
        .expect("into the pit");
        let bottom = down.waypoints.last().expect("waypoints");
        assert_eq!(bottom.y, G + 50, "{down:?}");
        assert!(
            matches!(
                bottom.movement,
                NavMove::Drop | NavMove::Jump | NavMove::Walk
            ),
            "{down:?}"
        );
        let up = plan(
            &pit,
            &clonk(true),
            Vector2::new(250, G + 50),
            goal(150, G - 10),
            20_000,
        )
        .expect("out of the pit");
        assert!(moves(&up).contains(&NavMove::Climb), "{up:?}");
    }

    #[test]
    fn drops_down_a_shaft_by_letting_go_of_its_walls() {
        // Walking off into a shaft ends against its far wall, which a scaler
        // grabs (C4Object.cpp:4406-4520). The executor lets go of any grab
        // a Drop did not plan (ObjectComLetGo: ObjectActionJump with a one
        // pixel per frame xdir away from the wall, C4ObjectCom.cpp), so the
        // fall goes on down the shaft.
        let shaft = terrain(&[(200, G, 220, G + 59, false)]);
        let plan = plan(
            &shaft,
            &clonk(true),
            Vector2::new(260, G - 10),
            NavGoal {
                x: 210,
                y: G + 50,
                range_x: 10,
                range_y: 6,
            },
            20_000,
        )
        .expect("down the shaft");
        assert_eq!(moves(&plan), vec![NavMove::Walk, NavMove::Drop], "{plan:?}");
        let bottom = plan.waypoints.last().expect("drop");
        assert_eq!(bottom.y, G + 50, "{plan:?}");
    }

    #[test]
    fn a_fast_wall_contact_tumbles_instead_of_grabbing() {
        // OCF_HitSpeed3 (|xdir| + |ydir| of six, C4Movement.cpp:37;
        // C4Object.cpp:593,2095-2101) turns a flier's wall contact into
        // TUMBLE (C4Object.cpp:4428-4433), which no plan can follow.
        let wall = terrain(&[(220, 0, 239, G - 1, true)]);
        let actor = clonk(true);
        let search = Search::new(&wall, &actor, Vector2::new(150, G - 10), goal(150, G - 10));
        let slow = search.fly(205, G - 60, actor.walk_speed, C4Fixed::ZERO);
        assert!(
            matches!(slow, Some(Landing::Wall { dir: 1, .. })),
            "{slow:?}"
        );
        let fast = search.fly(205, G - 60, actor.walk_speed, math::itofix(5));
        assert!(fast.is_none(), "{fast:?}");
    }

    #[test]
    fn a_side_vertex_caught_on_a_corner_grabs_the_wall() {
        // Contact direction is the touching vertex's CNAT (C4Movement.cpp:
        // 172), so a lower side vertex landing on a corner is a side contact,
        // which a scaler in flight answers with SCALE (C4Object.cpp:4423-4439).
        let shaft = terrain(&[(200, G, 220, G + 59, false)]);
        let actor = clonk(true);
        let search = Search::new(&shaft, &actor, Vector2::new(260, G - 10), goal(210, G + 50));
        let landing = search.fly(219, G - 10, actor.walk_speed * -1, C4Fixed::ZERO);
        assert!(
            matches!(landing, Some(Landing::Wall { dir: -1, .. })),
            "{landing:?}"
        );
    }

    #[test]
    fn a_drop_is_offered_only_where_a_late_takeoff_lands_alike() {
        // WALK carries the actor a pixel or two past the ledge before it
        // falls, so a landing that only holds for the exact takeoff misses
        // its lip and falls on (measured on Frontier: planned (221,130),
        // tumbled into the pit beside it).
        let drops = |landscape: &Landscape| {
            let actor = clonk(true);
            let search = Search::new(
                landscape,
                &actor,
                Vector2::new(199, G - 10),
                goal(215, G + 30),
            );
            let mut edges = Vec::new();
            search.successors(199, G - 10, &mut edges);
            edges
                .into_iter()
                .filter(|(_, edge, _)| *edge == Edge::Drop)
                .map(|(to, _, _)| to)
                .collect::<Vec<_>>()
        };
        let pit = (240, G, 320, H as i32 - 1, false);
        let lip = terrain(&[(200, G, 239, G + 39, false), pit]);
        assert_eq!(drops(&lip), vec![], "the landing is the lip of a pit");

        let floor = terrain(&[(200, G, 320, G + 39, false)]);
        let landed = drops(&floor);
        assert_eq!(landed.len(), 1, "{landed:?}");
        assert_eq!(landed[0].1, G + 30, "{landed:?}");
    }

    #[test]
    fn jumps_a_gap_instead_of_climbing_through_the_chasm() {
        let chasm = terrain(&[(200, G, 229, H as i32 - 1, false)]);
        let plan = plan(
            &chasm,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(300, G - 10),
            20_000,
        )
        .expect("across the gap");
        assert!(moves(&plan).contains(&NavMove::Jump), "{plan:?}");
        assert!(!moves(&plan).contains(&NavMove::Drop), "{plan:?}");
    }

    #[test]
    fn swims_across_a_pool_too_wide_to_jump() {
        // Measured with the engine: walking right off the bank, CLNK falls
        // into the pool, swims to its far wall at about a pixel a frame,
        // scales it, and KneelUps to stand on the far bank at (302,G-10)
        // (clonk-org/clonk-rs#1728).
        let pool = flooded_terrain(&[], &[(200, G, 299, G + 39)]);
        let plan = plan(
            &pool,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(350, G - 10),
            20_000,
        )
        .expect("across the pool");
        assert!(moves(&plan).contains(&NavMove::Swim), "{plan:?}");
        let out = plan
            .waypoints
            .iter()
            .find(|waypoint| waypoint.movement == NavMove::Climb)
            .expect("a climb out");
        assert_eq!((out.x, out.y), (302, G - 10), "{plan:?}");
    }

    /// A pool from x 150 to 449, 40 deep, split by a barrier from the top of
    /// the map down to 20 px below the surface, `thickness` px wide. Only the
    /// tunnel under it joins the two halves.
    fn underwater_tunnel(thickness: i32) -> Landscape {
        let right = 249 + thickness;
        flooded_terrain(
            &[(250, 0, right, G + 19, true)],
            &[
                (150, G, 249, G + 39),
                (right + 1, G, 449, G + 39),
                (250, G + 20, right, G + 39),
            ],
        )
    }

    #[test]
    fn dives_through_a_tunnel_it_can_hold_its_breath_for() {
        let short = underwater_tunnel(30);
        let plan = plan(
            &short,
            &clonk(true),
            Vector2::new(120, G - 10),
            goal(470, G - 10),
            40_000,
        )
        .expect("through the tunnel");
        assert!(moves(&plan).contains(&NavMove::Swim), "{plan:?}");
    }

    #[test]
    fn swim_waypoints_are_straight_lines_through_water() {
        // The executor steers a swimmer straight at each Swim waypoint, so
        // the line to it from the waypoint before must be swimmable all
        // along; here that means diving under the barrier, not through it.
        let short = underwater_tunnel(30);
        let actor = clonk(true);
        let plan = plan(
            &short,
            &actor,
            Vector2::new(120, G - 10),
            goal(470, G - 10),
            40_000,
        )
        .expect("through the tunnel");
        let search = Search::new(&short, &actor, Vector2::new(120, G - 10), goal(470, G - 10));
        for pair in plan.waypoints.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            if to.movement != NavMove::Swim {
                continue;
            }
            let steps = (to.x - from.x).abs().max((to.y - from.y).abs()).max(1);
            let blocked = (0..=steps)
                .map(|step| {
                    (
                        from.x + (to.x - from.x) * step / steps,
                        from.y + (to.y - from.y) * step / steps,
                    )
                })
                .find(|&(x, y)| !search.swimming(x, y));
            assert_eq!(blocked, None, "{from:?} -> {to:?} in {plan:?}");
        }
    }

    #[test]
    fn a_tunnel_longer_than_a_breath_is_no_route() {
        // Without air CLNK loses 2% of its 50000 Breath every five frames,
        // then energy (C4Object.cpp:880-921): 125 frames. The 140 px of
        // tunnel alone take about 146 at its 0.96 px/frame swim.
        let long = underwater_tunnel(140);
        assert!(plan(
            &long,
            &clonk(true),
            Vector2::new(120, G - 10),
            goal(470, G - 10),
            40_000
        )
        .is_none());
    }

    /// Water from x=150 to x=299 below the ground line, 40 px deep, but for
    /// a beach at its right end: the floor rises one pixel for every `run`
    /// across, to meet the ground at x=300.
    fn beach(run: i32) -> Vec<(i32, i32, i32, i32)> {
        (150..=299)
            .map(|x| (x, G, x, G + 40.min((299 - x) / run + 1) - 1))
            .collect()
    }

    #[test]
    fn swims_ashore_up_a_beach_it_cannot_climb() {
        // Measured with the engine at ground 140: a swimmer pushing right
        // along the surface meets this beach's floor with its bottom vertex
        // on the step to x=282, corner-scales out, KneelUps at (283,G-2)
        // and stands at (283,G-1) (clonk-org/clonk-rs#1728).
        let pool = flooded_terrain(&[], &beach(2));
        let plan = plan(
            &pool,
            &clonk(false),
            Vector2::new(120, G - 10),
            goal(350, G - 10),
            40_000,
        )
        .expect("ashore up the beach");
        let ashore = plan
            .waypoints
            .iter()
            .find(|waypoint| waypoint.movement == NavMove::Ashore)
            .expect("an ashore");
        assert_eq!(
            (ashore.x, ashore.y, ashore.right),
            (283, G - 1, true),
            "{plan:?}"
        );
    }

    #[test]
    fn a_floating_island_is_unreachable() {
        let island = terrain(&[(300, G - 80, 340, G - 75, true)]);
        assert!(plan(
            &island,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(320, G - 90),
            20_000
        )
        .is_none());
    }

    #[test]
    fn a_slit_too_low_for_the_body_is_not_a_route_but_a_tunnel_is() {
        let slit = [(270, 0, 281, G - 1, true), (270, G - 10, 281, G - 1, false)];
        let blocked = terrain(&slit);
        assert!(plan(
            &blocked,
            &clonk(true),
            Vector2::new(360, G - 10),
            goal(150, G - 10),
            20_000
        )
        .is_none());

        let mut tunnel = slit.to_vec();
        tunnel.extend([
            (240, G + 10, 312, G + 33, false),
            (292, G, 312, G + 9, false),
            (240, G, 260, G + 9, false),
        ]);
        let detour = terrain(&tunnel);
        let plan = plan(
            &detour,
            &clonk(true),
            Vector2::new(360, G - 10),
            goal(150, G - 10),
            20_000,
        )
        .expect("under the wall");
        let deepest = plan
            .waypoints
            .iter()
            .map(|waypoint| waypoint.y)
            .max()
            .expect("waypoints");
        assert!(deepest > G, "the route goes through the tunnel: {plan:?}");
    }

    #[test]
    fn an_exhausted_budget_answers_no_route() {
        let landscape = terrain(&[]);
        assert!(plan(
            &landscape,
            &clonk(true),
            Vector2::new(100, G - 10),
            goal(400, G - 10),
            50
        )
        .is_none());
    }

    #[test]
    fn plans_are_deterministic() {
        let landscape = terrain(&[
            (220, G, 280, G + 59, false),
            (330, G - 30, W as i32 - 1, G - 1, true),
        ]);
        let first = plan(
            &landscape,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(400, G - 40),
            20_000,
        );
        let second = plan(
            &landscape,
            &clonk(true),
            Vector2::new(150, G - 10),
            goal(400, G - 40),
            20_000,
        );
        assert!(first.is_some());
        assert_eq!(first, second);
    }
}
