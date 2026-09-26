#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostcodeValue {
    District(String),
    Outward(String),
    Full(String),
}

/// Classifies a string as a postcode district, a sub-district / outward code, a whole postcode, or neither.
///
/// An outward code ending in a digit becomes `District`.
/// An outward code ending in a letter becomes `Outward`.
/// A whole postcode becomes `Full`, written in the stored form: `LA105DL` and `la10 5dl` both give `Full("LA10 5DL")`.
/// Sector queries like "LA1 5" or "N1 1" classify as `None`.
pub fn classify(value: &str) -> Option<PostcodeValue> {
    let trimmed = value.trim();

    // Check if input has inner whitespace
    let has_whitespace = trimmed.chars().any(|c| c.is_whitespace());

    let squashed: String = trimmed
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();

    let bytes = squashed.as_bytes();
    let n = bytes.len();

    // Must have at least 2 chars (e.g. N1, M1) and at most 8 chars
    if !(2..=8).contains(&n) {
        return None;
    }

    // Check prefix: 1 or 2 uppercase letters
    let alpha_prefix_len = if bytes[0].is_ascii_uppercase() {
        if n > 1 && bytes[1].is_ascii_uppercase() {
            2
        } else {
            1
        }
    } else {
        return None;
    };

    // Followed by at least 1 digit
    if n <= alpha_prefix_len || !bytes[alpha_prefix_len].is_ascii_digit() {
        return None;
    }

    // Outward code check:
    // Outward codes cannot contain inner spaces.
    if !has_whitespace {
        let is_valid_outward = if n == alpha_prefix_len + 1 {
            // e.g. W1, N1, E1
            true
        } else if n == alpha_prefix_len + 2 && bytes[alpha_prefix_len + 1].is_ascii_alphanumeric() {
            // e.g. SW1, NW1, W1A, LA1
            true
        } else if n == alpha_prefix_len + 3
            && bytes[alpha_prefix_len + 1].is_ascii_digit()
            && bytes[alpha_prefix_len + 2].is_ascii_alphabetic()
        {
            // e.g. SW1A, EC1A
            true
        } else {
            false
        };

        if is_valid_outward {
            if bytes[n - 1].is_ascii_digit() {
                return Some(PostcodeValue::District(squashed));
            } else if bytes[n - 1].is_ascii_alphabetic() {
                return Some(PostcodeValue::Outward(squashed));
            }
        }
    }

    // Full postcode check:
    // Ends with [0-9][A-Z]{2} (inward code, length 3)
    // Outward part is everything before the last 3 chars.
    if n >= alpha_prefix_len + 4 {
        let inward_start = n - 3;
        let inward_bytes = &bytes[inward_start..];
        if inward_bytes[0].is_ascii_digit()
            && inward_bytes[1].is_ascii_uppercase()
            && inward_bytes[2].is_ascii_uppercase()
        {
            let outward = &squashed[..inward_start];
            let inward = &squashed[inward_start..];

            let out_len = outward.len();
            let out_bytes = outward.as_bytes();
            let is_valid_out = out_len == alpha_prefix_len + 1
                || (out_len == alpha_prefix_len + 2 && out_bytes[alpha_prefix_len + 1].is_ascii_alphanumeric())
                || (out_len == alpha_prefix_len + 3
                    && out_bytes[alpha_prefix_len + 1].is_ascii_digit()
                    && out_bytes[alpha_prefix_len + 2].is_ascii_alphabetic());

            if is_valid_out {
                return Some(PostcodeValue::Full(format!("{outward} {inward}")));
            }
        }
    }

    None
}

/// Drops the last character when it's a letter that follows a digit:
/// `W1A` → `W1`, `EC1A` → `EC1`, `E1W` → `E1`, `SE1P` → `SE1`.
/// `LA10`, `LS7` and `N11` come back unchanged.
/// This matches DuckDB macro: `regexp_replace(outward(p), '([0-9])[A-Z]$', '\1')`.
pub fn district(outward: &str) -> &str {
    let bytes = outward.as_bytes();
    let n = bytes.len();
    if n >= 2 && bytes[n - 1].is_ascii_alphabetic() && bytes[n - 2].is_ascii_digit() {
        &outward[..n - 1]
    } else {
        outward
    }
}

/// Matches a row's stored postcode against a PostcodeValue.
///
/// Its outward code is the text before the first space (as split_part gives).
/// `District` compares `district` of that with the value.
/// `Outward` compares the outward code directly.
/// `Full` compares the whole postcode.
/// Every comparison is exact.
pub fn matches_postcode(row_postcode: &str, value: &PostcodeValue) -> bool {
    let row_outward = row_postcode.split_whitespace().next().unwrap_or("");
    match value {
        PostcodeValue::District(v) => district(row_outward) == v,
        PostcodeValue::Outward(v) => row_outward == v,
        PostcodeValue::Full(v) => row_postcode == v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_table() {
        let cases = [
            ("LA1", Some(PostcodeValue::District("LA1".into()))),
            ("LA10", Some(PostcodeValue::District("LA10".into()))),
            ("N1", Some(PostcodeValue::District("N1".into()))),
            ("SW1", Some(PostcodeValue::District("SW1".into()))),
            ("SW1A", Some(PostcodeValue::Outward("SW1A".into()))),
            ("E1W", Some(PostcodeValue::Outward("E1W".into()))),
            ("LS7", Some(PostcodeValue::District("LS7".into()))),
            ("LA105DL", Some(PostcodeValue::Full("LA10 5DL".into()))),
            ("la10 5dl", Some(PostcodeValue::Full("LA10 5DL".into()))),
            ("LA1 5", None),
            ("N1 1", None),
            ("ab", None),
            ("durham", None),
        ];

        for (input, expected) in cases {
            assert_eq!(classify(input), expected, "failed for input: {}", input);
        }
    }

    #[test]
    fn test_district_table() {
        let cases = [
            ("W1A", "W1"),
            ("EC1A", "EC1"),
            ("E1W", "E1"),
            ("SE1P", "SE1"),
            ("LA10", "LA10"),
            ("LS7", "LS7"),
            ("N11", "N11"),
        ];

        for (input, expected) in cases {
            assert_eq!(district(input), expected, "failed for input: {}", input);
        }
    }
}
