use rand::Rng;

/// Generate a 6-digit pairing code (`100000`..=`999999`).
pub fn generate_pairing_code() -> String {
    let n: u32 = rand::thread_rng().gen_range(100_000..=999_999);
    format!("{n:06}")
}

/// Normalize and validate a pairing code (exactly 6 digits).
pub fn verify_pairing_code(code: &str) -> bool {
    let trimmed = code.trim();
    trimmed.len() == 6 && trimmed.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_six_digits() {
        let code = generate_pairing_code();
        assert!(verify_pairing_code(&code));
    }
}
