use client::config::Cache;

use super::{GameSnapshot, VarpView};

/// Why a varbit cannot be read from the current cache and snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarbitUnavailable {
    /// The selected cache has no definition at this id.
    MissingDefinition { id: i32 },
    /// The cache definition has no usable base-varp index.
    InvalidDefinition { id: i32, base_varp: i32 },
    /// The inclusive bit bounds are outside `0..=31` or reversed.
    InvalidBounds {
        id: i32,
        start_bit: i32,
        end_bit: i32,
    },
    /// The definition references no base varp in the selected cache.
    MissingBaseVarp { id: i32, base_varp: i32 },
    /// The snapshot has no row for the definition's base varp.
    MissingSnapshotValue { id: i32, base_varp: i32 },
    /// The caller has no independent receipt for the snapshot's base value.
    UnobservedBaseVarp { id: i32, base_varp: i32 },
}

/// Whether the selected base-varp value is known to have been received.
///
/// A snapshot row is not a packet receipt: the current snapshot representation
/// cannot distinguish an unobserved zero from a transmitted zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarbitObservation {
    /// The snapshot has a candidate base-varp value but no per-index receipt.
    Unknown { base_varp: i32, base_value: i32 },
}

/// Extracted varbit value and the observation status of its base varp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarbitValue {
    /// The value extracted using the selected cache definition's bit range.
    pub value: i32,
    /// Whether this base value is known to have been received.
    pub observation: VarbitObservation,
}

/// Read a varbit from its selected-cache definition and the snapshot's current
/// base-varp value. Bounds are the cache's inclusive bit range; the function
/// never guesses a range or synthesizes a missing base value.
///
/// A returned snapshot row is not proof that the server sent its value. In
/// particular, a zero may be a default for an unobserved varp. The returned
/// observation is therefore [`VarbitObservation::Unknown`]; callers must
/// supply a separate observed-varp witness before treating it as observed.
pub fn varbit_value(
    snapshot: &GameSnapshot,
    cache: &Cache,
    id: i32,
) -> Result<VarbitValue, VarbitUnavailable> {
    let id_index = usize::try_from(id).map_err(|_| VarbitUnavailable::MissingDefinition { id })?;
    let definition = cache
        .varbits
        .get(id_index)
        .ok_or(VarbitUnavailable::MissingDefinition { id })?;

    let base_varp = definition.basevar;
    if base_varp < 0 {
        return Err(VarbitUnavailable::InvalidDefinition { id, base_varp });
    }
    let start_bit = definition.startbit;
    let end_bit = definition.endbit;
    if !(0..=31).contains(&start_bit) || !(start_bit..=31).contains(&end_bit) {
        return Err(VarbitUnavailable::InvalidBounds {
            id,
            start_bit,
            end_bit,
        });
    }

    let base_index = base_varp as usize;
    if cache.varps.get(base_index).is_none() {
        return Err(VarbitUnavailable::MissingBaseVarp { id, base_varp });
    }

    let value = snapshot
        .varps()
        .iter()
        .find(|row| row.index == base_varp)
        .map(|row: &VarpView| row.value)
        .ok_or(VarbitUnavailable::MissingSnapshotValue { id, base_varp })?;
    let width = (end_bit - start_bit + 1) as u32;
    let mask = if width == 32 {
        u32::MAX
    } else {
        (1_u32 << width) - 1
    };
    Ok(VarbitValue {
        value: ((value as u32 >> start_bit as u32) & mask) as i32,
        observation: VarbitObservation::Unknown {
            base_varp,
            base_value: value,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::config::runtime_377::{CacheFields377, InterfaceFields377, VarpFields377};
    use client::config::{CacheRevision377, VarBitType};

    fn cache_with_varbit(transmit: bool) -> Cache {
        let mut cache = Cache::default();
        cache.varbits.push(VarBitType {
            basevar: 0,
            startbit: 2,
            endbit: 4,
            debugname: String::new(),
        });
        cache.varps.push(Default::default());
        cache.revision_377 = Some(Box::new(CacheRevision377 {
            config: CacheFields377 {
                varps: vec![VarpFields377 {
                    code1: 0,
                    code2: 0,
                    tracked: false,
                    code4: false,
                    code6: false,
                    code7: 0,
                    scope: 0,
                    transmit,
                    code12: 0,
                    code14: false,
                    debugname: None,
                }],
                ..Default::default()
            },
            interfaces: InterfaceFields377 {
                client_code_layers: Default::default(),
                type1_fields: Vec::new(),
                null_string_masks: Vec::new(),
                initial_overlays: Vec::new(),
            },
        }));
        cache
    }

    fn snapshot(value: Option<i32>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_varps(
            value
                .into_iter()
                .map(|value| VarpView { index: 0, value })
                .collect(),
        );
        snapshot
    }

    #[test]
    fn reads_the_inclusive_cache_bit_range_with_unknown_observation() {
        let cache = cache_with_varbit(true);
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &cache, 0),
            Ok(VarbitValue {
                value: 3,
                observation: VarbitObservation::Unknown {
                    base_varp: 0,
                    base_value: 44,
                },
            })
        );
    }

    #[test]
    fn supports_the_full_32_bit_range_without_overflowing_the_mask() {
        let mut cache = cache_with_varbit(true);
        cache.varbits[0].startbit = 0;
        cache.varbits[0].endbit = 31;
        assert_eq!(
            varbit_value(&snapshot(Some(-1)), &cache, 0),
            Ok(VarbitValue {
                value: -1,
                observation: VarbitObservation::Unknown {
                    base_varp: 0,
                    base_value: -1,
                },
            })
        );
    }

    #[test]
    fn reports_missing_definition_base_varp_and_snapshot_value() {
        let cache = cache_with_varbit(true);
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &cache, 1),
            Err(VarbitUnavailable::MissingDefinition { id: 1 })
        );
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &cache, -1),
            Err(VarbitUnavailable::MissingDefinition { id: -1 })
        );

        let mut invalid_base = cache_with_varbit(true);
        invalid_base.varbits[0].basevar = -1;
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &invalid_base, 0),
            Err(VarbitUnavailable::InvalidDefinition {
                id: 0,
                base_varp: -1
            })
        );

        let mut missing_base = cache_with_varbit(true);
        missing_base.varbits[0].basevar = 1;
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &missing_base, 0),
            Err(VarbitUnavailable::MissingBaseVarp {
                id: 0,
                base_varp: 1
            })
        );

        let cache = cache_with_varbit(true);
        assert_eq!(
            varbit_value(&snapshot(None), &cache, 0),
            Err(VarbitUnavailable::MissingSnapshotValue {
                id: 0,
                base_varp: 0
            })
        );
    }

    #[test]
    fn reports_invalid_bounds_and_ignores_transmission_metadata() {
        let mut invalid = cache_with_varbit(true);
        invalid.varbits[0].endbit = 32;
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &invalid, 0),
            Err(VarbitUnavailable::InvalidBounds {
                id: 0,
                start_bit: 2,
                end_bit: 32
            })
        );
        let mut reversed = cache_with_varbit(true);
        reversed.varbits[0].startbit = 5;
        reversed.varbits[0].endbit = 4;
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &reversed, 0),
            Err(VarbitUnavailable::InvalidBounds {
                id: 0,
                start_bit: 5,
                end_bit: 4
            })
        );

        let unknown = Ok(VarbitValue {
            value: 3,
            observation: VarbitObservation::Unknown {
                base_varp: 0,
                base_value: 44,
            },
        });
        let private = cache_with_varbit(false);
        assert_eq!(varbit_value(&snapshot(Some(44)), &private, 0), unknown);

        let mut missing_metadata = cache_with_varbit(true);
        missing_metadata.revision_377 = None;
        assert_eq!(
            varbit_value(&snapshot(Some(44)), &missing_metadata, 0),
            unknown
        );
    }

    #[test]
    fn private_base_286_is_only_an_unknown_candidate() {
        let mut cache = cache_with_varbit(false);
        cache.varbits[0].basevar = 286;
        cache.varbits[0].startbit = 14;
        cache.varbits[0].endbit = 27;
        cache.varps.resize_with(287, Default::default);
        let fields = &mut cache.revision_377.as_mut().unwrap().config.varps;
        fields.resize(287, fields[0].clone());
        assert!(!fields[286].transmit);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_varps(vec![VarpView {
            index: 286,
            value: 1 << 14,
        }]);
        assert_eq!(
            varbit_value(&snapshot, &cache, 0),
            Ok(VarbitValue {
                value: 1,
                observation: VarbitObservation::Unknown {
                    base_varp: 286,
                    base_value: 1 << 14,
                },
            })
        );
    }

    #[test]
    fn repeated_queries_allocate_nothing_for_candidates_or_missing_values() {
        let cache = cache_with_varbit(true);
        let present = snapshot(Some(44));
        let absent = snapshot(None);
        let measured = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                std::hint::black_box(varbit_value(&present, &cache, 0)).unwrap();
                assert!(std::hint::black_box(varbit_value(&absent, &cache, 0)).is_err());
                assert!(std::hint::black_box(varbit_value(&present, &cache, 1)).is_err());
            }
        });
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.bytes_total, 0);
    }
}
