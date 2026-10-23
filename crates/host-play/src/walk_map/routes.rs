use std::sync::Mutex;

use api::snapshot::WorldTile;
use nav::contract::RouteView;
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

/// A map projection keeps legacy routes borrowed and compact script routes in
/// their bounded `RouteView`; neither branch expands compact storage.
#[derive(Clone, Copy)]
pub enum RouteProjectionPath<'a> {
    /// Native live/manual route storage.
    Legacy(&'a Route),
    /// Script route storage published by the route owner.
    Compact(RouteView<'a>),
}

impl RouteProjectionPath<'_> {
    /// Destination without exposing or expanding the backing representation.
    pub fn destination(self) -> WorldTile {
        match self {
            Self::Legacy(route) => route.dest,
            Self::Compact(view) => view.destination(),
        }
    }
}
/// Borrow only during the callback. A renderer caches its clipped geometry by
/// (focus lifetime, RouteStamp, view), not a hash/deep clone of every route frame.
#[derive(Clone, Copy)]
pub struct RouteProjection<'a> {
    pub stamp: RouteStamp,
    pub route: RouteProjectionPath<'a>,
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
            route: RouteProjectionPath::Legacy(route),
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
            route: RouteProjectionPath::Legacy(route),
            aim: arm.current_aim(),
        })
    }
    fn script(bot: &'a crate::script_runtime::NavBot) -> Option<Self> {
        let route = bot.route_projection()?;
        Some(Self {
            stamp: RouteStamp {
                source: RouteSource::Script,
                generation: bot.map_route_generation,
                destination: route.destination(),
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
    /// Lock-order contract: slot ticks take `navs` then `WalkArm` in
    /// `NavBot::slot_escape`, so nothing may hold a `WalkArm` while taking
    /// `navs`. Pass the manual mutex without a guard. A supplied live route
    /// returns without locking either owner; otherwise the caller must release
    /// its live/scenario guard before calling. The script read releases `navs`
    /// before the manual fallback locks the arm. The reader must not re-enter
    /// either owner while its route is borrowed.
    pub fn with_map_route<R>(
        &self,
        name: &str,
        manual: Option<&Mutex<WalkArm>>,
        live: Option<RouteProjection<'_>>,
        read: impl FnOnce(Option<RouteProjection<'_>>) -> R,
    ) -> R {
        if let Some(live) = live {
            return read(Some(live));
        }
        {
            let scripts = self.navs.lock().unwrap();
            if let Some(script) = scripts.get(name).and_then(RouteProjection::script) {
                return read(Some(script));
            }
        }
        let manual = manual.map(|arm| arm.lock().unwrap());
        read(manual.as_deref().and_then(RouteProjection::manual))
    }
}
