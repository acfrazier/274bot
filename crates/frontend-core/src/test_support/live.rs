//! Front-end live-harness fixtures (see the parent module).

use std::any::Any;
#[cfg(feature = "js")]
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::live::{FixtureMode, Runner, RunnerHandle, RunnerStatus, Scenario};

impl crate::test_support::FixtureProfile {
    /// Assign a loaded or compiled card to this fixture profile.
    pub fn script_assignment(
        mut self,
        source_kind: &str,
        identity: &str,
        display_name: &str,
    ) -> Self {
        self.profile.settings.script_assignment = Some(vault::ScriptAssignment {
            source_kind: source_kind.into(),
            identity: identity.into(),
            display_name: display_name.into(),
            unavailable: None,
        });
        self
    }
}

/// Resolve a registered live scenario through the front-end core seam.
pub fn scenario(name: &str) -> Option<Scenario> {
    registered_scenario(name).map(Scenario::from)
}

#[cfg(feature = "js")]
/// Install the catalog-backed compiled cards without starting the V8 platform.
pub fn install_catalog_cards() {
    script_js::install_cards().expect("install canonical Sherlock card for front-end fixtures");
}

fn registered_scenario(name: &str) -> Option<scenario::Scenario> {
    #[cfg(feature = "js")]
    install_catalog_cards();
    scenario::get(name)
}

/// Resolve a registered scenario and apply the selected live fixture mode.
pub fn scenario_with_mode(name: &str, mode: FixtureMode) -> Result<Scenario, String> {
    scenario(name)
        .ok_or_else(|| format!("unknown scenario {name}"))?
        .fixture_mode(mode)
}

/// Resolve a scenario with the same test-time seed edits as the live UI.
pub fn scenario_with_profiles_for_test(
    name: &str,
    profile_count: usize,
    mainland: bool,
    additional_profile: (&'static str, &'static str),
) -> Result<Scenario, String> {
    let mut scenario =
        registered_scenario(name).ok_or_else(|| format!("unknown scenario {name}"))?;
    scenario.seed.mainland = mainland;
    scenario
        .seed
        .profiles
        .resize(profile_count, additional_profile);
    Ok(scenario.into())
}

/// Resolve a registered scenario with a test-selected Start script.
pub fn scenario_with_start_script_for_test(
    name: &str,
    start_script: &'static str,
) -> Result<Scenario, String> {
    let mut scenario =
        registered_scenario(name).ok_or_else(|| format!("unknown scenario {name}"))?;
    scenario.settings.start_script = Some(start_script);
    Ok(scenario.into())
}

/// Resolve a registered scenario with test-selected navigation permissions.
pub fn scenario_with_nav_for_test(
    name: &str,
    allow_teleports: bool,
    allow_wilderness: bool,
) -> Result<Scenario, String> {
    let mut scenario =
        registered_scenario(name).ok_or_else(|| format!("unknown scenario {name}"))?;
    scenario.settings.nav.allow_teleports = allow_teleports;
    scenario.settings.nav.allow_wilderness = allow_wilderness;
    Ok(scenario.into())
}

/// Install a scenario runner built by frontend-core on the shared runner.
pub fn install_scenario_for_test(handle: &RunnerHandle, scenario: Scenario, names: &[String]) {
    let mut runner = Runner::with_world(scenario, None);
    runner.set_live_names(names);
    handle.install(runner);
}

/// Build a real one-step runner that reaches PASS after the caller ticks it
/// and observes the run-energy update generation.
pub fn passing_runner_for_test(
    name: &'static str,
    terminal_shot: Option<&'static str>,
    wait_script_stop: Option<&'static str>,
) -> Runner {
    use scenario::{
        Proof, Scenario as AdapterScenario, ScenarioSettings, Seed, Step, StepKind, Wait,
    };

    let proof = Proof::Stat { id: 16, min: 5 };
    let mut runner = scenario::ScenarioRunner::new(AdapterScenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "energy",
            kind: StepKind::Perform {
                send: Box::new(|client, _| {
                    client.runenergy = 5;
                    true
                }),
            },
            wait: Wait {
                arm: proof,
                budget_ticks: 5,
            },
        }],
        proof,
        companions: vec![],
        settings: ScenarioSettings {
            wait_script_stop,
            ..ScenarioSettings::default()
        },
    });
    runner.set_scene_settle(Duration::ZERO);
    if let Some(label) = terminal_shot {
        runner.set_terminal_shot(label);
    }
    Runner::from_scenario_runner(runner)
}

/// Build a real one-step runner that reaches FAIL when the stat proof times
/// out after one tick.
pub fn failing_runner_for_test(name: &'static str, terminal_shot: Option<&'static str>) -> Runner {
    use scenario::{
        Proof, Scenario as AdapterScenario, ScenarioSettings, Seed, Step, StepKind, Wait,
    };

    let proof = Proof::Stat { id: 16, min: 999 };
    let mut runner = scenario::ScenarioRunner::new(AdapterScenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "never",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: proof,
                budget_ticks: 1,
            },
        }],
        proof,
        companions: vec![],
        settings: ScenarioSettings::default(),
    });
    runner.set_scene_settle(Duration::ZERO);
    if let Some(label) = terminal_shot {
        runner.set_terminal_shot(label);
    }
    Runner::from_scenario_runner(runner)
}

/// Tick the adapter runner inside frontend-core without exposing its type to
/// a front-end crate. A mismatched client fixture fails with a clear message.
pub fn tick_scenario_runner_for_test(runner: &mut Runner, client: &mut dyn Any) {
    fn downcast_for<C: 'static>(
        any: &mut dyn Any,
        _tick: fn(&mut scenario::ScenarioRunner, &mut C),
    ) -> &mut C {
        any.downcast_mut()
            .expect("scenario runner fixture received a different client type")
    }

    let client = downcast_for(client, scenario::ScenarioRunner::tick);
    runner.scenario_runner_mut().tick(client);
}

/// Read the provider-neutral status of the installed live runner.
pub fn runner_status_for_test(handle: &RunnerHandle) -> Option<RunnerStatus> {
    handle.status()
}

/// Create a catalog witness configured for one slot.
pub fn catalog_watch_for_test(
    case: host_play::catalog_core::CoreCase,
    username: &str,
) -> host_play::catalog_core::CoreWatch {
    let watch = host_play::catalog_core::CoreWatch::default();
    watch.configure(case, username);
    watch
}

/// Create a paired witness configured for two slots.
pub fn pair_watch_for_test(
    case: host_play::paired_core::PairCase,
    first: &str,
    second: &str,
) -> host_play::paired_core::PairWatch {
    let watch = host_play::paired_core::PairWatch::default();
    watch.configure(case, first, second);
    watch
}

#[cfg(feature = "js")]
/// Create an external-loader witness configured for one source file.
pub fn external_watch_for_test(
    account: &str,
    path: impl AsRef<Path>,
    source_sha256: impl Into<String>,
) -> host_play::external_loader::ExternalWatch {
    let watch = host_play::external_loader::ExternalWatch::default();
    watch.configure(account, PathBuf::from(path.as_ref()), source_sha256.into());
    watch
}
/// Read the normalized compiled-card identity from a pending Start.
pub fn pending_compiled_id(
    start: &host_play::live_start::PendingCatalogStart,
) -> Option<crate::scripts::CompiledCardId> {
    start
        .compiled
        .map(crate::scripts::CompiledCardId::from_script)
}

/// Read the scenario adapter's optional live-budget override for tests.
pub fn budget_s_from_env_for_test() -> Option<Duration> {
    scenario::budget_s_from_env()
}

/// Apply the same safe-label normalization used by whole-window shot writes.
pub fn safe_shot_label(label: &str) -> String {
    scenario::shot::safe_label(label)
}

/// The paired settings bag `case` gives seat `slot_index` (`me` beside
/// `partner`) when the card declares no schema.
pub fn pair_settings_without_schema(
    case: host_play::paired_core::PairCase,
    slot_index: usize,
    me: &str,
    partner: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    host_play::paired_core::pair_settings(case, &[], slot_index, me, partner)
}
