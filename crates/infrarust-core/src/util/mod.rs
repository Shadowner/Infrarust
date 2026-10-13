pub mod domain_rewrite;
pub(crate) mod guard;
pub mod provider_slot;
pub mod sync;
pub mod text;

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

pub(crate) fn host_suffix(raw_host: &str) -> &str {
    raw_host.find('\0').map_or("", |pos| &raw_host[pos..])
}

const FORGE_TOKENS: [&str; 2] = ["FML", "FORGE"];

fn is_forge_token(segment: &str) -> bool {
    FORGE_TOKENS.iter().any(|token| {
        segment
            .strip_prefix(token)
            .is_some_and(|version| version.bytes().all(|b| b.is_ascii_digit()))
    })
}

pub(crate) fn has_forge_marker(suffix: &str) -> bool {
    suffix.split('\0').any(is_forge_token)
}

pub(crate) fn forge_suffix(raw_host: &str) -> &str {
    let suffix = host_suffix(raw_host);
    let mut segments = suffix
        .split('\0')
        .filter(|segment| !segment.is_empty())
        .peekable();
    if segments.peek().is_some() && segments.all(is_forge_token) {
        suffix
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_suffix_starts_at_the_first_nul() {
        assert_eq!(host_suffix("mc.example.com"), "");
        assert_eq!(host_suffix("mc.example.com\0FML2\0"), "\0FML2\0");
        assert_eq!(host_suffix("mc.example.com.\0FORGE"), "\0FORGE");
        assert_eq!(host_suffix("\0FML\0"), "\0FML\0");
    }

    #[test]
    fn forge_markers_of_every_generation_are_recognised() {
        for marker in ["\0FML\0", "\0FML2\0", "\0FML3\0", "\0FORGE", "\0FORGE3"] {
            assert!(has_forge_marker(marker), "{marker:?}");
        }
    }

    #[test]
    fn only_a_suffix_made_of_forge_markers_is_kept() {
        for (raw_host, kept) in [
            ("mc.example.com", ""),
            ("mc.example.com\0FML\0", "\0FML\0"),
            ("mc.example.com\0FML2\0", "\0FML2\0"),
            ("mc.example.com\0FML3\0", "\0FML3\0"),
            ("mc.example.com\0FORGE", "\0FORGE"),
            ("mc.example.com\0", ""),
            ("mc.example.com\0XFER\0", ""),
            (
                "mc.example.com\x00198.51.100.7\x00069a79f444e94726a5befca90e38aaf5\x00[]",
                "",
            ),
            (
                "mc.example.com\0FML3\0\x00198.51.100.7\x00069a79f444e94726a5befca90e38aaf5\x00[]",
                "",
            ),
            (
                "mc.example.com\x00198.51.100.7\x00069a79f444e94726a5befca90e38aaf5\0FML3\0",
                "",
            ),
        ] {
            assert_eq!(forge_suffix(raw_host), kept, "{raw_host:?}");
        }
    }

    #[test]
    fn other_suffixes_are_not_forge_markers() {
        for suffix in [
            "",
            "\0",
            "\0FMLX\0",
            "\0forge",
            "\0NEOFORGE",
            "\x00198.51.100.7\0abc",
        ] {
            assert!(!has_forge_marker(suffix), "{suffix:?}");
        }
    }
}
