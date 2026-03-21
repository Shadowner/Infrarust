# Infrarust — Limbo Registry Data: Full NBT Approach

> **Statut** : Implémenté
> **Date** : Mars 2026
> **Référence** : NanoLimbo (`github.com/BoomEaro/NanoLimbo`)

---

## Problème

Le système de registry data du limbo ne fonctionnait que pour MC 1.21.11 (via un fichier `v774.bin` embarqué). Le trick KnownPacks (`has_data: false`) était désactivé car le champ `version` dans `CKnownPacks` ne peut pas matcher fiablement la version du client — un même numéro de protocole couvre plusieurs versions de jeu (ex: protocole 767 = 1.21 et 1.21.1).

## Approche retenue : Full NBT (NanoLimbo)

Envoyer **toutes les entries avec `has_data: true`** et les données NBT complètes de l'élément. Pas de handshake KnownPacks. Le client reçoit tout ce dont il a besoin directement.

Les données d'éléments sont définies en Rust via des structs `serde::Serialize`, sérialisées avec `fastnbt` (déjà une dépendance), et cachées au démarrage via `LazyLock`.

---

## Les 4 groupes de versions

### Groupe A — Legacy (1.7.2 → 1.15.2)

Pas de registry data. JoinGame est un paquet simple avec dimension ID entier. Déjà fonctionnel via `raw_payload` dans `spawn.rs`.

### Groupe B — JoinGame inline (1.16 → 1.20.1)

Le codec registry est embarqué dans JoinGame comme un compound tag NBT nommé. Trois sous-ères :

| Versions | Codec | Dimension dans JoinGame |
|---|---|---|
| 1.16.0-1.16.1 | Format plat (`"dimension": [...]`) | Nom (string) |
| 1.16.2-1.18.2 | Format registry wrapper (2-3 registries) | Element compound tag + nom |
| 1.19-1.20.1 | Format registry wrapper (3-4 registries) | Nom (string) uniquement |

Le codec est construit en Rust via `codec_nbt::codec_bytes_for_version()` qui retourne les bytes NBT standard (avec root name).

### Groupe C — Config bundled (1.20.2 → 1.20.4)

Un seul paquet `CRegistryData` contenant tout le codec en NBT nameless (format réseau). Construit via `codec_nbt::codec_network_nbt_for_config()`.

Flow :
```
LoginSuccess → LoginAcknowledged → Config state
  Serveur → Client : CRegistryData (codec complet en nameless NBT)
  Serveur → Client : CFinishConfig
  Client → Serveur : SAcknowledgeFinishConfig
→ Play state
```

### Groupe D — Config split (1.20.5+)

Un `CRegistryData` par type de registry. Chaque entry a `has_data: true` avec l'élément NBT complet en nameless compound.

Flow :
```
LoginSuccess → LoginAcknowledged → Config state
  Serveur → Client : CRegistryData("minecraft:dimension_type", [...])
  Serveur → Client : CRegistryData("minecraft:worldgen/biome", [...])
  Serveur → Client : CRegistryData("minecraft:damage_type", [...])
  ... (un paquet par registry)
  Serveur → Client : CFinishConfig
  Client → Serveur : SAcknowledgeFinishConfig
→ Play state
```

Pas de `CKnownPacks` envoyé.

---

## Données du limbo

Le limbo utilise un monde void dans The End avec un seul biome (plains).

### Entries envoyées

| Registry | Entry | Données |
|---|---|---|
| `minecraft:dimension_type` | `minecraft:the_end` | Element complet (15 champs) |
| `minecraft:worldgen/biome` | `minecraft:plains` | Element complet (température, effets, mood_sound) |
| `minecraft:chat_type` | `minecraft:chat` | Décorations chat + narration |
| `minecraft:damage_type` | 50 entries | Tous les types vanilla (message_id, scaling, exhaustion) |
| `minecraft:painting_variant` | `minecraft:kebab` | asset_id, dimensions |
| `minecraft:wolf_variant` | `minecraft:pale` | textures wild/tame/angry, biomes |
| `minecraft:banner_pattern` | `minecraft:base` | Stub minimal |
| `minecraft:trim_material` | `minecraft:iron` | Stub minimal |
| `minecraft:trim_pattern` | `minecraft:coast` | Stub minimal |
| Autres (jukebox, enchantment, instrument, variants...) | 1 entry chacun | Stubs minimaux |

### Types NBT

Les champs booléens MC sont des `TAG_Byte` (`i8` en Rust, pas `bool`). Les éléments utilisent les types exacts attendus par le client :

- `i8` → TAG_Byte (booleans)
- `i32` → TAG_Int
- `f32` → TAG_Float
- `f64` → TAG_Double
- `i64` → TAG_Long
- `String` → TAG_String

---

## Architecture

```
infrarust-core/src/registry_data/
  mod.rs                  ← RegistryDataProvider trait
  codec_nbt.rs            ← Types serde + données + builders de codec
  full_nbt.rs             ← FullNbtRegistryProvider (Groupe D, >= 1.20.5)
  version_router.rs       ← Dispatch par version (Groupes C et D)
  entry_lists/
    mod.rs                ← LimboRegistryEntries + get_entries()
    v766.rs               ← Noms des entries par registry
  embedded.rs             ← Ancien provider binaire (conservé pour tests)

infrarust-core/src/limbo/
  spawn.rs                ← Construction JoinGame (Groupes A et B)
  login.rs                ← Complétion config phase (Groupes C et D)
```

### `codec_nbt.rs`

Module central. Contient :

- **Structs serde** : `DimensionTypeElement`, `BiomeElement`, `DamageTypeElement`, `ChatTypeElement`, `PaintingVariantElement`, `WolfVariantElement`
- **Données statiques** : `the_end_dimension()`, `plains_biome()`, `chat_type_chat()`, table de 50 damage types
- **Cache d'éléments** : `ELEMENT_CACHE` (entries principales) et `STUB_CACHE` (entries minimales), initialisés via `LazyLock`
- **Builders de codec** : `build_codec_old()` à `build_codec_1_20()` pour les 7 ères de codec (Groupes B et C)
- **API publique** : `element_nbt()`, `codec_bytes_for_version()`, `codec_network_nbt_for_config()`, `the_end_element_bytes()`

### `full_nbt.rs`

Provider pour >= 1.20.5. Itère les entries de `entry_lists/v766.rs`, récupère les bytes NBT via `codec_nbt::element_nbt()`, et construit les frames `CRegistryData` avec `has_data: true`.

### `version_router.rs`

Dispatch :
- `< 1.20.2` → erreur (géré par `spawn.rs`)
- `1.20.2-1.20.4` → `build_bundled_codec_frames()` (codec complet en un seul paquet)
- `>= 1.20.5` → `FullNbtRegistryProvider`

### `spawn.rs`

`build_limbo_join_game_116_to_1201()` construit le `raw_payload` du JoinGame avec le codec NBT inline. Trois branches pour les sous-ères 1.16.

---

## Pourquoi pas KnownPacks ?

Le trick KnownPacks envoie `has_data: false` et compte sur le client pour charger les données depuis son pack local `minecraft:core`. Problèmes :

1. Le champ `version` du `KnownPack` doit matcher exactement la version du jeu du client. Infrarust ne connaît que le numéro de protocole, pas la version exacte (ex: protocole 768 = 1.21.2, 1.21.3, ou 1.21.4).
2. NanoLimbo n'utilise pas KnownPacks et fonctionne pour toutes les versions.
3. L'approche full NBT est universelle : fonctionne avec les clients vanilla, moddés, et toute version future sans dépendre du contenu local du client.

Le coût réseau supplémentaire (~5-10 KB vs ~500 bytes) est négligeable pour un limbo.
