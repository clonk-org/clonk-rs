use super::*;

pub(crate) const C4V_ANY: i32 = 0;
pub(crate) const C4V_INT: i32 = 1;
pub(crate) const C4V_BOOL: i32 = 2;
const C4V_ID: i32 = 3;
const C4V_OBJECT: i32 = 4;
pub(crate) const C4V_STRING: i32 = 5;
pub(crate) const C4V_ARRAY: i32 = 6;
pub(crate) const C4V_MAP: i32 = 7;
pub(crate) const LEGACY_MAX_ARRAY_SIZE: i32 = 1_000_000;

pub(crate) fn truncate_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

pub(crate) fn invert_rgba_alpha(color: u32) -> u32 {
    let alpha = (color >> 24) & 0xff;
    let rgb = color & 0x00ff_ffff;
    ((255 - alpha) << 24) | rgb
}

pub(crate) fn ensure_single_flag(flags: u32, mask: u32, error: &str) -> Result<(), RuntimeError> {
    let masked = flags & mask;
    if masked != 0 && (masked & (masked - 1)) != 0 {
        return Err(RuntimeError::new(error));
    }
    Ok(())
}

pub fn object_reference_value(id: ObjectId) -> Value {
    Value::Object(id.as_u64())
}

pub(crate) fn object_id_from_value(value: &Value) -> Option<ObjectId> {
    match value {
        Value::Object(id) if *id != 0 => Some(ObjectId::new(*id)),
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) if *id > 0 => Some(ObjectId::new(*id as u64)),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn parse_object_reference_argument(
    value: &Value,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    match value {
        Value::Object(_) | Value::Proplist(_) => Ok(object_id_from_value(value)),
        Value::Nil => Ok(None),
        Value::Int(id) if *id == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "{}: expected object, proplist, nil, or 0 for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

/// Apply native `C4Object *` parameter conversion. Before strict 3, C4Aul
/// eagerly resets raw-falsy values to nil; strict-3 callers retain typed
/// integer/bool zero and therefore fail the object typecheck. Canonical null
/// object and raw-zero C4ID values remain nil in either mode.
pub(crate) fn parse_native_object_argument(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    let value = value.unwrap_or(&Value::Nil);
    let canonical_nil = matches!(value, Value::Nil | Value::Object(0))
        || matches!(value, Value::C4Id(id) if cast_c4id_payload(id) == 0);
    let eager_falsy_conversion = !matches!(
        clonk_script::caller_origin_strictness(),
        clonk_script::HostCallerStrictness::Strict(level) if level >= 3
    );
    if canonical_nil || (eager_falsy_conversion && !value.as_bool()) {
        return Ok(None);
    }
    match value {
        Value::Object(_) | Value::Proplist(_) => Ok(object_id_from_value(value)),
        other => Err(RuntimeError::new(format!(
            "{function}: expected object for {parameter}, got {}",
            other.type_name()
        ))),
    }
}

pub(crate) fn consume_optional_object_reference_argument(
    args: &[Value],
    index: &mut usize,
    function: &str,
    parameter: &str,
) -> Result<Option<ObjectId>, RuntimeError> {
    let Some(value) = args.get(*index) else {
        return Ok(None);
    };
    if !matches!(
        value,
        Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
    ) {
        return Ok(None);
    }
    let object_id = parse_object_reference_argument(value, function, parameter)?;
    *index += 1;
    Ok(object_id)
}

pub(crate) fn value_to_i32(
    value: &Value,
    function: &str,
    parameter: &str,
) -> Result<i32, RuntimeError> {
    match value {
        Value::Int(int) => Ok(*int),
        // Unfilled parameter slots are nil and convert to 0; bools convert
        // directly (C4AulExec.cpp:1364-1396 CheckConvertFunctionParameters,
        // C4Value.cpp FnCnvGuess / Bool->Int CnvOK).
        Value::Nil => Ok(0),
        Value::Bool(flag) => Ok(i32::from(*flag)),
        Value::RawBool(raw) => Ok(*raw as u32 as i32),
        other => Err(RuntimeError::new(format!(
            "{}: expected integer for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

/// `StringBitEval` (C4Script.cpp:209-216): bytes other than underscore and
/// space set their original position in the 32-bit mask. Tutorial control
/// strings use the defined 30-bit range consumed by DrawPlayerControls.
pub(crate) fn string_bit_eval(value: &str) -> i32 {
    clonk_script::c4_string_bytes(value)
        .into_iter()
        .enumerate()
        .filter(|(_, byte)| !matches!(byte, b'_' | b' '))
        .filter_map(|(position, _)| 1u32.checked_shl(position as u32))
        .fold(0u32, u32::wrapping_add) as i32
}

/// C4IdText (C4Id.cpp:26-45) over a script value: C4ID_None -> "NONE",
/// numerical ids 0..9999 -> "%04u", literal ids stay as-is.
pub(crate) fn c4id_text_of(value: &Value) -> String {
    match value {
        Value::C4Id(id) => clonk_script::c4_id_text(id),
        Value::String(id) if !id.is_empty() => id.as_ref().to_owned(),
        Value::Int(raw) => c4id_to_definition(*raw).unwrap_or_else(|| "NONE".to_string()),
        _ => "NONE".to_string(),
    }
}

/// Apply native `C4String *` conversion without collapsing an explicit empty
/// string into a null pointer. Pre-strict-3 callers eagerly reset raw-falsy
/// values to nil; strict-3 callers retain their type, so typed `0`/`false`
/// fail conversion while nil remains a valid null string pointer.
pub(crate) fn parse_native_c4_string_argument(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<String>, RuntimeError> {
    let value = value.unwrap_or(&Value::Nil);
    let eager_falsy_conversion = !matches!(
        clonk_script::caller_origin_strictness(),
        clonk_script::HostCallerStrictness::Strict(level) if level >= 3
    );
    if eager_falsy_conversion && !value.as_bool() {
        return Ok(None);
    }
    match value {
        Value::Nil => Ok(None),
        Value::String(text) => Ok(Some(text.as_ref().to_owned())),
        other => Err(RuntimeError::new(format!(
            "{function}: expected string for {parameter}, got {}",
            other.type_name()
        ))),
    }
}

pub(crate) fn value_to_bool(
    value: &Value,
    _function: &str,
    _parameter: &str,
) -> Result<bool, RuntimeError> {
    Ok(match value {
        Value::Bool(flag) => *flag,
        Value::RawBool(raw) => (*raw as u32 as i32) != 0,
        Value::Int(int) => *int != 0,
        Value::Nil => false,
        Value::C4Id(id) => cast_c4id_payload(id) != 0,
        Value::Object(id) => *id != 0,
        // C++ tests the non-null pointer stored in the C4Value union, so
        // allocated values stay truthy even when their contents are empty.
        Value::String(_) | Value::Array(_) | Value::Proplist(_) => true,
    })
}

pub(crate) fn parse_optional_i32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<i32>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        Some(Value::Int(int)) => Ok(Some(*int)),
        Some(Value::Bool(flag)) => Ok(Some(i32::from(*flag))),
        Some(Value::RawBool(raw)) => Ok(Some(*raw as u32 as i32)),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected integer for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

/// Parse a VM-prepared native `std::optional<C4ValueInt>` parameter. The call
/// boundary has already applied legacy eager-nil conversion. At this point an
/// `Any` nil is absent, while every remaining Int/Bool payload is present --
/// including a zero extracted from a non-nil raw Bool value.
pub(crate) fn parse_native_optional_i32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<i32>, RuntimeError> {
    parse_optional_i32(value, function, parameter)
}

pub(crate) fn parse_optional_u32(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<u32>, RuntimeError> {
    Ok(parse_optional_i32(value, function, parameter)?.map(|raw| raw as u32))
}

pub(crate) fn parse_optional_string_value(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<clonk_script::C4StringValue>, RuntimeError> {
    match value {
        None => Ok(None),
        Some(Value::Nil) => Ok(None),
        // Falsy parameters reset to nil before the typecheck
        // (C4AulExec.cpp:1364-1396): a literal 0/false in a string slot is
        // a null string, not a conversion error (GoldRush passes 0 for the
        // FindObjectOwner action).
        Some(Value::Int(0)) | Some(Value::Bool(false)) | Some(Value::RawBool(0)) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected string for {}, got {}",
            function,
            parameter,
            other.type_name()
        ))),
    }
}

pub(crate) fn parse_optional_string(
    value: Option<&Value>,
    function: &str,
    parameter: &str,
) -> Result<Option<String>, RuntimeError> {
    Ok(parse_optional_string_value(value, function, parameter)?
        .map(|text| text.as_ref().to_owned()))
}

fn c4id_to_definition(id: i32) -> Option<String> {
    if id == 0 {
        return None;
    }
    if (0..=9999).contains(&id) {
        return Some(format!("{id:04}"));
    }
    let raw = id as u32;
    let mut bytes = [0u8; 4];
    bytes[0] = (raw & 0x0000_00FF) as u8;
    bytes[1] = ((raw & 0x0000_FF00) >> 8) as u8;
    bytes[2] = ((raw & 0x00FF_0000) >> 16) as u8;
    bytes[3] = ((raw & 0xFF00_0000) >> 24) as u8;
    let end = bytes
        .iter()
        .rposition(|&b| b != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    (end != 0).then(|| clonk_script::c4_string_from_bytes(&bytes[..end]))
}

/// Apply `CheckConvertFunctionParameters` for a native `C4ID` slot and
/// return the canonical raw-ID storage spelling used by the Rust engine.
///
/// C++ accepts an existing C4ID, nil/raw zero, or an integer in
/// `0..=9999` (`FnCnvInt2Id`). Before strict 3, every raw-falsy argument is
/// eagerly reset to nil before that conversion; strict-3 callers retain the
/// original type and therefore reject false/null-object values of the wrong
/// type. Strings never convert to C4ID.
pub(crate) fn parse_native_c4id_argument(
    value: Option<&Value>,
    function: &str,
) -> Result<Option<String>, RuntimeError> {
    let value = value.unwrap_or(&Value::Nil);
    let eager_falsy_conversion = !matches!(
        clonk_script::caller_origin_strictness(),
        clonk_script::HostCallerStrictness::Strict(level) if level >= 3
    );
    if eager_falsy_conversion && !value.as_bool() {
        return Ok(None);
    }

    match value {
        Value::Nil | Value::Int(0) => Ok(None),
        Value::C4Id(id) => Ok(definition_id_for_c4id(id)),
        Value::Int(id @ 1..=9999) => Ok(Some(format!("{id:04}"))),
        other => Err(RuntimeError::new(format!(
            "{function}: expected C4ID, got {}",
            other.type_name()
        ))),
    }
}

pub(crate) fn squared_distance(position: Vector2, x: i32, y: i32) -> i64 {
    let dx = position.x as i64 - x as i64;
    let dy = position.y as i64 - y as i64;
    dx * dx + dy * dy
}

pub(crate) fn normalise_precision(value: i32) -> i32 {
    if value == 0 {
        DEFAULT_VELOCITY_PRECISION
    } else {
        value
    }
}

/// Strips the failsafe marker(s) from a call-family function name:
/// `GetSFunc` strips one leading '~' (C4Aul.cpp:314) and its name-only
/// overload strips a second (C4Aul.cpp:350), so `"~~Name"` resolves to
/// `Name`. Failsafe only changes logging — a miss returns C4VNull either
/// way, so the marker carries no other semantics here.
pub(crate) fn strip_failsafe(name: &str) -> &str {
    let once = name.strip_prefix('~').unwrap_or(name);
    once.strip_prefix('~').unwrap_or(once)
}

fn value_to_data_string_with_context(value: &Value, context: Option<&EffectHostContext>) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Int(i) => i.to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::RawBool(raw) => (*raw != 0).to_string(),
        Value::String(text) => format!("\"{text}\""),
        Value::C4Id(id) => clonk_script::c4_id_text(id),
        Value::Object(id) => {
            let target = ObjectId::new(*id);
            let Some((context, object)) = context
                .and_then(|context| {
                    context
                        .get_world_object(target)
                        .map(|object| (context, object))
                })
                .filter(|(_, object)| object.status() != ObjectStatus::Deleted)
            else {
                return id.to_string();
            };
            let name = context
                .object_effective_name(target)
                .unwrap_or_else(|| object.definition_id().to_string());
            let rendered = format!("{name} #{id}");
            if object.status() == ObjectStatus::Normal {
                rendered
            } else {
                format!("{{{rendered}}}")
            }
        }
        Value::Array(values) => {
            let inner = values
                .iter()
                .map(|value| value_to_data_string_with_context(value, context))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Value::Proplist(entries) => {
            if entries.is_empty() {
                "{}".to_string()
            } else {
                let inner = entries
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{} = {}",
                            value_to_data_string_with_context(key, context),
                            value_to_data_string_with_context(value, context)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {inner} }}")
            }
        }
    }
}

/// C4Value::GetDataString for a live object used by clonk-script diagnostic
/// frames. Deleted/unknown pointers fall back to their raw object number in
/// clonk-script, while inactive objects retain C++'s brace notation.
pub(crate) fn diagnostic_object_data_string(id: u64) -> Option<(String, Option<String>)> {
    let target = ObjectId::new(id);
    HOST_CONTEXT.with(|cell| {
        // RuntimeError snapshots the active diagnostic stack at the error
        // site. A native may still hold this context mutably at that point;
        // diagnostic rendering must fall back instead of re-borrowing/panicking.
        let borrow = cell.try_borrow().ok()?;
        let context = borrow.as_ref()?;
        let scoped_object = context.object_scope(target);
        let status = scoped_object
            .map(|scope| {
                if scope.destroy {
                    ObjectStatus::Deleted
                } else {
                    scope.status()
                }
            })
            .or_else(|| {
                context
                    .get_world_object(target)
                    .map(|object| object.status())
            })?;
        if status == ObjectStatus::Deleted && scoped_object.is_none() {
            return None;
        }
        let definition = context.object_effective_definition_id(target);
        let name = context
            .object_effective_name(target)
            .or_else(|| definition.clone())?;
        let script_name = definition
            .as_deref()
            .and_then(|definition| context.world.definition_script(definition))
            .map(|script| script.script_name().to_owned());
        let display = format!("{name} #{id}");
        let display = if status == ObjectStatus::Normal {
            display
        } else {
            format!("{{{display}}}")
        };
        Some((display, script_name))
    })
}

pub(crate) fn value_to_data_string(value: &Value) -> String {
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        value_to_data_string_with_context(value, borrow.as_ref())
    })
}

fn format_int_value(value: &Value, function: &str) -> Result<i32, RuntimeError> {
    match value {
        Value::Int(i) => Ok(*i),
        Value::Bool(flag) => Ok(if *flag { 1 } else { 0 }),
        Value::RawBool(raw) => Ok(*raw as u32 as i32),
        Value::Nil => Ok(0),
        other => Err(RuntimeError::new(format!(
            "{function}: expected integer-compatible value for format placeholder, got {}",
            other.type_name()
        ))),
    }
}

fn render_c4id(raw: i32) -> String {
    clonk_script::c4_id_text(&clonk_script::c4_id_from_raw(raw as u32 as usize))
}

fn format_c4id_string(value: &Value, function: &str) -> Result<String, RuntimeError> {
    match value {
        Value::Int(raw) => Ok(render_c4id(*raw)),
        // A literal id value (FnFormat %i via C4VID).
        Value::C4Id(id) if !id.is_empty() => Ok(render_c4id(cast_c4id_payload(id) as i32)),
        Value::C4Id(_) => Ok("NONE".to_string()),
        Value::String(text) if !text.is_empty() => Ok(text.as_ref().to_owned()),
        Value::String(_) | Value::Nil => Ok("NONE".to_string()),
        other => Err(RuntimeError::new(format!(
            "{function}: expected C4ID-compatible value for format placeholder, got {}",
            other.type_name()
        ))),
    }
}

fn format_decimal(
    value: i32,
    width: Option<usize>,
    precision: Option<usize>,
    zero_pad: bool,
) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = if value < 0 {
        -(i64::from(value))
    } else {
        i64::from(value)
    };
    let mut digits = if precision == Some(0) && magnitude == 0 {
        String::new()
    } else {
        magnitude.abs().to_string()
    };
    if let Some(prec) = precision {
        if digits.len() < prec {
            let pad = "0".repeat(prec - digits.len());
            digits = format!("{pad}{digits}");
        }
    }
    let mut result = if sign.is_empty() {
        digits.clone()
    } else {
        format!("{sign}{digits}")
    };
    if let Some(width) = width {
        if result.len() < width {
            let pad_len = width - result.len();
            if zero_pad && precision.is_none() {
                let pad = "0".repeat(pad_len);
                if sign.is_empty() {
                    result = format!("{pad}{digits}");
                } else {
                    result = format!("-{pad}{digits}");
                }
            } else {
                let pad = " ".repeat(pad_len);
                result = format!("{pad}{result}");
            }
        }
    }
    result
}

fn format_hex(
    value: i32,
    width: Option<usize>,
    precision: Option<usize>,
    zero_pad: bool,
    uppercase: bool,
) -> String {
    let raw = value as u32;
    let mut digits = if precision == Some(0) && raw == 0 {
        String::new()
    } else if uppercase {
        format!("{raw:X}")
    } else {
        format!("{raw:x}")
    };
    if let Some(prec) = precision {
        if digits.len() < prec {
            let pad = "0".repeat(prec - digits.len());
            digits = format!("{pad}{digits}");
        }
    }
    let mut result = digits.clone();
    if let Some(width) = width {
        if result.len() < width {
            let pad_len = width - result.len();
            if zero_pad && precision.is_none() {
                let pad = "0".repeat(pad_len);
                result = format!("{pad}{digits}");
            } else {
                let pad = " ".repeat(pad_len);
                result = format!("{pad}{result}");
            }
        }
    }
    result
}

fn truncate_to_precision(text: &str, precision: Option<usize>) -> String {
    match precision {
        Some(limit) => {
            let bytes = clonk_script::c4_string_bytes(text);
            if bytes.len() <= limit {
                text.to_string()
            } else {
                clonk_script::c4_string_from_bytes(&bytes[..limit])
            }
        }
        None => text.to_string(),
    }
}

fn pad_left(text: &str, width: Option<usize>) -> String {
    match width {
        Some(width) => {
            let len = clonk_script::c4_string_byte_len(text);
            if len >= width {
                text.to_string()
            } else {
                let pad = " ".repeat(width - len);
                format!("{pad}{text}")
            }
        }
        None => text.to_string(),
    }
}

pub(crate) fn format_script_string_with_context(
    function: &str,
    format_str: &str,
    params: &[Value],
    context: Option<&EffectHostContext>,
) -> Result<String, RuntimeError> {
    let mut output = String::new();
    let mut chars = format_str.chars().peekable();
    let mut arg_index = 0usize;
    let called_with_strict_nil = matches!(
        clonk_script::caller_origin_strictness(),
        clonk_script::HostCallerStrictness::Strict(level) if level >= 3
    );

    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }

        if matches!(chars.peek(), Some('%')) {
            chars.next();
            output.push('%');
            continue;
        }

        let mut zero_pad = false;
        let mut width_value: Option<usize> = None;
        let mut first_width_digit: Option<char> = None;
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                if first_width_digit.is_none() {
                    first_width_digit = Some(c);
                }
                width_value =
                    Some(width_value.unwrap_or(0) * 10 + c.to_digit(10).unwrap() as usize);
                chars.next();
            } else {
                break;
            }
        }
        if matches!(first_width_digit, Some('0')) && width_value.unwrap_or(0) > 0 {
            zero_pad = true;
        }

        let mut precision: Option<usize> = None;
        if matches!(chars.peek(), Some('.')) {
            chars.next();
            let mut digits = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            precision = Some(digits.parse::<usize>().unwrap_or(0));
        }

        let spec = match chars.next() {
            Some(c) => c,
            None => {
                output.push('%');
                break;
            }
        };

        match spec {
            'd' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let value = format_int_value(param, function)?;
                output.push_str(&format_decimal(value, width_value, precision, zero_pad));
            }
            'x' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let value = format_int_value(param, function)?;
                output.push_str(&format_hex(value, width_value, precision, zero_pad, false));
            }
            'X' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let value = format_int_value(param, function)?;
                output.push_str(&format_hex(value, width_value, precision, zero_pad, true));
            }
            'c' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let byte = format_int_value(param, function)? as u8;
                let text = clonk_script::c4_string_from_bytes(&[byte]);
                output.push_str(&pad_left(&text, width_value));
            }
            'i' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let text = format_c4id_string(param, function)?;
                let truncated = truncate_to_precision(&text, precision);
                output.push_str(&pad_left(&truncated, width_value));
            }
            's' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                arg_index += 1;
                let raw = match param {
                    Value::String(text) => text.as_ref().to_owned(),
                    Value::Nil => "(null)".to_string(),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "{function}: string format placeholder requires string argument, got {}",
                            other.type_name()
                        )));
                    }
                };
                let truncated = truncate_to_precision(&raw, precision);
                output.push_str(&pad_left(&truncated, width_value));
            }
            'v' => {
                let param = params.get(arg_index).ok_or_else(|| {
                    RuntimeError::new(format!("{function}: format placeholder without parameter"))
                })?;
                let text = if !param.as_bool() && !called_with_strict_nil {
                    "0".to_string()
                } else {
                    arg_index += 1;
                    value_to_data_string_with_context(param, context)
                };
                output.push_str(&text);
            }
            '%' => output.push('%'),
            other => {
                output.push('%');
                output.push(other);
            }
        }
    }

    let bytes = clonk_script::c4_string_bytes(&output);
    match bytes.iter().position(|byte| *byte == 0) {
        Some(nul) => Ok(clonk_script::c4_string_from_bytes(&bytes[..nul])),
        None => Ok(clonk_script::c4_string_from_bytes(&bytes)),
    }
}

pub(crate) fn format_script_string(
    function: &str,
    format_str: &str,
    params: &[Value],
) -> Result<String, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        format_script_string_with_context(function, format_str, params, borrow.as_ref())
    })
}

pub(crate) fn c4_char_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - (b'a' - b'A'),
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

pub(crate) fn c4_bytes_equal_no_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| c4_char_capital(*left) == c4_char_capital(*right))
}

pub(crate) fn get_keys(args: &[Value]) -> Result<Value, RuntimeError> {
    let map = match args.first() {
        Some(Value::Proplist(map)) => map,
        Some(Value::Nil) | None => {
            return Err(RuntimeError::new("GetKeys(): map expected, got 0"));
        }
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetKeys(): map expected, got {}",
                other.type_name()
            )));
        }
    };

    Ok(Value::Array(map.keys().cloned().collect()))
}

pub(crate) fn get_values(args: &[Value]) -> Result<Value, RuntimeError> {
    let map = match args.first() {
        Some(Value::Proplist(map)) => map,
        Some(Value::Nil) | None => {
            return Err(RuntimeError::new("GetValues(): map expected, got 0"));
        }
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "GetValues(): map expected, got {}",
                other.type_name()
            )));
        }
    };

    Ok(Value::Array(map.values().cloned().collect()))
}

pub(crate) fn get_type(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("GetType expects 1 argument: value"));
    }

    // FnGetType (C4Script.cpp:3795-3799): a falsy value reports C4V_Any
    // only when there is a script caller below #strict 3. The caller's
    // declaring script supplies CalledWithStrictNil; direct native entry
    // has no Caller and therefore keeps the concrete type.
    let legacy_script_caller = match clonk_script::caller_origin_strictness() {
        clonk_script::HostCallerStrictness::NoCaller => false,
        clonk_script::HostCallerStrictness::NonStrict => true,
        clonk_script::HostCallerStrictness::Strict(level) => level < 3,
    };
    let value = &args[0];
    if legacy_script_caller && !value.as_bool() {
        return Ok(Value::Int(C4V_ANY));
    }

    let type_code = match value {
        Value::Int(_) => C4V_INT,
        Value::Bool(_) => C4V_BOOL,
        Value::RawBool(_) => C4V_BOOL,
        Value::String(_) => C4V_STRING,
        Value::C4Id(_) => C4V_ID,
        Value::Object(_) => C4V_OBJECT,
        Value::Array(_) => C4V_ARRAY,
        Value::Proplist(_) => C4V_MAP,
        Value::Nil => C4V_ANY,
    };

    Ok(Value::Int(type_code))
}

/// `C4AulDefCastFunc<C4V_Any, C4V_Any>` (C4Script.cpp:6184-6194,
/// 7043-7046). AddMenuItem serializes an untyped nil parameter as
/// `CastAny(0)` so its generated command can reconstruct C4V_Any/null.
pub(crate) fn cast_any(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new("CastAny expects at most 1 argument"));
    }
    Ok(match args.first().cloned().unwrap_or(Value::Nil) {
        Value::Int(0) | Value::Bool(false) | Value::RawBool(0) | Value::Nil => Value::Nil,
        value => value,
    })
}

fn cast_arg(args: &[Value]) -> &Value {
    args.first().unwrap_or(&Value::Nil)
}

pub(crate) fn cast_c4id_payload(id: &str) -> usize {
    clonk_script::c4_id_raw(id)
}

pub(crate) fn definition_id_for_c4id(id: &str) -> Option<String> {
    let raw = cast_c4id_payload(id);
    (raw != 0).then(|| clonk_script::c4_id_from_raw(raw))
}

pub(crate) fn render_cast_c4id(raw: i32) -> String {
    clonk_script::c4_id_from_raw(raw as u32 as usize)
}

fn cast_stable_data_raw(value: &Value, function: &str) -> Result<usize, RuntimeError> {
    match value {
        Value::Int(raw) => Ok(*raw as u32 as usize),
        Value::Bool(flag) => Ok(usize::from(*flag)),
        Value::RawBool(raw) => Ok(*raw),
        Value::Nil => Ok(0),
        Value::C4Id(id) => Ok(cast_c4id_payload(id)),
        Value::Object(_) | Value::String(_) | Value::Array(_) | Value::Proplist(_) => {
            Err(RuntimeError::new(format!(
                "{function}: pointer payload cannot be represented as a deterministic integer"
            )))
        }
    }
}

fn cast_stable_raw_i32(value: &Value, function: &str) -> Result<i32, RuntimeError> {
    Ok(cast_stable_data_raw(value, function)? as u32 as i32)
}

/// `C4AulDefCastFunc<C4V_Any, C4V_Int>` (C4Script.cpp:6184-6195,
/// :7043): retain the stable raw payload and replace only its type tag.
pub(crate) fn cast_int(args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Int(cast_stable_raw_i32(cast_arg(args), "CastInt")?))
}

/// `C4AulDefCastFunc<C4V_Any, C4V_Bool>`: pointer-backed values are nonzero
/// in C++, while the scalar variants have deterministic raw payloads here.
pub(crate) fn cast_bool(args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(match cast_arg(args) {
        Value::Int(raw) => Value::from_c4_bool_raw(*raw),
        value @ (Value::Bool(_) | Value::RawBool(_)) => value.clone(),
        Value::Nil => Value::Bool(false),
        Value::C4Id(id) => Value::from_c4_bool_data_raw(cast_c4id_payload(id)),
        Value::Object(id) => Value::Bool(*id != 0),
        Value::String(_) | Value::Array(_) | Value::Proplist(_) => Value::Bool(true),
    })
}

/// `C4AulDefCastFunc<C4V_Any, C4V_C4ID>`: zero becomes C4V_Any/null; numeric
/// ids keep their decimal representation and other payloads render as four
/// little-endian id bytes (C4Id.h:31-69).
pub(crate) fn cast_c4id(args: &[Value]) -> Result<Value, RuntimeError> {
    if let Value::C4Id(id) = cast_arg(args) {
        return Ok(if cast_c4id_payload(id) == 0 {
            Value::Nil
        } else {
            Value::C4Id(id.clone())
        });
    }
    let raw = cast_stable_data_raw(cast_arg(args), "CastC4ID")?;
    Ok(if raw == 0 {
        Value::Nil
    } else {
        Value::C4Id(clonk_script::c4_id_from_raw(raw))
    })
}

pub(crate) fn create_array(args: &[Value]) -> Result<Value, RuntimeError> {
    // Unfilled C4Aul parameters are nil and C4ValueInt converts nil to zero,
    // so bare CreateArray() constructs an empty array (FnCreateArray,
    // C4Script.cpp:3807-3810).
    let size = value_to_i32(args.first().unwrap_or(&Value::Nil), "CreateArray", "size")?;
    if !(0..=LEGACY_MAX_ARRAY_SIZE).contains(&size) {
        return Err(RuntimeError::new(format!(
            "CreateArray: invalid array size ({size})"
        )));
    }

    let values = vec![Value::Nil; size as usize];
    Ok(Value::Array(values))
}

/// FnIsRef (C4Script.cpp:3790-3793) is registered with a C4V_Any parameter.
/// Native parameter conversion dereferences C4V_pC4Value before the body,
/// so every script-visible argument reaches IsRef as a non-reference.
pub(crate) fn is_ref(_args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(false))
}

/// FnEqual (C4Script.cpp:3172-3175): compare the dereferenced C4Value Data
/// union without considering its type tag. The reference-aware host form with
/// no writable parameters retains pointer provenance for heap-backed values.
pub(crate) fn equal(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    let result = match (args.first(), args.get(1)) {
        (Some(left), Some(right)) => left.c4_equals(right, 0)?,
        (Some(value), None) | (None, Some(value)) => match value.read()? {
            Value::Nil
            | Value::Int(0)
            | Value::Bool(false)
            | Value::RawBool(0)
            | Value::Object(0) => true,
            Value::C4Id(id) => cast_c4id_payload(&id) == 0,
            Value::Int(_)
            | Value::Bool(true)
            | Value::RawBool(_)
            | Value::Object(_)
            | Value::String(_)
            | Value::Array(_)
            | Value::Proplist(_) => false,
        },
        (None, None) => true,
    };
    Ok(Value::Bool(result))
}

/// FnInc (C4Script.cpp:3770-3778): convert the referenced value to an
/// integer, add the optional difference, and write and return the result.
/// Failed target conversion returns nil without changing the reference.
pub(crate) fn inc_reference(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    let target = args.first().ok_or_else(|| {
        RuntimeError::new("call to \"Inc\" parameter 1: got \"nil\", but expected \"&\"!")
    })?;
    if !target.is_reference() {
        return Err(RuntimeError::new(format!(
            "call to \"Inc\" parameter 1: got \"{}\", but expected \"&\"!",
            target.read()?.type_name()
        )));
    }

    let Some(value) = target.read()?.as_c4_int() else {
        return Ok(Value::Nil);
    };
    let difference = args
        .get(1)
        .map(HostCallArg::read)
        .transpose()?
        .unwrap_or(Value::Nil)
        .as_c4_int()
        .unwrap_or(0);
    let result = Value::Int(value.wrapping_add(difference));
    if !target.write(result.clone())? {
        return Err(RuntimeError::new("Inc: variable reference expected"));
    }
    Ok(result)
}

/// FnSet (C4Script.cpp:3764-3768): assign through the first native
/// `C4Value *` parameter and return the value stored in that lvalue.
pub(crate) fn set_reference(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    let target = args.first().ok_or_else(|| {
        RuntimeError::new("call to \"Set\" parameter 1: got \"nil\", but expected \"&\"!")
    })?;
    if !target.is_reference() {
        return Err(RuntimeError::new(format!(
            "call to \"Set\" parameter 1: got \"{}\", but expected \"&\"!",
            target.read()?.type_name()
        )));
    }
    let value = args
        .get(1)
        .map(HostCallArg::read)
        .transpose()?
        .unwrap_or(Value::Nil);
    if !target.write(value)? {
        return Err(RuntimeError::new("Set: variable reference expected"));
    }
    target.read()
}

/// FnDec (C4Script.cpp:3780-3788): convert the referenced value to an
/// integer, subtract the optional difference, and write the integer result
/// through the original reference. Failed target conversion returns nil
/// without changing the referenced value; an unconvertible difference is 0.
pub(crate) fn dec_reference(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    let target = args.first().ok_or_else(|| {
        RuntimeError::new("call to \"Dec\" parameter 1: got \"nil\", but expected \"&\"!")
    })?;
    if !target.is_reference() {
        return Err(RuntimeError::new(format!(
            "call to \"Dec\" parameter 1: got \"{}\", but expected \"&\"!",
            target.read()?.type_name()
        )));
    }

    let Some(value) = target.read()?.as_c4_int() else {
        return Ok(Value::Nil);
    };
    let difference = args
        .get(1)
        .map(HostCallArg::read)
        .transpose()?
        .unwrap_or(Value::Nil)
        .as_c4_int()
        .unwrap_or(0);
    let result = Value::Int(value.wrapping_sub(difference));
    if !target.write(result.clone())? {
        return Err(RuntimeError::new("Dec: variable reference expected"));
    }
    Ok(result)
}

pub(crate) fn set_length(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    let target = args.first().ok_or_else(|| {
        RuntimeError::new("call to \"SetLength\" parameter 1: got \"nil\", but expected \"&\"!")
    })?;
    if !target.is_reference() {
        return Err(RuntimeError::new(format!(
            "call to \"SetLength\" parameter 1: got \"{}\", but expected \"&\"!",
            target.read()?.type_name()
        )));
    }

    let size_value = args
        .get(1)
        .map(HostCallArg::read)
        .transpose()?
        .unwrap_or(Value::Nil);
    let size = value_to_i32(&size_value, "SetLength", "size")?;
    if !(0..=LEGACY_MAX_ARRAY_SIZE).contains(&size) {
        return Err(RuntimeError::new(format!(
            "SetLength: invalid array size ({size})"
        )));
    }

    let Value::Array(mut values) = target.read()? else {
        return Err(RuntimeError::new("SetLength: array expected"));
    };
    values.resize(size as usize, Value::Nil);
    if !target.write(Value::Array(values))? {
        return Err(RuntimeError::new("SetLength: array reference expected"));
    }
    Ok(Value::Nil)
}

pub(crate) fn get_length(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("GetLength expects 1 argument: value"));
    }

    let value = &args[0];
    if !value.as_bool() {
        return Ok(Value::Nil);
    }

    match value {
        Value::String(text) => {
            let len = i32::try_from(clonk_script::c4_string_byte_len(text))
                .map_err(|_| RuntimeError::new("GetLength: string length exceeds i32 range"))?;
            Ok(Value::Int(len))
        }
        Value::Array(values) => {
            let len = i32::try_from(values.len())
                .map_err(|_| RuntimeError::new("GetLength: array length exceeds i32 range"))?;
            Ok(Value::Int(len))
        }
        Value::Proplist(entries) => {
            let len = i32::try_from(entries.len())
                .map_err(|_| RuntimeError::new("GetLength: map entry count exceeds i32 range"))?;
            Ok(Value::Int(len))
        }
        _ => Err(RuntimeError::new(
            "func \"GetLength\" par 0 cannot be converted to string or array or map",
        )),
    }
}

pub(crate) fn get_index_of(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    // Native calls normally always have a script caller. C++'s no-caller,
    // nonnil-array path dereferences a null Caller; strict3 is the conservative
    // defined fallback for engine-driven Rust calls.
    let strict_level = match clonk_script::caller_origin_strictness() {
        clonk_script::HostCallerStrictness::NoCaller => 3,
        clonk_script::HostCallerStrictness::NonStrict => 0,
        clonk_script::HostCallerStrictness::Strict(level) => level,
    };

    // C4Aul pads missing parameters with nil, and a nil array pointer is the
    // documented GetIndexOf(x, 0) fast path.
    let (Some(search), Some(array_arg)) = (args.first(), args.get(1)) else {
        return Ok(Value::Int(-1));
    };
    let array_value = array_arg.read()?;
    let Some(array) = array_arg.array_items()? else {
        let is_nil = matches!(&array_value, Value::Nil | Value::Object(0))
            || matches!(&array_value, Value::C4Id(id) if cast_c4id_payload(id) == 0);
        if is_nil || (strict_level < 3 && !array_value.as_bool()) {
            return Ok(Value::Int(-1));
        }
        return Err(RuntimeError::new(format!(
            "call to \"GetIndexOf\" parameter 2: got \"{}\", but expected \"array\"!",
            array_value.type_name()
        )));
    };

    for (index, entry) in array.iter().enumerate() {
        if search.c4_equals(entry, strict_level)? {
            let index = i32::try_from(index)
                .map_err(|_| RuntimeError::new("GetIndexOf: index exceeds i32 range"))?;
            return Ok(Value::Int(index));
        }
    }
    Ok(Value::Int(-1))
}

pub(crate) fn format_string(args: &[Value]) -> Result<Value, RuntimeError> {
    let format_str = match args.first().unwrap_or(&Value::Nil) {
        Value::String(text) => text.as_ref().to_owned(),
        Value::Nil => String::new(),
        other => {
            return Err(RuntimeError::new(format!(
                "Format: expected string for format, got {}",
                other.type_name()
            )));
        }
    };

    let format_args = if args.len() > 1 { &args[1..] } else { &[] };
    let formatted = format_script_string("Format", &format_str, format_args)?;
    Ok(Value::String(formatted.into()))
}

/// `SWildcardMatchEx` (C4Strings.cpp:531-562): `*`/`?` wildcard match with
/// backtracking, byte-wise like the C++ char loop.
pub(crate) fn s_wildcard_match_ex(string: &str, wildcard: &str) -> bool {
    let s = clonk_script::c4_string_bytes(string);
    let w = clonk_script::c4_string_bytes(wildcard);
    let (mut pos, mut wild) = (0usize, 0usize);
    let mut backtrack: Option<(usize, usize)> = None;
    while wild < w.len() || backtrack.is_some() {
        if w.get(wild) == Some(&b'*') {
            wild += 1;
            backtrack = Some((wild, pos));
        } else if pos >= s.len() {
            break;
        } else if w.get(wild) == Some(&b'?') || w.get(wild) == Some(&s[pos]) {
            wild += 1;
            pos += 1;
        } else if let Some((last_wild, last_pos)) = backtrack {
            backtrack = Some((last_wild, last_pos + 1));
            wild = last_wild;
            pos = last_pos + 1;
        } else {
            return false;
        }
    }
    wild >= w.len() && pos >= s.len()
}

/// `FnWildcardMatch` (C4Script.cpp:5606-5609): both params go through
/// `FnStringPar`, which maps nil (and Set0'd falsy pars,
/// C4AulExec.cpp:1370-1374) to `""` (C4Script.cpp:78-81).
pub(crate) fn wildcard_match(args: &[Value]) -> Result<Value, RuntimeError> {
    let string_par = |value: Option<&Value>, par: &str| -> Result<String, RuntimeError> {
        match value {
            Some(Value::String(text)) => Ok(text.as_ref().to_owned()),
            Some(Value::Nil) | Some(Value::Int(0)) | Some(Value::Bool(false)) | None => {
                Ok(String::new())
            }
            Some(other) => Err(RuntimeError::new(format!(
                "WildcardMatch: expected string or nil for {par}, got {}",
                other.type_name()
            ))),
        }
    };
    let string = string_par(args.first(), "string")?;
    let wildcard = string_par(args.get(1), "wildcard")?;
    Ok(Value::Int(i32::from(s_wildcard_match_ex(
        &string, &wildcard,
    ))))
}

/// `SCopy(..., C4MaxName)` in FnChangeEffect copies at most 30 native bytes
/// (C4Script.cpp:5534; C4Constants.h:26).
pub(crate) fn truncate_c4_max_name(name: &str) -> String {
    const C4_MAX_NAME: usize = 30;
    let bytes = clonk_script::c4_string_bytes(name);
    if bytes.len() <= C4_MAX_NAME {
        name.to_owned()
    } else {
        clonk_script::c4_string_from_bytes(&bytes[..C4_MAX_NAME])
    }
}

fn legacy_arg_int(args: &[Value], index: usize, function: &str) -> Result<i32, RuntimeError> {
    value_to_i32(
        args.get(index).unwrap_or(&Value::Nil),
        function,
        &format!("argument {}", index + 1),
    )
}

pub(crate) fn legacy_not(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new("Not expects at most 1 argument"));
    }
    Ok(Value::Bool(!args.first().unwrap_or(&Value::Nil).as_bool()))
}

pub(crate) fn legacy_or(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new("Or expects at most 5 arguments"));
    }
    Ok(Value::Bool(args.iter().any(Value::as_bool)))
}

pub(crate) fn legacy_and(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("And expects at most 2 arguments"));
    }
    Ok(Value::Bool((0..2).all(|index| {
        args.get(index).unwrap_or(&Value::Nil).as_bool()
    })))
}

pub(crate) fn legacy_bit_and(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("BitAnd expects at most 2 arguments"));
    }
    Ok(Value::Int(
        legacy_arg_int(args, 0, "BitAnd")? & legacy_arg_int(args, 1, "BitAnd")?,
    ))
}

pub(crate) fn legacy_sum(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 4 {
        return Err(RuntimeError::new("Sum expects at most 4 arguments"));
    }
    (0..4)
        .try_fold(0_i32, |sum, index| {
            Ok(sum.wrapping_add(legacy_arg_int(args, index, "Sum")?))
        })
        .map(Value::Int)
}

pub(crate) fn legacy_sub(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 4 {
        return Err(RuntimeError::new("Sub expects at most 4 arguments"));
    }
    let first = legacy_arg_int(args, 0, "Sub")?;
    (1..4)
        .try_fold(first, |difference, index| {
            Ok(difference.wrapping_sub(legacy_arg_int(args, index, "Sub")?))
        })
        .map(Value::Int)
}

pub(crate) fn legacy_mul(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("Mul expects at most 2 arguments"));
    }
    Ok(Value::Int(
        legacy_arg_int(args, 0, "Mul")?.wrapping_mul(legacy_arg_int(args, 1, "Mul")?),
    ))
}

pub(crate) fn legacy_div(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("Div expects at most 2 arguments"));
    }
    let dividend = legacy_arg_int(args, 0, "Div")?;
    let divisor = legacy_arg_int(args, 1, "Div")?;
    Ok(Value::Int(if divisor == 0 {
        0
    } else {
        dividend.wrapping_div(divisor)
    }))
}

pub(crate) fn legacy_less_than(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("LessThan expects at most 2 arguments"));
    }
    Ok(Value::Int(i32::from(
        legacy_arg_int(args, 0, "LessThan")? < legacy_arg_int(args, 1, "LessThan")?,
    )))
}

pub(crate) fn legacy_greater_than(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("GreaterThan expects at most 2 arguments"));
    }
    Ok(Value::Int(i32::from(
        legacy_arg_int(args, 0, "GreaterThan")? > legacy_arg_int(args, 1, "GreaterThan")?,
    )))
}

pub(crate) fn legacy_s_equal(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("SEqual expects at most 2 arguments"));
    }
    let string = |value: &Value, parameter: &str| match value {
        Value::String(value) => Ok(value.as_ref().to_owned()),
        Value::Nil => Ok(String::new()),
        other => Err(RuntimeError::new(format!(
            "SEqual: expected string for {parameter}, got {}",
            other.type_name()
        ))),
    };
    let left = string(args.first().unwrap_or(&Value::Nil), "first argument")?;
    let right = string(args.get(1).unwrap_or(&Value::Nil), "second argument")?;
    Ok(Value::Int(i32::from(clonk_script::c4_strings_equal(
        &left, &right,
    ))))
}

pub(crate) fn set_var(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new("SetVar expects at most 2 arguments"));
    }
    let index = legacy_arg_int(args, 0, "SetVar")?;
    let value = args.get(1).cloned().unwrap_or(Value::Nil);
    let Some(slots) = clonk_script::caller_var_slots() else {
        return Ok(Value::Nil);
    };
    // C4ValueList::GetItem clamps negative indices to zero, grows NumVars
    // through slot 999,999, and throws at the million-slot boundary.
    if index >= LEGACY_MAX_ARRAY_SIZE {
        return Err(RuntimeError::new("out of memory"));
    }
    slots.set(index, value.clone());
    Ok(value)
}

/// `FnDecVar` (C4Script.cpp:3385-3389): prefix-decrement a slot in the
/// calling function's `NumVars` list and return the new integer value.
pub(crate) fn dec_var(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new("DecVar expects at most 1 argument"));
    }
    let index = legacy_arg_int(args, 0, "DecVar")?;
    let Some(slots) = clonk_script::caller_var_slots() else {
        return Ok(Value::Nil);
    };
    if index >= LEGACY_MAX_ARRAY_SIZE {
        return Err(RuntimeError::new("out of memory"));
    }

    let decremented = Value::Int(cast_stable_raw_i32(&slots.get(index), "DecVar")?.wrapping_sub(1));
    slots.set(index, decremented.clone());
    Ok(decremented)
}

/// `FnIncVar` (C4Script.cpp:3379-3383): prefix-increment a slot in the
/// immediately calling function's `NumVars` list and return the new value.
pub(crate) fn inc_var(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = legacy_arg_int(args, 0, "IncVar")?;
    let Some(slots) = clonk_script::caller_var_slots() else {
        return Ok(Value::Nil);
    };
    if index >= LEGACY_MAX_ARRAY_SIZE {
        return Err(RuntimeError::new("out of memory"));
    }

    let incremented = Value::Int(cast_stable_raw_i32(&slots.get(index), "IncVar")?.wrapping_add(1));
    slots.set(index, incremented.clone());
    Ok(incremented)
}

/// The calling context for one script draw, mirroring the chain the oracle
/// logs from inside FnRandom (C4Script.cpp:3355). Alignment by draw index
/// alone cannot name a divergent caller, which is what the frame-810
/// divergence in clonk-org/clonk-rs#1050 needs; this shares the draw's sink so
/// the annotation lands immediately above the line it describes.
fn script_draw_callsite(range: i32) -> String {
    let caller = with_host_context(None, |context| {
        context.object_context().map(|object| {
            format!(
                "{} {}",
                object.id().as_u64(),
                object.effective_action_name()
            )
        })
    });
    let frame = ENVIRONMENT_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| context.frame)
            .unwrap_or(0)
    });
    format!("CALL {range} f{frame} {}", caller.as_deref().unwrap_or("-"))
}

pub(crate) fn random(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "Random expects at most 1 argument: upper exclusive bound",
        ));
    }

    // FnRandom's int parameter follows the C4AulExec conversion rules
    // (C4AulExec.cpp:1364-1396): a missing/nil/bool argument converts —
    // Random(GetActMapVal(...)) with a missing action is Random(0) in
    // C++. The count++ happens even for range 0 (C4Random.h:43), and a
    // negative range goes through the unsigned modulo like C++'s usual
    // arithmetic conversions — both live in LcgRng::random.
    let range = match args.first().unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Nil => 0,
        Value::Bool(flag) => i32::from(*flag),
        other => {
            return Err(RuntimeError::new(format!(
                "Random: expected int for range, got {}",
                other.type_name()
            )));
        }
    };

    RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("Random: host context unavailable"))?
            .clone();
        let mut rng = context.rng.borrow_mut();
        if rng.trace_index != 0 && std::env::var("LC_RUST_RNG_TRACE_CALLS").is_ok() {
            crate::rng::rng_trace_line(rng.trace_index, &script_draw_callsite(range));
        }
        let value = rng.random(range);
        Ok(Value::Int(value))
    })
}

/// FnAsyncRandom (C4Script.cpp:3367-3370): draw from the deliberately
/// unsynchronized SafeRandom stream without touching the lockstep RNG.
pub(crate) fn async_random(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "AsyncRandom expects at most 1 argument: upper exclusive bound",
        ));
    }
    let range = legacy_arg_int(args, 0, "AsyncRandom")?;
    let value = SCRIPT_SAFE_RNG.with(|rng| rng.borrow_mut().random(range));
    Ok(Value::Int(value))
}

// Mathematical host functions

/// FnAbs (C4Script.cpp:3197-3200) forwards to the `Abs` template
/// (C4Math.h:21) `val > 0 ? val : -val`; `wrapping_abs` keeps the
/// two's-complement negation of INT32_MIN instead of panicking.
pub(crate) fn abs_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new("Abs expects 1 argument: value"));
    }

    match &args[0] {
        Value::Int(value) => Ok(Value::Int(value.wrapping_abs())),
        Value::Nil => Ok(Value::Int(0)),
        other => Err(RuntimeError::new(format!(
            "Abs: expected int, got {}",
            other.type_name()
        ))),
    }
}

pub(crate) fn min_func(args: &[Value]) -> Result<Value, RuntimeError> {
    let val1 = value_to_i32(args.first().unwrap_or(&Value::Nil), "Min", "first argument")?;
    let val2 = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Min", "second argument")?;

    Ok(Value::Int(val1.min(val2)))
}

pub(crate) fn max_func(args: &[Value]) -> Result<Value, RuntimeError> {
    let val1 = value_to_i32(args.first().unwrap_or(&Value::Nil), "Max", "first argument")?;
    let val2 = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Max", "second argument")?;

    Ok(Value::Int(val1.max(val2)))
}

pub(crate) fn sqrt_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new("Sqrt expects 1 argument: value"));
    }

    let value = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Sqrt: expected int, got {}",
                other.type_name()
            )));
        }
    };

    // C++ returns 0 for negative values
    if value < 0 {
        return Ok(Value::Int(0));
    }

    // FnSqrt (C4Script.cpp:3240-3247) truncates the double root and then
    // corrects it with two `iSqrt * iSqrt` comparisons. Those products are
    // `C4ValueInt = int32_t` (C4Value.h:62) and wrap above 46340^2, so for
    // inputs in [2147395601, 2147483647] the decrement never fires and the
    // oracle returns floor(sqrt) + 1. `wrapping_mul` reproduces that.
    let mut root = f64::from(value).sqrt() as i32;
    if root.wrapping_mul(root) < value {
        root += 1;
    }
    if root.wrapping_mul(root) > value {
        root -= 1;
    }
    Ok(Value::Int(root))
}

fn inverse_trig_func(
    args: &[Value],
    function: &str,
    inverse: fn(f64) -> f64,
) -> Result<Value, RuntimeError> {
    let value = value_to_i32(args.first().unwrap_or(&Value::Nil), function, "value")?;
    let radius = value_to_i32(args.get(1).unwrap_or(&Value::Nil), function, "radius")?;
    // FnArcSin/FnArcCos (C4Script.cpp:3276-3298): the comparison is
    // deliberately signed (negative values within the domain remain
    // valid), followed by double-precision libm in degrees and
    // floor(angle + 0.5).
    if radius == 0 || value > radius {
        return Ok(Value::Int(0));
    }
    let angle = inverse(f64::from(value) / f64::from(radius)) * 180.0 * std::f64::consts::FRAC_1_PI;
    Ok(Value::Int(round_inverse_angle(angle)))
}

pub(crate) fn round_inverse_angle(angle: f64) -> i32 {
    (angle + 0.5).floor() as i32
}

pub(crate) fn arc_sin_func(args: &[Value]) -> Result<Value, RuntimeError> {
    inverse_trig_func(args, "ArcSin", f64::asin)
}

pub(crate) fn arc_cos_func(args: &[Value]) -> Result<Value, RuntimeError> {
    inverse_trig_func(args, "ArcCos", f64::acos)
}

/// FnAngle (C4Script.cpp:3255-3280): the position-to-position angle in
/// Clonk orientation (0 = up, 90 = right), `iPrec` scaling (default 1).
/// Axis-aligned deltas take the exact shortcuts; the general case is
/// `trunc(180 * prec * atan2(|dy|, |dx|) / pi)` folded into the quadrant.
pub(crate) fn angle_func(args: &[Value]) -> Result<Value, RuntimeError> {
    let x1 = value_to_i32(args.first().unwrap_or(&Value::Nil), "Angle", "x1")?;
    let y1 = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Angle", "y1")?;
    let x2 = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "Angle", "x2")?;
    let y2 = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "Angle", "y2")?;
    let mut prec = value_to_i32(args.get(4).unwrap_or(&Value::Nil), "Angle", "precision")?;
    if prec == 0 {
        prec = 1;
    }

    let dx = x2.wrapping_sub(x1);
    let dy = y2.wrapping_sub(y1);
    if dx == 0 {
        return Ok(Value::Int(if dy > 0 { 180 * prec } else { 0 }));
    }
    if dy == 0 {
        return Ok(Value::Int(if dx > 0 { 90 * prec } else { 270 * prec }));
    }

    let angle = (180.0
        * f64::from(prec)
        * f64::from(dy.abs()).atan2(f64::from(dx.abs()))
        * std::f64::consts::FRAC_1_PI) as i32;

    Ok(Value::Int(if x2 > x1 {
        if y2 < y1 {
            90 * prec - angle
        } else {
            90 * prec + angle
        }
    } else if y2 < y1 {
        270 * prec + angle
    } else {
        270 * prec - angle
    }))
}

/// FnMod (C4Script.cpp:3219-3223): truncated `%`, zero divisor yields 0.
pub(crate) fn modulo(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = parse_optional_i32(args.first(), "Mod", "value")?.unwrap_or(0);
    let divisor = parse_optional_i32(args.get(1), "Mod", "divisor")?.unwrap_or(0);
    Ok(Value::Int(if divisor == 0 {
        0
    } else {
        value.wrapping_rem(divisor)
    }))
}

/// FnC4Id (C4Script.cpp:2396-2399): string to C4ID; empty/nil is C4ID_None
/// (0 — falsy, like C4Id("") in C++).
pub(crate) fn c4_id(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = parse_optional_string(args.first(), "C4Id", "id")?;
    Ok(match name {
        Some(name) if clonk_script::c4_id_parse(&name) != 0 => Value::C4Id(
            clonk_script::c4_id_from_raw(clonk_script::c4_id_parse(&name)),
        ),
        _ => Value::Nil,
    })
}

pub(crate) fn pow_func(args: &[Value]) -> Result<Value, RuntimeError> {
    let base = value_to_i32(args.first().unwrap_or(&Value::Nil), "Pow", "base")?;
    let exponent = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Pow", "exponent")?;

    if exponent < 0 {
        return Ok(Value::Int(0)); // Match C++ behavior for negative exponents
    }

    Ok(Value::Int(base.wrapping_pow(exponent as u32)))
}

pub(crate) fn bound_by_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 3 {
        return Err(RuntimeError::new(
            "BoundBy expects 3 arguments: value, min, max",
        ));
    }

    let value = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "BoundBy: expected int for value, got {}",
                other.type_name()
            )));
        }
    };

    let range1 = match &args[1] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "BoundBy: expected int for range1, got {}",
                other.type_name()
            )));
        }
    };

    let range2 = match &args[2] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "BoundBy: expected int for range2, got {}",
                other.type_name()
            )));
        }
    };

    // BoundBy clamps value between range1 and range2 (order doesn't matter)
    let min = range1.min(range2);
    let max = range1.max(range2);
    Ok(Value::Int(value.clamp(min, max)))
}

pub(crate) fn sin_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 3 {
        return Err(RuntimeError::new(
            "Sin expects 1-3 arguments: angle, radius, precision",
        ));
    }

    let angle = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Sin: expected int for angle, got {}",
                other.type_name()
            )));
        }
    };

    let radius = if args.len() > 1 {
        match &args[1] {
            Value::Int(v) => *v,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sin: expected int for radius, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        0
    };

    let precision = if args.len() > 2 {
        match &args[2] {
            Value::Int(v) => {
                if *v == 0 {
                    1
                } else {
                    *v
                }
            }
            Value::Nil => 1,
            other => {
                return Err(RuntimeError::new(format!(
                    "Sin: expected int for precision, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        1
    };

    // FnSin uses C4Fixed's shared SineTable; no floating-point arithmetic is
    // allowed on this lockstep path (C4Script.cpp:3224-3231; Fixed.h:188-202).
    let angle_mod = angle % (360 * precision);
    let result = fixtoi_prec(itofix_prec(angle_mod, precision).sin_deg(), radius);
    Ok(Value::Int(result))
}

pub(crate) fn cos_func(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 3 {
        return Err(RuntimeError::new(
            "Cos expects 1-3 arguments: angle, radius, precision",
        ));
    }

    let angle = match &args[0] {
        Value::Int(v) => *v,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "Cos: expected int for angle, got {}",
                other.type_name()
            )));
        }
    };

    let radius = if args.len() > 1 {
        match &args[1] {
            Value::Int(v) => *v,
            Value::Nil => 0,
            other => {
                return Err(RuntimeError::new(format!(
                    "Cos: expected int for radius, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        0
    };

    let precision = if args.len() > 2 {
        match &args[2] {
            Value::Int(v) => {
                if *v == 0 {
                    1
                } else {
                    *v
                }
            }
            Value::Nil => 1,
            other => {
                return Err(RuntimeError::new(format!(
                    "Cos: expected int for precision, got {}",
                    other.type_name()
                )));
            }
        }
    } else {
        1
    };

    // FnCos uses the same fixed-point table and scaling path
    // (C4Script.cpp:3233-3238; Fixed.h:204-218).
    let angle_mod = angle % (360 * precision);
    let result = fixtoi_prec(itofix_prec(angle_mod, precision).cos_deg(), radius);
    Ok(Value::Int(result))
}

pub(crate) fn int_argument(
    args: &[Value],
    index: usize,
    fn_name: &str,
) -> Result<i32, RuntimeError> {
    match args.get(index) {
        Some(Value::Int(value)) => Ok(*value),
        Some(Value::Nil) | None => Ok(0),
        Some(other) => Err(RuntimeError::new(format!(
            "{fn_name}: expected int, got {}",
            other.type_name()
        ))),
    }
}

/// FnGetChar (C4Script.cpp:4361-4370): the unsigned native byte at the index,
/// 0 past the end, and nil without a string. C++'s forward loop leaves every
/// negative index at offset zero.
pub(crate) fn get_char(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(Value::String(text)) = args.first() else {
        return Ok(Value::Nil);
    };
    let index = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "GetChar", "index")?;
    if index < 0 {
        return Ok(Value::Int(
            clonk_script::c4_string_byte(text, 0)
                .map(i32::from)
                .unwrap_or(0),
        ));
    }
    Ok(Value::Int(
        clonk_script::c4_string_byte(text, index as usize)
            .map(i32::from)
            .unwrap_or(0),
    ))
}

pub(crate) fn value_as_object_id(value: &Value) -> Option<ObjectId> {
    match value {
        Value::Object(id) => Some(ObjectId::new(*id)),
        _ => object_id_from_value(value),
    }
}

/// `C4Value::getInt()` reads the shared low `Data.Int` word for Int and Bool
/// values. `RawBool` preserves the non-canonical Bool payload that script
/// casts can produce; every other C4V type yields zero.
pub(crate) fn value_as_i32(value: &Value) -> i32 {
    match value {
        Value::Int(value) => *value,
        Value::Bool(value) => i32::from(*value),
        Value::RawBool(value) => *value as u32 as i32,
        _ => 0,
    }
}

/// C4Rect::Intersect (C4Rect.cpp:101-133): narrow `a` to the overlap with
/// `b`; a degenerated result clamps to zero size.
pub(crate) fn rect_intersect_cpp(a: DefinitionRect, b: DefinitionRect) -> DefinitionRect {
    let mut result = a;
    if b.x > result.x {
        if b.x + b.width < result.x + result.width {
            result.x = b.x;
            result.width = b.width;
        } else {
            result.width -= b.x - result.x;
            result.x = b.x;
        }
    } else if b.x + b.width < result.x + result.width {
        result.width = b.x + b.width - result.x;
    }
    if b.y > result.y {
        if b.y + b.height < result.y + result.height {
            result.y = b.y;
            result.height = b.height;
        } else {
            result.height -= b.y - result.y;
            result.y = b.y;
        }
    } else if b.y + b.height < result.y + result.height {
        result.height = b.y + b.height - result.y;
    }
    result.width = result.width.max(0);
    result.height = result.height.max(0);
    result
}

/// C4Rect::Add (C4Rect.cpp:153-185): expand `a` to cover `b`; a null rect
/// on either side leaves the other unchanged.
pub(crate) fn rect_add_cpp(a: DefinitionRect, b: DefinitionRect) -> DefinitionRect {
    if b.width == 0 || b.height == 0 {
        return a;
    }
    if a.width == 0 || a.height == 0 {
        return b;
    }
    let mut result = a;
    if b.x < result.x {
        if b.x + b.width > result.x + result.width {
            result.x = b.x;
            result.width = b.width;
        } else {
            result.width += result.x - b.x;
            result.x = b.x;
        }
    } else if b.x + b.width > result.x + result.width {
        result.width = b.x + b.width - result.x;
    }
    if b.y < result.y {
        if b.y + b.height > result.y + result.height {
            result.y = b.y;
            result.height = b.height;
        } else {
            result.height += result.y - b.y;
            result.y = b.y;
        }
    } else if b.y + b.height > result.y + result.height {
        result.height = b.y + b.height - result.y;
    }
    result
}

/// Raw `C4Rect::Contains`; unlike the convenience `DefinitionRect` methods,
/// this preserves C++ behavior for zero/negative dimensions accepted by
/// `SetShape` and its wrapping `int32_t` edge arithmetic.
pub(crate) fn rect_contains_point_cpp(rect: DefinitionRect, x: i32, y: i32) -> bool {
    x >= rect.x
        && x < rect.x.wrapping_add(rect.width)
        && y >= rect.y
        && y < rect.y.wrapping_add(rect.height)
}

/// Raw `C4Rect::Overlap` (C4Rect.cpp:92-99).
pub(crate) fn rects_overlap_cpp(a: DefinitionRect, b: DefinitionRect) -> bool {
    a.x.wrapping_add(a.width) > b.x
        && a.x < b.x.wrapping_add(b.width)
        && a.y.wrapping_add(a.height) > b.y
        && a.y < b.y.wrapping_add(b.height)
}

/// Literal integer-edge port of `C4Rect::IntersectsLine`
/// (C4Rect.cpp:131-155). In particular this is not a rasterized line walk:
/// the native two edge probes and truncating divisions can accept a segment
/// that never visits an integer point inside the rectangle.
pub(crate) fn rect_intersects_line_cpp(
    rect: DefinitionRect,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> bool {
    if rect_contains_point_cpp(rect, x1, y1) || rect_contains_point_cpp(rect, x2, y2) {
        return true;
    }
    let right = rect.x.wrapping_add(rect.width);
    let bottom = rect.y.wrapping_add(rect.height);
    if (x1 < rect.x && x2 < rect.x)
        || (y1 < rect.y && y2 < rect.y)
        || (x1 >= right && x2 >= right)
        || (y1 >= bottom && y2 >= bottom)
    {
        return false;
    }
    if x1 == x2 || y1 == y2 {
        return true;
    }

    let intersect_x = if x1 < rect.x { rect.x } else { right };
    let intersect_y = y1.wrapping_add(
        y2.wrapping_sub(y1)
            .wrapping_mul(intersect_x.wrapping_sub(x1))
            .wrapping_div(x2.wrapping_sub(x1)),
    );
    if intersect_y >= rect.y && intersect_y < bottom {
        return true;
    }

    let intersect_y = if y1 < rect.y { rect.y } else { bottom };
    let intersect_x = x1.wrapping_add(
        x2.wrapping_sub(x1)
            .wrapping_mul(intersect_y.wrapping_sub(y1))
            .wrapping_div(y2.wrapping_sub(y1)),
    );
    intersect_x >= rect.x && intersect_x < right
}

/// `C4Value::getInt` tolerance on the staged Var slots: non-numeric
/// values read as 0.
pub(crate) fn value_as_int(value: &Value) -> i32 {
    match value {
        Value::Int(int) => *int,
        Value::Bool(flag) => i32::from(*flag),
        _ => 0,
    }
}

/// FnDistance (C4Script.cpp:3316-3319) -> Distance (C4Math.cpp:22-31):
/// integer euclidean distance; the float sqrt is post-adjusted to the
/// exact integer floor (++/-- until dist^2 brackets d2). Negative d2
/// (int64 overflow in C++) returns -1.
pub(crate) fn distance(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 4 {
        return Err(RuntimeError::new(
            "Distance expects at most 4 arguments: x1, y1, x2, y2",
        ));
    }
    let x1 = parse_optional_i32(args.first(), "Distance", "x1")?.unwrap_or(0);
    let y1 = parse_optional_i32(args.get(1), "Distance", "y1")?.unwrap_or(0);
    let x2 = parse_optional_i32(args.get(2), "Distance", "x2")?.unwrap_or(0);
    let y2 = parse_optional_i32(args.get(3), "Distance", "y2")?.unwrap_or(0);

    let dx = i64::from(x1) - i64::from(x2);
    let dy = i64::from(y1) - i64::from(y2);
    let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    if d2 < 0 {
        return Ok(Value::Int(-1));
    }
    let mut dist = (d2 as f64).sqrt() as i64;
    if dist.wrapping_mul(dist) < d2 {
        dist += 1;
    }
    if dist.wrapping_mul(dist) > d2 {
        dist -= 1;
    }
    Ok(Value::Int(dist as i32))
}

pub(crate) fn is_removed_object_value(value: &Value, removed: &HashSet<ObjectId>) -> bool {
    matches!(value, Value::Object(id) if removed.contains(&ObjectId::new(*id)))
}

/// `C4Value::operator bool` (C4Value.h:76,183-185): raw-data truthiness —
/// false only for nil, 0 and false; non-empty-ness is NOT required for
/// strings/arrays/maps, and no type conversion happens (unlike `getBool`).
pub(crate) fn value_raw_truthy(value: &Value) -> bool {
    value.as_bool()
}
