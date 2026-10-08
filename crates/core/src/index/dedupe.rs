use std::collections::HashSet;

// 2^32 records = 512 MB of bits; anything past that is not a real MFT and goes to the set.
const MAX_BITSET_RECORD: u64 = 1 << 32;
const RECORD_MASK: u64 = 0xFFFF_FFFF_FFFF;

pub(crate) struct SeenSet {
    ntfs: bool,
    bits: Vec<u64>,
    other: HashSet<u128>,
}

impl SeenSet {
    pub fn new(ntfs: bool) -> SeenSet {
        SeenSet {
            ntfs,
            bits: Vec::new(),
            other: HashSet::new(),
        }
    }

    // All-zero ids mean the lister could not read one, so they are never treated as repeats.
    pub fn first_sighting(&mut self, id: &[u8; 16]) -> bool {
        if id.iter().all(|&b| b == 0) {
            return true;
        }
        if self.ntfs && id[8..].iter().all(|&b| b == 0) {
            let mut low = [0u8; 8];
            low.copy_from_slice(&id[..8]);
            let record = u64::from_le_bytes(low) & RECORD_MASK;
            if record < MAX_BITSET_RECORD {
                let word = (record / 64) as usize;
                if word >= self.bits.len() {
                    self.bits.resize((word + 1).next_power_of_two(), 0);
                }
                let mask = 1u64 << (record % 64);
                let seen = self.bits[word] & mask != 0;
                self.bits[word] |= mask;
                return !seen;
            }
        }
        self.other.insert(u128::from_le_bytes(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ntfs_id(record: u64, seq: u16) -> [u8; 16] {
        let frn = record | (u64::from(seq) << 48);
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&frn.to_le_bytes());
        id
    }

    #[test]
    fn bitset_detects_repeats_by_record_number() {
        let mut s = SeenSet::new(true);
        assert!(s.first_sighting(&ntfs_id(5, 1)));
        assert!(s.first_sighting(&ntfs_id(7_000_000, 3)));
        assert!(!s.first_sighting(&ntfs_id(5, 1)));
        assert!(!s.first_sighting(&ntfs_id(7_000_000, 3)));
        assert!(s.bits.len() * 8 <= 2 << 20);
    }

    #[test]
    fn zero_ids_never_repeat_and_wide_ids_use_the_set() {
        let mut s = SeenSet::new(true);
        assert!(s.first_sighting(&[0; 16]));
        assert!(s.first_sighting(&[0; 16]));
        let wide = [9u8; 16];
        assert!(s.first_sighting(&wide));
        assert!(!s.first_sighting(&wide));
        let mut r = SeenSet::new(false);
        assert!(r.first_sighting(&ntfs_id(5, 1)));
        assert!(!r.first_sighting(&ntfs_id(5, 1)));
        assert!(r.bits.is_empty());
    }
}
