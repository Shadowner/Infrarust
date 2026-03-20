//! Vanilla registry entry lists by protocol version.

pub(crate) mod v766;

use infrarust_protocol::version::ProtocolVersion;

pub(crate) struct LimboRegistryEntries {
    pub dimension_types: &'static [&'static str],
    pub biomes: &'static [&'static str],
    pub chat_types: &'static [&'static str],
    pub damage_types: &'static [&'static str],
    pub painting_variants: &'static [&'static str],
    pub wolf_variants: &'static [&'static str],
    pub banner_patterns: &'static [&'static str],
    pub trim_materials: &'static [&'static str],
    pub trim_patterns: &'static [&'static str],

    pub jukebox_songs: Option<&'static [&'static str]>,
    pub enchantments: Option<&'static [&'static str]>,
    pub instruments: Option<&'static [&'static str]>,
    pub wolf_sound_variants: Option<&'static [&'static str]>,
    pub pig_variants: Option<&'static [&'static str]>,
    pub frog_variants: Option<&'static [&'static str]>,
    pub cat_variants: Option<&'static [&'static str]>,
    pub cow_variants: Option<&'static [&'static str]>,
    pub chicken_variants: Option<&'static [&'static str]>,

    pub zombie_nautilus_variants: Option<&'static [&'static str]>,
    pub test_environments: Option<&'static [&'static str]>,
    pub test_instances: Option<&'static [&'static str]>,
    pub dialogs: Option<&'static [&'static str]>,
    pub timelines: Option<&'static [&'static str]>,
}

impl LimboRegistryEntries {
    #[allow(dead_code)]
    pub fn dimension_id(&self, name: &str) -> Option<i32> {
        self.dimension_types
            .iter()
            .position(|&e| e == name)
            .map(|i| i as i32)
    }

    pub fn registries(&self, version: ProtocolVersion) -> Vec<(&'static str, &'static [&'static str])> {
        let mut out = vec![
            ("minecraft:dimension_type", self.dimension_types),
            ("minecraft:worldgen/biome", self.biomes),
            ("minecraft:chat_type", self.chat_types),
            ("minecraft:damage_type", self.damage_types),
            ("minecraft:painting_variant", self.painting_variants),
            ("minecraft:wolf_variant", self.wolf_variants),
            ("minecraft:banner_pattern", self.banner_patterns),
            ("minecraft:trim_material", self.trim_materials),
            ("minecraft:trim_pattern", self.trim_patterns),
        ];

        if version.no_less_than(ProtocolVersion::V1_21) {
            if let Some(songs) = self.jukebox_songs {
                out.push(("minecraft:jukebox_song", songs));
            }
        }

        if version.no_less_than(ProtocolVersion::V1_21_2) {
            if let Some(enchants) = self.enchantments {
                out.push(("minecraft:enchantment", enchants));
            }
            if let Some(instr) = self.instruments {
                out.push(("minecraft:instrument", instr));
            }
        }

        if version.no_less_than(ProtocolVersion::V1_21_5) {
            if let Some(v) = self.wolf_sound_variants {
                out.push(("minecraft:wolf_sound_variant", v));
            }
            if let Some(v) = self.pig_variants {
                out.push(("minecraft:pig_variant", v));
            }
            if let Some(v) = self.frog_variants {
                out.push(("minecraft:frog_variant", v));
            }
            if let Some(v) = self.cat_variants {
                out.push(("minecraft:cat_variant", v));
            }
            if let Some(v) = self.cow_variants {
                out.push(("minecraft:cow_variant", v));
            }
            if let Some(v) = self.chicken_variants {
                out.push(("minecraft:chicken_variant", v));
            }
        }

        if version.no_less_than(ProtocolVersion::V1_21_9) {
            if let Some(v) = self.zombie_nautilus_variants {
                out.push(("minecraft:zombie_nautilus_variant", v));
            }
        }

        if version.no_less_than(ProtocolVersion::V1_21_11) {
            if let Some(v) = self.test_environments {
                out.push(("minecraft:test_environment", v));
            }
            if let Some(v) = self.test_instances {
                out.push(("minecraft:test_instance", v));
            }
            if let Some(v) = self.dialogs {
                out.push(("minecraft:dialog", v));
            }
            if let Some(v) = self.timelines {
                out.push(("minecraft:timeline", v));
            }
        }

        out
    }
}

pub(crate) fn get_entries(version: ProtocolVersion) -> Option<&'static LimboRegistryEntries> {
    if version.no_less_than(ProtocolVersion::V1_20_5) {
        Some(&v766::ENTRIES)
    } else {
        None
    }
}
