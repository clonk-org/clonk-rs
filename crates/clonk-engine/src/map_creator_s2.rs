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
//! `evalFn=`/`drawFn=` record pixels during rendering for the post-landscape
//! scenario-script callback phase. `algo=script` synchronously calls the
//! live scenario host's `ScriptAlgo<Name>` function at each native algorithm
//! evaluation point; a missing function or caught script error is false.

use crate::map_creator::evaluate_map_size;
use crate::rng::LcgRng;
use crate::scenario::{LegacyC4SVal, MapPixelClassifier};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// `C4MC_SizeRes` — positions in percent (C4MapCreatorS2.h:29).
const SIZE_RES: i32 = 100;
/// `C4MC_ZoomRes` (C4MapCreatorS2.h:30).
const ZOOM_RES: i32 = 100;

type NodeId = usize;
type CallbackId = usize;

/// Live `C4MCOverlay::AlgoScript` dispatch. The caller owns the script host,
/// while the map creator owns the active C4Landscape::FixRandom ledger.
/// Passing that ledger through the seam keeps script-side `Random()` calls in
/// exact render traversal order without coupling this parser to clonk-script.
pub(crate) type ScriptAlgoCall<'a> = dyn FnMut(&mut LcgRng, &str, [i32; 4]) -> bool + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    None,
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    Script,
    RndAll,
    Poly,
}

impl Algo {
    /// C4MCAlgoMap (src/C4MapCreatorS2.cpp:1572-1589).
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
            "script" => Self::Script,
            "rndall" => Self::RndAll,
            "poly" => Self::Poly,
            _ => return None,
        })
    }
}

/// C4MCNode::int_bool (C4MapCreatorS2.h:181-197).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Creator-global callback arrays. Template clones copy these IDs so all
    /// instances union their enabled pixels into the original array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    eval_callback: Option<CallbackId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draw_callback: Option<CallbackId>,
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
            eval_callback: None,
            draw_callback: None,
        }
    }

    fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.wdt && y < self.y + self.hgt
    }
}

/// C4MCPoint (C4MapCreatorS2.h:301-324).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
    rx: IntBool,
    ry: IntBool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum NodeKind {
    /// The global scope (C4MapCreatorS2 itself, Type MCN_Node).
    Root,
    Overlay(Overlay),
    Point(Point),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Node {
    owner: Option<NodeId>,
    children: Vec<NodeId>,
    name: String,
    kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Tree {
    nodes: Vec<Node>,
    /// C4MapCreatorS2::CallbackArrays in successful field-assignment order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    callbacks: Vec<CallbackDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CallbackDefinition {
    function: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallbackArray {
    function: String,
    width: i32,
    height: i32,
    bits: Vec<u8>,
}

/// Pixel masks deferred from C4MCMap::RenderTo to
/// C4Landscape::PostInitMap. They remain live on the retained creator until
/// PostInitMap finishes so callback-triggered DrawDefMap/DrawMap calls can add
/// pixels that have not yet been visited by the descending execution pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PostInitMapCallbacks {
    arrays: Vec<CallbackArray>,
    map_zoom: i32,
}

impl PostInitMapCallbacks {
    pub(crate) fn set_map_zoom(&mut self, map_zoom: i32) {
        self.map_zoom = map_zoom;
    }

    pub(crate) fn invocations(&self) -> impl Iterator<Item = (&str, [i32; 3])> + '_ {
        let zoom = self.map_zoom;
        self.arrays.iter().flat_map(move |array| {
            let size = array.width.saturating_mul(array.height).max(0) as usize;
            (0..size).rev().filter_map(move |index| {
                let enabled = array
                    .bits
                    .get(index / 8)
                    .is_some_and(|byte| byte & (1 << (index % 8)) != 0);
                enabled.then(|| {
                    let index = index as i32;
                    (
                        array.function.as_str(),
                        [
                            (index % array.width) * zoom - zoom / 2,
                            (index / array.width) * zoom - zoom / 2,
                            zoom,
                        ],
                    )
                })
            })
        })
    }

    fn append_from(&mut self, other: &Self) {
        self.arrays.extend(other.arrays.iter().cloned());
        if other.map_zoom != 0 {
            self.map_zoom = other.map_zoom;
        }
    }

    pub(crate) fn array_count(&self) -> usize {
        self.arrays.len()
    }

    pub(crate) fn array_size(&self, array: usize) -> usize {
        self.arrays
            .get(array)
            .map(|array| array.width.saturating_mul(array.height).max(0) as usize)
            .unwrap_or(0)
    }

    pub(crate) fn invocation_at(&self, array: usize, index: usize) -> Option<(String, [i32; 3])> {
        let array = self.arrays.get(array)?;
        let enabled = array
            .bits
            .get(index / 8)
            .is_some_and(|byte| byte & (1 << (index % 8)) != 0);
        enabled.then(|| {
            let index = index as i32;
            (
                array.function.clone(),
                [
                    (index % array.width) * self.map_zoom - self.map_zoom / 2,
                    (index / array.width) * self.map_zoom - self.map_zoom / 2,
                    self.map_zoom,
                ],
            )
        })
    }

    fn merge_runtime_clone_prefix_from(&mut self, other: &Self, count: usize) {
        for (target, source) in self.arrays.iter_mut().zip(&other.arrays).take(count) {
            // A DrawMap clone's overlays still point at arrays owned by the
            // original creator. If such an array has no bitmap yet,
            // EnablePixel consults the ORIGINAL pCurrentMap (null while the
            // clone renders) and therefore cannot allocate it.
            if target.is_empty() {
                continue;
            }
            for index in 0..source.width.saturating_mul(source.height).max(0) as usize {
                if source
                    .bits
                    .get(index / 8)
                    .is_some_and(|byte| byte & (1 << (index % 8)) != 0)
                {
                    let index = index as i32;
                    target.enable(
                        index % source.width,
                        index / source.width,
                        source.width,
                        source.height,
                    );
                }
            }
        }
    }

    /// Merge the callback masks produced by rendering another map on the
    /// SAME creator. Existing arrays retain their first map dimensions and
    /// accumulate in-bounds pixels; arrays declared by the appended source
    /// are added in declaration order.
    fn merge_live_render_from(&mut self, other: &Self, retained_count: usize) {
        for index in 0..retained_count.min(other.arrays.len()) {
            let source = &other.arrays[index];
            let Some(target) = self.arrays.get_mut(index) else {
                self.arrays.push(source.clone());
                continue;
            };
            if target.is_empty() {
                *target = source.clone();
                continue;
            }
            for bit_index in 0..source.width.saturating_mul(source.height).max(0) as usize {
                if source
                    .bits
                    .get(bit_index / 8)
                    .is_some_and(|byte| byte & (1 << (bit_index % 8)) != 0)
                {
                    let bit_index = bit_index as i32;
                    target.enable(
                        bit_index % source.width,
                        bit_index / source.width,
                        source.width,
                        source.height,
                    );
                }
            }
        }
        self.arrays
            .extend(other.arrays.iter().skip(retained_count).cloned());
    }
}

impl CallbackArray {
    fn new(definition: &CallbackDefinition) -> Self {
        Self {
            function: definition.function.clone(),
            width: 0,
            height: 0,
            // C4MCCallbackArray allocates its bitmap on the first enabled
            // pixel; unused and overwritten declarations stay allocation-free.
            bits: Vec::new(),
        }
    }

    fn enable(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if self.bits.is_empty() {
            self.width = width;
            self.height = height;
            let size = self.width.saturating_mul(self.height).max(0) as usize;
            self.bits.resize(size.div_ceil(8), 0);
        }
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let index = (x + y * self.width) as usize;
        self.bits[index / 8] |= 1 << (index % 8);
    }

    fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }
}

/// The parsed and evaluated `C4MapCreatorS2` node tree retained by
/// `KeepMapCreator`. Runtime `DrawMap` can clone this tree to resolve the
/// scenario's named overlay templates without reparsing ranges or drawing
/// new synced random values (C4Landscape.cpp:2650-2658).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MapCreatorS2State {
    tree: Tree,
    /// `C4MapCreatorS2::DefaultMap` is evaluated only when the creator is
    /// constructed. Section overloads reuse the live creator, so maps parsed
    /// by a later `Landscape.txt` inherit these original dimensions instead
    /// of evaluating the new section's `MapWdt`/`MapHgt` again.
    #[serde(default = "default_retained_map")]
    default_map: Overlay,
    #[serde(skip, default)]
    callbacks: PostInitMapCallbacks,
    /// C4Landscape's fixed RNG position immediately before RenderTo. Initial
    /// scenario loading parses resources before the live script host exists,
    /// then replays only RenderTo after script linking from this exact point.
    #[serde(skip, default)]
    pre_render_rng: Option<LcgRng>,
    /// Rust-authority validation is restricted to the exact shipped
    /// HarpoonRace/Sky Race operator program. A map name alone is not an
    /// identity: third-party content may legitimately reuse `SkyParcour`.
    #[serde(default, skip_serializing_if = "bool_is_false")]
    skyparcour_water_exposure_guard: bool,
}

// Callback bitmaps are transient PostInitMap work, deliberately omitted from
// saves. Creator identity/equality follows the persisted S2 tree, its
// construction-time default map, and the behavior-bearing validation guard.
impl PartialEq for MapCreatorS2State {
    fn eq(&self, other: &Self) -> bool {
        self.tree == other.tree
            && self.default_map == other.default_map
            && self.skyparcour_water_exposure_guard == other.skyparcour_water_exposure_guard
    }
}

impl Eq for MapCreatorS2State {}

impl MapCreatorS2State {
    #[cfg(test)]
    fn node_count(&self) -> usize {
        self.tree.nodes.len()
    }

    pub(crate) fn has_skyparcour_water_exposure_guard(&self) -> bool {
        self.skyparcour_water_exposure_guard
    }

    pub(crate) fn set_callback_map_zoom(&mut self, map_zoom: i32) {
        self.callbacks.set_map_zoom(map_zoom);
    }

    pub(crate) fn pre_render_rng(&self) -> Option<LcgRng> {
        self.pre_render_rng.clone()
    }

    pub(crate) fn callbacks(&self) -> &PostInitMapCallbacks {
        &self.callbacks
    }

    pub(crate) fn append_from(&mut self, other: &Self) {
        let callback_offset = self.tree.callbacks.len();
        self.tree
            .callbacks
            .extend(other.tree.callbacks.iter().cloned());
        let node_offset = self.tree.nodes.len().saturating_sub(1);
        let remap_node = |id: NodeId| if id == 0 { 0 } else { id + node_offset };
        self.tree.nodes[0]
            .children
            .extend(other.tree.nodes[0].children.iter().copied().map(remap_node));
        for source in other.tree.nodes.iter().skip(1) {
            let mut node = source.clone();
            node.owner = node.owner.map(remap_node);
            node.children = node.children.into_iter().map(remap_node).collect();
            if let NodeKind::Overlay(overlay) = &mut node.kind {
                overlay.eval_callback = overlay
                    .eval_callback
                    .map(|callback| callback + callback_offset);
                overlay.draw_callback = overlay
                    .draw_callback
                    .map(|callback| callback + callback_offset);
            }
            self.tree.nodes.push(node);
        }
        self.callbacks.append_from(&other.callbacks);
        self.skyparcour_water_exposure_guard |= other.skyparcour_water_exposure_guard;
    }

    pub(crate) fn callback_state(&self) -> PostInitMapCallbacks {
        self.callbacks.clone()
    }

    pub(crate) fn remap_material_colors(&mut self, remap: &[u8; 128]) {
        for node in &mut self.tree.nodes {
            let NodeKind::Overlay(overlay) = &mut node.kind else {
                continue;
            };
            let ift = overlay.mat_clr & 0x80;
            let slot = usize::from(overlay.mat_clr & 0x7f);
            if slot != 0 {
                overlay.mat_clr = ift | remap[slot];
            }
        }
    }
}

pub(crate) struct S2MapCreation {
    pub(crate) bitmap: Option<clonk_resources::bitmap::IndexedBitmap>,
    pub(crate) creator: MapCreatorS2State,
    pub(crate) callbacks: PostInitMapCallbacks,
}

const SKYPARCOUR_WATER_EXPOSURE_CANONICAL_LEN: usize = 1_125;
const SKYPARCOUR_WATER_EXPOSURE_CANONICAL_FNV1A64: u64 = 0x6abc_3e93_6ed2_fcda;

fn bool_is_false(value: &bool) -> bool {
    !*value
}

fn source_has_skyparcour_water_exposure_bug(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut canonical_len = 0_usize;
    let mut index = 0_usize;
    let mut pending_separator = false;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while bytes
                .get(index)
                .is_some_and(|byte| !matches!(*byte, b'\r' | b'\n'))
            {
                index += 1;
            }
            pending_separator = true;
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index < bytes.len()
                && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
            {
                index += 1;
            }
            index = index.saturating_add(2).min(bytes.len());
            pending_separator = true;
            continue;
        }
        let byte = bytes[index];
        index += 1;
        if byte.is_ascii_whitespace() {
            pending_separator = true;
            continue;
        }
        if pending_separator && canonical_len != 0 {
            hash ^= u64::from(b' ');
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            canonical_len += 1;
        }
        pending_separator = false;
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        canonical_len += 1;
    }
    canonical_len == SKYPARCOUR_WATER_EXPOSURE_CANONICAL_LEN
        && hash == SKYPARCOUR_WATER_EXPOSURE_CANONICAL_FNV1A64
}

#[derive(Default)]
struct PixelRenderTrace {
    eval_callbacks: Vec<CallbackId>,
    rendered_overlay: Option<NodeId>,
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
            callbacks: Vec::new(),
        }
    }

    /// C4MapCreatorS2's copy constructor clones the evaluated tree, keeping
    /// names only on global children; nested node names reset in
    /// C4MCNode(owner, template, true) because their owner is not MCN_Node
    /// (C4MapCreatorS2.cpp:137-150,701-714).
    fn clone_creator(&self) -> Self {
        let mut cloned = self.clone();
        for node in cloned.nodes.iter_mut().skip(1) {
            if node.owner != Some(0) {
                node.name.clear();
            }
        }
        cloned
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

    fn add_callback(&mut self, function: String) -> CallbackId {
        let id = self.callbacks.len();
        self.callbacks.push(CallbackDefinition { function });
        id
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
                        let texture =
                            (!overlay.texture.is_empty()).then_some(overlay.texture.as_str());
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
    fn check_mask(
        &self,
        id: NodeId,
        ix: i32,
        iy: i32,
        rng: &mut LcgRng,
        script_algo: &mut ScriptAlgoCall<'_>,
    ) -> bool {
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
                        dx += (((dx / 7 + itofix(seed2) / overlay.zoom_x + dy) / j + d) * rad2grad)
                            .sin_deg()
                            * j
                            / 2;
                        dy += (((dy / 7 + itofix(seed2) / overlay.zoom_y + dx) / j - d) * rad2grad)
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
        self.run_algorithm(id, ix, iy, rng, script_algo) ^ overlay.invert
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
        mut trace: Option<&mut PixelRenderTrace>,
        rng: &mut LcgRng,
        script_algo: &mut ScriptAlgoCall<'_>,
    ) -> bool {
        let overlay = self.overlay(id).expect("render_pix on overlay");
        let set_this = self.check_mask(id, ix, iy, rng, script_algo);
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
                if let Some(trace) = trace.as_deref_mut() {
                    trace.rendered_overlay = Some(id);
                }
            }
            let mut last_set_c = false;
            let mut child_op = Op::None;
            for &child in &self.nodes[id].children {
                if let Some(child_overlay) = self.overlay(child) {
                    last_set_c = self.render_pix(
                        child,
                        ix,
                        iy,
                        pix,
                        child_op,
                        last_set_c,
                        draw,
                        trace.as_deref_mut(),
                        rng,
                        script_algo,
                    );
                    if overlay.group && child_overlay.op == Op::None {
                        do_set |= last_set_c;
                    }
                    child_op = child_overlay.op;
                }
            }
            if do_set && draw {
                if let (Some(callback), Some(trace)) = (overlay.eval_callback, trace) {
                    trace.eval_callbacks.push(callback);
                }
            }
        }
        do_set
    }

    /// C4MCOverlay::PeekPix (src/C4MapCreatorS2.cpp:555-571).
    fn peek_pix(
        &self,
        start: NodeId,
        ix: i32,
        iy: i32,
        rng: &mut LcgRng,
        script_algo: &mut ScriptAlgoCall<'_>,
    ) -> bool {
        let mut current = start;
        let mut last_set = false;
        let mut last_op = Op::None;
        let mut crap = 0u8;
        loop {
            last_set = self.render_pix(
                current,
                ix,
                iy,
                &mut crap,
                last_op,
                last_set,
                false,
                None,
                rng,
                script_algo,
            );
            let overlay = self.overlay(current).expect("peek chain overlay");
            last_op = overlay.op;
            if overlay.op == Op::None {
                break;
            }
            // must be another overlay, since there's an operator
            match self
                .next_sibling(current)
                .filter(|&next| self.overlay(next).is_some())
            {
                Some(next) => current = next,
                None => break,
            }
        }
        last_set
    }

    /// The algorithm dispatch (src/C4MapCreatorS2.cpp:1351-1564).
    fn run_algorithm(
        &self,
        id: NodeId,
        ix: i32,
        iy: i32,
        rng: &mut LcgRng,
        script_algo: &mut ScriptAlgoCall<'_>,
    ) -> bool {
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
            Algo::Border => self.algo_border(id, ix, iy, rng, script_algo),
            Algo::Mandel => {
                let mut iter = overlay.alpha.evaluate(SIZE_RES);
                if iter == 0 {
                    iter = 1000;
                }
                // C++ converts a negative alpha to u32, yielding billions of
                // iterations. Keep a finite ten-step safety policy instead of
                // making in-set pixels effectively hang.
                let iter = iter.max(10) as u32;
                // Unlike Gradient below, these are floating divisions. Rust
                // f64 preserves C++'s zero-dimension inf/NaN behavior and
                // ordinary signed division for negative dimensions.
                let c = (f64::from(ix) / f64::from(z) / f64::from(overlay.wdt)
                    - 0.5 * (f64::from(overlay.zoom_x) / f64::from(z)))
                    * 4.0;
                let ci = (f64::from(iy) / f64::from(z) / f64::from(overlay.hgt)
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
                // C++ performs integer division by zero here. Retain Rust's
                // denominator-one fallback only for that terminal input;
                // nonzero negative widths keep their signed C++ arithmetic.
                let wdt = if overlay.wdt == 0 { 1 } else { overlay.wdt };
                // (abs((iX ^ (iY*3)) * 2531011L) % 214013L) % z — the
                // multiply promotes to 64-bit long on LP64 (no overflow).
                let v = i64::from(ix ^ iy.wrapping_mul(3)) * 2531011;
                (v.abs() % 214013) % i64::from(z) > i64::from(ix / wdt)
            }
            // GetSFunc is deliberately repeated for every evaluation. Border
            // peeks and non-drawing operator operands therefore make the same
            // extra synchronous calls as C++ rather than caching per pixel.
            Algo::Script => script_algo(
                rng,
                &format!("ScriptAlgo{}", self.nodes[id].name),
                [
                    ix,
                    iy,
                    overlay.alpha.evaluate(SIZE_RES),
                    overlay.beta.evaluate(SIZE_RES),
                ],
            ),
            Algo::RndAll => modulo(s, 100) < overlay.alpha.evaluate(SIZE_RES),
            Algo::Poly => self.algo_polygon(id, ix, iy),
        }
    }

    /// AlgoBorder (src/C4MapCreatorS2.cpp:1408-1420) with PreparePeek
    /// (src/C4MapCreatorS2.cpp:1327-1343).
    fn algo_border(
        &self,
        id: NodeId,
        ix: i32,
        iy: i32,
        rng: &mut LcgRng,
        script_algo: &mut ScriptAlgoCall<'_>,
    ) -> bool {
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
            if top_overlay.in_bounds(x, iy) && !self.peek_pix(chain, x, iy, rng, script_algo) {
                return true;
            }
        }
        for y in (iy - lb)..=(iy + lb) {
            if top_overlay.in_bounds(ix, y) && !self.peek_pix(chain, ix, y, rng, script_algo) {
                return true;
            }
        }
        false
    }

    /// AlgoPolygon's backward `u` scan and `pStartChild` selection
    /// (src/C4MapCreatorS2.cpp:1487-1497).
    fn algo_polygon_start_state(&self, id: NodeId, iy: i32) -> Option<(usize, i32, i32, i32)> {
        let children = &self.nodes[id].children;
        if children.is_empty() {
            return None;
        }

        // C++ scans ChildL backwards only while pChild->Prev exists. Child0
        // is therefore never examined here, even when it is a point. The
        // cursor still ends on Child0, so pStartChild is its Next sibling.
        let (mut ux, mut uy, mut lx) = (0, 0, 0);
        let mut scan = children.len() - 1;
        while scan > 0 {
            if let NodeKind::Point(point) = &self.nodes[children[scan]].kind {
                ux = point.x * 100;
                lx = ux;
                uy = point.y * 100;
                if uy != iy {
                    break;
                }
            }
            scan -= 1;
        }
        let start = (scan + 1) % children.len();
        Some((start, ux, uy, lx))
    }

    /// AlgoPolygon (src/C4MapCreatorS2.cpp:1474-1564): even-odd ray cast
    /// over the overlay's child points (coordinates scaled by 100).
    fn algo_polygon(&self, id: NodeId, ix: i32, iy: i32) -> bool {
        let children = &self.nodes[id].children;
        let Some((start, mut ux, mut uy, mut lx)) = self.algo_polygon_start_state(id, iy) else {
            return false;
        };
        let mut count = 0;
        let mut ignore = false;
        for offset in 0..children.len() {
            let child = children[(start + offset) % children.len()];
            let NodeKind::Point(point) = &self.nodes[child].kind else {
                continue;
            };
            let (cx, cy) = (point.x * 100, point.y * 100);
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
        // C++ evaluates `% 0` for alpha=-2. Keep malformed maps nonfatal and
        // contribute a false mask instead of reproducing the terminal fault.
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
    script_functions: &'b HashSet<String>,
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
                                // The native operator type check below this
                                // branch observes PS_KEYWD1, not PS_GOTOP.
                                // Preserve that precedence/state bug: a point
                                // is accepted after an overlay operator.
                            } else if name == "map" {
                                if !global_scope {
                                    return Err("can't declare map in local scope".into());
                                }
                                let mut map = self.default_map.clone();
                                map.op = Op::None;
                                let id =
                                    self.tree
                                        .add(to_node, String::new(), NodeKind::Overlay(map));
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
                let percent_par = |value: i32| {
                    IntBool::new(value, matches!(val_type, ValType::Percent | ValType::Int))
                };
                let pixels_par = |value: i32| IntBool::new(value, val_type == ValType::Percent);
                // Field validation that needs &mut self.classifier happens
                // before re-borrowing the overlay.
                enum Pending {
                    Material(String),
                    Texture(String),
                    Algo(Algo),
                    Callback(String),
                }
                let pending = match field {
                    "mat" => {
                        let name = str_par()?.to_string();
                        // MatMap->Get validity (src/C4MapCreatorS2.cpp:341-343).
                        if self.classifier.material(&name).is_none() {
                            return Err(format!("material '{name}' not found"));
                        }
                        Some(Pending::Material(name))
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
                        let name = str_par()?.to_string();
                        if !self.script_functions.contains(&name) {
                            return Err(format!("script func '{name}' not found"));
                        }
                        Some(Pending::Callback(name))
                    }
                    _ => None,
                };
                let callback = match &pending {
                    Some(Pending::Callback(name)) => Some(self.tree.add_callback(name.clone())),
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
                    "evalFn" => overlay.eval_callback = callback,
                    "drawFn" => overlay.draw_callback = callback,
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
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    create_s2_map_with_state(
        source,
        classifier,
        map_width,
        map_height,
        map_player_extend,
        player_count,
        rng,
    )
    .bitmap
}

/// The state-bearing form of [`create_s2_map`]. Unlike the compatibility
/// wrapper, this returns the evaluated creator tree even when the source has
/// no renderable map node; C++ still retains that creator when
/// `KeepMapCreator=1` (C4Landscape.cpp:537-556, 606-614).
pub(crate) fn create_s2_map_with_state(
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
) -> S2MapCreation {
    create_s2_map_with_state_and_functions(
        source,
        classifier,
        map_width,
        map_height,
        map_player_extend,
        player_count,
        rng,
        &HashSet::new(),
    )
}

/// Initial-landscape form with the already-linked scenario host's named
/// script functions. C4MCV_ScriptFunc rejects a missing name during parsing,
/// but rendering merely records pixels for the later PostInitMap phase.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_s2_map_with_state_and_functions(
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
) -> S2MapCreation {
    let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
    create_s2_map_with_state_and_functions_with_script_algo(
        source,
        classifier,
        map_width,
        map_height,
        map_player_extend,
        player_count,
        rng,
        script_functions,
        &mut missing_script_algo,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_s2_map_with_state_and_functions_with_script_algo(
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> S2MapCreation {
    // C4MCMap::Default (src/C4MapCreatorS2.cpp:633-644) runs at creator
    // construction: MapWdt/MapHgt evaluate through the synced rng.
    let (wdt, hgt) = evaluate_map_size(map_width, map_height, map_player_extend, player_count, rng);
    let mut creation = parse_and_render_s2_map_with_callbacks_and_script_algo(
        Tree::new(),
        source,
        classifier,
        default_map_for_size(wdt, hgt),
        rng,
        false,
        script_functions,
        None,
        script_algo,
    );
    creation.creator.skyparcour_water_exposure_guard =
        source_has_skyparcour_water_exposure_bug(source);
    creation
}

/// Unified `CreateMapS2` entry point for scenario-section activation. With a
/// retained creator, `ReadFile` mutates that creator and keeps its evaluated
/// defaults/callback arrays. Without one, construction evaluates the active
/// section's MapWdt/MapHgt before parsing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_s2_map_for_section_with_state_and_functions(
    retained: Option<MapCreatorS2State>,
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
) -> S2MapCreation {
    let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
    create_s2_map_for_section_with_state_and_functions_with_script_algo(
        retained,
        source,
        classifier,
        map_width,
        map_height,
        map_player_extend,
        player_count,
        rng,
        script_functions,
        &mut missing_script_algo,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_s2_map_for_section_with_state_and_functions_with_script_algo(
    retained: Option<MapCreatorS2State>,
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_player_extend: bool,
    player_count: i32,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> S2MapCreation {
    let Some(retained) = retained else {
        return create_s2_map_with_state_and_functions_with_script_algo(
            source,
            classifier,
            map_width,
            map_height,
            map_player_extend,
            player_count,
            rng,
            script_functions,
            script_algo,
        );
    };
    let MapCreatorS2State {
        tree,
        default_map,
        callbacks,
        pre_render_rng: _,
        skyparcour_water_exposure_guard,
    } = retained;
    let mut creation = parse_and_render_s2_map_with_callbacks_and_script_algo(
        tree,
        source,
        classifier,
        default_map,
        rng,
        false,
        script_functions,
        Some(callbacks),
        script_algo,
    );
    creation.creator.skyparcour_water_exposure_guard =
        skyparcour_water_exposure_guard || source_has_skyparcour_water_exposure_bug(source);
    creation
}

/// Compatibility name for the retained-creator branch of
/// [`create_s2_map_for_section_with_state_and_functions`].
pub(crate) fn extend_s2_map_with_state_and_functions(
    creator: MapCreatorS2State,
    source: &str,
    classifier: &mut MapPixelClassifier,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
) -> S2MapCreation {
    create_s2_map_for_section_with_state_and_functions(
        Some(creator),
        source,
        classifier,
        LegacyC4SVal::default(),
        LegacyC4SVal::default(),
        false,
        1,
        rng,
        script_functions,
    )
}

/// Section-overload form of `C4Landscape::CreateMapS2`. Native C++ calls
/// `ReadFile` on the existing `pMapCreator`, rather than constructing a new
/// creator, so the appended source can resolve named templates from earlier
/// sections. The creator's construction-time `DefaultMap` and callback masks
/// also remain live. In particular this performs no new MapWdt/MapHgt RNG
/// evaluations.
pub(crate) fn create_s2_map_from_retained_state_and_functions(
    retained: &MapCreatorS2State,
    source: &str,
    classifier: &mut MapPixelClassifier,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
) -> S2MapCreation {
    create_s2_map_for_section_with_state_and_functions(
        Some(retained.clone()),
        source,
        classifier,
        LegacyC4SVal::default(),
        LegacyC4SVal::default(),
        false,
        1,
        rng,
        script_functions,
    )
}

/// Runtime C4MapCreatorS2 construction seam for C4Landscape::DrawMap
/// (C4Landscape.cpp:2636-2668). With KeepMapCreator state, the evaluated
/// tree is cloned so additional script can resolve its named templates;
/// otherwise parsing starts from a fresh root. Overlay callback pointers in
/// the clone still target the original creator's live arrays, so matching
/// runtime pixels are merged back before the temporary creator is discarded.
/// No landscape write or script host is performed here.
pub(crate) fn render_runtime_s2_map(
    retained: Option<&mut MapCreatorS2State>,
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: i32,
    map_height: i32,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
    render_runtime_s2_map_with_script_algo(
        retained,
        source,
        classifier,
        map_width,
        map_height,
        rng,
        script_functions,
        &mut missing_script_algo,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_runtime_s2_map_with_script_algo(
    retained: Option<&mut MapCreatorS2State>,
    source: &str,
    classifier: &mut MapPixelClassifier,
    map_width: i32,
    map_height: i32,
    rng: &mut LcgRng,
    script_functions: &HashSet<String>,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let original_callback_count = retained
        .as_ref()
        .map(|creator| creator.tree.callbacks.len())
        .unwrap_or(0);
    let tree = retained
        .as_ref()
        .map(|creator| creator.tree.clone_creator())
        .unwrap_or_else(Tree::new);
    // FakeLS.MapWdt/MapHgt are C4SVal(requested, 0, requested, requested).
    // C4SVal::Evaluate still calls Random(1) once per axis before BoundBy
    // returns the exact value (C4Scenario.cpp:38-46).
    let _ = rng.random(1);
    let _ = rng.random(1);
    let creation = parse_and_render_s2_map_with_callbacks_and_script_algo(
        tree,
        source,
        classifier,
        default_map_for_size(map_width, map_height),
        rng,
        true,
        script_functions,
        None,
        script_algo,
    );
    if let Some(retained) = retained {
        retained
            .callbacks
            .merge_runtime_clone_prefix_from(&creation.callbacks, original_callback_count);
    }
    creation.bitmap
}

fn default_map_for_size(wdt: i32, hgt: i32) -> Overlay {
    let mut default_map = Overlay::default_template();
    default_map.is_map = true;
    default_map.wdt = wdt;
    default_map.hgt = hgt;
    default_map
}

fn default_retained_map() -> Overlay {
    default_map_for_size(SIZE_RES, SIZE_RES)
}

fn parse_and_render_s2_map(
    tree: Tree,
    source: &str,
    classifier: &mut MapPixelClassifier,
    default_map: Overlay,
    rng: &mut LcgRng,
    runtime_source: bool,
    script_functions: &HashSet<String>,
) -> S2MapCreation {
    let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
    parse_and_render_s2_map_with_callbacks_and_script_algo(
        tree,
        source,
        classifier,
        default_map,
        rng,
        runtime_source,
        script_functions,
        None,
        &mut missing_script_algo,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_and_render_s2_map_with_callbacks(
    tree: Tree,
    source: &str,
    classifier: &mut MapPixelClassifier,
    default_map: Overlay,
    rng: &mut LcgRng,
    runtime_source: bool,
    script_functions: &HashSet<String>,
    retained_callbacks: Option<PostInitMapCallbacks>,
) -> S2MapCreation {
    let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
    parse_and_render_s2_map_with_callbacks_and_script_algo(
        tree,
        source,
        classifier,
        default_map,
        rng,
        runtime_source,
        script_functions,
        retained_callbacks,
        &mut missing_script_algo,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_and_render_s2_map_with_callbacks_and_script_algo(
    tree: Tree,
    source: &str,
    classifier: &mut MapPixelClassifier,
    default_map: Overlay,
    rng: &mut LcgRng,
    runtime_source: bool,
    script_functions: &HashSet<String>,
    retained_callbacks: Option<PostInitMapCallbacks>,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> S2MapCreation {
    let retained_callback_count = tree.callbacks.len();
    let mut parser = Parser {
        tokens: Tokenizer::new(source),
        tree,
        default_map: default_map.clone(),
        classifier,
        rng,
        script_functions,
    };
    if let Err(error) = parser.parse_into(0) {
        // C4MCParserErr::show (src/C4MapCreatorS2.cpp:823-827): log and
        // carry on with whatever parsed.
        if runtime_source {
            tracing::warn!(%error, "runtime map script parse error; rendering the nodes parsed so far");
        } else {
            tracing::warn!(%error, "Landscape.txt parse error; rendering the nodes parsed so far");
        }
    }
    let tree = parser.tree;

    let pre_render_rng = Some(rng.clone());
    let (bitmap, rendered_callbacks) = render_last_map(&tree, rng, script_algo);
    let mut callbacks = retained_callbacks.unwrap_or_default();
    if callbacks.arrays.is_empty() && retained_callback_count == 0 {
        callbacks = rendered_callbacks;
    } else {
        callbacks.merge_live_render_from(&rendered_callbacks, retained_callback_count);
    }
    S2MapCreation {
        bitmap,
        creator: MapCreatorS2State {
            tree,
            default_map,
            callbacks: callbacks.clone(),
            pre_render_rng,
            skyparcour_water_exposure_guard: false,
        },
        callbacks,
    }
}

fn last_map(tree: &Tree) -> Option<NodeId> {
    // GetMap(nullptr): the last map entry (src/C4MapCreatorS2.cpp:786-792).
    tree.nodes[0]
        .children
        .iter()
        .rev()
        .find(|&&child| tree.overlay(child).is_some_and(|overlay| overlay.is_map))
        .copied()
}

fn render_last_map(
    tree: &Tree,
    rng: &mut LcgRng,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> (
    Option<clonk_resources::bitmap::IndexedBitmap>,
    PostInitMapCallbacks,
) {
    let Some(map) = last_map(tree) else {
        return (None, PostInitMapCallbacks::default());
    };
    render_map_with_callbacks(tree, map, rng, script_algo).map_or_else(
        || (None, PostInitMapCallbacks::default()),
        |(bitmap, callbacks)| (Some(bitmap), callbacks),
    )
}

fn render_map(
    tree: &Tree,
    map: NodeId,
    rng: &mut LcgRng,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let map_overlay = tree.overlay(map)?;
    let (wdt, hgt) = (map_overlay.wdt, map_overlay.hgt);
    if wdt <= 0 || hgt <= 0 {
        return None;
    }

    let mut bytes = vec![0u8; (wdt * hgt) as usize];
    for iy in 0..hgt {
        for ix in 0..wdt {
            let pix = &mut bytes[(iy * wdt + ix) as usize];
            *pix = 0;
            tree.render_pix(
                map,
                ix,
                iy,
                pix,
                Op::None,
                false,
                true,
                None,
                rng,
                script_algo,
            );
        }
    }
    Some(clonk_resources::bitmap::IndexedBitmap {
        width: wdt as u32,
        height: hgt as u32,
        indices: bytes,
    })
}

fn render_map_with_callbacks(
    tree: &Tree,
    map: NodeId,
    rng: &mut LcgRng,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> Option<(clonk_resources::bitmap::IndexedBitmap, PostInitMapCallbacks)> {
    if tree.callbacks.is_empty() {
        return render_map(tree, map, rng, script_algo)
            .map(|bitmap| (bitmap, PostInitMapCallbacks::default()));
    }
    let mut callbacks = PostInitMapCallbacks {
        arrays: tree.callbacks.iter().map(CallbackArray::new).collect(),
        map_zoom: 0,
    };
    let bitmap = render_map_recording_callbacks(tree, map, &mut callbacks, rng, script_algo)?;
    Some((bitmap, callbacks))
}

fn render_map_recording_callbacks(
    tree: &Tree,
    map: NodeId,
    callbacks: &mut PostInitMapCallbacks,
    rng: &mut LcgRng,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let map_overlay = tree.overlay(map)?;
    let (wdt, hgt) = (map_overlay.wdt, map_overlay.hgt);
    if wdt <= 0 || hgt <= 0 {
        return None;
    }

    // C4MCMap::RenderTo (src/C4MapCreatorS2.cpp:646-674).
    let mut bytes = vec![0u8; (wdt * hgt) as usize];
    let mut trace = PixelRenderTrace::default();
    for iy in 0..hgt {
        for ix in 0..wdt {
            let pix = &mut bytes[(iy * wdt + ix) as usize];
            *pix = 0;
            trace.eval_callbacks.clear();
            trace.rendered_overlay = None;
            tree.render_pix(
                map,
                ix,
                iy,
                pix,
                Op::None,
                false,
                true,
                Some(&mut trace),
                rng,
                script_algo,
            );
            for &callback in &trace.eval_callbacks {
                if let Some(array) = callbacks.arrays.get_mut(callback) {
                    array.enable(ix, iy, wdt, hgt);
                }
            }
            if let Some(callback) = trace
                .rendered_overlay
                .and_then(|overlay| tree.overlay(overlay))
                .and_then(|overlay| overlay.draw_callback)
            {
                if let Some(array) = callbacks.arrays.get_mut(callback) {
                    array.enable(ix, iy, wdt, hgt);
                }
            }
        }
    }
    Some(clonk_resources::bitmap::IndexedBitmap {
        width: wdt as u32,
        height: hgt as u32,
        indices: bytes,
    })
}

/// C4Landscape::DrawDefMap/C4MCMap::SetSize (C4Landscape.cpp:2672-2696;
/// C4MapCreatorS2.cpp:676-681): resolve a map in the retained scenario
/// creator, resize it, re-evaluate the complete tree through the live synced
/// RNG, then render that exact map. Unlike DrawMap, this mutates the retained
/// creator and performs no FakeLS MapWdt/MapHgt draws.
pub(crate) fn render_named_s2_map(
    creator: &mut MapCreatorS2State,
    name: &str,
    classifier: &mut MapPixelClassifier,
    map_width: i32,
    map_height: i32,
    rng: &mut LcgRng,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
    render_named_s2_map_with_script_algo(
        creator,
        name,
        classifier,
        map_width,
        map_height,
        rng,
        &mut missing_script_algo,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_named_s2_map_with_script_algo(
    creator: &mut MapCreatorS2State,
    name: &str,
    classifier: &mut MapPixelClassifier,
    map_width: i32,
    map_height: i32,
    rng: &mut LcgRng,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let map = if name.is_empty() {
        last_map(&creator.tree)
    } else {
        creator.tree.node_by_name(0, name).filter(|&node| {
            creator
                .tree
                .overlay(node)
                .is_some_and(|overlay| overlay.is_map)
        })
    }?;

    let map_overlay = creator.tree.overlay_mut(map)?;
    map_overlay.wdt = map_width;
    map_overlay.hgt = map_height;
    creator.tree.re_evaluate(0, classifier, rng);
    creator.pre_render_rng = Some(rng.clone());
    render_map_recording_callbacks(&creator.tree, map, &mut creator.callbacks, rng, script_algo)
}

/// Re-run only C4MCMap::RenderTo after the actual scenario host has linked.
/// Parsing, material lookup and overlay seed evaluation already occurred at
/// resource-load time; `pre_render_rng` restores the exact fixed ledger seam.
pub(crate) fn rerender_last_s2_map_with_script_algo(
    creator: &mut MapCreatorS2State,
    rng: &mut LcgRng,
    script_algo: &mut ScriptAlgoCall<'_>,
) -> Option<clonk_resources::bitmap::IndexedBitmap> {
    let (bitmap, callbacks) = render_last_map(&creator.tree, rng, script_algo);
    creator.callbacks = callbacks;
    bitmap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_exposure_guard_matches_only_the_shipped_skyparcour_program() {
        let shipped_skyparcour =
            include_str!("../../../content/Races.c4f/Skyrace.c4s/Landscape.txt");

        assert!(source_has_skyparcour_water_exposure_bug(shipped_skyparcour));
        assert!(source_has_skyparcour_water_exposure_bug(
            &shipped_skyparcour.replacen("/* Skylands parcour */", "/* copied scenario */", 1)
        ));
        assert!(
            !source_has_skyparcour_water_exposure_bug(&shipped_skyparcour.replacen(
                "SkyParcour",
                "Sky Parcour",
                1
            )),
            "whitespace that changes token boundaries must change the program identity"
        );
        assert!(
            !source_has_skyparcour_water_exposure_bug(
                "map SkyParcour { overlay { mat=Water; }; };"
            ),
            "a third-party map name must not opt into the shipped-content repair"
        );

        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (width, height) = params();
        let creator = create_s2_map_with_state(
            shipped_skyparcour,
            &mut classifier,
            width,
            height,
            false,
            1,
            &mut rng,
        )
        .creator;
        assert!(creator.has_skyparcour_water_exposure_guard());
        let encoded = serde_json::to_string(&creator).expect("guarded creator serializes");
        let restored: MapCreatorS2State =
            serde_json::from_str(&encoded).expect("guarded creator restores");
        assert_eq!(restored, creator);
        assert!(restored.has_skyparcour_water_exposure_guard());
    }

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
        let library = clonk_resources::MaterialLibrary::parse(
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

    fn polygon_tree(points: &[(i32, i32)]) -> (Tree, NodeId) {
        let mut tree = Tree::new();
        let mut overlay = Overlay::default_template();
        overlay.algorithm = Algo::Poly;
        let polygon = tree.add(0, String::new(), NodeKind::Overlay(overlay));
        for &(x, y) in points {
            tree.add(
                polygon,
                String::new(),
                NodeKind::Point(Point {
                    x,
                    y,
                    ..Point::default()
                }),
            );
        }
        (tree, polygon)
    }

    fn bitmap_indices(rows: &[&str], filled: u8) -> Vec<u8> {
        rows.iter()
            .flat_map(|row| row.bytes())
            .map(|byte| if byte == b'#' { filled } else { 0 })
            .collect()
    }

    fn algorithm_tree(
        algorithm: Algo,
        alpha: i32,
        seed: i32,
        wdt: i32,
        hgt: i32,
        zoom_x: i32,
        zoom_y: i32,
    ) -> (Tree, NodeId) {
        let mut tree = Tree::new();
        let mut overlay = Overlay::default_template();
        overlay.algorithm = algorithm;
        overlay.alpha = IntBool::new(alpha, false);
        overlay.seed = seed;
        overlay.wdt = wdt;
        overlay.hgt = hgt;
        overlay.zoom_x = zoom_x;
        overlay.zoom_y = zoom_y;
        let node = tree.add(0, String::new(), NodeKind::Overlay(overlay));
        (tree, node)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_algorithm_case(
        algorithm: Algo,
        alpha: i32,
        seed: i32,
        wdt: i32,
        hgt: i32,
        zoom_x: i32,
        zoom_y: i32,
        ix: i32,
        iy: i32,
    ) -> bool {
        let (tree, node) = algorithm_tree(algorithm, alpha, seed, wdt, hgt, zoom_x, zoom_y);
        let mut rng = LcgRng::seed_from_u64(0);
        let mut missing_script_algo = |_: &mut LcgRng, _: &str, _: [i32; 4]| false;
        tree.run_algorithm(node, ix, iy, &mut rng, &mut missing_script_algo)
    }

    fn cpp_mandel_formula(
        alpha: i32,
        wdt: i32,
        hgt: i32,
        zoom_x: i32,
        zoom_y: i32,
        ix: i32,
        iy: i32,
    ) -> bool {
        assert!(
            alpha >= 0,
            "negative-alpha C++ oracle is intentionally excluded"
        );
        let mut iterations = if alpha == 0 { 1000 } else { alpha as u32 };
        iterations = iterations.max(10);
        let c = (f64::from(ix) / f64::from(ZOOM_RES) / f64::from(wdt)
            - 0.5 * (f64::from(zoom_x) / f64::from(ZOOM_RES)))
            * 4.0;
        let ci = (f64::from(iy) / f64::from(ZOOM_RES) / f64::from(hgt)
            - 0.5 * (f64::from(zoom_y) / f64::from(ZOOM_RES)))
            * 4.0;
        let (mut zr, mut zi) = (c, ci);
        for _ in 0..iterations {
            let xz = zr * zr - zi * zi;
            zi = 2.0 * zr * zi + ci;
            zr = xz + c;
            if zr * zr + zi * zi > 4.0 {
                return false;
            }
        }
        true
    }

    fn cpp_gradient_formula(wdt: i32, ix: i32, iy: i32) -> bool {
        let value = i64::from(ix ^ (iy * 3)) * 2_531_011;
        (value.abs() % 214_013) % i64::from(ZOOM_RES) > i64::from(ix / wdt)
    }

    fn cpp_random_formula(seed: i32, alpha: i32, ix: i32, iy: i32) -> bool {
        let mixed = (seed ^ (ix << 2) ^ (iy << 5)) ^ ((seed >> 16) + 1 + ix + (iy << 2));
        (mixed / 17) % (alpha + 2) == 0
    }

    #[test]
    fn nondegenerate_mandel_gradient_random_match_cpp_formula_sweep() {
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        for alpha in [0, 1, 9, 10, 13, 100] {
            for (wdt, hgt) in [(1, 1), (3, 7), (100, 100), (-3, 7), (3, -7)] {
                for (zoom_x, zoom_y) in [(50, 75), (100, 100), (175, 60)] {
                    for (ix, iy) in [(0, 0), (100, 250), (3500, 3300), (4700, 2500), (-200, 700)] {
                        actual.push(u8::from(run_algorithm_case(
                            Algo::Mandel,
                            alpha,
                            0,
                            wdt,
                            hgt,
                            zoom_x,
                            zoom_y,
                            ix,
                            iy,
                        )));
                        expected.push(u8::from(cpp_mandel_formula(
                            alpha, wdt, hgt, zoom_x, zoom_y, ix, iy,
                        )));
                    }
                }
            }
        }
        assert_eq!(actual, expected, "Mandel mask bytes match the C++ formula");

        actual.clear();
        expected.clear();
        for wdt in [-9, -2, 1, 2, 9] {
            for (ix, iy) in [(-100, -50), (-1, 0), (0, 0), (1, 3), (100, 50)] {
                actual.push(u8::from(run_algorithm_case(
                    Algo::Gradient,
                    0,
                    0,
                    wdt,
                    1,
                    100,
                    100,
                    ix,
                    iy,
                )));
                expected.push(u8::from(cpp_gradient_formula(wdt, ix, iy)));
            }
        }
        assert_eq!(
            actual, expected,
            "Gradient mask bytes match the C++ formula"
        );

        actual.clear();
        expected.clear();
        for alpha in [-5, -3, -1, 0, 1, 10] {
            for seed in [0, 1, 0x0123_4567] {
                for (ix, iy) in [(0, 0), (1, 2), (17, 9), (100, 50)] {
                    actual.push(u8::from(run_algorithm_case(
                        Algo::Random,
                        alpha,
                        seed,
                        1,
                        1,
                        100,
                        100,
                        ix,
                        iy,
                    )));
                    expected.push(u8::from(cpp_random_formula(seed, alpha, ix, iy)));
                }
            }
        }
        assert_eq!(actual, expected, "Random mask bytes match the C++ formula");
    }

    #[test]
    fn degenerate_algorithm_policies_are_bounded_and_explicit() {
        // c=-0.6, ci=-0.68 stays bounded for ten updates and escapes on
        // update eleven. Never use an in-set point here: a regression to the
        // C++ negative-to-u32 cast would then run billions of iterations.
        assert!(run_algorithm_case(
            Algo::Mandel,
            -1,
            0,
            100,
            100,
            100,
            100,
            3500,
            3300,
        ));
        assert!(!run_algorithm_case(
            Algo::Mandel,
            11,
            0,
            100,
            100,
            100,
            100,
            3500,
            3300,
        ));

        for (wdt, hgt, ix, iy, expected) in
            [(0, 1, 0, 0, true), (0, 1, 1, 0, false), (1, 0, 0, 0, true)]
        {
            let actual = run_algorithm_case(Algo::Mandel, 10, 0, wdt, hgt, 100, 100, ix, iy);
            assert_eq!(
                actual, expected,
                "floating zero dimension keeps C++ inf/NaN behavior"
            );
        }

        let gradient_zero = run_algorithm_case(Algo::Gradient, 0, 0, 0, 1, 100, 100, 5, 3);
        let gradient_one = run_algorithm_case(Algo::Gradient, 0, 0, 1, 1, 100, 100, 5, 3);
        assert!(gradient_zero);
        assert_eq!(gradient_zero, gradient_one);
        assert!(!algo_random_seeded(17, -2, 5, 3));
    }

    #[test]
    fn polygon_start_scan_excludes_child_zero_node_like_cpp() {
        let (triangle, polygon) = polygon_tree(&[(5, 0), (3, 10), (7, 10)]);
        for ix in [0, 100, 200] {
            assert!(
                triangle.algo_polygon(polygon, ix, 1000),
                "the closing edge includes ix={ix} on the horizontal base row"
            );
        }

        let (all_on_row, polygon) = polygon_tree(&[(5, 10), (3, 10), (7, 10)]);
        assert_eq!(
            all_on_row.algo_polygon_start_state(polygon, 1000),
            Some((1, 300, 1000, 300)),
            "scan exhaustion leaves u at child 1 and starts at child 1"
        );

        // The exclusion is by child NODE, not filtered point index. Child 1
        // is a non-point start while u/lx remain the on-row point at child 2.
        let (mut mixed, polygon) = polygon_tree(&[(5, 0)]);
        mixed.add(
            polygon,
            String::new(),
            NodeKind::Overlay(Overlay::default_template()),
        );
        for x in [3, 7] {
            mixed.add(
                polygon,
                String::new(),
                NodeKind::Point(Point {
                    x,
                    y: 10,
                    ..Point::default()
                }),
            );
        }
        assert_eq!(
            mixed.algo_polygon_start_state(polygon, 1000),
            Some((1, 300, 1000, 300))
        );
    }

    #[test]
    fn polygon_triangle_and_quad_match_cpp_full_grid_golden() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let triangle = create_s2_map(
            "map Triangle { seed=1; mat=Earth; tex=Smooth3; sub=0; algo=poly; \
             point { x=5px; y=0px; }; point { x=3px; y=10px; }; \
             point { x=7px; y=10px; }; };",
            &mut classifier,
            LegacyC4SVal::new(10, 0, 10, 10),
            LegacyC4SVal::new(11, 0, 11, 11),
            false,
            1,
            &mut rng,
        )
        .expect("triangle map renders");
        assert_eq!(
            triangle.indices,
            bitmap_indices(
                &[
                    ".....#....",
                    ".....#....",
                    ".....#....",
                    ".....#....",
                    ".....#....",
                    "....###...",
                    "....###...",
                    "....###...",
                    "....###...",
                    "....###...",
                    "########..",
                ],
                1,
            )
        );

        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let quad = create_s2_map(
            "map Quad { seed=1; mat=Earth; tex=Smooth3; sub=0; algo=poly; \
             point { x=0px; y=0px; }; point { x=10px; y=0px; }; \
             point { x=10px; y=10px; }; point { x=0px; y=10px; }; };",
            &mut classifier,
            LegacyC4SVal::new(11, 0, 11, 11),
            LegacyC4SVal::new(11, 0, 11, 11),
            false,
            1,
            &mut rng,
        )
        .expect("quad map renders");
        assert_eq!(quad.indices, vec![1; 11 * 11]);
    }

    #[test]
    fn evaluated_creator_tree_round_trips_without_a_renderable_map() {
        // CreateMapS2 leaves pMapCreator alive even when Render(nullptr)
        // finds no map and the caller falls back to C4MapCreator. With
        // KeepMapCreator, named templates must therefore survive that path
        // too (C4Landscape.cpp:537-556, 606-614).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let (w, h) = params();
        let creation = create_s2_map_with_state(
            "overlay Named { wdt = 50; seed = 7; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        );
        assert!(creation.bitmap.is_none());
        assert!(creation.creator.node_count() > 1);

        let encoded = serde_json::to_string(&creation.creator).expect("creator serializes");
        assert!(!encoded.contains("callbacks"));
        assert!(!encoded.contains("eval_callback"));
        assert!(!encoded.contains("draw_callback"));
        let restored: MapCreatorS2State = serde_json::from_str(&encoded).expect("creator restores");
        assert_eq!(restored, creation.creator);
    }

    #[test]
    fn section_reparse_resolves_retained_templates_and_keeps_creator_defaults() {
        // CreateMapS2 reuses pMapCreator across a section overload. The new
        // file can instantiate a named overlay from the old tree, uses the
        // creator's already-evaluated DefaultMap, and ORs callback pixels
        // into the old callback array (C4Landscape.cpp:531-546).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(23);
        let functions = HashSet::from(["Paint".to_string()]);
        let retained = create_s2_map_with_state_and_functions(
            "overlay Named { mat=Earth; tex=Rough; wdt=50; seed=7; drawFn=Paint; }; \
             map First { seed=9; Named; };",
            &mut classifier,
            LegacyC4SVal::new(8, 0, 8, 8),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
            &functions,
        );
        assert_eq!(
            retained.callbacks.arrays[0]
                .bits
                .iter()
                .map(|byte| byte.count_ones())
                .sum::<u32>(),
            4
        );
        let before = rng.count;

        let appended = create_s2_map_from_retained_state_and_functions(
            &retained.creator,
            "map Second { seed=11; Named { x=50; }; };",
            &mut classifier,
            &mut rng,
            &functions,
        );
        let map = appended.bitmap.as_ref().expect("appended map renders");
        assert_eq!((map.width, map.height), (8, 1));
        assert_eq!(&map.indices[..4], &[0; 4]);
        assert_eq!(&map.indices[4..], &[2 | 0x80; 4]);
        assert_eq!(appended.callbacks.arrays.len(), 1);
        assert_eq!(
            appended.callbacks.arrays[0]
                .bits
                .iter()
                .map(|byte| byte.count_ones())
                .sum::<u32>(),
            8,
            "the retained callback array accumulates both section renders"
        );
        assert_eq!(
            rng.count, before,
            "reusing a creator does not reevaluate DefaultMap or fixed seeds"
        );

        // Render(nullptr) always chooses the last map in the combined tree;
        // a template-only section therefore renders the preceding map.
        let template_only = create_s2_map_from_retained_state_and_functions(
            &appended.creator,
            "overlay Later { seed=13; };",
            &mut classifier,
            &mut rng,
            &functions,
        );
        assert_eq!(template_only.bitmap, appended.bitmap);
    }

    #[test]
    fn runtime_creator_clone_resolves_retained_named_template_without_mutation() {
        // DrawMap copies pMapCreator when KeepMapCreator retained it, swaps
        // in a FakeLS carrying the requested map size, then ReadScript can
        // resolve named overlays from the cloned tree
        // (C4Landscape.cpp:2636-2662; C4MapCreatorS2.cpp:701-714).
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(17);
        let (w, h) = params();
        let mut retained = create_s2_map_with_state(
            "overlay Named { mat = Earth; tex = Rough; wdt = 50; seed = 7; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut rng,
        )
        .creator;
        let original = retained.clone();
        let before = rng.count;

        let map = render_runtime_s2_map(
            Some(&mut retained),
            "map Runtime { seed = 9; Named; };",
            &mut classifier,
            8,
            4,
            &mut rng,
            &HashSet::new(),
        )
        .expect("cloned creator renders retained template");

        assert_eq!(retained, original, "DrawMap works on a creator copy");
        assert_eq!((map.width, map.height), (8, 4));
        let at = |x: u32, y: u32| map.indices[(y * map.width + x) as usize];
        assert_eq!(at(0, 0), 2 | 0x80);
        assert_eq!(at(3, 3), 2 | 0x80, "50% template uses requested width");
        assert_eq!(at(4, 0), 0, "right half remains sky");
        assert_eq!(
            rng.count - before,
            2,
            "FakeLS exact MapWdt/MapHgt still evaluate with two Random(1) draws"
        );
    }

    #[test]
    fn retained_section_append_skips_size_draws_and_resolves_main_template() {
        // A later CreateMapS2 call parses into the retained creator instead
        // of constructing one, so it keeps DefaultMap and performs no
        // MapWdt/MapHgt evaluations (src/C4Landscape.cpp:531-546;
        // src/C4MapCreatorS2.cpp:633-644,741-751).
        let mut classifier = test_classifier();
        let mut main_rng = LcgRng::seed_from_u64(17);
        let creator = create_s2_map_with_state(
            "overlay Shared { mat=Earth; tex=Rough; sub=0; seed=7; }; \
             map Main { seed=9; };",
            &mut classifier,
            LegacyC4SVal::new(2, 0, 2, 2),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut main_rng,
        )
        .creator;
        let main_node_count = creator.node_count();
        let mut section_rng = LcgRng::seed_from_u64(17);
        let section_count_before = section_rng.count;

        let creation = extend_s2_map_with_state_and_functions(
            creator,
            "map Section { seed=11; Shared; };",
            &mut classifier,
            &mut section_rng,
            &HashSet::new(),
        );
        let map = creation.bitmap.expect("retained section map renders");

        assert_eq!((map.width, map.height), (2, 1));
        assert_eq!(map.indices, vec![2, 2]);
        assert_eq!(
            section_rng.count, section_count_before,
            "no constructor-time size draws"
        );
        assert!(creation.creator.node_count() > main_node_count);
    }

    #[test]
    fn retained_creator_resizes_and_renders_the_requested_named_map_in_place() {
        // DrawDefMap resolves Requested rather than the last map (Decoy),
        // then SetSize re-evaluates the whole retained tree. Exactly three
        // seedless overlays consume two draws each; fixed-seed Decoy nodes
        // consume none, and there are no DrawMap FakeLS size draws.
        let mut classifier = test_classifier();
        let mut setup_rng = LcgRng::seed_from_u64(3);
        let (w, h) = params();
        let mut creator = create_s2_map_with_state(
            "overlay Half { mat = Earth; tex = Rough; wdt = 50; }; \
             map Requested { Half; }; \
             map Decoy { seed = 41; overlay { mat = Earth; tex = Rough; seed = 43; }; };",
            &mut classifier,
            w,
            h,
            false,
            1,
            &mut setup_rng,
        )
        .creator;

        let mut rng = LcgRng::seed_from_u64(17);
        let before = rng.count;
        let map = render_named_s2_map(&mut creator, "Requested", &mut classifier, 2, 2, &mut rng)
            .expect("named map renders");

        assert_eq!((map.width, map.height), (2, 2));
        assert_eq!(map.indices, vec![2 | 0x80, 0, 2 | 0x80, 0]);
        assert_eq!(rng.count - before, 6, "full-tree seed ledger");

        let creator_before_missing = creator.clone();
        let rng_before_missing = rng.clone();
        assert!(
            render_named_s2_map(&mut creator, "Missing", &mut classifier, 1, 1, &mut rng,)
                .is_none()
        );
        assert_eq!(creator, creator_before_missing);
        assert_eq!(rng, rng_before_missing);
    }

    #[test]
    fn runtime_creator_fresh_fallback_matches_exact_initial_creation() {
        // Without pMapCreator, DrawMap constructs a fresh C4MapCreatorS2
        // with the same FakeLS and then ReadScript/Render(nullptr)
        // (C4Landscape.cpp:2642-2663). The seam must preserve the existing
        // create_s2_map bitmap and synced RNG order for equivalent exact
        // dimensions.
        let source = "map Runtime { turbulence = 10 - 20; \
                      overlay { mat = Rock; tex = Ridge; wdt = 50; }; };";
        let mut runtime_classifier = test_classifier();
        let mut creation_classifier = test_classifier();
        let mut runtime_rng = LcgRng::seed_from_u64(23);
        let mut creation_rng = runtime_rng.clone();
        let before = runtime_rng.count;

        let runtime = render_runtime_s2_map(
            None,
            source,
            &mut runtime_classifier,
            9,
            5,
            &mut runtime_rng,
            &HashSet::new(),
        )
        .expect("fresh runtime creator renders");
        let initial = create_s2_map(
            source,
            &mut creation_classifier,
            LegacyC4SVal::new(9, 0, 9, 9),
            LegacyC4SVal::new(5, 0, 5, 5),
            false,
            1,
            &mut creation_rng,
        )
        .expect("initial creator renders");

        assert_eq!(runtime, initial);
        assert_eq!(runtime_rng, creation_rng, "RNG ledger remains identical");
        assert_eq!(runtime_rng.count - before, 7);
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
    fn point_keyword_after_operator_is_ignored_and_chain_reaches_next_overlay_like_cpp() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let creation = create_s2_map_with_state(
            "map Test { seed=1; wdt=3px; hgt=1px; \
               overlay A { seed=2; algo=solid; x=0px; wdt=1px; \
                           mat=Rock; tex=Ridge; sub=0; } & \
               point { x=2px; y=0px; }; \
               overlay B { seed=3; algo=solid; x=0px; wdt=3px; \
                           mat=Earth; tex=Rough; sub=0; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(3, 0, 3, 3),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
        );

        assert_eq!(
            creation.bitmap.expect("map renders").indices,
            vec![2, 0, 0],
            "the point is skipped and B paints only the A & B intersection"
        );
        let map = last_map(&creation.creator.tree).expect("map remains parsed");
        let children = &creation.creator.tree.nodes[map].children;
        assert_eq!(children.len(), 3, "A, the point, and B all stay linked");
        assert_eq!(creation.creator.tree.nodes[children[0]].name, "A");
        assert_eq!(
            creation
                .creator
                .tree
                .overlay(children[0])
                .map(|overlay| overlay.op),
            Some(Op::And)
        );
        assert!(matches!(
            creation.creator.tree.nodes[children[1]].kind,
            NodeKind::Point(_)
        ));
        assert_eq!(creation.creator.tree.nodes[children[2]].name, "B");
        assert!(creation.creator.tree.overlay(children[2]).is_some());

        let mut rng = LcgRng::seed_from_u64(1);
        let rejected = create_s2_map_with_state(
            "map Test { seed=1; wdt=1px; hgt=1px; \
               point {} & overlay B { seed=2; mat=Earth; tex=Rough; sub=0; }; \
               overlay C { seed=3; mat=Rock; tex=Ridge; sub=0; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(1, 0, 1, 1),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
        );
        let map = last_map(&rejected.creator.tree).expect("partial map stays linked");
        let children = &rejected.creator.tree.nodes[map].children;
        assert_eq!(children.len(), 1, "a point still cannot carry an operator");
        assert!(matches!(
            rejected.creator.tree.nodes[children[0]].kind,
            NodeKind::Point(_)
        ));
        assert!(rejected.creator.tree.node_by_name(map, "B").is_none());
        assert!(rejected.creator.tree.node_by_name(map, "C").is_none());
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
    fn missing_script_algorithm_is_false_without_truncating_later_overlay() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let map = create_s2_map(
            "map Test { seed=1; wdt=2px; hgt=1px; \
               overlay Missing { algo=script; mat=Rock; tex=Ridge; sub=0; }; \
               overlay { x=1px; y=0px; wdt=1px; hgt=1px; seed=2; \
                         mat=Earth; tex=Rough; sub=0; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(2, 0, 2, 2),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
        )
        .expect("script algorithm does not abort the later overlay");

        assert_eq!(
            map.indices,
            vec![0, 2],
            "missing ScriptAlgoMissing paints nothing and parsing continues"
        );
    }

    #[test]
    fn script_algorithm_calls_existing_named_function_per_pixel_with_cpp_arguments() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let mut calls = Vec::new();
        let mut script_algo = |_: &mut LcgRng, function: &str, args: [i32; 4]| -> bool {
            calls.push((function.to_string(), args));
            true
        };
        let creation = create_s2_map_with_state_and_functions_with_script_algo(
            "map Test { seed=1; wdt=3px; hgt=2px; \
               overlay Probe { seed=2; algo=script; a=17; b=29; \
                               mat=Earth; tex=Rough; sub=0; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(3, 0, 3, 3),
            LegacyC4SVal::new(2, 0, 2, 2),
            false,
            1,
            &mut rng,
            &HashSet::new(),
            &mut script_algo,
        );

        assert_eq!(
            calls,
            [
                ("ScriptAlgoProbe".to_string(), [0, 0, 17, 29]),
                ("ScriptAlgoProbe".to_string(), [100, 0, 17, 29]),
                ("ScriptAlgoProbe".to_string(), [200, 0, 17, 29]),
                ("ScriptAlgoProbe".to_string(), [0, 100, 17, 29]),
                ("ScriptAlgoProbe".to_string(), [100, 100, 17, 29]),
                ("ScriptAlgoProbe".to_string(), [200, 100, 17, 29]),
            ],
            "RenderTo is row-major and passes transformed coordinates plus a/b"
        );
        assert_eq!(
            creation.bitmap.expect("map renders").indices,
            vec![2; 6],
            "truthy ScriptAlgo results paint every reached pixel"
        );
    }

    #[test]
    fn mapgen_mat_sky_without_loaded_material_truncates_parse() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let base = rng.count;
        let creation = create_s2_map_with_state(
            "map M { \
               overlay { mat=Sky; }; \
               overlay { mat=Earth; tex=Rough; sub=0; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(2, 0, 2, 2),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
        );

        assert_eq!(
            rng.count - base,
            2,
            "the incomplete top-level map is never re-evaluated after MatNotFound"
        );
        assert_eq!(
            creation
                .bitmap
                .expect("the incomplete map still renders")
                .indices,
            vec![0, 0],
            "MatNotFound leaves the map subtree unevaluated, so its material colors stay zero"
        );
        let map = last_map(&creation.creator.tree).expect("the incomplete map stays linked");
        assert_eq!(
            creation.creator.tree.nodes[map].children.len(),
            1,
            "the failing overlay stays linked but the later Earth overlay is never parsed"
        );
    }

    #[test]
    fn mapgen_mat_sky_resolves_loaded_literal_material() {
        let library = clonk_resources::MaterialLibrary::parse("[Material]\nName=Sky\nDensity=0\n")
            .expect("Sky material parses");
        let densities = [0; 128];
        let mut names = vec![None; 128];
        names[1] = Some("Sky".to_string());
        let mut textures = vec![None; 128];
        textures[1] = Some("Rough".to_string());
        let mut classifier = MapPixelClassifier::from_slots_with_library(
            densities,
            names,
            textures,
            vec![None; 128],
            library,
            vec!["rough".to_string()],
        );
        let mut rng = LcgRng::seed_from_u64(1);
        let map = create_s2_map(
            "map M { overlay { mat=sKy; tex=Rough; sub=0; }; };",
            &mut classifier,
            LegacyC4SVal::new(1, 0, 1, 1),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
        )
        .expect("a loaded Sky material renders normally");

        assert_eq!(map.indices, vec![1]);
        assert_eq!(classifier.get_index("Sky", Some("Rough"), false), 1);
    }

    #[test]
    fn eval_and_draw_callbacks_capture_cpp_pixel_masks_in_field_order() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let functions = ["EvalA", "DrawA", "EvalB", "DrawB", "EvalMask", "DrawMask"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let creation = create_s2_map_with_state_and_functions(
            "map Test { seed=1; \
               overlay { x=0px; y=0px; wdt=2px; hgt=2px; seed=2; \
                         evalFn=EvalA; drawFn=DrawA; }; \
               overlay { x=1px; y=0px; wdt=2px; hgt=1px; seed=3; \
                         evalFn=EvalB; drawFn=DrawB; }; \
               overlay { wdt=3px; hgt=2px; seed=4; mask=1; \
                         evalFn=EvalMask; drawFn=DrawMask; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(3, 0, 3, 3),
            LegacyC4SVal::new(2, 0, 2, 2),
            false,
            1,
            &mut rng,
            &functions,
        );

        let bitmap = creation.bitmap.expect("callback map still renders");
        assert_eq!((bitmap.width, bitmap.height), (3, 2));
        assert_eq!(
            creation
                .callbacks
                .arrays
                .iter()
                .filter(|array| !array.is_empty())
                .map(|array| (array.function.as_str(), array.bits.as_slice()))
                .collect::<Vec<_>>(),
            [
                ("EvalA", &[0x1b][..]),
                ("DrawA", &[0x19][..]),
                ("EvalB", &[0x06][..]),
                ("DrawB", &[0x06][..]),
                ("EvalMask", &[0x3f][..]),
            ],
            "eval records boolean fulfillment while draw records only the final writer"
        );
    }

    #[test]
    fn operator_suppressed_group_never_arms_eval_or_draw_callbacks() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let functions = [
            "GroupEval",
            "ChildEval",
            "ChildDraw",
            "TailEval",
            "TailDraw",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let creation = create_s2_map_with_state_and_functions(
            "map Test { seed=1; \
               overlay { seed=2; grp=1; evalFn=GroupEval; \
                 overlay { seed=3; evalFn=ChildEval; drawFn=ChildDraw; }; \
               } & overlay { seed=4; evalFn=TailEval; drawFn=TailDraw; }; \
             };",
            &mut classifier,
            LegacyC4SVal::new(3, 0, 3, 3),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
            &functions,
        );

        assert!(creation.bitmap.is_some());
        assert_eq!(
            creation
                .callbacks
                .arrays
                .iter()
                .filter(|array| !array.is_empty())
                .map(|array| (array.function.as_str(), array.bits.as_slice()))
                .collect::<Vec<_>>(),
            [("TailEval", &[0x07][..]), ("TailDraw", &[0x07][..])],
            "the left operand and its children evaluate with fDraw=false"
        );
    }

    #[test]
    fn template_copies_share_the_original_callback_arrays() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let functions = ["OnEval", "OnDraw"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let creation = create_s2_map_with_state_and_functions(
            "overlay Marked { x=0px; y=0px; wdt=1px; hgt=1px; seed=2; \
                              evalFn=OnEval; drawFn=OnDraw; }; \
             map Test { seed=1; Marked; Marked { x=2px; }; };",
            &mut classifier,
            LegacyC4SVal::new(3, 0, 3, 3),
            LegacyC4SVal::new(1, 0, 1, 1),
            false,
            1,
            &mut rng,
            &functions,
        );

        assert!(creation.bitmap.is_some());
        assert_eq!(
            creation
                .callbacks
                .arrays
                .iter()
                .map(|array| (array.function.as_str(), array.bits.as_slice()))
                .collect::<Vec<_>>(),
            [("OnEval", &[0x05][..]), ("OnDraw", &[0x05][..])]
        );
    }

    #[test]
    fn runtime_draw_map_accepts_valid_callback_fields_without_executing_them() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let functions = ["OnDraw"].into_iter().map(str::to_string).collect();
        let map = render_runtime_s2_map(
            None,
            "map Runtime { seed=1; drawFn=OnDraw; mat=Earth; tex=Rough; \
               overlay { x=1px; y=0px; wdt=1px; hgt=1px; seed=2; \
                         mat=Rock; tex=Ridge; sub=0; }; \
             };",
            &mut classifier,
            2,
            1,
            &mut rng,
            &functions,
        )
        .expect("valid runtime callback field does not abort the later overlay");

        assert_eq!(map.indices, vec![2 | 0x80, 3]);
    }

    #[test]
    fn missing_callback_function_stops_parse_before_later_map() {
        let mut classifier = test_classifier();
        let mut rng = LcgRng::seed_from_u64(1);
        let creation = create_s2_map_with_state_and_functions(
            "overlay Bad { evalFn=Missing; }; map Late { seed=1; };",
            &mut classifier,
            LegacyC4SVal::new(3, 0, 3, 3),
            LegacyC4SVal::new(2, 0, 2, 2),
            false,
            1,
            &mut rng,
            &HashSet::new(),
        );

        assert!(creation.bitmap.is_none(), "the later map was never parsed");
        assert!(creation.callbacks.arrays.is_empty());
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
        let map =
            create_s2_map(source, &mut classifier, w, h, false, 1, &mut rng).expect("map renders");
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
