pub mod domain_rewrite;

/// Normalizes a handshake `serverAddress` for routing.
///
/// Two things are stripped:
/// - Forge Mod Loader markers (`\0FML\0`, `\0FML2\0`, `\0FML3\0`) appended by
///   Forge/Fabric clients after the hostname
/// - A trailing dot. The vanilla Java client passes the SRV-resolved FQDN
///   verbatim into the handshake `serverAddress` field, and Java's DNS
///   resolver returns SRV targets in absolute form (`mc.example.com.`)
pub(crate) fn normalize_handshake(domain: &str) -> &str {
    let stripped = domain.find('\0').map_or(domain, |pos| &domain[..pos]);
    stripped.trim_end_matches('.')
}
