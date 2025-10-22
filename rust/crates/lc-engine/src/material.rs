use lc_resources::{MaterialDefinition as ResourceMaterialDefinition, MaterialLibrary};
use std::collections::HashMap;

fn normalize_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[derive(Debug, Clone, Default)]
pub struct MaterialSet {
    materials: Vec<Material>,
    by_name: HashMap<String, usize>,
}

impl MaterialSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_resource_library(library: &MaterialLibrary) -> Self {
        let mut materials = Vec::new();
        let mut by_name = HashMap::new();
        for definition in library.iter() {
            let material = Material::from_definition(definition.clone());
            let key = normalize_key(material.name());
            if by_name.contains_key(&key) {
                continue;
            }
            by_name.insert(key, materials.len());
            materials.push(material);
        }
        Self { materials, by_name }
    }

    pub fn push(&mut self, material: Material) {
        let key = normalize_key(material.name());
        if self.by_name.contains_key(&key) {
            return;
        }
        let index = self.materials.len();
        self.materials.push(material);
        self.by_name.insert(key, index);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Material> {
        self.materials.iter()
    }

    pub fn get(&self, name: &str) -> Option<&Material> {
        self.by_name
            .get(&normalize_key(name))
            .and_then(|&index| self.materials.get(index))
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Material {
    definition: ResourceMaterialDefinition,
    density: i32,
    friction: i32,
    dig_free: bool,
    blast_free: bool,
    inflammable: bool,
    placement: i32,
    splash_rate: i32,
    color: Vec<i32>,
    alpha: Vec<i32>,
}

impl Material {
    pub fn from_definition(definition: ResourceMaterialDefinition) -> Self {
        let density = definition.int("density").unwrap_or(0);
        let friction = definition.int("friction").unwrap_or(0);
        let dig_free = definition.bool_flag("digfree").unwrap_or(false);
        let blast_free = definition.bool_flag("blastfree").unwrap_or(false);
        let inflammable = definition.bool_flag("inflammable").unwrap_or(false);
        let placement = definition.int("placement").unwrap_or(0);
        let splash_rate = definition.int("splashrate").unwrap_or(10);
        let color = definition.int_list("color").unwrap_or_default();
        let alpha = definition.int_list("alpha").unwrap_or_default();
        Self {
            definition,
            density,
            friction,
            dig_free,
            blast_free,
            inflammable,
            placement,
            splash_rate,
            color,
            alpha,
        }
    }

    pub fn name(&self) -> &str {
        self.definition.name()
    }

    pub fn definition(&self) -> &ResourceMaterialDefinition {
        &self.definition
    }

    pub fn density(&self) -> i32 {
        self.density
    }

    pub fn friction(&self) -> i32 {
        self.friction
    }

    pub fn dig_free(&self) -> bool {
        self.dig_free
    }

    pub fn blast_free(&self) -> bool {
        self.blast_free
    }

    pub fn inflammable(&self) -> bool {
        self.inflammable
    }

    pub fn placement(&self) -> i32 {
        self.placement
    }

    pub fn splash_rate(&self) -> i32 {
        self.splash_rate
    }

    pub fn color(&self) -> &[i32] {
        &self.color
    }

    pub fn alpha(&self) -> &[i32] {
        &self.alpha
    }
}
