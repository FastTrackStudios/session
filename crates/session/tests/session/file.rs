//! One file format for a session, whoever saves it.

use loro::ExportMode;
use session::doc::SessionDoc;
use session::file::SessionFile;
use session::{ExtensionId, Time};

#[tokio::test(flavor = "current_thread")]
async fn a_session_file_keeps_what_was_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Drive To LA.session");
    let id = {
        let file = SessionFile::create(&path).await.unwrap();
        file.session()
            .set_bar_one(Some(Time::Seconds(6.0)))
            .unwrap();
        file.session()
            .ext(&ExtensionId::from("keyflow"))
            .unwrap()
            .set_text("chart", "Verse: C G Am F\n")
            .unwrap();
        file.save().await.unwrap();
        file.id()
    };
    let file = SessionFile::open(&path).await.unwrap();
    assert_eq!(file.id(), id);
    assert_eq!(file.session().bar_one(), Some(Time::Seconds(6.0)));
    let keyflow = file.session().read_ext(&ExtensionId::from("keyflow"));
    assert_eq!(keyflow.text("chart"), Some("Verse: C G Am F\n"));
}

/// A session saved as a bare Loro snapshot (the DAW engine's file before
/// the format) opens as a session file, brought into the format in place.
#[tokio::test(flavor = "current_thread")]
async fn an_engines_bare_snapshot_opens_as_a_session_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drive-to-la.session");
    let engine = SessionDoc::new();
    engine
        .ext(&ExtensionId::from("daw"))
        .unwrap()
        .set_text("notes", "38 tracks")
        .unwrap();
    engine.doc().commit();
    std::fs::write(&path, engine.doc().export(ExportMode::Snapshot).unwrap()).unwrap();

    // Opened as a document file it is not one.
    assert!(SessionFile::open(&path).await.is_err());
    let file = SessionFile::open_or_import(&path).await.unwrap();
    let daw = file.session().read_ext(&ExtensionId::from("daw"));
    assert_eq!(daw.text("notes"), Some("38 tracks"));
    // The snapshot is kept beside it, and the file is a session file now.
    let mut kept = path.clone().into_os_string();
    kept.push(".snapshot");
    assert!(std::path::Path::new(&kept).exists());
    drop(file);
    let again = SessionFile::open(&path).await.unwrap();
    assert_eq!(
        again
            .session()
            .read_ext(&ExtensionId::from("daw"))
            .text("notes"),
        Some("38 tracks")
    );
}
