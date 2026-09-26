use super::cache::*;
use super::formats::*;
use super::identity::*;
use super::poi::*;
use super::records::*;
use super::spatial::*;
use super::{MapError, Rows, Text};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn image_identity() -> ImageIdentity {
    ImageIdentity {
        revision: 289,
        content: Digest([0x11; 32]),
        policy: Digest([0x22; 32]),
    }
}
fn catalogue_identity() -> CatalogueIdentity {
    CatalogueIdentity {
        revision: 289,
        content: Digest([0x11; 32]),
        policy: Digest([0x22; 32]),
    }
}
fn policy() -> BakePolicy {
    BakePolicy {
        algorithm: Text::new("terrain-v1").unwrap(),
        producer_sources: Digest([0x33; 32]),
        encoder: EncoderLibrary {
            name: Text::new("png").unwrap(),
            version: Text::new("0.17.16").unwrap(),
        },
        libraries: Rows::new(vec![
            EncoderLibrary {
                name: Text::new("flate2").unwrap(),
                version: Text::new("1.1.2").unwrap(),
            },
            EncoderLibrary {
                name: Text::new("miniz_oxide").unwrap(),
                version: Text::new("0.8.9").unwrap(),
            },
        ])
        .unwrap(),
        options: Text::new("rgba8/filter=adaptive/compression=6").unwrap(),
    }
}
fn booth() -> PoiRecord {
    PoiRecord {
        key: PoiKey {
            entity: EntityKind::Loc,
            id: 2213,
            x: 3513,
            z: 3479,
            source: SourceSpace::ClientVisual {
                plane: 1,
                link_below: true,
            },
            shape: 10,
            rotation: 0,
        },
        name: Text::new("Bank booth").unwrap(),
        kind: PoiKind::Bank,
        effective_plane: 0,
        footprint: Footprint {
            width: 1,
            length: 1,
        },
        display: DisplayAnchor {
            x: 3513.5,
            z: 3479.5,
            plane: 0,
        },
        evidence: Rows::new(vec![CapabilityEvidence::ActiveQuickBooth]).unwrap(),
        walk_target: None,
    }
}
fn coverage() -> Coverage {
    Coverage {
        npc_placements: CoverageLevel::Unavailable,
        bank_services: CoverageLevel::Limited,
        place_labels: CoverageLevel::Unavailable,
        unresolved: Rows::new(vec![]).unwrap(),
    }
}
fn catalogue() -> ClientPois {
    ClientPois {
        schema: CATALOGUE_SCHEMA,
        identity: catalogue_identity(),
        coverage: coverage(),
        records: Rows::new(vec![booth()]).unwrap(),
    }
}
fn receipt(bytes: &[u8]) -> PayloadReceipt {
    PayloadReceipt {
        bytes: bytes.len() as u32,
        sha256: Digest::of(bytes),
    }
}
fn catalogue_manifest(bytes: &[u8]) -> CatalogueManifest {
    CatalogueManifest {
        schema: CATALOGUE_SCHEMA,
        identity: catalogue_identity(),
        key: catalogue_identity().key().unwrap(),
        record_count: 1,
        payload: receipt(bytes),
    }
}
fn tile_key() -> TileKey {
    TileKey {
        plane: 0,
        lod: 0,
        x: 54,
        z: 54,
    }
}
fn image_manifest(bytes: &[u8]) -> ImageManifest {
    let bounds = WorldBounds {
        west: 3456,
        south: 3456,
        east: 3520,
        north: 3520,
    };
    ImageManifest {
        schema: IMAGE_SCHEMA,
        identity: image_identity(),
        key: image_identity().key().unwrap(),
        extent: bounds,
        planes: Rows::new(vec![PlaneBounds { plane: 0, bounds }]).unwrap(),
        max_lod: 0,
        interior: TILE_INTERIOR,
        gutter: TILE_GUTTER,
        color: ColorFormat::Rgba8Unorm,
        tiles: Rows::new(vec![TileReceipt {
            key: tile_key(),
            payload: receipt(bytes),
        }])
        .unwrap(),
    }
}
fn png_header(width: u32, height: u32) -> Vec<u8> {
    // Deliberately header-only: verify_png is a preallocation guard, not a PNG
    // decoder. The external consumer smoke uses a complete independently built PNG.
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[test]
fn canifis_visual_booth_shifts_once_but_server_banker_does_not() {
    let booth = booth();
    booth.validate().unwrap();
    assert_eq!(booth.key.source.game_plane().unwrap(), Some(0));
    let mut npc = booth.clone();
    npc.key = PoiKey {
        entity: EntityKind::Npc,
        id: 1036,
        x: 3514,
        z: 3479,
        source: SourceSpace::ServerGame { plane: 0 },
        shape: 0,
        rotation: 0,
    };
    npc.name = Text::new("Banker").unwrap();
    npc.display.x = 3514.5;
    npc.evidence = Rows::new(vec![CapabilityEvidence::ClientOperation {
        capability: Capability::Bank,
        slot: OperationSlot::new(3).unwrap(),
    }])
    .unwrap();
    npc.validate().unwrap();
    assert_eq!(npc.key.source.game_plane().unwrap(), Some(0));
    assert_eq!(
        SourceSpace::Game { plane: 1 }.game_plane().unwrap(),
        Some(1)
    );
    assert_eq!(
        SourceSpace::ClientVisual {
            plane: 0,
            link_below: true
        }
        .game_plane()
        .unwrap(),
        None
    );
    assert!(matches!(
        SourceSpace::ServerGame { plane: 4 }.game_plane(),
        Err(MapError::Invalid(_))
    ));
    npc.key.source = booth.key.source;
    assert!(matches!(
        npc.validate(),
        Err(MapError::Invalid("non-visual entity source space"))
    ));
    let mut shifted_twice = booth;
    shifted_twice.key.source = SourceSpace::ClientVisual {
        plane: 0,
        link_below: true,
    };
    assert!(matches!(
        shifted_twice.validate(),
        Err(MapError::Invalid("POI plane conversion"))
    ));
}

#[test]
fn canonical_keys_match_independent_binary_preimages_and_encoder_invalidation() {
    // Independently generated with node:crypto over documented binary fields,
    // not serialized Rust/JSON or an invocation of the implementation under test.
    assert_eq!(
        image_identity().key().unwrap().0.to_string(),
        "bb1a5823644de7575fa7e27ec6087de85704e4c44c0eb502ab9ccb42135d72c4"
    );
    assert_eq!(
        catalogue_identity().key().unwrap().0.to_string(),
        "99a66ffec090f7df1de64f784dcaa4b8c70d3a920db5666d73e7eaa6a7a19c94"
    );
    let mut encoder = policy();
    let before = encoder.identity().unwrap();
    assert_eq!(
        before.to_string(),
        "a0427a8aae783885f514d5fb6b673f698184ae989431bd88765bf1a822e02d5a"
    );
    encoder.encoder.version = Text::new("0.18.0").unwrap();
    let after = encoder.identity().unwrap();
    assert_ne!(
        ImageIdentity {
            policy: before,
            ..image_identity()
        }
        .key()
        .unwrap(),
        ImageIdentity {
            policy: after,
            ..image_identity()
        }
        .key()
        .unwrap()
    );
    assert_ne!(
        image_identity().key().unwrap(),
        ImageIdentity {
            revision: 274,
            ..image_identity()
        }
        .key()
        .unwrap()
    );
    assert_ne!(
        catalogue_identity()
            .merged_key(Digest([1; 32]), None)
            .unwrap(),
        catalogue_identity()
            .merged_key(Digest([1; 32]), Some(Digest([0; 32])))
            .unwrap()
    );
    assert_ne!(
        catalogue_identity()
            .merged_key(Digest([1; 32]), None)
            .unwrap(),
        catalogue_identity()
            .merged_key(Digest([2; 32]), None)
            .unwrap()
    );
    assert!(Digest::from_hex(&"AA".repeat(32)).is_err());
}

#[test]
fn classifier_preserves_slots_and_distinguishes_discovery_from_service_proof() {
    let mut classified = Vec::new();
    let mut definition = Definition {
        entity: EntityKind::Npc,
        name: "Banker",
        operations: [None, None, Some(" Bank "), None, None],
        active: true,
        mapfunction: None,
    };
    classify_definition(289, &definition, |kind, evidence| {
        classified.push((kind, evidence))
    });
    assert_eq!(
        classified,
        vec![(
            PoiKind::Bank,
            CapabilityEvidence::ClientOperation {
                capability: Capability::Bank,
                slot: OperationSlot::new(3).unwrap()
            }
        )]
    );
    classified.clear();
    definition.entity = EntityKind::Loc;
    definition.name = "Bank noticeboard";
    definition.operations = [Some("Read"), None, None, None, None];
    classify_definition(289, &definition, |kind, evidence| {
        classified.push((kind, evidence))
    });
    assert!(classified.is_empty());
    definition.name = "Bank booth";
    definition.operations = [Some("Use"), Some("Use-quickly"), None, None, None];
    classify_definition(289, &definition, |kind, evidence| {
        classified.push((kind, evidence))
    });
    assert_eq!(
        classified,
        vec![(PoiKind::Bank, CapabilityEvidence::ActiveQuickBooth)]
    );
    classified.clear();
    definition.active = false;
    classify_definition(289, &definition, |kind, evidence| {
        classified.push((kind, evidence))
    });
    assert!(classified.is_empty());
    definition.mapfunction = Some(50);
    classify_definition(289, &definition, |kind, evidence| {
        classified.push((kind, evidence))
    });
    assert_eq!(
        classified,
        vec![(
            PoiKind::MapSymbol { symbol: 50 },
            CapabilityEvidence::MapFunction { symbol: 50 }
        )]
    );
    classified.clear();
    definition.mapfunction = Some(5);
    classify_definition(274, &definition, |kind, evidence| {
        classified.push((kind, evidence))
    });
    assert_eq!(classified[0].0, PoiKind::MapSymbol { symbol: 5 });
    assert!(OperationSlot::new(0).is_err());
    assert!(OperationSlot::new(6).is_err());
}

fn view(width: f64, height: f64, pixels_per_tile: f64) -> View {
    View {
        west: 0.0,
        south: 0.0,
        east: width / pixels_per_tile,
        north: height / pixels_per_tile,
        pixels_per_tile,
        plane: 0,
        max_lod: MAX_LOD,
    }
}
#[test]
fn lod_obeys_native_density_slot_cap_and_half_open_negative_edges() {
    let ordinary = select_lod(view(765.0, 503.0, 4.0), TEXTURE_CAP).unwrap();
    assert_eq!((ordinary.lod, ordinary.len()), (0, 6));
    let hidpi = select_lod(view(7680.0, 4320.0, 4.0), TEXTURE_CAP).unwrap();
    assert_eq!((hidpi.lod, hidpi.len()), (3, 12));
    let narrow = select_lod(view(16_384.0, 128.0, 4.0), TEXTURE_CAP).unwrap();
    assert_eq!((narrow.lod, narrow.len()), (2, 16));
    let negative = select_lod(
        View {
            west: -64.0,
            south: -64.0,
            east: 0.0,
            north: 0.0,
            ..view(1.0, 1.0, 4.0)
        },
        TEXTURE_CAP,
    )
    .unwrap();
    assert_eq!(
        negative.keys().collect::<Vec<_>>(),
        vec![TileKey {
            plane: 0,
            lod: 0,
            x: -1,
            z: -1
        }]
    );
    assert_eq!(
        TileKey {
            lod: 1,
            x: -1,
            z: -1,
            ..tile_key()
        }
        .bounds()
        .unwrap(),
        [-128, -128, 0, 0]
    );
    assert_eq!(
        select_lod(view(765.0, 503.0, 0.75), TEXTURE_CAP)
            .unwrap()
            .lod,
        2
    );
}
#[test]
fn lod_rejects_unrepresentable_or_unsatisfiable_views_instead_of_overallocating() {
    for invalid in [
        View {
            pixels_per_tile: f64::NAN,
            ..view(765.0, 503.0, 4.0)
        },
        View {
            max_lod: 27,
            ..view(765.0, 503.0, 4.0)
        },
        View {
            east: 0.0,
            ..view(765.0, 503.0, 4.0)
        },
    ] {
        assert!(matches!(
            select_lod(invalid, TEXTURE_CAP),
            Err(MapError::Invalid(_))
        ));
    }
    let crossing = View {
        west: -1.0,
        south: -1.0,
        east: 1.0,
        north: 1.0,
        ..view(1.0, 1.0, 4.0)
    };
    assert!(matches!(select_lod(crossing, 3), Err(MapError::Limit(_))));
    assert!(select_lod(view(765.0, 503.0, 4.0), 25).is_err());
    assert!(select_lod(
        View {
            max_lod: 0,
            ..view(7680.0, 4320.0, 4.0)
        },
        TEXTURE_CAP
    )
    .is_err());
}
#[test]
fn north_up_centres_edges_inverse_and_rotated_footprint_are_unambiguous() {
    let v = View {
        west: -64.0,
        south: -128.0,
        east: 0.0,
        north: -64.0,
        ..view(256.0, 256.0, 4.0)
    };
    assert_eq!(v.world_to_screen(-63.5, -64.5), [2.0, 2.0]);
    assert_eq!(v.screen_to_world(2.0, 2.0), [-63.5, -64.5]);
    assert_eq!(
        v.tile_at(0.0, 0.0).unwrap(),
        Some(GameTile {
            x: -64,
            z: -65,
            plane: 0
        })
    );
    assert_eq!(
        v.tile_at(255.0, 255.0).unwrap(),
        Some(GameTile {
            x: -1,
            z: -128,
            plane: 0
        })
    );
    assert_eq!(v.tile_at(256.0, 0.0).unwrap(), None);
    assert_eq!(v.tile_at(0.0, 256.0).unwrap(), None);
    assert_eq!(
        Footprint {
            width: 2,
            length: 3
        }
        .rotated(1)
        .unwrap(),
        Footprint {
            width: 3,
            length: 2
        }
    );
    assert_eq!(TILE_RGBA_BYTES, 266_256);
    assert_eq!(INTERIOR_UV[0][0] * 258.0, 1.0);
    assert_eq!(INTERIOR_UV[1][0] * 258.0, 257.0);
}
#[test]
fn snap_uses_distance_then_coordinates_and_never_fabricates_a_miss() {
    let request = GameTile {
        x: 0,
        z: 0,
        plane: 2,
    };
    let found = snap_walkable(request, |t| {
        matches!((t.x, t.z), (-1, -1) | (-1, 0) | (0, -1) | (2, 0))
    });
    assert_eq!(
        found,
        Some(GameTile {
            x: -1,
            z: 0,
            plane: 2
        })
    );
    let mut examined = 0;
    assert_eq!(
        snap_walkable(request, |_| {
            examined += 1;
            false
        }),
        None
    );
    assert_eq!(examined, 33 * 33);
    assert_eq!(snap_walkable(request, |t| t.x == 17), None);
    assert_eq!(
        snap_walkable(
            GameTile {
                x: i32::MIN,
                z: i32::MAX,
                plane: 0
            },
            |_| false
        ),
        None
    );
}

#[test]
fn catalogue_rejects_identity_and_version_before_deserializing_records() {
    let identity = serde_json::to_string(&catalogue_identity()).unwrap();
    let wrong_version = format!("{{\"records\":[0],\"identity\":{identity},\"schema\":2}}");
    assert!(matches!(
        ClientPois::decode(wrong_version.as_bytes(), catalogue_identity()),
        Err(MapError::Unsupported { .. })
    ));
    let wrong_identity = format!("{{\"records\":[0],\"identity\":{identity},\"schema\":1}}");
    assert!(matches!(
        ClientPois::decode(
            wrong_identity.as_bytes(),
            CatalogueIdentity {
                revision: 274,
                ..catalogue_identity()
            }
        ),
        Err(MapError::Identity)
    ));
    assert!(matches!(
        ClientPois::decode(&vec![b' '; MAX_JSON_BYTES + 1], catalogue_identity()),
        Err(MapError::Limit(_))
    ));
}
#[test]
fn catalogue_rejects_duplicate_unknown_and_non_client_records() {
    let mut c = catalogue();
    c.records = Rows::new(vec![booth(), booth()]).unwrap();
    assert!(matches!(
        ClientPois::decode(&serde_json::to_vec(&c).unwrap(), c.identity),
        Err(MapError::Duplicate("POI key"))
    ));
    let mut value = serde_json::to_value(catalogue()).unwrap();
    value["unexpected"] = true.into();
    assert!(matches!(
        ClientPois::decode(&serde_json::to_vec(&value).unwrap(), catalogue_identity()),
        Err(MapError::Schema(_))
    ));
    let duplicated_field = format!(
        "{{\"schema\":1,\"schema\":1,\"identity\":{}}}",
        serde_json::to_string(&catalogue_identity()).unwrap()
    );
    assert!(matches!(
        ClientPois::decode(duplicated_field.as_bytes(), catalogue_identity()),
        Err(MapError::Schema(_))
    ));
    value = serde_json::to_value(catalogue()).unwrap();
    value["records"][0]["name"] = "x".repeat(Text::MAX_BYTES + 1).into();
    assert!(matches!(
        ClientPois::decode(&serde_json::to_vec(&value).unwrap(), catalogue_identity()),
        Err(MapError::Schema(_))
    ));
    value = serde_json::to_value(catalogue()).unwrap();
    value["records"][0]["walk_target"] =
        serde_json::json!({"tile":{"x":3514,"z":3479,"plane":0},"nav_sha256":Digest([9;32])});
    assert!(matches!(
        ClientPois::decode(&serde_json::to_vec(&value).unwrap(), catalogue_identity()),
        Err(MapError::Invalid("non-client POI in client catalogue"))
    ));
}
#[test]
fn bounded_sequences_refuse_extra_elements_before_decoding_them() {
    // 999 cannot deserialize as u8: it must be rejected for count, not converted.
    let error = serde_json::from_str::<Rows<u8, 2>>("[1,2,999]").unwrap_err();
    assert!(error.to_string().contains("record count limit"));
}
#[test]
fn maximum_image_documents_fit_the_json_contract() {
    const START: i32 = -33_000_000;
    assert_eq!(MAX_IMAGE_TILES, 5_440);
    let west = START * 64;
    let east = (START + MAX_IMAGE_TILES as i32) * 64;
    let south = START * 64;
    let north = south + 64;
    let bounds = WorldBounds {
        west,
        south,
        east,
        north,
    };
    let receipts: Vec<_> = (0..MAX_IMAGE_TILES)
        .map(|index| TileReceipt {
            key: TileKey {
                plane: 0,
                lod: 0,
                x: START + index as i32,
                z: START,
            },
            // 255 maximum PNGs plus one byte for every remaining tile stays
            // below MAX_IMAGE_BYTES while exercising both receipt widths.
            payload: PayloadReceipt {
                bytes: if index < 255 { MAX_PNG_BYTES } else { 1 },
                sha256: Digest([0xff; 32]),
            },
        })
        .collect();
    let manifest = ImageManifest {
        schema: IMAGE_SCHEMA,
        identity: image_identity(),
        key: image_identity().key().unwrap(),
        extent: bounds,
        planes: Rows::new(vec![PlaneBounds { plane: 0, bounds }]).unwrap(),
        max_lod: 0,
        interior: TILE_INTERIOR,
        gutter: TILE_GUTTER,
        color: ColorFormat::Rgba8Unorm,
        tiles: Rows::new(receipts.clone()).unwrap(),
    };
    let manifest_bytes = manifest.encode().unwrap();
    assert!(
        manifest_bytes.len() <= MAX_JSON_BYTES,
        "{}-byte maximum image manifest exceeds {} bytes",
        manifest_bytes.len(),
        MAX_JSON_BYTES
    );

    let checkpoint = Checkpoint {
        schema: CHECKPOINT_SCHEMA,
        identity: ArtifactIdentity::Image(image_identity()),
        stage: BakeStage::Publishing,
        planned_units: MAX_IMAGE_TILES as u32,
        completed: Rows::new(
            receipts
                .into_iter()
                .map(|receipt| CompletedUnit {
                    key: UnitKey::Terrain { tile: receipt.key },
                    payload: receipt.payload,
                })
                .collect(),
        )
        .unwrap(),
    };
    let checkpoint_bytes = checkpoint.encode().unwrap();
    assert!(
        checkpoint_bytes.len() <= MAX_JSON_BYTES,
        "{}-byte maximum checkpoint exceeds {} bytes",
        checkpoint_bytes.len(),
        MAX_JSON_BYTES
    );
}

#[test]
fn image_header_rejects_pixel_bombs_before_tile_deserialization() {
    let mut value = serde_json::to_value(image_manifest(b"png")).unwrap();
    value["interior"] = u32::MAX.into();
    value["tiles"] = serde_json::json!([0]);
    assert!(matches!(
        ImageManifest::decode(&serde_json::to_vec(&value).unwrap(), image_identity()),
        Err(MapError::Invalid("raster dimensions/LOD"))
    ));
    let bytes = png_header(u32::MAX, TILE_PIXELS);
    let tile = TileReceipt {
        key: tile_key(),
        payload: receipt(&bytes),
    };
    assert!(matches!(tile.verify_png(&bytes), Err(MapError::Invalid(_))));
    let bytes = png_header(TILE_PIXELS, TILE_PIXELS);
    assert_eq!(
        TileReceipt {
            key: tile_key(),
            payload: receipt(&bytes)
        }
        .verify_png(&bytes)
        .unwrap(),
        TILE_RGBA_BYTES
    );
    assert!(matches!(
        PayloadReceipt {
            bytes: MAX_PNG_BYTES + 1,
            sha256: Digest([0; 32])
        }
        .verify(&[], MAX_PNG_BYTES),
        Err(MapError::Limit(_))
    ));
}
#[test]
fn image_manifest_rejects_duplicate_keys_missing_parent_and_traversal_fields() {
    let mut m = image_manifest(b"png");
    let base = m.tiles.as_slice()[0];
    m.tiles = Rows::new(vec![base, base]).unwrap();
    assert!(matches!(
        m.validate(m.identity),
        Err(MapError::Duplicate("tile key"))
    ));
    m = image_manifest(b"png");
    m.max_lod = 1;
    m.extent.east = 3648;
    m.planes = Rows::new(vec![PlaneBounds {
        plane: 0,
        bounds: m.extent,
    }])
    .unwrap();
    m.tiles = Rows::new(vec![
        base,
        TileReceipt {
            key: TileKey {
                lod: 1,
                x: 28,
                z: 27,
                ..tile_key()
            },
            payload: base.payload,
        },
    ])
    .unwrap();
    assert!(matches!(
        m.validate(m.identity),
        Err(MapError::Invalid("missing parent tile"))
    ));
    let mut value = serde_json::to_value(image_manifest(b"png")).unwrap();
    value["tiles"][0]["path"] = "../../other.png".into();
    assert!(matches!(
        ImageManifest::decode(&serde_json::to_vec(&value).unwrap(), image_identity()),
        Err(MapError::Schema(_))
    ));
}

fn service_identity() -> ServiceIdentity {
    ServiceIdentity {
        revision: 289,
        content: Digest([0x11; 32]),
        nav_sha256: Digest([1; 32]),
        source_sha256: Digest([2; 32]),
        generator_sha256: Digest([3; 32]),
        policy: Digest([4; 32]),
    }
}
#[test]
fn navpois_rejects_stale_source_corruption_and_oversized_declarations() {
    let s = ServicePois {
        schema: NAVPOIS_VERSION,
        identity: service_identity(),
        coverage: coverage(),
        records: Rows::new(vec![booth()]).unwrap(),
    };
    let bytes = s.encode_navpois().unwrap();
    let file_digest = Digest::of(&bytes);
    let decoded = ServicePois::decode_navpois(&bytes, s.identity, file_digest).unwrap();
    assert_eq!(decoded.records.as_slice()[0].effective_plane, 0);
    assert!(matches!(
        ServicePois::decode_navpois(
            &bytes,
            ServiceIdentity {
                source_sha256: Digest([5; 32]),
                ..s.identity
            },
            file_digest
        ),
        Err(MapError::Identity)
    ));
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        ServicePois::decode_navpois(&corrupt, s.identity, file_digest),
        Err(MapError::Digest)
    ));
    let mut excessive = bytes.clone();
    excessive[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        ServicePois::decode_navpois(&excessive, s.identity, file_digest),
        Err(MapError::Limit("navpois header"))
    ));
    let mut future = bytes;
    future[4] = 2;
    assert!(matches!(
        ServicePois::decode_navpois(&future, s.identity, file_digest),
        Err(MapError::Unsupported { .. })
    ));
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "274bot-map-contracts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn partial_catalogue_requires_checkpoint_verification_and_explicit_publication() {
    let root = Fixture::new();
    let identity = catalogue_identity();
    let key = identity.key().unwrap().0;
    let partial = root.0.join(format!(".{key}.partial"));
    std::fs::create_dir(&partial).unwrap();
    let payload = catalogue().encode().unwrap();
    std::fs::write(partial.join("client-pois.json"), &payload).unwrap();
    std::fs::write(
        partial.join("manifest.json"),
        serde_json::to_vec(&catalogue_manifest(&payload)).unwrap(),
    )
    .unwrap();
    let checkpoint = Checkpoint {
        schema: CHECKPOINT_SCHEMA,
        identity: ArtifactIdentity::Catalogue(identity),
        stage: BakeStage::Publishing,
        planned_units: 1,
        completed: Rows::new(vec![CompletedUnit {
            key: UnitKey::ClientPois,
            payload: receipt(&payload),
        }])
        .unwrap(),
    };
    std::fs::write(
        partial.join("checkpoint.json"),
        checkpoint.encode().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        ReadyCatalogue::open(&partial, identity),
        Err(MapError::NotReady)
    ));
    let entry = PartialEntry::at(&partial, checkpoint.identity).unwrap();
    assert_eq!(
        entry.load_checkpoint().unwrap().completed.as_slice()[0].payload,
        receipt(&payload)
    );
    assert!(matches!(
        Checkpoint::decode(
            &checkpoint.encode().unwrap(),
            ArtifactIdentity::Catalogue(CatalogueIdentity {
                policy: Digest([9; 32]),
                ..identity
            })
        ),
        Err(MapError::Identity)
    ));
    let ready = root.0.join(key.to_string());
    std::fs::rename(&partial, &ready).unwrap();
    assert!(PartialEntry::at(&ready, checkpoint.identity).is_err());
    assert_eq!(
        ReadyCatalogue::open(&ready, identity)
            .unwrap()
            .pois()
            .records
            .as_slice()[0]
            .name
            .as_str(),
        "Bank booth"
    );
    std::fs::write(ready.join("client-pois.json"), b"{}").unwrap();
    assert!(matches!(
        ReadyCatalogue::open(&ready, identity),
        Err(MapError::Invalid("payload length"))
    ));
}
#[test]
fn ready_images_distinguish_unlisted_art_from_a_damaged_listed_tile() {
    let root = Fixture::new();
    let ready = root.0.join(image_identity().key().unwrap().0.to_string());
    std::fs::create_dir(&ready).unwrap();
    let png = png_header(TILE_PIXELS, TILE_PIXELS);
    std::fs::write(
        ready.join("manifest.json"),
        image_manifest(&png).encode().unwrap(),
    )
    .unwrap();
    let image = ReadyImages::open(&ready, image_identity()).unwrap();
    let mut buffer = Vec::new();
    assert_eq!(
        image
            .read_tile_into(
                TileKey {
                    x: 55,
                    ..tile_key()
                },
                &mut buffer
            )
            .unwrap(),
        None
    );
    assert!(matches!(
        image.read_tile_into(tile_key(), &mut buffer),
        Err(MapError::Io(_))
    ));
    let tile = ready.join(tile_key().relative_path().unwrap());
    std::fs::create_dir_all(tile.parent().unwrap()).unwrap();
    std::fs::write(&tile, &png).unwrap();
    assert_eq!(
        image.read_tile_into(tile_key(), &mut buffer).unwrap(),
        Some(266_256)
    );
    let mut corrupt = png;
    corrupt[20] ^= 1;
    std::fs::write(&tile, &corrupt).unwrap();
    assert!(matches!(
        image.read_tile_into(tile_key(), &mut buffer),
        Err(MapError::Digest)
    ));
}
#[test]
fn image_checkpoint_is_invalidated_by_encoder_policy_changes() {
    let old = ImageIdentity {
        policy: policy().identity().unwrap(),
        ..image_identity()
    };
    let checkpoint = Checkpoint {
        schema: CHECKPOINT_SCHEMA,
        identity: ArtifactIdentity::Image(old),
        stage: BakeStage::BaseTerrain,
        planned_units: 2,
        completed: Rows::new(vec![CompletedUnit {
            key: UnitKey::Terrain { tile: tile_key() },
            payload: receipt(b"png"),
        }])
        .unwrap(),
    };
    let mut encoder = policy();
    encoder.libraries = Rows::new(vec![EncoderLibrary {
        name: Text::new("libpng").unwrap(),
        version: Text::new("1.6.50").unwrap(),
    }])
    .unwrap();
    let changed = ImageIdentity {
        policy: encoder.identity().unwrap(),
        ..old
    };
    assert!(matches!(
        Checkpoint::decode(
            &checkpoint.encode().unwrap(),
            ArtifactIdentity::Image(changed)
        ),
        Err(MapError::Identity)
    ));
}

struct TrackedReader {
    inner: Cursor<Vec<u8>>,
    read_bytes: usize,
}
impl Read for TrackedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(out)?;
        self.read_bytes += n;
        Ok(n)
    }
}
impl Seek for TrackedReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(from)
    }
}
#[test]
fn map_record_index_seeks_past_payloads_and_reads_only_the_requested_record() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&9u32.to_le_bytes());
    bytes.extend_from_slice(&65536u32.to_le_bytes());
    bytes.resize(8 + 65536, 0x77);
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&[7, 8, 9]);
    let mut input = TrackedReader {
        inner: Cursor::new(bytes),
        read_bytes: 0,
    };
    let index = MapRecordIndex::read(&mut input).unwrap();
    assert_eq!(input.read_bytes, 16);
    assert_eq!(
        index.records().iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![2, 9]
    );
    let mut buffer = Vec::new();
    assert!(index.read_record(&mut input, 2, &mut buffer).unwrap());
    assert_eq!(buffer, [7, 8, 9]);
    assert_eq!(input.read_bytes, 19);
    assert!(!index.read_record(&mut input, 3, &mut buffer).unwrap());
}
#[test]
fn map_record_index_rejects_duplicate_truncated_and_oversized_records() {
    let row = [1, 0, 0, 0, 1, 0, 0, 0, 7];
    assert!(matches!(
        MapRecordIndex::read(&mut Cursor::new([row, row].concat())),
        Err(MapError::Duplicate("map record id"))
    ));
    assert!(matches!(
        MapRecordIndex::read(&mut Cursor::new(&row[..8])),
        Err(MapError::Truncated)
    ));
    let huge = [0, 0, 0, 0, 255, 255, 255, 255];
    assert!(matches!(
        MapRecordIndex::read(&mut Cursor::new(huge)),
        Err(MapError::Limit("map record bytes"))
    ));
}
