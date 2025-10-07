use std::convert::TryFrom;

use lc_script::{Engine as ScriptEngine, RuntimeError, Value};

pub fn register_host_functions(script: &mut ScriptEngine) {
    script.register_host_function("GetEffect", get_effect);
}

fn get_effect(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() < 2 {
        return Err(RuntimeError::new(
            "GetEffect expects at least 2 arguments: name and state",
        ));
    }

    let name_filter = match &args[0] {
        Value::String(name) if !name.is_empty() => Some(name.as_str()),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected string or nil for name, got {}",
                other.type_name()
            )))
        }
    };

    let state = match &args[1] {
        Value::Proplist(map) => map,
        Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected proplist or nil for state, got {}",
                other.type_name()
            )))
        }
    };

    let effects_value = match state.get("effects") {
        Some(value) => value,
        None => return Ok(Value::Nil),
    };

    let effects = match effects_value {
        Value::Array(entries) => entries,
        Value::Nil => return Ok(Value::Nil),
        other => {
            return Err(RuntimeError::new(format!(
                "GetEffect: state.effects must be an array, got {}",
                other.type_name()
            )))
        }
    };

    let desired_index = match args.get(2) {
        Some(Value::Int(value)) if *value >= 0 => *value as usize,
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffect: index argument must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for index, got {}",
                other.type_name()
            )))
        }
    };

    let query = match args.get(3) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for query, got {}",
                other.type_name()
            )))
        }
    };

    let max_priority = match args.get(4) {
        Some(Value::Int(value)) if *value >= 0 => Some(*value),
        Some(Value::Int(_)) => {
            return Err(RuntimeError::new(
                "GetEffect: max priority must be >= 0 when provided",
            ))
        }
        Some(Value::Nil) | None => None,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetEffect: expected int for max priority, got {}",
                other.type_name()
            )))
        }
    };

    let mut match_index = 0;
    for effect in effects {
        let map = match effect {
            Value::Proplist(map) => map,
            _ => continue,
        };

        let effect_name = match map.get("name") {
            Some(Value::String(name)) => name,
            _ => continue,
        };

        if let Some(filter) = name_filter {
            if effect_name != filter {
                continue;
            }
        }

        let priority = match map.get("priority") {
            Some(Value::Int(value)) => *value,
            _ => 0,
        };

        if let Some(limit) = max_priority {
            if priority.abs() > limit {
                continue;
            }
        }

        if match_index == desired_index {
            return Ok(match query {
                0 => {
                    let identifier = match_index.saturating_add(1);
                    let id = i32::try_from(identifier).unwrap_or(i32::MAX);
                    Value::Int(id)
                }
                1 => Value::String(effect_name.clone()),
                2 => Value::Int(priority),
                3 => map
                    .get("interval")
                    .and_then(|value| match value {
                        Value::Int(interval) => Some(*interval),
                        _ => None,
                    })
                    .map(Value::Int)
                    .unwrap_or(Value::Nil),
                6 => map
                    .get("timer")
                    .and_then(|value| match value {
                        Value::Int(timer) => Some(*timer),
                        _ => None,
                    })
                    .map(Value::Int)
                    .unwrap_or(Value::Nil),
                _ => Value::Proplist(map.clone()),
            });
        }

        match_index += 1;
    }

    Ok(Value::Nil)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_effect_rejects_invalid_args() {
        let error = get_effect(&[]).expect_err("missing args should error");
        assert!(error.message().contains("GetEffect expects"));

        let error =
            get_effect(&[Value::Int(0), Value::Nil]).expect_err("invalid name type should error");
        assert!(error.message().contains("expected string or nil for name"));

        let error =
            get_effect(&[Value::Nil, Value::Int(7)]).expect_err("invalid state type should error");
        assert!(error
            .message()
            .contains("expected proplist or nil for state"));
    }
}
