//! Call-id encoding for the parallel handler.
//!
//! The single WebSocket connection delivers all responses through one stream.
//! To dispatch each response to the right `SessionTask` without a per-call
//! lookup table, we embed the slot index in the high bits of the CDP `id`:
//!
//! ```text
//!   63              48 47                                                0
//!   ┌─────────────────┬──────────────────────────────────────────────────┐
//!   │   slot (16b)    │                  seq (48b)                       │
//!   └─────────────────┴──────────────────────────────────────────────────┘
//! ```
//!
//! Slot 0 is reserved for the Router (browser-level commands). Slots 1..=N
//! are SessionTasks. Decoding the slot is a single shift; routing has no
//! shared map and no contention.

use chromiumoxide_types::CallId;

/// Maximum number of session slots (slot 0 = Router).
#[allow(dead_code)]
pub const MAX_SLOTS: u16 = u16::MAX;

const SLOT_SHIFT: u32 = 48;
const SEQ_MASK: u64 = (1u64 << 48) - 1;

/// Compose a CallId from a slot index and a per-slot sequence counter.
#[inline]
pub fn encode(slot: u16, seq: u64) -> CallId {
    let composed = ((slot as u64) << SLOT_SHIFT) | (seq & SEQ_MASK);
    CallId::new(composed as usize)
}

/// Recover the slot index from a CallId.
#[inline]
pub fn decode_slot(id: CallId) -> u16 {
    ((id.as_usize() as u64) >> SLOT_SHIFT) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for &slot in &[0u16, 1, 7, 255, 4096, MAX_SLOTS - 1, MAX_SLOTS] {
            for &seq in &[0u64, 1, 1024, (1u64 << 32) + 7, SEQ_MASK] {
                let id = encode(slot, seq);
                assert_eq!(decode_slot(id), slot, "slot for {slot}/{seq}");
            }
        }
    }

    #[test]
    fn router_slot_is_zero() {
        let id = encode(0, 42);
        assert_eq!(decode_slot(id), 0);
        assert_eq!(id.as_usize() as u64, 42);
    }
}
