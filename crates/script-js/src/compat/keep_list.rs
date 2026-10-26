//! JavaScript argument coercion for the native combat keep-list options.

use script::keep_list::CombatKeepOptions;
use serde_json::Value;

pub fn from_args(args: &[Value]) -> CombatKeepOptions {
    let Some(object) = args.first().and_then(Value::as_object) else {
        return CombatKeepOptions::default();
    };
    let string = |field| {
        object
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let extra = object
        .get("extra")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    CombatKeepOptions::new(
        string("food"),
        string("style"),
        string("spell"),
        string("ammo"),
        string("weapon"),
        extra,
    )
}

#[cfg(test)]
mod tests {
    use super::from_args;
    use script::keep_list::*;

    #[test]
    fn bound_arguments_accept_supported_strings_and_arrays() {
        let args = [serde_json::json!({
            "food": "Cake",
            "style": "range",
            "spell": "Wind Strike",
            "ammo": "Iron arrow",
            "weapon": "Maple shortbow",
            "extra": ["Coins", 1, "Rope"]
        })];
        assert_eq!(
            from_args(&args),
            CombatKeepOptions::new(
                "Cake".into(),
                "range".into(),
                "Wind Strike".into(),
                "Iron arrow".into(),
                "Maple shortbow".into(),
                vec!["Coins".into(), "Rope".into()],
            )
        );
    }
}
