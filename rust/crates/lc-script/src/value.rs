use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Int(i32),
    Bool(bool),
    String(String),
    C4Id(String),
    Array(Vec<Value>),
    Proplist(HashMap<String, Value>),
    Nil,
}

impl Value {
    /// C4Script truthiness, matching C++ `C4Value::operator bool` (C4Value.h:185
    /// → `C4V_Data::operator bool`, :76): raw-nonzero on the `Data` union. For
    /// strings/arrays/proplists that is a *pointer*, so a non-nil one is truthy
    /// even when empty; only nil and integer/bool zero are falsy.
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::String(_) => true,
            Value::C4Id(id) => !id.is_empty(),
            Value::Array(_) => true,
            Value::Proplist(_) => true,
            Value::Nil => false,
        }
    }

    /// Mirror C++ `C4Value::_getInt()` (C4Value.h:170) for the value types with
    /// a deterministic integer representation. C++ stores Int and Bool in the
    /// same `Data.Int` slot (bool is 0/1) and nil's `Data` is 0, so the integer
    /// operators — which read operands via `_getInt()` under
    /// `CheckOpPars<C4V_Any, ...>` (no conversion, C4AulExec.cpp) — treat nil,
    /// false, and true as 0, 0, and 1. String/Array/Proplist have no
    /// deterministic integer value in C++ (their `Data` is a pointer), so they
    /// return `None` and the caller keeps its type-error behavior.
    pub fn as_c4_int(&self) -> Option<i32> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(*b as i32),
            Value::Nil => Some(0),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Bool(_) => "bool",
            Value::String(_) => "string",
            Value::C4Id(_) => "id",
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
    C4Id(String),
    Nil,
}

impl From<Literal> for Value {
    fn from(literal: Literal) -> Self {
        match literal {
            Literal::Int(i) => Value::Int(i),
            Literal::Bool(b) => Value::Bool(b),
            Literal::String(s) => Value::String(s),
            Literal::C4Id(id) => Value::C4Id(id),
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
            Value::C4Id(id) => write!(f, "{id}"),
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
