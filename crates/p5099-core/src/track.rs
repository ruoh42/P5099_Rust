pub const P5099_FALLBACK_TRACK_NAME: &str = "village_R01";

pub const P5099_FALLBACK_TRACK_ID: u32 = 0x34ca_03f6;

pub const P5099_RANDOM_TRACK_SELECTORS: [u32; 12] = [0, 1, 3, 4, 5, 6, 7, 8, 23, 30, 33, 40];

#[must_use]
pub fn is_random_track_selector(track: u32) -> bool {
    P5099_RANDOM_TRACK_SELECTORS.contains(&track)
}

#[cfg(test)]
mod tests {
    use crate::adler32;

    use super::{P5099_FALLBACK_TRACK_ID, P5099_FALLBACK_TRACK_NAME, is_random_track_selector};

    #[test]
    fn fallback_identifier_matches_the_csharp_unicode_hash() {
        assert_eq!(
            adler32::unicode_hash(P5099_FALLBACK_TRACK_NAME),
            P5099_FALLBACK_TRACK_ID
        );
    }

    #[test]
    fn distinguishes_pool_selectors_from_concrete_track_hashes() {
        for selector in [0, 1, 3, 4, 5, 6, 7, 8, 23, 30, 33, 40] {
            assert!(is_random_track_selector(selector));
        }
        assert!(!is_random_track_selector(P5099_FALLBACK_TRACK_ID));
    }
}
