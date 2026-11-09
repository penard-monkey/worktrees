//! ULIDs: 26 characters of Crockford base32, 48 bits of milliseconds and 80
//! bits of randomness (<https://github.com/ulid/spec>). An idea's `id` is one
//! (`ideas`), written once and never changed.
//!
//! A small function of our own rather than a crate (`deny.toml` keeps the
//! dependency list short). It is MONOTONIC inside one process: a second id
//! in the same millisecond is the previous one's random part plus one, so two
//! ids made back to back can never be equal and always sort in the order
//! they were made, which is the spec's own monotonic rule.

use std::sync::Mutex;

/// Crockford's alphabet: no I, L, O or U.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The last id this process made, as (ms, random), for the monotonic rule.
static LAST: Mutex<(u64, u128)> = Mutex::new((0, 0));

const RANDOM_BITS: u32 = 80;
const RANDOM_MAX: u128 = (1u128 << RANDOM_BITS) - 1;

/// A new ULID for now.
pub fn new() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let fresh = ((crate::fsx::random_u64() as u128) << 64 | crate::fsx::random_u64() as u128) & RANDOM_MAX;
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
    let (ms, rand) = next(*last, ms, fresh);
    *last = (ms, rand);
    encode(ms, rand)
}

/// The monotonic step, on its inputs: a clock at or behind the last id's
/// millisecond continues that millisecond's sequence (a clock stepped back
/// must not mint an id that sorts before one already handed out); a random
/// part that would overflow moves on to the next millisecond.
fn next(last: (u64, u128), now_ms: u64, fresh: u128) -> (u64, u128) {
    let (lms, lrand) = last;
    if now_ms > lms {
        return (now_ms, fresh);
    }
    if lrand < RANDOM_MAX {
        (lms, lrand + 1)
    } else {
        (lms + 1, fresh)
    }
}

fn encode(ms: u64, rand: u128) -> String {
    let v: u128 = ((ms as u128 & ((1 << 48) - 1)) << RANDOM_BITS) | (rand & RANDOM_MAX);
    (0..26).rev().map(|i| ALPHABET[((v >> (i * 5)) & 31) as usize] as char).collect()
}

/// `s` is a ULID as this module writes them: 26 characters of the upper-case
/// alphabet, the first at most `7` (128 bits). The check every id read from a
/// FILE passes before it names anything, a `.history/<id>/` directory
/// included: a person can type anything into a frontmatter.
pub fn is_valid(s: &str) -> bool {
    s.len() == 26 && s.bytes().all(|b| ALPHABET.contains(&b)) && s.as_bytes()[0] <= b'7'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_26_crockford_characters_with_the_time_first() {
        let id = new();
        assert!(is_valid(&id), "{id}");
        // The spec's own example time: 1469918176385 ms is `01ARYZ6S41`.
        assert_eq!(&encode(1_469_918_176_385, 0)[..10], "01ARYZ6S41");
        assert_eq!(encode(0, 0), "00000000000000000000000000");
        assert_eq!(encode((1 << 48) - 1, RANDOM_MAX), "7ZZZZZZZZZZZZZZZZZZZZZZZZZ");
    }

    #[test]
    fn the_id_generator_never_repeats_within_a_millisecond() {
        // The rule on its inputs: the same millisecond, the same random draw.
        let a = next((0, 0), 5, 42);
        let b = next(a, 5, 42);
        let c = next(b, 4, 42); // a clock stepped back
        assert_eq!(a, (5, 42));
        assert_eq!(b, (5, 43), "same ms: the last random part plus one");
        assert_eq!(c, (5, 44), "a clock behind the last id continues its sequence");
        assert_eq!(next((5, RANDOM_MAX), 5, 7), (6, 7), "an overflow moves to the next ms");
        // And the real thing, many in a row, which land in few milliseconds.
        let ids: Vec<String> = (0..10_000).map(|_| new()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "no id repeats");
        assert_eq!(sorted, ids, "and they sort in the order they were made");
    }

    #[test]
    fn only_a_well_formed_id_is_valid() {
        assert!(is_valid("01JC8Z4K3M9QX2T7VBN5R6WYHD"));
        for bad in ["", "01JC8Z4K3M9QX2T7VBN5R6WYH", "01jc8z4k3m9qx2t7vbn5r6wyhd", "01JC8Z4K3M9QX2T7VBN5R6WYHI", "../../../../etc/passwd....", "81JC8Z4K3M9QX2T7VBN5R6WYHD"] {
            assert!(!is_valid(bad), "{bad}");
        }
    }
}
