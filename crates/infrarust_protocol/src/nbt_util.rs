use serde::Serialize;

pub fn to_network_nbt<T: Serialize>(value: &T) -> Result<Vec<u8>, fastnbt::error::Error> {
    let mut bytes = fastnbt::to_bytes(value)?;
    if bytes.len() >= 3 && bytes[0] == 0x0A {
        bytes.drain(1..3);
    }
    Ok(bytes)
}

pub use fastnbt::LongArray;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct SimpleCompound {
        name: String,
        value: i32,
    }

    #[test]
    fn test_to_network_nbt_removes_root_name() {
        let data = SimpleCompound {
            name: "test".into(),
            value: 42,
        };

        let standard = fastnbt::to_bytes(&data).unwrap();
        let network = to_network_nbt(&data).unwrap();

        assert_eq!(standard[0], 0x0A);
        assert_eq!(standard[1], 0x00);
        assert_eq!(standard[2], 0x00);

        assert_eq!(network[0], 0x0A);
        assert_eq!(network.len(), standard.len() - 2);

        assert_eq!(&network[1..], &standard[3..]);
    }

    #[test]
    fn test_to_network_nbt_empty_struct() {
        #[derive(Serialize)]
        struct Empty {}
        let network = to_network_nbt(&Empty {}).unwrap();
        assert_eq!(network[0], 0x0A);
        assert_eq!(*network.last().unwrap(), 0x00);
    }
}
