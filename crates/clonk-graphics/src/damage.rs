use crate::Rect;
use std::collections::HashMap;
use std::hash::Hash;

const MAX_DAMAGE_RECTS: usize = 128;

/// Sparse logical-pixel damage clipped to one presentation target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamageRegion {
    bounds: Rect,
    rects: Vec<Rect>,
}

/// One retained painter-order owner and the exact state that affects its pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintNode<K, V> {
    id: K,
    bounds: Rect,
    visual: V,
}

impl<K, V> PaintNode<K, V> {
    pub const fn new(id: K, bounds: Rect, visual: V) -> Self {
        Self { id, bounds, visual }
    }

    pub const fn id(&self) -> &K {
        &self.id
    }

    pub const fn bounds(&self) -> Rect {
        self.bounds
    }

    pub const fn visual(&self) -> &V {
        &self.visual
    }
}

impl DamageRegion {
    pub const fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            rects: Vec::new(),
        }
    }

    pub fn add(&mut self, rect: Rect) {
        let Some(rect) = rect.intersection(self.bounds) else {
            return;
        };

        let mut uncovered = vec![rect];
        for covered in self.rects.iter().copied() {
            uncovered = uncovered
                .into_iter()
                .flat_map(|candidate| subtract_rect(candidate, covered))
                .collect();
            if uncovered.is_empty() {
                return;
            }
            if self.rects.len().saturating_add(uncovered.len()) > MAX_DAMAGE_RECTS {
                self.rects.clear();
                self.rects.push(self.bounds);
                return;
            }
        }

        self.rects.extend(uncovered);
        normalize_rectangles(&mut self.rects);
        if self.rects.len() > MAX_DAMAGE_RECTS {
            self.rects.clear();
            self.rects.push(self.bounds);
            return;
        }
        self.rects
            .sort_unstable_by_key(|rect| (rect.y, rect.x, rect.height, rect.width));
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// Whether this region replaces every pixel in its target.
    pub fn is_full(&self) -> bool {
        let area = |rect: Rect| u64::from(rect.width) * u64::from(rect.height);
        self.rects.iter().copied().map(area).sum::<u64>() == area(self.bounds)
    }

    pub fn intersects(&self, bounds: Rect) -> bool {
        self.rects
            .iter()
            .any(|damage| damage.intersection(bounds).is_some())
    }

    pub fn between<K: Eq + Hash, V: PartialEq>(
        bounds: Rect,
        previous: &[PaintNode<K, V>],
        current: &[PaintNode<K, V>],
    ) -> Self {
        let mut damage = Self::new(bounds);
        let (Some(previous_indices), Some(current_indices)) =
            (unique_indices(previous), unique_indices(current))
        else {
            damage.add(bounds);
            return damage;
        };
        for old in previous {
            match current_indices.get(&old.id).map(|index| &current[*index]) {
                Some(new) if new.bounds == old.bounds && new.visual == old.visual => {}
                Some(new) => {
                    damage.add(old.bounds);
                    damage.add(new.bounds);
                }
                None => damage.add(old.bounds),
            }
        }
        for new in current {
            if !previous_indices.contains_key(&new.id) {
                damage.add(new.bounds);
            }
        }
        let shared_current_order = previous
            .iter()
            .filter_map(|old| current_indices.get(&old.id).copied())
            .collect::<Vec<_>>();
        if shared_current_order
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return damage;
        }
        for (left_index, old_left) in previous.iter().enumerate() {
            let Some(&new_left_index) = current_indices.get(&old_left.id) else {
                continue;
            };
            for old_right in &previous[left_index + 1..] {
                let Some(&new_right_index) = current_indices.get(&old_right.id) else {
                    continue;
                };
                if new_left_index < new_right_index {
                    continue;
                }
                old_left
                    .bounds
                    .intersection(old_right.bounds)
                    .into_iter()
                    .for_each(|rect| damage.add(rect));
                current[new_left_index]
                    .bounds
                    .intersection(current[new_right_index].bounds)
                    .into_iter()
                    .for_each(|rect| damage.add(rect));
            }
        }
        damage
    }
}

fn unique_indices<K: Eq + Hash, V>(nodes: &[PaintNode<K, V>]) -> Option<HashMap<&K, usize>> {
    let mut indices = HashMap::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        if indices.insert(&node.id, index).is_some() {
            return None;
        }
    }
    Some(indices)
}

fn bounding_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (i64::from(left.x) + i64::from(left.width))
        .max(i64::from(right.x) + i64::from(right.width));
    let bottom_edge = (i64::from(left.y) + i64::from(left.height))
        .max(i64::from(right.y) + i64::from(right.height));
    Rect::new(
        x,
        y,
        u32::try_from(right_edge - i64::from(x)).unwrap_or(u32::MAX),
        u32::try_from(bottom_edge - i64::from(y)).unwrap_or(u32::MAX),
    )
}

fn union_is_rectangular(left: Rect, right: Rect) -> bool {
    let bounds = bounding_rect(left, right);
    let area = |rect: Rect| i128::from(rect.width) * i128::from(rect.height);
    let overlap = left.intersection(right).map(area).unwrap_or(0);
    area(bounds) == area(left) + area(right) - overlap
}

fn subtract_rect(rect: Rect, covered: Rect) -> Vec<Rect> {
    let Some(overlap) = rect.intersection(covered) else {
        return vec![rect];
    };
    let left = i64::from(rect.x);
    let top = i64::from(rect.y);
    let right = left + i64::from(rect.width);
    let bottom = top + i64::from(rect.height);
    let overlap_left = i64::from(overlap.x);
    let overlap_top = i64::from(overlap.y);
    let overlap_right = overlap_left + i64::from(overlap.width);
    let overlap_bottom = overlap_top + i64::from(overlap.height);

    [
        rect_from_edges(left, top, right, overlap_top),
        rect_from_edges(left, overlap_bottom, right, bottom),
        rect_from_edges(left, overlap_top, overlap_left, overlap_bottom),
        rect_from_edges(overlap_right, overlap_top, right, overlap_bottom),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn rect_from_edges(left: i64, top: i64, right: i64, bottom: i64) -> Option<Rect> {
    (right > left && bottom > top).then(|| {
        Rect::new(
            left.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            top.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            u32::try_from(right - left).unwrap_or(u32::MAX),
            u32::try_from(bottom - top).unwrap_or(u32::MAX),
        )
    })
}

fn normalize_rectangles(rects: &mut Vec<Rect>) {
    loop {
        let pair = rects.iter().enumerate().find_map(|(left_index, left)| {
            rects
                .iter()
                .enumerate()
                .skip(left_index + 1)
                .find(|(_, right)| union_is_rectangular(*left, **right))
                .map(|(right_index, _)| (left_index, right_index))
        });
        let Some((left_index, right_index)) = pair else {
            return;
        };
        let right = rects.remove(right_index);
        let left = rects.remove(left_index);
        rects.push(bounding_rect(left, right));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Rect clips against half-open screen bounds (src/C4Rect.h:40-43).
    // C++ redraws the whole GUI after a move; Rust keeps the old and new
    // ownership rectangles separate so their untouched gap stays untouched.
    #[test]
    fn moved_owner_keeps_clipped_old_and_new_damage_separate() {
        let mut damage = DamageRegion::new(Rect::new(0, 0, 100, 50));

        damage.add(Rect::new(-4, 10, 12, 8));
        damage.add(Rect::new(80, 10, 12, 8));

        assert_eq!(
            damage.rects(),
            &[Rect::new(0, 10, 8, 8), Rect::new(80, 10, 12, 8)]
        );
    }

    #[test]
    fn rectangular_neighbors_normalize_without_covering_the_gap() {
        let mut damage = DamageRegion::new(Rect::new(0, 0, 100, 50));

        damage.add(Rect::new(20, 10, 10, 8));
        damage.add(Rect::new(10, 10, 10, 8));
        damage.add(Rect::new(12, 12, 4, 2));
        damage.add(Rect::new(40, 10, 10, 8));

        assert_eq!(
            damage.rects(),
            &[Rect::new(10, 10, 20, 8), Rect::new(40, 10, 10, 8)]
        );
    }

    #[test]
    fn non_rectangular_overlap_is_partitioned_without_double_painting() {
        let mut damage = DamageRegion::new(Rect::new(0, 0, 40, 40));

        damage.add(Rect::new(0, 0, 10, 30));
        damage.add(Rect::new(0, 10, 30, 10));

        assert_eq!(
            damage.rects(),
            &[Rect::new(0, 0, 10, 30), Rect::new(10, 10, 20, 10)]
        );
    }

    #[test]
    fn excessive_fragmentation_falls_back_to_the_target_bounds() {
        let bounds = Rect::new(0, 0, 257, 1);
        let mut damage = DamageRegion::new(bounds);

        for x in (0..257).step_by(2) {
            damage.add(Rect::new(x, 0, 1, 1));
        }

        assert_eq!(damage.rects(), &[bounds]);
    }

    #[test]
    fn non_rectangular_tiling_is_recognized_as_full_damage() {
        let bounds = Rect::new(0, 0, 6, 6);
        let mut damage = DamageRegion::new(bounds);

        for rect in [
            Rect::new(0, 0, 4, 2),
            Rect::new(4, 0, 2, 4),
            Rect::new(2, 4, 4, 2),
            Rect::new(0, 2, 2, 4),
            Rect::new(2, 2, 2, 2),
        ] {
            damage.add(rect);
        }

        assert!(damage.is_full());
    }

    // Element::UpdatePos overwrites the current bounds and notifies its parent
    // (src/C4Gui.cpp:159-173); C++ then redraws the complete tree. The retained
    // Rust tree must own both positions before repainting the intersecting nodes.
    #[test]
    fn moving_a_paint_node_damages_its_old_and_new_bounds() {
        let previous = [PaintNode::new(7, Rect::new(10, 12, 20, 8), "button")];
        let current = [PaintNode::new(7, Rect::new(50, 12, 20, 8), "button")];

        let damage = DamageRegion::between(Rect::new(0, 0, 100, 50), &previous, &current);

        assert_eq!(
            damage.rects(),
            &[Rect::new(10, 12, 20, 8), Rect::new(50, 12, 20, 8)]
        );
    }

    #[test]
    fn paint_visuals_need_only_exact_partial_equality() {
        let previous = [PaintNode::new(1, Rect::new(4, 5, 6, 7), 1.25_f32)];
        let current = [PaintNode::new(1, Rect::new(4, 5, 6, 7), 2.5_f32)];

        assert_eq!(*previous[0].visual(), 1.25);

        let damage = DamageRegion::between(Rect::new(0, 0, 20, 20), &previous, &current);

        assert_eq!(damage.rects(), &[Rect::new(4, 5, 6, 7)]);
    }

    // C4GUI inserts equal-z dialogs after existing peers and paints children
    // head-to-tail (src/C4Gui.cpp:556-584; src/C4GuiContainers.cpp:33-45).
    #[test]
    fn changing_painter_order_damages_only_the_nodes_overlap() {
        let previous = [
            PaintNode::new("older", Rect::new(10, 10, 30, 20), 0),
            PaintNode::new("newer", Rect::new(30, 20, 30, 20), 0),
        ];
        let current = [previous[1].clone(), previous[0].clone()];

        let damage = DamageRegion::between(Rect::new(0, 0, 100, 50), &previous, &current);

        assert_eq!(damage.rects(), &[Rect::new(30, 20, 10, 10)]);
    }

    #[test]
    fn duplicate_owner_identity_fails_closed_to_full_damage() {
        let bounds = Rect::new(0, 0, 100, 50);
        let previous = [
            PaintNode::new("duplicate", Rect::new(10, 10, 10, 10), 0),
            PaintNode::new("duplicate", Rect::new(40, 10, 10, 10), 0),
        ];

        let damage = DamageRegion::between(bounds, &previous, &previous);

        assert_eq!(damage.rects(), &[bounds]);
    }

    // Derived optimization invariant, not a C++ dirty-region rule: C++
    // redraws every visible child in list order (src/C4GuiContainers.cpp:33-45).
    #[test]
    fn intersection_filter_preserves_the_full_painter_order() {
        let nodes = [
            PaintNode::new("background", Rect::new(0, 0, 100, 50), 0),
            PaintNode::new("left-button", Rect::new(5, 5, 10, 10), 0),
            PaintNode::new("dialog", Rect::new(40, 5, 40, 30), 0),
            PaintNode::new("tooltip", Rect::new(45, 20, 20, 10), 0),
        ];
        let mut damage = DamageRegion::new(Rect::new(0, 0, 100, 50));
        damage.add(Rect::new(50, 22, 2, 2));

        let selected = nodes
            .iter()
            .filter(|node| damage.intersects(node.bounds()))
            .map(PaintNode::id)
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(selected, vec!["background", "dialog", "tooltip"]);
    }
}
