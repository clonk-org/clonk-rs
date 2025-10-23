use lc_resources::{MaterialDefinition as ResourceMaterialDefinition, MaterialLibrary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const C4M_SOLID: i32 = 50;
const C4M_LIQUID: i32 = 25;
const SKY_KEY: &str = "sky";

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
    },
    Insert,
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

#[inline]
fn density_is_solid(density: i32) -> bool {
    density >= C4M_SOLID
}

#[inline]
fn density_is_liquid(density: i32) -> bool {
    density >= C4M_LIQUID && density < C4M_SOLID
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
    wind_drift: i32,
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
        let inflammable = definition.int("inflammable").unwrap_or(0);
        let incindiary = definition.int("incindiary").unwrap_or(0);
        let extinguisher = definition.int("extinguisher").unwrap_or(0);
        let corrosive = definition.int("corrosive").unwrap_or(0);
        let corrode = definition.int("corrode").unwrap_or(0);
        let temp_conv_strength = definition.int("tempconvstrength").unwrap_or(0);
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
            wind_drift,
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

    pub fn inflammable(&self) -> i32 {
        self.properties.inflammable
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
        Self { materials, by_name }
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
        }
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

    pub fn reaction(
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
            };
        }
        MaterialReactionKind::Insert
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
        let reaction = set.reaction(Some(snow), Some(water));
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
            set.reaction(Some(fire), Some(water)),
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
            set.reaction(Some(acid), Some(rock)),
            MaterialReactionKind::Corrode {
                corrosive_strength: 75,
                corrode_resistance: 50,
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
}
