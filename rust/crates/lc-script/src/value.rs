use std::collections::HashMap;
use std::fmt;

const C4V_ANY: usize = 0;
const C4V_INT: usize = 1;
const C4V_BOOL: usize = 2;
const C4V_C4ID: usize = 3;
const C4V_C4OBJECT: usize = 4;
const C4V_STRING: usize = 5;
const C4V_ARRAY: usize = 6;
const C4V_MAP: usize = 7;

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

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Int(i32),
    Bool(bool),
    String(String),
    C4Id(String),
    Object(u64),
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
            Value::String(value) => {
                c4_hash_typed(C4V_STRING, cpp_string_view_hash(value.as_bytes()))
            }
            Value::C4Id(value) => c4_hash_typed(C4V_C4ID, hash_i32(c4_id_raw(value) as i32)),
            Value::Object(id) => c4_hash_typed(C4V_C4OBJECT, *id as usize),
            Value::Array(values) => values.iter().fold(C4V_ARRAY, |hash, value| {
                c4_hash_combine(hash, value.c4_value_hash())
            }),
            Value::Proplist(entries) => {
                let content_hash = entries.iter().fold(0, |content_hash, (key, value)| {
                    let item_hash = c4_hash_combine(
                        c4_hash_typed(C4V_STRING, cpp_string_view_hash(key.as_bytes())),
                        value.c4_value_hash(),
                    );
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

fn hash_i32(value: i32) -> usize {
    value as usize
}

fn c4_id_raw(id: &str) -> usize {
    if id.len() < 4 || id == "NONE" {
        return 0;
    }

    let mut raw = 0usize;
    let mut numeric = true;
    for byte in id.bytes() {
        if !byte.is_ascii_digit() {
            numeric = false;
            break;
        }
        raw = raw.wrapping_mul(10).wrapping_add((byte - b'0') as usize);
    }
    if numeric {
        return raw;
    }

    id.as_bytes()
        .iter()
        .take(4)
        .rev()
        .fold(0usize, |raw, byte| (raw << 8) | (*byte as usize))
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
        assert_eq!(Value::Proplist(HashMap::new()).c4v_type(), C4VType::Map);
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
        assert!(Value::Proplist(HashMap::new()).convert_to(Bool, true));
    }
}
