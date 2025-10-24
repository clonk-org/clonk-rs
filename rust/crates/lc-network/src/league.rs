use std::collections::HashMap;

/// Tracks league feedback identifiers (FBIDs) for authenticated accounts.
///
/// The classic runtime keeps a linked list that is synchronised with the list of
/// players reported to the league backend. Consumers look up FBIDs by account
/// name when constructing disconnect reports or when restoring cached
/// authentication state.  This registry mirrors the semantics of
/// `C4LeagueFBIDList`: inserting a new FBID replaces any previous mapping for
/// the account and removal is a no-op if the entry does not exist.
#[derive(Debug, Clone, Default)]
pub struct LeagueFbidRegistry {
    entries: HashMap<String, String>,
}

impl LeagueFbidRegistry {
    /// Returns an empty registry.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Clears every stored FBID.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Associates `account` with `fbid`, replacing any previous value.
    pub fn insert(&mut self, account: impl Into<String>, fbid: impl Into<String>) {
        self.entries.insert(account.into(), fbid.into());
    }

    /// Removes the FBID associated with `account`.
    ///
    /// Returns `true` if an entry was present.
    pub fn remove(&mut self, account: &str) -> bool {
        self.entries.remove(account).is_some()
    }

    /// Looks up the FBID registered for `account`.
    pub fn get(&self, account: &str) -> Option<&str> {
        self.entries.get(account).map(|value| value.as_str())
    }

    /// Returns the number of tracked accounts.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reports whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::LeagueFbidRegistry;

    #[test]
    fn insert_and_lookup_round_trip() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Bob", "FBID-456");

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get("Alice"), Some("FBID-123"));
        assert_eq!(registry.get("Bob"), Some("FBID-456"));
        assert!(registry.get("Eve").is_none());
    }

    #[test]
    fn replacing_existing_account_overwrites_value() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Alice", "FBID-999");

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("Alice"), Some("FBID-999"));
    }

    #[test]
    fn removing_unknown_account_is_a_noop() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        assert!(!registry.remove("Bob"));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("Alice"), Some("FBID-123"));
    }

    #[test]
    fn removal_drops_entry() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Bob", "FBID-456");

        assert!(registry.remove("Alice"));
        assert_eq!(registry.get("Alice"), None);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut registry = LeagueFbidRegistry::new();
        registry.insert("Alice", "FBID-123");
        registry.insert("Bob", "FBID-456");
        registry.clear();

        assert!(registry.is_empty());
        assert_eq!(registry.get("Alice"), None);
        assert_eq!(registry.get("Bob"), None);
    }
}
