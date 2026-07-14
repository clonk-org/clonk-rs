use thiserror::Error;

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
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct RuntimeError {
    message: String,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
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
