pub mod domain_rewrite;

/// Strips Forge Mod Loader markers from a domain string.
///
/// FML markers (`\0FML\0`, `\0FML2\0`, `\0FML3\0`) are appended by
/// Forge/Fabric clients in the handshake hostname.
pub(crate) fn strip_fml_markers(domain: &str) -> &str {
    domain.find('\0').map_or(domain, |pos| &domain[..pos])
}
