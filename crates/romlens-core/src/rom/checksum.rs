//! The header checksum: the 16-bit wrapping sum of every byte the CPU can
//! see, which for a non-power-of-two image means the remainder is repeated to
//! fill the space above the largest power of two (3 MB = 2 MB + 2 × 1 MB).
//! This is the same rule as [`crate::memory::map::mirror_offset`].

fn plain_sum(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .fold(0u16, |acc, &b| acc.wrapping_add(b as u16))
}

fn largest_power_of_two_at_most(len: usize) -> usize {
    1usize << (usize::BITS - 1 - len.leading_zeros())
}

/// Sum of `part` as seen when it is mirrored to fill `span` bytes.
fn mirrored_sum(part: &[u8], span: usize) -> u16 {
    if part.len().is_power_of_two() {
        let reps = (span / part.len()) as u16;
        return plain_sum(part).wrapping_mul(reps);
    }
    let big = largest_power_of_two_at_most(part.len());
    let period = big * 2;
    let reps = (span / period) as u16;
    let per_period = plain_sum(&part[..big]).wrapping_add(mirrored_sum(&part[big..], big));
    per_period.wrapping_mul(reps)
}

/// Checksum of a payload with the mirrored-sum rule. An empty payload is 0.
pub fn compute_checksum(payload: &[u8]) -> u16 {
    if payload.is_empty() {
        return 0;
    }
    if payload.len().is_power_of_two() {
        return plain_sum(payload);
    }
    let big = largest_power_of_two_at_most(payload.len());
    plain_sum(&payload[..big]).wrapping_add(mirrored_sum(&payload[big..], big))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_of_two_is_a_plain_sum() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(1 << 16).collect();
        assert_eq!(compute_checksum(&bytes), plain_sum(&bytes));
    }

    #[test]
    fn three_mb_counts_the_last_megabyte_twice() {
        let mb = 1 << 20;
        let mut bytes = vec![1u8; 2 * mb];
        bytes.extend(std::iter::repeat_n(3u8, mb));
        let expected = (2u32 * mb as u32 + 2 * mb as u32 * 3) as u16;
        assert_eq!(compute_checksum(&bytes), expected);
    }
}
