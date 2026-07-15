//! `.musx` container handling and `.mxl` packaging.
//!
//! A `.musx` file is a ZIP archive containing (among other things) an encrypted
//! `score.dat` (the gzip-compressed EnigmaXML score, XOR-obfuscated with the
//! MUSX stream cipher) and a plaintext `NotationMetadata.xml`.

use std::io::{Cursor, Read, Write};

use flate2::read::GzDecoder;
use zip::write::SimpleFileOptions;
use zip::ZipArchive;

use crate::cipher::decrypt;
use crate::Error;

/// The decoded pieces of a `.musx` archive.
pub struct MusxArchive {
    /// The decrypted, decompressed EnigmaXML score (`score.dat`).
    pub enigmaxml: Vec<u8>,
    /// The raw `NotationMetadata.xml` bytes (may be latin1-encoded).
    pub metadata: Vec<u8>,
}

fn read_zip_entry(archive_bytes: &[u8], name: &str) -> Result<Vec<u8>, Error> {
    let mut zip = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|e| Error::Container(format!("not a valid musx (zip) file: {e}")))?;
    let mut file = zip
        .by_name(name)
        .map_err(|_| Error::Container(format!("{name} not found in the archive")))?;
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf)
        .map_err(|e| Error::Container(format!("failed reading {name}: {e}")))?;
    Ok(buf)
}

/// Read `score.dat` + `NotationMetadata.xml` from a `.musx` archive, decrypt and
/// decompress the score to EnigmaXML. Port of `convert_file`'s decode prelude.
pub fn decode_musx(archive_bytes: &[u8]) -> Result<MusxArchive, Error> {
    let mut data = read_zip_entry(archive_bytes, "score.dat")?;
    let metadata = read_zip_entry(archive_bytes, "NotationMetadata.xml")?;

    decrypt(&mut data);

    let mut decoder = GzDecoder::new(Cursor::new(data));
    let mut enigmaxml = Vec::new();
    decoder
        .read_to_end(&mut enigmaxml)
        .map_err(|e| Error::Container(format!("failed to gunzip score.dat: {e}")))?;

    Ok(MusxArchive {
        enigmaxml,
        metadata,
    })
}

/// Package a MusicXML document into a compressed `.mxl` archive.
///
/// Port of `save_as_mxl`: a ZIP with an uncompressed `mimetype` entry first,
/// the score, and `META-INF/container.xml`.
pub fn save_as_mxl(musicxml: &[u8], musicxml_filename: &str) -> Result<Vec<u8>, Error> {
    let container = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<container>\n  <rootfiles>\n    <rootfile full-path=\"{musicxml_filename}\" \
media-type=\"application/vnd.recordare.musicxml+xml\"/>\n  </rootfiles>\n</container>\n"
    );

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);

        let stored = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored)
            .map_err(|e| Error::Container(e.to_string()))?;
        zip.write_all(b"application/vnd.recordare.musicxml")
            .map_err(|e| Error::Container(e.to_string()))?;

        let deflated = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(musicxml_filename, deflated)
            .map_err(|e| Error::Container(e.to_string()))?;
        zip.write_all(musicxml)
            .map_err(|e| Error::Container(e.to_string()))?;

        zip.start_file("META-INF/container.xml", deflated)
            .map_err(|e| Error::Container(e.to_string()))?;
        zip.write_all(container.as_bytes())
            .map_err(|e| Error::Container(e.to_string()))?;

        zip.finish()
            .map_err(|e| Error::Container(e.to_string()))?;
    }
    Ok(cursor.into_inner())
}
