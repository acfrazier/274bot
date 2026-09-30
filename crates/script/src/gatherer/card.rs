use super::runner::Gatherer;
use super::settings::{resolve_methods, schema, GathererSettings};
use crate::native::{
    CompiledCard, ConfigError, PrepareContext, PreparedConfig, RetainedMemory, SettingsBag,
    StartError,
};
use crate::CompiledId;
use api::game_data::SelectedGameData;
use api::selected::RunKey;
use std::sync::Arc;

pub const CARD: CompiledCard = CompiledCard {
    id: CompiledId("Gatherer"),
    name: "Gatherer",
    description: "Gather selected woodcutting and mining resources from live observation.",
    category: "Gathering",
    schema_version: 1,
    schema,
    per_account_settings: &[],
    prepare,
    create,
};

pub struct Prepared {
    pub selected: Arc<SelectedGameData>,
    pub catalog: Arc<api::gather_methods::GatherCatalog>,
    pub settings: GathererSettings,
    pub method_bits: u64,
    pub methods: Arc<[usize]>,
    pub excluded_targets: Arc<str>,
}

pub fn prepare(
    cx: &mut PrepareContext<'_>,
    revision: u64,
    bag: Arc<SettingsBag>,
) -> Result<Arc<PreparedConfig>, StartError> {
    if cx
        .selected
        .selected_pin()
        .map_err(StartError::Facts)?
        .as_ref()
        != cx.pin.as_ref()
    {
        return Err(StartError::Facts(api::selected::FactError::PinMismatch));
    }
    let settings = GathererSettings::from_bag(&bag).map_err(StartError::Config)?;
    let catalog =
        api::gather_methods::prepare(&cx.selected, cx.families).map_err(StartError::Facts)?;
    let (method_bits, methods) = resolve_methods(&settings, &cx.selected, &catalog)?;
    let excluded_targets: Arc<str> = methods
        .iter()
        .flat_map(|&index| api::gather_methods::known_rows(&catalog.methods()[index].targets))
        .filter(|target| !matches!(target.respawn, api::selected::Knowledge::Known(_)))
        .map(|target| format!("{:?}", target.entity))
        .collect::<Vec<_>>()
        .join(", ")
        .into();
    Ok(PreparedConfig::new(
        CARD.id,
        CARD.schema_version,
        revision,
        bag,
        Arc::new(Prepared {
            selected: Arc::clone(&cx.selected),
            catalog,
            settings,
            method_bits,
            methods,
            excluded_targets,
        }),
    ))
}

pub fn create(
    run: RunKey,
    config: Arc<PreparedConfig>,
    retained: &mut RetainedMemory,
) -> Result<Box<dyn crate::native::Script>, StartError> {
    let prepared = config.get::<Arc<Prepared>>().ok_or_else(|| {
        StartError::Config(ConfigError::new("", "config-identity", "not Gatherer"))
    })?;
    let retained = *retained.gather();
    Ok(Box::new(Gatherer::new(
        run,
        Arc::clone(&config),
        Arc::clone(prepared),
        retained,
    )))
}
