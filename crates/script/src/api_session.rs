//! Native Gatherer session families for JS API v2.
//!
//! The `GatherSession` row owns the isolate-local admission record. Its Drop
//! clears Busy on every terminal and abort path, including ordinary Done.
use crate::api_gather::GatherEnd;
use crate::api_progress::ProgressPage;
use crate::gatherer::GathererSettings;
use crate::machine::{Begin, Cx, Family, Step};
use crate::native::SettingsBag;
use crate::observed;
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::Value;
use std::cell::RefCell;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const MAX_SAFE_TOKEN: u64 = (1 << 53) - 1;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Stop intent must outlive the sync `gather.stop()` call and follow the
    /// live GatherSession row across reconnect scene epochs.
    static GATHER: RefCell<Option<GatherRecord>> = const { RefCell::new(None) };
}

struct GatherRecord {
    token: NonZeroU64,
    stop_requested: bool,
    stop_epoch: u64,
}

/// Allocate a nonzero integer exactly representable by a JavaScript Number.
/// The counter advances past the public range once, then remains exhausted;
/// it never wraps or reuses a token.
fn allocate_token(next: &AtomicU64) -> Option<u64> {
    next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |candidate| {
        if candidate != 0 && candidate <= MAX_SAFE_TOKEN {
            Some(candidate + 1)
        } else {
            None
        }
    })
    .ok()
}

fn settings_error(settings: &SettingsBag) -> Option<String> {
    GathererSettings::from_bag(settings)
        .err()
        .map(crate::api_gather::settings_refusal)
}

/// One awaited Gatherer-card session, correlated with host pages by a public
/// safe-integer token. Acknowledged sessions are not re-started on reconnect;
/// unacknowledged start and Stop rows are carried into each new scene epoch.
pub(crate) struct GatherSession {
    token: u64,
    settings: Arc<SettingsBag>,
    emitted_epoch: u64,
    acknowledged: bool,
}

impl Family for GatherSession {
    const NAME: &'static str = "gather";
    type Args = SettingsBag;
    type Output = GatherEnd;

    fn begin(settings: Self::Args, cx: &mut Cx<'_>) -> Begin<Self> {
        if GATHER.with(|state| state.borrow().is_some()) {
            return Begin::Refuse("busy".into());
        }
        if let Some(reason) = settings_error(&settings) {
            return Begin::Refuse(reason);
        }
        if !crate::isolate_fb::gather_settings_within_limit(&settings) {
            return Begin::Refuse("invalid-settings".into());
        }
        let Some(token) = allocate_token(&NEXT_TOKEN) else {
            return Begin::Refuse("token-exhausted".into());
        };
        let settings = Arc::new(settings);
        let epoch = observed::with(observed::Scene::epoch);
        cx.emit(InteractReq::GatherRun {
            request_id: token,
            settings: Arc::clone(&settings),
        });
        GATHER.with(|state| {
            *state.borrow_mut() = Some(GatherRecord {
                token: NonZeroU64::new(token).expect("nonzero session token"),
                stop_requested: false,
                stop_epoch: epoch,
            });
        });
        Begin::Run(Self {
            token,
            settings,
            emitted_epoch: epoch,
            acknowledged: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let (epoch, outcome, acknowledged) = observed::with(|scene| {
            let since_login = scene.since_login();
            let outcome = since_login
                .api_gather_outcome()
                .filter(|page| page.request_id == self.token)
                .map(|page| page.end.clone());
            let acknowledged = since_login
                .api_gather()
                .is_some_and(|page| page.request_id == self.token);
            (scene.epoch(), outcome, acknowledged)
        });

        // A retained terminal in the reconnect keyframe wins over any
        // unacknowledged-start or lost-Stop carry.
        if let Some(outcome) = outcome {
            return Step::Done(outcome);
        }
        self.acknowledged |= acknowledged;

        if !self.acknowledged && epoch != self.emitted_epoch {
            cx.emit(InteractReq::GatherRun {
                request_id: self.token,
                settings: Arc::clone(&self.settings),
            });
            self.emitted_epoch = epoch;
        }

        let carry_stop = GATHER.with(|state| {
            let mut state = state.borrow_mut();
            let Some(record) = state
                .as_mut()
                .filter(|record| record.token.get() == self.token)
            else {
                return false;
            };
            if record.stop_requested && record.stop_epoch != epoch {
                record.stop_epoch = epoch;
                true
            } else {
                false
            }
        });
        if carry_stop {
            cx.emit(InteractReq::GatherStop {
                request_id: self.token,
            });
        }
        Step::Wait
    }

    /// Supersede/terminate releases the host seat. Reset deliberately emits
    /// no release because the host destroys that session at the boundary.
    fn release(&self) -> Option<InteractReq> {
        Some(InteractReq::GatherStop {
            request_id: self.token,
        })
    }
}

impl Drop for GatherSession {
    fn drop(&mut self) {
        GATHER.with(|state| {
            let mut state = state.borrow_mut();
            if state
                .as_ref()
                .is_some_and(|record| record.token.get() == self.token)
            {
                *state = None;
            }
        });
    }
}

#[derive(Deserialize)]
pub(crate) struct StopArgs {}

/// Synchronous Stop request. Stop intent is retained on the live row so it
/// can be re-emitted after the reconnect drops its original queued row.
pub(crate) struct GatherStop;

impl Family for GatherStop {
    const NAME: &'static str = "gather-stop";
    type Args = StopArgs;
    type Output = Value;

    fn begin(_: Self::Args, cx: &mut Cx<'_>) -> Begin<Self> {
        let epoch = observed::with(observed::Scene::epoch);
        let stopped = GATHER.with(|state| {
            let mut state = state.borrow_mut();
            let Some(record) = state.as_mut() else {
                return Err("no-session");
            };
            if record.stop_requested {
                return Ok(None);
            }
            record.stop_requested = true;
            record.stop_epoch = epoch;
            Ok(Some(record.token.get()))
        });
        match stopped {
            Err(reason) => Begin::Refuse(reason.into()),
            Ok(None) => Begin::Done(Value::Null),
            Ok(Some(token)) => {
                cx.emit(InteractReq::GatherStop { request_id: token });
                Begin::Done(Value::Null)
            }
        }
    }

    fn step(&mut self, _: &mut Cx<'_>) -> Step<Self::Output> {
        Step::Wait
    }
}

/// One awaited read of a released quest Path's resolved progress.
#[derive(Deserialize)]
pub(crate) struct ProgressQueryArgs {
    #[serde(default)]
    quest: String,
}

/// Requests one fresh host-side progress read. The query is exclusive so a
/// newer read supersedes it; the host owns any open journal lifecycle.
pub(crate) struct ProgressQuery {
    token: u64,
    quest: String,
    emitted_epoch: u64,
    acknowledged: bool,
}

impl Family for ProgressQuery {
    const NAME: &'static str = "quest-progress";
    const EXCLUSIVE: bool = true;
    type Args = ProgressQueryArgs;
    type Output = ProgressPage;

    fn begin(args: Self::Args, cx: &mut Cx<'_>) -> Begin<Self> {
        if args.quest.trim().is_empty() {
            return Begin::Refuse("invalid-args".into());
        }
        let Some(token) = allocate_token(&NEXT_TOKEN) else {
            return Begin::Refuse("token-exhausted".into());
        };
        let emitted_epoch = observed::with(observed::Scene::epoch);
        cx.emit(InteractReq::ProgressRead {
            request_id: token,
            name: args.quest.clone(),
        });
        Begin::Run(Self {
            token,
            quest: args.quest,
            emitted_epoch,
            acknowledged: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let (epoch, page) = observed::with(|scene| {
            let page = scene
                .since_login()
                .api_progress()
                .filter(|page| page.token() == self.token)
                .cloned();
            (scene.epoch(), page)
        });

        match page {
            Some(ProgressPage::Reading { .. }) => self.acknowledged = true,
            Some(page @ (ProgressPage::Done { .. } | ProgressPage::Refused { .. })) => {
                return Step::Done(page);
            }
            None => {}
        }

        if !self.acknowledged && epoch != self.emitted_epoch {
            cx.emit(InteractReq::ProgressRead {
                request_id: self.token,
                name: self.quest.clone(),
            });
            self.emitted_epoch = epoch;
        }
        Step::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::{allocate_token, MAX_SAFE_TOKEN};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn gather_tokens_stop_at_the_javascript_safe_integer_boundary() {
        let next = AtomicU64::new(MAX_SAFE_TOKEN - 1);
        assert_eq!(allocate_token(&next), Some(MAX_SAFE_TOKEN - 1));
        assert_eq!(allocate_token(&next), Some(MAX_SAFE_TOKEN));
        assert_eq!(allocate_token(&next), None);
        assert_eq!(next.load(Ordering::Relaxed), MAX_SAFE_TOKEN + 1);

        assert_eq!(allocate_token(&AtomicU64::new(0)), None);
        assert_eq!(allocate_token(&AtomicU64::new(MAX_SAFE_TOKEN + 1)), None);
        assert_eq!(allocate_token(&AtomicU64::new(u64::MAX)), None);
    }

    #[test]
    fn gather_isolate_rust_records_stay_inside_estimated_inline_bounds() {
        let record = std::mem::size_of::<Option<super::GatherRecord>>();
        let row = std::mem::size_of::<super::GatherSession>();
        println!("API gather isolate Rust idle record={record}B, live family row={row}B (settings heap/wire pages measured separately)");
        assert!(record <= 24);
        assert!(row <= 64);
    }
}
