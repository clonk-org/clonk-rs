//! C4MapCreatorS2 — the complex dynamic-map creator (src/C4MapCreatorS2.cpp).
//!
//! `Landscape.txt` describes the map as a tree of overlays combined with
//! `&`/`|`/`^` operator chains; each overlay carries a material/texture, an
//! algorithm and turbulence/rotation transforms evaluated per map pixel
//! (C4MCOverlay::CheckMask/RenderPix, src/C4MapCreatorS2.cpp:448-553).
//! The creator's RNG draws — the DefaultMap size evaluates, the parse-time
//! `a - b` range draws and the per-overlay seed draws — come from the
//! FixRandom bracket of C4Landscape::Init (src/C4Landscape.cpp:578,734),
//! keeping the post-init synced ledger untouched.
//!
//! Unsupported (no shipped content uses them): `evalFn=`/`drawFn=` script
//! callbacks and `algo=script` — the field setters fail like C++'s missing
//! script-func parse error, which degrades to the basic map creator.

use crate::map_creator::evaluate_map_size;
use crate::rng::LcgRng;
use crate::scenario::{LegacyC4SVal, MapPixelClassifier};

/// `C4MC_SizeRes` — positions in percent (C4MapCreatorS2.h:29).
const SIZE_RES: i32 = 100;
/// `C4MC_ZoomRes` (C4MapCreatorS2.h:30).
const ZOOM_RES: i32 = 100;

type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    None,
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Algo {
    Solid,
    Random,
    Checker,
    Bozo,
    Sin,
    Boxes,
    RndChecker,
    Lines,
    Border,
    Mandel,
    Gradient,
    RndAll,
    Poly,
}

impl Algo {
    /// C4MCAlgoMap (src/C4MapCreatorS2.cpp:1572-1589), sans `script`.
    fn by_name(name: &str) -> Option<Self> {
        Some(match name {
            "solid" => Self::Solid,
            "random" => Self::Random,
            "checker" => Self::Checker,
            "bozo" => Self::Bozo,
            "sin" => Self::Sin,
            "boxes" => Self::Boxes,
            "rndchecker" => Self::RndChecker,
            "lines" => Self::Lines,
            "border" => Self::Border,
            "mandel" => Self::Mandel,
            "gradient" => Self::Gradient,
            "rndall" => Self::RndAll,
            "poly" => Self::Poly,
            _ => return None,
        })
    }
}

/// C4MCNode::int_bool (C4MapCreatorS2.h:181-197).
#[derive(Debug, Clone, Copy, Default)]
struct IntBool {
    value: i32,
    percent: bool,
}

impl IntBool {
    fn new(value: i32, percent: bool) -> Self {
        Self { value, percent }
    }

    fn evaluate(self, relative_to: i32) -> i32 {
        if self.percent {
            self.value * relative_to / SIZE_RES
        } else {
            self.value
        }
    }
}

/// C4MCOverlay fields (C4MapCreatorS2.h:248-298). Maps are overlays with
/// `is_map` (C4MCMap, C4MapCreatorS2.h:327-347).
#[derive(Debug, Clone)]
struct Overlay {
    is_map: bool,
    seed: i32,
    fixed_seed: i32,
    x: i32,
    y: i32,
    wdt: i32,
    hgt: i32,
    off_x: i32,
    off_y: i32,
    rx: IntBool,
    ry: IntBool,
    rwdt: IntBool,
    rhgt: IntBool,
    roff_x: IntBool,
    roff_y: IntBool,
    /// Material NAME; None = MNone (sky).
    material: Option<String>,
    sub: bool,
    texture: String,
    mat_clr: u8,
    op: Op,
    algorithm: Algo,
    turbulence: i32,
    lambda: i32,
    rotate: i32,
    alpha: IntBool,
    beta: IntBool,
    zoom_x: i32,
    zoom_y: i32,
    invert: bool,
    loose_bounds: bool,
    group: bool,
    mask: bool,
}

impl Overlay {
    /// C4MCOverlay::Default (src/C4MapCreatorS2.cpp:290-311).
    fn default_template() -> Self {
        Self {
            is_map: false,
            seed: 0,
            fixed_seed: 0,
            x: 0,
            y: 0,
            wdt: SIZE_RES,
            hgt: SIZE_RES,
            off_x: 0,
            off_y: 0,
            rx: IntBool::new(0, true),
            ry: IntBool::new(0, true),
            rwdt: IntBool::new(SIZE_RES, true),
            rhgt: IntBool::new(SIZE_RES, true),
            roff_x: IntBool::new(0, true),
            roff_y: IntBool::new(0, true),
            material: None,
            // "but if mat is set, assume it sub" (C4MapCreatorS2.cpp:298).
            sub: true,
            texture: String::new(),
            mat_clr: 0,
            op: Op::None,
            algorithm: Algo::Solid,
            turbulence: 0,
            lambda: 0,
            rotate: 0,
            alpha: IntBool::new(0, false),
            beta: IntBool::new(0, false),
            zoom_x: ZOOM_RES,
            zoom_y: ZOOM_RES,
            invert: false,
            loose_bounds: false,
            group: false,
            mask: false,
        }
    }

    fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.wdt && y < self.y + self.hgt
    }
}

/// C4MCPoint (C4MapCreatorS2.h:301-324).
#[derive(Debug, Clone, Default)]
struct Point {
    x: i32,
    y: i32,
    rx: IntBool,
    ry: IntBool,
}

#[derive(Debug, Clone)]
enum NodeKind {
    /// The global scope (C4MapCreatorS2 itself, Type MCN_Node).
    Root,
    Overlay(Overlay),
    Point(Point),
}

#[derive(Debug, Clone)]
struct Node {
    owner: Option<NodeId>,
    children: Vec<NodeId>,
    name: String,
    kind: NodeKind,
}

struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    fn new() -> Self {
        Self {
            nodes: vec![Node {
                owner: None,
                children: Vec::new(),
                name: String::new(),
                kind: NodeKind::Root,
            }],
        }
    }

    fn overlay(&self, id: NodeId) -> Option<&Overlay> {
        match &self.nodes[id].kind {
            NodeKind::Overlay(overlay) => Some(overlay),
            _ => None,
        }
    }

    fn overlay_mut(&mut self, id: NodeId) -> Option<&mut Overlay> {
        match &mut self.nodes[id].kind {
            NodeKind::Overlay(overlay) => Some(overlay),
            _ => None,
        }
    }

    fn add(&mut self, owner: NodeId, name: String, kind: NodeKind) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            owner: Some(owner),
            children: Vec::new(),
            name,
            kind,
        });
        self.nodes[owner].children.push(id);
        id
    }

    /// Deep copy of a subtree under `owner` (the C4MCNode template
    /// constructor, src/C4MapCreatorS2.cpp:137-150): children clone with
    /// their op; the copied node itself keeps the template op only when
    /// `f_clone`. Names survive only at global scope.
    fn copy_from(&mut self, template: NodeId, owner: NodeId, f_clone: bool) -> NodeId {
        let mut node = self.nodes[template].clone();
        if !matches!(self.nodes[owner].kind, NodeKind::Root) {
            node.name = String::new();
        }
        if let NodeKind::Overlay(overlay) = &mut node.kind {
            if !f_clone {
                overlay.op = Op::None;
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            owner: Some(owner),
            children: Vec::new(),
            name: node.name,
            kind: node.kind,
        });
        self.nodes[owner].children.push(id);
        let children = self.nodes[template].children.clone();
        for child in children {
            self.copy_from(child, id, true);
        }
        id
    }

    /// C4MCNode::GetNodeByName (src/C4MapCreatorS2.cpp:200-212): local
    /// list backwards, then the owner chain.
    fn node_by_name(&self, scope: NodeId, name: &str) -> Option<NodeId> {
        for &child in self.nodes[scope].children.iter().rev() {
            if self.nodes[child].name == name {
                return Some(child);
            }
        }
        self.nodes[scope]
            .owner
            .and_then(|owner| self.node_by_name(owner, name))
    }

    /// C4MCNode::OwnerOverlay (src/C4MapCreatorS2.cpp:191-198).
    fn owner_overlay(&self, id: NodeId) -> Option<NodeId> {
        let mut current = self.nodes[id].owner;
        while let Some(owner) = current {
            if self.overlay(owner).is_some() {
                return Some(owner);
            }
            current = self.nodes[owner].owner;
        }
        None
    }

    /// The previous sibling (C4MCNode::Prev).
    fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        let owner = self.nodes[id].owner?;
        let index = self.nodes[owner]
            .children
            .iter()
            .position(|&child| child == id)?;
        (index > 0).then(|| self.nodes[owner].children[index - 1])
    }

    /// The next sibling (C4MCNode::Next).
    fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let owner = self.nodes[id].owner?;
        let index = self.nodes[owner]
            .children
            .iter()
            .position(|&child| child == id)?;
        self.nodes[owner].children.get(index + 1).copied()
    }

    /// C4MCOverlay::FirstOfChain (src/C4MapCreatorS2.cpp:433-446).
    fn first_of_chain(&self, id: NodeId) -> NodeId {
        let mut current = id;
        while let Some(prev) = self.prev_sibling(current) {
            let Some(prev_overlay) = self.overlay(prev) else {
                break;
            };
            if prev_overlay.op == Op::None {
                break;
            }
            current = prev;
        }
        current
    }

    /// C4MCNode::ReEvaluate (src/C4MapCreatorS2.cpp:239-246): the node,
    /// then its children in order — the per-overlay seed draws happen in
    /// this exact pre-order.
    fn re_evaluate(&mut self, id: NodeId, classifier: &mut MapPixelClassifier, rng: &mut LcgRng) {
        self.evaluate(id, classifier, rng);
        let children = self.nodes[id].children.clone();
        for child in children {
            self.re_evaluate(child, classifier, rng);
        }
    }

    /// C4MCOverlay::Evaluate (src/C4MapCreatorS2.cpp:402-431) and
    /// C4MCPoint::Evaluate (src/C4MapCreatorS2.cpp:610-625).
    fn evaluate(&mut self, id: NodeId, classifier: &mut MapPixelClassifier, rng: &mut LcgRng) {
        let owner_overlay = self
            .owner_overlay(id)
            .and_then(|owner| self.overlay(owner))
            .map(|owner| (owner.x, owner.y, owner.wdt, owner.hgt));
        match &mut self.nodes[id].kind {
            NodeKind::Overlay(overlay) => {
                // Mat color: GetIndexMatTex(name, texture-or-default) +128
                // when sub.
                overlay.mat_clr = match &overlay.material {
                    Some(name) => {
                        let texture = (!overlay.texture.is_empty())
                            .then_some(overlay.texture.as_str());
                        let clr = classifier.get_index_mat_tex(name, texture);
                        if overlay.sub {
                            clr.wrapping_add(128)
                        } else {
                            clr
                        }
                    }
                    None => 0,
                };
                if let Some((ox, oy, owdt, ohgt)) = owner_overlay {
                    overlay.x = overlay.rx.evaluate(owdt) + ox;
                    overlay.y = overlay.ry.evaluate(ohgt) + oy;
                    overlay.wdt = overlay.rwdt.evaluate(owdt);
                    overlay.hgt = overlay.rhgt.evaluate(ohgt);
                    overlay.off_x = overlay.roff_x.evaluate(owdt);
                    overlay.off_y = overlay.roff_y.evaluate(ohgt);
                }
                // calc seed (src/C4MapCreatorS2.cpp:430).
                overlay.seed = overlay.fixed_seed;
                if overlay.seed == 0 {
                    overlay.seed = (rng.random(32768) << 16) | rng.random(65536);
                }
            }
            NodeKind::Point(point) => {
                if let Some((ox, oy, owdt, ohgt)) = owner_overlay {
                    point.x = point.rx.evaluate(owdt) + ox;
                    point.y = point.ry.evaluate(ohgt) + oy;
                }
            }
            NodeKind::Root => {}
        }
    }

    /// C4MCOverlay::CheckMask (src/C4MapCreatorS2.cpp:448-504).
    fn check_mask(&self, id: NodeId, ix: i32, iy: i32) -> bool {
        use crate::math::{fixed10, fixtoi_prec, itofix, C4Fixed};
        let overlay = self.overlay(id).expect("check_mask on overlay");
        if !overlay.loose_bounds && !overlay.in_bounds(ix, iy) {
            return false;
        }
        let mut ix = ix;
        let mut iy = iy;
        let mut dx = itofix(ix);
        let mut dy = itofix(iy);
        if overlay.turbulence != 0 {
            // Rad2Grad = itofix(3754936, 65536) — 57.295… degrees/radian.
            let rad2grad = C4Fixed::from_raw(3754936);
            let mut j = 3i32;
            let mut i = 10i32;
            while i <= overlay.turbulence {
                let mut seed2 = overlay.seed;
                for _ in 0..=overlay.lambda {
                    let mut d = itofix(2);
                    while d < 6 {
                        dx += (((dx / 7 + itofix(seed2) / overlay.zoom_x + dy) / j + d)
                            * rad2grad)
                            .sin_deg()
                            * j
                            / 2;
                        dy += (((dy / 7 + itofix(seed2) / overlay.zoom_y + dx) / j - d)
                            * rad2grad)
                            .cos_deg()
                            * j
                            / 2;
                        d += fixed10(15);
                    }
                    seed2 = (overlay
                        .seed
                        .wrapping_mul(seed2.wrapping_shl(3))
                        .wrapping_add(0x4465))
                        & 0xffff;
                }
                j += 3;
                i = i.saturating_mul(10);
            }
        }
        if overlay.rotate != 0 {
            let (dxo, dyo) = (dx, dy);
            let cos = itofix(overlay.rotate).cos_deg();
            let sin = itofix(overlay.rotate).sin_deg();
            dx = dxo * cos - dyo * sin;
            dy = dyo * cos + dxo * sin;
        }
        if overlay.rotate != 0 || overlay.turbulence != 0 {
            ix = fixtoi_prec(dx, overlay.zoom_x);
            iy = fixtoi_prec(dy, overlay.zoom_y);
        } else {
            ix = ix.wrapping_mul(overlay.zoom_x);
            iy = iy.wrapping_mul(overlay.zoom_y);
        }
        // apply offset
        ix -= overlay.off_x.wrapping_mul(overlay.zoom_x);
        iy -= overlay.off_y.wrapping_mul(overlay.zoom_y);
        // check bounds, if loose
        if overlay.loose_bounds
            && (ix < overlay.x * overlay.zoom_x
                || iy < overlay.y * overlay.zoom_y
                || ix >= (overlay.x + overlay.wdt) * overlay.zoom_x
                || iy >= (overlay.y + overlay.hgt) * overlay.zoom_y)
        {
            return overlay.invert;
        }
        self.run_algorithm(id, ix, iy) ^ overlay.invert
    }

    /// C4MCOverlay::RenderPix (src/C4MapCreatorS2.cpp:506-553).
    fn render_pix(
        &self,
        id: NodeId,
        ix: i32,
        iy: i32,
        pix: &mut u8,
        last_op: Op,
        last_set: bool,
        draw: bool,
    ) -> bool {
        let overlay = self.overlay(id).expect("render_pix on overlay");
        let set_this = self.check_mask(id, ix, iy);
        let mut do_set = match last_op {
            Op::And => set_this && last_set,
            Op::Or => set_this || last_set,
            Op::Xor => set_this ^ last_set,
            Op::None => set_this,
        };

        if (do_set && draw && overlay.op == Op::None) || overlay.group {
            // groups don't set a pixel value, if they're associated with
            // an operator
            let draw = draw && (!overlay.group || overlay.op == Op::None);
            if draw && do_set && !overlay.mask {
                *pix = overlay.mat_clr;
            }
            let mut last_set_c = false;
            let mut child_op = Op::None;
            for &child in &self.nodes[id].children {
                if let Some(child_overlay) = self.overlay(child) {
                    last_set_c = self.render_pix(child, ix, iy, pix, child_op, last_set_c, draw);
                    if overlay.group && child_overlay.op == Op::None {
                        do_set |= last_set_c;
                    }
                    child_op = child_overlay.op;
                }
            }
        }
        do_set
    }

    /// C4MCOverlay::PeekPix (src/C4MapCreatorS2.cpp:555-571).
    fn peek_pix(&self, start: NodeId, ix: i32, iy: i32) -> bool {
        let mut current = start;
        let mut last_set = false;
        let mut last_op = Op::None;
        let mut crap = 0u8;
        loop {
            last_set = self.render_pix(current, ix, iy, &mut crap, last_op, last_set, false);
            let overlay = self.overlay(current).expect("peek chain overlay");
            last_op = overlay.op;
            if overlay.op == Op::None {
                break;
            }
            // must be another overlay, since there's an operator
            match self.next_sibling(current).filter(|&next| self.overlay(next).is_some()) {
                Some(next) => current = next,
                None => break,
            }
        }
        last_set
    }

    /// The algorithm dispatch (src/C4MapCreatorS2.cpp:1351-1564).
    fn run_algorithm(&self, id: NodeId, ix: i32, iy: i32) -> bool {
        let overlay = self.overlay(id).expect("algorithm on overlay");
        let s = overlay.seed;
        let z = ZOOM_RES;
        let z2 = ZOOM_RES * ZOOM_RES;
        let modulo = |value: i32, divisor: i32| {
            if divisor == 0 {
                0
            } else {
                value % divisor
            }
        };
        match overlay.algorithm {
            Algo::Solid => true,
            Algo::Random => algo_random(overlay, ix, iy),
            Algo::Checker => modulo(ix.div_euclid(1) / (z * 10), 2) ^ modulo(iy / (z * 10), 2) == 0,
            Algo::Bozo => {
                let ixc = modulo(ix / 10 + s + (iy / 80), z * 2) - z;
                let iyc = modulo(iy / 10 + s + (ix / 80), z * 2) - z;
                let id_value = ixc.wrapping_mul(iyc).abs();
                id_value > z2 * (overlay.alpha.evaluate(SIZE_RES) + 10) / 50
            }
            Algo::Sin => {
                use crate::math::{fixtoi, itofix};
                iy > fixtoi((itofix(ix / z * 10).sin_deg() + 1) * z * 10)
            }
            Algo::Boxes => {
                let pxb = overlay.beta.evaluate(overlay.wdt);
                let pxa = overlay.alpha.evaluate(overlay.wdt);
                modulo((ix + modulo(s, 4738)).abs(), pxb * z + 1) < pxa * z + 1
                    && modulo((iy + (s / 4738)).abs(), pxb * z + 1) < pxa * z + 1
            }
            Algo::RndChecker => algo_random_at(overlay, ix / (z * 10), iy / (z * 10)),
            Algo::Lines => {
                let pxb = overlay.beta.evaluate(overlay.wdt);
                let pxa = overlay.alpha.evaluate(overlay.wdt);
                modulo((ix + modulo(s, 4738)).abs(), pxb * z + 1) < pxa * z + 1
            }
            Algo::Border => self.algo_border(id, ix, iy),
            Algo::Mandel => {
                let mut iter = overlay.alpha.evaluate(SIZE_RES);
                if iter == 0 {
                    iter = 1000;
                }
                let iter = iter.max(10) as u32;
                let c = (f64::from(ix) / f64::from(z) / f64::from(overlay.wdt.max(1))
                    - 0.5 * (f64::from(overlay.zoom_x) / f64::from(z)))
                    * 4.0;
                let ci = (f64::from(iy) / f64::from(z) / f64::from(overlay.hgt.max(1))
                    - 0.5 * (f64::from(overlay.zoom_y) / f64::from(z)))
                    * 4.0;
                let (mut zr, mut zi) = (c, ci);
                let mut i = 0;
                while i < iter {
                    let xz = zr * zr - zi * zi;
                    zi = 2.0 * zr * zi + ci;
                    zr = xz + c;
                    if zr * zr + zi * zi > 4.0 {
                        break;
                    }
                    i += 1;
                }
                i >= iter
            }
            Algo::Gradient => {
                // (abs((iX ^ (iY*3)) * 2531011L) % 214013L) % z — the
                // multiply promotes to 64-bit long on LP64 (no overflow).
                let v = i64::from(ix ^ iy.wrapping_mul(3)) * 2531011;
                (v.abs() % 214013) % i64::from(z) > i64::from(ix / overlay.wdt.max(1))
            }
            Algo::RndAll => modulo(s, 100) < overlay.alpha.evaluate(SIZE_RES),
            Algo::Poly => self.algo_polygon(id, ix, iy),
        }
    }

    /// AlgoBorder (src/C4MapCreatorS2.cpp:1408-1420) with PreparePeek
    /// (src/C4MapCreatorS2.cpp:1327-1343).
    fn algo_border(&self, id: NodeId, ix: i32, iy: i32) -> bool {
        let overlay = self.overlay(id).expect("border overlay");
        let la = overlay.alpha.evaluate(overlay.wdt);
        let lb = overlay.beta.evaluate(overlay.hgt);
        // PreparePeek: zoom out, then peek the owner's operator chain.
        let ix = ix / overlay.zoom_x.max(1);
        let iy = iy / overlay.zoom_y.max(1);
        let Some(owner) = self.owner_overlay(id) else {
            return false;
        };
        let mut top = owner;
        while let Some(next) = self.owner_overlay(top) {
            top = next;
        }
        let chain = self.first_of_chain(owner);
        let top_overlay = self.overlay(top).expect("top overlay");
        for x in (ix - la)..=(ix + la) {
            if top_overlay.in_bounds(x, iy) && !self.peek_pix(chain, x, iy) {
                return true;
            }
        }
        for y in (iy - lb)..=(iy + lb) {
            if top_overlay.in_bounds(ix, y) && !self.peek_pix(chain, ix, y) {
                return true;
            }
        }
        false
    }

    /// AlgoPolygon (src/C4MapCreatorS2.cpp:1474-1564): even-odd ray cast
    /// over the overlay's child points (coordinates scaled by 100).
    fn algo_polygon(&self, id: NodeId, ix: i32, iy: i32) -> bool {
        let points: Vec<(i32, i32)> = self.nodes[id]
            .children
            .iter()
            .filter_map(|&child| match &self.nodes[child].kind {
                NodeKind::Point(point) => Some((point.x * 100, point.y * 100)),
                _ => None,
            })
            .collect();
        if points.is_empty() {
            return false;
        }
        // Get a start point with uY != iY, or the first.
        let start = points
            .iter()
            .rposition(|&(_, y)| y != iy)
            .unwrap_or(points.len() - 1);
        let (mut ux, mut uy) = points[start];
        let mut lx = ux;
        let mut count = 0;
        let mut ignore = false;
        for offset in 1..=points.len() {
            let (cx, cy) = points[(start + offset) % points.len()];
            if ignore {
                if cy == iy {
                    if ((lx < ix) == (ix < cx)) || cx == ix {
                        return true;
                    }
                } else {
                    if ((uy < iy) == (iy < cy)) && lx >= ix {
                        count += 1;
                    }
                    ignore = false;
                    ux = cx;
                    uy = cy;
                }
            } else if cy == iy {
                if cx == ix {
                    return true;
                }
                ignore = true;
            } else {
                if (uy < iy) == (iy <= cy) {
                    if ix < ux.min(cx) {
                        count += 1;
                    } else if ix <= ux.max(cx) {
                        let zx = (cx - ux) * (iy - uy) / (cy - uy) + ux;
                        if ix < zx {
                            count += 1;
                        }
                        if zx == ix {
                            return true;
                        }
                    }
                }
                ux = cx;
                uy = cy;
            }
            lx = cx;
        }
        (count & 1) > 0
    }
}

/// AlgoRandom (src/C4MapCreatorS2.cpp:1357-1361).
fn algo_random(overlay: &Overlay, ix: i32, iy: i32) -> bool {
    algo_random_seeded(overlay.seed, overlay.alpha.evaluate(SIZE_RES), ix, iy)
}

fn algo_random_at(overlay: &Overlay, ix: i32, iy: i32) -> bool {
    algo_random_seeded(overlay.seed, overlay.alpha.evaluate(SIZE_RES), ix, iy)
}

fn algo_random_seeded(s: i32, a: i32, ix: i32, iy: i32) -> bool {
    let mixed = (s ^ ix.wrapping_shl(2) ^ iy.wrapping_shl(5))
        ^ ((s >> 16)
            .wrapping_add(1)
            .wrapping_add(ix)
            .wrapping_add(iy.wrapping_shl(2)));
    let divisor = a + 2;
    if divisor == 0 {
        return false;
    }
    (mixed / 17) % divisor == 0
}

// ── parser ───────────────────────────────────────────────────────────────────

/// C4MCTokenType (C4MapCreatorS2.h:91-108).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Dir(String),
    Idtf(String),
    Int(i32),
    Percent(i32),
    Px(i32),
    Eq,
    BlOpen,
    BlClose,
    SColon,
    And,
    Or,
    Xor,
    Range,
    Eof,
}

/// Value kinds a field setter receives (int-ish tokens keep their flavor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValType {
    Int,
    Percent,
    Px,
    Idtf,
}

struct Tokenizer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
        }
    }

    /// C4MCParser::AdvanceSpaces (src/C4MapCreatorS2.cpp:853-889).
    fn advance_spaces(&mut self) -> bool {
        let mut in_comment = 0u8;
        let mut prev = 0u8;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            match in_comment {
                0 => {
                    if c == b'/' {
                        match self.bytes.get(self.pos + 1) {
                            Some(b'/') => {
                                in_comment = 1;
                                self.pos += 1;
                            }
                            Some(b'*') => {
                                in_comment = 2;
                                self.pos += 1;
                            }
                            _ => return true,
                        }
                    } else if c > 32 {
                        return true;
                    }
                }
                1 => {
                    if c == 13 || c == 10 {
                        in_comment = 0;
                    }
                }
                _ => {
                    if c == b'/' && prev == b'*' {
                        in_comment = 0;
                    }
                }
            }
            self.pos += 1;
            prev = c;
        }
        false
    }

    /// C4MCParser::GetNextToken (src/C4MapCreatorS2.cpp:891-980).
    fn next(&mut self) -> Result<Token, String> {
        if !self.advance_spaces() {
            return Ok(Token::Eof);
        }
        let start = self.pos;
        let first = self.bytes[self.pos];
        match first {
            b';' => {
                self.pos += 1;
                return Ok(Token::SColon);
            }
            b'=' => {
                self.pos += 1;
                return Ok(Token::Eq);
            }
            b'{' => {
                self.pos += 1;
                return Ok(Token::BlOpen);
            }
            b'}' => {
                self.pos += 1;
                return Ok(Token::BlClose);
            }
            b'&' => {
                self.pos += 1;
                return Ok(Token::And);
            }
            b'|' => {
                self.pos += 1;
                return Ok(Token::Or);
            }
            b'^' => {
                self.pos += 1;
                return Ok(Token::Xor);
            }
            _ => {}
        }
        if first.is_ascii_digit() || first == b'+' || first == b'-' {
            // integer (or the '-' range operator)
            self.pos += 1;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                self.pos += 1;
            }
            let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("");
            if text == "-" {
                return Ok(Token::Range);
            }
            let value: i32 = text.parse().map_err(|_| format!("bad integer `{text}`"))?;
            match self.bytes.get(self.pos) {
                Some(b'%') => {
                    self.pos += 1;
                    Ok(Token::Percent(value))
                }
                Some(b'p') => {
                    self.pos += 1;
                    if self.bytes.get(self.pos) == Some(&b'x') {
                        self.pos += 1;
                    }
                    Ok(Token::Px(value))
                }
                _ => Ok(Token::Int(value)),
            }
        } else if first == b'#' {
            self.pos += 1;
            let idtf_start = self.pos;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                self.pos += 1;
            }
            Ok(Token::Dir(
                String::from_utf8_lossy(&self.bytes[idtf_start..self.pos]).into_owned(),
            ))
        } else if first >= b'@' {
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                self.pos += 1;
            }
            Ok(Token::Idtf(
                String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned(),
            ))
        } else {
            Err("unexpected character found".to_string())
        }
    }
}

/// The parser (C4MCParser::ParseTo, src/C4MapCreatorS2.cpp:997-1223).
struct Parser<'a, 'b> {
    tokens: Tokenizer<'a>,
    tree: Tree,
    default_map: Overlay,
    classifier: &'b mut MapPixelClassifier,
    rng: &'b mut LcgRng,
}

impl Parser<'_, '_> {
    fn parse_into(&mut self, to_node: NodeId) -> Result<(), String> {
        #[derive(PartialEq)]
        enum State {
            None,
            Keywd1,
            Keywd1N,
            AfterNode,
            GotOp,
            GotIdtf,
            GotOpIdtf,
            SetField,
        }
        let global_scope = matches!(self.tree.nodes[to_node].kind, NodeKind::Root);
        let mut state = State::None;
        let mut new_node: Option<NodeId> = None;
        let mut field_name = String::new();
        let mut idtf = String::new();
        loop {
            let mut token = self.tokens.next()?;
            if token == Token::Eof && state == State::None {
                break;
            }
            let mut done = false;
            match state {
                State::None | State::GotOp => {
                    let was_got_op = state == State::GotOp;
                    match &token {
                        Token::Dir(name) => {
                            if !global_scope {
                                return Err("can't use directives in local scope".into());
                            }
                            return Err(format!("unknown directive: {name}"));
                        }
                        Token::Idtf(name) => {
                            if name == "overlay" {
                                let id = self.tree.add(
                                    to_node,
                                    String::new(),
                                    NodeKind::Overlay(Overlay::default_template()),
                                );
                                new_node = Some(id);
                                state = State::Keywd1;
                            } else if name == "point"
                                && self.tree.node_by_name(to_node, name).is_none()
                            {
                                if global_scope {
                                    return Err("point only allowed in overlays".into());
                                }
                                let id = self.tree.add(
                                    to_node,
                                    String::new(),
                                    NodeKind::Point(Point::default()),
                                );
                                new_node = Some(id);
                                state = State::Keywd1;
                                // operator type check (C4MapCreatorS2.cpp:
                                // 1067-1069): the last operand was an
                                // overlay (points cannot carry ops).
                                if was_got_op {
                                    return Err("operator type mismatch".into());
                                }
                            } else if name == "map" {
                                if !global_scope {
                                    return Err("can't declare map in local scope".into());
                                }
                                let mut map = self.default_map.clone();
                                map.op = Op::None;
                                let id =
                                    self.tree.add(to_node, String::new(), NodeKind::Overlay(map));
                                new_node = Some(id);
                                state = State::Keywd1;
                            } else {
                                idtf = name.clone();
                                state = if was_got_op {
                                    State::GotOpIdtf
                                } else {
                                    State::GotIdtf
                                };
                            }
                        }
                        Token::BlClose | Token::Eof => done = true,
                        _ => return Err("identifier expected".into()),
                    }
                }
                State::Keywd1 => {
                    if let Token::Idtf(name) = &token {
                        let id = new_node.expect("named node");
                        self.tree.nodes[id].name = name.clone();
                        state = State::Keywd1N;
                    } else if global_scope {
                        return Err("unnamed objects not allowed in global scope".into());
                    } else if token == Token::BlOpen {
                        self.parse_into(new_node.expect("block node"))?;
                        state = State::AfterNode;
                    } else {
                        return Err("'{' expected".into());
                    }
                }
                State::Keywd1N => {
                    if token != Token::BlOpen {
                        return Err("'{' expected".into());
                    }
                    self.parse_into(new_node.expect("block node"))?;
                    state = State::AfterNode;
                }
                State::GotIdtf | State::GotOpIdtf => {
                    let was_op = state == State::GotOpIdtf;
                    match &token {
                        Token::Eq => {
                            if was_op {
                                return Err("second operand expected".into());
                            }
                            field_name = idtf.clone();
                            state = State::SetField;
                        }
                        Token::BlOpen | Token::SColon | Token::And | Token::Or | Token::Xor => {
                            // node copy by name (C4MapCreatorS2.cpp:
                            // 1122-1167)
                            if global_scope {
                                return Err(format!(
                                    "can't reinstanciate object '{idtf}' in global scope"
                                ));
                            }
                            let template = self
                                .tree
                                .node_by_name(to_node, &idtf)
                                .ok_or_else(|| format!("unknown object: {idtf}"))?;
                            match &self.tree.nodes[template].kind {
                                NodeKind::Overlay(overlay) if !overlay.is_map => {}
                                _ => {
                                    return Err(format!(
                                        "can't reinstanciate '{idtf}'; object type is unknown"
                                    ))
                                }
                            }
                            let id = self.tree.copy_from(template, to_node, false);
                            new_node = Some(id);
                            if token == Token::BlOpen {
                                self.parse_into(id)?;
                                // fall through to AfTERNODE with the token
                                // AFTER the block (C4MapCreatorS2.cpp:
                                // 1155-1164).
                                token = self.tokens.next()?;
                                if token == Token::Eof {
                                    return Err("unexpected end of file".into());
                                }
                            }
                            state = if self.after_node(&token, global_scope, &mut new_node)? {
                                State::GotOp
                            } else {
                                State::None
                            };
                        }
                        _ => return Err("'=', ';' or '{' expected".into()),
                    }
                }
                State::AfterNode => {
                    state = if self.after_node(&token, global_scope, &mut new_node)? {
                        State::GotOp
                    } else {
                        State::None
                    };
                }
                State::SetField => {
                    self.parse_value(to_node, &field_name, token)?;
                    state = State::None;
                }
            }
            if done {
                break;
            }
        }
        match state {
            State::None => Ok(()),
            State::GotOp => Err("second operand expected".into()),
            _ => Err("unexpected end of file".into()),
        }
    }

    /// PS_AFTERNODE (src/C4MapCreatorS2.cpp:1175-1205): `;` or an
    /// operator; top-level nodes evaluate here. Returns whether an
    /// operator follows (PS_GOTOP).
    fn after_node(
        &mut self,
        token: &Token,
        global_scope: bool,
        new_node: &mut Option<NodeId>,
    ) -> Result<bool, String> {
        let got_op = match token {
            Token::SColon => false,
            Token::And | Token::Or | Token::Xor => {
                if global_scope {
                    return Err("operators not allowed in global scope".into());
                }
                let id = new_node.ok_or("';' expected")?;
                let Some(overlay) = self.tree.overlay_mut(id) else {
                    // SetOp fails on points (C4MapCreatorS2.cpp:1190).
                    return Err("';' expected".into());
                };
                overlay.op = match token {
                    Token::And => Op::And,
                    Token::Or => Op::Or,
                    _ => Op::Xor,
                };
                true
            }
            _ => return Err("';' or operator expected".into()),
        };
        // evaluate node and children, if this is top-level
        // (C4MapCreatorS2.cpp:1201-1203)
        if global_scope {
            if let Some(id) = *new_node {
                self.tree.re_evaluate(id, self.classifier, self.rng);
            }
        }
        *new_node = None;
        Ok(got_op)
    }

    /// C4MCParser::ParseValue (src/C4MapCreatorS2.cpp:1225-1279).
    fn parse_value(
        &mut self,
        to_node: NodeId,
        field_name: &str,
        token: Token,
    ) -> Result<(), String> {
        match token {
            Token::Idtf(value) => {
                self.set_field(to_node, field_name, &value, 0, ValType::Idtf)?;
                let next = self.tokens.next()?;
                if next == Token::Eof {
                    return Err("unexpected end of file".into());
                }
                if next != Token::SColon {
                    return Err("';' expected".into());
                }
                Ok(())
            }
            Token::Int(value) | Token::Px(value) | Token::Percent(value) => {
                let mut value = value;
                let mut val_type = match token {
                    Token::Int(_) => ValType::Int,
                    Token::Px(_) => ValType::Px,
                    _ => ValType::Percent,
                };
                let mut next = self.tokens.next()?;
                if next == Token::Range {
                    // `lo - hi`: one synced draw at PARSE time
                    // (src/C4MapCreatorS2.cpp:1250-1263).
                    next = self.tokens.next()?;
                    match next {
                        Token::Int(hi) | Token::Px(hi) | Token::Percent(hi) => {
                            value += self.rng.random(hi - value);
                            val_type = match next {
                                Token::Int(_) => ValType::Int,
                                Token::Px(_) => ValType::Px,
                                _ => ValType::Percent,
                            };
                        }
                        _ => return Err(format!("constant for field '{field_name}' expected")),
                    }
                    next = self.tokens.next()?;
                }
                self.set_field(to_node, field_name, "", value, val_type)?;
                if next != Token::SColon {
                    return Err("';' expected".into());
                }
                Ok(())
            }
            _ => Err(format!("constant for field '{field_name}' expected")),
        }
    }

    /// C4MCOverlay::SetField / C4MCPoint::SetField with the C4MCOvrlMap
    /// attribute table (src/C4MapCreatorS2.cpp:313-388,593-608,1591-1618).
    fn set_field(
        &mut self,
        node: NodeId,
        field: &str,
        s_val: &str,
        i_val: i32,
        val_type: ValType,
    ) -> Result<(), String> {
        let int_par = || -> Result<i32, String> {
            match val_type {
                ValType::Int | ValType::Percent | ValType::Px => Ok(i_val),
                ValType::Idtf => Err(format!("'{s_val}' is not a valid value for this field")),
            }
        };
        let str_par = || -> Result<&str, String> {
            match val_type {
                ValType::Idtf => Ok(s_val),
                _ => Err(format!("'{s_val}' is not a valid value for this field")),
            }
        };
        match &mut self.tree.nodes[node].kind {
            NodeKind::Point(point) => {
                // only explicit %/px (src/C4MapCreatorS2.cpp:595-607)
                if val_type == ValType::Int {
                    return Err(format!("field '{field}' not found"));
                }
                match field {
                    "x" => point.rx = IntBool::new(int_par()?, val_type == ValType::Percent),
                    "y" => point.ry = IntBool::new(int_par()?, val_type == ValType::Percent),
                    _ => return Err(format!("field '{field}' not found")),
                }
                Ok(())
            }
            NodeKind::Overlay(_) => {
                // C4MCV_Percent: plain ints count as percent; C4MCV_Pixels:
                // plain ints count as px (src/C4MapCreatorS2.cpp:329-337).
                let percent_par =
                    |value: i32| IntBool::new(value, matches!(val_type, ValType::Percent | ValType::Int));
                let pixels_par = |value: i32| IntBool::new(value, val_type == ValType::Percent);
                // Field validation that needs &mut self.classifier happens
                // before re-borrowing the overlay.
                enum Pending {
                    Material(String),
                    Texture(String),
                    Algo(Algo),
                }
                let pending = match field {
                    "mat" => {
                        let name = str_par()?.to_string();
                        // MatMap->Get validity (src/C4MapCreatorS2.cpp:341-343);
                        // "Sky" is the MNone idiom in shipped content.
                        if self.classifier.material(&name).is_none() {
                            if name.eq_ignore_ascii_case("sky") {
                                None
                            } else {
                                return Err(format!("material '{name}' not found"));
                            }
                        } else {
                            Some(Pending::Material(name))
                        }
                    }
                    "tex" => {
                        let name = str_par()?.to_string();
                        if !self.classifier.texture_exists(&name) {
                            return Err(format!("texture '{name}' not found"));
                        }
                        Some(Pending::Texture(name))
                    }
                    "algo" => {
                        let name = str_par()?;
                        let algo = Algo::by_name(name)
                            .ok_or_else(|| format!("algorithm '{name}' not found"))?;
                        Some(Pending::Algo(algo))
                    }
                    "evalFn" | "drawFn" => {
                        return Err(format!(
                            "script func '{}' not supported by the rust map creator",
                            str_par()?
                        ));
                    }
                    _ => None,
                };
                let overlay = self.tree.overlay_mut(node).expect("overlay field");
                match field {
                    "x" => overlay.rx = percent_par(int_par()?),
                    "y" => overlay.ry = percent_par(int_par()?),
                    "wdt" => overlay.rwdt = percent_par(int_par()?),
                    "hgt" => overlay.rhgt = percent_par(int_par()?),
                    "ox" => overlay.roff_x = percent_par(int_par()?),
                    "oy" => overlay.roff_y = percent_par(int_par()?),
                    "mat" => {
                        overlay.material = match pending {
                            Some(Pending::Material(name)) => Some(name),
                            _ => None,
                        }
                    }
                    "tex" => {
                        if let Some(Pending::Texture(name)) = pending {
                            overlay.texture = name;
                        }
                    }
                    "algo" => {
                        if let Some(Pending::Algo(algo)) = pending {
                            overlay.algorithm = algo;
                        }
                    }
                    "sub" => overlay.sub = int_par()? != 0,
                    // C4MCV_Zoom: BoundBy(ZoomRes - value, 1, 2*ZoomRes)
                    // (src/C4MapCreatorS2.cpp:366-368).
                    "zoomX" => overlay.zoom_x = (ZOOM_RES - int_par()?).clamp(1, ZOOM_RES * 2),
                    "zoomY" => overlay.zoom_y = (ZOOM_RES - int_par()?).clamp(1, ZOOM_RES * 2),
                    "a" => overlay.alpha = pixels_par(int_par()?),
                    "b" => overlay.beta = pixels_par(int_par()?),
                    "turbulence" => overlay.turbulence = int_par()?,
                    "lambda" => overlay.lambda = int_par()?,
                    "rotate" => overlay.rotate = int_par()?,
                    "seed" => overlay.fixed_seed = int_par()?,
                    "invert" => overlay.invert = int_par()? != 0,
                    "loosebounds" => overlay.loose_bounds = int_par()? != 0,
                    "grp" => overlay.group = int_par()? != 0,
                    "mask" => overlay.mask = int_par()? != 0,
                    _ => return Err(format!("field '{field}' not found")),
                }
                Ok(())
            }
            NodeKind::Root => Err(format!("field '{field}' not found")),
        }
    }
}

/// C4Landscape::CreateMapS2 (src/C4Landscape.cpp:530-546): construct the
/// creator (DefaultMap evaluates the scenario map size — two draws), parse
/// `Landscape.txt`, render the last complete `map` node. A parse error is
/// logged and rendering proceeds with the nodes parsed so far, exactly
/// like C++ ignoring ReadFile's return value (src/C4Landscape.cpp:540).
/// `Ok(None)` = no map node; the caller falls back to the basic creator.
pub(crate) fn create_s2_map(
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
) -> Option<lc_resources::bitmap::IndexedBitmap> {
    // C4MCMap::Default (src/C4MapCreatorS2.cpp:633-644) runs at creator
    // construction: MapWdt/MapHgt evaluate through the synced rng.
    let (wdt, hgt) = evaluate_map_size(map_width, map_height, map_player_extend, player_count, rng);
    let mut default_map = Overlay::default_template();
    default_map.is_map = true;
    default_map.wdt = wdt;
    default_map.hgt = hgt;

    let mut parser = Parser {
        tokens: Tokenizer::new(source),
        tree: Tree::new(),
        default_map,
        classifier,
        rng,
    };
    if let Err(error) = parser.parse_into(0) {
        // C4MCParserErr::show (src/C4MapCreatorS2.cpp:823-827): log and
        // carry on with whatever parsed.
        tracing::warn!(%error, "Landscape.txt parse error; rendering the nodes parsed so far");
    }
    let tree = parser.tree;

    // GetMap(nullptr): the last map entry (src/C4MapCreatorS2.cpp:786-792).
    let map = tree.nodes[0]
        .children
        .iter()
        .rev()
        .find(|&&child| tree.overlay(child).is_some_and(|overlay| overlay.is_map))
        .copied()?;
    let map_overlay = tree.overlay(map).expect("map overlay");
    let (wdt, hgt) = (map_overlay.wdt, map_overlay.hgt);
    if wdt <= 0 || hgt <= 0 {
        return None;
    }

    // C4MCMap::RenderTo (src/C4MapCreatorS2.cpp:646-674).
    let mut bytes = vec![0u8; (wdt * hgt) as usize];
    for iy in 0..hgt {
        for ix in 0..wdt {
            let pix = &mut bytes[(iy * wdt + ix) as usize];
            *pix = 0;
            tree.render_pix(map, ix, iy, pix, Op::None, false, true);
        }
    }
    Some(lc_resources::bitmap::IndexedBitmap {
        width: wdt as u32,
        height: hgt as u32,
        indices: bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_classifier() -> MapPixelClassifier {
        let mut densities = [0i32; 128];
        let mut names: Vec<Option<String>> = vec![None; 128];
        let mut textures: Vec<Option<String>> = vec![None; 128];
        for (slot, (name, texture, density)) in [
            ("Earth", "Smooth3", 100),
            ("Earth", "Rough", 100),
            ("Rock", "Ridge", 100),
            ("Water", "Smooth", 25),
            ("Gold", "Rough", 100),
        ]
        .iter()
        .enumerate()
        {
            let slot = slot + 1;
            densities[slot] = *density;
            names[slot] = Some(name.to_string());
            textures[slot] = Some(texture.to_string());
        }
        let library = lc_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\n\
             [Material]\nName=Rock\nDensity=100\n\
             [Material]\nName=Water\nDensity=25\n\
             [Material]\nName=Gold\nDensity=100\n",
        )
        .expect("test materials parse");
        MapPixelClassifier::from_slots_with_library(
            densities,
            names,
            textures,
            vec![None; 128],
            library,
            ["smooth3", "rough", "ridge", "smooth"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    }

    fn params() -> (LegacyC4SVal, LegacyC4SVal) {
        (
            LegacyC4SVal::new(20, 0, 10, 250),
            LegacyC4SVal::new(10, 0, 10, 250),
        )
    }

    #[test]
    fn solid_map_fills_with_the_overlay_material_and_ift() {
        // A full-size solid overlay: every pixel gets GetIndexMatTex(mat,
        // tex) + 128 (Sub default true, C4MapCreatorS2.cpp:298,410).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let map = create_s2_map(
            "map Test { overlay { mat = Earth; tex = Smooth3; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        assert_eq!((map.width, map.height), (20, 10));
        assert!(map.indices.iter().all(|&byte| byte == 1 | 0x80));
    }

    #[test]
    fn sub_zero_drops_the_ift_bit() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let map = create_s2_map(
            "map Test { overlay { mat = Earth; tex = Smooth3; sub = 0; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        assert!(map.indices.iter().all(|&byte| byte == 1));
    }

    #[test]
    fn percent_bounds_clip_the_overlay_rect() {
        // x/y/wdt/hgt percent of the owner (C4MCOverlay::Evaluate,
        // C4MapCreatorS2.cpp:415-428): 50% width on a 20x10 map = 10 px.
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let map = create_s2_map(
            "map Test { overlay { x = 0; y = 0; wdt = 50; hgt = 100; mat = Earth; tex = Rough; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        assert_eq!(at(0, 0), 2 | 0x80);
        assert_eq!(at(9, 9), 2 | 0x80);
        assert_eq!(at(10, 0), 0, "right half stays sky");
    }

    #[test]
    fn and_operator_chains_intersect() {
        // ovl1 & ovl2: the chain draws only where BOTH masks hold
        // (RenderPix eLastOp AND, C4MapCreatorS2.cpp:514-516).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let map = create_s2_map(
            "map Test { overlay { wdt = 50; mat = Earth; tex = Rough; } & overlay { x = 25; wdt = 50; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        // Chain: first overlay 0..10, second 5..15 — the SECOND draws the
        // intersection with ITS color (sky, mat unset → 0)? No: the LAST
        // overlay of the chain does the set; its mat is unset so MatClr=0.
        // Overlap pixels render 0; outside stays 0.
        assert_eq!(at(7, 5), 0);
        // Give the second overlay a material to see the intersection:
        let mut rng = LcgRng::seed_from_u64(1);
        let map = create_s2_map(
            "map Test { overlay { wdt = 50; } & overlay { x = 25; wdt = 50; mat = Rock; tex = Ridge; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        assert_eq!(at(4, 5), 0, "left-only region unset");
        assert_eq!(at(7, 5), 3 | 0x80, "intersection set by the chain tail");
        assert_eq!(at(12, 5), 0, "right-only region unset");
    }

    #[test]
    fn named_template_copies_reinstantiate_with_overloads() {
        // overlay Tmpl {...}; map { Tmpl { mat = Rock; }; } — the copy
        // clones the template and the overload block overrides fields
        // (PS_GOTIDTF node copy, C4MapCreatorS2.cpp:1122-1167).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let map = create_s2_map(
            "overlay Tmpl { mat = Earth; tex = Rough; wdt = 50; };\n\
             map Test { Tmpl { mat = Rock; tex = Ridge; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        assert_eq!(at(0, 0), 3 | 0x80, "copy carries the overridden Rock");
        assert_eq!(at(15, 0), 0, "template wdt=50% survives the copy");
    }

    #[test]
    fn evaluation_draws_two_seeds_per_overlay_in_pre_order() {
        // DefaultMap size: 2 draws at construction; each overlay/map node
        // without seed= draws Random(32768)+Random(65536) at Evaluate
        // (C4MapCreatorS2.cpp:430), pre-order per top-level node.
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let base = rng.count;
        let (w, h) = params();
        create_s2_map(
            "overlay Tmpl { mat = Earth; tex = Rough; };\n\
             map Test { overlay { }; Tmpl; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        );
        // 2 (map size) + 2 (Tmpl template) + 2 (map) + 2 (inner overlay)
        // + 2 (Tmpl copy) = 10.
        assert_eq!(rng.count - base, 10);
    }

    #[test]
    fn fixed_seed_draws_nothing() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let base = rng.count;
        let (w, h) = params();
        create_s2_map(
            "map Test { seed = 42; overlay { seed = 7; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        );
        assert_eq!(rng.count - base, 2, "only the DefaultMap size evaluates");
    }

    #[test]
    fn range_values_draw_at_parse_time() {
        // `a - b` consumes Random(b - a) DURING parsing
        // (C4MapCreatorS2.cpp:1250-1263).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let base = rng.count;
        let (w, h) = params();
        create_s2_map(
            "map Test { seed = 42; turbulence = 10 - 100; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        );
        assert_eq!(rng.count - base, 3, "map size + one range draw");
    }

    #[test]
    fn border_algo_rims_the_owner_mask() {
        // border draws where an a/b-neighborhood pixel of the OWNER chain
        // is unset (AlgoBorder, C4MapCreatorS2.cpp:1408-1420).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let map = create_s2_map(
            "map Test { overlay { x = 10; y = 10; wdt = 80; hgt = 80; mat = Earth; tex = Rough; \
             overlay { algo = border; a = 1; b = 1; mat = Rock; tex = Ridge; }; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .expect("map renders");
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        // Owner overlay: x=2..18, y=1..9 (percent of 20x10). Its border
        // pixels turn Rock, interior stays Earth.
        assert_eq!(at(2, 5), 3 | 0x80, "left rim is Rock");
        assert_eq!(at(9, 5), 2 | 0x80, "interior stays Earth-Rough");
        assert_eq!(at(1, 5), 0, "outside stays sky");
    }

    #[test]
    fn skies_of_fire_like_script_parses_and_renders_materials() {
        // The SkiesOfFire structure: global templates + a map of grouped
        // turbulent overlays (content/Fantasy.c4f/SkiesOfFire.c4s).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(0);
        // The real scenario renders 150x150; heavy turbulence needs room.
        let (w, h) = (
            LegacyC4SVal::new(150, 0, 10, 250),
            LegacyC4SVal::new(150, 0, 10, 250),
        );
        let source = "overlay InMat { algo = rndchecker; a = 14; turbulence=1000; zoomX=-50; zoomY=-50; };\n\
             overlay AltTex { algo = rndchecker; a = 2; turbulence=1000; };\n\
             map Test {\n\
               overlay { x=5; y=5; wdt=90; hgt=90; grp=1; loosebounds=1; turbulence=1000; } &\n\
               overlay {\n\
                 mat = Earth; tex = Smooth3;\n\
                 algo = rndchecker; turbulence = 100; a = 1; zoomX = 60; zoomY = 60;\n\
                 AltTex { mat = Earth; tex = Rough; };\n\
                 InMat { a = 6; mat = Rock; tex = Ridge; };\n\
               };\n\
             };";
        let map = create_s2_map(source, &mut classifier, w, h, false, 1, &mut rng)
            .expect("map renders");
        let mut histogram = std::collections::BTreeMap::new();
        for &byte in &map.indices {
            *histogram.entry(byte & 0x7f).or_insert(0usize) += 1;
        }
        assert!(
            histogram.contains_key(&1) || histogram.contains_key(&2),
            "earth appears: {histogram:?}"
        );
        assert!(
            histogram.len() >= 3,
            "several materials + sky appear: {histogram:?}"
        );
    }
}
