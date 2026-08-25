use std::collections::HashSet;

#[cfg(test)]
use crate::SECTOR_RANK_REBUILDS;
#[cfg(test)]
use std::cell::Cell;

use crate::{DefinitionRect, ObjectId, Vector2};

pub(crate) const SECTOR_WIDTH: i32 = 50;
#[doc(hidden)]
pub const SECTOR_HEIGHT: i32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(hidden)]
pub enum SectorKey {
    Inside { x: i32, y: i32 },
    Outside,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sector {
    objects: Vec<ObjectId>,
    object_shapes: Vec<ObjectId>,
}

impl Sector {
    fn new(_x: i32, _y: i32) -> Self {
        Self {
            objects: Vec::new(),
            object_shapes: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.objects.clear();
        self.object_shapes.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SectorObject {
    pub id: ObjectId,
    pub position: Vector2,
    pub shape_rect: DefinitionRect,
}

#[derive(Debug, Clone)]
struct SectorMembership {
    position: Vector2,
    area: SectorArea,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct SectorMap {
    px_width: i32,
    px_height: i32,
    width: i32,
    height: i32,
    sectors: Vec<Sector>,
    outside: Sector,
    /// Both maps are probed by `ObjectId` from `update` on every moved object
    /// and are never iterated: `ranks` is rebuilt from `order`, the explicit
    /// total order, and every read is a `get`/`insert`/`remove`. So they carry
    /// the engine's fixed-seed hasher rather than `RandomState`'s.
    memberships: rustc_hash::FxHashMap<ObjectId, SectorMembership>,
    order: Vec<ObjectId>,
    ranks: rustc_hash::FxHashMap<ObjectId, usize>,
}

impl SectorMap {
    pub(crate) fn new(px_width: i32, px_height: i32) -> Self {
        let px_width = px_width.max(0);
        let px_height = px_height.max(0);
        let width = sector_count(px_width, SECTOR_WIDTH);
        let height = sector_count(px_height, SECTOR_HEIGHT);
        let mut sectors = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                sectors.push(Sector::new(x, y));
            }
        }
        Self {
            px_width,
            px_height,
            width,
            height,
            sectors,
            outside: Sector::new(-1, -1),
            memberships: rustc_hash::FxHashMap::default(),
            order: Vec::new(),
            ranks: rustc_hash::FxHashMap::default(),
        }
    }

    pub(crate) fn rebuild<I>(&mut self, records: I)
    where
        I: IntoIterator<Item = SectorObject>,
    {
        self.clear_lists();
        self.memberships.clear();
        self.order.clear();
        self.ranks.clear();
        // Records arrive in object order, so every id gets a rank above all
        // ids already present and the rank-ordered insert position is always
        // the tail — push directly instead of paying `add`'s O(len) scans
        // per sector list (rebuild runs per host-context build; the scans
        // made it quadratic in the object count).
        for record in records {
            if self.memberships.contains_key(&record.id) {
                self.remove(record.id);
            }
            if !self.ranks.contains_key(&record.id) {
                let rank = self.order.len();
                self.order.push(record.id);
                self.ranks.insert(record.id, rank);
            }

            let point_sector = self.sector_at(record.position.x, record.position.y);
            self.sector_mut(point_sector).objects.push(record.id);

            let area = self.area(record.shape_rect);
            for key in area.iter() {
                self.sector_mut(key).object_shapes.push(record.id);
            }
            self.memberships.insert(
                record.id,
                SectorMembership {
                    position: record.position,
                    area,
                },
            );
        }
    }

    /// Refresh the rank oracle used by `C4ObjectList::stMain` insertions.
    ///
    /// `C4LSectors::Add` receives the live forward master list and inserts a
    /// new object relative to that list (C4Sector.cpp:88-101;
    /// C4ObjectList.cpp:138-205). Existing entries keep their relative order;
    /// only the not-yet-indexed object needs the refreshed ranks when `add`
    /// inserts it below.
    pub(crate) fn set_master_order<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = ObjectId>,
    {
        let previous = std::mem::take(&mut self.order);
        let mut seen = HashSet::new();
        self.order
            .extend(ids.into_iter().filter(|id| seen.insert(*id)));
        self.order
            .extend(previous.into_iter().filter(|id| seen.insert(*id)));
        self.rebuild_ranks();
    }

    pub(crate) fn set_master_order_if_changed(&mut self, ids: &[ObjectId]) -> bool {
        // `set_master_order` keeps old entries after the incoming master list,
        // so an equal prefix already has the exact resulting order.
        if self.order.starts_with(ids) {
            return false;
        }
        self.set_master_order(ids.iter().copied());
        true
    }

    pub(crate) fn add(&mut self, record: SectorObject) {
        if self.memberships.contains_key(&record.id) {
            self.remove(record.id);
        }
        if !self.ranks.contains_key(&record.id) {
            let rank = self.order.len();
            self.order.push(record.id);
            self.ranks.insert(record.id, rank);
        }

        let point_sector = self.sector_at(record.position.x, record.position.y);
        self.insert_object(point_sector, record.id);

        let area = self.area(record.shape_rect);
        for key in area.iter() {
            self.insert_shape(key, record.id);
        }
        self.memberships.insert(
            record.id,
            SectorMembership {
                position: record.position,
                area,
            },
        );
    }

    pub(crate) fn update(&mut self, record: SectorObject) {
        let Some(previous) = self.memberships.get(&record.id).cloned() else {
            self.add(record);
            return;
        };

        let old_point_sector = self.sector_at(previous.position.x, previous.position.y);
        let new_point_sector = self.sector_at(record.position.x, record.position.y);
        if old_point_sector != new_point_sector {
            self.remove_object(old_point_sector, record.id);
            self.insert_object(new_point_sector, record.id);
        }

        let new_area = self.area(record.shape_rect);
        if previous.area != new_area {
            for key in previous.area.iter() {
                if !new_area.contains(key) {
                    self.remove_shape(key, record.id);
                }
            }
            for key in new_area.iter() {
                if !previous.area.contains(key) {
                    self.insert_shape(key, record.id);
                }
            }
        }

        self.memberships.insert(
            record.id,
            SectorMembership {
                position: record.position,
                area: new_area,
            },
        );
    }

    pub(crate) fn remove(&mut self, id: ObjectId) {
        self.remove_many(std::iter::once(id));
    }

    pub(crate) fn remove_many<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = ObjectId>,
    {
        let mut removed = HashSet::new();
        for id in ids {
            let Some(previous) = self.memberships.remove(&id) else {
                continue;
            };
            let point_sector = self.sector_at(previous.position.x, previous.position.y);
            self.remove_object(point_sector, id);
            for key in previous.area.iter() {
                self.remove_shape(key, id);
            }
            removed.insert(id);
        }
        if !removed.is_empty() {
            self.order.retain(|candidate| !removed.contains(candidate));
            self.rebuild_ranks();
        }
    }

    pub(crate) fn sector_at(&self, x: i32, y: i32) -> SectorKey {
        if x < 0 || y < 0 || x >= self.px_width || y >= self.px_height {
            SectorKey::Outside
        } else {
            SectorKey::Inside {
                x: x / SECTOR_WIDTH,
                y: y / SECTOR_HEIGHT,
            }
        }
    }

    #[doc(hidden)]
    pub fn area(&self, rect: DefinitionRect) -> SectorArea {
        SectorArea::new(self, Rect::from(rect))
    }

    #[doc(hidden)]
    pub fn object_ids(&self, key: SectorKey) -> &[ObjectId] {
        &self.sector(key).objects
    }

    /// Candidates in C++ area-enumeration order: sectors row-major with the
    /// outside-sector last (C4LArea::Next, C4Sector.cpp:264-277), each
    /// sector's list in master-list rank order, duplicates dropped at their
    /// first encounter (the Marker, C4GameObjects.cpp:155-165,
    /// C4FindObject.cpp:325-353). NOT globally rank-sorted — pair order
    /// feeds the RNG, so this is lockstep order.
    pub(crate) fn object_ids_in_area(&self, area: &SectorArea) -> Vec<ObjectId> {
        // No dedup needed: an object's center point is in exactly one sector,
        // so the point lists are disjoint (C++ pass 2 keeps a Marker anyway,
        // but it can never fire for point lists).
        let mut ids = Vec::new();
        for key in area.iter() {
            ids.extend_from_slice(self.object_ids(key));
        }
        ids
    }

    /// The per-sector point lists in C4LArea enumeration order, kept
    /// separate: C4FindObject::Find with a sort finds a best PER LIST
    /// before meeting the running best (C4FindObject.cpp:296-306).
    pub(crate) fn object_id_lists_in_area(&self, area: &SectorArea) -> Vec<Vec<ObjectId>> {
        area.iter()
            .map(|key| self.object_ids(key).to_vec())
            .collect()
    }

    #[doc(hidden)]
    pub fn shape_ids(&self, key: SectorKey) -> &[ObjectId] {
        &self.sector(key).object_shapes
    }

    pub(crate) fn shape_ids_at(&self, x: i32, y: i32) -> &[ObjectId] {
        self.shape_ids(self.sector_at(x, y))
    }

    /// The per-sector shape lists in C4LArea order, NOT deduplicated:
    /// C4FindObject::Find has no Marker (C4FindObject.cpp:283-294), so an
    /// object whose shape spans sectors is re-encountered per sector.
    pub(crate) fn shape_id_lists_in_area(&self, area: &SectorArea) -> Vec<Vec<ObjectId>> {
        area.iter()
            .map(|key| self.shape_ids(key).to_vec())
            .collect()
    }

    /// Same C++ enumeration order as `object_ids_in_area`, over the
    /// per-sector shape lists (`FirstObjectShapes`/`NextObjectShapes`).
    #[doc(hidden)]
    pub fn shape_ids_in_area(&self, area: &SectorArea) -> Vec<ObjectId> {
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for key in area.iter() {
            for &id in self.shape_ids(key) {
                if seen.insert(id) {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// `C4LSectors::getShapeSum` (C4Sector.cpp:197-203): the sum of the
    /// per-sector shape-list object counts — part of the sync-check digest.
    #[doc(hidden)]
    pub fn shape_sum(&self) -> usize {
        self.sectors
            .iter()
            .map(|sector| sector.object_shapes.len())
            .sum()
    }

    fn clear_lists(&mut self) {
        for sector in &mut self.sectors {
            sector.clear();
        }
        self.outside.clear();
    }

    fn sector(&self, key: SectorKey) -> &Sector {
        match key {
            SectorKey::Inside { x, y } => self
                .inside_index(x, y)
                .and_then(|index| self.sectors.get(index))
                .unwrap_or(&self.outside),
            SectorKey::Outside => &self.outside,
        }
    }

    fn sector_mut(&mut self, key: SectorKey) -> &mut Sector {
        match key {
            SectorKey::Inside { x, y } => match self.inside_index(x, y) {
                Some(index) => &mut self.sectors[index],
                None => &mut self.outside,
            },
            SectorKey::Outside => &mut self.outside,
        }
    }

    fn inside_index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            None
        } else {
            Some((y * self.width + x) as usize)
        }
    }

    fn insert_object(&mut self, key: SectorKey, id: ObjectId) {
        let rank = self.rank(id);
        let insert_at = insert_index_by_rank(&self.ranks, rank, &self.sector(key).objects, id);
        if let Some(insert_at) = insert_at {
            self.sector_mut(key).objects.insert(insert_at, id);
        }
    }

    fn insert_shape(&mut self, key: SectorKey, id: ObjectId) {
        let rank = self.rank(id);
        let insert_at =
            insert_index_by_rank(&self.ranks, rank, &self.sector(key).object_shapes, id);
        if let Some(insert_at) = insert_at {
            self.sector_mut(key).object_shapes.insert(insert_at, id);
        }
    }

    fn remove_object(&mut self, key: SectorKey, id: ObjectId) {
        remove_id(&mut self.sector_mut(key).objects, id);
    }

    fn remove_shape(&mut self, key: SectorKey, id: ObjectId) {
        remove_id(&mut self.sector_mut(key).object_shapes, id);
    }

    fn rank(&self, id: ObjectId) -> usize {
        self.ranks.get(&id).copied().unwrap_or(usize::MAX)
    }

    fn rebuild_ranks(&mut self) {
        #[cfg(test)]
        SECTOR_RANK_REBUILDS.with(|count| count.set(count.get() + 1));
        self.ranks.clear();
        for (rank, &id) in self.order.iter().enumerate() {
            self.ranks.insert(id, rank);
        }
    }
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct SectorArea {
    first: Option<SectorKey>,
    x_limit: i32,
    y_limit: i32,
    dpitch: i32,
    out: bool,
    sector_width: i32,
    sector_count: i32,
}

impl SectorArea {
    fn new(map: &SectorMap, rect: Rect) -> Self {
        let mut clipped = rect;
        let bounds = Rect {
            x: 0,
            y: 0,
            width: map.px_width,
            height: map.px_height,
        };
        clipped.normalize();
        let out = !bounds.contains_rect(clipped);
        if out {
            clipped.intersect(bounds);
        }
        let first = Some(map.sector_at(clipped.x, clipped.y));
        if clipped.width == 0 {
            clipped.width = 1;
        }
        if clipped.height == 0 {
            clipped.height = 1;
        }
        let right_sector = (clipped.x + clipped.width - 1) / SECTOR_WIDTH;
        let left_sector = clipped.x / SECTOR_WIDTH;
        let bottom_sector = (clipped.y + clipped.height - 1) / SECTOR_HEIGHT;
        let dpitch = map.width - right_sector + left_sector;
        Self {
            first,
            x_limit: right_sector,
            y_limit: bottom_sector,
            dpitch,
            out,
            sector_width: map.width,
            sector_count: map.width * map.height,
        }
    }

    #[cfg(test)]
    pub(crate) fn first(&self) -> Option<SectorKey> {
        self.first
    }

    #[cfg(test)]
    pub(crate) fn dpitch(&self) -> i32 {
        self.dpitch
    }

    pub(crate) fn iter(&self) -> SectorAreaIter<'_> {
        SectorAreaIter {
            area: self,
            next: self.first,
        }
    }

    pub(crate) fn next(&self, previous: SectorKey) -> Option<SectorKey> {
        match previous {
            SectorKey::Outside => None,
            SectorKey::Inside { x, y } => {
                if x < self.x_limit {
                    return Some(SectorKey::Inside { x: x + 1, y });
                }
                if y < self.y_limit {
                    let current_index = y * self.sector_width + x;
                    let next_index = current_index + self.dpitch;
                    return if next_index >= 0 && next_index < self.sector_count {
                        Some(SectorKey::Inside {
                            x: next_index % self.sector_width,
                            y: next_index / self.sector_width,
                        })
                    } else {
                        Some(SectorKey::Outside)
                    };
                }
                if self.out {
                    Some(SectorKey::Outside)
                } else {
                    None
                }
            }
        }
    }

    pub(crate) fn contains(&self, key: SectorKey) -> bool {
        let Some(first) = self.first else {
            return false;
        };
        if key == SectorKey::Outside {
            return self.out;
        }
        let SectorKey::Inside {
            x: first_x,
            y: first_y,
        } = first
        else {
            return false;
        };
        let SectorKey::Inside { x, y } = key else {
            return false;
        };
        x >= first_x && y >= first_y && x <= self.x_limit && y <= self.y_limit
    }
}

impl PartialEq for SectorArea {
    fn eq(&self, other: &Self) -> bool {
        self.first == other.first
            && self.x_limit == other.x_limit
            && self.y_limit == other.y_limit
            && self.out == other.out
    }
}

impl Eq for SectorArea {}

pub(crate) struct SectorAreaIter<'a> {
    area: &'a SectorArea,
    next: Option<SectorKey>,
}

impl Iterator for SectorAreaIter<'_> {
    type Item = SectorKey;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.area.next(current);
        Some(current)
    }
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    fn normalize(&mut self) {
        if self.width < 0 {
            self.x = self.x.saturating_add(self.width).saturating_add(1);
            self.width = self.width.saturating_neg();
        }
        if self.height < 0 {
            self.y = self.y.saturating_add(self.height).saturating_add(1);
            self.height = self.height.saturating_neg();
        }
    }

    fn contains_rect(&self, other: Rect) -> bool {
        other.x >= self.x
            && i64::from(other.x) + i64::from(other.width)
                < i64::from(self.x) + i64::from(self.width)
            && other.y >= self.y
            && i64::from(other.y) + i64::from(other.height)
                < i64::from(self.y) + i64::from(self.height)
    }

    fn intersect(&mut self, other: Rect) {
        if other.x > self.x {
            if rect_right(other) < rect_right(*self) {
                self.x = other.x;
                self.width = other.width;
            } else {
                self.width = self.width.saturating_sub(other.x.saturating_sub(self.x));
                self.x = other.x;
            }
        } else if rect_right(other) < rect_right(*self) {
            self.width = saturating_i64_to_i32(rect_right(other) - i64::from(self.x));
        }

        if other.y > self.y {
            if rect_bottom(other) < rect_bottom(*self) {
                self.y = other.y;
                self.height = other.height;
            } else {
                self.height = self.height.saturating_sub(other.y.saturating_sub(self.y));
                self.y = other.y;
            }
        } else if rect_bottom(other) < rect_bottom(*self) {
            self.height = saturating_i64_to_i32(rect_bottom(other) - i64::from(self.y));
        }

        if self.width < 0 {
            self.width = 0;
        }
        if self.height < 0 {
            self.height = 0;
        }
    }
}

impl From<DefinitionRect> for Rect {
    fn from(rect: DefinitionRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

fn rect_right(rect: Rect) -> i64 {
    i64::from(rect.x) + i64::from(rect.width)
}

fn rect_bottom(rect: Rect) -> i64 {
    i64::from(rect.y) + i64::from(rect.height)
}

fn sector_count(px: i32, sector_size: i32) -> i32 {
    if px <= 0 {
        1
    } else {
        ((px - 1) / sector_size) + 1
    }
}

fn remove_id(ids: &mut Vec<ObjectId>, id: ObjectId) {
    ids.retain(|&candidate| candidate != id);
}

fn insert_index_by_rank(
    ranks: &rustc_hash::FxHashMap<ObjectId, usize>,
    rank: usize,
    ids: &[ObjectId],
    id: ObjectId,
) -> Option<usize> {
    if ids.contains(&id) {
        return None;
    }
    let insert_at = ids
        .iter()
        .position(|candidate| ranks.get(candidate).copied().unwrap_or(usize::MAX) > rank)
        .unwrap_or(ids.len());
    Some(insert_at)
}

fn saturating_i64_to_i32(value: i64) -> i32 {
    if value > i64::from(i32::MAX) {
        i32::MAX
    } else if value < i64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(area: &SectorArea) -> Vec<SectorKey> {
        area.iter().collect()
    }

    #[test]
    fn area_queries_enumerate_sector_major_like_cpp() {
        // C++ area enumeration is NEVER a global rank sort: C4LArea::Next
        // walks sectors row-major with the outside-sector last
        // (C4Sector.cpp:264-277), each sector's list is in master-list
        // (rank) order, and the Marker only dedups repeat encounters
        // (C4GameObjects.cpp:155-165, C4FindObject.cpp:325-353). An object
        // in an earlier sector is visited before a lower-ranked object in a
        // later sector — pair order feeds the RNG, so this is lockstep
        // order.
        let mut sectors = SectorMap::new(150, 100);
        // Rank order: 1, 2, 3 — but 2 sits in the SECOND sector while 1 and
        // 3 share the first.
        sectors.rebuild(vec![
            SectorObject {
                id: ObjectId::new(1),
                position: Vector2::new(10, 10),
                shape_rect: DefinitionRect::new(8, 8, 4, 4),
            },
            SectorObject {
                id: ObjectId::new(2),
                position: Vector2::new(80, 10),
                shape_rect: DefinitionRect::new(78, 8, 4, 4),
            },
            SectorObject {
                id: ObjectId::new(3),
                position: Vector2::new(20, 20),
                shape_rect: DefinitionRect::new(18, 18, 4, 4),
            },
        ]);
        let area = sectors.area(DefinitionRect::new(0, 0, 120, 40));
        assert_eq!(
            sectors.object_ids_in_area(&area),
            vec![ObjectId::new(1), ObjectId::new(3), ObjectId::new(2)],
            "point lists concatenate in sector order, rank-ordered within"
        );
        assert_eq!(
            sectors.shape_ids_in_area(&area),
            vec![ObjectId::new(1), ObjectId::new(3), ObjectId::new(2)],
            "shape lists concatenate in sector order with first-encounter dedup"
        );
    }

    #[test]
    fn area_iterates_rows_with_cpp_pitch_order() {
        let sectors = SectorMap::new(150, 150);
        let area = sectors.area(DefinitionRect::new(60, 10, 60, 80));

        assert_eq!(area.dpitch(), 2);
        assert_eq!(
            keys(&area),
            vec![
                SectorKey::Inside { x: 1, y: 0 },
                SectorKey::Inside { x: 2, y: 0 },
                SectorKey::Inside { x: 1, y: 1 },
                SectorKey::Inside { x: 2, y: 1 },
            ]
        );
    }

    #[test]
    fn area_touching_right_or_bottom_edge_yields_out_sector_last() {
        let sectors = SectorMap::new(100, 100);
        let area = sectors.area(DefinitionRect::new(99, 10, 1, 1));

        assert_eq!(area.first(), Some(SectorKey::Inside { x: 1, y: 0 }));
        assert_eq!(
            keys(&area),
            vec![SectorKey::Inside { x: 1, y: 0 }, SectorKey::Outside]
        );

        let area = sectors.area(DefinitionRect::new(10, 99, 1, 1));
        assert_eq!(
            keys(&area),
            vec![SectorKey::Inside { x: 0, y: 1 }, SectorKey::Outside]
        );
    }

    #[test]
    fn clipped_areas_follow_legacy_outside_edges() {
        let sectors = SectorMap::new(100, 100);

        assert_eq!(
            keys(&sectors.area(DefinitionRect::new(-60, 10, 10, 10))),
            vec![SectorKey::Inside { x: 0, y: 0 }, SectorKey::Outside]
        );
        assert_eq!(
            keys(&sectors.area(DefinitionRect::new(160, 10, 10, 10))),
            vec![SectorKey::Outside]
        );
        assert_eq!(
            keys(&sectors.area(DefinitionRect::new(10, -60, 10, 10))),
            vec![SectorKey::Inside { x: 0, y: 0 }, SectorKey::Outside]
        );
        assert_eq!(
            keys(&sectors.area(DefinitionRect::new(10, 160, 10, 10))),
            vec![SectorKey::Outside]
        );
    }

    #[test]
    fn update_moves_point_and_shape_memberships() {
        let mut sectors = SectorMap::new(120, 120);
        let id = ObjectId::new(1);
        sectors.add(SectorObject {
            id,
            position: Vector2::new(10, 10),
            shape_rect: DefinitionRect::new(5, 5, 10, 10),
        });

        assert_eq!(sectors.object_ids(SectorKey::Inside { x: 0, y: 0 }), &[id]);
        assert_eq!(sectors.shape_sum(), 1);

        sectors.update(SectorObject {
            id,
            position: Vector2::new(70, 70),
            shape_rect: DefinitionRect::new(65, 65, 60, 60),
        });

        assert!(sectors
            .object_ids(SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
        assert_eq!(sectors.object_ids(SectorKey::Inside { x: 1, y: 1 }), &[id]);
        assert!(sectors
            .shape_ids(SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
        assert_eq!(sectors.shape_ids(SectorKey::Inside { x: 1, y: 1 }), &[id]);
        assert_eq!(sectors.shape_ids(SectorKey::Inside { x: 2, y: 2 }), &[id]);
    }

    #[test]
    fn removing_many_objects_rebuilds_master_ranks_once() {
        // C4GameObjects::DeleteObjects removes every flagged link while
        // retaining the relative order of surviving master-list entries;
        // sector teardown observes that same completed removal set
        // (C4GameObjects.cpp:295-335; C4Sector.cpp:74-86).
        let mut sectors = SectorMap::new(100, 100);
        sectors.rebuild((1..=3).map(|raw| SectorObject {
            id: ObjectId::new(raw),
            position: Vector2::new(raw as i32, raw as i32),
            shape_rect: DefinitionRect::new(raw as i32, raw as i32, 1, 1),
        }));
        SECTOR_RANK_REBUILDS.with(|count| count.set(0));

        sectors.remove_many([ObjectId::new(1), ObjectId::new(2)]);

        assert_eq!(sectors.order, vec![ObjectId::new(3)]);
        assert_eq!(sectors.rank(ObjectId::new(3)), 0);
        assert_eq!(SECTOR_RANK_REBUILDS.with(Cell::get), 1);
    }
}
