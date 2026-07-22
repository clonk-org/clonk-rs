use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use indexmap::{Equivalent, IndexMap};

const C4V_ANY: usize = 0;
const C4V_INT: usize = 1;
const C4V_BOOL: usize = 2;
const C4V_C4ID: usize = 3;
const C4V_C4OBJECT: usize = 4;
const C4V_STRING: usize = 5;
const C4V_ARRAY: usize = 6;
const C4V_MAP: usize = 7;

// Rust strings cannot contain arbitrary C4Script bytes. Keep ASCII as-is so
// ordinary script literals compare naturally, and encode high raw bytes in a
// supplementary private-use range. All byte-observing VM operations decode
// this range back to the original byte.
const C4_RAW_BYTE_ESCAPE_BASE: u32 = 0xF0000;
const C4_RAW_BYTE_ESCAPE_END: u32 = C4_RAW_BYTE_ESCAPE_BASE + u8::MAX as u32;

fn c4_raw_byte_escape(character: char) -> Option<u8> {
    let scalar = u32::from(character);
    (C4_RAW_BYTE_ESCAPE_BASE..=C4_RAW_BYTE_ESCAPE_END)
        .contains(&scalar)
        .then(|| (scalar - C4_RAW_BYTE_ESCAPE_BASE) as u8)
}

/// Build a script string that losslessly represents arbitrary native bytes.
pub fn c4_string_from_bytes(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        if !text
            .chars()
            .any(|character| c4_raw_byte_escape(character).is_some())
        {
            return text.to_owned();
        }
    }
    bytes
        .iter()
        .copied()
        .map(|byte| {
            if byte.is_ascii() {
                char::from(byte)
            } else {
                char::from_u32(C4_RAW_BYTE_ESCAPE_BASE + u32::from(byte))
                    .expect("private-use byte escape is a valid scalar")
            }
        })
        .collect()
}

/// Borrow the native byte spelling when the Rust string needs no raw-byte
/// projection. Ordinary UTF-8 is already the exact C4 string byte sequence;
/// only the private-use escape range requires decoding into owned storage.
fn c4_string_bytes_cow(value: &str) -> Cow<'_, [u8]> {
    if !value
        .chars()
        .any(|character| c4_raw_byte_escape(character).is_some())
    {
        return Cow::Borrowed(value.as_bytes());
    }

    let mut bytes = Vec::with_capacity(value.len());
    for character in value.chars() {
        if let Some(byte) = c4_raw_byte_escape(character) {
            bytes.push(byte);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
    Cow::Owned(bytes)
}

/// Recover the native byte buffer represented by a script string.
pub fn c4_string_bytes(value: &str) -> Vec<u8> {
    c4_string_bytes_cow(value).into_owned()
}

pub fn c4_string_byte_len(value: &str) -> usize {
    if !value
        .chars()
        .any(|character| c4_raw_byte_escape(character).is_some())
    {
        return value.len();
    }

    value
        .chars()
        .map(|character| {
            if c4_raw_byte_escape(character).is_some() {
                1
            } else {
                character.len_utf8()
            }
        })
        .sum()
}

/// One native `C4String` identity.
///
/// Copies share the native pointer's mutable `iEnumID`; separately created
/// runtime strings retain distinct pointer identity even when their bytes are
/// equal. Textual C4Value equality and hashing deliberately ignore that
/// identity.
#[derive(Clone)]
pub struct C4StringValue(Arc<C4StringValueInner>);

pub(crate) struct C4StringValueInner {
    value: String,
    enum_id: AtomicI32,
}

impl C4StringValue {
    /// Construct a fresh, unenumerated runtime string.
    pub fn new(value: String) -> Self {
        Self::loaded(value, -1)
    }

    /// Construct a string with an existing native enumeration ID.
    pub fn loaded(value: String, enum_id: i32) -> Self {
        Self(Arc::new(C4StringValueInner {
            value,
            enum_id: AtomicI32::new(enum_id),
        }))
    }

    pub fn enum_id(&self) -> i32 {
        self.0.enum_id.load(Ordering::Relaxed)
    }

    pub fn as_str(&self) -> &str {
        &self.0.value
    }

    /// Set the shared enumeration ID. Public serializers use this when they
    /// perform a self-contained native string-table enumeration.
    #[doc(hidden)]
    pub fn set_enum_id(&self, enum_id: i32) {
        self.0.enum_id.store(enum_id, Ordering::Relaxed);
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn into_string(self) -> String {
        match Arc::try_unwrap(self.0) {
            Ok(inner) => inner.value,
            Err(inner) => inner.value.clone(),
        }
    }

    pub(crate) fn downgrade(&self) -> std::sync::Weak<C4StringValueInner> {
        Arc::downgrade(&self.0)
    }

    pub(crate) fn from_inner(inner: Arc<C4StringValueInner>) -> Self {
        Self(inner)
    }
}

impl Default for C4StringValue {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl Deref for C4StringValue {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0.value
    }
}

impl AsRef<str> for C4StringValue {
    fn as_ref(&self) -> &str {
        self
    }
}

impl std::borrow::Borrow<str> for C4StringValue {
    fn borrow(&self) -> &str {
        self
    }
}

impl From<String> for C4StringValue {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for C4StringValue {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

impl From<C4StringValue> for String {
    fn from(value: C4StringValue) -> Self {
        value.into_string()
    }
}

impl fmt::Debug for C4StringValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_ref(), formatter)
    }
}

impl fmt::Display for C4StringValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self)
    }
}

impl PartialEq for C4StringValue {
    fn eq(&self, other: &Self) -> bool {
        c4_strings_equal(self, other)
    }
}

impl PartialEq<str> for C4StringValue {
    fn eq(&self, other: &str) -> bool {
        c4_strings_equal(self, other)
    }
}

impl PartialEq<String> for C4StringValue {
    fn eq(&self, other: &String) -> bool {
        c4_strings_equal(self, other)
    }
}

impl PartialEq<C4StringValue> for str {
    fn eq(&self, other: &C4StringValue) -> bool {
        c4_strings_equal(self, other)
    }
}

impl PartialEq<C4StringValue> for String {
    fn eq(&self, other: &C4StringValue) -> bool {
        c4_strings_equal(self, other)
    }
}

impl Eq for C4StringValue {}

impl Hash for C4StringValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        c4_string_hash(self).hash(state);
    }
}

impl serde::Serialize for C4StringValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        c4_string_serde::serialize(self, serializer)
    }
}

impl<'de> serde::Deserialize<'de> for C4StringValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        c4_string_serde::deserialize(deserializer)
    }
}

pub(crate) fn c4_string_from_literal(value: String) -> String {
    if value.chars().any(|character| c4_raw_byte_escape(character).is_some()) {
        c4_string_from_bytes(value.as_bytes())
    } else {
        value
    }
}

fn c4_string_literal_query(value: &str) -> Option<Cow<'_, str>> {
    if value
        .chars()
        .any(|character| c4_raw_byte_escape(character).is_some())
    {
        Some(Cow::Owned(c4_string_from_bytes(value.as_bytes())))
    } else {
        None
    }
}

pub fn c4_string_byte(value: &str, index: usize) -> Option<u8> {
    c4_string_bytes_cow(value).get(index).copied()
}

pub fn c4_strings_equal(left: &str, right: &str) -> bool {
    c4_string_bytes_cow(left) == c4_string_bytes_cow(right)
}

const C4_ID_RAW_PREFIX: &str = "\u{f0ffe}c4id:";

// C++ defines C4ID as `unsigned long`. That is pointer-width on the supported
// LP64 targets, but remains 32-bit on 64-bit Windows (LLP64).
#[inline]
fn c4_id_normalize_raw(raw: usize) -> usize {
    #[cfg(target_os = "windows")]
    {
        raw & u32::MAX as usize
    }
    #[cfg(not(target_os = "windows"))]
    {
        raw
    }
}

/// Store an already-constructed C4ID payload without reparsing its bytes.
/// This is required for CastC4ID, whose union payload can differ from the
/// signed-char behavior of C4Id(string).
pub fn c4_id_from_raw(raw: usize) -> String {
    let raw = c4_id_normalize_raw(raw);
    if raw == 0 {
        return "NONE".to_owned();
    }
    if (1..=9999).contains(&raw) {
        return format!("{raw:04}");
    }

    if raw <= u32::MAX as usize {
        let bytes = (raw as u32).to_le_bytes();
        if bytes != *b"NONE"
            && !bytes.iter().all(u8::is_ascii_digit)
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
        {
            return String::from_utf8(bytes.to_vec()).expect("C4ID text bytes are ASCII");
        }
    }

    format!("{C4_ID_RAW_PREFIX}{raw:x}")
}

fn c4_id_tagged_raw(id: &str) -> Option<usize> {
    let hex = id.strip_prefix(C4_ID_RAW_PREFIX)?;
    (!hex.is_empty() && hex.len() <= std::mem::size_of::<usize>() * 2)
        .then(|| usize::from_str_radix(hex, 16).ok())
        .flatten()
        .map(c4_id_normalize_raw)
}

/// C4IdText for an already-typed C4ID value.
pub fn c4_id_text(id: &str) -> String {
    let raw = c4_id_raw(id);
    if raw == 0 {
        return "NONE".to_owned();
    }
    let signed = raw as u32 as i32;
    if (0..=9999).contains(&signed) {
        return format!("{signed:04}");
    }
    let bytes = (raw as u32).to_le_bytes();
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    c4_string_from_bytes(&bytes[..end])
}

pub mod c4_id_serde {
    use super::{c4_id_from_raw, c4_id_parse, c4_id_raw, c4_string_from_bytes};

    pub fn serialize<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(c4_id_raw(value) as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Raw(u64),
            Legacy(String),
        }

        match <Repr as serde::Deserialize>::deserialize(deserializer)? {
            Repr::Raw(raw) => usize::try_from(raw)
                .map(c4_id_from_raw)
                .map_err(serde::de::Error::custom),
            Repr::Legacy(value) => {
                let text = c4_string_from_bytes(value.as_bytes());
                Ok(c4_id_from_raw(c4_id_parse(&text)))
            }
        }
    }
}

pub mod c4_optional_id_serde {
    use super::c4_id_raw;

    struct Ref<'a>(&'a str);

    impl serde::Serialize for Ref<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_u64(c4_id_raw(self.0) as u64)
        }
    }

    pub fn serialize<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&Ref(value)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Wrapped(#[serde(with = "super::c4_id_serde")] String);

        <Option<Wrapped> as serde::Deserialize>::deserialize(deserializer)
            .map(|value| value.map(|value| value.0))
    }
}

pub mod c4_string_serde {
    use std::borrow::Cow;

    use super::{c4_string_bytes_cow, c4_string_from_bytes, c4_string_from_literal};

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Text(String),
        Bytes { c4_bytes: Vec<u8> },
    }

    struct Ref<'a>(&'a str);

    impl serde::Serialize for Ref<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match c4_string_bytes_cow(self.0) {
                Cow::Borrowed(_) => serializer.serialize_str(self.0),
                Cow::Owned(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => serializer.serialize_str(&text),
                    Err(error) => Repr::Bytes {
                        c4_bytes: error.into_bytes(),
                    }
                    .serialize(serializer),
                },
            }
        }
    }

    pub fn serialize<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        T: AsRef<str> + ?Sized,
    {
        serde::Serialize::serialize(&Ref(value.as_ref()), serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: From<String>,
    {
        let value = match <Repr as serde::Deserialize>::deserialize(deserializer)? {
            Repr::Text(value) => Ok(c4_string_from_literal(value)),
            Repr::Bytes { c4_bytes } => Ok(c4_string_from_bytes(&c4_bytes)),
        }?;
        Ok(T::from(value))
    }

    pub fn serialize_ref<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(&Ref(value), serializer)
    }
}

pub mod c4_optional_string_serde {
    use super::c4_string_serde;

    struct Ref<'a>(&'a str);

    impl serde::Serialize for Ref<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            c4_string_serde::serialize_ref(self.0, serializer)
        }
    }

    pub fn serialize<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&Ref(value)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Wrapped(#[serde(with = "super::c4_string_serde")] String);

        <Option<Wrapped> as serde::Deserialize>::deserialize(deserializer)
            .map(|value| value.map(|value| value.0))
    }
}

/// C4Script value type tag, mirroring the C++ `enum C4V_Type` (C4Value.h:37-54)
/// for the entries that index `C4ScriptCnvMap` (`C4V_Any`..`C4V_pC4Value`, i.e.
/// `0..=C4V_Last`). `C4V_C4ObjectEnum` (9) only appears in serialization and is
/// deliberately outside the conversion table, so it is omitted here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum C4VType {
    Any = 0,
    Int = 1,
    Bool = 2,
    C4Id = 3,
    C4Object = 4,
    String = 5,
    Array = 6,
    Map = 7,
    Ref = 8,
}

impl C4VType {
    /// `C4V_Last + 1`: the conversion table is `[C4V_Last + 1][C4V_Last + 1]`.
    const COUNT: usize = 9;

    /// Every table type in index order — the row/column order of `C4ScriptCnvMap`.
    pub const ALL: [C4VType; Self::COUNT] = [
        C4VType::Any,
        C4VType::Int,
        C4VType::Bool,
        C4VType::C4Id,
        C4VType::C4Object,
        C4VType::String,
        C4VType::Array,
        C4VType::Map,
        C4VType::Ref,
    ];

    /// This type's `C4ScriptCnvMap` row/column index.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One classification cell of `C4ScriptCnvMap`, mirroring the `C4VCnvFn` struct
/// (C4Value.h:81-85) via the six converter macros (C4Value.cpp:481-486). The
/// `Warn` flag is a pure function of the class (only `CnvError`/`CnvDirectOld`
/// warn), so it is derived in [`CnvFn::warns`] rather than stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CnvFn {
    /// `CnvOK` — convert by same value (the C++ cell holds a null function ptr).
    Ok,
    /// `CnvError` (`FnCnvError`, C4Value.cpp:439-443) — deny the conversion.
    Error,
    /// `CnvGuess` (`FnCnvGuess`, C4Value.cpp:453-467) — guess an `Any` value's
    /// type, then retry.
    Guess,
    /// `CnvInt2Id` (`FnCnvInt2Id`, C4Value.cpp:469-478) — `int`→`id` iff the
    /// value is in `0..=9999`.
    Int2Id,
    /// `CnvDirectOld` (`FnCnvDirectOld`, C4Value.cpp:431-437) — fails under
    /// `#strict`, no-op otherwise.
    DirectOld,
    /// `CnvDeref` (`FnCnvDeref`, C4Value.cpp:445-451) — resolve a reference and
    /// retry.
    Deref,
}

impl CnvFn {
    /// The `Warn` flag from the C4Value.cpp:481-486 macros: only `CnvError`
    /// (`CnvError = FnCnvError, true`) and `CnvDirectOld`
    /// (`CnvDirectOld = FnCnvDirectOld, true`) set it.
    pub fn warns(self) -> bool {
        matches!(self, CnvFn::Error | CnvFn::DirectOld)
    }

    /// A stable single-character code for the differential parity golden:
    /// `O`k / `E`rror / `G`uess / int`2`id / `D`irectOld / de`R`ef.
    pub fn code(self) -> char {
        match self {
            CnvFn::Ok => 'O',
            CnvFn::Error => 'E',
            CnvFn::Guess => 'G',
            CnvFn::Int2Id => '2',
            CnvFn::DirectOld => 'D',
            CnvFn::Deref => 'R',
        }
    }
}

/// The 9×9 `C4ScriptCnvMap` type-conversion table, transcribed cell-for-cell
/// from `C4Value::C4ScriptCnvMap` (C4Value.cpp:488-598). Rows are the source
/// `C4V_Type`, columns the destination type, both in [`C4VType::ALL`] order.
#[rustfmt::skip]
const C4_SCRIPT_CNV_MAP: [[CnvFn; C4VType::COUNT]; C4VType::COUNT] = {
    use CnvFn::{Deref, DirectOld, Error, Guess, Int2Id, Ok};
    [
        //   any     int        bool   c4id       object  string  array   map    ref
        [    Ok,     Guess,     Guess, Guess,     Guess,  Guess,  Guess,  Guess, Error ], // C4V_Any      (:490-501)
        [    Ok,     Ok,        Ok,    Int2Id,    Error,  Error,  Error,  Error, Error ], // C4V_Int      (:502-513)
        [    Ok,     Ok,        Ok,    DirectOld, Error,  Error,  Error,  Error, Error ], // C4V_Bool     (:514-525)
        [    Ok,     DirectOld, Ok,    Ok,        Error,  Error,  Error,  Error, Error ], // C4V_C4ID     (:526-537)
        [    Ok,     DirectOld, Ok,    Error,     Ok,     Error,  Error,  Error, Error ], // C4V_C4Object (:538-549)
        [    Ok,     DirectOld, Ok,    Error,     Error,  Ok,     Error,  Error, Error ], // C4V_String   (:550-561)
        [    Ok,     Error,     Ok,    Error,     Error,  Error,  Ok,     Error, Error ], // C4V_Array    (:562-573)
        [    Ok,     Error,     Ok,    Error,     Error,  Error,  Error,  Ok,    Error ], // C4V_Map      (:574-585)
        [    Deref,  Deref,     Deref, Deref,     Deref,  Deref,  Deref,  Deref, Ok    ], // C4V_pC4Value (:586-597)
    ]
};

/// The `C4ScriptCnvMap[from][to]` classification (C4Value.cpp:488-598).
pub fn cnv_fn(from: C4VType, to: C4VType) -> CnvFn {
    C4_SCRIPT_CNV_MAP[from.index()][to.index()]
}

/// A C4Script map: arbitrary [`Value`] keys with stable insertion order.
///
/// C++ uses an unordered map for lookup plus a separate insertion-order list.
/// `IndexMap` provides the same externally visible behavior: replacing an
/// existing key keeps its position, while removing and reinserting appends it.
#[derive(Clone, Default)]
pub struct ValueMap(
    IndexMap<Value, Value>,
    // C4ValueHash allocates mapped C4Value slots separately from its hash
    // entries. Removing an entry retains that slot in emptyValues, including
    // any value left behind when the key itself became nil. New keys reuse
    // the most recently removed slot.
    Vec<Value>,
);

impl fmt::Debug for ValueMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ValueMap").field(&self.0).finish()
    }
}

impl PartialEq for ValueMap {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for ValueMap {}

impl ValueMap {
    pub fn new() -> Self {
        Self(IndexMap::new(), Vec::new())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(IndexMap::with_capacity(capacity), Vec::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> indexmap::map::Iter<'_, Value, Value> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> indexmap::map::IterMut<'_, Value, Value> {
        self.0.iter_mut()
    }

    pub fn keys(&self) -> indexmap::map::Keys<'_, Value, Value> {
        self.0.keys()
    }

    pub fn values(&self) -> indexmap::map::Values<'_, Value, Value> {
        self.0.values()
    }

    pub fn values_mut(&mut self) -> indexmap::map::ValuesMut<'_, Value, Value> {
        self.0.values_mut()
    }

    /// Values retained in native `C4ValueHash::emptyValues` slot-reuse order.
    /// They are not map entries, but remain live C4Values until reuse or map
    /// destruction and therefore participate in C4String enumeration.
    #[doc(hidden)]
    pub fn hidden_values(&self) -> impl DoubleEndedIterator<Item = &Value> {
        self.1.iter().rev()
    }

    /// String-property lookup (`map.foo` and the overwhelmingly common engine
    /// access pattern) without allocating a temporary [`Value::String`].
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(&StringQuery(key)).or_else(|| {
            c4_string_literal_query(key)
                .and_then(|key| self.0.get(&StringQuery(key.as_ref())))
        })
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        if self.0.contains_key(&StringQuery(key)) {
            return self.0.get_mut(&StringQuery(key));
        }
        let key = c4_string_literal_query(key)?;
        self.0.get_mut(&StringQuery(key.as_ref()))
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(&StringQuery(key))
            || c4_string_literal_query(key)
                .is_some_and(|key| self.0.contains_key(&StringQuery(key.as_ref())))
    }

    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.insert_key(Value::from(key), value)
    }

    /// Assigns through a C4ValueHash-owned string slot. Nonnil -> nil erases
    /// the entry, while missing/already-nil -> nil remains present because
    /// C4Value::Set returns before CheckRemoveFromMap for an unchanged nil.
    pub(crate) fn assign(&mut self, key: String, value: Value) {
        if matches!(value, Value::Nil)
            && self
                .get(&key)
                .is_some_and(|current| !matches!(current, Value::Nil))
        {
            self.shift_remove(&key);
            self.recycle_value_slot(Value::Nil);
        } else {
            self.insert(key, value);
        }
    }

    pub fn shift_remove(&mut self, key: &str) -> Option<Value> {
        if self.0.contains_key(&StringQuery(key)) {
            return self.0.shift_remove(&StringQuery(key));
        }
        let key = c4_string_literal_query(key)?;
        self.0.shift_remove(&StringQuery(key.as_ref()))
    }

    pub fn get_key(&self, key: &Value) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn get_key_mut(&mut self, key: &Value) -> Option<&mut Value> {
        self.0.get_mut(key)
    }

    pub fn contains_value_key(&self, key: &Value) -> bool {
        self.0.contains_key(key)
    }

    pub fn insert_key(&mut self, key: Value, value: Value) -> Option<Value> {
        if let Some(current) = self.0.get_mut(&key) {
            return Some(std::mem::replace(current, value));
        }

        if let Some(recycled) = self.1.pop() {
            // operator[] first attaches the recycled mapped slot to the new
            // key and only then assigns. A nonnil recycled value assigned nil
            // therefore clears the slot and immediately removes the new key;
            // an already-nil slot takes C4Value::Set's unchanged-value return
            // and leaves a visible nil entry behind.
            if matches!(value, Value::Nil) && !matches!(recycled, Value::Nil) {
                self.recycle_value_slot(Value::Nil);
                return None;
            }
        }

        self.0.insert(key, value)
    }

    /// Retain one removed C4ValueHash mapped slot for native-order reuse.
    ///
    /// This is exposed for loaders that denumerate an owning map key: native
    /// `removeValue` erases that key but leaves its mapped value alive in the
    /// map's `emptyValues` pool until another key reuses it or the map dies.
    #[doc(hidden)]
    pub fn recycle_value_slot(&mut self, value: Value) {
        self.1.push(value);
    }

    /// Arbitrary-key form of [`Self::assign`].
    pub(crate) fn assign_key(&mut self, key: Value, value: Value) {
        if matches!(value, Value::Nil)
            && self
                .get_key(&key)
                .is_some_and(|current| !matches!(current, Value::Nil))
        {
            self.shift_remove_key(&key);
            self.recycle_value_slot(Value::Nil);
        } else {
            self.insert_key(key, value);
        }
    }

    /// Assign a retained `C4V_C4ID(0)` mapped value through C4Value::Set.
    /// Unlike assigning canonical nil, the source type differs from a fresh
    /// `C4V_Any(0)` slot: Set canonicalizes it and CheckRemoveFromMap erases
    /// the entry. The only surviving case is Set's exact data/type early
    /// return when the destination slot already contains a retained zero ID.
    pub(crate) fn assign_key_zero_c4id(&mut self, key: Value) {
        if self
            .0
            .get(&key)
            .is_some_and(|current| matches!(current, Value::C4Id(id) if c4_id_raw(id) == 0))
        {
            return;
        }

        if self.0.shift_remove(&key).is_some() {
            self.1.push(Value::Nil);
            return;
        }

        if let Some(recycled) = self.1.pop() {
            if matches!(&recycled, Value::C4Id(id) if c4_id_raw(id) == 0) {
                self.0.insert(key, recycled);
            } else {
                self.1.push(Value::Nil);
            }
        } else {
            self.1.push(Value::Nil);
        }
    }

    /// String-key counterpart used by `map.property = retained_id_zero`.
    /// Preserve the native literal-string lookup fallback before delegating
    /// the missing-slot case to the arbitrary-key implementation.
    pub(crate) fn assign_zero_c4id(&mut self, key: String) {
        if self
            .get(&key)
            .is_some_and(|current| matches!(current, Value::C4Id(id) if c4_id_raw(id) == 0))
        {
            return;
        }
        if self.shift_remove(&key).is_some() {
            self.1.push(Value::Nil);
            return;
        }
        self.assign_key_zero_c4id(Value::from(key));
    }

    pub fn shift_remove_key(&mut self, key: &Value) -> Option<Value> {
        self.0.shift_remove(key)
    }
}

impl<K> Extend<(K, Value)> for ValueMap
where
    K: Into<Value>,
{
    fn extend<T>(&mut self, iter: T)
    where
        T: IntoIterator<Item = (K, Value)>,
    {
        for (key, value) in iter {
            self.insert_key(key.into(), value);
        }
    }
}

impl<K> FromIterator<(K, Value)> for ValueMap
where
    K: Into<Value>,
{
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = (K, Value)>,
    {
        let iter = iter.into_iter();
        let mut map = Self::with_capacity(iter.size_hint().0);
        map.extend(iter);
        map
    }
}

impl<K, const N: usize> From<[(K, Value); N]> for ValueMap
where
    K: Into<Value>,
{
    fn from(entries: [(K, Value); N]) -> Self {
        entries.into_iter().collect()
    }
}

impl<K> From<IndexMap<K, Value>> for ValueMap
where
    K: Into<Value>,
{
    fn from(entries: IndexMap<K, Value>) -> Self {
        entries.into_iter().collect()
    }
}

impl IntoIterator for ValueMap {
    type Item = (Value, Value);
    type IntoIter = indexmap::map::IntoIter<Value, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a ValueMap {
    type Item = (&'a Value, &'a Value);
    type IntoIter = indexmap::map::Iter<'a, Value, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut ValueMap {
    type Item = (&'a Value, &'a mut Value);
    type IntoIter = indexmap::map::IterMut<'a, Value, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl serde::Serialize for ValueMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.keys().all(|key| {
            matches!(key, Value::String(value) if std::str::from_utf8(c4_string_bytes_cow(value).as_ref()).is_ok())
        }) {
            use serde::ser::SerializeMap;

            let mut map = serializer.serialize_map(Some(self.len()))?;
            for (key, value) in self {
                let Value::String(key) = key else {
                    unreachable!("all map keys were checked as strings")
                };
                let canonical_key = String::from_utf8(c4_string_bytes(key))
                    .expect("all string keys were checked as UTF-8");
                map.serialize_entry(&canonical_key, value)?;
            }
            map.end()
        } else {
            use serde::ser::SerializeSeq;

            let mut entries = serializer.serialize_seq(Some(self.len()))?;
            for entry in self {
                entries.serialize_element(&entry)?;
            }
            entries.end()
        }
    }
}

impl<'de> serde::Deserialize<'de> for ValueMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Legacy(IndexMap<String, Value>),
            Entries(Vec<(Value, Value)>),
        }

        Ok(match <Repr as serde::Deserialize>::deserialize(deserializer)? {
            Repr::Legacy(entries) => entries
                .into_iter()
                .map(|(key, value)| (c4_string_from_literal(key), value))
                .collect(),
            Repr::Entries(entries) => entries.into_iter().collect(),
        })
    }
}

struct StringQuery<'a>(&'a str);

impl Hash for StringQuery<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        c4_string_hash(self.0).hash(state);
    }
}

impl Equivalent<Value> for StringQuery<'_> {
    fn equivalent(&self, key: &Value) -> bool {
        matches!(key, Value::String(value) if c4_strings_equal(value, self.0))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Int(i32),
    Bool(bool),
    /// A C4V_Bool whose pointer-width `C4V_Data` payload is not canonical 0/1.
    ///
    /// C++ casts change the type tag without normalizing the union payload, so
    /// `CastBool(7)` is truthy and must remain `b7` when compiled into a live
    /// save. On LP64, `CastBool(C4Id("4294967296"))` retains the high word too:
    /// it is truthy even though its low `Data.Int` (and saved value) is zero.
    /// Canonical Boolean values keep using [`Value::Bool`] so the public API
    /// remains ergonomic.
    RawBool(usize),
    String(C4StringValue),
    C4Id(#[serde(with = "c4_id_serde")] String),
    Object(u64),
    Array(Vec<Value>),
    Proplist(ValueMap),
    Nil,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Bool(left), Self::RawBool(right)) | (Self::RawBool(right), Self::Bool(left)) => {
                usize::from(*left) == *right
            }
            (Self::RawBool(left), Self::RawBool(right)) => left == right,
            (Self::String(left), Self::String(right)) => c4_strings_equal(left, right),
            (Self::C4Id(left), Self::C4Id(right)) => c4_id_raw(left) == c4_id_raw(right),
            (Self::Object(left), Self::Object(right)) => left == right,
            (Self::Array(left), Self::Array(right)) => left == right,
            (Self::Proplist(left), Self::Proplist(right)) => left == right,
            (Self::Nil, Self::Nil) => true,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.c4_value_hash().hash(state);
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(c4_string_from_literal(value).into())
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(c4_string_from_literal(value.to_owned()).into())
    }
}

impl Value {
    /// Build a C4V_Bool from its raw `Data.Int` representation, retaining
    /// noncanonical payloads while keeping 0 and 1 on the existing Bool API.
    pub fn from_c4_bool_raw(raw: i32) -> Self {
        Self::from_c4_bool_data_raw(raw as u32 as usize)
    }

    /// Retag one complete native `C4V_Data` payload as C4V_Bool.
    pub fn from_c4_bool_data_raw(raw: usize) -> Self {
        match raw {
            0 => Self::Bool(false),
            1 => Self::Bool(true),
            raw => Self::RawBool(raw),
        }
    }

    /// Return the exact C++ `Data.Int` payload for a C4V_Bool.
    pub fn c4_bool_raw(&self) -> Option<i32> {
        match self {
            Self::Bool(value) => Some(i32::from(*value)),
            Self::RawBool(value) => Some(*value as u32 as i32),
            _ => None,
        }
    }

    /// Return the complete pointer-width C++ `C4V_Data` payload for a bool.
    pub fn c4_bool_data_raw(&self) -> Option<usize> {
        match self {
            Self::Bool(value) => Some(usize::from(*value)),
            Self::RawBool(value) => Some(*value),
            _ => None,
        }
    }

    /// C4Script truthiness, matching C++ `C4Value::operator bool` (C4Value.h:185
    /// → `C4V_Data::operator bool`, :76): raw-nonzero on the `Data` union. For
    /// strings/arrays/proplists that is a *pointer*, so a non-nil one is truthy
    /// even when empty. Nil, integer/bool zero, null objects, and the raw-zero
    /// C4IDs `NONE`/`0000` are falsy.
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::RawBool(raw) => *raw != 0,
            Value::Int(i) => *i != 0,
            Value::String(_) => true,
            Value::C4Id(id) => c4_id_raw(id) != 0,
            Value::Object(id) => *id != 0,
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
            Value::RawBool(raw) => Some(*raw as u32 as i32),
            Value::Nil => Some(0),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Bool(_) => "bool",
            Value::RawBool(_) => "bool",
            Value::String(_) => "string",
            Value::C4Id(_) => "id",
            Value::Object(_) => "object",
            Value::Array(_) => "array",
            Value::Proplist(_) => "map",
            Value::Nil => "nil",
        }
    }

    /// Deterministic map-key hash matching C++ `std::hash<C4Value>`
    /// (`C4Value.cpp:965-1029`). This is intentionally separate from Rust's
    /// `Hash` trait so it never flows through randomized `HashMap` state.
    pub fn c4_value_hash(&self) -> usize {
        match self {
            Value::Int(value) => c4_hash_typed(C4V_INT, hash_i32(*value)),
            Value::Bool(value) => c4_hash_typed(C4V_BOOL, *value as usize),
            // std::hash<C4Value> hashes C4V_Bool through `_getBool()`, so
            // the low Data.Int determines `_getBool()`. A high-word-only raw
            // payload therefore hashes like false even though control-flow
            // truth testing observes the complete union and treats it true.
            Value::RawBool(value) => {
                c4_hash_typed(C4V_BOOL, usize::from((*value as u32 as i32) != 0))
            }
            Value::String(value) => c4_string_hash(value),
            Value::C4Id(value) => c4_hash_typed(C4V_C4ID, hash_i32(c4_id_raw(value) as i32)),
            Value::Object(id) => c4_hash_typed(C4V_C4OBJECT, *id as usize),
            Value::Array(values) => values.iter().fold(C4V_ARRAY, |hash, value| {
                c4_hash_combine(hash, value.c4_value_hash())
            }),
            Value::Proplist(entries) => {
                let content_hash = entries.iter().fold(0, |content_hash, (key, value)| {
                    let item_hash = c4_hash_combine(key.c4_value_hash(), value.c4_value_hash());
                    content_hash ^ item_hash
                });
                c4_hash_combine(C4V_MAP, content_hash)
            }
            Value::Nil => C4V_ANY,
        }
    }

    /// The `C4V_Type` this value presents to the conversion table
    /// (`C4ScriptCnvMap`, indexed by `C4V_Type`, C4Value.h:37-54). The Rust
    /// value model tags every value with its concrete type, so the only value
    /// that maps to `C4V_Any` is `Nil` (a C++ `C4V_Any` whose `Data == 0`).
    /// `C4V_pC4Value` is represented internally by VM lvalue handles, not as a
    /// public [`Value`] variant.
    pub fn c4v_type(&self) -> C4VType {
        match self {
            Value::Nil => C4VType::Any,
            Value::Int(_) => C4VType::Int,
            Value::Bool(_) => C4VType::Bool,
            Value::RawBool(_) => C4VType::Bool,
            Value::C4Id(_) => C4VType::C4Id,
            Value::Object(_) => C4VType::C4Object,
            Value::String(_) => C4VType::String,
            Value::Array(_) => C4VType::Array,
            Value::Proplist(_) => C4VType::Map,
        }
    }

    /// Mirror C++ `C4Value::ConvertTo` (C4Value.h:248-254): can this value
    /// convert to `to_type` under the `#strict` flag? Dispatches the
    /// `C4ScriptCnvMap[from][to]` converter (C4Value.cpp:431-598).
    pub fn convert_to(&self, to_type: C4VType, strict: bool) -> bool {
        match cnv_fn(self.c4v_type(), to_type) {
            // CnvOK is a null function pointer; `ConvertTo` then returns true.
            CnvFn::Ok => true,
            // FnCnvError (C4Value.cpp:439-443): deny unconditionally.
            CnvFn::Error => false,
            // FnCnvDirectOld (C4Value.cpp:431-437): fail under #strict, else
            // a no-op success.
            CnvFn::DirectOld => !strict,
            // FnCnvInt2Id (C4Value.cpp:469-478): an int in 0..=9999 becomes an
            // id. Only the [Int][C4ID] cell uses this, so `self` is an `Int`.
            CnvFn::Int2Id => matches!(self, Value::Int(i) if (0..=9999).contains(i)),
            // FnCnvGuess (C4Value.cpp:453-467): a non-zero `C4V_Any` guesses its
            // type and retries, while nil (Data == 0) "is every possible type
            // except a reference" and converts unconditionally. The eager Rust
            // value model only ever presents `Nil` as `C4V_Any`, so the
            // Game-dependent `GuessType` path (C4Value.cpp:299-331) is
            // unreachable here and the nil branch always applies.
            CnvFn::Guess => true,
            // FnCnvDeref (C4Value.cpp:445-451): resolve the reference and retry.
            // No Rust value is a `C4V_pC4Value`, so this cell is never selected
            // by `c4v_type()`.
            CnvFn::Deref => false,
        }
    }

    /// The mutating half of C++ `C4Value::ConvertTo`. Most successful table
    /// cells only validate the value, but `FnCnvInt2Id` also changes its type
    /// tag. Function-parameter checking needs that mutation to be visible in
    /// the callee (`GetType(1000)` on an `id` parameter observes C4V_C4ID).
    pub(crate) fn convert_to_in_place(&mut self, to_type: C4VType, strict: bool) -> bool {
        let conversion = cnv_fn(self.c4v_type(), to_type);
        if !self.convert_to(to_type, strict) {
            return false;
        }
        if conversion == CnvFn::Int2Id {
            let Value::Int(i) = self else {
                unreachable!("only int values select FnCnvInt2Id");
            };
            let id = if *i == 0 {
                "NONE".to_string()
            } else {
                format!("{i:04}")
            };
            *self = Value::C4Id(id);
        }
        true
    }
}

/// Boost container_hash-style combiner copied from C++ `C4Value.cpp:923-960`.
pub fn c4_hash_combine(hash: usize, next_hash: usize) -> usize {
    c4_hash_combine_inner(hash, next_hash)
}

#[cfg(target_pointer_width = "32")]
fn c4_hash_combine_inner(mut hash: usize, mut next_hash: usize) -> usize {
    const C1: usize = 0xcc9e2d51;
    const C2: usize = 0x1b873593;

    next_hash = next_hash.wrapping_mul(C1);
    next_hash = next_hash.rotate_left(15);
    next_hash = next_hash.wrapping_mul(C2);

    hash ^= next_hash;
    hash = hash.rotate_left(13);
    hash.wrapping_mul(5).wrapping_add(0xe6546b64)
}

#[cfg(target_pointer_width = "64")]
fn c4_hash_combine_inner(mut hash: usize, mut next_hash: usize) -> usize {
    const M: usize = 0xc6a4a7935bd1e995;
    const R: usize = 47;

    next_hash = next_hash.wrapping_mul(M);
    next_hash ^= next_hash >> R;
    next_hash = next_hash.wrapping_mul(M);

    hash ^= next_hash;
    hash = hash.wrapping_mul(M);
    hash.wrapping_add(0xe6546b64)
}

#[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
fn c4_hash_combine_inner(hash: usize, next_hash: usize) -> usize {
    hash ^ next_hash
        .wrapping_add(0x9e3779b9)
        .wrapping_add(hash << 6)
        .wrapping_add(hash >> 2)
}

fn c4_hash_typed(type_hash: usize, value_hash: usize) -> usize {
    c4_hash_combine(type_hash, value_hash)
}

fn c4_string_hash(value: &str) -> usize {
    let bytes = c4_string_bytes_cow(value);
    c4_hash_typed(C4V_STRING, cpp_string_view_hash(bytes.as_ref()))
}

fn hash_i32(value: i32) -> usize {
    value as usize
}

pub fn c4_id_raw(id: &str) -> usize {
    if let Some(raw) = c4_id_tagged_raw(id) {
        return raw;
    }
    c4_id_parse(id)
}

/// Parse a C4 string through `C4Id(std::string_view)` without recognizing
/// the Rust-only typed-ID storage tag.
pub fn c4_id_parse(id: &str) -> usize {
    let native_bytes = c4_string_bytes_cow(id);
    // Script-facing C4Id receives a native C string (`FnStringPar`), so an
    // embedded NUL terminates the argument before C4Id(std::string_view).
    let bytes = native_bytes
        .split(|byte| *byte == 0)
        .next()
        .unwrap_or_default();
    if bytes.len() < 4 || bytes == b"NONE" {
        return 0;
    }

    let mut raw = 0usize;
    let mut numeric = true;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            numeric = false;
            break;
        }
        raw = c4_id_normalize_raw(raw.wrapping_mul(10).wrapping_add((*byte - b'0') as usize));
    }
    if numeric {
        return raw;
    }

    bytes.iter().take(4).rev().fold(0usize, |raw, byte| {
        // C4Id casts the platform's signed plain `char` directly to
        // unsigned long. High bytes therefore sign-extend before the
        // OR on every supported LegacyClonk target.
        let byte = c4_id_normalize_raw(*byte as i8 as isize as usize);
        c4_id_normalize_raw(raw.wrapping_shl(8) | byte)
    })
}

// `std::hash<std::string_view>` in libc++ dispatches to Murmur2 on 32-bit and
// CityHash64 on 64-bit; C4Value.cpp feeds that hash into `hashCombine`.
#[cfg(target_pointer_width = "32")]
fn cpp_string_view_hash(bytes: &[u8]) -> usize {
    const M: u32 = 0x5bd1e995;
    const R: u32 = 24;

    let mut hash = bytes.len() as u32;
    let mut offset = 0;
    while bytes.len() - offset >= 4 {
        let mut k = load_u32(bytes, offset);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        hash = hash.wrapping_mul(M);
        hash ^= k;
        offset += 4;
    }

    match bytes.len() - offset {
        3 => {
            hash ^= (bytes[offset + 2] as u32) << 16;
            hash ^= (bytes[offset + 1] as u32) << 8;
            hash ^= bytes[offset] as u32;
            hash = hash.wrapping_mul(M);
        }
        2 => {
            hash ^= (bytes[offset + 1] as u32) << 8;
            hash ^= bytes[offset] as u32;
            hash = hash.wrapping_mul(M);
        }
        1 => {
            hash ^= bytes[offset] as u32;
            hash = hash.wrapping_mul(M);
        }
        _ => {}
    }

    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^= hash >> 15;
    hash as usize
}

#[cfg(target_pointer_width = "64")]
fn cpp_string_view_hash(bytes: &[u8]) -> usize {
    cityhash64(bytes) as usize
}

fn load_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut word = [0; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_ne_bytes(word)
}

#[cfg(target_pointer_width = "64")]
fn load_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut word = [0; 8];
    word.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_ne_bytes(word)
}

#[cfg(target_pointer_width = "64")]
fn cityhash64(bytes: &[u8]) -> u64 {
    let len = bytes.len();
    if len <= 32 {
        return if len <= 16 {
            city_hash_len_0_to_16(bytes)
        } else {
            city_hash_len_17_to_32(bytes)
        };
    }
    if len <= 64 {
        return city_hash_len_33_to_64(bytes);
    }

    let mut x = load_u64(bytes, len - 40);
    let mut y = load_u64(bytes, len - 16).wrapping_add(load_u64(bytes, len - 56));
    let mut z = city_hash_len_16(
        load_u64(bytes, len - 48).wrapping_add(len as u64),
        load_u64(bytes, len - 24),
    );
    let mut v = weak_hash_len_32_with_seeds(bytes, len - 64, len as u64, z);
    let mut w = weak_hash_len_32_with_seeds(bytes, len - 32, y.wrapping_add(CITY_K1), x);
    x = x.wrapping_mul(CITY_K1).wrapping_add(load_u64(bytes, 0));

    let mut offset = 0;
    let mut remaining = (len - 1) & !63;
    while remaining != 0 {
        x = city_rotate(
            x.wrapping_add(y)
                .wrapping_add(v.0)
                .wrapping_add(load_u64(bytes, offset + 8)),
            37,
        )
        .wrapping_mul(CITY_K1);
        y = city_rotate(
            y.wrapping_add(v.1)
                .wrapping_add(load_u64(bytes, offset + 48)),
            42,
        )
        .wrapping_mul(CITY_K1);
        x ^= w.1;
        y = y
            .wrapping_add(v.0)
            .wrapping_add(load_u64(bytes, offset + 40));
        z = city_rotate(z.wrapping_add(w.0), 33).wrapping_mul(CITY_K1);
        v = weak_hash_len_32_with_seeds(
            bytes,
            offset,
            v.1.wrapping_mul(CITY_K1),
            x.wrapping_add(w.0),
        );
        w = weak_hash_len_32_with_seeds(
            bytes,
            offset + 32,
            z.wrapping_add(w.1),
            y.wrapping_add(load_u64(bytes, offset + 16)),
        );
        std::mem::swap(&mut z, &mut x);
        offset += 64;
        remaining -= 64;
    }

    city_hash_len_16(
        city_hash_len_16(v.0, w.0)
            .wrapping_add(city_shift_mix(y).wrapping_mul(CITY_K1))
            .wrapping_add(z),
        city_hash_len_16(v.1, w.1).wrapping_add(x),
    )
}

#[cfg(target_pointer_width = "64")]
const CITY_K0: u64 = 0xc3a5c85c97cb3127;
#[cfg(target_pointer_width = "64")]
const CITY_K1: u64 = 0xb492b66fbe98f273;
#[cfg(target_pointer_width = "64")]
const CITY_K2: u64 = 0x9ae16a3b2f90404f;
#[cfg(target_pointer_width = "64")]
const CITY_K3: u64 = 0xc949d7c7509e6557;

#[cfg(target_pointer_width = "64")]
fn city_rotate(value: u64, shift: u32) -> u64 {
    if shift == 0 {
        value
    } else {
        value.rotate_right(shift)
    }
}

#[cfg(target_pointer_width = "64")]
fn city_shift_mix(value: u64) -> u64 {
    value ^ (value >> 47)
}

#[cfg(target_pointer_width = "64")]
fn city_hash_len_16(u: u64, v: u64) -> u64 {
    const MUL: u64 = 0x9ddfea08eb382d69;
    let mut a = (u ^ v).wrapping_mul(MUL);
    a ^= a >> 47;
    let mut b = (v ^ a).wrapping_mul(MUL);
    b ^= b >> 47;
    b.wrapping_mul(MUL)
}

#[cfg(target_pointer_width = "64")]
fn city_hash_len_0_to_16(bytes: &[u8]) -> u64 {
    let len = bytes.len();
    if len > 8 {
        let a = load_u64(bytes, 0);
        let b = load_u64(bytes, len - 8);
        return city_hash_len_16(a, city_rotate(b.wrapping_add(len as u64), len as u32)) ^ b;
    }
    if len >= 4 {
        let a = load_u32(bytes, 0);
        let b = load_u32(bytes, len - 4);
        return city_hash_len_16((len as u64).wrapping_add((a << 3) as u64), b as u64);
    }
    if len > 0 {
        let a = bytes[0] as u32;
        let b = bytes[len >> 1] as u32;
        let c = bytes[len - 1] as u32;
        let y = a + (b << 8);
        let z = len as u32 + (c << 2);
        return city_shift_mix((y as u64).wrapping_mul(CITY_K2) ^ (z as u64).wrapping_mul(CITY_K3))
            .wrapping_mul(CITY_K2);
    }
    CITY_K2
}

#[cfg(target_pointer_width = "64")]
fn city_hash_len_17_to_32(bytes: &[u8]) -> u64 {
    let len = bytes.len();
    let a = load_u64(bytes, 0).wrapping_mul(CITY_K1);
    let b = load_u64(bytes, 8);
    let c = load_u64(bytes, len - 8).wrapping_mul(CITY_K2);
    let d = load_u64(bytes, len - 16).wrapping_mul(CITY_K0);
    city_hash_len_16(
        city_rotate(a.wrapping_sub(b), 43)
            .wrapping_add(city_rotate(c, 30))
            .wrapping_add(d),
        a.wrapping_add(city_rotate(b ^ CITY_K3, 20))
            .wrapping_sub(c)
            .wrapping_add(len as u64),
    )
}

#[cfg(target_pointer_width = "64")]
fn weak_hash_len_32_with_seeds_words(
    w: u64,
    x: u64,
    y: u64,
    z: u64,
    mut a: u64,
    mut b: u64,
) -> (u64, u64) {
    a = a.wrapping_add(w);
    b = city_rotate(b.wrapping_add(a).wrapping_add(z), 21);
    let c = a;
    a = a.wrapping_add(x).wrapping_add(y);
    b = b.wrapping_add(city_rotate(a, 44));
    (a.wrapping_add(z), b.wrapping_add(c))
}

#[cfg(target_pointer_width = "64")]
fn weak_hash_len_32_with_seeds(bytes: &[u8], offset: usize, a: u64, b: u64) -> (u64, u64) {
    weak_hash_len_32_with_seeds_words(
        load_u64(bytes, offset),
        load_u64(bytes, offset + 8),
        load_u64(bytes, offset + 16),
        load_u64(bytes, offset + 24),
        a,
        b,
    )
}

#[cfg(target_pointer_width = "64")]
fn city_hash_len_33_to_64(bytes: &[u8]) -> u64 {
    let len = bytes.len();
    let mut z = load_u64(bytes, 24);
    let mut a = load_u64(bytes, 0).wrapping_add(
        (len as u64)
            .wrapping_add(load_u64(bytes, len - 16))
            .wrapping_mul(CITY_K0),
    );
    let mut b = city_rotate(a.wrapping_add(z), 52);
    let mut c = city_rotate(a, 37);
    a = a.wrapping_add(load_u64(bytes, 8));
    c = c.wrapping_add(city_rotate(a, 7));
    a = a.wrapping_add(load_u64(bytes, 16));
    let vf = a.wrapping_add(z);
    let vs = b.wrapping_add(city_rotate(a, 31)).wrapping_add(c);
    a = load_u64(bytes, 16).wrapping_add(load_u64(bytes, len - 32));
    z = z.wrapping_add(load_u64(bytes, len - 8));
    b = city_rotate(a.wrapping_add(z), 52);
    c = city_rotate(a, 37);
    a = a.wrapping_add(load_u64(bytes, len - 24));
    c = c.wrapping_add(city_rotate(a, 7));
    a = a.wrapping_add(load_u64(bytes, len - 16));
    let wf = a.wrapping_add(z);
    let ws = b.wrapping_add(city_rotate(a, 31)).wrapping_add(c);
    let r = city_shift_mix(
        vf.wrapping_add(ws)
            .wrapping_mul(CITY_K2)
            .wrapping_add(wf.wrapping_add(vs).wrapping_mul(CITY_K0)),
    );
    city_shift_mix(r.wrapping_mul(CITY_K0).wrapping_add(vs)).wrapping_mul(CITY_K2)
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
            Literal::String(s) => Value::String(s.into()),
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
            Value::RawBool(raw) => write!(f, "{}", *raw != 0),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::C4Id(id) => write!(f, "{}", c4_id_text(id)),
            Value::Object(id) => write!(f, "<object {id}>"),
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
                let mut first = true;
                write!(f, "{{")?;
                for (key, value) in entries {
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

#[cfg(test)]
mod map_tests {
    use super::*;

    #[test]
    fn raw_bool_keeps_union_payload_but_hashes_native_truth_value() {
        assert_eq!(Value::from_c4_bool_raw(0), Value::Bool(false));
        assert_eq!(Value::from_c4_bool_raw(1), Value::Bool(true));

        let seven = Value::from_c4_bool_raw(7);
        let two = Value::from_c4_bool_raw(2);
        assert_eq!(seven, Value::RawBool(7));
        assert_eq!(seven.as_c4_int(), Some(7));
        assert!(seven.as_bool());
        assert_ne!(seven, two, "C4Value operator== compares raw union data");
        assert_eq!(
            seven.c4_value_hash(),
            two.c4_value_hash(),
            "std::hash<C4Value> hashes bool through _getBool()"
        );
        assert_eq!(seven.c4_value_hash(), Value::Bool(true).c4_value_hash());

        #[cfg(target_pointer_width = "64")]
        {
            let high_word = 1_usize << 32;
            let wide = Value::from_c4_bool_data_raw(high_word);
            assert!(wide.as_bool(), "control flow reads the complete C4V_Data");
            assert_eq!(wide.as_c4_int(), Some(0), "_getInt reads Data.Int");
            assert_eq!(wide.c4_bool_raw(), Some(0));
            assert_eq!(wide.c4_bool_data_raw(), Some(high_word));
            assert_eq!(
                wide.c4_value_hash(),
                Value::Bool(false).c4_value_hash(),
                "native bool hashing reads _getBool from the low Data.Int"
            );
        }

        let encoded = serde_json::to_string(&seven).expect("raw bool serializes");
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).expect("raw bool deserializes"),
            seven
        );
    }

    #[test]
    fn arbitrary_keys_coexist_with_string_properties_in_insertion_order() {
        let mut map = ValueMap::new();
        map.insert("name".into(), Value::Int(1));
        map.insert_key(Value::Int(42), Value::String("answer".into()));
        map.insert_key(Value::Bool(true), Value::Int(3));

        assert_eq!(map.get("name"), Some(&Value::Int(1)));
        assert_eq!(
            map.get_key(&Value::Int(42)),
            Some(&Value::String("answer".into()))
        );
        assert!(map.contains_key("name"));
        assert!(map.contains_value_key(&Value::Bool(true)));
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            vec![
                Value::String("name".into()),
                Value::Int(42),
                Value::Bool(true)
            ]
        );

        // Replacement keeps the original key position. Removal followed by
        // reinsertion appends, matching C4ValueHash::keyOrder.
        map.insert_key(Value::Int(42), Value::Int(2));
        assert_eq!(map.keys().nth(1), Some(&Value::Int(42)));
        assert_eq!(map.shift_remove_key(&Value::Int(42)), Some(Value::Int(2)));
        map.insert_key(Value::Int(42), Value::Int(4));
        assert_eq!(map.keys().last(), Some(&Value::Int(42)));
    }

    #[test]
    fn removed_key_value_slots_are_reused_lifo_like_c4valuehash() {
        let mut map = ValueMap::new();
        map.recycle_value_slot(Value::String("older".into()));
        map.recycle_value_slot(Value::String("newer".into()));

        let cloned = map.clone();
        assert_eq!(
            cloned.hidden_values().cloned().collect::<Vec<_>>(),
            vec![Value::String("newer".into()), Value::String("older".into())]
        );
        assert_eq!(cloned, ValueMap::new(), "hidden slots do not affect equality");

        let registrations = crate::new_string_registrations();
        crate::register_c4_value_strings(
            &registrations,
            &Value::Proplist(cloned.clone()),
        );
        assert_eq!(
            crate::c4_string_registration_order(&registrations),
            vec!["newer".to_string(), "older".to_string()],
            "hidden slots remain live C4Values for string traversal"
        );

        map.insert_key(Value::Int(1), Value::Int(10));
        assert_eq!(map.get_key(&Value::Int(1)), Some(&Value::Int(10)));
        assert_eq!(map.1, vec![Value::String("older".into())]);

        // Reusing a nonnil slot and assigning nil changes that slot to nil,
        // which immediately removes the just-created entry again.
        map.assign_key(Value::Int(2), Value::Nil);
        assert!(!map.contains_value_key(&Value::Int(2)));
        assert_eq!(map.1, vec![Value::Nil]);

        // Reusing an already-nil slot takes C4Value::Set's early return, so
        // the nil entry remains present and the pool is consumed.
        map.assign_key(Value::Int(3), Value::Nil);
        assert_eq!(map.get_key(&Value::Int(3)), Some(&Value::Nil));
        assert!(map.1.is_empty());
    }

    #[test]
    fn string_property_queries_canonicalize_reserved_marker_literals() {
        let literal = char::from_u32(C4_RAW_BYTE_ESCAPE_BASE + 0x80)
            .expect("reserved marker is a valid scalar")
            .to_string();
        let mut map = ValueMap::new();
        map.insert(literal.clone(), Value::Int(1));

        assert_eq!(map.get(&literal), Some(&Value::Int(1)));
        assert!(map.contains_key(&literal));
        *map.get_mut(&literal).expect("reserved-marker key is mutable") = Value::Int(2);
        assert_eq!(map.shift_remove(&literal), Some(Value::Int(2)));
        assert!(!map.contains_key(&literal));

        let projected_byte = c4_string_from_bytes(&[0x80]);
        map.insert_key(Value::String(projected_byte.clone().into()), Value::Int(3));
        assert_eq!(map.get(&projected_byte), Some(&Value::Int(3)));
        assert!(map.contains_key(&projected_byte));
        *map.get_mut(&projected_byte)
            .expect("projected-byte key is mutable") = Value::Int(4);
        assert_eq!(map.shift_remove(&projected_byte), Some(Value::Int(4)));
    }

    #[test]
    fn map_equality_and_c4_hash_ignore_insertion_order() {
        let forward = Value::Proplist(ValueMap::from([
            (Value::Int(7), Value::String("seven".into())),
            (Value::String("flag".into()), Value::Bool(true)),
        ]));
        let reverse = Value::Proplist(ValueMap::from([
            (Value::String("flag".into()), Value::Bool(true)),
            (Value::Int(7), Value::String("seven".into())),
        ]));

        assert_eq!(forward, reverse);
        assert_eq!(forward.c4_value_hash(), reverse.c4_value_hash());
    }

    #[test]
    fn serde_keeps_legacy_string_map_shape() {
        let map = ValueMap::from([("alpha".to_string(), Value::Int(1))]);
        let encoded = serde_json::to_string(&map).expect("string map serializes");
        assert_eq!(encoded, r#"{"alpha":{"Int":1}}"#);

        let decoded: ValueMap = serde_json::from_str(&encoded).expect("string map deserializes");
        assert_eq!(decoded, map);

        let legacy: ValueMap = serde_json::from_str(r#"{"old":{"Bool":true}}"#)
            .expect("legacy object representation remains accepted");
        assert_eq!(legacy.get("old"), Some(&Value::Bool(true)));
    }

    #[test]
    fn serde_round_trips_non_string_keys_as_ordered_entries() {
        let map = ValueMap::from([
            (Value::Int(42), Value::String("answer".into())),
            (Value::C4Id("CLNK".into()), Value::Object(7)),
            (Value::Bool(false), Value::Int(0)),
        ]);
        let encoded = serde_json::to_value(&map).expect("arbitrary-key map serializes");
        assert!(
            encoded.is_array(),
            "non-string keys require an entry sequence"
        );

        let decoded: ValueMap =
            serde_json::from_value(encoded).expect("arbitrary-key map deserializes");
        assert_eq!(decoded, map);
        assert_eq!(
            decoded.keys().cloned().collect::<Vec<_>>(),
            map.keys().cloned().collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod c4_string_tests {
    use super::*;

    #[test]
    fn native_byte_view_borrows_text_and_decodes_only_raw_byte_escapes() {
        let ascii = "Clonk";
        assert!(matches!(
            c4_string_bytes_cow(ascii),
            Cow::Borrowed(bytes) if bytes == ascii.as_bytes()
        ));

        let utf8 = "Clönk";
        assert!(matches!(
            c4_string_bytes_cow(utf8),
            Cow::Borrowed(bytes) if bytes == utf8.as_bytes()
        ));

        let escaped_utf8 = [0xc3_u8, 0xb6]
            .into_iter()
            .map(|byte| {
                char::from_u32(C4_RAW_BYTE_ESCAPE_BASE + u32::from(byte))
                    .expect("raw-byte escape is a valid scalar")
            })
            .collect::<String>();
        assert!(matches!(
            c4_string_bytes_cow(&escaped_utf8),
            Cow::Owned(bytes) if bytes == "ö".as_bytes()
        ));
        assert!(c4_strings_equal("ö", &escaped_utf8));
        assert_eq!(c4_string_bytes(&escaped_utf8), "ö".as_bytes());

        let utf8_value = Value::String("ö".into());
        let escaped_value = Value::String(escaped_utf8.into());
        assert_eq!(utf8_value, escaped_value);
        let mut values = ValueMap::new();
        values.insert_key(escaped_value, Value::Int(1));
        assert_eq!(values.get_key(&utf8_value), Some(&Value::Int(1)));
        assert_eq!(
            serde_json::to_string(&utf8_value).expect("UTF-8 value serializes"),
            serde_json::to_string(
                values
                    .keys()
                    .next()
                    .expect("raw-escape key remains present")
            )
            .expect("raw-escape value serializes")
        );
    }

    #[test]
    fn raw_byte_projection_round_trips_high_bytes_without_rewriting_utf8_literals() {
        let raw = [0, b'A', 0x7f, 0x80, 0xfe, 0xff];
        let projected = c4_string_from_bytes(&raw);
        assert_eq!(c4_string_bytes(&projected), raw);
        assert_eq!(c4_string_byte_len(&projected), raw.len());
        assert_eq!(c4_string_byte(&projected, 4), Some(0xfe));

        let utf8_literal = "A\u{ff}";
        assert_eq!(c4_string_bytes(utf8_literal), utf8_literal.as_bytes());
        assert_ne!(projected, utf8_literal);
        assert_eq!(c4_string_from_bytes("\u{ff}".as_bytes()), "\u{ff}");
    }

    #[test]
    fn raw_byte_projection_keeps_equality_hashing_and_literal_collisions_byte_exact() {
        let utf8_literal = Value::String("\u{ff}".into());
        let equivalent_raw_bytes =
            Value::String(c4_string_from_bytes("\u{ff}".as_bytes()).into());
        let single_high_byte = Value::String(c4_string_from_bytes(&[0xff]).into());

        assert_eq!(utf8_literal, equivalent_raw_bytes);
        assert_ne!(utf8_literal, single_high_byte);
        assert_eq!(
            serde_json::to_string(&utf8_literal).expect("UTF-8 spelling serializes"),
            serde_json::to_string(&equivalent_raw_bytes).expect("raw spelling serializes"),
            "byte-equal strings have one canonical wire spelling"
        );

        let mut map = ValueMap::new();
        map.insert_key(equivalent_raw_bytes, Value::Int(7));
        assert_eq!(map.get_key(&utf8_literal), Some(&Value::Int(7)));
        assert_eq!(map.get_key(&single_high_byte), None);
        let encoded_map = serde_json::to_string(&map).expect("string-key map serializes");
        let mut equivalent_map = ValueMap::new();
        equivalent_map.insert_key(utf8_literal.clone(), Value::Int(7));
        assert_eq!(
            encoded_map,
            serde_json::to_string(&equivalent_map).expect("equivalent map serializes")
        );

        let private_use_literal = char::from_u32(C4_RAW_BYTE_ESCAPE_BASE + 0xff)
            .expect("private-use literal is a valid scalar")
            .to_string();
        let canonical_literal = c4_string_from_literal(private_use_literal.clone());
        assert_eq!(
            c4_string_bytes(&canonical_literal),
            private_use_literal.as_bytes()
        );
        assert_ne!(Value::String(canonical_literal.into()), single_high_byte);

        let encoded = serde_json::to_string(&single_high_byte)
            .expect("raw-byte string serializes");
        let decoded: Value =
            serde_json::from_str(&encoded).expect("raw-byte string deserializes");
        assert_eq!(decoded, single_high_byte);
        let Value::String(decoded) = decoded else {
            panic!("round-trip preserves the string variant");
        };
        assert_eq!(c4_string_bytes(&decoded), [0xff]);
    }

    #[test]
    fn c4id_payloads_use_signed_char_parsing_and_canonical_serde() {
        assert_eq!(
            c4_id_raw(&c4_string_from_bytes(&[0xff, b'A', b'B', b'C'])),
            c4_id_normalize_raw(usize::MAX)
        );
        assert_eq!(
            c4_id_parse(&c4_string_from_bytes(b"ABC\0D")),
            0,
            "FnC4Id sees only the native C-string prefix"
        );
        assert_eq!(c4_id_parse(&c4_string_from_bytes(b"1234\0suffix")), 1234);

        #[cfg(all(not(target_os = "windows"), target_pointer_width = "64"))]
        assert_eq!(c4_id_parse("4294967297"), 4_294_967_297);
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        {
            assert_eq!(c4_id_parse("4294967297"), 1);
            assert_eq!(c4_id_raw(&c4_id_from_raw(4_294_967_297)), 1);
        }

        let parsed_numeric = Value::C4Id("12345".into());
        let typed_payload = Value::C4Id(c4_id_from_raw(12345));
        assert_eq!(parsed_numeric, typed_payload);
        assert_ne!(
            c4_id_parse(match &typed_payload {
                Value::C4Id(id) => id,
                _ => unreachable!(),
            }),
            12345,
            "ordinary string parsing must not recognize the typed-ID tag"
        );
        assert_eq!(c4_id_text("12345"), c4_string_from_bytes(b"90"));
        assert_eq!(typed_payload.to_string(), "90");

        let parsed_wire = serde_json::to_string(&parsed_numeric).expect("C4ID serializes");
        let typed_wire = serde_json::to_string(&typed_payload).expect("C4ID serializes");
        assert_eq!(parsed_wire, typed_wire);
        let decoded: Value = serde_json::from_str(&parsed_wire).expect("C4ID deserializes");
        assert_eq!(decoded, typed_payload);
        let Value::C4Id(decoded) = decoded else {
            panic!("round-trip preserves the C4ID variant");
        };
        assert_eq!(c4_id_raw(&decoded), 12345);

        for raw in [u32::from_le_bytes(*b"NONE"), u32::from_le_bytes(*b"1111")] {
            let stored = c4_id_from_raw(raw as usize);
            assert_eq!(c4_id_raw(&stored), raw as usize);
        }
    }
}

#[cfg(test)]
mod cnv_tests {
    use super::*;

    // Each Rust value presents the `C4V_Type` the conversion table is indexed
    // by (C4Value.h:37-54). The eager Rust value model only ever maps `Nil` to
    // `C4V_Any`; there is no `C4V_pC4Value` public value representation.
    #[test]
    fn value_reports_its_c4v_type() {
        assert_eq!(Value::Nil.c4v_type(), C4VType::Any);
        assert_eq!(Value::Int(0).c4v_type(), C4VType::Int);
        assert_eq!(Value::Bool(true).c4v_type(), C4VType::Bool);
        assert_eq!(Value::C4Id("CLNK".into()).c4v_type(), C4VType::C4Id);
        assert_eq!(Value::Object(42).c4v_type(), C4VType::C4Object);
        assert_eq!(Value::String("x".into()).c4v_type(), C4VType::String);
        assert_eq!(Value::Array(vec![]).c4v_type(), C4VType::Array);
        assert_eq!(Value::Proplist(ValueMap::new()).c4v_type(), C4VType::Map);
    }

    #[test]
    fn object_values_report_cpp_type_name() {
        assert_eq!(Value::Object(42).type_name(), "object");
    }

    // Representative cells of C4ScriptCnvMap (C4Value.cpp:488-598).
    #[test]
    fn cnv_map_classifies_cells_like_cpp() {
        use C4VType::*;
        // C4V_Any row: same-type is OK, everything but a reference guesses
        // (C4Value.cpp:490-501).
        assert_eq!(cnv_fn(Any, Any), CnvFn::Ok);
        assert_eq!(cnv_fn(Any, Int), CnvFn::Guess);
        assert_eq!(cnv_fn(Any, Ref), CnvFn::Error);
        // int -> id is the numeric-ID guard (C4Value.cpp:507).
        assert_eq!(cnv_fn(Int, C4Id), CnvFn::Int2Id);
        // bool/id/object/string -> int is #strict-forbidden old syntax
        // (C4Value.cpp:519,529,541,553).
        assert_eq!(cnv_fn(Bool, C4Id), CnvFn::DirectOld);
        assert_eq!(cnv_fn(C4Id, Int), CnvFn::DirectOld);
        assert_eq!(cnv_fn(String, Int), CnvFn::DirectOld);
        // array/map never coerce to int (C4Value.cpp:565,577).
        assert_eq!(cnv_fn(Array, Int), CnvFn::Error);
        assert_eq!(cnv_fn(Map, Int), CnvFn::Error);
        // every type is truthy-convertible to bool except a reference, which
        // dereferences first (C4Value.cpp:566,578,590).
        assert_eq!(cnv_fn(Array, Bool), CnvFn::Ok);
        assert_eq!(cnv_fn(Map, Bool), CnvFn::Ok);
        assert_eq!(cnv_fn(Ref, Bool), CnvFn::Deref);
        // the reference row derefs then retries; same-type is OK
        // (C4Value.cpp:586-596).
        assert_eq!(cnv_fn(Ref, Map), CnvFn::Deref);
        assert_eq!(cnv_fn(Ref, Ref), CnvFn::Ok);
    }

    // Only CnvError / CnvDirectOld carry the Warn flag (C4Value.cpp:481-486).
    #[test]
    fn cnv_warn_flag_matches_macros() {
        assert!(!CnvFn::Ok.warns());
        assert!(CnvFn::Error.warns());
        assert!(!CnvFn::Guess.warns());
        assert!(!CnvFn::Int2Id.warns());
        assert!(CnvFn::DirectOld.warns());
        assert!(!CnvFn::Deref.warns());
    }

    // ConvertTo dispatch (C4Value.h:248-254 + the converter fns C4Value.cpp:431-478).
    #[test]
    fn convert_to_dispatches_like_cpp() {
        use C4VType::*;
        // nil "is every possible type except a reference" (FnCnvGuess,
        // C4Value.cpp:453-467 — Data==0 branch).
        assert!(Value::Nil.convert_to(Int, true));
        assert!(Value::Nil.convert_to(Map, true));
        assert!(!Value::Nil.convert_to(Ref, true));
        // FnCnvInt2Id only inside 0..=9999 (C4Value.cpp:469-478).
        assert!(Value::Int(0).convert_to(C4Id, true));
        assert!(Value::Int(9999).convert_to(C4Id, true));
        assert!(!Value::Int(10000).convert_to(C4Id, true));
        assert!(!Value::Int(-1).convert_to(C4Id, true));
        // FnCnvDirectOld: denied under #strict, allowed in old syntax
        // (C4Value.cpp:431-437).
        assert!(!Value::C4Id("CLNK".into()).convert_to(Int, true));
        assert!(Value::C4Id("CLNK".into()).convert_to(Int, false));
        assert!(!Value::String("x".into()).convert_to(Int, true));
        assert!(Value::String("x".into()).convert_to(Int, false));
        assert!(!Value::Object(42).convert_to(Int, true));
        assert!(Value::Object(42).convert_to(Int, false));
        // FnCnvError is absolute regardless of #strict (C4Value.cpp:439-443).
        assert!(!Value::Array(vec![]).convert_to(Int, false));
        assert!(!Value::Int(5).convert_to(String, false));
        // same-type and "to bool" are always OK for non-references.
        assert!(Value::Object(42).convert_to(C4Object, true));
        assert!(Value::Object(42).convert_to(Bool, true));
        assert!(Value::Array(vec![]).convert_to(Array, true));
        assert!(Value::Proplist(ValueMap::new()).convert_to(Bool, true));
    }
}
