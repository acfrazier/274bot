    use super::*;

    #[test]
    fn walk_destinations_are_the_12_rs2b0t_towns() {
        assert_eq!(WALK_DESTINATIONS.len(), 12);
        let lumbridge = &WALK_DESTINATIONS[0];
        assert_eq!(lumbridge.name, "Lumbridge");
        assert_eq!((lumbridge.x, lumbridge.z, lumbridge.level), (3221, 3218, 0));
    }
