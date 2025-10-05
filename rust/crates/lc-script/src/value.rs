use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Bool(bool),
    String(String),
    Nil,
}

impl Value {
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::String(s) => !s.is_empty(),
            Value::Nil => false,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Bool(_) => "bool",
            Value::String(_) => "string",
            Value::Nil => "nil",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(i32),
    Bool(bool),
    String(String),
    Nil,
}

impl From<Literal> for Value {
    fn from(literal: Literal) -> Self {
        match literal {
            Literal::Int(i) => Value::Int(i),
            Literal::Bool(b) => Value::Bool(b),
            Literal::String(s) => Value::String(s),
            Literal::Nil => Value::Nil,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Nil => write!(f, "nil"),
        }
    }
}
