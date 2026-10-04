use super::runner::Gatherer;
use super::settings::{resolve_methods_for_prepare, schema, GathererSettings};
use super::supply::PreparedSupply;
use crate::native::{
    CompiledCard, ConfigError, PrepareContext, PreparedConfig, RetainedMemory, SettingsBag,
    StartError,
};
use crate::CompiledId;
use api::game_data::{GatherSiteOption, SelectedGameData};
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
    site_index: Option<usize>,
    pub site_error: Option<ConfigError>,
    pub products: Arc<[i32]>,
    pub banks: Arc<api::named_banks::NamedBankFacts>,
    pub supply: PreparedSupply,
}

impl Prepared {
    pub fn site(&self) -> Option<&GatherSiteOption> {
        self.site_index
            .map(|index| &self.selected.gather_sites()[index])
    }
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
    let mut settings = GathererSettings::from_bag_for_prepare(&bag).map_err(StartError::Config)?;
    let catalog =
        api::gather_methods::prepare(&cx.selected, cx.families).map_err(StartError::Facts)?;
    let prepared = resolve_methods_for_prepare(&settings, &cx.selected, &catalog)?;
    let method_bits = prepared.bits;
    let methods = prepared.indices;
    let resource_error = prepared.resource_error;
    let (site_index, site_error) = resolve_site(&settings, &cx.selected, &catalog, &methods);
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
            site_index,
            site_error,
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
    if let Some(error) = &prepared.site_error {
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

fn resolve_site(
    settings: &GathererSettings,
    selected: &SelectedGameData,
    catalog: &api::gather_methods::GatherCatalog,
    methods: &[usize],
) -> (Option<usize>, Option<ConfigError>) {
    if !settings.location.eq_ignore_ascii_case("site") {
        return (None, None);
    }
    let id = settings.site.trim();
    if id.is_empty() {
        return (
            None,
            Some(ConfigError::new(
                "site",
                "required",
                "Site location needs a named site",
            )),
        );
    }
    let skill = settings.skill_kind().stat_name();
    let error = |message| {
        (
            None,
            Some(ConfigError::new("site", "invalid-site", message)),
        )
    };
    let Some((index, site)) =
        selected.gather_sites().iter().enumerate().find(|(_, row)| {
            row.skill.eq_ignore_ascii_case(skill) && row.id.eq_ignore_ascii_case(id)
        })
    else {
        return error(
            if selected
                .gather_sites()
                .iter()
                .any(|row| row.id.eq_ignore_ascii_case(id))
            {
                format!("{id} is not a {skill} site")
            } else {
                format!("unknown {id}")
            },
        );
    };
    if !site.keys.iter().any(|key| {
        selected.gather_option(skill, &key.key).is_some_and(|row| {
            methods.iter().any(|&index| {
                row.methods
                    .iter()
                    .any(|id| id == catalog.methods()[index].id.0.as_ref())
            })
        })
    }) {
        return error(format!("{id} is not for the selected resources"));
    }
    (Some(index), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prepared_site(
        skill: &str,
        resource: &str,
        site: &str,
        location: &str,
    ) -> Arc<PreparedConfig> {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut bag = SettingsBag::new();
        bag.insert("skill".into(), json!(skill));
        bag.insert("location".into(), json!(location));
        bag.insert("site".into(), json!(site));
        bag.insert("radius".into(), json!(20));
        bag.insert(
            match skill {
                "Mining" => "miningResources",
                "Fishing" => "fishingMethod",
                _ => "woodcuttingResources",
            }
            .into(),
            if skill == "Fishing" {
                json!(resource)
            } else {
                json!([resource])
            },
        );
        api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                CARD.id,
                1,
                Arc::new(bag),
                selected,
                Arc::default(),
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap()
    }

    #[test]
    fn site_errors_are_deferred_to_start_not_unrelated_settings_edits() {
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        for (site, message) in [
            ("", "Site location needs a named site"),
            ("gone", "unknown gone"),
            ("fishing.catherby", "fishing.catherby is not a mining site"),
            (
                "mining.varrock_east.se",
                "mining.varrock_east.se is not for the selected resources",
            ),
        ] {
            let config = prepared_site("Mining", "runite", site, "Site");
            let prepared = config.get::<Arc<Prepared>>().unwrap();
            assert_eq!(prepared.settings.radius, 20);
            assert_eq!(prepared.settings.site, site);
            assert!(prepared.site().is_none());
            assert_eq!(
                prepared.site_error.as_ref().unwrap().message.as_ref(),
                message
            );
            assert_eq!(
                prepared.site_error.as_ref().unwrap().code.as_ref(),
                if site.is_empty() {
                    "required"
                } else {
                    "invalid-site"
                }
            );
            assert!(matches!(
                create(run, config, &mut RetainedMemory::default()),
                Err(StartError::Config(error)) if error.field.as_ref() == "site"
            ));
        }
        for location in ["Start", "Auto"] {
            let config = prepared_site("Mining", "copper", "gone", location);
            let prepared = config.get::<Arc<Prepared>>().unwrap();
            assert!(prepared.site().is_none());
            assert!(prepared.site_error.is_none());
        }
    }

    #[test]
    fn site_prepare_accepts_group_key_and_saved_fishing_alias_without_rewriting() {
        for resource in [
            "fishing.harpoon.tool_311.products_359_371",
            "fishing.rarefish.op3",
        ] {
            let config = prepared_site("Fishing", resource, "fishing.catherby", "Site");
            let prepared = config.get::<Arc<Prepared>>().unwrap();
            assert!(prepared.site_error.is_none());
            assert_eq!(prepared.site().unwrap().id, "fishing.catherby");
            assert_eq!(prepared.settings.fishing_method, resource);
        }
    }
}
