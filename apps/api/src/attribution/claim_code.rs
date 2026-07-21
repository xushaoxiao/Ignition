//! Claim-code and tracking-id generation, normalisation, and format validation.

use rand::Rng;

/// Claim-code alphabet.
///
/// Deliberately excludes `0 O 1 I L` — on iOS users type codes manually, the only billable path.
/// One misread character is lost revenue; smaller code space is acceptable.
/// 31 chars × 8 digits ≈ 8.5e11 combinations; rate limits handle enumeration.
const CLAIM_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CLAIM_CODE_LEN: usize = 8;

/// Tracking-id alphabet.
// TODO(console): remove these allows when placement issuance (KOL console) ships.
#[allow(dead_code)]
const TRACKING_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
#[allow(dead_code)]
const TRACKING_ID_LEN: usize = 10;

/// Generate a claim code.
///
/// Cryptographically secure randomness — codes map to billable conversions; predictable codes forge revenue.
pub fn new_claim_code() -> String {
    random_string(CLAIM_ALPHABET, CLAIM_CODE_LEN)
}

/// Generate a tracking id.
///
/// Must be unguessable: if KOLs can guess each other's links, they can farm or steal attribution.
/// No human-readability optimisations — tracking_id is never typed manually.
#[allow(dead_code)]
pub fn new_tracking_id() -> String {
    random_string(TRACKING_ALPHABET, TRACKING_ID_LEN)
}

/// Normalise user input to storage form: case-insensitive, strip separators and whitespace users or
/// IMEs may insert.
///
/// Manual entry is mandatory on iOS; tolerance directly affects redemption completion (core W7 metric).
///
/// **No confusable-character mapping**: the alphabet already excludes `0 O 1 I L`, so these in input
/// are mistakes and `0` could mean `O` or `D` — unreliable. Guessing wrong maps one valid code to
/// another under the wrong KOL — better format error and retry.
pub fn normalize_claim_code(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_' | '\t' | '\n' | '\r' | '.'))
        .flat_map(char::to_uppercase)
        .collect()
}

/// Whether a claim code matches the expected format.
///
/// Validate before hitting the database — rejects most enumeration and typos without DB cost and gives
/// a more useful error than "code not found".
pub fn is_valid_claim_code(code: &str) -> bool {
    code.len() == CLAIM_CODE_LEN && code.bytes().all(|b| CLAIM_ALPHABET.contains(&b))
}

fn random_string(alphabet: &[u8], len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| alphabet[rng.gen_range(0..alphabet.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Claim alphabet must exclude confusable characters — manual iOS entry is the only billable path.
    #[test]
    fn claim_code_excludes_confusable_chars() {
        for _ in 0..500 {
            let code = new_claim_code();
            assert_eq!(code.len(), 8);
            for bad in ['0', 'O', '1', 'I', 'L'] {
                assert!(!code.contains(bad), "code {code} contains confusable {bad}");
            }
        }
    }

    #[test]
    fn claim_codes_do_not_repeat() {
        let codes: HashSet<String> = (0..200).map(|_| new_claim_code()).collect();
        assert_eq!(codes.len(), 200, "duplicate claim codes");
    }

    /// tracking_id must be unguessable — predictable links enable farming or attribution theft.
    #[test]
    fn tracking_ids_are_unpredictable() {
        let ids: HashSet<String> = (0..300).map(|_| new_tracking_id()).collect();
        assert_eq!(ids.len(), 300, "duplicate tracking_id");
        assert!(ids.iter().all(|id| id.len() == 10));
    }

    #[test]
    fn normalize_handles_common_input_noise() {
        for (input, want) in [
            ("ab3xy9zk", "AB3XY9ZK"),
            ("AB3X-Y9ZK", "AB3XY9ZK"),
            ("ab3x y9zk", "AB3XY9ZK"),
            (" AB3XY9ZK ", "AB3XY9ZK"),
            ("ab3x_y9zk", "AB3XY9ZK"),
        ] {
            assert_eq!(normalize_claim_code(input), want, "input {input:?}");
        }
    }

    /// No confusable mapping: guessing `0` as `O` or `D` can turn one valid code into another under the wrong KOL.
    #[test]
    fn normalize_does_not_guess_confusables() {
        let got = normalize_claim_code("AB0XY1ZK");

        assert_eq!(got, "AB0XY1ZK", "must not substitute 0 and 1");
        assert!(!is_valid_claim_code(&got), "should fail format validation");
    }

    #[test]
    fn format_validation() {
        assert!(is_valid_claim_code(&new_claim_code()));

        assert!(!is_valid_claim_code(""), "empty code");
        assert!(!is_valid_claim_code("AB3XY9Z"), "too short");
        assert!(!is_valid_claim_code("AB3XY9ZKQ"), "too long");
        assert!(!is_valid_claim_code("ab3xy9zk"), "lowercase not normalised");
        assert!(!is_valid_claim_code("00000000"), "out-of-alphabet chars");
    }
}
