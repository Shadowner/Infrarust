use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IpMaskingMode {
    #[default]
    LastTwoOctets,
    LastOctet,
    None,
}

pub fn mask_ip_for_log(ip: &IpAddr, mode: &IpMaskingMode) -> String {
    match mode {
        IpMaskingMode::None => ip.to_string(),
        IpMaskingMode::LastOctet => mask_last_octet(ip),
        IpMaskingMode::LastTwoOctets => mask_last_two_octets(ip),
    }
}

fn mask_last_octet(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            format!("{}.{}.{}.x", octets[0], octets[1], octets[2])
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}:x:x:x:x",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
    }
}

fn mask_last_two_octets(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            format!("{}.{}.x.x", octets[0], octets[1])
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            format!(
                "{:x}:{:x}:{:x}:x:x:x:x:x",
                segments[0], segments[1], segments[2]
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_none() {
        let ip: IpAddr = "192.168.1.42".parse().unwrap();
        assert_eq!(mask_ip_for_log(&ip, &IpMaskingMode::None), "192.168.1.42");
    }

    #[test]
    fn ipv4_last_octet() {
        let ip: IpAddr = "192.168.1.42".parse().unwrap();
        assert_eq!(
            mask_ip_for_log(&ip, &IpMaskingMode::LastOctet),
            "192.168.1.x"
        );
    }

    #[test]
    fn ipv4_last_two() {
        let ip: IpAddr = "192.168.1.42".parse().unwrap();
        assert_eq!(
            mask_ip_for_log(&ip, &IpMaskingMode::LastTwoOctets),
            "192.168.x.x"
        );
    }

    #[test]
    fn ipv6_last_octet_shows_64_prefix() {
        let ip: IpAddr = "2001:db8:85a3:1234:5678:abcd:ef01:2345".parse().unwrap();
        assert_eq!(
            mask_ip_for_log(&ip, &IpMaskingMode::LastOctet),
            "2001:db8:85a3:1234:x:x:x:x"
        );
    }

    #[test]
    fn ipv6_last_two_shows_48_prefix() {
        let ip: IpAddr = "2001:db8:85a3:1234:5678:abcd:ef01:2345".parse().unwrap();
        assert_eq!(
            mask_ip_for_log(&ip, &IpMaskingMode::LastTwoOctets),
            "2001:db8:85a3:x:x:x:x:x"
        );
    }
}
