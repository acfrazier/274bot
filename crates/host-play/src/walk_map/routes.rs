use api::snapshot::WorldTile;
use nav::router::Route;

use crate::{Play, ScriptNavPaint, WalkArm};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Manual,
    Script,
    Live,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteStamp {
    pub source: RouteSource,
    pub generation: u64,
    pub destination: WorldTile,
}
/// Borrow only during the callback. A renderer caches its clipped geometry by
/// (focus lifetime, RouteStamp, view), not a hash/deep clone of every route frame.
#[derive(Clone, Copy)]
pub struct RouteProjection<'a> {
    pub stamp: RouteStamp,
    pub route: &'a Route,
    pub aim: Option<WorldTile>,
}
impl<'a> RouteProjection<'a> {
    pub fn live(route: &'a Route, generation: u64, aim: Option<WorldTile>) -> Self {
        Self {
            stamp: RouteStamp {
                source: RouteSource::Live,
                generation,
                destination: route.dest,
            },
            route,
            aim,
        }
    }
    pub fn manual(arm: &'a WalkArm) -> Option<Self> {
        arm.route.as_ref().map(|route| Self {
            stamp: RouteStamp {
                source: RouteSource::Manual,
                generation: arm.route_generation,
                destination: route.dest,
            },
            route,
            aim: arm.traveller.current_aim(),
        })
    }
}
impl ScriptNavPaint {
    pub fn with_map_route<R>(
        &self,
        name: &str,
        read: impl FnOnce(Option<RouteProjection<'_>>) -> R,
    ) -> R {
        let all = self.navs.lock().unwrap();
        let projection = all.get(name).and_then(|bot| {
            bot.route.as_ref().map(|route| RouteProjection {
                stamp: RouteStamp {
                    source: RouteSource::Script,
                    generation: bot.map_route_generation,
                    destination: route.dest,
                },
                route,
                aim: bot.traveller.current_aim(),
            })
        });
        read(projection)
    }
}
impl Play {
    /// Same precedence as the active follow owners: a driven live scenario,
    /// then a manual arm, then the script route. Caller supplies live only for
    /// this exact focused slot/world, never another bot's route/session.
    pub fn with_map_route<R>(
        &self,
        name: &str,
        manual: Option<&WalkArm>,
        live: Option<RouteProjection<'_>>,
        read: impl FnOnce(Option<RouteProjection<'_>>) -> R,
    ) -> R {
        if let Some(projection) = live.or_else(|| manual.and_then(RouteProjection::manual)) {
            return read(Some(projection));
        }
        let all = self.navs.lock().unwrap();
        read(all.get(name).and_then(|bot| {
            bot.route.as_ref().map(|route| RouteProjection {
                stamp: RouteStamp {
                    source: RouteSource::Script,
                    generation: bot.map_route_generation,
                    destination: route.dest,
                },
                route,
                aim: bot.traveller.current_aim(),
            })
        }))
    }
}
