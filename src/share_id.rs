use rand::Rng;

const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const SHARE_ID_LEN: usize = 16;

/// Generate a cryptographically random, URL-safe share ID.
pub fn generate() -> String {
    let mut rng = rand::thread_rng();
    (0..SHARE_ID_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Generate an API token with the `sss_` prefix.
pub fn generate_api_token() -> String {
    let mut rng = rand::thread_rng();
    let token: String = (0..40)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();
    format!("sss_{}", token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_ids_are_16_url_safe_chars() {
        let id = generate();

        assert_eq!(id.len(), SHARE_ID_LEN);
        assert!(id.bytes().all(|b| CHARSET.contains(&b)));
    }
}
