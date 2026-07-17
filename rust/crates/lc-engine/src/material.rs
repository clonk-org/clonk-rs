use lc_resources::{
    material::MaterialReactionDefinition, MaterialDefinition as ResourceMaterialDefinition,
    MaterialLibrary, MutableGroup, MutableGroupError,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::landscape::Landscape;
use crate::rng::LcgRng;

const C4M_SOLID: i32 = 50;
const C4M_LIQUID: i32 = 25;
const SKY_KEY: &str = "sky";
const MATERIAL_EVENT_COUNT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemperatureDirection {
    Downwards,
    Upwards,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemperatureConversionKind {
    Below,
    Above,
}

impl TemperatureDirection {
    fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Downwards),
            1 => Some(Self::Upwards),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemperatureTarget {
    Material(MaterialId),
    MaterialName(String),
}

impl TemperatureTarget {
    fn resolve_with(&mut self, lookup: &HashMap<String, MaterialId>) {
        if let TemperatureTarget::MaterialName(name) = self {
            let material_name = name.split_once('-').map(|(name, _)| name).unwrap_or(name);
            if let Some(id) = lookup.get(material_name) {
                *self = TemperatureTarget::Material(*id);
            }
        }
    }

    fn resolved(&self, lookup: &HashMap<String, MaterialId>) -> TemperatureTarget {
        match self {
            TemperatureTarget::Material(id) => TemperatureTarget::Material(*id),
            TemperatureTarget::MaterialName(name) => {
                let material_name = name.split_once('-').map(|(name, _)| name).unwrap_or(name);
                lookup
                    .get(material_name)
                    .copied()
                    .map(TemperatureTarget::Material)
                    .unwrap_or_else(|| TemperatureTarget::MaterialName(name.clone()))
            }
        }
    }

    pub fn as_material_id(&self) -> Option<MaterialId> {
        match self {
            TemperatureTarget::Material(id) => Some(*id),
            TemperatureTarget::MaterialName(_) => None,
        }
    }

    pub fn is_sky(&self) -> bool {
        matches!(self, TemperatureTarget::MaterialName(name) if name.split_once('-').map(|(name, _)| name).unwrap_or(name).eq_ignore_ascii_case(SKY_KEY))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperatureConversion {
    threshold: i32,
    direction: TemperatureDirection,
    target: TemperatureTarget,
    target_spec: String,
    kind: TemperatureConversionKind,
}

impl TemperatureConversion {
    fn resolve_target(&mut self, lookup: &HashMap<String, MaterialId>) {
        self.target.resolve_with(lookup);
    }

    fn applies(&self, ambient_temperature: i32, direction: TemperatureDirection) -> bool {
        if self.direction != direction {
            return false;
        }
        match self.kind {
            TemperatureConversionKind::Below => ambient_temperature < self.threshold,
            TemperatureConversionKind::Above => ambient_temperature > self.threshold,
        }
    }

    fn target(&self) -> &TemperatureTarget {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperatureConversionOutcome {
    pub target: TemperatureTarget,
    pub target_spec: String,
    pub strength: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialReactionKind {
    None,
    Convert {
        target: Option<MaterialId>,
        depth: Option<i32>,
    },
    Poof,
    Incinerate,
    Corrode {
        corrosive_strength: i32,
        corrode_resistance: i32,
        corrosion_probability: Option<i32>,
    },
    Insert,
    /// mrfScript (C4Material.cpp:800-835): `func` indexes the set's
    /// `script_reactions` name table (kept `Copy` for the flattened table).
    Script {
        func: u16,
    },
}

/// One resolved reaction-table entry — the `C4MaterialReaction` essentials
/// the mrf* functions consult: the dispatch kind plus the
/// `fUserDefined`/`CheckSlide` flags (C4Material.cpp:48-69, 612-625).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialReaction {
    pub kind: MaterialReactionKind,
    /// User reactions route through `mrfUserCheck`; hardcoded defaults do
    /// their arm-specific unconditional checks.
    pub user_defined: bool,
    /// `CheckSlide=` (`fInsertionCheck`, default true): gates the
    /// splash/slide check for user reactions on PXS movement.
    pub insertion_check: bool,
}

impl MaterialReaction {
    fn builtin(kind: MaterialReactionKind) -> Self {
        Self {
            kind,
            user_defined: false,
            insertion_check: true,
        }
    }

    fn masked_user_noop() -> Self {
        Self {
            kind: MaterialReactionKind::None,
            user_defined: true,
            // C++ rejects ExecMask before it reaches CheckSlide.
            insertion_check: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MaterialInteractionEvent {
    PxsPos = 0,
    PxsMove = 1,
    MassMove = 2,
}

impl MaterialInteractionEvent {
    const ALL: [Self; MATERIAL_EVENT_COUNT] = [Self::PxsPos, Self::PxsMove, Self::MassMove];

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    fn mask(self) -> u32 {
        1u32 << (self as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialReactionExecution {
    Unhandled,
    Consumed,
    Converted(MaterialId),
}

impl MaterialReactionExecution {
    pub fn consumes_material(self) -> bool {
        !matches!(self, MaterialReactionExecution::Unhandled)
    }
}

fn normalize_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn parse_temperature_conversion(
    definition: &ResourceMaterialDefinition,
    threshold_key: &str,
    direction_key: &str,
    target_key: &str,
    kind: TemperatureConversionKind,
) -> Option<TemperatureConversion> {
    let threshold = definition.int(threshold_key).unwrap_or(0);
    let direction_raw = definition.int(direction_key).unwrap_or(0);
    let direction = TemperatureDirection::from_raw(direction_raw)?;
    let target_value = definition.value(target_key)?;
    let trimmed = target_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let target_spec = normalize_key(trimmed);
    Some(TemperatureConversion {
        threshold,
        direction,
        target: TemperatureTarget::MaterialName(target_spec.clone()),
        target_spec,
        kind,
    })
}

fn parse_blast_shift_to(value: Option<&str>) -> (Option<String>, bool) {
    let Some(raw_value) = value else {
        return (None, false);
    };
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return (None, false);
    }

    let normalized_full = normalize_key(trimmed);
    if normalized_full == SKY_KEY {
        return (None, true);
    }

    let material_part = trimmed
        .split(['-', '+', '\\', '/', '.'])
        .next()
        .unwrap_or(trimmed)
        .trim();
    if material_part.is_empty() {
        return (None, false);
    }

    let normalized = normalize_key(material_part);
    if normalized == SKY_KEY {
        (None, true)
    } else {
        (Some(normalized), false)
    }
}

#[inline]
fn density_is_solid(density: i32) -> bool {
    density >= C4M_SOLID
}

#[inline]
fn density_is_semi_solid(density: i32) -> bool {
    density >= C4M_LIQUID
}

#[inline]
fn density_is_liquid(density: i32) -> bool {
    (C4M_LIQUID..C4M_SOLID).contains(&density)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaterialId(u16);

impl MaterialId {
    pub fn new(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(MaterialId)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone)]
pub struct Material {
    id: MaterialId,
    definition: ResourceMaterialDefinition,
    properties: MaterialProperties,
    color: Vec<i32>,
    alpha: Vec<i32>,
    normalized_name: String,
}

#[derive(Debug, Clone)]
pub struct MaterialProperties {
    density: i32,
    friction: i32,
    placement: i32,
    /// C4MaterialCore::MinHeightCount: minimum vertical run length for
    /// `EffectiveMatCount` accounting (C4Material.h:120).
    min_height_count: i32,
    splash_rate: i32,
    dig_free: bool,
    blast_free: bool,
    blast_to_object: Option<String>,
    blast_to_object_ratio: Option<i32>,
    blast_to_pxs_ratio: Option<i32>,
    blast_shift_to: Option<String>,
    blast_shift_to_target: Option<MaterialId>,
    blast_shift_to_clears: bool,
    dig_to_object: Option<String>,
    dig_to_object_ratio: Option<i32>,
    dig_to_object_on_request_only: bool,
    wind_drift: i32,
    max_slide: i32,
    instable: bool,
    inflammable: i32,
    incindiary: i32,
    extinguisher: i32,
    corrosive: i32,
    corrode: i32,
    temp_conv_strength: i32,
    in_mat_convert: Option<String>,
    in_mat_convert_to: Option<String>,
    in_mat_convert_target: Option<MaterialId>,
    in_mat_convert_depth: Option<i32>,
    above_temperature: Option<TemperatureConversion>,
    below_temperature: Option<TemperatureConversion>,
}

impl MaterialProperties {
    fn from_definition(definition: &ResourceMaterialDefinition) -> Self {
        let density = definition.int("density").unwrap_or(0);
        let friction = definition.int("friction").unwrap_or(0);
        let min_height_count = definition.int("minheightcount").unwrap_or(0);
        let dig_free = definition.bool_flag("digfree").unwrap_or(false);
        let blast_free = definition.bool_flag("blastfree").unwrap_or(false);
        let dig_to_object_on_request_only = definition
            .bool_flag("dig2objectrequest")
            .or_else(|| definition.bool_flag("dig2objectonrequestonly"))
            .unwrap_or(false);
        let placement = match definition.int("placement") {
            Some(value) if value > 0 => value,
            _ => Self::default_placement(
                density,
                dig_free,
                blast_free,
                dig_to_object_on_request_only,
            ),
        };
        let splash_rate = definition.int("splashrate").unwrap_or(10).max(0);
        let wind_drift = definition.int("winddrift").unwrap_or(0);
        let max_slide = definition.int("maxslide").unwrap_or(0).max(0);
        let instable = definition.int("instable").unwrap_or(0) != 0;
        let inflammable = definition.int("inflammable").unwrap_or(0);
        let incindiary = definition.int("incindiary").unwrap_or(0);
        let extinguisher = definition.int("extinguisher").unwrap_or(0);
        let corrosive = definition.int("corrosive").unwrap_or(0);
        let corrode = definition.int("corrode").unwrap_or(0);
        let temp_conv_strength = definition.int("tempconvstrength").unwrap_or(0);
        let blast_to_object = definition.value("blast2object").and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_uppercase())
            }
        });
        let blast_to_object_ratio = definition
            .int("blast2objectratio")
            .filter(|ratio| *ratio > 0);
        let blast_to_pxs_ratio = definition.int("blast2pxsratio").filter(|ratio| *ratio > 0);
        let (blast_shift_to, blast_shift_to_clears) =
            parse_blast_shift_to(definition.value("blastshiftto"));
        let dig_to_object = definition.value("dig2object").and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_uppercase())
            }
        });
        let dig_to_object_ratio = definition.int("dig2objectratio").filter(|ratio| *ratio > 0);
        let in_mat_convert = definition.value("inmatconvert").and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(normalize_key(trimmed))
            }
        });
        let in_mat_convert_to = definition.value("inmatconvertto").and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(normalize_key(trimmed))
            }
        });
        let in_mat_convert_depth = match definition.int("inmatconvertdepth") {
            Some(value) if value > 0 => Some(value),
            _ => None,
        };
        let above_temperature = parse_temperature_conversion(
            definition,
            "abovetempconvert",
            "abovetempconvertdir",
            "abovetempconvertto",
            TemperatureConversionKind::Above,
        );
        let below_temperature = parse_temperature_conversion(
            definition,
            "belowtempconvert",
            "belowtempconvertdir",
            "belowtempconvertto",
            TemperatureConversionKind::Below,
        );
        Self {
            density,
            friction,
            placement,
            min_height_count,
            splash_rate,
            dig_free,
            blast_free,
            blast_to_object,
            blast_to_object_ratio,
            blast_to_pxs_ratio,
            blast_shift_to,
            blast_shift_to_target: None,
            blast_shift_to_clears,
            dig_to_object,
            dig_to_object_ratio,
            dig_to_object_on_request_only,
            wind_drift,
            max_slide,
            instable,
            inflammable,
            incindiary,
            extinguisher,
            corrosive,
            corrode,
            temp_conv_strength,
            in_mat_convert,
            in_mat_convert_to,
            in_mat_convert_target: None,
            in_mat_convert_depth,
            above_temperature,
            below_temperature,
        }
    }

    fn default_placement(
        density: i32,
        dig_free: bool,
        blast_free: bool,
        dig_to_object_on_request_only: bool,
    ) -> i32 {
        if density_is_solid(density) {
            let mut placement = 30;
            if !dig_free {
                placement += 20;
            }
            if !blast_free {
                placement += 10;
            }
            if !dig_to_object_on_request_only {
                placement += 10;
            }
            placement
        } else if density_is_liquid(density) {
            10
        } else {
            5
        }
    }
}

impl Material {
    fn new(id: MaterialId, definition: ResourceMaterialDefinition) -> Self {
        let properties = MaterialProperties::from_definition(&definition);
        // C4MaterialCore::CompileFunc reads Color and then ColorX into the
        // same fixed array, so a present ColorX entry replaces Color.
        let color = definition
            .int_list("colorx")
            .or_else(|| definition.int_list("color"))
            .unwrap_or_default();
        let alpha = definition.int_list("alpha").unwrap_or_default();
        let normalized_name = normalize_key(definition.name());
        Self {
            id,
            definition,
            properties,
            color,
            alpha,
            normalized_name,
        }
    }

    fn set_id(&mut self, id: MaterialId) {
        self.id = id;
    }

    pub fn id(&self) -> MaterialId {
        self.id
    }

    pub fn name(&self) -> &str {
        self.definition.name()
    }

    pub fn definition(&self) -> &ResourceMaterialDefinition {
        &self.definition
    }

    /// Raw `[Material]` core entry by compile name and value index
    /// (GetValByStdCompiler over C4MaterialCore — FnGetMaterialVal,
    /// C4Script.cpp:4282-4300).
    pub fn core_entry(&self, entry: &str, index: usize) -> Option<&str> {
        self.definition
            .values(entry)
            .and_then(|values| values.get(index))
            .map(|value| value.as_str())
    }

    pub fn density(&self) -> i32 {
        self.properties.density
    }

    pub fn friction(&self) -> i32 {
        self.properties.friction
    }

    pub fn placement(&self) -> i32 {
        self.properties.placement
    }

    pub fn min_height_count(&self) -> i32 {
        self.properties.min_height_count
    }

    pub fn splash_rate(&self) -> i32 {
        self.properties.splash_rate
    }

    pub fn dig_free(&self) -> bool {
        self.properties.dig_free
    }

    pub fn blast_free(&self) -> bool {
        self.properties.blast_free
    }

    pub fn wind_drift(&self) -> i32 {
        self.properties.wind_drift
    }

    pub fn max_slide(&self) -> i32 {
        self.properties.max_slide
    }

    pub fn instable(&self) -> bool {
        self.properties.instable
    }

    pub fn inflammable(&self) -> i32 {
        self.properties.inflammable
    }

    pub fn blast_to_object_name(&self) -> Option<&str> {
        self.properties.blast_to_object.as_deref()
    }

    pub fn blast_to_object_ratio(&self) -> Option<i32> {
        self.properties.blast_to_object_ratio
    }

    pub fn blast_to_pxs_ratio(&self) -> Option<i32> {
        self.properties.blast_to_pxs_ratio
    }

    pub fn blast_shift_to_target(&self) -> Option<MaterialId> {
        self.properties.blast_shift_to_target
    }

    /// The complete `BlastShiftTo` material-texture specification. C++ keeps
    /// this string until `C4MaterialMap::CrossMapMaterials` resolves the exact
    /// texmap byte (C4Material.cpp:474-479); reducing it to a material id loses
    /// texture identity when one material occupies multiple texmap slots.
    pub fn blast_shift_to_spec(&self) -> Option<&str> {
        self.definition
            .value("blastshiftto")
            .map(str::trim)
            .filter(|spec| !spec.is_empty())
    }

    pub fn blast_shift_to_clears(&self) -> bool {
        self.properties.blast_shift_to_clears
    }

    pub fn dig_to_object_name(&self) -> Option<&str> {
        self.properties.dig_to_object.as_deref()
    }

    pub fn dig_to_object_ratio(&self) -> Option<i32> {
        self.properties.dig_to_object_ratio
    }

    pub fn dig_to_object_on_request_only(&self) -> bool {
        self.properties.dig_to_object_on_request_only
    }

    pub fn color(&self) -> &[i32] {
        &self.color
    }

    pub fn alpha(&self) -> &[i32] {
        &self.alpha
    }

    pub fn is_solid(&self) -> bool {
        density_is_solid(self.density())
    }

    pub fn is_liquid(&self) -> bool {
        density_is_liquid(self.density())
    }

    pub fn normalized_name(&self) -> &str {
        &self.normalized_name
    }

    pub fn incindiary(&self) -> i32 {
        self.properties.incindiary
    }

    pub fn extinguisher(&self) -> i32 {
        self.properties.extinguisher
    }

    pub fn corrosive(&self) -> i32 {
        self.properties.corrosive
    }

    pub fn corrode(&self) -> i32 {
        self.properties.corrode
    }

    pub fn temp_conv_strength(&self) -> i32 {
        self.properties.temp_conv_strength
    }

    pub fn in_mat_convert_target(&self) -> Option<MaterialId> {
        self.properties.in_mat_convert_target
    }

    pub fn in_mat_convert_to_name(&self) -> Option<&str> {
        self.properties.in_mat_convert_to.as_deref()
    }

    pub fn in_mat_convert_depth(&self) -> Option<i32> {
        self.properties.in_mat_convert_depth
    }

    pub fn evaluate_temperature_conversion(
        &self,
        direction: TemperatureDirection,
        ambient_temperature: i32,
    ) -> Option<TemperatureConversionOutcome> {
        let candidates = [
            self.properties.below_temperature.as_ref(),
            self.properties.above_temperature.as_ref(),
        ];
        let mut outcome = None;
        for conversion in candidates.into_iter().flatten() {
            if conversion.applies(ambient_temperature, direction) {
                outcome = Some(TemperatureConversionOutcome {
                    target: conversion.target().clone(),
                    target_spec: conversion.target_spec.clone(),
                    strength: self.properties.temp_conv_strength,
                });
            }
        }
        outcome
    }

    fn matches_in_mat_conversion(&self, landscape: Option<&Material>) -> bool {
        let Some(trigger) = self.properties.in_mat_convert.as_deref() else {
            return false;
        };
        match landscape {
            Some(material) => trigger == material.normalized_name(),
            None => trigger == SKY_KEY,
        }
    }

    fn resolve_relations(&mut self, lookup: &HashMap<String, MaterialId>) {
        if let Some(name) = &self.properties.blast_shift_to {
            if let Some(id) = lookup.get(name) {
                self.properties.blast_shift_to_target = Some(*id);
            }
        }
        if let Some(name) = &self.properties.in_mat_convert_to {
            if let Some(id) = lookup.get(name) {
                self.properties.in_mat_convert_target = Some(*id);
            }
        }
        if let Some(conversion) = self.properties.above_temperature.as_mut() {
            conversion.resolve_target(lookup);
        }
        if let Some(conversion) = self.properties.below_temperature.as_mut() {
            conversion.resolve_target(lookup);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MaterialSet {
    materials: Vec<Material>,
    by_name: HashMap<String, MaterialId>,
    custom_reactions_by_event: [Vec<Option<MaterialReaction>>; MATERIAL_EVENT_COUNT],
    /// `ScriptFunc=` names of `Type=Script` reactions, indexed by
    /// `MaterialReactionKind::Script::func` (resolved lazily at call time —
    /// C++ resolves on the global script engine, C4Material.cpp:71-78).
    script_reactions: Vec<String>,
}

impl MaterialSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_resource_library(library: &MaterialLibrary) -> Self {
        let mut materials = Vec::new();
        let mut by_name = HashMap::new();
        for definition in library.iter() {
            let id = match MaterialId::new(materials.len()) {
                Some(id) => id,
                None => break,
            };
            let material = Material::new(id, definition.clone());
            let key = normalize_key(material.name());
            if by_name.contains_key(&key) {
                continue;
            }
            by_name.insert(key, id);
            materials.push(material);
        }
        for material in &mut materials {
            material.resolve_relations(&by_name);
        }
        let (custom_reactions_by_event, script_reactions) =
            build_custom_reactions(&materials, &by_name);
        Self {
            materials,
            by_name,
            custom_reactions_by_event,
            script_reactions,
        }
    }

    /// `C4MaterialMap::SaveEnumeration`: add/replace root MatMap.txt with the
    /// current runtime material names in numeric-id order.
    pub fn save_enumeration(
        &self,
        group: &mut MutableGroup,
    ) -> Result<(), MutableGroupError> {
        let enumeration = lc_resources::material::MaterialEnumeration::from_names(
            self.materials.iter().map(Material::name),
        );
        group.add_file("MatMap.txt", enumeration.to_bytes())
    }

    /// The `ScriptFunc=` name behind a `MaterialReactionKind::Script` entry.
    pub fn script_reaction_name(&self, func: u16) -> Option<&str> {
        self.script_reactions
            .get(usize::from(func))
            .map(String::as_str)
    }

    pub fn push(&mut self, mut material: Material) {
        let key = normalize_key(material.name());
        if self.by_name.contains_key(&key) {
            return;
        }
        if let Some(id) = MaterialId::new(self.materials.len()) {
            material.set_id(id);
            self.by_name.insert(key, id);
            material.resolve_relations(&self.by_name);
            self.materials.push(material);
            self.rebuild_custom_reactions();
        }
    }

    fn rebuild_custom_reactions(&mut self) {
        let (entries, script_reactions) = build_custom_reactions(&self.materials, &self.by_name);
        self.custom_reactions_by_event = entries;
        self.script_reactions = script_reactions;
    }

    pub fn iter(&self) -> impl Iterator<Item = &Material> {
        self.materials.iter()
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Material> {
        self.by_name
            .get(&normalize_key(name))
            .and_then(|id| self.materials.get(id.index()))
    }

    pub fn get_by_id(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id.index())
    }

    pub fn id_of(&self, name: &str) -> Option<MaterialId> {
        self.by_name.get(&normalize_key(name)).copied()
    }

    pub fn default_ground_material(&self) -> Option<MaterialId> {
        self.materials
            .iter()
            .find(|material| material.is_solid())
            .map(|material| material.id())
            .or_else(|| self.materials.first().map(|material| material.id()))
    }

    pub fn default_liquid_material(&self) -> Option<MaterialId> {
        self.materials
            .iter()
            .find(|material| material.is_liquid())
            .map(|material| material.id())
    }

    fn custom_reaction(
        &self,
        pxs_material: Option<MaterialId>,
        landscape_material: Option<MaterialId>,
        event: MaterialInteractionEvent,
    ) -> Option<MaterialReaction> {
        let custom_reactions = &self.custom_reactions_by_event[event.index()];
        if custom_reactions.is_empty() {
            return None;
        }
        let width = self.materials.len() + 1;
        let pxs_index = option_to_index(pxs_material);
        let landscape_index = option_to_index(landscape_material);
        custom_reactions
            .get(landscape_index * width + pxs_index)
            .copied()
            .flatten()
    }

    pub fn reaction(
        &self,
        pxs_material: Option<MaterialId>,
        landscape_material: Option<MaterialId>,
    ) -> MaterialReaction {
        self.reaction_for_event(
            pxs_material,
            landscape_material,
            MaterialInteractionEvent::PxsMove,
        )
    }

    pub fn reaction_for_event(
        &self,
        pxs_material: Option<MaterialId>,
        landscape_material: Option<MaterialId>,
        event: MaterialInteractionEvent,
    ) -> MaterialReaction {
        if let Some(custom) = self.custom_reaction(pxs_material, landscape_material, event) {
            return custom;
        }
        MaterialReaction::builtin(self.builtin_reaction_kind(pxs_material, landscape_material))
    }

    fn builtin_reaction_kind(
        &self,
        pxs_material: Option<MaterialId>,
        landscape_material: Option<MaterialId>,
    ) -> MaterialReactionKind {
        let Some(pxs_id) = pxs_material else {
            return MaterialReactionKind::None;
        };
        let Some(pxs_mat) = self.get_by_id(pxs_id) else {
            return MaterialReactionKind::None;
        };
        let landscape_mat = landscape_material.and_then(|id| self.get_by_id(id));
        if pxs_mat.matches_in_mat_conversion(landscape_mat) {
            return MaterialReactionKind::Convert {
                target: pxs_mat.in_mat_convert_target(),
                depth: pxs_mat.in_mat_convert_depth(),
            };
        }
        let Some(landscape) = landscape_mat else {
            return MaterialReactionKind::None;
        };
        if pxs_mat.density() > landscape.density() {
            return MaterialReactionKind::None;
        }
        if (pxs_mat.incindiary() > 0 && landscape.extinguisher() > 0)
            || (pxs_mat.extinguisher() > 0 && landscape.incindiary() > 0)
        {
            return MaterialReactionKind::Poof;
        }
        if (pxs_mat.incindiary() > 0 && landscape.inflammable() > 0)
            || (pxs_mat.inflammable() > 0 && landscape.incindiary() > 0)
        {
            return MaterialReactionKind::Incinerate;
        }
        if pxs_mat.corrosive() > 0 && landscape.corrode() > 0 {
            return MaterialReactionKind::Corrode {
                corrosive_strength: pxs_mat.corrosive(),
                corrode_resistance: landscape.corrode(),
                corrosion_probability: None,
            };
        }
        MaterialReactionKind::Insert
    }

    /// `instability_probes` collects the coordinates where the C++ reaction
    /// ran `ExtractMaterial` — the caller (the engine) owes each one a
    /// `CheckInstabilityRange` (C4Landscape.cpp:1154). The deferral past the
    /// reaction's trailing Rnd3/Random draws is RNG-neutral: the probe
    /// itself draws nothing.
    pub fn execute_mass_move_reaction(
        &self,
        landscape: &mut Landscape,
        pxs_material: MaterialId,
        pxs_x: i32,
        pxs_y: i32,
        landscape_x: i32,
        landscape_y: i32,
        rng: &mut LcgRng,
        instability_probes: &mut Vec<(i32, i32)>,
    ) -> MaterialReactionExecution {
        let landscape_material = landscape.material_at(landscape_x, landscape_y);
        let reaction = self.reaction_for_event(
            Some(pxs_material),
            landscape_material,
            MaterialInteractionEvent::MassMove,
        );
        execute_mass_move_reaction_kind(
            reaction.kind,
            landscape,
            self,
            pxs_material,
            pxs_x,
            pxs_y,
            landscape_x,
            landscape_y,
            rng,
            instability_probes,
        )
    }

    pub fn evaluate_temperature_conversion(
        &self,
        material: MaterialId,
        direction: TemperatureDirection,
        ambient_temperature: i32,
    ) -> Option<TemperatureConversionOutcome> {
        let material = self.get_by_id(material)?;
        let mut outcome =
            material.evaluate_temperature_conversion(direction, ambient_temperature)?;
        outcome.target = outcome.target.resolved(&self.by_name);
        Some(outcome)
    }

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }
}

pub fn evaluate_corrosion(
    corrosive_strength: i32,
    corrode_resistance: i32,
    corrosion_probability: Option<i32>,
    rng: &mut LcgRng,
) -> bool {
    if let Some(probability) = corrosion_probability {
        return rng.random(100) < probability;
    }

    rng.random(100) < corrosive_strength && rng.random(100) < corrode_resistance
}

pub fn consume_corrosion_effect_rng(rng: &mut LcgRng) {
    if rng.random(5) == 0 {
        let _ = rng.random(3);
    }
    let _ = rng.random(20);
}

#[allow(clippy::too_many_arguments)]
fn execute_mass_move_reaction_kind(
    reaction: MaterialReactionKind,
    landscape: &mut Landscape,
    materials: &MaterialSet,
    pxs_material: MaterialId,
    pxs_x: i32,
    pxs_y: i32,
    landscape_x: i32,
    landscape_y: i32,
    rng: &mut LcgRng,
    instability_probes: &mut Vec<(i32, i32)>,
) -> MaterialReactionExecution {
    match reaction {
        MaterialReactionKind::None | MaterialReactionKind::Insert => {
            MaterialReactionExecution::Unhandled
        }
        // meeMassMove Script and Incinerate reactions run at the ENGINE
        // level (`Engine::execute_mass_move_reaction`): both need state this
        // material/landscape-only helper cannot access.
        MaterialReactionKind::Script { .. } | MaterialReactionKind::Incinerate => {
            MaterialReactionExecution::Unhandled
        }
        // mrfConvert meeMassMove (C4Material.cpp:654-657): unconditional
        // conversion-transfer of the MOVER's material to PXS — the convert
        // target (even an invalid one) plays no role on this event.
        MaterialReactionKind::Convert { .. } => {
            MaterialReactionExecution::Converted(pxs_material)
        }
        MaterialReactionKind::Poof => {
            // mrfPoof meeMassMove (C4Material.cpp:669-670): a real
            // ExtractMaterial — FindMatTop + clear here, the
            // CheckInstabilityRange half owed at the cleared coordinates.
            if let Some((_, top_x, top_y)) =
                landscape.extract_material_probe(landscape_x, landscape_y, materials)
            {
                instability_probes.push((top_x, top_y));
            }
            let _ = rng.rnd3();
            let _ = rng.rnd3();
            MaterialReactionExecution::Consumed
        }
        MaterialReactionKind::Corrode {
            corrosive_strength,
            corrode_resistance,
            corrosion_probability,
        } => {
            if evaluate_corrosion(
                corrosive_strength,
                corrode_resistance,
                corrosion_probability,
                rng,
            ) {
                // mrfCorrode meeMassMove (C4Material.cpp:707-709):
                // ClearBackPix (= ClearPix, C4Wrappers.h:92) — an IN-PLACE
                // clear with NO instability probe on this event.
                if !landscape.clear_pix(landscape_x, landscape_y) {
                    // column-model fixture worlds keep the column removal
                    let _ = landscape.extract_material_at(landscape_x, landscape_y);
                }
                consume_corrosion_effect_rng(rng);
                MaterialReactionExecution::Consumed
            } else {
                MaterialReactionExecution::Unhandled
            }
        }
    }
}

fn option_to_index(material: Option<MaterialId>) -> usize {
    material.map(|id| id.index() + 1).unwrap_or(0)
}

fn build_custom_reactions(
    materials: &[Material],
    by_name: &HashMap<String, MaterialId>,
) -> (
    [Vec<Option<MaterialReaction>>; MATERIAL_EVENT_COUNT],
    Vec<String>,
) {
    let width = materials.len() + 1;
    let mut entries = std::array::from_fn(|_| vec![None; width * width]);
    let mut script_reactions = Vec::new();
    if materials.is_empty() {
        return (entries, script_reactions);
    }

    for material in materials {
        let pxs_id = material.id();
        for reaction_def in material.definition().reactions() {
            let exec_mask = reaction_def
                .int("execmask")
                .map(|value| value as u32)
                .unwrap_or(u32::MAX);

            let Some(kind) =
                parse_custom_reaction_kind(reaction_def, by_name, &mut script_reactions)
            else {
                continue;
            };
            // `CheckSlide=` (fInsertionCheck), default true
            // (C4Material.cpp:66).
            let insertion_check = reaction_def.bool_flag("checkslide").unwrap_or(true);
            let reaction = MaterialReaction {
                kind,
                user_defined: true,
                insertion_check,
            };

            let Some(target_raw) = reaction_def
                .value("targetspec")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };

            let inverse = reaction_def.bool_flag("inversespec").unwrap_or(false);
            let reverse = reaction_def.bool_flag("reverse").unwrap_or(false);

            let targets = resolve_reaction_targets(target_raw, inverse, materials, by_name);
            if targets.is_empty() {
                continue;
            }

            for target in targets {
                for event in MaterialInteractionEvent::ALL {
                    // C++ stores one event-agnostic user-reaction pointer in
                    // the shared pair table. mrfUserCheck turns a masked
                    // event into a no-op; it must not expose the builtin
                    // reaction that occupied the slot before this definition.
                    let event_reaction = if exec_mask & event.mask() == 0 {
                        MaterialReaction::masked_user_noop()
                    } else {
                        reaction
                    };
                    set_custom_reaction(
                        &mut entries[event.index()],
                        width,
                        Some(pxs_id),
                        target,
                        event_reaction,
                        reverse,
                    );
                }
            }
        }
    }

    (entries, script_reactions)
}

fn parse_custom_reaction_kind(
    definition: &MaterialReactionDefinition,
    by_name: &HashMap<String, MaterialId>,
    script_reactions: &mut Vec<String>,
) -> Option<MaterialReactionKind> {
    let reaction_type = definition
        .value("type")
        .map(normalize_key)
        .unwrap_or_default();
    match reaction_type.as_str() {
        // mrfScript (C4Material.cpp:40): the ScriptFunc name is retained
        // verbatim for call-time resolution. An over-full table degrades to
        // NoReaction rather than mis-indexing.
        "script" => {
            let name = definition
                .value("scriptfunc")
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            let index = u16::try_from(script_reactions.len()).ok()?;
            script_reactions.push(name);
            Some(MaterialReactionKind::Script { func: index })
        }
        "convert" => {
            let depth = definition.int("depth").filter(|value| *value > 0);
            let target = definition
                .value("convertmat")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(normalize_key)
                .and_then(|name| {
                    if name == SKY_KEY {
                        Some(None)
                    } else {
                        by_name.get(&name).copied().map(Some)
                    }
                })
                .unwrap_or(None);
            Some(MaterialReactionKind::Convert { target, depth })
        }
        "poof" => Some(MaterialReactionKind::Poof),
        "insert" => Some(MaterialReactionKind::Insert),
        "corrode" => {
            let rate = definition.int("corrosionrate").unwrap_or(100).clamp(0, 100);
            Some(MaterialReactionKind::Corrode {
                corrosive_strength: rate,
                corrode_resistance: 100,
                corrosion_probability: Some(rate),
            })
        }
        // Any unknown (or absent) Type — "Incinerate" included — binds the
        // ReactionFuncMap nullptr sentinel's NoReaction: a user-defined
        // no-op that still occupies the slot and overrides the hardcoded
        // default (C4Material.cpp:38-46,53-57).
        _ => Some(MaterialReactionKind::None),
    }
}

fn resolve_reaction_targets(
    target_spec: &str,
    inverse: bool,
    materials: &[Material],
    by_name: &HashMap<String, MaterialId>,
) -> Vec<Option<MaterialId>> {
    let normalized = normalize_key(target_spec);
    match normalized.as_str() {
        "all" => {
            if inverse {
                Vec::new()
            } else {
                let mut targets = Vec::with_capacity(materials.len() + 1);
                targets.push(None);
                targets.extend(materials.iter().map(|material| Some(material.id())));
                targets
            }
        }
        "solid" => collect_targets_by_predicate(materials, inverse, true, |mat| {
            density_is_solid(mat.density())
        }),
        "semisolid" => collect_targets_by_predicate(materials, inverse, true, |mat| {
            density_is_semi_solid(mat.density())
        }),
        "background" => {
            let mut targets = Vec::new();
            if !inverse {
                targets.push(None);
            }
            for material in materials {
                if (material.density() == 0) != inverse {
                    targets.push(Some(material.id()));
                }
            }
            targets
        }
        "sky" => {
            if inverse {
                materials
                    .iter()
                    .map(|material| Some(material.id()))
                    .collect()
            } else {
                vec![None]
            }
        }
        "incindiary" => {
            collect_targets_by_predicate(materials, inverse, true, |mat| mat.incindiary() > 0)
        }
        "extinguisher" => {
            collect_targets_by_predicate(materials, inverse, true, |mat| mat.extinguisher() > 0)
        }
        "inflammable" => {
            collect_targets_by_predicate(materials, inverse, true, |mat| mat.inflammable() > 0)
        }
        "corrosive" => {
            collect_targets_by_predicate(materials, inverse, true, |mat| mat.corrosive() > 0)
        }
        "corrode" => {
            collect_targets_by_predicate(materials, inverse, true, |mat| mat.corrode() > 0)
        }
        _ => {
            if let Some(&id) = by_name.get(&normalized) {
                if inverse {
                    let mut targets = Vec::with_capacity(materials.len() + 1);
                    targets.push(None);
                    targets.extend(materials.iter().filter_map(|material| {
                        let candidate = material.id();
                        if candidate == id {
                            None
                        } else {
                            Some(Some(candidate))
                        }
                    }));
                    targets
                } else {
                    vec![Some(id)]
                }
            } else {
                Vec::new()
            }
        }
    }
}

fn collect_targets_by_predicate(
    materials: &[Material],
    inverse: bool,
    include_sky_on_inverse: bool,
    predicate: impl Fn(&Material) -> bool,
) -> Vec<Option<MaterialId>> {
    let mut targets = Vec::new();
    if inverse && include_sky_on_inverse {
        targets.push(None);
    }
    for material in materials {
        if predicate(material) != inverse {
            targets.push(Some(material.id()));
        }
    }
    targets
}

fn set_custom_reaction(
    entries: &mut [Option<MaterialReaction>],
    width: usize,
    pxs_material: Option<MaterialId>,
    landscape_material: Option<MaterialId>,
    reaction: MaterialReaction,
    reverse: bool,
) {
    let mut pxs_index = option_to_index(pxs_material);
    let mut landscape_index = option_to_index(landscape_material);
    if reverse {
        std::mem::swap(&mut pxs_index, &mut landscape_index);
    }
    let slot = landscape_index * width + pxs_index;
    if let Some(entry) = entries.get_mut(slot) {
        *entry = Some(reaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_material_set(source: &str) -> MaterialSet {
        let library = MaterialLibrary::parse(source).expect("material library parses");
        MaterialSet::from_resource_library(&library)
    }

    #[test]
    fn material_enumeration_save_writes_cpp_matmap_in_material_id_order() {
        let set = build_material_set(
            r#"
            [Material Granite]
            Name=Granite

            [Material Earth]
            Name=Earth

            [Material Water]
            Name=Water
            "#,
        );
        let mut mutable = MutableGroup::new("Runtime.c4s");
        mutable
            .add_file("matmap.TXT", b"stale".to_vec())
            .expect("stale map adds");

        set.save_enumeration(&mut mutable)
            .expect("material enumeration saves");

        let group = lc_resources::Group::from_raw_memory(
            std::path::PathBuf::from("Runtime.c4s"),
            mutable.pack_raw().expect("runtime group packs"),
        )
        .expect("runtime group reopens");
        assert_eq!(group.entries().expect("entries read").len(), 1);
        assert_eq!(
            group.read_file("MatMap.txt").expect("MatMap reads"),
            b"[Enumeration]\r\nGranite\r\nEarth\r\nWater\r\n "
        );
    }

    #[test]
    fn reaction_returns_convert_for_matching_in_mat_trigger() {
        let set = build_material_set(
            r#"
            [Material Snow]
            Name=Snow
            Density=10
            Friction=5
            InMatConvert=Water
            InMatConvertTo=Water
            InMatConvertDepth=2

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
        );
        let snow = set.id_of("Snow").expect("snow exists");
        let water = set.id_of("Water").expect("water exists");
        let reaction = set.reaction(Some(snow), Some(water)).kind;
        assert_eq!(
            reaction,
            MaterialReactionKind::Convert {
                target: Some(water),
                depth: Some(2),
            }
        );
    }

    #[test]
    fn reaction_returns_poof_for_incindiary_vs_extinguisher() {
        let set = build_material_set(
            r#"
            [Material Fire]
            Name=Fire
            Density=20
            Friction=1
            Incindiary=100

            [Material Water]
            Name=Water
            Density=60
            Friction=0
            Extinguisher=1
        "#,
        );
        let fire = set.id_of("Fire").expect("fire exists");
        let water = set.id_of("Water").expect("water exists");
        assert_eq!(
            set.reaction(Some(fire), Some(water)).kind,
            MaterialReactionKind::Poof
        );
    }

    #[test]
    fn reaction_returns_corrode_for_corrosive_pairing() {
        let set = build_material_set(
            r#"
            [Material Acid]
            Name=Acid
            Density=30
            Friction=0
            Corrosive=75

            [Material Rock]
            Name=Rock
            Density=80
            Friction=10
            Corrode=50
        "#,
        );
        let acid = set.id_of("Acid").expect("acid exists");
        let rock = set.id_of("Rock").expect("rock exists");
        assert_eq!(
            set.reaction(Some(acid), Some(rock)).kind,
            MaterialReactionKind::Corrode {
                corrosive_strength: 75,
                corrode_resistance: 50,
                corrosion_probability: None,
            }
        );
    }

    #[test]
    fn temperature_conversion_resolves_target_material() {
        let set = build_material_set(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            Friction=15
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Water
            TempConvStrength=4

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
        );
        let ice = set.id_of("Ice").expect("ice exists");
        let water = set.id_of("Water").expect("water exists");

        let outcome = set
            .evaluate_temperature_conversion(ice, TemperatureDirection::Downwards, 5)
            .expect("conversion triggered");
        assert_eq!(
            outcome.target,
            TemperatureTarget::Material(water),
            "expected conversion target to resolve to Water material id"
        );
        assert_eq!(outcome.strength, 4);

        assert!(
            set.evaluate_temperature_conversion(ice, TemperatureDirection::Downwards, -10)
                .is_none(),
            "temperature below threshold should not trigger conversion"
        );
    }

    #[test]
    fn temperature_conversion_handles_below_threshold() {
        let set = build_material_set(
            r#"
            [Material Steam]
            Name=Steam
            Density=10
            Friction=1
            BelowTempConvert=30
            BelowTempConvertDir=1
            BelowTempConvertTo=Water

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
        );
        let steam = set.id_of("Steam").expect("steam exists");
        let water = set.id_of("Water").expect("water exists");
        let outcome = set
            .evaluate_temperature_conversion(steam, TemperatureDirection::Upwards, 5)
            .expect("conversion triggered when temperature below threshold");
        assert_eq!(outcome.target, TemperatureTarget::Material(water));
        assert!(
            set.evaluate_temperature_conversion(steam, TemperatureDirection::Upwards, 40)
                .is_none(),
            "temperature above threshold should not trigger below conversion"
        );
    }

    #[test]
    fn temperature_conversion_direction_defaults_downward_like_cpp() {
        let set = build_material_set(
            r#"
            [Material Water]
            Name=Water
            Density=30
            BelowTempConvert=0
            BelowTempConvertTo=Ice
            TempConvStrength=3

            [Material Ice]
            Name=Ice
            Density=80
        "#,
        );
        let water = set.id_of("Water").expect("water exists");
        let ice = set.id_of("Ice").expect("ice exists");

        let outcome = set
            .evaluate_temperature_conversion(water, TemperatureDirection::Downwards, -1)
            .expect("omitted direction defaults to the downward surface");
        assert_eq!(outcome.target, TemperatureTarget::Material(ice));
    }

    #[test]
    fn script_reaction_type_captures_function_name() {
        // ReactionFuncMap binds "Script" to mrfScript (C4Material.cpp:40);
        // the ScriptFunc= name is kept for resolution at call time
        // (ResolveScriptFuncs resolves on the global script engine,
        // C4Material.cpp:71-78 — the Rust port resolves lazily against the
        // scenario script).
        let set = build_material_set(
            r#"
            [Material Goo]
            Name=Goo
            Density=25

            [Reaction]
            Type=Script
            ScriptFunc=GooHitsEarth
            TargetSpec=Earth

            [Material Earth]
            Name=Earth
            Density=100
        "#,
        );
        let goo = set.id_of("Goo").expect("goo exists");
        let earth = set.id_of("Earth").expect("earth exists");
        let reaction = set.reaction(Some(goo), Some(earth));
        assert!(reaction.user_defined);
        let MaterialReactionKind::Script { func } = reaction.kind else {
            panic!("Type=Script parses as a Script reaction");
        };
        assert_eq!(
            set.script_reaction_name(func),
            Some("GooHitsEarth"),
            "the ScriptFunc name is retained verbatim"
        );
    }

    #[test]
    fn unknown_reaction_types_install_overriding_no_reaction_like_cpp() {
        // ReactionFuncMap (C4Material.cpp:38-46) names only Script/Convert/
        // Poof/Corrode/Insert; any other Type — including "Incinerate" and
        // an absent one — binds the nullptr sentinel's NoReaction
        // (C4Material.cpp:45): a user-defined no-op that still OCCUPIES the
        // reaction slot, suppressing the hardcoded default reaction.
        let set = build_material_set(
            r#"
            [Material Fire]
            Name=Fire
            Density=20
            Incindiary=100

            [Reaction]
            Type=Incinerate
            TargetSpec=Snow

            [Material Snow]
            Name=Snow
            Density=50
            Inflammable=100

            [Material Acid]
            Name=Acid
            Density=25
            Corrosive=80

            [Reaction]
            Type=Frobnicate
            TargetSpec=Rock

            [Material Rock]
            Name=Rock
            Density=70
            Corrode=60

            [Material Mist]
            Name=Mist
            Density=20
            Extinguisher=1

            [Reaction]
            TargetSpec=Ember

            [Material Ember]
            Name=Ember
            Density=60
            Incindiary=1
            "#,
        );
        let fire = set.id_of("Fire").expect("fire material");
        let snow = set.id_of("Snow").expect("snow material");
        let reaction = set.reaction(Some(fire), Some(snow));
        assert_eq!(
            reaction.kind,
            MaterialReactionKind::None,
            "Type=Incinerate is not user-nameable; the slot holds NoReaction \
             instead of the default incineration",
        );
        assert!(reaction.user_defined, "the no-op entry is user-defined");

        let acid = set.id_of("Acid").expect("acid material");
        let rock = set.id_of("Rock").expect("rock material");
        assert_eq!(
            set.reaction(Some(acid), Some(rock)).kind,
            MaterialReactionKind::None,
            "an unknown Type still overrides the hardcoded default",
        );

        let mist = set.id_of("Mist").expect("mist material");
        let ember = set.id_of("Ember").expect("ember material");
        assert_eq!(
            set.reaction(Some(mist), Some(ember)).kind,
            MaterialReactionKind::None,
            "a [Reaction] without Type= also binds NoReaction",
        );
    }

    #[test]
    fn custom_reaction_poof_with_reverse_category_target() {
        let set = build_material_set(
            r#"
            [Material Fire]
            Name=Fire
            Density=20
            Incindiary=100

            [Material Snow]
            Name=Snow
            Density=50

            [Reaction]
            Type=Poof
            TargetSpec=Incindiary
            Reverse=1
            "#,
        );
        let fire = set.id_of("Fire").expect("fire material");
        let snow = set.id_of("Snow").expect("snow material");
        assert_eq!(
            set.reaction(Some(fire), Some(snow)).kind,
            MaterialReactionKind::Poof,
            "reverse category reaction should apply to incoming incendiary material",
        );
    }

    #[test]
    fn custom_reaction_convert_with_depth_override() {
        let set = build_material_set(
            r#"
            [Material Acid]
            Name=Acid
            Density=30

            [Reaction]
            Type=Convert
            TargetSpec=Rock
            ConvertMat=Water
            Depth=2

            [Material Rock]
            Name=Rock
            Density=80

            [Material Water]
            Name=Water
            Density=25
            "#,
        );
        let acid = set.id_of("Acid").expect("acid material");
        let rock = set.id_of("Rock").expect("rock material");
        let water = set.id_of("Water").expect("water material");
        assert_eq!(
            set.reaction(Some(acid), Some(rock)).kind,
            MaterialReactionKind::Convert {
                target: Some(water),
                depth: Some(2),
            },
            "explicit convert reaction should override default behavior",
        );
    }

    #[test]
    fn custom_reaction_applies_to_sky_target() {
        let set = build_material_set(
            r#"
            [Material Steam]
            Name=Steam
            Density=5

            [Reaction]
            Type=Poof
            TargetSpec=Sky
            "#,
        );
        let steam = set.id_of("Steam").expect("steam material");
        assert_eq!(
            set.reaction(Some(steam), None).kind,
            MaterialReactionKind::Poof,
            "reaction targeting sky should trigger when no landscape material is present",
        );
    }

    #[test]
    fn custom_reaction_exec_mask_without_pxs_move_occupies_slot_with_noop() {
        let set = build_material_set(
            r#"
            [Material Mist]
            Name=Mist
            Density=25

            [Reaction]
            Type=Poof
            TargetSpec=Water
            ExecMask=1

            [Material Water]
            Name=Water
            Density=25
            "#,
        );
        let mist = set.id_of("Mist").expect("mist material");
        let water = set.id_of("Water").expect("water material");
        let reaction = set.reaction_for_event(
            Some(mist),
            Some(water),
            MaterialInteractionEvent::PxsMove,
        );
        assert_eq!(reaction.kind, MaterialReactionKind::None);
        assert!(
            reaction.user_defined,
            "the masked event still occupies the pair slot instead of falling back to builtin Insert"
        );
        assert!(
            !reaction.insertion_check,
            "ExecMask rejection precedes CheckSlide in C++"
        );
    }

    #[test]
    fn custom_reaction_exec_mask_applies_to_mass_move() {
        let set = build_material_set(
            r#"
            [Material Mist]
            Name=Mist
            Density=25

            [Reaction]
            Type=Poof
            TargetSpec=Water
            ExecMask=4

            [Material Water]
            Name=Water
            Density=25
            "#,
        );
        let mist = set.id_of("Mist").expect("mist material");
        let water = set.id_of("Water").expect("water material");
        let pxs_move = set.reaction_for_event(
            Some(mist),
            Some(water),
            MaterialInteractionEvent::PxsMove,
        );
        assert_eq!(pxs_move.kind, MaterialReactionKind::None);
        assert!(
            pxs_move.user_defined,
            "masked PXSMove must not fall through to builtin Insert"
        );
        assert!(!pxs_move.insertion_check);
        let mass_move =
            set.reaction_for_event(Some(mist), Some(water), MaterialInteractionEvent::MassMove);
        assert_eq!(
            mass_move.kind,
            MaterialReactionKind::Poof,
            "mass-move exec mask should be available to mass movers",
        );
        assert!(mass_move.user_defined);
    }
}
