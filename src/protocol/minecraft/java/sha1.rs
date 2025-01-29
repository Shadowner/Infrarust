use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tracing::debug;
use num_bigint::{BigInt, Sign};
use sha1::{digest::Update, Digest, Sha1};

pub fn calc_hash(input: &str) -> String {
    let hash = Sha1::new().chain(input).finalize();
    BigInt::from_signed_bytes_be(&hash).to_str_radix(16)
}

pub fn generate_server_hash(server_id: &str, shared_secret: &[u8], public_key: &[u8]) -> String {
    debug!("Generating server hash with:");
    debug!("Server ID (empty is normal): '{}'", server_id);
    debug!("Shared secret length: {}", shared_secret.len());
    debug!("Public key length: {}", public_key.len());
    debug!("Public key base64: {}", BASE64.encode(public_key));

    // Encoder server_id en ISO-8859-1 (pratiquement identique à ASCII pour les caractères simples)
    let server_id_bytes = server_id.as_bytes();

    let hash = Sha1::new()
        .chain(server_id_bytes)
        .chain(shared_secret)
        .chain(public_key)
        .finalize();

    // Convertir le hash en BigInt signé et le formater en hex
    let big_int = BigInt::from_signed_bytes_be(&hash);
    let hex = big_int.to_str_radix(16);

    // Ajouter le signe négatif si nécessaire (comme le fait le client Java)
    if big_int.sign() == Sign::Minus {
        format!("-{}", hex.replace("-", ""))
    } else {
        hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_hash() {
        assert_eq!(
            calc_hash("jeb_"),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
        assert_eq!(
            calc_hash("Notch"),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
        assert_eq!(
            calc_hash("simon"),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn test_server_hash() {
        let hash = generate_server_hash("test", b"secret", b"key");
        assert!(!hash.is_empty());
    }
}
