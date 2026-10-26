use super::*;

#[test]
fn walk_destinations_are_the_12_rs2b0t_towns() {
    let towns: Vec<(&str, i32, i32, i32)> = WALK_DESTINATIONS
        .iter()
        .map(|town| (town.name, town.x, town.z, town.level))
        .collect();
    assert_eq!(
        towns,
        [
            ("Lumbridge", 3221, 3218, 0),
            ("Varrock", 3213, 3424, 0),
            ("Falador", 2965, 3378, 0),
            ("Ardougne", 2661, 3301, 0),
            ("Rellekka", 2668, 3660, 0),
            ("Taverley", 2895, 3435, 0),
            ("Draynor", 3093, 3243, 0),
            ("Al Kharid", 3269, 3167, 0),
            ("Edgeville", 3094, 3493, 0),
            ("Seers' Village", 2725, 3491, 0),
            ("Catherby", 2809, 3441, 0),
            ("Yanille", 2612, 3092, 0),
        ]
    );
}
