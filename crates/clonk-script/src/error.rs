use std::any::Any;
use std::fmt;
use std::num::NonZeroUsize;
use std::rc::Rc;

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

    /// Consume an ordinary script error into the thread-safe diagnostic that
    /// embedding engines may retain or send across their worker boundary.
    /// Host continuations stay in the VM-facing error channel: returning the
    /// original error here prevents an accidental drop of an owned request or
    /// suspended frame at the diagnostic boundary.
    pub fn into_diagnostic(self) -> Result<ScriptErrorDiagnostic, Self> {
        match self {
            Self::Parse(line, column, message) => {
                Ok(ScriptErrorDiagnostic::Parse(line, column, message))
            }
            Self::Runtime(error) => error.into_diagnostic().map_err(Self::Runtime),
        }
    }

    pub fn call_frames(&self) -> &[RuntimeCallFrame] {
        match self {
            Self::Runtime(error) => error.call_frames(),
            Self::Parse(..) => &[],
        }
    }
}

/// Thread-safe, owned representation of an ordinary script failure.
///
/// A [`RuntimeError`] may also carry a host continuation and therefore cannot
/// cross an embedding engine's worker boundary. Callers that need to retain
/// or send a script failure should first call [`ScriptError::into_diagnostic`];
/// that conversion is fallible and leaves continuation-bearing errors in the
/// VM-facing [`ScriptError`] form.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScriptErrorDiagnostic {
    #[error("parse error at {0}:{1}: {2}")]
    Parse(usize, usize, String),
    #[error("runtime error: {message}")]
    Runtime {
        message: String,
        call_frames: Vec<RuntimeCallFrame>,
    },
}

impl ScriptErrorDiagnostic {
    pub fn message(&self) -> &str {
        match self {
            Self::Parse(_, _, message) | Self::Runtime { message, .. } => message,
        }
    }

    pub fn call_frames(&self) -> &[RuntimeCallFrame] {
        match self {
            Self::Parse(..) => &[],
            Self::Runtime { call_frames, .. } => call_frames,
        }
    }
}

impl TryFrom<ScriptError> for ScriptErrorDiagnostic {
    type Error = ScriptError;

    fn try_from(error: ScriptError) -> Result<Self, Self::Error> {
        error.into_diagnostic()
    }
}

impl From<ParseError> for ScriptErrorDiagnostic {
    fn from(error: ParseError) -> Self {
        Self::Parse(error.line, error.column, error.message)
    }
}

/// A control transfer raised by a host callback. This is deliberately kept
/// out of the public error display: a host request is an execution result,
/// while every other `RuntimeError` remains an ordinary script failure.
pub(crate) enum RuntimeControl {
    HostContinuation {
        // The child and the parent suspension share this request while a
        // native continuation is parked between its phases.  A request is
        // opaque to the VM, so sharing the erased value avoids moving it out
        // of the child merely to expose it at the outer host boundary.
        request: Rc<dyn Any>,
        resume_value: crate::value::Value,
        continuation: Option<Box<dyn Any>>,
    },
}

#[derive(Error)]
#[error("{message}")]
pub struct RuntimeError {
    message: String,
    call_frames: Vec<RuntimeCallFrame>,
    pub(crate) control: Option<Box<RuntimeControl>>,
    /// Native parameter slots that were live while the callback raised its
    /// continuation. The script frame resumes without this native frame, but
    /// an embedding engine's synchronous inline work must account for it.
    pub(crate) host_parameter_slots: Option<NonZeroUsize>,
}

impl fmt::Debug for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeError")
            .field("message", &self.message)
            .field("call_frames", &self.call_frames)
            .field("has_control", &self.control.is_some())
            .field("host_parameter_slots", &self.host_parameter_slots)
            .finish()
    }
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
            control: None,
            host_parameter_slots: None,
        }
    }

    /// Yield from a host callback while retaining the VM's current frame.
    /// `resume_value` is the value that the native call returns when the
    /// embedding engine has committed the request and resumes the frame.
    pub fn host_continuation<T: Any>(request: T, resume_value: crate::value::Value) -> Self {
        Self {
            message: "script execution suspended by host".to_string(),
            call_frames: crate::vm::snapshot_active_runtime_frames(),
            control: Some(Box::new(RuntimeControl::HostContinuation {
                request: Rc::new(request),
                resume_value,
                continuation: None,
            })),
            host_parameter_slots: None,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn call_frames(&self) -> &[RuntimeCallFrame] {
        &self.call_frames
    }

    fn into_diagnostic(self) -> Result<ScriptErrorDiagnostic, Self> {
        if self.control.is_some() {
            return Err(self);
        }
        Ok(ScriptErrorDiagnostic::Runtime {
            message: self.message,
            call_frames: self.call_frames,
        })
    }

    pub(crate) fn take_control(&mut self) -> Option<RuntimeControl> {
        self.control.take().map(|control| *control)
    }

    pub(crate) fn with_host_parameter_slots(mut self, slots: usize) -> Self {
        if self.control.is_some() {
            // Store slots plus one so zero-slot native frames have a value
            // while the option remains niche-optimized in RuntimeError.
            self.host_parameter_slots = NonZeroUsize::new(slots.saturating_add(1));
        }
        self
    }

    pub(crate) fn take_host_parameter_slots(&mut self) -> Option<usize> {
        self.host_parameter_slots
            .take()
            .map(|slots| slots.get() - 1)
    }

    pub(crate) fn with_control(mut self, control: RuntimeControl) -> Self {
        self.control = Some(Box::new(control));
        self
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
