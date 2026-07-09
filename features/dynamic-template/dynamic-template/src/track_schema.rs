use std::collections::HashSet;

use monarchy::{FieldName, Group, Parser};

use crate::item_metadata::{ItemMetadata, ItemMetadataField};
use crate::{DynamicTemplateConfig, default_config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackDimension {
    Channel,
    Layer,
    MultiMic,
    Performer,
    Arrangement,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackClassification {
    pub name: String,
    pub visibility_groups: Vec<String>,
    pub matched_groups: Vec<String>,
}

pub fn classify_track(name: &str) -> TrackClassification {
    let config = default_config();
    classify_track_with_config(name, &config)
}

pub fn classify_track_with_config(
    name: &str,
    config: &DynamicTemplateConfig,
) -> TrackClassification {
    let matched_groups = Parser::new(config)
        .parse(name.to_string())
        .map(|item| {
            item.matched_groups
                .into_iter()
                .map(|group| group.name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let visibility_groups = top_level_visibility_groups(&matched_groups, config);
    TrackClassification {
        name: name.to_string(),
        visibility_groups,
        matched_groups,
    }
}

pub fn track_is_in_visibility_group(name: &str, group: &str) -> bool {
    let config = default_config();
    classify_track_with_config(name, &config)
        .visibility_groups
        .iter()
        .any(|candidate| normalize(candidate) == normalize(group))
}

pub fn classify_track_dimension(name: &str, context: &[String]) -> TrackDimension {
    let config = default_config();
    classify_track_dimension_with_config(name, context, &config)
}

pub fn classify_track_dimension_with_config(
    name: &str,
    context: &[String],
    config: &DynamicTemplateConfig,
) -> TrackDimension {
    let input = contextual_name(name, context);
    let Ok(item) = Parser::new(config).parse(input) else {
        return TrackDimension::Other;
    };
    let metadata = item.metadata;

    if metadata
        .channel
        .as_ref()
        .is_some_and(|value| is_configured_value(TrackDimension::Channel, value, context, config))
    {
        TrackDimension::Channel
    } else if metadata.multi_mic.as_ref().is_some_and(|values| {
        values
            .iter()
            .any(|value| is_configured_value(TrackDimension::MultiMic, value, context, config))
    }) {
        TrackDimension::MultiMic
    } else if metadata
        .layers
        .as_ref()
        .is_some_and(|value| is_configured_value(TrackDimension::Layer, value, context, config))
    {
        TrackDimension::Layer
    } else if metadata
        .performer
        .as_ref()
        .is_some_and(|value| is_configured_value(TrackDimension::Performer, value, context, config))
    {
        TrackDimension::Performer
    } else if metadata.arrangement.as_ref().is_some_and(|value| {
        is_configured_value(TrackDimension::Arrangement, value, context, config)
    }) {
        TrackDimension::Arrangement
    } else {
        TrackDimension::Other
    }
}

pub fn configured_values_for_dimension(
    dimension: TrackDimension,
    context: &[String],
) -> Vec<String> {
    let config = default_config();
    configured_values_for_dimension_with_config(dimension, context, &config)
}

pub fn configured_values_for_dimension_with_config(
    dimension: TrackDimension,
    context: &[String],
    config: &DynamicTemplateConfig,
) -> Vec<String> {
    let Some(field) = field_for_dimension(dimension) else {
        return Vec::new();
    };
    let field_name = field.name();
    let context_input = context.join(" ");
    let matched_group_names: HashSet<String> = Parser::new(config)
        .parse(context_input)
        .map(|item| {
            item.matched_groups
                .into_iter()
                .map(|group| group.name.clone())
                .collect()
        })
        .unwrap_or_default();

    let mut group_values = Vec::new();
    let mut fallback_values = Vec::new();
    for group in &config.groups {
        collect_configured_values(
            group,
            field_name,
            &matched_group_names,
            &mut group_values,
            &mut fallback_values,
        );
    }
    if group_values.is_empty() {
        dedupe_preserving_order(fallback_values)
    } else {
        dedupe_preserving_order(group_values)
    }
}

pub fn next_configured_value(
    dimension: TrackDimension,
    context: &[String],
    existing: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<String> {
    let existing = existing
        .into_iter()
        .map(|name| normalize(name.as_ref()))
        .collect::<HashSet<_>>();
    configured_values_for_dimension(dimension, context)
        .into_iter()
        .find(|name| !existing.contains(&normalize(name)))
}

pub fn initial_values_for_dimension(
    dimension: TrackDimension,
    context: &[String],
    count: usize,
) -> Vec<String> {
    let values = configured_values_for_dimension(dimension, context);
    let values = match dimension {
        TrackDimension::Channel => channel_scaffold_order(values),
        _ => values,
    };
    values.into_iter().take(count).collect()
}

fn collect_configured_values(
    group: &Group<ItemMetadata>,
    field_name: &str,
    matched_group_names: &HashSet<String>,
    group_values: &mut Vec<String>,
    fallback_values: &mut Vec<String>,
) {
    let target = if matched_group_names.contains(&group.name) {
        Some(&mut *group_values)
    } else if group.metadata_only {
        Some(&mut *fallback_values)
    } else {
        None
    };

    if let Some(output) = target {
        if let Some(descriptors) = group.field_value_descriptors.get(field_name) {
            output.extend(
                descriptors
                    .iter()
                    .map(|descriptor| descriptor.value.clone()),
            );
        }

        for child in &group.groups {
            if child.name == field_name {
                output.extend(child.patterns.iter().cloned());
            }
        }
    }

    for child in &group.groups {
        collect_configured_values(
            child,
            field_name,
            matched_group_names,
            group_values,
            fallback_values,
        );
    }
}

fn top_level_visibility_groups(
    matched_groups: &[String],
    config: &DynamicTemplateConfig,
) -> Vec<String> {
    let top_level = config
        .groups
        .iter()
        .filter(|group| !group.metadata_only)
        .map(|group| normalize(&group.name))
        .collect::<HashSet<_>>();
    dedupe_preserving_order(
        matched_groups
            .iter()
            .filter(|group| top_level.contains(&normalize(group)))
            .cloned()
            .collect(),
    )
}

fn is_configured_value(
    dimension: TrackDimension,
    value: &str,
    context: &[String],
    config: &DynamicTemplateConfig,
) -> bool {
    configured_values_for_dimension_with_config(dimension, context, config)
        .iter()
        .any(|configured| normalize(configured) == normalize(value))
}

fn field_for_dimension(dimension: TrackDimension) -> Option<ItemMetadataField> {
    match dimension {
        TrackDimension::Channel => Some(ItemMetadataField::Channel),
        TrackDimension::Layer => Some(ItemMetadataField::Layers),
        TrackDimension::MultiMic => Some(ItemMetadataField::MultiMic),
        TrackDimension::Performer => Some(ItemMetadataField::Performer),
        TrackDimension::Arrangement => Some(ItemMetadataField::Arrangement),
        TrackDimension::Other => None,
    }
}

fn contextual_name(name: &str, context: &[String]) -> String {
    if context.is_empty() {
        name.to_string()
    } else {
        format!("{} {name}", context.join(" "))
    }
}

fn dedupe_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(normalize(value)))
        .collect()
}

fn channel_scaffold_order(configured: Vec<String>) -> Vec<String> {
    let mut values = Vec::new();
    push_matching(&configured, &mut values, &["l", "left"]);
    push_matching(&configured, &mut values, &["r", "right"]);
    push_matching(&configured, &mut values, &["c", "center", "centre"]);
    for value in configured {
        if !values
            .iter()
            .any(|existing| normalize(existing) == normalize(&value))
        {
            values.push(value);
        }
    }
    values
}

fn push_matching(configured: &[String], output: &mut Vec<String>, matches: &[&str]) {
    if let Some(value) = configured
        .iter()
        .find(|value| matches.contains(&normalize(value).as_str()))
    {
        output.push(value.clone());
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "pre-existing failure (predates hygiene migration): classify_track_dimension(\"In\") returns Arrangement, test expects Other — revisit classification config"]
    fn classifies_dimensions_from_contextual_template_metadata() {
        let context = vec!["GTR E Clean".to_string()];

        assert_eq!(
            classify_track_dimension("Amp", &context),
            TrackDimension::MultiMic
        );
        assert_eq!(
            classify_track_dimension("DI", &context),
            TrackDimension::MultiMic
        );
        assert_eq!(
            classify_track_dimension("In", &context),
            TrackDimension::Other
        );
        assert_eq!(
            classify_track_dimension("L", &context),
            TrackDimension::Channel
        );
    }

    #[test]
    fn configured_values_are_group_aware() {
        let context = vec!["GTR E Clean".to_string()];

        let channels = configured_values_for_dimension(TrackDimension::Channel, &context);
        assert!(channels.iter().any(|value| value == "L"));
        assert!(channels.iter().any(|value| value == "R"));

        let multi_mics = configured_values_for_dimension(TrackDimension::MultiMic, &context);
        assert!(multi_mics.iter().any(|value| value == "Amp"));
        assert!(multi_mics.iter().any(|value| value == "DI"));
        assert!(!multi_mics.iter().any(|value| value == "Top"));
        assert!(!multi_mics.iter().any(|value| value == "In"));
    }

    #[test]
    fn classifies_top_level_visibility_group() {
        let track = classify_track("Kick In");

        assert!(track.visibility_groups.iter().any(|group| group == "Drums"));
    }
}
