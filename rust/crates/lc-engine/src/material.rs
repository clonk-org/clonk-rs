use lc_resources::{
    material::MaterialReactionDefinition, MaterialDefinition as ResourceMaterialDefinition,
    MaterialLibrary,
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
            if let Some(id) = lookup.get(name) {
                *self = TemperatureTarget::Material(*id);
            }
        }
    }

    fn resolved(&self, lookup: &HashMap<String, MaterialId>) -> TemperatureTarget {
        match self {
            TemperatureTarget::Material(id) => TemperatureTarget::Material(*id),
            TemperatureTarget::MaterialName(name) => lookup
                .get(name)
                .copied()
                .map(TemperatureTarget::Material)
                .unwrap_or_else(|| TemperatureTarget::MaterialName(name.clone())),
        }
    }

    pub fn as_material_id(&self) -> Option<MaterialId> {
        match self {
            TemperatureTarget::Material(id) => Some(*id),
            TemperatureTarget::MaterialName(_) => None,
        }
    }

    pub fn is_sky(&self) -> bool {
        matches!(self, TemperatureTarget::MaterialName(name) if name == SKY_KEY)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperatureConversion {
    threshold: i32,
    direction: TemperatureDirection,
    target: TemperatureTarget,
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

    fn index(self) -> usize {
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
    let threshold = definition.int(threshold_key)?;
    let direction_raw = definition.int(direction_key)?;
    let direction = TemperatureDirection::from_raw(direction_raw)?;
    let target_value = definition.value(target_key)?;
    let trimmed = target_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(TemperatureConversion {
        threshold,
        direction,
        target: TemperatureTarget::MaterialName(normalize_key(trimmed)),
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
        let color = definition.int_list("color").unwrap_or_default();
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

    pub fn density(&self) -> i32 {
        self.properties.density
    }

    pub fn friction(&self) -> i32 {
        self.properties.friction
    }

    pub fn placement(&self) -> i32 {
        self.properties.placement
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
        for conversion in candidates.into_iter().flatten() {
            if conversion.applies(ambient_temperature, direction) {
                return Some(TemperatureConversionOutcome {
                    target: conversion.target().clone(),
                    strength: self.properties.temp_conv_strength,
                });
            }
        }
        None
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
        let custom_reactions_by_event = build_custom_reactions(&materials, &by_name);
        Self {
            materials,
            by_name,
            custom_reactions_by_event,
        }
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
        self.custom_reactions_by_event = build_custom_reactions(&self.materials, &self.by_name);
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

    pub fn execute_mass_move_reaction(
        &self,
        landscape: &mut Landscape,
        pxs_material: MaterialId,
        pxs_x: i32,
        pxs_y: i32,
        landscape_x: i32,
        landscape_y: i32,
        rng: &mut LcgRng,
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
            pxs_x,
            pxs_y,
            landscape_x,
            landscape_y,
            rng,
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

fn execute_mass_move_reaction_kind(
    reaction: MaterialReactionKind,
    landscape: &mut Landscape,
    materials: &MaterialSet,
    pxs_x: i32,
    pxs_y: i32,
    landscape_x: i32,
    landscape_y: i32,
    rng: &mut LcgRng,
) -> MaterialReactionExecution {
    match reaction {
        MaterialReactionKind::None | MaterialReactionKind::Insert => {
            MaterialReactionExecution::Unhandled
        }
        MaterialReactionKind::Convert {
            target: Some(target),
            ..
        } => MaterialReactionExecution::Converted(target),
        MaterialReactionKind::Convert { target: None, .. } => MaterialReactionExecution::Consumed,
        MaterialReactionKind::Poof => {
            let _ = landscape.extract_material_at(landscape_x, landscape_y);
            let _ = rng.rnd3();
            let _ = rng.rnd3();
            MaterialReactionExecution::Consumed
        }
        MaterialReactionKind::Incinerate => {
            if landscape.incinerate_at(pxs_x, pxs_y, materials) {
                MaterialReactionExecution::Consumed
            } else {
                MaterialReactionExecution::Unhandled
            }
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
                let _ = landscape.extract_material_at(landscape_x, landscape_y);
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
) -> [Vec<Option<MaterialReaction>>; MATERIAL_EVENT_COUNT] {
    let width = materials.len() + 1;
    let mut entries = std::array::from_fn(|_| vec![None; width * width]);
    if materials.is_empty() {
        return entries;
    }

    for material in materials {
        let pxs_id = material.id();
        for reaction_def in material.definition().reactions() {
            let exec_mask = reaction_def
                .int("execmask")
                .map(|value| value as u32)
                .unwrap_or(u32::MAX);

            let Some(kind) = parse_custom_reaction_kind(reaction_def, by_name) else {
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
                    if exec_mask & event.mask() == 0 {
                        continue;
                    }
                    set_custom_reaction(
                        &mut entries[event.index()],
                        width,
                        Some(pxs_id),
                        target,
                        reaction,
                        reverse,
                    );
                }
            }
        }
    }

    entries
}

fn parse_custom_reaction_kind(
    definition: &MaterialReactionDefinition,
    by_name: &HashMap<String, MaterialId>,
) -> Option<MaterialReactionKind> {
    let reaction_type = normalize_key(definition.value("type")?);
    match reaction_type.as_str() {
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
        "incinerate" => Some(MaterialReactionKind::Incinerate),
        "insert" => Some(MaterialReactionKind::Insert),
        "corrode" => {
            let rate = definition.int("corrosionrate").unwrap_or(100).clamp(0, 100);
            Some(MaterialReactionKind::Corrode {
                corrosive_strength: rate,
                corrode_resistance: 100,
                corrosion_probability: Some(rate),
            })
        }
        _ => None,
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
    fn custom_reaction_exec_mask_without_pxs_move_is_ignored() {
        let set = build_material_set(
            r#"
            [Material Mist]
            Name=Mist
            Density=5

            [Reaction]
            Type=Poof
            TargetSpec=Sky
            ExecMask=1
            "#,
        );
        let mist = set.id_of("Mist").expect("mist material");
        assert_eq!(
            set.reaction(Some(mist), None).kind,
            MaterialReactionKind::None,
            "reaction without PXSMove bit should not apply to particle movement",
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
            TargetSpec=Sky
            ExecMask=4
            "#,
        );
        let mist = set.id_of("Mist").expect("mist material");
        assert_eq!(
            set.reaction(Some(mist), None).kind,
            MaterialReactionKind::None,
            "mass-move-only reaction should not affect PXSMove lookups",
        );
        assert_eq!(
            set.reaction_for_event(Some(mist), None, MaterialInteractionEvent::MassMove).kind,
            MaterialReactionKind::Poof,
            "mass-move exec mask should be available to mass movers",
        );
    }
}
