//! Shared `(namespace, name)` keying for [`ConfigCache`] and [`SecretCache`].
//!
//! Lookups in those caches happen on the hot gRPC `Get` path; the previous
//! key type `(String, String)` forced two heap allocations per call to copy
//! `namespace` and `name` into an owned tuple. This module provides:
//!
//! * [`OwnedKey`] — what we actually store in the map.
//! * [`BorrowedKey`] — a zero-allocation `&str`/`&str` view constructed at
//!   the lookup site.
//! * The `dyn KeyRef` trick (`Borrow<dyn KeyRef + '_>` for `OwnedKey`) so the
//!   underlying `HashMap` resolves `BorrowedKey` and `OwnedKey` to the same
//!   bucket without an owned-tuple round trip.
//!
//! The `Hash` impls on `OwnedKey` and `dyn KeyRef` write `namespace` then
//! `name` so the two paths agree byte-for-byte — required for `Borrow`-based
//! HashMap lookup to find the entry.
//!
//! [`ConfigCache`]: crate::cache::ConfigCache
//! [`SecretCache`]: crate::secret_cache::SecretCache

use std::borrow::Borrow;
use std::hash::{Hash, Hasher};

pub trait KeyRef {
    fn ns(&self) -> &str;
    fn name(&self) -> &str;
}

impl Hash for dyn KeyRef + '_ {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.ns().hash(h);
        self.name().hash(h);
    }
}

impl PartialEq for dyn KeyRef + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.ns() == other.ns() && self.name() == other.name()
    }
}

impl Eq for dyn KeyRef + '_ {}

#[derive(Clone, Debug)]
pub struct OwnedKey {
    pub namespace: String,
    pub name: String,
}

impl OwnedKey {
    pub fn new(namespace: String, name: String) -> Self {
        Self { namespace, name }
    }
}

impl PartialEq for OwnedKey {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace && self.name == other.name
    }
}

impl Eq for OwnedKey {}

impl Hash for OwnedKey {
    fn hash<H: Hasher>(&self, h: &mut H) {
        // Must match `dyn KeyRef`'s hash impl above so Borrow-based lookup
        // hits the same bucket.
        self.namespace.hash(h);
        self.name.hash(h);
    }
}

impl KeyRef for OwnedKey {
    fn ns(&self) -> &str {
        &self.namespace
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl<'a> Borrow<dyn KeyRef + 'a> for OwnedKey {
    fn borrow(&self) -> &(dyn KeyRef + 'a) {
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BorrowedKey<'a> {
    pub namespace: &'a str,
    pub name: &'a str,
}

impl<'a> BorrowedKey<'a> {
    pub fn new(namespace: &'a str, name: &'a str) -> Self {
        Self { namespace, name }
    }
}

impl<'a> KeyRef for BorrowedKey<'a> {
    fn ns(&self) -> &str {
        self.namespace
    }
    fn name(&self) -> &str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn owned_and_borrowed_hash_to_same_bucket() {
        let owned = OwnedKey::new("ns".to_string(), "cfg".to_string());
        let borrowed = BorrowedKey::new("ns", "cfg");

        let mut map: HashMap<OwnedKey, &'static str> = HashMap::new();
        map.insert(owned, "value");
        let got = map.get(&borrowed as &dyn KeyRef);
        assert_eq!(got, Some(&"value"));
    }

    #[test]
    fn missing_key_returns_none() {
        let map: HashMap<OwnedKey, ()> = HashMap::new();
        let q = BorrowedKey::new("nope", "nada");
        assert!(!map.contains_key(&q as &dyn KeyRef));
    }

    /// Keys with the same ns+name MUST compare equal, regardless of which
    /// representation (Owned vs Borrowed) is on each side. Required for
    /// HashMap correctness.
    #[test]
    fn equality_holds_across_representations() {
        let owned = OwnedKey::new("ns".to_string(), "cfg".to_string());
        let borrowed = BorrowedKey::new("ns", "cfg");
        let o_ref: &dyn KeyRef = &owned;
        let b_ref: &dyn KeyRef = &borrowed;
        assert!(o_ref == b_ref);
    }
}
