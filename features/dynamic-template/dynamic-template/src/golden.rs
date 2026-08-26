//! Derive the golden [`IdealFullSessionTemplate`] from the classification
//! [`Config`](crate::DynamicTemplateConfig).
//!
//! The configuration *is* the single source of truth: walking the Group tree
//! yields the structural folders (instrument groups and sub-groups), and each
//! group's dimension sub-groups (Performer / Section / Arrangement / Layers /
//! Channel) plus its multi-mic descriptors yield the per-project deepening
//! axes — with their configured descriptor vocabularies attached. The engine
//! later expands and collapses that schema per project.

use dynamic_template_proto::{IdealFullSessionTemplate, TemplateBus, TemplateNode};
use monarchy::Group;

use crate::colors::color_for_path;
use crate::ItemMetadata;

/// Sub-group names that are per-project **dimensions** rather than structural
/// folders, in canonical deepening order. Multi-mic is handled separately
/// (it usually arrives via `field_value_descriptors`) but is also recognised
/// here for the configs that model it as a nested group.
const DIMENSION_NAMES: &[&str] = &[
    "Performer",
    "Section",
    "Arrangement",
    "Layers",
    "Channel",
    "MultiMic",
];

fn is_dimension(name: &str) -> bool {
    DIMENSION_NAMES.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// The slot band whose canonical path matches `path` exactly (case-insensitive,
/// element for element). Unlike `band_for_category`'s loose element matching,
/// this won't mistake `Synths/Lead` for the `Vocals/Lead` band or `Keys/Electric`
/// for `Guitars/Electric`.
fn band_for_exact_path(path: &[String]) -> Option<&'static music_catalog::groups::GroupSlotBand> {
    music_catalog::groups::SLOT_BANDS.iter().find(|b| {
        b.path.len() == path.len()
            && b.path
                .iter()
                .zip(path)
                .all(|(a, c)| a.eq_ignore_ascii_case(c))
    })
}

/// Build the canonical FTS golden session by walking [`default_config`].
///
/// [`default_config`]: crate::default_config
pub fn golden_template() -> IdealFullSessionTemplate {
    let cfg = crate::default_config();
    let mut root = Vec::new();
    for g in &cfg.groups {
        // Skip metadata-only groups (e.g. the global "MetadataFields" patterns)
        // and transparent groups, which don't produce a folder of their own.
        if g.metadata_only || g.transparent {
            continue;
        }
        let mut path = Vec::new();
        root.push(build_node(g, &mut path));
    }

    IdealFullSessionTemplate {
        name: "FTS Golden Session".to_string(),
        version: 1,
        root,
        buses: vec![TemplateBus {
            name: "Mix Bus".to_string(),
            channels: 2,
        }],
    }
}

/// Build a [`TemplateNode`] subtree for one config group. `path` tracks the
/// canonical group path (top-level first) as we recurse.
fn build_node(g: &Group<ItemMetadata>, path: &mut Vec<String>) -> TemplateNode {
    path.push(g.name.clone());

    let group_path: Vec<&str> = path.iter().map(String::as_str).collect();
    let mut node = TemplateNode::folder(&g.name, path.clone());
    node.defaults.color_hex = color_for_path(&group_path).map(|c| c.to_hex_string());

    // Group-slot membership where this node's full canonical path maps to a
    // 128-slot band (e.g. ["Guitars","Electric"] → Electric Gtr; ["Drums"] →
    // Drums). Matched on the full path so sub-groups like Synths/Lead or
    // Keys/Electric don't inherit an unrelated band.
    if let Some(band) = band_for_exact_path(path) {
        node = node.in_group(band.label);
    }

    // Structural sub-groups become child folders (recurse); dimension
    // sub-groups feed the deepening chain. Config order is preserved.
    let mut children: Vec<TemplateNode> = Vec::new();
    for child in g.groups.iter().filter(|c| !is_dimension(&c.name)) {
        children.push(build_node(child, path));
    }

    let dimensions: Vec<&Group<ItemMetadata>> =
        g.groups.iter().filter(|c| is_dimension(&c.name)).collect();
    let mic_vocab: Vec<String> = g
        .field_value_descriptors
        .get("MultiMic")
        .map(|ds| ds.iter().map(|d| d.value.clone()).collect())
        .unwrap_or_default();

    if let Some(chain) = dimension_chain(&dimensions, &mic_vocab) {
        children.push(chain);
    }

    // A tagged collection (e.g. drum SUM) declares a conditional grouping over
    // its pattern vocabulary — the engine groups matching captures under it and
    // leaves the rest alongside.
    if let Some(tc) = &g.tagged_collection {
        children.push(TemplateNode::collection(&tc.name, tc.patterns.clone()));
    }

    node = node.with_children(children);

    path.pop();
    node
}

/// Fold the dimension sub-groups (outer → inner) into a nested chain of
/// dimension nodes, bottoming out at the multi-mic captures from
/// `field_value_descriptors` (if any). Each dimension carries its configured
/// `patterns` as its vocabulary. Returns `None` when there are no dimensions
/// and no multi-mic captures.
fn dimension_chain(dims: &[&Group<ItemMetadata>], mic_vocab: &[String]) -> Option<TemplateNode> {
    // Innermost: the multi-mic capture dimension from field_value_descriptors.
    let mut inner: Option<TemplateNode> =
        (!mic_vocab.is_empty()).then(|| TemplateNode::dimension("MultiMic", mic_vocab.to_vec()));

    for d in dims.iter().rev() {
        // A nested-group "MultiMic" already supplies its own vocabulary via
        // patterns; other dimensions likewise use their configured patterns.
        let mut dim = TemplateNode::dimension(&d.name, d.patterns.clone());
        if let Some(child) = inner.take() {
            dim = dim.with_child(child);
        }
        inner = Some(dim);
    }
    inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use dynamic_template_proto::NodeKind;

    fn find<'a>(node: &'a TemplateNode, name: &str) -> Option<&'a TemplateNode> {
        node.children.iter().find(|c| c.name == name)
    }

    #[test]
    fn report_uncolored_folders() {
        fn walk<'a>(n: &'a TemplateNode, out: &mut Vec<&'a TemplateNode>) {
            if n.kind == NodeKind::Folder
                && !n.group_path.is_empty()
                && n.defaults.color_hex.is_none()
            {
                out.push(n);
            }
            for c in &n.children {
                walk(c, out);
            }
        }
        let t = golden_template();
        let mut missing = Vec::new();
        for n in &t.root {
            walk(n, &mut missing);
        }
        let paths: Vec<String> = missing.iter().map(|n| n.group_path.join("/")).collect();
        assert!(
            paths.is_empty(),
            "folders without a resolved color: {paths:#?}"
        );
    }

    #[test]
    fn golden_has_top_level_instrument_folders() {
        let t = golden_template();
        let names: Vec<&str> = t.root.iter().map(|n| n.name.as_str()).collect();
        // A few expected instrument groups; the metadata-only group is skipped.
        assert!(names.contains(&"Drums"));
        assert!(names.contains(&"Guitars"));
        assert!(names.contains(&"Synths"));
        assert!(!names.contains(&"MetadataFields"));
    }

    #[test]
    fn drums_kick_carries_multimic_vocabulary() {
        let t = golden_template();
        let drums = t.root.iter().find(|n| n.name == "Drums").unwrap();
        // Drums slot-band membership is set on the top folder.
        assert_eq!(
            drums.group_membership.as_ref().map(|g| g.category.as_str()),
            Some("Drums")
        );
        let kit = find(drums, "Drum Kit").expect("Drum Kit sub-folder");
        let kick = find(kit, "Kick").expect("Kick sub-folder");
        let mics = find(kick, "MultiMic").expect("Kick MultiMic dimension");
        assert_eq!(mics.kind, NodeKind::Dimension);
        assert_eq!(mics.vocabulary, vec!["In", "Out", "Top", "Bottom"]);

        // Kick also declares the SUM tagged collection (In/Out/Trig).
        let sum = find(kick, "SUM").expect("Kick SUM collection");
        assert_eq!(sum.kind, NodeKind::Collection);
        assert_eq!(sum.vocabulary, vec!["In", "Out", "Trig"]);
    }

    #[test]
    fn electric_guitar_nests_configured_dimensions() {
        let t = golden_template();
        let guitars = t.root.iter().find(|n| n.name == "Guitars").unwrap();
        let electric = find(guitars, "Electric").expect("Electric sub-folder");
        // Electric maps to the Electric Gtr slot band.
        assert_eq!(
            electric
                .group_membership
                .as_ref()
                .map(|g| g.category.as_str()),
            Some("Electric Gtr")
        );

        // Performer → Arrangement → Layers → Channel → MultiMic, in order.
        let performer = find(electric, "Performer").expect("Performer dimension");
        assert_eq!(performer.kind, NodeKind::Dimension);
        let arrangement = find(performer, "Arrangement").expect("Arrangement");
        assert!(arrangement.vocabulary.contains(&"Clean".to_string()));
        assert!(arrangement.vocabulary.contains(&"Crunch".to_string()));
        let layers = find(arrangement, "Layers").expect("Layers");
        let channel = find(layers, "Channel").expect("Channel");
        assert!(channel.vocabulary.contains(&"L".to_string()));
        let mics = find(channel, "MultiMic").expect("MultiMic captures");
        assert_eq!(mics.vocabulary, vec!["Amp", "DI"]);
    }

    #[test]
    fn membership_matches_full_path_not_stray_elements() {
        let t = golden_template();
        // Synths/Lead must NOT inherit the Vocals/Lead band.
        let synths = t.root.iter().find(|n| n.name == "Synths").unwrap();
        let synth_lead = find(synths, "Lead").expect("Synths/Lead");
        assert!(synth_lead.group_membership.is_none());
        // Keys/Electric must NOT inherit the Guitars/Electric band.
        let keys = t.root.iter().find(|n| n.name == "Keys").unwrap();
        let keys_electric = find(keys, "Electric").expect("Keys/Electric");
        assert!(keys_electric.group_membership.is_none());

        // Vocals/BGVs DOES map to the Background Vox band (path aligned to the
        // config's "BGVs" sub-group name).
        let vocals = t.root.iter().find(|n| n.name == "Vocals").unwrap();
        let bgvs = find(vocals, "BGVs").expect("Vocals/BGVs");
        assert_eq!(
            bgvs.group_membership.as_ref().map(|g| g.category.as_str()),
            Some("Background Vox")
        );
    }
}
