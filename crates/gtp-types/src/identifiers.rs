use core::fmt;

/// 64-bit opaque Connection Identifier for routing and session continuity across NAT rebinding.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct ConnectionId(pub u64);

impl ConnectionId {
    pub const fn from_u64(val: u64) -> Self {
        Self(val)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub fn to_be_bytes(self) -> [u8; 8] {
        self.0.to_be_bytes()
    }

    pub fn from_be_bytes(bytes: [u8; 8]) -> Self {
        Self(u64::from_be_bytes(bytes))
    }
}

impl fmt::Debug for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CID({:016x})", self.0)
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Monotonically increasing transport-level packet number.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct PacketNumber(pub u64);

impl PacketNumber {
    pub const fn from_u64(val: u64) -> Self {
        Self(val)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        // Wrapping, not panicking: debug builds must not crash at the u64 boundary
        // (WIR-9). Packet-number exhaustion handling is a documented separate concern.
        Self(self.0.wrapping_add(1))
    }
}

impl fmt::Debug for PacketNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pkt#{}", self.0)
    }
}

impl fmt::Display for PacketNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Logical game message identifier.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct MessageId(pub u64);

impl MessageId {
    pub const fn from_u64(val: u64) -> Self {
        Self(val)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl fmt::Debug for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Msg#{}", self.0)
    }
}

/// Fragment index within a fragmented message.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct FragmentId(pub u16);

impl FragmentId {
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl fmt::Debug for FragmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Frag#{}", self.0)
    }
}

/// Transmission attempt identifier for tracking retries.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct TransmissionId(pub u8);

impl TransmissionId {
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Debug for TransmissionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Xmit#{}", self.0)
    }
}

/// Modulo-safe state sequence number for freshness comparison.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Default)]
pub struct StateSequence(pub u32);

impl StateSequence {
    pub const fn from_u32(val: u32) -> Self {
        Self(val)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Determines if `self` is strictly newer than `other` using serial number arithmetic (RFC 1982).
    pub fn is_newer_than(self, other: StateSequence) -> bool {
        let diff = self.0.wrapping_sub(other.0);
        diff > 0 && diff < (1 << 31)
    }

    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl fmt::Debug for StateSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Seq#{}", self.0)
    }
}

/// World snapshot or state generation identifier for bulk supersession.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct GenerationId(pub u32);

impl GenerationId {
    pub const fn from_u32(val: u32) -> Self {
        Self(val)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Determines if `self` is strictly newer than `other` using serial number
    /// arithmetic (RFC 1982) — SEM-1: generations wrap like any u32 counter, so a
    /// plain `>` comparison would permanently invert supersession after 2^32.
    pub fn is_newer_than(self, other: GenerationId) -> bool {
        let diff = self.0.wrapping_sub(other.0);
        diff > 0 && diff < (1 << 31)
    }
}

impl fmt::Debug for GenerationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Gen#{}", self.0)
    }
}

/// Identifies an entity and component state channel: `(entity_id, state_type)`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Default)]
pub struct StateKey {
    pub entity_id: u32,
    pub state_type: u16,
}

impl StateKey {
    pub const fn new(entity_id: u32, state_type: u16) -> Self {
        Self {
            entity_id,
            state_type,
        }
    }

    pub fn to_u48(self) -> u64 {
        ((self.entity_id as u64) << 16) | (self.state_type as u64)
    }

    pub fn from_u48(val: u64) -> Self {
        Self {
            entity_id: (val >> 16) as u32,
            state_type: (val & 0xFFFF) as u16,
        }
    }
}

impl fmt::Debug for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "StateKey(entity: {}, type: {})",
            self.entity_id, self.state_type
        )
    }
}

/// Independent ordered delivery stream/group identifier.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct OrderedGroupId(pub u16);

impl OrderedGroupId {
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl fmt::Debug for OrderedGroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Group#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_sequence_modulo_arithmetic() {
        let s1 = StateSequence(100);
        let s2 = StateSequence(105);
        assert!(s2.is_newer_than(s1));
        assert!(!s1.is_newer_than(s2));

        // Wrap-around boundary
        let s_max = StateSequence(u32::MAX - 5);
        let s_wrap = StateSequence(5);
        assert!(s_wrap.is_newer_than(s_max));
        assert!(!s_max.is_newer_than(s_wrap));
    }

    /// SEM-1: GenerationId must share the RFC 1982 discipline of StateSequence.
    #[test]
    fn test_generation_id_modulo_arithmetic() {
        let g_max = GenerationId(u32::MAX - 5);
        let g_wrap = GenerationId(5);
        assert!(g_wrap.is_newer_than(g_max));
        assert!(!g_max.is_newer_than(g_wrap));
        assert!(!GenerationId(7).is_newer_than(GenerationId(7)));
        assert!(GenerationId(8).is_newer_than(GenerationId(7)));
    }

    /// WIR-9: next() must wrap instead of panicking in debug builds.
    #[test]
    fn test_packet_number_next_wraps_safely() {
        let pn = PacketNumber(u64::MAX);
        let _ = pn.next(); // must not panic
        assert_eq!(MessageId(u64::MAX).next(), MessageId(0));
    }

    #[test]
    fn test_state_key_encoding() {
        let key = StateKey::new(0x12345678, 0x9ABC);
        let encoded = key.to_u48();
        let decoded = StateKey::from_u48(encoded);
        assert_eq!(key, decoded);
    }
}
