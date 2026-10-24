use super::*;

#[test]
fn copied_constants_equal_transport_values() {
    // Compare arrays at run time so this remains a cross-module equality test.
    let actual = std::hint::black_box([
        CELLAR_SHIFT,
        SHANTAY_HENGE_LOC_ID,
        AL_KHARID_TOLL_LEFT_LOC_ID,
        AL_KHARID_TOLL_RIGHT_LOC_ID,
        ARDOUGNE_WILDERNESS_LEVER_LOC_ID,
    ]);
    let expected = [
        crate::transport::CELLAR_SHIFT,
        crate::transport::SHANTAY_HENGE_LOC_ID,
        crate::transport::AL_KHARID_TOLL_LEFT_LOC_ID,
        crate::transport::AL_KHARID_TOLL_RIGHT_LOC_ID,
        crate::transport::ARDOUGNE_WILDERNESS_LEVER_LOC_ID,
    ];
    assert_eq!(actual, expected);
}
