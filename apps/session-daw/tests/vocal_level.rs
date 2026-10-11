//! A vocal sung 18 dB quieter in the verse than in the chorus, leveled:
//! the verse brought up to sit with the chorus, the four lanes kept with
//! the track for each item, and their sum written as each item's volume.

#![cfg(feature = "native")]

use daw::service::{Items as _, ProjectContext, Takes as _, Tracks};
use daw_proto::automation::Automation as _;
use daw_proto::{EnvelopeLocation, EnvelopeRef, ItemRef, TakeEnvelopeKind, TrackRef};

/// Six lines a second and a half long, half a second apart, of a 220 Hz
/// tone at `db` dBFS — over a noise floor 70 dB under it.
fn sung(path: &std::path::Path, db: f64) {
    let rate = 48_000_u32;
    let amp = 10f64.powf(db / 20.0) * std::f64::consts::SQRT_2;
    let mut seed = 0x9e37_79b9_u32;
    let samples: Vec<i16> = (0..rate * 12)
        .map(|n| {
            let t = f64::from(n) / f64::from(rate);
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (f64::from(seed) / f64::from(u32::MAX) - 0.5) * amp * 0.001;
            let line = (t % 2.0) < 1.5;
            let tone = if line {
                (t * 220.0 * std::f64::consts::TAU).sin() * amp
            } else {
                0.0
            };
            num_traits::ToPrimitive::to_i16(&((tone + noise) * 32_767.0).round()).unwrap_or(0)
        })
        .collect();
    let data = u32::try_from(samples.len() * 2).unwrap_or(0);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("a wav");
}

const SONG: &str = r#"<REAPER_PROJECT 0.1 "7.75/linux-x86_64" 1790015830 0
  TEMPO 120 4 4 0
  SAMPLERATE 48000 0 0
  MARKER 1 0 "VS" 1
  MARKER 1 12 "" 1
  MARKER 2 12 "CH" 1
  MARKER 2 24 "" 1
  <TRACK {B0000000-0000-0000-0000-000000000001}
    NAME Vocals
    ISBUS 1 1
  >
  <TRACK {B0000000-0000-0000-0000-000000000002}
    NAME "Lead Vocal"
    ISBUS 2 -1
    <ITEM
      POSITION 0
      LENGTH 12
      IGUID {C0000000-0000-0000-0000-000000000001}
      <SOURCE WAVE
        FILE "VERSE_WAV"
      >
    >
    <ITEM
      POSITION 12
      LENGTH 12
      IGUID {C0000000-0000-0000-0000-000000000002}
      <SOURCE WAVE
        FILE "CHORUS_WAV"
      >
    >
  >
>
"#;

// r[verify level.vocal]
#[test]
fn a_quiet_verse_comes_up_to_the_chorus() {
    let dir = std::env::temp_dir().join(format!("vocal-level-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let (verse, chorus) = (dir.join("verse.wav"), dir.join("chorus.wav"));
    sung(&verse, -38.0);
    sung(&chorus, -20.0);
    let song = SONG
        .replace("VERSE_WAV", &verse.to_string_lossy())
        .replace("CHORUS_WAV", &chorus.to_string_lossy());
    let daw = daw::standalone::Standalone::new();
    let loaded =
        daw::standalone::project_loader::load_rpp_text(&daw, "song", "/tmp/song.RPP", &song)
            .expect("the song");
    let project = ProjectContext::Project(loaded.project_guid);
    let lead = Tracks::all(&daw, project.clone())
        .into_iter()
        .find(|t| t.name == "Lead Vocal")
        .expect("the lead");

    let knobs = session_daw::vocal_level::Knobs::default();
    let said =
        session_daw::vocal_level::level(&daw, &project, &lead, (&knobs, false)).expect("leveled");
    assert_eq!(said.items, 2);
    let section = |name: &str| {
        said.sections
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name}: {:?}", said.sections))
    };
    let (vs, ch) = (section("VS"), section("CH"));
    assert!(ch.before_db - vs.before_db > 15.0, "{said:?}");
    assert!(
        (ch.after_db - vs.after_db).abs() < 3.0,
        "the verse with the chorus: {vs:?} {ch:?}"
    );
    // The makeup: finished where the chorus was sung.
    assert!((ch.after_db - ch.before_db).abs() < 3.0, "{ch:?}");

    for item in daw.get_items(project.clone(), TrackRef::Guid(lead.guid.clone())) {
        let take = daw
            .get_active_take(project.clone(), ItemRef::Guid(item.guid.clone()))
            .expect("a take");
        let points = daw.points(
            project.clone(),
            EnvelopeLocation {
                track: TrackRef::Index(0),
                envelope: EnvelopeRef::Take {
                    item_guid: item.guid.clone(),
                    take_guid: take.guid,
                    kind: TakeEnvelopeKind::Volume,
                },
            },
        );
        assert!(
            points.len() > 2,
            "{}: its volume is the lanes' sum",
            item.guid
        );
        let lanes = session_daw::vocal_level::kept(&daw, &project, &lead.guid, &item.guid)
            .expect("its four lanes kept");
        assert!(!lanes.ride.is_empty() && !lanes.gate.is_empty());
    }
}
