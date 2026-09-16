/// Phone number normalization utility for DeezChatz CLI.
/// Complies with ITU-T E.164 international numbering plan.

/// List of standard 2-digit ITU-T calling codes.
const TWO_DIGIT_CALLING_CODES: &[&str] = &[
    "20", "27",
    "30", "31", "32", "33", "34", "36", "39",
    "40", "41", "43", "44", "45", "46", "47", "48", "49",
    "51", "52", "53", "54", "55", "56", "57", "58",
    "60", "61", "62", "63", "64", "65", "66",
    "81", "82", "84", "86",
    "90", "91", "92", "93", "94", "95", "98",
];

/// Parses a string of digits into (calling_code, national_number) if possible.
pub fn parse_country_code(digits: &str) -> Option<(&str, &str)> {
    if digits.is_empty() {
        return None;
    }

    // 1-digit codes: +1 (NANP: US/Canada/Caribbean), +7 (Russia/Kazakhstan)
    if digits.starts_with('1') || digits.starts_with('7') {
        return Some((&digits[..1], &digits[1..]));
    }

    // 2-digit codes
    if digits.len() >= 2 {
        let two = &digits[..2];
        if TWO_DIGIT_CALLING_CODES.contains(&two) {
            return Some((two, &digits[2..]));
        }
    }

    // 3-digit codes (all remaining valid ITU-T codes)
    if digits.len() >= 3 {
        return Some((&digits[..3], &digits[3..]));
    }

    None
}

/// Extracts the international calling code from a user's phone string (e.g. "+919999999999" -> "91").
pub fn extract_country_code(phone: &str) -> Option<String> {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }

    parse_country_code(&digits).map(|(code, _)| code.to_string())
}

/// Normalizes a contact identifier (phone number, email, or UUID) into canonical form.
///
/// If input is an email (contains '@') or a UUID, it is returned trimmed as-is.
/// Otherwise, treats input as a phone number:
/// - Strips spaces, dashes, parentheses, and dots.
/// - If input does not specify an international country code (no leading '+'):
///     - Removes preceding 0s (national trunk prefix).
///     - Prepends the user's calling code (defaulting to "91").
/// - If input specifies an international country code (starts with '+'):
///     - Preserves the country code.
///     - Removes preceding 0s from the national number portion if present.
/// - Produces standard E.164 output (e.g. "+918906620865").
pub fn normalize_identifier(input: &str, default_country_code: Option<&str>) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 1. Email address
    if trimmed.contains('@') {
        return trimmed.to_lowercase();
    }

    // 2. UUID
    if uuid::Uuid::parse_str(trimmed).is_ok() {
        return trimmed.to_string();
    }

    // 3. Phone Number normalization to E.164
    let default_code = default_country_code
        .unwrap_or("91")
        .trim_start_matches('+');

    if trimmed.starts_with('+') {
        // Explicit international code provided
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return String::new();
        }

        if let Some((code, national)) = parse_country_code(&digits) {
            let cleaned_national = national.trim_start_matches('0');
            format!("+{code}{cleaned_national}")
        } else {
            let cleaned = digits.trim_start_matches('0');
            format!("+{cleaned}")
        }
    } else {
        // No '+' prefix: strip non-digits and remove preceding zeros
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        let cleaned_national = digits.trim_start_matches('0');
        if cleaned_national.is_empty() {
            return String::new();
        }

        format!("+{default_code}{cleaned_national}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_country_code() {
        assert_eq!(extract_country_code("+919999999999").as_deref(), Some("91"));
        assert_eq!(extract_country_code("+14155552671").as_deref(), Some("1"));
        assert_eq!(extract_country_code("+447911123456").as_deref(), Some("44"));
        assert_eq!(extract_country_code("+971501234567").as_deref(), Some("971"));
        assert_eq!(extract_country_code("").as_deref(), None);
    }

    #[test]
    fn test_normalize_local_phone_without_prefix() {
        assert_eq!(
            normalize_identifier("8906620865", Some("91")),
            "+918906620865"
        );
    }

    #[test]
    fn test_normalize_local_phone_with_preceding_zeros() {
        assert_eq!(
            normalize_identifier("08906620865", Some("91")),
            "+918906620865"
        );
        assert_eq!(
            normalize_identifier("008906620865", Some("91")),
            "+918906620865"
        );
    }

    #[test]
    fn test_normalize_phone_with_formatting() {
        assert_eq!(
            normalize_identifier(" (08906) 620-865 ", Some("91")),
            "+918906620865"
        );
    }

    #[test]
    fn test_normalize_e164_with_plus() {
        assert_eq!(
            normalize_identifier("+918906620865", Some("91")),
            "+918906620865"
        );
        assert_eq!(
            normalize_identifier("+91 89066 20865", Some("91")),
            "+918906620865"
        );
        assert_eq!(
            normalize_identifier("+1 (555) 234-5678", Some("91")),
            "+15552345678"
        );
    }

    #[test]
    fn test_normalize_e164_with_plus_and_trunk_zero() {
        assert_eq!(
            normalize_identifier("+91 08906620865", Some("91")),
            "+918906620865"
        );
        assert_eq!(
            normalize_identifier("+44 07911 123456", Some("91")),
            "+447911123456"
        );
    }

    #[test]
    fn test_normalize_preserves_email_and_uuid() {
        assert_eq!(
            normalize_identifier("User@Example.com", Some("91")),
            "user@example.com"
        );
        assert_eq!(
            normalize_identifier("7b8f9e0a-1234-4567-89ab-cdef01234567", Some("91")),
            "7b8f9e0a-1234-4567-89ab-cdef01234567"
        );
    }
}
