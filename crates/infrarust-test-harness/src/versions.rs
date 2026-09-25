use infrarust_protocol::version::ProtocolVersion;

pub const CURRENT: i32 = infrarust_protocol::CURRENT_MC_PROTOCOL;

pub const LIFECYCLE: &[i32] = &[47, 340, 754, 762, 763, 764, 765, 766, 770, 774, 776];

pub const SWITCH: &[i32] = &[47, 754, 763, 764, 766, CURRENT];

pub const CHAT: &[i32] = &[340, 758, 760, 762, 770, CURRENT];

pub const TEXT: &[i32] = &[47, 764, 765, CURRENT];

pub fn versions(matrix: &[i32]) -> impl Iterator<Item = ProtocolVersion> + '_ {
    matrix.iter().copied().map(ProtocolVersion)
}

#[macro_export]
macro_rules! version_matrix {
    (LIFECYCLE, $body:ident) => {
        $crate::version_matrix!(@named LIFECYCLE, $body;
            p47 = 47, p340 = 340, p754 = 754, p762 = 762, p763 = 763, p764 = 764,
            p765 = 765, p766 = 766, p770 = 770, p774 = 774, p776 = 776);
    };
    (SWITCH, $body:ident) => {
        $crate::version_matrix!(@named SWITCH, $body;
            p47 = 47, p754 = 754, p763 = 763, p764 = 764, p766 = 766, p774 = 774);
    };
    (CHAT, $body:ident) => {
        $crate::version_matrix!(@named CHAT, $body;
            p340 = 340, p758 = 758, p760 = 760, p762 = 762, p770 = 770, p774 = 774);
    };
    (TEXT, $body:ident) => {
        $crate::version_matrix!(@named TEXT, $body;
            p47 = 47, p764 = 764, p765 = 765, p774 = 774);
    };
    (@named $matrix:ident, $body:ident; $($name:ident = $version:literal),+ $(,)?) => {
        mod $body {
            $(
                #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
                async fn $name() {
                    super::$body($crate::ProtocolVersion($version)).await;
                }
            )+

            #[test]
            fn matrix_matches_constant() {
                assert_eq!(&[$($version),+][..], $crate::versions::$matrix);
            }
        }
    };
    ($body:ident; $($name:ident = $version:literal),+ $(,)?) => {
        mod $body {
            $(
                #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
                async fn $name() {
                    super::$body($crate::ProtocolVersion($version)).await;
                }
            )+
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_matrix_version_is_known_to_the_registry() {
        for matrix in [LIFECYCLE, SWITCH, CHAT, TEXT] {
            for version in versions(matrix) {
                assert!(
                    ProtocolVersion::SUPPORTED.contains(&version),
                    "{} is not in ProtocolVersion::SUPPORTED",
                    version.0
                );
            }
        }
    }
}
