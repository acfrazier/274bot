use super::settings::{GathererSettings, Location};
use api::gather_methods::SceneRegionInput;
use api::snapshot::WorldTile;

/// A retained work area; Auto resolves its anchor through the sliced search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaMode {
    Start,
    Custom,
    Auto,
}

impl AreaMode {
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("start") {
            Some(Self::Start)
        } else if value.eq_ignore_ascii_case("custom") {
            Some(Self::Custom)
        } else if value.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else {
            None
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Custom => "custom",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkArea {
    pub mode: AreaMode,
    pub anchor: WorldTile,
    pub radius: u16,
}

impl WorkArea {
    /// Resolve an area from settings and the retained Start anchor. A Start
    /// area has no stable anchor until the first ready local-player frame.
    pub fn resolve(
        settings: &GathererSettings,
        retained_anchor: Option<WorldTile>,
        here: Option<WorldTile>,
    ) -> Result<Self, AreaError> {
        let mode: AreaMode = settings.location_mode()?.into();
        let anchor = match mode {
            AreaMode::Start => retained_anchor.or(here).ok_or(AreaError::NotReady)?,
            AreaMode::Custom => settings.custom_tile.ok_or(AreaError::Invalid)?,
            AreaMode::Auto => retained_anchor.ok_or(AreaError::NotReady)?,
        };
        Ok(Self {
            mode,
            anchor,
            radius: settings.radius,
        })
    }

    pub const fn region(self) -> SceneRegionInput {
        let radius = self.radius as i32;
        SceneRegionInput {
            min_x: self.anchor.x.saturating_sub(radius),
            min_z: self.anchor.z.saturating_sub(radius),
            max_x: self.anchor.x.saturating_add(radius),
            max_z: self.anchor.z.saturating_add(radius),
            level: self.anchor.level,
        }
    }

    pub const fn contains(self, tile: WorldTile) -> bool {
        let region = self.region();
        tile.level == region.level
            && tile.x >= region.min_x
            && tile.x <= region.max_x
            && tile.z >= region.min_z
            && tile.z <= region.max_z
    }

    pub fn label(self) -> String {
        format!(
            "{} ({},{},{}) r{}",
            self.mode.name(),
            self.anchor.x,
            self.anchor.z,
            self.anchor.level,
            self.radius
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaError {
    Invalid,
    NotReady,
}

impl From<Location> for AreaMode {
    fn from(value: Location) -> Self {
        match value {
            Location::Start => Self::Start,
            Location::Custom => Self::Custom,
            Location::Auto => Self::Auto,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gatherer::settings::GathererSettings;

    #[test]
    fn start_captures_once_and_custom_is_position_independent() {
        let mut settings = GathererSettings {
            location: "Start".into(),
            radius: 12,
            ..GathererSettings::default()
        };
        let here = WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        };
        let first = WorkArea::resolve(&settings, None, Some(here)).unwrap();
        let moved = WorldTile { x: 3500, ..here };
        let second = WorkArea::resolve(&settings, Some(first.anchor), Some(moved)).unwrap();
        assert_eq!(first.anchor, second.anchor);

        settings.location = "Custom".into();
        settings.custom_tile = Some(WorldTile {
            x: 2912,
            z: 4833,
            level: 0,
        });
        let custom = WorkArea::resolve(&settings, None, Some(moved)).unwrap();
        assert_eq!(custom.anchor.x, 2912);
        assert_eq!(custom.anchor.z, 4833);
    }

    #[test]
    fn region_is_inclusive_and_saturating() {
        let area = WorkArea {
            mode: AreaMode::Custom,
            anchor: WorldTile {
                x: 0,
                z: 0,
                level: 1,
            },
            radius: 2,
        };
        assert!(area.contains(WorldTile {
            x: 2,
            z: -2,
            level: 1,
        }));
        assert!(!area.contains(WorldTile {
            x: 3,
            z: 0,
            level: 1,
        }));
        assert!(!area.contains(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        }));
    }
}
