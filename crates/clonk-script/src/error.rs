use thiserror::Error;

use crate::vm::ScriptHostIdentity;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("parse error at {0}:{1}: {2}")]
    Parse(usize, usize, String),
    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
}

impl ScriptError {
    pub fn parse(message: impl Into<String>, line: usize, column: usize) -> Self {
        ScriptError::Parse(line, column, message.into())
    }

    pub fn call_frames(&self) -> &[RuntimeCallFrame] {
        match self {
            Self::Runtime(error) => error.call_frames(),
            Self::Parse(..) => &[],
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct RuntimeError {
    message: String,
    call_frames: Vec<RuntimeCallFrame>,
}

/// One active C4Aul script context captured when a runtime error is raised.
/// Frames are stored in native dump order: innermost first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCallFrame {
    function: String,
    arguments: String,
    direct_exec_display: Option<String>,
    object_context: Option<String>,
    definition_context: Option<String>,
    source_host_identity: Option<ScriptHostIdentity>,
    source_name: Option<String>,
    source_line: usize,
}

impl RuntimeCallFrame {
    pub(crate) fn new(
        function: String,
        arguments: String,
        object_context: Option<String>,
        definition_context: Option<String>,
        source_host_identity: Option<ScriptHostIdentity>,
        source_name: Option<String>,
        source_line: usize,
    ) -> Self {
        Self {
            function,
            arguments,
            direct_exec_display: None,
            object_context,
            definition_context,
            source_host_identity,
            source_name,
            source_line,
        }
    }

    pub(crate) fn direct_exec(stack_display: String) -> Self {
        Self {
            function: String::new(),
            arguments: String::new(),
            direct_exec_display: Some(stack_display),
            object_context: None,
            definition_context: None,
            source_host_identity: None,
            source_name: None,
            source_line: 0,
        }
    }

    pub fn function(&self) -> &str {
        &self.function
    }

    pub fn arguments(&self) -> &str {
        &self.arguments
    }

    /// Exact native stack display for a temporary C4Aul DirectExec context.
    /// Ordinary function frames return `None`.
    pub fn direct_exec_display(&self) -> Option<&str> {
        self.direct_exec_display.as_deref()
    }

    pub fn object_context(&self) -> Option<&str> {
        self.object_context.as_deref()
    }

    pub fn definition_context(&self) -> Option<&str> {
        self.definition_context.as_deref()
    }

    pub fn source_host_identity(&self) -> Option<ScriptHostIdentity> {
        self.source_host_identity
    }

    pub fn source_name(&self) -> Option<&str> {
        self.source_name.as_deref()
    }

    /// Zero-based declaration line used when the tree-walking VM has no
    /// bytecode-program-counter location for the active expression.
    pub fn source_line(&self) -> usize {
        self.source_line
    }
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            call_frames: crate::vm::snapshot_active_runtime_frames(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn call_frames(&self) -> &[RuntimeCallFrame] {
        &self.call_frames
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("parse error at {line}:{column}: {message}")]
pub struct ParseError {
    message: String,
    line: usize,
    column: usize,
}

impl ParseError {
    pub fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn column(&self) -> usize {
        self.column
    }
}

impl From<ParseError> for ScriptError {
    fn from(err: ParseError) -> Self {
        ScriptError::parse(err.message, err.line, err.column)
    }
}
