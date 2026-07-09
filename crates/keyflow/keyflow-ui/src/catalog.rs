//! Local MusicXML lead-sheet catalog support.
//!
//! The catalog intentionally reads from ignored reference data, so the app can
//! browse real charts without committing copyrighted or bulky corpus files.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub title: String,
    pub composer: Option<String>,
    pub path: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn local_musicxml_catalog() -> Vec<CatalogEntry> {
    let mut entries = Vec::new();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference-data/wikifonia/xml");
    collect_musicxml_entries(&root, &mut entries);
    entries.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    entries
}

#[cfg(target_arch = "wasm32")]
pub fn local_musicxml_catalog() -> Vec<CatalogEntry> {
    Vec::new()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_musicxml_catalog_chart(path: &str) -> Result<String, String> {
    let xml = read_musicxml_path(std::path::Path::new(path))?;
    musicxml_to_keyflow_chart(&xml)
}

#[cfg(target_arch = "wasm32")]
pub fn load_musicxml_catalog_chart(_path: &str) -> Result<String, String> {
    Err("local MusicXML catalog is not available in wasm builds".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_musicxml_entries(dir: &std::path::Path, entries: &mut Vec<CatalogEntry>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_musicxml_entries(&path, entries);
            continue;
        }

        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if !matches!(
            ext.to_ascii_lowercase().as_str(),
            "xml" | "musicxml" | "mxl"
        ) {
            continue;
        }

        let fallback_title = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .trim()
            .to_string();

        let (title, composer) = read_musicxml_path(&path)
            .ok()
            .and_then(|xml| musicxml_metadata(&xml).ok())
            .unwrap_or((fallback_title, None));

        entries.push(CatalogEntry {
            title,
            composer,
            path: path.to_string_lossy().to_string(),
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_musicxml_path(path: &std::path::Path) -> Result<String, String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("mxl") => read_mxl_score_xml(path),
        _ => std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display())),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_mxl_score_xml(path: &std::path::Path) -> Result<String, String> {
    let listing = std::process::Command::new("unzip")
        .arg("-Z1")
        .arg(path)
        .output()
        .map_err(|err| format!("failed to run unzip for {}: {err}", path.display()))?;
    if !listing.status.success() {
        return Err(format!("failed to list {}", path.display()));
    }

    let member = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(str::trim)
        .find(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".xml") && lower != "meta-inf/container.xml"
        })
        .ok_or_else(|| format!("no score XML found in {}", path.display()))?
        .to_string();

    let output = std::process::Command::new("unzip")
        .arg("-p")
        .arg(path)
        .arg(&member)
        .output()
        .map_err(|err| format!("failed to extract {member} from {}: {err}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "failed to extract {member} from {}",
            path.display()
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|err| format!("{member} in {} is not UTF-8: {err}", path.display()))
}

fn musicxml_to_keyflow_chart(xml: &str) -> Result<String, String> {
    let xml = strip_doctype(xml);
    let doc = roxmltree::Document::parse(&xml).map_err(|err| format!("invalid MusicXML: {err}"))?;
    let (title, composer) = musicxml_metadata_from_doc(&doc);
    let time_sig = first_time_signature(&doc).unwrap_or((4, 4));
    let key = first_key(&doc).unwrap_or_else(|| "C".to_string());
    let measures = parse_musicxml_measures(&doc);

    let mut out = String::new();
    out.push_str(&title);
    out.push('\n');
    if let Some(composer) = composer {
        if !composer.trim().is_empty() {
            out.push_str("Composer: ");
            out.push_str(composer.trim());
            out.push('\n');
        }
    }
    out.push_str(&format!(
        "120bpm {}/{} #{}\n\n",
        time_sig.0, time_sig.1, key
    ));
    out.push_str("CHART\n");

    for (idx, measure) in measures.iter().enumerate() {
        if idx > 0 && idx % 4 == 0 {
            out.push('\n');
        }
        if idx % 4 != 0 {
            out.push_str(" | ");
        }
        if measure.harmonies.is_empty() {
            out.push('.');
        } else {
            out.push_str(
                &measure
                    .harmonies
                    .iter()
                    .map(|harmony| harmony.symbol.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }
    out.push('\n');

    Ok(out)
}

fn musicxml_metadata(xml: &str) -> Result<(String, Option<String>), String> {
    let xml = strip_doctype(xml);
    let doc = roxmltree::Document::parse(&xml).map_err(|err| format!("invalid MusicXML: {err}"))?;
    Ok(musicxml_metadata_from_doc(&doc))
}

fn musicxml_metadata_from_doc(doc: &roxmltree::Document<'_>) -> (String, Option<String>) {
    let title = doc
        .descendants()
        .find(|n| n.has_tag_name("work-title"))
        .and_then(|n| n.text())
        .or_else(|| {
            doc.descendants()
                .find(|n| {
                    n.has_tag_name("credit-words")
                        && n.attribute("justify")
                            .is_some_and(|value| value == "center")
                })
                .and_then(|n| n.text())
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Untitled")
        .to_string();

    let composer = doc
        .descendants()
        .find(|n| n.has_tag_name("creator") && n.attribute("type") == Some("composer"))
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    (title, composer)
}

fn first_time_signature(doc: &roxmltree::Document<'_>) -> Option<(u8, u8)> {
    let time = doc.descendants().find(|n| n.has_tag_name("time"))?;
    let beats = child_text(time, "beats")?.trim().parse::<u8>().ok()?;
    let beat_type = child_text(time, "beat-type")?.trim().parse::<u8>().ok()?;
    Some((beats, beat_type))
}

fn first_key(doc: &roxmltree::Document<'_>) -> Option<String> {
    let fifths = doc
        .descendants()
        .find(|n| n.has_tag_name("fifths"))
        .and_then(|n| n.text())
        .and_then(|s| s.trim().parse::<i8>().ok())?;
    Some(
        match fifths {
            -7 => "Cb",
            -6 => "Gb",
            -5 => "Db",
            -4 => "Ab",
            -3 => "Eb",
            -2 => "Bb",
            -1 => "F",
            0 => "C",
            1 => "G",
            2 => "D",
            3 => "A",
            4 => "E",
            5 => "B",
            6 => "F#",
            7 => "C#",
            _ => "C",
        }
        .to_string(),
    )
}

fn parse_musicxml_measures(doc: &roxmltree::Document<'_>) -> Vec<CatalogMeasure> {
    doc.descendants()
        .filter(|n| n.has_tag_name("measure"))
        .map(|measure| {
            let mut harmonies = Vec::new();
            for child in measure.children().filter(|n| n.is_element()) {
                if child.has_tag_name("harmony") {
                    if let Some(symbol) = harmony_symbol(child) {
                        harmonies.push(CatalogHarmony { symbol });
                    }
                }
            }
            CatalogMeasure { harmonies }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct CatalogMeasure {
    harmonies: Vec<CatalogHarmony>,
}

#[derive(Debug, Clone)]
struct CatalogHarmony {
    symbol: String,
}

fn strip_doctype(xml: &str) -> String {
    let Some(start) = xml.find("<!DOCTYPE") else {
        return xml.to_string();
    };
    let Some(end) = xml[start..].find('>') else {
        return xml.to_string();
    };

    let mut stripped = String::with_capacity(xml.len());
    stripped.push_str(&xml[..start]);
    stripped.push_str(&xml[start + end + 1..]);
    stripped
}

fn harmony_symbol(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let root = node.children().find(|n| n.has_tag_name("root"))?;
    let root_step = child_text(root, "root-step")?;
    let root_alter = child_text(root, "root-alter").and_then(alter_suffix);
    let kind = node.children().find(|n| n.has_tag_name("kind"));
    let kind_text = kind.and_then(|n| n.attribute("text")).unwrap_or("");
    let kind_name = kind
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let bass = node.children().find(|n| n.has_tag_name("bass"));

    let mut symbol = format!(
        "{}{}{}",
        root_step.trim(),
        root_alter.unwrap_or_default(),
        if kind_text.is_empty() {
            musicxml_kind_suffix(kind_name)
        } else {
            kind_text
        }
    );

    if let Some(bass) = bass {
        if let Some(step) = child_text(bass, "bass-step") {
            let alter = child_text(bass, "bass-alter").and_then(alter_suffix);
            symbol.push('/');
            symbol.push_str(step.trim());
            symbol.push_str(alter.unwrap_or_default());
        }
    }

    Some(normalize_chord_symbol(&symbol))
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
}

fn alter_suffix(value: &str) -> Option<&'static str> {
    match value.trim() {
        "1" => Some("#"),
        "-1" => Some("b"),
        _ => None,
    }
}

fn musicxml_kind_suffix(kind: &str) -> &'static str {
    match kind {
        "" | "major" => "",
        "minor" => "m",
        "dominant" => "7",
        "dominant-ninth" => "9",
        "dominant-11th" => "11",
        "dominant-13th" => "13",
        "major-seventh" => "maj7",
        "major-sixth" => "6",
        "major-ninth" => "maj9",
        "major-11th" => "maj11",
        "major-13th" => "maj13",
        "minor-sixth" => "m6",
        "minor-seventh" => "m7",
        "minor-ninth" => "m9",
        "minor-11th" => "m11",
        "minor-13th" => "m13",
        "diminished" => "dim",
        "augmented" => "+",
        "augmented-seventh" => "+7",
        "half-diminished" => "m7b5",
        "diminished-seventh" => "dim7",
        "suspended-fourth" => "sus4",
        "suspended-second" => "sus2",
        _ => "",
    }
}

fn normalize_chord_symbol(symbol: &str) -> String {
    symbol
        .replace('△', "maj")
        .replace('ø', "m7b5")
        .replace('°', "dim")
        .replace('−', "m")
        .replace("min", "m")
        .replace("M7", "maj7")
        .replace(' ', "")
}
