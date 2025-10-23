use lc_resources::{MaterialDefinition as ResourceMaterialDefinition, MaterialLibrary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const C4M_SOLID: i32 = 50;
const C4M_LIQUID: i32 = 25;

fn normalize_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
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
        Self {
            density,
            friction,
            placement,
            splash_rate,
            dig_free,
            blast_free,
            wind_drift,
            inflammable,
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
        Self {
            id,
            definition,
            properties,
            color,
            alpha,
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

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }
}
