//! Document-level parsing for multi-block .kf files
//!
//! Handles `--- name ---` delimiters to split a .kf file into named blocks.
//! Without delimiters, the entire content is treated as a single "keyflow" block.

use keyflow_proto::document::{KfBlock, KfBlockKind, KfDocument};

/// Parse a .kf document string into a `KfDocument` with named blocks.
///
/// Block delimiters use the format `--- name ---` (at least 3 dashes on each side).
/// If no delimiters are found, the entire content is treated as a single "keyflow" block.
///
/// # Examples
/// ```ignore
/// let doc = parse_kf_document("--- keyflow ---\nVS\nC G Am F\n--- chordpro ---\n[C]Hello");
/// assert_eq!(doc.blocks.len(), 2);
/// ```
pub fn parse_kf_document(content: &str) -> Result<KfDocument, String> {
    let mut blocks = Vec::new();
    let mut current_block_name = String::from("keyflow");
    let mut current_block_content = String::new();
    let mut found_delimiter = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("---") && trimmed.ends_with("---") && trimmed.len() > 6 {
            let middle = trimmed[3..trimmed.len() - 3].trim();
            if !middle.is_empty() {
                found_delimiter = true;

                if !current_block_content.trim().is_empty() {
                    let kind = KfBlockKind::from_name(&current_block_name);
                    blocks.push(KfBlock::new(
                        current_block_name.clone(),
                        kind,
                        current_block_content.trim().to_string(),
                    ));
                }

                current_block_name = middle.to_string();
                current_block_content = String::new();
                continue;
            }
        }

        if !current_block_content.is_empty() {
            current_block_content.push('\n');
        }
        current_block_content.push_str(line);
    }

    if !current_block_content.trim().is_empty() {
        let kind = KfBlockKind::from_name(&current_block_name);
        blocks.push(KfBlock::new(
            current_block_name,
            kind,
            current_block_content.trim().to_string(),
        ));
    }

    if !found_delimiter {
        // No delimiters found — treat entire content as a single keyflow block
        return Ok(KfDocument::new(KfBlock::keyflow(content)));
    }

    Ok(KfDocument { blocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_block_no_delimiter() {
        let doc = parse_kf_document("VS\nC G Am F").unwrap();
        assert!(doc.is_plain_keyflow());
        assert_eq!(doc.blocks.len(), 1);
    }

    #[test]
    fn test_multi_block() {
        let content = "--- keyflow ---\nVS\nC G Am F\n--- chordpro ---\n[C]Hello [G]World";
        let doc = parse_kf_document(content).unwrap();
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.blocks[0].kind, KfBlockKind::Keyflow);
        assert_eq!(doc.blocks[1].kind, KfBlockKind::ChordPro);
    }

    #[test]
    fn test_find_block_case_insensitive() {
        let content = "--- keyflow ---\nVS\nC G\n--- ChordPro ---\n[C]Hello";
        let doc = parse_kf_document(content).unwrap();
        assert!(doc.find_block("chordpro").is_some());
        assert!(doc.find_block("KEYFLOW").is_some());
    }
}
