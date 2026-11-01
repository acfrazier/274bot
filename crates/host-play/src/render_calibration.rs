//! Explicit, provisional render inputs for hook-free timing executables.
//!
//! Read and apply these inputs before creating a Play session or render head.
//! This module never supplies production defaults or installs test controls.

use std::num::NonZeroUsize;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

static CONFIGURED: OnceLock<Inputs> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
pub enum CalibrationError {
    #[error("{0} is required for M3_FAIR_PROBE=1")]
    Missing(&'static str),
    #[error("{0} must contain Unicode text")]
    NonUnicode(&'static str),
    #[error("M3_FAIR_PROBE must be 0 or 1")]
    InvalidFlag,
    #[error("{0} must be a positive integer")]
    InvalidPositive(&'static str),
    #[error("T44_RENDER_BUDGET_LIMITS_JSON must contain all thirteen positive byte ceilings: {0}")]
    ByteLimits(#[from] serde_json::Error),
    #[error("render timing calibration must not compile render-test-hooks or draw-memory-profile")]
    Instrumented,
    #[error(transparent)]
    Render(#[from] host::render::RenderError),
    #[error("retained-present calibration configuration failed: {0:?}")]
    RetainedPresent(client::render::backend::ReadbackError),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ByteLimits {
    capture: NonZeroUsize,
    nav_vertices: NonZeroUsize,
    nav_quads: NonZeroUsize,
    nav_edges: NonZeroUsize,
    nav_input: NonZeroUsize,
    head_models: NonZeroUsize,
    head_misc: NonZeroUsize,
    temp_models: NonZeroUsize,
    shared_models: NonZeroUsize,
    shared_sprites: NonZeroUsize,
    vertex: NonZeroUsize,
    context_cpu: NonZeroUsize,
    context_gpu: NonZeroUsize,
}

impl ByteLimits {
    fn limits(self) -> client::render::budget::RenderBudgetLimits {
        client::render::budget::RenderBudgetLimits::new(
            self.capture.get(),
            self.nav_vertices.get(),
            self.nav_quads.get(),
            self.nav_edges.get(),
            self.nav_input.get(),
            self.head_models.get(),
            self.head_misc.get(),
            self.temp_models.get(),
            self.shared_models.get(),
            self.shared_sprites.get(),
            self.vertex.get(),
            self.context_cpu.get(),
            self.context_gpu.get(),
        )
    }
}

/// The exact operator-supplied inputs, not qualified production constants.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Inputs {
    grant_ttl_ms: u64,
    scene_quantum_ms: u64,
    retained_present_cap: NonZeroUsize,
    byte_limits: ByteLimits,
}

impl Inputs {
    fn parse(
        mut value: impl FnMut(&'static str) -> Result<Option<String>, CalibrationError>,
    ) -> Result<Option<Self>, CalibrationError> {
        match value("M3_FAIR_PROBE")?.as_deref() {
            None | Some("0") => return Ok(None),
            Some("1") => {}
            Some(_) => return Err(CalibrationError::InvalidFlag),
        }
        let mut required = |key| value(key)?.ok_or(CalibrationError::Missing(key));
        let positive_ms = |key, text: String| {
            text.parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(CalibrationError::InvalidPositive(key))
        };
        let grant_ttl_ms = positive_ms("M3_GRANT_TTL_MS", required("M3_GRANT_TTL_MS")?)?;
        let scene_quantum_ms =
            positive_ms("M3_SCENE_QUANTUM_MS", required("M3_SCENE_QUANTUM_MS")?)?;
        let retained_present_cap = required("T44_RETAINED_PRESENT_CAP")?
            .parse::<NonZeroUsize>()
            .map_err(|_| CalibrationError::InvalidPositive("T44_RETAINED_PRESENT_CAP"))?;
        let byte_limits = serde_json::from_str(&required("T44_RENDER_BUDGET_LIMITS_JSON")?)?;
        Ok(Some(Self {
            grant_ttl_ms,
            scene_quantum_ms,
            retained_present_cap,
            byte_limits,
        }))
    }

    pub const fn grant_ttl_ms(self) -> u64 {
        self.grant_ttl_ms
    }

    pub const fn scene_quantum_ms(self) -> u64 {
        self.scene_quantum_ms
    }
}

/// Exact inputs that the process successfully applied.
pub fn configured_inputs() -> Option<Inputs> {
    CONFIGURED.get().copied()
}

/// Static calibration state, separate from render-resource reservations.
pub const fn static_storage_bytes() -> usize {
    std::mem::size_of::<OnceLock<Inputs>>()
}

/// Configure the real fair renderer before its first use, if explicitly selected.
///
/// Disabled calibration reads no other input and changes no process state.
/// Reconfiguration or selection after process initialization is an error.
pub fn configure_from_env() -> Result<Option<Inputs>, CalibrationError> {
    let inputs = Inputs::parse(|key| match std::env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(CalibrationError::NonUnicode(key)),
    })?;
    let Some(inputs) = inputs else {
        return Ok(None);
    };
    if client::render::backend::render_test_hooks_compiled()
        || client::render::backend::draw_memory_profile_compiled()
    {
        return Err(CalibrationError::Instrumented);
    }
    host::render_head::configure_calibration_limits(inputs.byte_limits.limits())?;
    client::render::backend::configure_retained_present_budget(inputs.retained_present_cap)
        .map_err(CalibrationError::RetainedPresent)?;
    host::render_head::configure_calibration(host::render_head::CalibrationConfig {
        grant_ttl: Duration::from_millis(inputs.grant_ttl_ms),
        scene_quantum: Duration::from_millis(inputs.scene_quantum_ms),
    })?;
    CONFIGURED
        .set(inputs)
        .expect("successful render configuration is one-shot");
    Ok(Some(inputs))
}

#[cfg(test)]
mod tests {
    use super::{CalibrationError, Inputs};
    use std::collections::HashMap;

    #[test]
    #[ignore = "manual boundary proof; requires all thirteen explicit T44_RENDER_BUDGET_LIMITS_JSON ceilings"]
    fn explicit_byte_ceiling_refusals_leave_every_live_owner_unchanged() {
        use client::render::budget::RenderBudget;
        let raw = std::env::var("T44_RENDER_BUDGET_LIMITS_JSON")
            .expect("the refusal proof requires the same explicit ceilings as the live run");
        let limits: super::ByteLimits = serde_json::from_str(&raw).expect("thirteen positive ceilings");
        let budgets = RenderBudget::new(limits.limits());
        for initial in budgets.diagnostic_snapshots() {
            let budget = budgets.for_resource(initial.resource).unwrap();
            let requested = initial.maximum.checked_add(1).expect("ceiling permits a boundary probe");
            let refused = match budget.reserve(requested) {
                Err(limit) => limit,
                Ok(_) => panic!("an oversized reservation was admitted"),
            };
            assert_eq!(refused.resource(), initial.resource);
            assert_eq!(refused.requested(), requested);
            assert_eq!(refused.maximum(), initial.maximum);
            let after = budget.diagnostic_snapshot();
            assert_eq!(after.budget_id, initial.budget_id);
            assert_eq!(after.live, initial.live);
            assert_eq!(after.peak, initial.peak);
            assert_eq!(after.limit_actions, initial.limit_actions + 1);
            assert_eq!(after.maximum_failed_request, requested);
            assert!(after.event_order > initial.event_order);
        }
    }

    fn explicit_values() -> HashMap<&'static str, String> {
        HashMap::from([
            ("M3_FAIR_PROBE", "1".into()),
            ("M3_GRANT_TTL_MS", "7".into()),
            ("M3_SCENE_QUANTUM_MS", "11".into()),
            ("T44_RETAINED_PRESENT_CAP", "13".into()),
            (
                "T44_RENDER_BUDGET_LIMITS_JSON",
                serde_json::json!({
                    "capture": 1, "nav_vertices": 2, "nav_quads": 3,
                    "nav_edges": 4, "nav_input": 5, "head_models": 6,
                    "head_misc": 7, "temp_models": 8, "shared_models": 9,
                    "shared_sprites": 10, "vertex": 11, "context_cpu": 12,
                    "context_gpu": 13,
                })
                .to_string(),
            ),
        ])
    }

    #[test]
    fn disabled_probe_reads_no_calibration_values() {
        for flag in [None, Some("0".to_string())] {
            assert!(Inputs::parse(|key| {
                assert_eq!(key, "M3_FAIR_PROBE");
                Ok(flag.clone())
            })
            .unwrap()
            .is_none());
        }
    }

    #[test]
    fn explicit_inputs_preserve_each_value() {
        let values = explicit_values();
        let inputs = Inputs::parse(|key| Ok(values.get(key).cloned()))
            .unwrap()
            .unwrap();
        assert_eq!(inputs.grant_ttl_ms(), 7);
        assert_eq!(inputs.scene_quantum_ms(), 11);
        let receipt = serde_json::to_value(inputs).unwrap();
        assert_eq!(receipt["retained_present_cap"], 13);
        assert_eq!(receipt["byte_limits"]["context_gpu"], 13);
        assert_eq!(receipt["byte_limits"]["capture"], 1);
    }

    #[test]
    fn enabled_probe_requires_every_input_without_defaults() {
        for key in [
            "M3_GRANT_TTL_MS",
            "M3_SCENE_QUANTUM_MS",
            "T44_RETAINED_PRESENT_CAP",
            "T44_RENDER_BUDGET_LIMITS_JSON",
        ] {
            let mut values = explicit_values();
            values.remove(key);
            assert!(matches!(
                Inputs::parse(|name| Ok(values.get(name).cloned())),
                Err(CalibrationError::Missing(missing)) if missing == key
            ));
        }
        let mut values = explicit_values();
        values.insert("M3_FAIR_PROBE", "yes".into());
        assert!(matches!(
            Inputs::parse(|key| Ok(values.get(key).cloned())),
            Err(CalibrationError::InvalidFlag)
        ));
    }

    #[test]
    fn byte_ceilings_reject_missing_zero_and_unknown_fields() {
        let mut values = explicit_values();
        let original: serde_json::Value =
            serde_json::from_str(&values["T44_RENDER_BUDGET_LIMITS_JSON"]).unwrap();
        for mutation in ["missing", "zero", "unknown"] {
            let mut limits = original.clone();
            match mutation {
                "missing" => {
                    limits.as_object_mut().unwrap().remove("capture");
                }
                "zero" => limits["capture"] = 0.into(),
                "unknown" => limits["extra"] = 1.into(),
                _ => unreachable!(),
            }
            values.insert("T44_RENDER_BUDGET_LIMITS_JSON", limits.to_string());
            assert!(matches!(
                Inputs::parse(|key| Ok(values.get(key).cloned())),
                Err(CalibrationError::ByteLimits(_))
            ));
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn calibration_state_size_stays_process_scoped() {
        assert_eq!(std::mem::size_of::<Inputs>(), 128);
        assert_eq!(super::static_storage_bytes(), 136);
    }
}
