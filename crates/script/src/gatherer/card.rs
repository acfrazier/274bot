use super::runner::Gatherer;
use super::settings::{resolve_methods_for_prepare, schema, GathererSettings};
use super::supply::PreparedSupply;
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
    description: "Gather selected woodcutting, mining and fishing resources from live observation.",
    category: "Gathering",
    schema_version: 4,
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
    pub resource_error: Option<ConfigError>,
    pub products: Arc<[i32]>,
    pub banks: Arc<api::named_banks::NamedBankFacts>,
    pub supply: PreparedSupply,
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
    let mut settings = GathererSettings::from_bag(&bag).map_err(StartError::Config)?;
    let catalog =
        api::gather_methods::prepare(&cx.selected, cx.families).map_err(StartError::Facts)?;
    let prepared = resolve_methods_for_prepare(&settings, &cx.selected, &catalog)?;
    let method_bits = prepared.bits;
    let methods = prepared.indices;
    let resource_error = prepared.resource_error;
    let supply = PreparedSupply::prepare(&mut settings, &cx.selected, &catalog, &methods)
        .map_err(StartError::Config)?;
    let excluded_targets: Arc<str> = methods
        .iter()
        .flat_map(|&index| api::gather_methods::known_rows(&catalog.methods()[index].targets))
        .filter(|target| !matches!(target.respawn, api::selected::Knowledge::Known(_)))
        .map(|target| format!("{:?}", target.entity))
        .collect::<Vec<_>>()
        .join(", ")
        .into();
    let mut products: Vec<i32> = methods
        .iter()
        .flat_map(|&index| api::gather_methods::known_rows(&catalog.methods()[index].products))
        .map(|product| product.item)
        .collect();
    if settings.skill_kind() == super::settings::Skill::Mining {
        products.extend_from_slice(catalog.incidental_gem_ids());
    }
    products.sort_unstable();
    products.dedup();
    Ok(PreparedConfig::new(
        CARD.id,
        CARD.schema_version,
        revision,
        bag,
        Arc::new(Prepared {
            banks: Arc::clone(&cx.banks),
            supply,
            selected: Arc::clone(&cx.selected),
            catalog,
            settings,
            method_bits,
            methods,
            excluded_targets,
            resource_error,
            products: products.into(),
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
    if let Some(error) = &prepared.resource_error {
        return Err(StartError::Config(error.clone()));
    }
    let retained = *retained.gather();
    Ok(Box::new(Gatherer::new(
        run,
        Arc::clone(&config),
        Arc::clone(prepared),
        retained,
    )))
}
