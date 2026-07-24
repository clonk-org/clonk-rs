use crate::math::integer_distance;
use crate::{Landscape, ObjectId, TransferZoneState, Vector2};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

const MAX_DEPTH: i32 = 35;
const MAX_CRAWL: i32 = 800;
const MAX_RAY: i32 = 350;
const THRESHOLD: i32 = 10;

const DIRECTION_LEFT: i32 = -1;
const DIRECTION_RIGHT: i32 = 1;

const CRAWL_NO_ATTACH: i32 = 0;
const CRAWL_TOP: i32 = 1;
const CRAWL_RIGHT: i32 = 2;
const CRAWL_BOTTOM: i32 = 3;
const CRAWL_LEFT: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub length: i32,
    pub waypoints: Vec<PathWaypoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathWaypoint {
    pub x: i32,
    pub y: i32,
    pub transfer_target: Option<ObjectId>,
}

/// Presentation-only copy of the most recent `C4PathFinder` search graph.
/// Native keeps these rays on the game-global pathfinder so the viewport can
/// draw them after object rendering (C4PathFinder.cpp:253-289,589-593).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PathfinderDebugRayStatus {
    #[default]
    Launch,
    Crawl,
    Still,
    Failure,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PathfinderDebugRay {
    pub status: PathfinderDebugRayStatus,
    pub start: Vector2,
    pub end: Vector2,
    pub target: Vector2,
    pub crawl_attach: i32,
    pub direction: i32,
    pub uses_transfer_zone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathfinderDebugZone {
    pub owner: ObjectId,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub used: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PathfinderDebugSnapshot {
    pub rays: Vec<PathfinderDebugRay>,
    pub zones: Vec<PathfinderDebugZone>,
}

impl PathfinderDebugSnapshot {
    pub fn is_empty(&self) -> bool {
        self.rays.is_empty() && self.zones.is_empty()
    }
}

pub struct PathFinder<'a> {
    landscape: &'a Landscape,
    zones: Vec<Zone>,
    transfer_zones_enabled: bool,
    level: i32,
    last_debug: PathfinderDebugSnapshot,
}

impl<'a> PathFinder<'a> {
    pub fn new(landscape: &'a Landscape, transfer_zones: &'a [TransferZoneState]) -> Self {
        let zones = transfer_zones
            .iter()
            .map(|state| Zone {
                owner: state.owner,
                x: state.x,
                y: state.y,
                width: state.width,
                height: state.height,
                used: false,
            })
            .collect();
        Self {
            landscape,
            zones,
            transfer_zones_enabled: true,
            level: 1,
            last_debug: PathfinderDebugSnapshot::default(),
        }
    }

    pub fn set_level(&mut self, level: i32) {
        self.level = level.clamp(1, 10);
    }

    pub fn enable_transfer_zones(&mut self, enabled: bool) {
        self.transfer_zones_enabled = enabled;
    }

    pub fn debug_snapshot(&self) -> &PathfinderDebugSnapshot {
        &self.last_debug
    }

    pub fn find(&mut self, from: Vector2, to: Vector2) -> Option<Path> {
        let mut state = PathFinderState::new(
            self.landscape,
            &mut self.zones,
            self.transfer_zones_enabled,
            self.level,
            from,
        );
        if !state.point_free(from.x, from.y) || !state.point_free(to.x, to.y) {
            self.last_debug = state.debug_snapshot();
            return None;
        }
        if !state.add_ray(from.x, from.y, to.x, to.y, 0, DIRECTION_LEFT, None, None) {
            self.last_debug = state.debug_snapshot();
            return None;
        }
        if !state.add_ray(from.x, from.y, to.x, to.y, 0, DIRECTION_RIGHT, None, None) {
            self.last_debug = state.debug_snapshot();
            return None;
        }
        state.run();
        let debug = state.debug_snapshot();
        let result = if state.success {
            Some(state.into_path(to))
        } else {
            None
        };
        self.last_debug = debug;
        result
    }
}

struct Zone {
    owner: ObjectId,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    used: bool,
}

#[derive(Clone, Copy)]
struct ZoneEntryPoint {
    x: i32,
    y: i32,
    found: bool,
}

impl Zone {
    fn contains(&self, x: i32, y: i32) -> bool {
        inside(x - self.x, 0, self.width - 1) && inside(y - self.y, 0, self.height - 1)
    }

    fn entry_point(
        &self,
        state: &PathFinderState<'_>,
        _from_x: i32,
        _from_y: i32,
        mut to_x: i32,
        to_y: i32,
    ) -> ZoneEntryPoint {
        if self.contains(to_x, to_y) {
            if to_x < self.x + self.width / 2 {
                to_x = self.x - 1;
            } else {
                to_x = self.x + self.width;
            }
        }
        let mut rx = clamp(to_x, self.x - 1, self.x + self.width);
        let mut ry = clamp(to_y, self.y - 1, self.y + self.height);
        let mut x1 = rx;
        let mut y1 = ry;
        let mut x2 = rx;
        let mut y2 = ry;
        let mut x_incr1 = 0;
        let mut y_incr1 = -1;
        let mut x_incr2 = 0;
        let mut y_incr2 = 1;
        let perimeter = 2 * self.width + 2 * self.height;
        let mut found = false;
        for _ in 0..perimeter {
            if !state.is_solid(x1, y1) {
                rx = x1;
                ry = y1;
                found = true;
                break;
            }
            if !state.is_solid(x2, y2) {
                rx = x2;
                ry = y2;
                found = true;
                break;
            }
            x1 += x_incr1;
            y1 += y_incr1;
            x2 += x_incr2;
            y2 += y_incr2;
            if y1 < self.y - 1 {
                y1 = self.y - 1;
                x_incr1 = 1;
                y_incr1 = 0;
            }
            if x1 > self.x + self.width {
                x1 = self.x + self.width;
                x_incr1 = 0;
                y_incr1 = 1;
            }
            if y1 > self.y + self.height {
                y1 = self.y + self.height;
                x_incr1 = -1;
                y_incr1 = 0;
            }
            if x1 < self.x - 1 {
                x1 = self.x - 1;
                x_incr1 = 0;
                y_incr1 = -1;
            }
            if y2 < self.y - 1 {
                y2 = self.y - 1;
                x_incr2 = -1;
                y_incr2 = 0;
            }
            if x2 > self.x + self.width {
                x2 = self.x + self.width;
                x_incr2 = 0;
                y_incr2 = -1;
            }
            if y2 > self.y + self.height {
                y2 = self.y + self.height;
                x_incr2 = 1;
                y_incr2 = 0;
            }
            if x2 < self.x - 1 {
                x2 = self.x - 1;
                x_incr2 = 0;
                y_incr2 = 1;
            }
        }
        if found && !inside(rx - self.x, 0, self.width - 1) {
            state.adjust_move_to_target(&mut rx, &mut ry, false, 20);
        }
        // C++ returns the perimeter scan result without revalidating the
        // coordinates changed by AdjustMoveToTarget.
        ZoneEntryPoint {
            x: rx,
            y: ry,
            found,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RayStatus {
    Launch,
    Crawl,
    Still,
    Failure,
    Deleted,
}

struct Ray {
    status: RayStatus,
    x: i32,
    y: i32,
    x2: i32,
    y2: i32,
    target_x: i32,
    target_y: i32,
    crawl_start_x: i32,
    crawl_start_y: i32,
    crawl_attach: i32,
    crawl_start_attach: i32,
    crawl_length: i32,
    direction: i32,
    depth: i32,
    use_zone: Option<usize>,
    from: Option<usize>,
}

impl Ray {
    fn new(
        from_x: i32,
        from_y: i32,
        target_x: i32,
        target_y: i32,
        depth: i32,
        direction: i32,
        from: Option<usize>,
        use_zone: Option<usize>,
    ) -> Self {
        Self {
            status: RayStatus::Launch,
            x: from_x,
            y: from_y,
            x2: from_x,
            y2: from_y,
            target_x,
            target_y,
            crawl_start_x: from_x,
            crawl_start_y: from_y,
            crawl_attach: CRAWL_NO_ATTACH,
            crawl_start_attach: CRAWL_NO_ATTACH,
            crawl_length: 0,
            direction,
            depth,
            use_zone,
            from,
        }
    }
}

struct PathBuilder {
    waypoints: Vec<PathWaypoint>,
    length: i32,
}

impl PathBuilder {
    fn new(start: PathWaypoint) -> Self {
        Self {
            waypoints: vec![start],
            length: 0,
        }
    }

    fn push(&mut self, waypoint: PathWaypoint) {
        if let Some(last) = self.waypoints.last() {
            self.length = self
                .length
                .saturating_add(integer_distance(last.x, last.y, waypoint.x, waypoint.y));
        }
        self.waypoints.push(waypoint);
    }
}

struct PathFinderState<'a> {
    landscape: &'a Landscape,
    width: i32,
    height: i32,
    zones: &'a mut [Zone],
    transfer_zones_enabled: bool,
    level: i32,
    rays: Vec<RefCell<Ray>>,
    active: Vec<usize>,
    success: bool,
    success_ray: Option<usize>,
    builder: PathBuilder,
}

impl<'a> PathFinderState<'a> {
    fn new(
        landscape: &'a Landscape,
        zones: &'a mut [Zone],
        transfer_zones_enabled: bool,
        level: i32,
        start: Vector2,
    ) -> Self {
        for zone in zones.iter_mut() {
            zone.used = false;
        }
        let width = landscape.width().min(i32::MAX as u32) as i32;
        let height = landscape.estimated_height();
        Self {
            landscape,
            width,
            height,
            zones,
            transfer_zones_enabled,
            level: level.clamp(1, 10),
            rays: Vec::new(),
            active: Vec::new(),
            success: false,
            success_ray: None,
            builder: PathBuilder::new(PathWaypoint {
                x: start.x,
                y: start.y,
                transfer_target: None,
            }),
        }
    }

    fn run(&mut self) {
        self.success = false;
        while !self.success {
            if !self.execute_iteration() {
                break;
            }
        }
    }

    fn debug_snapshot(&self) -> PathfinderDebugSnapshot {
        let status = |status| match status {
            RayStatus::Launch => PathfinderDebugRayStatus::Launch,
            RayStatus::Crawl => PathfinderDebugRayStatus::Crawl,
            RayStatus::Still => PathfinderDebugRayStatus::Still,
            RayStatus::Failure => PathfinderDebugRayStatus::Failure,
            RayStatus::Deleted => PathfinderDebugRayStatus::Deleted,
        };
        PathfinderDebugSnapshot {
            rays: self
                .rays
                .iter()
                // Native rays are linked by inserting each allocation at
                // FirstRay, and Draw walks that list from newest to oldest.
                .rev()
                .map(|ray| {
                    let ray = ray.borrow();
                    PathfinderDebugRay {
                        status: status(ray.status),
                        start: Vector2::new(ray.x, ray.y),
                        end: Vector2::new(ray.x2, ray.y2),
                        target: Vector2::new(ray.target_x, ray.target_y),
                        crawl_attach: ray.crawl_attach,
                        direction: ray.direction,
                        uses_transfer_zone: ray.use_zone.is_some(),
                    }
                })
                .collect(),
            zones: self
                .zones
                .iter()
                .map(|zone| PathfinderDebugZone {
                    owner: zone.owner,
                    x: zone.x,
                    y: zone.y,
                    width: zone.width,
                    height: zone.height,
                    used: zone.used,
                })
                .collect(),
        }
    }

    fn into_path(mut self, target: Vector2) -> Path {
        self.builder.push(PathWaypoint {
            x: target.x,
            y: target.y,
            transfer_target: None,
        });
        Path {
            length: self.builder.length,
            waypoints: self.builder.waypoints,
        }
    }

    fn point_free(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height && !self.landscape.is_solid_at(x, y)
    }

    fn is_solid(&self, x: i32, y: i32) -> bool {
        // Unlike the strict LandscapeFree callback used by PointFree,
        // C4Command helpers call GBackSolid and therefore observe configured
        // open/closed borders (C4Command.cpp:98-141; C4Landscape.h:144-161).
        self.landscape.is_solid_at(x, y)
    }

    fn is_semi_solid(&self, x: i32, y: i32) -> bool {
        self.landscape.is_semi_solid_at(x, y)
    }

    fn execute_iteration(&mut self) -> bool {
        let mut continue_search = false;
        let mut processed = 0;
        let snapshot = self.active.clone();
        for index in snapshot {
            if self.success {
                break;
            }
            processed += 1;
            if self.execute_ray(index) {
                continue_search = true;
            }
        }
        if processed >= MAX_RAY {
            return false;
        }
        continue_search
    }

    fn execute_ray(&mut self, index: usize) -> bool {
        let status = self.rays[index].borrow().status;
        match status {
            RayStatus::Launch => {
                self.execute_launch(index);
                true
            }
            RayStatus::Crawl => {
                self.execute_crawl(index);
                true
            }
            RayStatus::Still | RayStatus::Failure | RayStatus::Deleted => false,
        }
    }

    fn execute_launch(&mut self, index: usize) {
        let use_zone = self.rays[index].borrow().use_zone;
        let direction = self.rays[index].borrow().direction;
        let depth = self.rays[index].borrow().depth;
        let target_x = self.rays[index].borrow().target_x;
        let target_y = self.rays[index].borrow().target_y;
        if let Some(zone_index) = use_zone {
            self.zones[zone_index].used = true;
            let target_inside = {
                let mut ray = self.rays[index].borrow_mut();
                if self.zones[zone_index].contains(ray.target_x, ray.target_y) {
                    ray.x2 = ray.target_x;
                    ray.y2 = ray.target_y;
                    true
                } else {
                    false
                }
            };
            if target_inside {
                self.set_complete_path(index);
                self.rays[index].borrow_mut().status = RayStatus::Still;
                return;
            }
            let entry = {
                let ray = self.rays[index].borrow();
                self.zones[zone_index].entry_point(self, ray.x2, ray.y2, ray.target_x, ray.target_y)
            };
            // C++ passes this use-zone ray's X2/Y2 by reference to
            // GetEntryPoint; SetCompletePath later emits that far-side exit
            // for both the Transfer and following MoveTo waypoints
            // (C4PathFinder.cpp:128-139,383-400).
            {
                let mut ray = self.rays[index].borrow_mut();
                ray.x2 = entry.x;
                ray.y2 = entry.y;
            }
            if !entry.found {
                self.rays[index].borrow_mut().status = RayStatus::Failure;
                return;
            }
            if !self.add_ray(
                entry.x,
                entry.y,
                target_x,
                target_y,
                depth + 1,
                direction,
                Some(index),
                None,
            ) {
                self.rays[index].borrow_mut().status = RayStatus::Failure;
                return;
            }
            self.rays[index].borrow_mut().status = RayStatus::Still;
            return;
        }

        let mut last_x = self.rays[index].borrow().x2;
        let mut last_y = self.rays[index].borrow().y2;
        let path_result = self.path_free(true, &mut last_x, &mut last_y, target_x, target_y);
        // C++ passes X2/Y2 by reference, so a blocked ray keeps the final
        // free pixel reached by PathFree and begins crawling at the actual
        // obstacle boundary (C4PathFinder.cpp:112-149,263-309).
        {
            let mut ray = self.rays[index].borrow_mut();
            ray.x2 = last_x;
            ray.y2 = last_y;
        }
        if path_result.free {
            self.set_complete_path(index);
            self.rays[index].borrow_mut().status = RayStatus::Still;
            return;
        }
        if let Some(zone_index) = path_result.zone {
            let (start_x, start_y) = {
                let ray = self.rays[index].borrow();
                if self.zones[zone_index].contains(ray.x, ray.y) {
                    (ray.x2, ray.y2)
                } else {
                    let entry =
                        self.zones[zone_index].entry_point(self, ray.x2, ray.y2, ray.x2, ray.y2);
                    (entry.x, entry.y)
                }
            };
            // The initial zone-entry adjustment also mutates the parent
            // ray's X2/Y2 in C++; retaining it is required for the later
            // backtrace/waypoint coordinates (C4PathFinder.cpp:153-161).
            {
                let mut ray = self.rays[index].borrow_mut();
                ray.x2 = start_x;
                ray.y2 = start_y;
            }
            if !self.add_ray(
                start_x,
                start_y,
                target_x,
                target_y,
                depth + 1,
                direction,
                Some(index),
                Some(zone_index),
            ) {
                self.rays[index].borrow_mut().status = RayStatus::Failure;
                return;
            }
            self.rays[index].borrow_mut().status = RayStatus::Still;
            return;
        }

        {
            let mut ray = self.rays[index].borrow_mut();
            ray.status = RayStatus::Crawl;
            ray.crawl_start_x = ray.x2;
            ray.crawl_start_y = ray.y2;
            ray.crawl_attach = self.find_crawl_attach(ray.x2, ray.y2);
            if ray.crawl_attach == CRAWL_NO_ATTACH {
                ray.crawl_attach = self.find_crawl_attach_diagonal(ray.x2, ray.y2, ray.direction);
            }
            ray.crawl_start_attach = ray.crawl_attach;
            ray.crawl_length = 0;
            if ray.crawl_attach == CRAWL_NO_ATTACH {
                ray.status = RayStatus::Failure;
            }
        }
    }

    fn execute_crawl(&mut self, index: usize) {
        if self.rays[index].borrow().crawl_attach == CRAWL_NO_ATTACH {
            self.rays[index].borrow_mut().status = RayStatus::Failure;
            return;
        }
        let last_x = self.rays[index].borrow().x2;
        let last_y = self.rays[index].borrow().y2;
        if !self.crawl(index) {
            self.rays[index].borrow_mut().status = RayStatus::Failure;
            return;
        }
        let returned_to_start = {
            let ray = self.rays[index].borrow();
            ray.x2 == ray.crawl_start_x
                && ray.y2 == ray.crawl_start_y
                && ray.crawl_attach == ray.crawl_start_attach
        };
        if returned_to_start {
            // C++ marks the exhausted ray still after the read-only cycle
            // check (C4PathFinder.cpp:193-197); end the RefCell read before
            // mutating the ported ray state.
            self.rays[index].borrow_mut().status = RayStatus::Still;
            return;
        }
        if self.transfer_zones_enabled {
            let (x2, y2) = {
                let ray = self.rays[index].borrow();
                (ray.x2, ray.y2)
            };
            if let Some(zone_index) = self.find_zone(x2, y2) {
                if !self.zones[zone_index].used {
                    let entry = self.zones[zone_index].entry_point(self, x2, y2, x2, y2);
                    if !entry.found {
                        // C4PF_Ray_Crawl returns for this pass even when
                        // GetEntryPoint fails, leaving the ray crawling
                        // (C4PathFinder.cpp:198-212).
                        return;
                    }
                    let target_x = self.rays[index].borrow().target_x;
                    let target_y = self.rays[index].borrow().target_y;
                    let depth = self.rays[index].borrow().depth;
                    let direction = self.rays[index].borrow().direction;
                    if !self.add_ray(
                        entry.x,
                        entry.y,
                        target_x,
                        target_y,
                        depth + 1,
                        direction,
                        Some(index),
                        Some(zone_index),
                    ) {
                        self.rays[index].borrow_mut().status = RayStatus::Failure;
                        return;
                    }
                    return;
                }
            }
        }
        {
            let mut ray = self.rays[index].borrow_mut();
            ray.crawl_length += 1;
            if ray.crawl_length >= MAX_CRAWL * self.level {
                ray.status = RayStatus::Still;
                return;
            }
        }
        let (mut start_x, mut start_y) = {
            let ray = self.rays[index].borrow();
            (ray.x, ray.y)
        };
        let (x2, y2) = {
            let ray = self.rays[index].borrow();
            (ray.x2, ray.y2)
        };
        if !self
            .path_free(false, &mut start_x, &mut start_y, x2, y2)
            .free
            && !self.split_ray(index, last_x, last_y)
        {
            self.rays[index].borrow_mut().status = RayStatus::Failure;
            return;
        }

        let crawl_length = self.rays[index].borrow().crawl_length;
        if crawl_length > THRESHOLD {
            let target_x = self.rays[index].borrow().target_x;
            let target_y = self.rays[index].borrow().target_y;
            let mut from_x = x2;
            let mut from_y = y2;
            let direct = self.path_free(false, &mut from_x, &mut from_y, target_x, target_y);
            let start_dist = integer_distance(from_x, from_y, x2, y2);
            let crawl_start = {
                let ray = self.rays[index].borrow();
                (ray.crawl_start_x, ray.crawl_start_y)
            };
            let new_dist = integer_distance(from_x, from_y, crawl_start.0, crawl_start.1);
            let old_dist = integer_distance(x2, y2, crawl_start.0, crawl_start.1);
            if direct.free || (start_dist > THRESHOLD && new_dist > old_dist) {
                let depth = self.rays[index].borrow().depth + 1;
                if !self.add_ray(
                    x2,
                    y2,
                    target_x,
                    target_y,
                    depth,
                    DIRECTION_LEFT,
                    Some(index),
                    None,
                ) || !self.add_ray(
                    x2,
                    y2,
                    target_x,
                    target_y,
                    depth,
                    DIRECTION_RIGHT,
                    Some(index),
                    None,
                ) {
                    self.rays[index].borrow_mut().status = RayStatus::Failure;
                    return;
                }
                self.rays[index].borrow_mut().status = RayStatus::Still;
            }
        }
    }

    fn crawl(&mut self, index: usize) -> bool {
        let attach = self.rays[index].borrow().crawl_attach;
        if attach == CRAWL_NO_ATTACH {
            return false;
        }
        if self.rays[index].borrow().crawl_length > 0 && !self.is_crawl_attach_current(index) {
            {
                let mut ray = self.rays[index].borrow_mut();
                let (mut x, mut y) = (ray.x2, ray.y2);
                crawl_to_attach(&mut x, &mut y, attach);
                ray.x2 = x;
                ray.y2 = y;
                let new_attach = turn_attach(attach, -ray.direction);
                if !self.is_crawl_attach_at(ray.x2, ray.y2, new_attach) {
                    return false;
                }
                ray.crawl_attach = new_attach;
            }
            return true;
        }

        let mut turned = 0;
        while !self.crawl_target_free(index) {
            {
                let mut ray = self.rays[index].borrow_mut();
                ray.crawl_attach = turn_attach(ray.crawl_attach, ray.direction);
                turned += 1;
            }
            if turned == 4 {
                return false;
            }
        }

        self.crawl_by_attach(index);
        true
    }

    fn crawl_target_free(&self, index: usize) -> bool {
        let ray = self.rays[index].borrow();
        let mut x = ray.x2;
        let mut y = ray.y2;
        crawl_by_attach_coords(&mut x, &mut y, ray.crawl_attach, ray.direction);
        drop(ray);
        self.point_free(x, y)
    }

    fn crawl_by_attach(&mut self, index: usize) {
        let (attach, direction, mut x2, mut y2) = {
            let ray = self.rays[index].borrow();
            (ray.crawl_attach, ray.direction, ray.x2, ray.y2)
        };
        let mut ray = self.rays[index].borrow_mut();
        crawl_by_attach_coords(&mut x2, &mut y2, attach, direction);
        ray.x2 = x2;
        ray.y2 = y2;
    }

    fn is_crawl_attach_current(&self, index: usize) -> bool {
        let ray = self.rays[index].borrow();
        self.is_crawl_attach_at(ray.x2, ray.y2, ray.crawl_attach)
    }

    fn is_crawl_attach_at(&self, x: i32, y: i32, attach: i32) -> bool {
        if attach == CRAWL_NO_ATTACH {
            return false;
        }
        let mut cx = x;
        let mut cy = y;
        crawl_to_attach(&mut cx, &mut cy, attach);
        !self.point_free(cx, cy)
    }

    fn find_crawl_attach(&self, x: i32, y: i32) -> i32 {
        if !self.point_free(x, y - 1) {
            return CRAWL_TOP;
        }
        if !self.point_free(x, y + 1) {
            return CRAWL_BOTTOM;
        }
        if !self.point_free(x - 1, y) {
            return CRAWL_LEFT;
        }
        if !self.point_free(x + 1, y) {
            return CRAWL_RIGHT;
        }
        CRAWL_NO_ATTACH
    }

    fn find_crawl_attach_diagonal(&self, x: i32, y: i32, direction: i32) -> i32 {
        if direction == DIRECTION_LEFT {
            if !self.point_free(x - 1, y - 1) {
                return CRAWL_TOP;
            }
            if !self.point_free(x - 1, y + 1) {
                return CRAWL_LEFT;
            }
            if !self.point_free(x + 1, y - 1) {
                return CRAWL_RIGHT;
            }
            if !self.point_free(x + 1, y + 1) {
                return CRAWL_BOTTOM;
            }
        } else if direction == DIRECTION_RIGHT {
            if !self.point_free(x - 1, y - 1) {
                return CRAWL_LEFT;
            }
            if !self.point_free(x - 1, y + 1) {
                return CRAWL_BOTTOM;
            }
            if !self.point_free(x + 1, y - 1) {
                return CRAWL_TOP;
            }
            if !self.point_free(x + 1, y + 1) {
                return CRAWL_RIGHT;
            }
        }
        CRAWL_NO_ATTACH
    }

    fn add_ray(
        &mut self,
        from_x: i32,
        from_y: i32,
        target_x: i32,
        target_y: i32,
        depth: i32,
        direction: i32,
        from: Option<usize>,
        use_zone: Option<usize>,
    ) -> bool {
        if depth >= MAX_DEPTH * self.level {
            return false;
        }
        let index = self.rays.len();
        self.rays.push(RefCell::new(Ray::new(
            from_x, from_y, target_x, target_y, depth, direction, from, use_zone,
        )));
        self.active.insert(0, index);
        true
    }

    fn split_ray(&mut self, index: usize, at_x: i32, at_y: i32) -> bool {
        let depth = self.rays[index].borrow().depth;
        if depth >= MAX_DEPTH * self.level {
            return false;
        }
        let direction = self.rays[index].borrow().direction;
        let target_x = self.rays[index].borrow().target_x;
        let target_y = self.rays[index].borrow().target_y;
        let from = self.rays[index].borrow().from;
        let new_index = self.rays.len();
        let mut new_ray = Ray::new(
            self.rays[index].borrow().x,
            self.rays[index].borrow().y,
            target_x,
            target_y,
            depth,
            direction,
            from,
            None,
        );
        new_ray.status = RayStatus::Still;
        new_ray.x2 = at_x;
        new_ray.y2 = at_y;
        self.rays.push(RefCell::new(new_ray));
        self.active.insert(0, new_index);
        {
            let mut ray = self.rays[index].borrow_mut();
            ray.from = Some(new_index);
            ray.x = at_x;
            ray.y = at_y;
        }
        true
    }

    fn find_zone(&self, x: i32, y: i32) -> Option<usize> {
        self.zones.iter().position(|zone| zone.contains(x, y))
    }

    fn path_free(
        &self,
        detect_zone: bool,
        start_x: &mut i32,
        start_y: &mut i32,
        target_x: i32,
        target_y: i32,
    ) -> PathCheck {
        let x = *start_x;
        let y = *start_y;
        if (target_x - x).abs() < (target_y - y).abs() {
            let xincr = if target_x > x {
                1
            } else if target_x < x {
                -1
            } else {
                0
            };
            let yincr = if target_y > y {
                1
            } else if target_y < y {
                -1
            } else {
                0
            };
            let dy = (target_y - y).abs();
            let dx = (target_x - x).abs();
            let mut d = 2 * dx - dy;
            let aincr = 2 * (dx - dy);
            let bincr = 2 * dx;
            let mut xi = x;
            let mut yi = y;
            while yi != target_y {
                if self.point_free(xi, yi) {
                    *start_x = xi;
                    *start_y = yi;
                } else {
                    return PathCheck {
                        free: false,
                        zone: None,
                    };
                }
                if detect_zone && self.transfer_zones_enabled {
                    if let Some(zone) = self.find_zone(xi, yi) {
                        return PathCheck {
                            free: false,
                            zone: Some(zone),
                        };
                    }
                }
                if d >= 0 {
                    xi += xincr;
                    d += aincr;
                } else {
                    d += bincr;
                }
                yi += yincr;
            }
        } else {
            let yincr = if target_y > y {
                1
            } else if target_y < y {
                -1
            } else {
                0
            };
            let xincr = if target_x > x {
                1
            } else if target_x < x {
                -1
            } else {
                0
            };
            let dx = (target_x - x).abs();
            let dy = (target_y - y).abs();
            let mut d = 2 * dy - dx;
            let aincr = 2 * (dy - dx);
            let bincr = 2 * dy;
            let mut xi = x;
            let mut yi = y;
            while xi != target_x {
                if self.point_free(xi, yi) {
                    *start_x = xi;
                    *start_y = yi;
                } else {
                    return PathCheck {
                        free: false,
                        zone: None,
                    };
                }
                if detect_zone && self.transfer_zones_enabled {
                    if let Some(zone) = self.find_zone(xi, yi) {
                        return PathCheck {
                            free: false,
                            zone: Some(zone),
                        };
                    }
                }
                if d >= 0 {
                    yi += yincr;
                    d += aincr;
                } else {
                    d += bincr;
                }
                xi += xincr;
            }
        }
        PathCheck {
            free: true,
            zone: None,
        }
    }

    fn set_complete_path(&mut self, index: usize) {
        if self.success {
            return;
        }
        let mut current = index;
        loop {
            while self.check_back_ray_shorten(current) {}
            let from_index = match self.rays[current].borrow().from {
                Some(value) => value,
                None => break,
            };
            let transfer_target = self.rays[current].borrow().use_zone;
            if let Some(zone_index) = transfer_target {
                let (x, y) = {
                    let ray = self.rays[current].borrow();
                    (ray.x2, ray.y2)
                };
                let owner = self.zones[zone_index].owner;
                self.builder.push(PathWaypoint {
                    x,
                    y,
                    transfer_target: Some(owner),
                });
            } else {
                let (x, y) = {
                    let parent = self.rays[from_index].borrow();
                    (parent.x2, parent.y2)
                };
                self.builder.push(PathWaypoint {
                    x,
                    y,
                    transfer_target: None,
                });
            }
            current = from_index;
        }
        self.success = true;
        self.success_ray = Some(index);
    }

    fn check_back_ray_shorten(&mut self, index: usize) -> bool {
        let from_index = match self.rays[index].borrow().from {
            Some(value) => value,
            None => return false,
        };
        let x = self.rays[index].borrow().x;
        let y = self.rays[index].borrow().y;
        let mut ancestor = Some(from_index);
        while let Some(parent_index) = ancestor {
            if self.rays[parent_index].borrow().use_zone.is_some() {
                return false;
            }
            if parent_index != from_index {
                let mut px = x;
                let mut py = y;
                let parent_coords = {
                    let parent = self.rays[parent_index].borrow();
                    (parent.x, parent.y)
                };
                if self
                    .path_free(false, &mut px, &mut py, parent_coords.0, parent_coords.1)
                    .free
                {
                    let mut walker = from_index;
                    while walker != parent_index {
                        self.rays[walker].borrow_mut().status = RayStatus::Deleted;
                        walker = match self.rays[walker].borrow().from {
                            Some(next) => next,
                            None => break,
                        };
                    }
                    self.rays[parent_index].borrow_mut().x2 = x;
                    self.rays[parent_index].borrow_mut().y2 = y;
                    self.rays[index].borrow_mut().from = Some(parent_index);
                    return true;
                }
            }
            ancestor = self.rays[parent_index].borrow().from;
        }
        false
    }

    fn adjust_move_to_target(&self, x: &mut i32, y: &mut i32, free_move: bool, shape_height: i32) {
        let mut ty = *y;
        while ty >= 0 && self.is_solid(*x, ty) {
            ty -= 1;
        }
        if ty >= 0 {
            *y = ty;
        }
        if free_move {
            return;
        }
        if !self.is_semi_solid(*x, *y) {
            let mut ny = *y;
            let limit = self.height;
            while ny < limit && !self.is_semi_solid(*x, ny + 1) {
                ny += 1;
            }
            if ny < limit {
                *y = ny;
            }
        }
        if (self.is_solid(*x, *y + 1) || self.is_solid(*x, *y + 5))
            && !self.is_semi_solid(*x, *y - shape_height / 2)
        {
            *y -= shape_height / 2;
        }
    }
}

struct PathCheck {
    free: bool,
    zone: Option<usize>,
}

fn clamp(value: i32, min_value: i32, max_value: i32) -> i32 {
    // C++ BoundBy intentionally does not normalize inverted bounds; negative
    // transfer-zone extents can produce them (C4Math.h:23).
    if value < min_value {
        min_value
    } else if value > max_value {
        max_value
    } else {
        value
    }
}

fn inside(value: i32, min_value: i32, max_value: i32) -> bool {
    value >= min_value && value <= max_value
}

fn turn_attach(attach: i32, direction: i32) -> i32 {
    let mut result = attach + direction;
    if result > CRAWL_LEFT {
        result = CRAWL_TOP;
    }
    if result < CRAWL_TOP {
        result = CRAWL_LEFT;
    }
    result
}

fn crawl_to_attach(x: &mut i32, y: &mut i32, attach: i32) {
    match attach {
        CRAWL_TOP => *y -= 1,
        CRAWL_BOTTOM => *y += 1,
        CRAWL_LEFT => *x -= 1,
        CRAWL_RIGHT => *x += 1,
        _ => {}
    }
}

fn crawl_by_attach_coords(x: &mut i32, y: &mut i32, attach: i32, direction: i32) {
    match attach {
        CRAWL_TOP => *x += direction,
        CRAWL_BOTTOM => *x -= direction,
        CRAWL_LEFT => *y -= direction,
        CRAWL_RIGHT => *y += direction,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::PixelGrid;
    use crate::{Landscape, TransferZoneState};

    #[test]
    fn finds_direct_path_without_obstacles() {
        let landscape = Landscape::flat(32, 40);
        let mut finder = PathFinder::new(&landscape, &[]);
        let path = finder
            .find(Vector2::new(2, 5), Vector2::new(20, 5))
            .expect("path exists");
        let first = path.waypoints.first().expect("has start");
        assert_eq!((first.x, first.y), (2, 5));
        let last = path.waypoints.last().expect("has target");
        assert_eq!((last.x, last.y), (20, 5));
        assert_eq!(path.length, integer_distance(2, 5, 20, 5));
    }

    #[test]
    fn l140_retains_the_latest_search_graph_for_viewport_debug_drawing() {
        let landscape = Landscape::flat(32, 40);
        let mut finder = PathFinder::new(&landscape, &[]);
        finder
            .find(Vector2::new(2, 5), Vector2::new(20, 5))
            .expect("direct path exists");

        let graph = finder.debug_snapshot();
        assert_eq!(graph.rays.len(), 2);
        assert_eq!(graph.rays[0].start, Vector2::new(2, 5));
        assert_eq!(graph.rays[0].target, Vector2::new(20, 5));
        assert_eq!(graph.rays[0].direction, DIRECTION_RIGHT);
        assert!(
            graph
                .rays
                .iter()
                .any(|ray| ray.status == PathfinderDebugRayStatus::Still),
            "the successful direction is retained with its final status"
        );
        assert!(
            graph
                .rays
                .iter()
                .any(|ray| ray.status == PathfinderDebugRayStatus::Launch),
            "native stops the opposite initial direction once success is found"
        );
        assert!(graph.zones.is_empty());
    }

    #[test]
    fn no_path_when_start_inside_solid() {
        let landscape = Landscape::flat(16, 20);
        let mut finder = PathFinder::new(&landscape, &[]);
        let result = finder.find(Vector2::new(4, 25), Vector2::new(8, 25));
        assert!(result.is_none());
    }

    #[test]
    fn point_free_keeps_strict_bounds_when_the_landscape_border_is_open() {
        // LandscapeFree rejects out-of-map points before reading GBackDensity
        // (C4Game.cpp:2288-2292), even when GetPix would return sky for an
        // open scenario border (C4Landscape.h:144-161).
        let mut landscape = Landscape::flat(16, 20);
        landscape.set_border_open(i32::MAX, i32::MAX, true, true);
        let mut finder = PathFinder::new(&landscape, &[]);

        assert!(finder
            .find(Vector2::new(-1, 5), Vector2::new(8, 5))
            .is_none());
        assert!(finder
            .find(Vector2::new(4, 5), Vector2::new(16, 5))
            .is_none());
        assert!(finder
            .find(Vector2::new(4, -1), Vector2::new(8, 5))
            .is_none());
        assert!(finder
            .find(Vector2::new(4, 5), Vector2::new(8, 20))
            .is_none());
    }

    #[test]
    fn max_ray_final_pass_executes_entire_snapshot_before_stopping() {
        // C4PathFinder::Execute checks C4PF_MaxRay only after walking the
        // complete pass-start list. A successful ray beyond position 350
        // therefore still completes the path before Execute returns false
        // (C4PathFinder.cpp:576-599).
        let landscape = Landscape::flat(32, 40);
        let target = Vector2::new(20, 5);
        for ray_count in [MAX_RAY as usize, MAX_RAY as usize + 1] {
            let mut zones = [];
            let mut state =
                PathFinderState::new(&landscape, &mut zones, true, 1, Vector2::new(2, 5));
            for _ in 0..ray_count {
                assert!(state.add_ray(2, 5, 20, 5, 0, DIRECTION_RIGHT, None, None));
                state
                    .rays
                    .last()
                    .expect("ray was appended")
                    .borrow_mut()
                    .status = RayStatus::Still;
            }
            let winner = *state.active.last().expect("snapshot has a final ray");
            state.rays[winner].borrow_mut().status = RayStatus::Launch;

            assert!(
                !state.execute_iteration(),
                "a pass with {ray_count} rays terminates at the post-pass cap"
            );
            assert!(
                state.success,
                "ray #{ray_count} must execute before MAX_RAY termination"
            );
            assert_eq!(state.success_ray, Some(winner));
            assert!(matches!(
                state.rays[winner].borrow().status,
                RayStatus::Still
            ));

            let path = state.into_path(target);
            assert_eq!(
                path.waypoints,
                vec![
                    PathWaypoint {
                        x: 2,
                        y: 5,
                        transfer_target: None,
                    },
                    PathWaypoint {
                        x: 20,
                        y: 5,
                        transfer_target: None,
                    },
                ]
            );
            assert_eq!(path.length, integer_distance(2, 5, 20, 5));
        }
    }

    #[test]
    fn routes_around_solid_pixel_wall_below_column_surface() {
        // C4PathFinder's PointFree callback is LandscapeFree: strict map
        // bounds plus !DensitySolid(GBackDensity(x,y)) (C4Game.cpp:2288-2292).
        // A one-dimensional surface probe misses this cave wall entirely.
        let mut landscape =
            Landscape::with_default_material(100, vec![100; 100], None).expect("cave landscape");
        landscape.set_world_height(100);
        let mut bytes = vec![0; 100 * 100];
        for y in 45..55 {
            for x in 45..47 {
                bytes[y * 100 + x] = 1;
            }
        }
        landscape.set_pixel_grid(PixelGrid::new(
            100,
            100,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));

        let mut finder = PathFinder::new(&landscape, &[]);
        let path = finder
            .find(Vector2::new(10, 50), Vector2::new(90, 50))
            .expect("the open cave has a route around the wall");
        assert_eq!(
            path.waypoints,
            vec![
                PathWaypoint {
                    x: 10,
                    y: 50,
                    transfer_target: None,
                },
                PathWaypoint {
                    x: 47,
                    y: 46,
                    transfer_target: None,
                },
                PathWaypoint {
                    x: 47,
                    y: 44,
                    transfer_target: None,
                },
                PathWaypoint {
                    x: 90,
                    y: 50,
                    transfer_target: None,
                },
            ],
            "the Bresenham last-free mutation, clockwise crawl, back-shortening, and destination-to-source waypoint walk are deterministic (C4PathFinder.cpp:294-340,345-400)"
        );
    }

    #[test]
    fn set_level_clamps_raw_defcore_value_like_cpp() {
        // C4PathFinder::SetLevel applies BoundBy(level, 1, 10)
        // (C4PathFinder.cpp:557-560).
        let landscape = Landscape::flat(16, 20);
        let mut finder = PathFinder::new(&landscape, &[]);

        finder.set_level(-4);
        assert_eq!(finder.level, 1);
        finder.set_level(27);
        assert_eq!(finder.level, 10);
    }

    fn landscape_with_solid_transfer_zone_ring() -> Landscape {
        let mut landscape =
            Landscape::with_default_material(16, vec![16; 16], None).expect("test landscape");
        landscape.set_world_height(16);
        let mut bytes = vec![0; 16 * 16];
        for x in 4..=9 {
            bytes[4 * 16 + x] = 1;
            bytes[9 * 16 + x] = 1;
        }
        for y in 4..=9 {
            bytes[y * 16 + 4] = 1;
            bytes[y * 16 + 9] = 1;
        }
        landscape.set_pixel_grid(PixelGrid::new(
            16,
            16,
            bytes,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        landscape
    }

    #[test]
    fn transfer_waypoints_keep_the_far_side_zone_exit() {
        // C4PathFinder passes X2/Y2 by reference through GetEntryPoint and
        // emits both the far-side MoveTo and Transfer waypoint while walking
        // the completed ray chain (C4PathFinder.cpp:128-139,383-400;
        // C4TransferZone.cpp:155-207).
        let landscape = Landscape::flat(200, 100);
        let zone_owner = ObjectId::new(9);
        let zones = [TransferZoneState {
            owner: zone_owner,
            x: 80,
            y: 40,
            width: 20,
            height: 20,
        }];
        let mut finder = PathFinder::new(&landscape, &zones);

        let path = finder
            .find(Vector2::new(20, 50), Vector2::new(160, 50))
            .expect("the transfer zone bridges the route");

        assert_eq!(
            path.waypoints,
            vec![
                PathWaypoint {
                    x: 20,
                    y: 50,
                    transfer_target: None,
                },
                PathWaypoint {
                    x: 100,
                    y: 89,
                    transfer_target: None,
                },
                PathWaypoint {
                    x: 100,
                    y: 89,
                    transfer_target: Some(zone_owner),
                },
                PathWaypoint {
                    x: 160,
                    y: 50,
                    transfer_target: None,
                },
            ],
            "C++ retains both GetEntryPoint mutations before ObjectAddWaypoint"
        );
    }

    #[test]
    fn launch_zone_intersection_uses_clamped_entry_when_scan_fails() {
        // The ordinary launch arm ignores GetEntryPoint's return value but
        // still consumes its unconditionally clamped rX/rY out-parameters
        // (C4PathFinder.cpp:153-161; C4TransferZone.cpp:167-178).
        let landscape = landscape_with_solid_transfer_zone_ring();
        let mut zones = [Zone {
            owner: ObjectId::new(9),
            x: 5,
            y: 5,
            width: 4,
            height: 4,
            used: false,
        }];
        let mut state = PathFinderState::new(&landscape, &mut zones, true, 1, Vector2::new(1, 6));
        assert!(state.add_ray(1, 6, 12, 6, 0, DIRECTION_RIGHT, None, None));
        {
            let mut ray = state.rays[0].borrow_mut();
            // Force the already-checked PathFree point inside the zone while
            // preserving an origin outside, selecting the pZone launch arm.
            ray.x2 = 5;
            ray.y2 = 6;
        }

        state.execute_launch(0);

        {
            let parent = state.rays[0].borrow();
            assert!(matches!(parent.status, RayStatus::Still));
            assert_eq!((parent.x2, parent.y2), (4, 6));
        }
        assert_eq!(state.rays.len(), 2);
        let child = state.rays[1].borrow();
        assert!(matches!(child.status, RayStatus::Launch));
        assert_eq!((child.x, child.y, child.x2, child.y2), (4, 6, 4, 6));
        assert_eq!(child.use_zone, Some(0));
    }

    #[test]
    fn zero_perimeter_entry_does_not_recover_an_unscanned_free_clamp() {
        // C++ returns false when the signed perimeter loop has no
        // iterations; it never promotes the free initial clamp afterward
        // (C4TransferZone.cpp:175-198).
        let landscape = Landscape::flat(16, 20);
        let mut zones = [Zone {
            owner: ObjectId::new(9),
            x: 5,
            y: 5,
            width: 0,
            height: 0,
            used: false,
        }];
        let state = PathFinderState::new(&landscape, &mut zones, true, 1, Vector2::new(1, 1));

        let entry = state.zones[0].entry_point(&state, 5, 5, 5, 5);

        assert!(!entry.found);
        assert_eq!((entry.x, entry.y), (5, 5));
    }

    #[test]
    fn negative_transfer_zone_uses_signed_perimeter_and_cpp_bounds() {
        // C++ keeps the perimeter expression signed. With both dimensions
        // negative the loop has no iterations, even when its initial clamp
        // is free; BoundBy also accepts the resulting inverted bounds
        // (C4TransferZone.cpp:173-198; C4Math.h:23).
        let landscape = Landscape::flat(16, 20);
        let mut zones = [Zone {
            owner: ObjectId::new(9),
            x: 5,
            y: 5,
            width: -2,
            height: -2,
            used: false,
        }];
        let state = PathFinderState::new(&landscape, &mut zones, true, 1, Vector2::new(1, 1));

        let entry = state.zones[0].entry_point(&state, 5, 5, 5, 5);

        assert!(!entry.found);
        assert_eq!((entry.x, entry.y), (3, 3));

        // A negative dimension is not a blanket rejection: C++ still scans
        // when the signed width-plus-height perimeter remains positive.
        let mut mixed_zones = [Zone {
            owner: ObjectId::new(9),
            x: 5,
            y: 5,
            width: -1,
            height: 2,
            used: false,
        }];
        let mixed_state =
            PathFinderState::new(&landscape, &mut mixed_zones, true, 1, Vector2::new(1, 1));
        let mixed_entry = mixed_state.zones[0].entry_point(&mixed_state, 5, 5, 5, 5);
        assert!(mixed_entry.found);
        assert_eq!(mixed_entry.x, 4);
    }

    #[test]
    fn failed_crawl_zone_entry_keeps_the_ray_crawling() {
        // C4PF_Ray_Crawl returns early after touching an unused transfer
        // zone even when GetEntryPoint exhausts a fully solid perimeter. It
        // neither kills the ray nor increments CrawlLength
        // (C4PathFinder.cpp:198-212).
        let landscape = landscape_with_solid_transfer_zone_ring();

        let mut zones = [Zone {
            owner: ObjectId::new(9),
            x: 5,
            y: 5,
            width: 4,
            height: 4,
            used: false,
        }];
        let mut state = PathFinderState::new(&landscape, &mut zones, true, 1, Vector2::new(5, 5));
        assert!(state.add_ray(5, 5, 12, 12, 0, DIRECTION_RIGHT, None, None));
        {
            let mut ray = state.rays[0].borrow_mut();
            ray.status = RayStatus::Crawl;
            ray.x2 = 5;
            ray.y2 = 5;
            ray.crawl_start_x = 5;
            ray.crawl_start_y = 6;
            ray.crawl_attach = CRAWL_TOP;
            ray.crawl_start_attach = CRAWL_TOP;
            ray.crawl_length = 7;
        }

        let entry = state.zones[0].entry_point(&state, 5, 5, 5, 5);
        assert!(!entry.found);
        assert_eq!((entry.x, entry.y), (4, 5));

        state.execute_crawl(0);
        {
            let ray = state.rays[0].borrow();
            assert!(matches!(ray.status, RayStatus::Crawl));
            assert_eq!((ray.x2, ray.y2), (6, 5));
            assert_eq!(ray.crawl_length, 7);
        }
        assert_eq!(state.rays.len(), 1);

        state.execute_crawl(0);
        let ray = state.rays[0].borrow();
        assert!(matches!(ray.status, RayStatus::Crawl));
        assert_eq!((ray.x2, ray.y2), (7, 5));
        assert_eq!(ray.crawl_length, 7);
    }
}
