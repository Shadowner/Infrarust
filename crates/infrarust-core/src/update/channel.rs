use infrarust_config::UpdateChannel;
use semver::Version;

pub fn parse_version(tag: &str) -> Option<Version> {
    let raw = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(raw).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFilter {
    prereleases: bool,
}

impl ChannelFilter {
    pub fn new(channel: UpdateChannel, current: &Version) -> Self {
        let prereleases = match channel {
            UpdateChannel::Auto => !current.pre.is_empty(),
            UpdateChannel::Stable => false,
            UpdateChannel::Prerelease => true,
        };
        Self { prereleases }
    }

    pub const fn admits_prereleases(&self) -> bool {
        self.prereleases
    }

    pub fn admits(&self, version: &Version) -> bool {
        version.pre.is_empty() || self.prereleases
    }
}

pub fn newer_first<T>(
    items: impl IntoIterator<Item = (Version, T)>,
    filter: ChannelFilter,
    current: &Version,
) -> Vec<(Version, T)> {
    let mut candidates: Vec<(Version, T)> = items
        .into_iter()
        .filter(|(version, _)| version > current && filter.admits(version))
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn v(text: &str) -> Version {
        parse_version(text).unwrap()
    }

    fn offered(channel: UpdateChannel, current: &str, tags: &[&str]) -> Vec<String> {
        let current = v(current);
        let items = tags
            .iter()
            .filter_map(|tag| parse_version(tag).map(|version| (version, *tag)));
        newer_first(items, ChannelFilter::new(channel, &current), &current)
            .into_iter()
            .map(|(_, tag)| tag.to_string())
            .collect()
    }

    #[test]
    fn tags_parse_with_or_without_a_v() {
        assert_eq!(v("v2.0.0-beta.6"), v("2.0.0-beta.6"));
        assert!(parse_version("nightly").is_none());
        assert!(parse_version("v2.0").is_none());
        assert!(parse_version("").is_none());
    }

    #[test]
    fn prerelease_numbers_compare_numerically() {
        assert!(v("2.0.0-beta.4") < v("2.0.0-beta.10"));
        assert!(v("2.0.0-beta.10") < v("2.0.0"));
        assert!(v("v2.0.0-beta.4") < v("v2.0.0"));
    }

    #[test]
    fn a_beta_build_on_auto_sees_betas_and_stables() {
        let tags = ["v2.0.0-beta.4", "v2.0.0-beta.10", "v2.0.0", "v1.9.0"];
        assert_eq!(
            offered(UpdateChannel::Auto, "2.0.0-beta.5", &tags),
            ["v2.0.0", "v2.0.0-beta.10"]
        );
    }

    #[test]
    fn a_stable_build_on_auto_sees_stables_only() {
        let tags = ["v2.1.0-beta.1", "v2.0.1", "v2.0.0"];
        assert_eq!(offered(UpdateChannel::Auto, "2.0.0", &tags), ["v2.0.1"]);
    }

    #[test]
    fn the_stable_channel_hides_prereleases_even_on_a_beta_build() {
        let tags = ["v2.0.0-beta.10", "v2.0.0"];
        assert_eq!(
            offered(UpdateChannel::Stable, "2.0.0-beta.5", &tags),
            ["v2.0.0"]
        );
        assert!(offered(UpdateChannel::Stable, "2.0.0-beta.5", &["v2.0.0-beta.10"]).is_empty());
    }

    #[test]
    fn the_prerelease_channel_shows_prereleases_on_a_stable_build() {
        let tags = ["v2.1.0-beta.1", "v2.0.1"];
        assert_eq!(
            offered(UpdateChannel::Prerelease, "2.0.0", &tags),
            ["v2.1.0-beta.1", "v2.0.1"]
        );
    }

    #[test]
    fn equal_or_lower_versions_are_never_offered() {
        let tags = ["v2.0.0-beta.5", "v2.0.0-beta.4", "v1.0.0", "garbage"];
        for channel in [
            UpdateChannel::Auto,
            UpdateChannel::Stable,
            UpdateChannel::Prerelease,
        ] {
            assert!(offered(channel, "2.0.0-beta.5", &tags).is_empty());
        }
    }
}
