use crate::{Group, GroupError};
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FontResourceError {
    #[error("font resource `{name}` not found")]
    NotFound { name: String },
    #[error(transparent)]
    Group(#[from] GroupError),
}

#[derive(Debug, Clone)]
pub struct FontResource {
    name: String,
    data: Arc<[u8]>,
}

impl FontResource {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            data: Arc::from(bytes.into_boxed_slice()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn clone_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }
}

pub fn load_ttf(group: &Group, name: &str) -> Result<FontResource, FontResourceError> {
    let normalized = name.replace('\\', "/");
    let candidate = Path::new(&normalized);
    if !group.exists(candidate) {
        return Err(FontResourceError::NotFound {
            name: name.to_string(),
        });
    }
    let bytes = group.read_file(candidate)?;
    Ok(FontResource::new(name, bytes))
}

pub fn load_endeavour_font(group: &Group) -> Result<FontResource, FontResourceError> {
    load_ttf(group, "Endeavour.ttf")
}
