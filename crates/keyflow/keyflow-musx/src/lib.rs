// Lint debt: workspace flipped dead_code/unused to warn (task cleanup);
// this crate predates that — burn down separately.
#![allow(dead_code, unused)]

//! Finale `.musx` → MusicXML converter.
//!
//! A Rust port of [joris-vaneyghen/musx2mxl](https://github.com/joris-vaneyghen/musx2mxl)
//! (MIT). A `.musx` file is a ZIP archive whose `score.dat` is a gzip-compressed
//! [EnigmaXML] score, XOR-obfuscated with a small PRNG stream cipher. This crate
//! decodes that container and translates the EnigmaXML into MusicXML 4.0 that the
//! rest of the pipeline (e.g. [`keyflow-musicxml`](../keyflow_musicxml)) can import.
//!
//! ```no_run
//! let musx = std::fs::read("score.musx").unwrap();
//! let musicxml = keyflow_musx::musx_to_musicxml(&musx).unwrap();
//! // …or a compressed .mxl:
//! let mxl = keyflow_musx::musx_to_mxl(&musx).unwrap();
//! ```
//!
//! [EnigmaXML]: https://github.com/finale-lua/musx-document-model

mod cipher;
mod container;
mod converter;
mod helper;
mod xml_in;
mod xml_out;

pub use container::save_as_mxl;

/// Errors produced while decoding a `.musx` or converting EnigmaXML.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `.musx` container could not be read, decrypted, or decompressed.
    #[error("musx container error: {0}")]
    Container(String),
    /// An XML document failed to parse.
    #[error("xml parse error: {0}")]
    Xml(String),
}

/// The default entry name used for the score inside an `.mxl` archive.
pub const MXL_SCORE_NAME: &str = "score.musicxml";

/// Decode a `.musx` archive to its raw EnigmaXML score bytes.
pub fn musx_to_enigmaxml(musx_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(container::decode_musx(musx_bytes)?.enigmaxml)
}

/// Decode a `.musx` archive and return `(enigmaxml, notation_metadata)`.
pub fn musx_parts(musx_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let parts = container::decode_musx(musx_bytes)?;
    Ok((parts.enigmaxml, parts.metadata))
}

/// Convert EnigmaXML (optionally with `NotationMetadata.xml`) to a MusicXML
/// document string.
pub fn enigmaxml_to_musicxml(enigmaxml: &[u8], metadata: Option<&[u8]>) -> Result<String, Error> {
    let xml = decode_xml(enigmaxml);
    let doc = roxmltree::Document::parse(&xml).map_err(|e| Error::Xml(e.to_string()))?;

    let meta_string = metadata.map(decode_xml);
    let meta_doc = match &meta_string {
        Some(s) => Some(roxmltree::Document::parse(s).map_err(|e| Error::Xml(e.to_string()))?),
        None => None,
    };
    let meta_root = meta_doc.as_ref().map(|d| d.root_element());

    Ok(converter::convert(doc.root_element(), meta_root))
}

/// Convert a `.musx` archive directly to a MusicXML document string.
pub fn musx_to_musicxml(musx_bytes: &[u8]) -> Result<String, Error> {
    let (enigmaxml, metadata) = musx_parts(musx_bytes)?;
    enigmaxml_to_musicxml(&enigmaxml, Some(&metadata))
}

/// Convert a `.musx` archive directly to a compressed `.mxl` archive.
pub fn musx_to_mxl(musx_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let musicxml = musx_to_musicxml(musx_bytes)?;
    save_as_mxl(musicxml.as_bytes(), MXL_SCORE_NAME)
}

/// Decode XML bytes to a `String`, falling back to Latin-1 when the bytes are
/// not valid UTF-8 (mirrors the converter's `latin1` metadata fallback).
fn decode_xml(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cipher_is_symmetric() {
        let original = b"The quick brown fox jumps over 0x2800 lazy dogs.".to_vec();
        let mut buf = original.clone();
        cipher::decrypt(&mut buf);
        assert_ne!(buf, original, "cipher should transform the buffer");
        cipher::decrypt(&mut buf);
        assert_eq!(
            buf, original,
            "applying the cipher twice restores the input"
        );
    }

    #[test]
    fn mxl_is_a_zip_with_mimetype_first() {
        let mxl = save_as_mxl(b"<score-partwise/>", MXL_SCORE_NAME).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(mxl)).unwrap();
        assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
        assert!(archive.by_name(MXL_SCORE_NAME).is_ok());
        assert!(archive.by_name("META-INF/container.xml").is_ok());
    }

    #[test]
    fn converts_minimal_enigmaxml() {
        // A single-staff, single-measure C-major whole note.
        let enigma = r#"<?xml version="1.0" encoding="UTF-8"?>
<finale xmlns="http://www.makemusic.com/2012/finale">
  <others>
    <staffSpec cmper="1"><instUuid>x</instUuid></staffSpec>
    <measSpec cmper="1"><beats>4</beats><divbeat>1024</divbeat></measSpec>
    <frameSpec cmper="10"><startEntry>100</startEntry><endEntry>100</endEntry></frameSpec>
  </others>
  <details>
    <gfhold cmper1="1" cmper2="1"><clefID>0</clefID><frame1>10</frame1></gfhold>
  </details>
  <entries>
    <entry entnum="100" next="0"><dura>4096</dura><isNote/><numNotes>1</numNotes>
      <note id="1"><harmLev>0</harmLev><harmAlt>0</harmAlt></note>
    </entry>
  </entries>
</finale>"#;
        let musicxml = enigmaxml_to_musicxml(enigma.as_bytes(), None).unwrap();
        assert!(musicxml.contains("<score-partwise version=\"4.0\">"));
        assert!(musicxml.contains("<part id=\"P1\">"));
        assert!(musicxml.contains("<step>C</step>"));
        assert!(musicxml.contains("<octave>4</octave>"));
        assert!(musicxml.contains("<type>whole</type>"));
    }
}
