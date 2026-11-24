/// A node's id: a random (version 4) UUID, the same in every outline the
/// node reaches (share packages keep it). Shown as 32 lowercase hex digits;
/// `ShortIds` gives the shortest unambiguous prefix for display.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct NodeId(pub u128);

/// The invisible root of every outline (the nil UUID). Its children are the top-level bullets.
pub const ROOT: NodeId = NodeId(0);

/// Bits 76–79 of a UUID hold its version.
const VERSION_SHIFT: u32 = 76;
const VERSION_MASK: u128 = 0xF << VERSION_SHIFT;
/// Bits 62–63 hold the variant (`10` for RFC 9562 UUIDs).
const VARIANT_MASK: u128 = 0b11 << 62;
const VARIANT_RFC: u128 = 0b10 << 62;
/// Version 8 ("custom") marks *virtual* ids: the rows shown inside a mirror.
/// The tree resolves them to the real node they display (see `add_virtuals`).
const VIRTUAL_VERSION: u128 = 8 << VERSION_SHIFT;

impl NodeId {
    /// A new random id.
    pub fn random() -> NodeId {
        NodeId(uuid::Uuid::new_v4().as_u128())
    }

    /// Whether this is a virtual id (a row inside a mirror), not a stored node.
    #[inline]
    pub fn is_virtual(self) -> bool {
        self.0 & VERSION_MASK == VIRTUAL_VERSION
    }

    /// The bytes stored in the database (big-endian, as UUIDs are written).
    pub fn to_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Option<NodeId> {
        Some(NodeId(u128::from_be_bytes(b.try_into().ok()?)))
    }

    /// A short number for UI element ids (not unique; fine for hashing).
    pub fn as_usize(self) -> usize {
        (self.0 as u64 ^ (self.0 >> 64) as u64) as usize
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl std::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

/// Parses the full id: 32 hex digits, with or without the UUID hyphens.
impl std::str::FromStr for NodeId {
    type Err = ();

    fn from_str(s: &str) -> Result<NodeId, ()> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(());
        }
        u128::from_str_radix(&hex, 16).map(NodeId).map_err(|_| ())
    }
}

impl serde::Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<NodeId, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(|_| serde::de::Error::custom("expected a node id (32 hex digits)"))
    }
}

/// A stable virtual id for `child` shown under the row `parent_key`.
pub fn virtual_id(parent_key: NodeId, child: NodeId) -> NodeId {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (parent_key, child).hash(&mut h);
    let lo = h.finish();
    (child, parent_key).hash(&mut h);
    let hi = h.finish();
    let bits = ((hi as u128) << 64) | lo as u128;
    NodeId((bits & !VERSION_MASK & !VARIANT_MASK) | VIRTUAL_VERSION | VARIANT_RFC)
}
