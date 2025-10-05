use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Proplist(HashMap<String, Value>),
    Nil,
}

impl Value {
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::String(s) => !s.is_empty(),
            Value::Array(values) => !values.is_empty(),
            Value::Proplist(entries) => !entries.is_empty(),
            Value::Nil => false,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Bool(_) => "bool",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Proplist(_) => "proplist",
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
            Value::Array(values) => {
                let mut first = true;
                write!(f, "[")?;
                for value in values {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{value}")?;
                }
                write!(f, "]")
            }
            Value::Proplist(entries) => {
                let mut items: Vec<_> = entries.iter().collect();
                items.sort_by(|a, b| a.0.cmp(b.0));
                let mut first = true;
                write!(f, "{{")?;
                for (key, value) in items {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{key} = {value}")?;
                }
                write!(f, "}}")
            }
            Value::Nil => write!(f, "nil"),
        }
    }
}
