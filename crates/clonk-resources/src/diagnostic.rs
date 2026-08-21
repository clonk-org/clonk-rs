use std::path::PathBuf;

/// Non-fatal diagnostics produced while compiling legacy resources.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceLoadDiagnostic {
    UnknownDefinitionBitName { bit_name: String },
    ScriptStringTableEntryNotFound { path: PathBuf, key: String },
}

impl ResourceLoadDiagnostic {
    /// Emits the diagnostic through the ordinary resource logging callsite.
    pub fn emit(self) {
        match self {
            Self::UnknownDefinitionBitName { bit_name } => {
                crate::definition::emit_unknown_definition_bit_name(&bit_name);
            }
            Self::ScriptStringTableEntryNotFound { path, key } => {
                crate::script_strings::emit_missing_script_string(&path, &key);
            }
        }
    }
}
