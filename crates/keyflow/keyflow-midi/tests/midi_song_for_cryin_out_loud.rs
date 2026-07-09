mod support;

#[test]
fn marker_chords_match_detected_chords() {
    support::assert_marker_chords_match("../keyflow/tests/midi/For Cryin' Out Loud - FINNEAS.mid");
}
