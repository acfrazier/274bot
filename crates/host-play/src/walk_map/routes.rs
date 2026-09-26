use api::snapshot::WorldTile;
use nav::router::Route;

use crate::{Play, ScriptNavPaint, WalkArm};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Manual,
    Script,
    Live,
}

/// One route-owner precedence for map and in-game paint: driven live,
/// then script, then manual WalkTo. Hidden layers never change this choice.
pub fn select_route_source(live: bool, script: bool, manual: bool) -> Option<RouteSource> {
    if live {
        Some(RouteSource::Live)
    } else if script {
        Some(RouteSource::Script)
    } else if manual {
        Some(RouteSource::Manual)
    } else {
        None
    }
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
    fn script(bot: &'a crate::script_runtime::NavBot) -> Option<Self> {
        bot.route.as_ref().map(|route| Self {
            stamp: RouteStamp {
                source: RouteSource::Script,
                generation: bot.map_route_generation,
                destination: route.dest,
            },
            route,
            aim: bot.traveller.current_aim(),
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
        read(all.get(name).and_then(RouteProjection::script))
    }
}
impl Play {
    /// Shared paint precedence: driven live, then script, then manual WalkTo.
    /// Caller supplies live only for this exact focused slot/world, never
    /// another bot's route/session.
    pub fn with_map_route<R>(
        &self,
        name: &str,
        manual: Option<&WalkArm>,
        live: Option<RouteProjection<'_>>,
        read: impl FnOnce(Option<RouteProjection<'_>>) -> R,
    ) -> R {
        // A driven live route needs no script lock or manual projection.
        let scripts = live.is_none().then(|| self.navs.lock().unwrap());
        let script = scripts
            .as_ref()
            .and_then(|all| all.get(name))
            .and_then(RouteProjection::script);
        let source = select_route_source(
            live.is_some(),
            script.is_some(),
            manual.is_some_and(|arm| arm.route.is_some()),
        );
        read(match source {
            Some(RouteSource::Live) => live,
            Some(RouteSource::Script) => script,
            Some(RouteSource::Manual) => manual.and_then(RouteProjection::manual),
            None => None,
        })
    }
}
