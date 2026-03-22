//! Minimal NBT registry codec builder for limbo JoinGame (1.16–1.20.1).
//!
//! Constructs the smallest valid registry codec NBT that the Minecraft client
//! will accept. The JoinGame packet for 1.16–1.20.1 embeds this data inline.
const TAG_END: u8 = 0;
const TAG_BYTE: u8 = 1;
const TAG_INT: u8 = 3;
const TAG_FLOAT: u8 = 5;
const TAG_DOUBLE: u8 = 6;
const TAG_STRING: u8 = 8;
const TAG_LIST: u8 = 9;
const TAG_COMPOUND: u8 = 10;

pub(crate) fn build_registry_codec(pvn: i32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);

    buf.push(TAG_COMPOUND);
    write_nbt_string(&mut buf, "");

    if pvn < 751 {
        write_registry_entry_container(&mut buf, "dimension", "minecraft:dimension_type", &end_dimension_type_element(pvn));
    } else {
        write_registry_entry_container(&mut buf, "minecraft:dimension_type", "minecraft:dimension_type", &end_dimension_type_element(pvn));

        write_registry_entry_container(&mut buf, "minecraft:worldgen/biome", "minecraft:worldgen/biome", &plains_biome_element(pvn));

        if pvn >= 759 {
            write_chat_type_registry(&mut buf);
        }

        if pvn >= 762 {
            write_damage_type_registry(&mut buf);
        }
    }

    buf.push(TAG_END);
    buf
}

pub(crate) fn build_dimension_codec(pvn: i32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    buf.push(TAG_COMPOUND);
    write_nbt_string(&mut buf, "");

    write_dimension_type_fields(&mut buf, pvn);

    buf.push(TAG_END);
    buf
}

/// Writes a registry container: `{ "type": type_id, "value": [ entry ] }`
fn write_registry_entry_container(buf: &mut Vec<u8>, key: &str, type_id: &str, element: &[u8]) {
    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, key);

    write_nbt_string_field(buf, "type", type_id);

    buf.push(TAG_LIST);
    write_nbt_string(buf, "value");
    buf.push(TAG_COMPOUND); 
    buf.extend_from_slice(&1_i32.to_be_bytes());

    write_nbt_string_field(buf, "name", "minecraft:the_end");
    write_nbt_int_field(buf, "id", 0);

    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "element");
    buf.extend_from_slice(element);
    buf.push(TAG_END);

    buf.push(TAG_END);
    buf.push(TAG_END);
}

fn end_dimension_type_element(pvn: i32) -> Vec<u8> {
    let mut buf = Vec::new();
    write_dimension_type_fields(&mut buf, pvn);
    buf
}

fn write_dimension_type_fields(buf: &mut Vec<u8>, pvn: i32) {
    write_nbt_byte_field(buf, "piglin_safe", 0);
    write_nbt_byte_field(buf, "natural", 0);
    write_nbt_float_field(buf, "ambient_light", 0.0);

    if pvn >= 759 {
        // 1.19+: infiniburn uses tag syntax
        write_nbt_string_field(buf, "infiniburn", "#minecraft:infiniburn_end");
    } else {
        write_nbt_string_field(buf, "infiniburn", "minecraft:infiniburn_end");
    }

    write_nbt_byte_field(buf, "respawn_anchor_works", 0);
    write_nbt_byte_field(buf, "has_skylight", 0);
    write_nbt_byte_field(buf, "bed_works", 0);
    write_nbt_string_field(buf, "effects", "minecraft:the_end");
    write_nbt_byte_field(buf, "has_raids", 1);

    if pvn >= 751 {
        // 1.16.2+: min_y + height
        write_nbt_int_field(buf, "min_y", 0);
        write_nbt_int_field(buf, "height", 256);
    }

    write_nbt_int_field(buf, "logical_height", 256);
    write_nbt_double_field(buf, "coordinate_scale", 1.0);
    write_nbt_byte_field(buf, "ultrawarm", 0);
    write_nbt_byte_field(buf, "has_ceiling", 0);

    if pvn >= 759 {
        // 1.19+: monster spawn fields
        write_nbt_int_field(buf, "monster_spawn_block_light_limit", 0);
        // monster_spawn_light_level as compound (uniform distribution)
        buf.push(TAG_COMPOUND);
        write_nbt_string(buf, "monster_spawn_light_level");
        write_nbt_string_field(buf, "type", "minecraft:uniform");
        buf.push(TAG_COMPOUND);
        write_nbt_string(buf, "value");
        write_nbt_int_field(buf, "max_inclusive", 7);
        write_nbt_int_field(buf, "min_inclusive", 0);
        buf.push(TAG_END); 
        buf.push(TAG_END); 
    }
}
fn plains_biome_element(pvn: i32) -> Vec<u8> {
    let mut buf = Vec::new();

    if pvn < 759 {
        write_nbt_string_field(&mut buf, "precipitation", "rain");
    } else {
        write_nbt_byte_field(&mut buf, "has_precipitation", 1);
    }

    buf.push(TAG_COMPOUND);
    write_nbt_string(&mut buf, "effects");
    write_nbt_int_field(&mut buf, "sky_color", 7_907_327);
    write_nbt_int_field(&mut buf, "water_fog_color", 329_011);
    write_nbt_int_field(&mut buf, "fog_color", 12_638_463);
    write_nbt_int_field(&mut buf, "water_color", 4_159_204);

    buf.push(TAG_COMPOUND);
    write_nbt_string(&mut buf, "mood_sound");
    write_nbt_int_field(&mut buf, "tick_delay", 6000);
    write_nbt_double_field(&mut buf, "offset", 2.0);
    write_nbt_string_field(&mut buf, "sound", "minecraft:ambient.cave");
    write_nbt_int_field(&mut buf, "block_search_extent", 8);
    buf.push(TAG_END);

    buf.push(TAG_END);

    write_nbt_float_field(&mut buf, "temperature", 0.8);
    write_nbt_float_field(&mut buf, "downfall", 0.4);

    if pvn < 759 {
        write_nbt_float_field(&mut buf, "depth", 0.125);
        write_nbt_float_field(&mut buf, "scale", 0.05);
        write_nbt_string_field(&mut buf, "category", "plains");
    }

    buf
}

fn write_chat_type_registry(buf: &mut Vec<u8>) {
    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "minecraft:chat_type");

    write_nbt_string_field(buf, "type", "minecraft:chat_type");

    buf.push(TAG_LIST);
    write_nbt_string(buf, "value");
    buf.push(TAG_COMPOUND);
    buf.extend_from_slice(&1_i32.to_be_bytes());

    write_nbt_string_field(buf, "name", "minecraft:chat");
    write_nbt_int_field(buf, "id", 0);

    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "element");

    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "chat");
    write_nbt_string_field(buf, "translation_key", "chat.type.text");
    buf.push(TAG_LIST);
    write_nbt_string(buf, "parameters");
    buf.push(TAG_STRING);
    buf.extend_from_slice(&2_i32.to_be_bytes());
    write_nbt_raw_string(buf, "sender");
    write_nbt_raw_string(buf, "content");
    buf.push(TAG_END);

    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "narration");
    write_nbt_string_field(buf, "translation_key", "chat.type.text.narrate");
    buf.push(TAG_LIST);
    write_nbt_string(buf, "parameters");
    buf.push(TAG_STRING);
    buf.extend_from_slice(&2_i32.to_be_bytes());
    write_nbt_raw_string(buf, "sender");
    write_nbt_raw_string(buf, "content");
    buf.push(TAG_END);

    buf.push(TAG_END);
    buf.push(TAG_END);
    buf.push(TAG_END);
}

fn write_damage_type_registry(buf: &mut Vec<u8>) {
    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "minecraft:damage_type");

    write_nbt_string_field(buf, "type", "minecraft:damage_type");

    buf.push(TAG_LIST);
    write_nbt_string(buf, "value");
    buf.push(TAG_COMPOUND);
    buf.extend_from_slice(&1_i32.to_be_bytes());

    // Single damage type entry (generic)
    write_nbt_string_field(buf, "name", "minecraft:generic");
    write_nbt_int_field(buf, "id", 0);

    buf.push(TAG_COMPOUND);
    write_nbt_string(buf, "element");
    write_nbt_string_field(buf, "message_id", "generic");
    write_nbt_string_field(buf, "scaling", "never");
    write_nbt_float_field(buf, "exhaustion", 0.0);
    buf.push(TAG_END);

    buf.push(TAG_END);
    buf.push(TAG_END);
}

fn write_nbt_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// Writes a raw NBT string value (no tag header, just u16 length + bytes).
/// Used for list elements of type TAG_STRING.
fn write_nbt_raw_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(bytes);
}

fn write_nbt_string_field(buf: &mut Vec<u8>, name: &str, value: &str) {
    buf.push(TAG_STRING);
    write_nbt_string(buf, name);
    write_nbt_string(buf, value);
}

fn write_nbt_byte_field(buf: &mut Vec<u8>, name: &str, value: u8) {
    buf.push(TAG_BYTE);
    write_nbt_string(buf, name);
    buf.push(value);
}

fn write_nbt_int_field(buf: &mut Vec<u8>, name: &str, value: i32) {
    buf.push(TAG_INT);
    write_nbt_string(buf, name);
    buf.extend_from_slice(&value.to_be_bytes());
}

fn write_nbt_float_field(buf: &mut Vec<u8>, name: &str, value: f32) {
    buf.push(TAG_FLOAT);
    write_nbt_string(buf, name);
    buf.extend_from_slice(&value.to_be_bytes());
}

fn write_nbt_double_field(buf: &mut Vec<u8>, name: &str, value: f64) {
    buf.push(TAG_DOUBLE);
    write_nbt_string(buf, name);
    buf.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn registry_codec_1_16_not_empty() {
        let data = build_registry_codec(735);
        assert!(data.len() > 50, "1.16 registry codec should be non-trivial: {} bytes", data.len());
        assert_eq!(data[0], TAG_COMPOUND);
    }

    #[test]
    fn registry_codec_1_16_2_has_biome() {
        let data = build_registry_codec(751);
        assert!(data.len() > 100, "1.16.2 registry codec should include biome: {} bytes", data.len());
    }

    #[test]
    fn registry_codec_1_19_has_chat_type() {
        let data = build_registry_codec(759);
        assert!(data.len() > 200, "1.19 registry codec should include chat_type: {} bytes", data.len());
    }

    #[test]
    fn registry_codec_1_19_4_has_damage_type() {
        let data = build_registry_codec(762);
        assert!(data.len() > 300, "1.19.4 registry codec should include damage_type: {} bytes", data.len());
    }

    #[test]
    fn dimension_codec_1_16_2() {
        let data = build_dimension_codec(751);
        assert!(data.len() > 50, "dimension codec should be non-trivial: {} bytes", data.len());
        assert_eq!(data[0], TAG_COMPOUND);
    }

    #[test]
    fn registry_codec_starts_with_named_compound() {
        for pvn in [735, 751, 759, 762] {
            let data = build_registry_codec(pvn);
            assert_eq!(data[0], TAG_COMPOUND, "pvn {pvn}: must start with TAG_Compound");
            // Named root: next two bytes are name length (0 for empty name)
            assert_eq!(data[1], 0x00, "pvn {pvn}: name length high byte");
            assert_eq!(data[2], 0x00, "pvn {pvn}: name length low byte");
        }
    }

    #[test]
    fn registry_codec_ends_with_tag_end() {
        for pvn in [735, 751, 759, 762] {
            let data = build_registry_codec(pvn);
            assert_eq!(*data.last().unwrap(), TAG_END, "pvn {pvn}: must end with TAG_End");
        }
    }
}
