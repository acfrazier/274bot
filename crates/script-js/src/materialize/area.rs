use super::callback_v8 as cb;
use rand::random;
use rustyscript::Runtime;

const RECTANGULAR: f64 = 0.0;
const CIRCULAR: f64 = 1.0;
const RANDOM_ATTEMPTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tile {
    x: f64,
    z: f64,
    level: f64,
}

#[derive(Clone, Copy, Debug)]
enum Area {
    Rectangular {
        min_x: f64,
        max_x: f64,
        min_z: f64,
        max_z: f64,
        level: f64,
    },
    Circular {
        center: Tile,
        radius: f64,
    },
}

impl Area {
    fn rectangular(a: Tile, b: Tile) -> Self {
        Self::Rectangular {
            min_x: js_min(a.x, b.x),
            max_x: js_max(a.x, b.x),
            min_z: js_min(a.z, b.z),
            max_z: js_max(a.z, b.z),
            level: a.level,
        }
    }

    fn circular(center: Tile, radius: f64) -> Self {
        Self::Circular { center, radius }
    }

    fn contains(self, tile: Tile, live_center: Option<Tile>) -> bool {
        match self {
            Self::Rectangular {
                min_x,
                max_x,
                min_z,
                max_z,
                level,
            } => {
                tile.level == level
                    && tile.x >= min_x
                    && tile.x <= max_x
                    && tile.z >= min_z
                    && tile.z <= max_z
            }
            Self::Circular {
                center: stored_center,
                radius,
            } => {
                let center = live_center.unwrap_or(stored_center);
                let dx = tile.x - center.x;
                let dz = tile.z - center.z;
                tile.level == center.level && dx * dx + dz * dz <= radius * radius
            }
        }
    }

    fn random_tile(self, live_center: Option<Tile>) -> Tile {
        let mut sample = || random::<f64>();
        self.random_tile_with(live_center, &mut sample)
    }

    fn random_tile_with(self, live_center: Option<Tile>, sample: &mut impl FnMut() -> f64) -> Tile {
        match self {
            Self::Rectangular {
                min_x,
                max_x,
                min_z,
                max_z,
                level,
            } => Tile {
                x: min_x + (sample() * (max_x - min_x + 1.0)).floor(),
                z: min_z + (sample() * (max_z - min_z + 1.0)).floor(),
                level,
            },
            Self::Circular {
                center: stored_center,
                radius,
            } => {
                let center = live_center.unwrap_or(stored_center);
                for _ in 0..RANDOM_ATTEMPTS {
                    let tile = Tile {
                        x: center.x + (sample() * (2.0 * radius + 1.0)).floor() - radius,
                        z: center.z + (sample() * (2.0 * radius + 1.0)).floor() - radius,
                        level: center.level,
                    };
                    if self.contains(tile, Some(center)) {
                        return tile;
                    }
                }
                center
            }
        }
    }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() || b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a < b {
        a
    } else {
        b
    }
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() && b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if a > b {
        a
    } else {
        b
    }
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name =
        v8::String::new(&mut scope, "__rs2b0t_area").ok_or_else(|| "area name".to_string())?;
    let func =
        v8::Function::new(&mut scope, area_callback).ok_or_else(|| "area function".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "area set".to_string())?;
    Ok(())
}

fn area_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run_area(scope, &args);
    cb::finish(scope, rv, result);
}

fn area_error<'s>(scope: &mut v8::HandleScope<'s>, error: String) -> cb::Throw<'s> {
    let message = v8::String::new(scope, &error).unwrap_or_else(|| v8::String::empty(scope));
    cb::Throw::Value(v8::Exception::error(scope, message))
}

fn run_area<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args
        .get(0)
        .int32_value(scope)
        .ok_or_else(|| area_error(scope, "invalid area operation".to_string()))?;
    match operation {
        0 => {
            let a =
                required_tile(scope, args.get(1), "a").map_err(|error| area_error(scope, error))?;
            let b =
                required_tile(scope, args.get(2), "b").map_err(|error| area_error(scope, error))?;
            area_value(scope, Area::rectangular(a, b)).map_err(|error| area_error(scope, error))
        }
        1 => {
            let center = required_tile(scope, args.get(1), "center")
                .map_err(|error| area_error(scope, error))?;
            let radius = required_number(scope, args.get(2), "radius")
                .map_err(|error| area_error(scope, error))?;
            area_value(scope, Area::circular(center, radius))
                .map_err(|error| area_error(scope, error))
        }
        2 => {
            let area =
                required_area(scope, args.get(1)).map_err(|error| area_error(scope, error))?;
            let Some(tile) = contains_tile(scope, args.get(2), "tile")
                .map_err(|error| area_error(scope, error))?
            else {
                return Ok(v8::Boolean::new(scope, false).into());
            };
            let live_center = if matches!(area, Area::Circular { .. }) {
                Some(
                    required_tile(scope, args.get(3), "center")
                        .map_err(|error| area_error(scope, error))?,
                )
            } else {
                None
            };
            Ok(v8::Boolean::new(scope, area.contains(tile, live_center)).into())
        }
        3 => {
            let area =
                required_area(scope, args.get(1)).map_err(|error| area_error(scope, error))?;
            let live_center = if matches!(area, Area::Circular { .. }) {
                Some(
                    required_tile(scope, args.get(2), "center")
                        .map_err(|error| area_error(scope, error))?,
                )
            } else {
                None
            };
            tile_value(scope, args.get(3), area.random_tile(live_center))
        }
        _ => Err(area_error(scope, "invalid area operation".to_string())),
    }
}

fn area_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    area: Area,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let value = v8::Object::new(scope);
    match area {
        Area::Rectangular {
            min_x,
            max_x,
            min_z,
            max_z,
            level,
        } => {
            set_number_field(scope, value, "kind", RECTANGULAR)?;
            set_number_field(scope, value, "minX", min_x)?;
            set_number_field(scope, value, "maxX", max_x)?;
            set_number_field(scope, value, "minZ", min_z)?;
            set_number_field(scope, value, "maxZ", max_z)?;
            set_number_field(scope, value, "level", level)?;
        }
        Area::Circular { center, radius } => {
            set_number_field(scope, value, "kind", CIRCULAR)?;
            set_number_field(scope, value, "centerX", center.x)?;
            set_number_field(scope, value, "centerZ", center.z)?;
            set_number_field(scope, value, "level", center.level)?;
            set_number_field(scope, value, "radius", radius)?;
        }
    }
    Ok(value.into())
}

fn required_area(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<Area, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Err("invalid area: expected an Area value".to_string());
    }
    let kind = required_number_field(scope, value, "area", "kind")?;
    if kind == RECTANGULAR {
        Ok(Area::Rectangular {
            min_x: required_number_field(scope, value, "area", "minX")?,
            max_x: required_number_field(scope, value, "area", "maxX")?,
            min_z: required_number_field(scope, value, "area", "minZ")?,
            max_z: required_number_field(scope, value, "area", "maxZ")?,
            level: required_number_field(scope, value, "area", "level")?,
        })
    } else if kind == CIRCULAR {
        Ok(Area::Circular {
            center: Tile {
                x: required_number_field(scope, value, "area", "centerX")?,
                z: required_number_field(scope, value, "area", "centerZ")?,
                level: required_number_field(scope, value, "area", "level")?,
            },
            radius: required_number_field(scope, value, "area", "radius")?,
        })
    } else {
        Err("invalid area: unknown Area kind".to_string())
    }
}

fn required_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
) -> Result<Tile, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Err(format!("invalid area: {side} must be a Tile-like object"));
    }
    Ok(Tile {
        x: required_number_field(scope, value, side, "x")?,
        z: required_number_field(scope, value, side, "z")?,
        level: required_number_field(scope, value, side, "level")?,
    })
}
fn contains_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
) -> Result<Option<Tile>, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Err(format!("invalid area: {side} must be a Tile-like object"));
    }
    let Some(level_value) = optional_field(scope, value, "level")? else {
        return Ok(None);
    };
    if !level_value.is_number() {
        return Ok(None);
    }
    let Some(level) = level_value.number_value(scope) else {
        return Ok(None);
    };
    Ok(Some(Tile {
        x: required_number_field(scope, value, side, "x")?,
        z: required_number_field(scope, value, side, "z")?,
        level,
    }))
}

fn optional_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<v8::Value>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, String> {
    let obj = value
        .to_object(scope)
        .ok_or_else(|| "invalid area: not an object".to_string())?;
    let key = v8::String::new(scope, name).ok_or_else(|| "area field name".to_string())?;
    Ok(obj.get(scope, key.into()))
}

fn required_number_field(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    side: &str,
    field: &str,
) -> Result<f64, String> {
    let value = optional_field(scope, value, field)?
        .ok_or_else(|| format!("invalid area: {side}.{field} must be a number"))?;
    if !value.is_number() {
        return Err(format!("invalid area: {side}.{field} must be a number"));
    }
    value
        .number_value(scope)
        .ok_or_else(|| format!("invalid area: {side}.{field} must be a number"))
}
fn required_number(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    field: &str,
) -> Result<f64, String> {
    if !value.is_number() {
        return Err(format!("invalid area: {field} must be a number"));
    }
    value
        .number_value(scope)
        .ok_or_else(|| format!("invalid area: {field} must be a number"))
}

fn set_number_field(
    scope: &mut v8::HandleScope,
    object: v8::Local<v8::Object>,
    field: &str,
    value: f64,
) -> Result<(), String> {
    let key = v8::String::new(scope, field).ok_or_else(|| "area field name".to_string())?;
    let number = v8::Number::new(scope, value);
    let set = object
        .set(scope, key.into(), number.into())
        .ok_or_else(|| format!("area field {field} set"))?;
    if !set {
        return Err(format!("area field {field} set"));
    }
    Ok(())
}

fn tile_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    constructor: v8::Local<'s, v8::Value>,
    tile: Tile,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let x = cb::num(scope, tile.x);
    let z = cb::num(scope, tile.z);
    let level = cb::num(scope, tile.level);
    cb::construct(scope, constructor, &[x, z, level], "Tile")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn js_min_and_max_match_math_nan_and_signed_zero_behavior() {
        assert!(js_min(f64::NAN, 1.0).is_nan());
        assert!(js_min(1.0, f64::NAN).is_nan());
        assert!(js_max(f64::NAN, 1.0).is_nan());
        assert!(js_max(1.0, f64::NAN).is_nan());

        assert!(js_min(-0.0, -0.0).is_sign_negative());
        assert!(js_min(-0.0, 0.0).is_sign_negative());
        assert!(js_min(0.0, -0.0).is_sign_negative());
        assert!(!js_min(0.0, 0.0).is_sign_negative());
        assert!(js_max(-0.0, -0.0).is_sign_negative());
        assert!(!js_max(0.0, 0.0).is_sign_negative());
        assert!(!js_max(-0.0, 0.0).is_sign_negative());
        assert!(!js_max(0.0, -0.0).is_sign_negative());
    }

    fn tile(x: f64, z: f64, level: f64) -> Tile {
        Tile { x, z, level }
    }

    #[test]
    fn rectangular_contains_inclusive_edges_and_uses_a_level() {
        let area = Area::rectangular(tile(10.0, 20.0, 1.0), tile(12.0, 18.0, 9.0));
        assert!(area.contains(tile(10.0, 18.0, 1.0), None));
        assert!(area.contains(tile(12.0, 20.0, 1.0), None));
        assert!(!area.contains(tile(9.0, 19.0, 1.0), None));
        assert!(!area.contains(tile(11.0, 21.0, 1.0), None));
        assert!(!area.contains(tile(11.0, 19.0, 9.0), None));
    }

    #[test]
    fn circular_contains_the_radius_edge_only_on_its_level() {
        let area = Area::circular(tile(10.0, 20.0, 2.0), 5.0);
        assert!(area.contains(tile(13.0, 24.0, 2.0), None));
        assert!(!area.contains(tile(13.0, 25.0, 2.0), None));
        assert!(!area.contains(tile(10.0, 20.0, 1.0), None));
    }

    #[test]
    fn rectangular_random_tiles_cover_each_inclusive_coordinate_pair() {
        let area = Area::rectangular(tile(1.0, 8.0, 3.0), tile(2.0, 9.0, 1.0));
        let mut values = [0.0, 0.0, 0.0, 0.9, 0.9, 0.0, 0.9, 0.9].into_iter();
        let mut sample = || values.next().expect("one sample per coordinate");
        let tiles: Vec<_> = (0..4)
            .map(|_| area.random_tile_with(None, &mut sample))
            .collect();
        assert_eq!(
            tiles,
            vec![
                tile(1.0, 8.0, 3.0),
                tile(1.0, 9.0, 3.0),
                tile(2.0, 8.0, 3.0),
                tile(2.0, 9.0, 3.0),
            ]
        );
        assert!(tiles.iter().all(|tile| area.contains(*tile, None)));
    }

    #[test]
    fn radius_zero_circle_random_returns_its_center() {
        let center = tile(27.0, 42.0, 1.0);
        let area = Area::circular(center, 0.0);
        let mut calls = 0;
        let mut sample = || {
            calls += 1;
            0.75
        };
        assert_eq!(area.random_tile_with(None, &mut sample), center);
        assert_eq!(calls, 2);
    }

    #[test]
    fn circle_random_falls_back_to_the_center_after_64_rejections() {
        let center = tile(50.0, 60.0, 3.0);
        let area = Area::circular(center, 1.0);
        let mut calls = 0;
        let mut sample = || {
            calls += 1;
            0.0
        };
        assert_eq!(area.random_tile_with(None, &mut sample), center);
        assert_eq!(calls, RANDOM_ATTEMPTS * 2);
    }

    #[test]
    fn geometry_core_has_fixed_stack_ownership_and_zero_cold_or_warm_heap() {
        assert_eq!(std::mem::size_of::<Tile>(), 24);
        assert_eq!(std::mem::size_of::<Area>(), 48);
        assert!(!std::mem::needs_drop::<Tile>());
        assert!(!std::mem::needs_drop::<Area>());
        let cold = allocation_counter::measure(|| {
            let area = Area::rectangular(tile(10.0, 20.0, 1.0), tile(12.0, 22.0, 1.0));
            let result = area.random_tile_with(None, &mut || 0.5);
            assert!(area.contains(result, None));
            std::hint::black_box((area, result));
        });
        let center = tile(40.0, 50.0, 2.0);
        let area = Area::circular(center, 5.0);
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                let result = area.random_tile_with(Some(center), &mut || 0.5);
                assert!(area.contains(result, Some(center)));
                std::hint::black_box(result);
            }
        });
        eprintln!("world geometry core: Tile=24B Area=48B cold={cold:?} warm_1000={warm:?}");
        assert_eq!(cold.count_total, 0);
        assert_eq!(cold.bytes_current, 0);
        assert_eq!(warm.count_total, 0);
        assert_eq!(warm.bytes_current, 0);
    }
}
