//! An object's locals in the property panel's emission order.
//!
//! `C4PropertyDlg::Update` emits the indexed array first, then the named list
//! (`C4PropertyDlg.cpp:210-234`):
//!
//! ```cpp
//! for (cnt = 0; cnt < cobj->Local.GetSize(); cnt++)
//!     if (cobj->Local[cnt]) { ... " Local({}) = " ... }
//! for (cnt = 0; cnt < cobj->LocalNamed.GetAnzItems(); cnt++)
//!     { ... " {name} = " ... }
//! ```
//!
//! Two asymmetries the port must keep:
//!
//! - The indexed loop **skips falsy slots** (`C4Value::operator bool`, so an
//!   unset slot, integer zero and `false` all vanish); the named loop emits
//!   **every declared name**, assigned or not.
//! - `LocalNamed.GetAnzItems()` is the size of the *definition's* name list.
//!   `C4ValueMapData::SetNameList`/`OnNameListChanged` (`C4ValueMap.cpp`)
//!   re-map a loaded object's values onto that list by name after compiling
//!   `LocalNamed=`, so a saved name the definition no longer declares is
//!   dropped, and the emission order is the definition's `local` declaration
//!   order — never the file's.
//!
//! The port stores both kinds in one `ObjectState::local_vars` map: C++'s
//! numbered `Local[n]` slots are keyed `__local_{n}` (the same key the VM's
//! `Local(n)`/`SetLocal` path uses, `clonk-script/src/vm.rs:3354-3372`), and
//! named locals keep their own names. Declaration order comes from the
//! definition script's ordered `local` declarations
//! (`clonk_script::engine::Script::var_decls`), which is the port's
//! `C4ValueMapNames`.

use clonk_script::Value;
use std::collections::HashMap;

/// The synthetic key the port gives C++'s `Local[n]` slots.
pub const INDEXED_LOCAL_PREFIX: &str = "__local_";

/// One local as the property panel emits it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalEntry {
    /// `Local[n]`, rendered ` Local({index}) = {value}`. `key` is the
    /// `local_vars` key holding it.
    Indexed { index: u32, key: String },
    /// A declared named local, rendered ` {name} = {value}`. Absent from
    /// `local_vars` means it is still nil, which C++ prints as `nil`.
    Named { name: String },
}

/// The index encoded in an `__local_{n}` key, if the key is one.
pub fn indexed_local_index(key: &str) -> Option<u32> {
    key.strip_prefix(INDEXED_LOCAL_PREFIX)
        .and_then(|index| index.parse().ok())
}

/// The property panel's local entries for one object (`C4PropertyDlg.cpp:210-234`).
///
/// `declared_names` is the definition's `local` declarations in declaration
/// order — C++'s `LocalNamed.pNames`. Named locals are emitted from that list
/// alone, so an entry the definition does not declare never appears however it
/// reached `local_vars`.
pub fn locals_in_emission_order(
    local_vars: &HashMap<String, Value>,
    declared_names: &[String],
) -> Vec<LocalEntry> {
    let mut indexed: Vec<(u32, &String)> = local_vars
        .iter()
        // `if (cobj->Local[cnt])` — falsy slots are not emitted at all.
        .filter(|(_, value)| value.as_bool())
        .filter_map(|(key, _)| indexed_local_index(key).map(|index| (index, key)))
        .collect();
    indexed.sort_by_key(|(index, _)| *index);
    indexed
        .into_iter()
        .map(|(index, key)| LocalEntry::Indexed {
            index,
            key: key.clone(),
        })
        .chain(
            declared_names
                .iter()
                .map(|name| LocalEntry::Named { name: name.clone() }),
        )
        .collect()
}

/// The property panel's local lines, ready for
/// [`crate::developer_property_text::PropertyPanelObject::locals`].
///
/// C++ writes `" Local({n}) = {value}"` for a slot and `" {name} = {value}"`
/// for a named local (the leading space comes from `LineFeed " "`), both with
/// `C4Value::GetDataString`. An undeclared-yet-unassigned name prints `nil`,
/// which is what `GetDataString` returns for `C4V_Any`.
pub fn local_lines(local_vars: &HashMap<String, Value>, declared_names: &[String]) -> Vec<String> {
    locals_in_emission_order(local_vars, declared_names)
        .into_iter()
        .map(|entry| match entry {
            LocalEntry::Indexed { index, key } => {
                format!(" Local({index}) = {}", display(local_vars.get(&key)))
            }
            LocalEntry::Named { name } => {
                let value = display(local_vars.get(&name));
                format!(" {name} = {value}")
            }
        })
        .collect()
}

fn display(value: Option<&Value>) -> String {
    clonk_script::data_string(value.unwrap_or(&Value::Nil))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locals(entries: [(&str, Value); 5]) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
    }

    fn declared(names: [&str; 2]) -> Vec<String> {
        names.into_iter().map(str::to_owned).collect()
    }

    // C4PropertyDlg.cpp:210-234 — the indexed array ascending and truthy-only,
    // then every declared name in declaration order.
    #[test]
    fn locals_split_into_indexed_then_named_like_the_property_panel() {
        assert_eq!(indexed_local_index("__local_0"), Some(0));
        assert_eq!(indexed_local_index("__local_12"), Some(12));
        // Anything that is not the synthetic form is a named local, including
        // near-misses that would otherwise be mis-sorted into the array.
        assert_eq!(indexed_local_index("__local_"), None);
        assert_eq!(indexed_local_index("__local_x"), None);
        assert_eq!(indexed_local_index("__local_-1"), None);
        assert_eq!(indexed_local_index("speed"), None);

        // Indexed entries sort numerically, not lexically — `__local_10` must
        // follow `__local_9`.
        let vars = locals([
            ("__local_10", Value::Int(4)),
            ("__local_2", Value::Int(3)),
            ("__local_9", Value::Int(2)),
            ("__local_0", Value::Int(1)),
            ("speed", Value::Int(7)),
        ]);
        // Declaration order, deliberately not alphabetical.
        let names = declared(["speed", "aim"]);
        assert_eq!(
            locals_in_emission_order(&vars, &names),
            vec![
                LocalEntry::Indexed {
                    index: 0,
                    key: "__local_0".to_owned()
                },
                LocalEntry::Indexed {
                    index: 2,
                    key: "__local_2".to_owned()
                },
                LocalEntry::Indexed {
                    index: 9,
                    key: "__local_9".to_owned()
                },
                LocalEntry::Indexed {
                    index: 10,
                    key: "__local_10".to_owned()
                },
                // `aim` is never assigned, but C++ emits the whole name list.
                LocalEntry::Named {
                    name: "speed".to_owned()
                },
                LocalEntry::Named {
                    name: "aim".to_owned()
                },
            ],
            "a lexical sort would place __local_10 before __local_2"
        );

        // `if (cobj->Local[cnt])` skips falsy slots — zero, false and nil all
        // vanish, while an empty string stays because C++ tests the pointer.
        let sparse = locals([
            ("__local_0", Value::Int(0)),
            ("__local_1", Value::Bool(false)),
            ("__local_2", Value::Nil),
            ("__local_3", Value::String(String::new().into())),
            ("__local_4", Value::Int(-1)),
        ]);
        assert_eq!(
            locals_in_emission_order(&sparse, &[]),
            vec![
                LocalEntry::Indexed {
                    index: 3,
                    key: "__local_3".to_owned()
                },
                LocalEntry::Indexed {
                    index: 4,
                    key: "__local_4".to_owned()
                },
            ]
        );

        // A named entry the definition does not declare is dropped, exactly as
        // OnNameListChanged drops it when the loaded list is re-mapped.
        let undeclared = locals([
            ("stale", Value::Int(1)),
            ("aim", Value::Int(2)),
            ("__local_0", Value::Int(3)),
            ("__local_1", Value::Int(0)),
            ("__local_2", Value::Nil),
        ]);
        assert_eq!(
            locals_in_emission_order(&undeclared, &declared(["aim", "power"])),
            vec![
                LocalEntry::Indexed {
                    index: 0,
                    key: "__local_0".to_owned()
                },
                LocalEntry::Named {
                    name: "aim".to_owned()
                },
                LocalEntry::Named {
                    name: "power".to_owned()
                },
            ],
            "only the definition's declared names are emitted"
        );

        // An object with nothing at all emits nothing.
        assert!(locals_in_emission_order(&HashMap::new(), &[]).is_empty());

        // The rendered lines carry C4Value::GetDataString values, and a
        // declared-but-unassigned name prints nil.
        assert_eq!(
            local_lines(&undeclared, &declared(["aim", "power"])),
            vec![
                " Local(0) = 3".to_owned(),
                " aim = 2".to_owned(),
                " power = nil".to_owned(),
            ]
        );
    }
}
