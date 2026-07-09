use dynamic_template::{ItemMetadata, OrganizeIntoTracks, default_config};
use monarchy::Parser;
use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_usage();
        return;
    }

    // Check for flags
    let verbose = args.iter().any(|a| a == "-v" || a == "--verbose");
    let tree_only = args.iter().any(|a| a == "-t" || a == "--tree");
    let json_output = args.iter().any(|a| a == "-j" || a == "--json");

    // Filter out flags to get inputs
    let inputs: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    if inputs.is_empty() {
        print_usage();
        return;
    }

    let config = default_config();

    if inputs.len() == 1 && !tree_only {
        // Single input - show detailed parsing info
        analyze_single(inputs[0], &config, verbose, json_output);
    } else {
        // Multiple inputs - show tree structure
        analyze_multiple(&inputs, &config, verbose, json_output);
    }
}

fn print_usage() {
    eprintln!(
        r#"
Dynamic Template CLI - Test track sorting and parsing

USAGE:
    dynamic-template [FLAGS] <input>...

FLAGS:
    -v, --verbose    Show detailed parsing information
    -t, --tree       Force tree output even for single input
    -j, --json       Output results as JSON

EXAMPLES:
    # Single input - shows parsing details
    dynamic-template "Kick In"

    # Multiple inputs - shows tree structure
    dynamic-template "Kick In" "Kick Out" "Snare Top" "Snare Bot"

    # Verbose mode with tree
    dynamic-template -v "Acc Guitar" "Electric Guitar Clean"

    # JSON output
    dynamic-template -j "SNR VERB" "Hi Hat"
"#
    );
}

fn analyze_single(
    input: &str,
    config: &dynamic_template::DynamicTemplateConfig,
    verbose: bool,
    json: bool,
) {
    println!("═══════════════════════════════════════════════════════════════");
    println!("INPUT: \"{}\"", input);
    println!("═══════════════════════════════════════════════════════════════\n");

    // Parse the input to get metadata
    let parser = Parser::new(config);
    let item = match parser.parse(input.to_string()) {
        Ok(item) => item,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            return;
        }
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&item.metadata).unwrap_or_default()
        );
        return;
    }

    // Show parsed metadata
    println!("PARSED METADATA:");
    println!("────────────────────────────────────────────────────────────────");
    print_metadata(&item.metadata, verbose);

    // Show matched groups
    println!("\nMATCHED GROUPS:");
    println!("────────────────────────────────────────────────────────────────");
    if item.matched_groups.is_empty() {
        println!("  (none - will go to Unsorted)");
    } else {
        for (i, mg) in item.matched_groups.iter().enumerate() {
            let tree_prefix = if i == item.matched_groups.len() - 1 {
                "└─"
            } else {
                "├─"
            };
            println!("  {} {} (prefix: {:?})", tree_prefix, mg.name, mg.prefix);
        }
    }

    // Show what display name would be generated
    let display_name = item.derive_display_name();
    println!("\nDISPLAY NAME:");
    println!("────────────────────────────────────────────────────────────────");
    println!("  \"{}\"", display_name);

    // Show final tracks
    println!("\nTRACKS:");
    println!("────────────────────────────────────────────────────────────────");
    match vec![input.to_string()].organize_into_tracks(config, None) {
        Ok(tracks) => {
            print_tracks(&tracks);
        }
        Err(e) => {
            eprintln!("  Error: {}", e);
        }
    }
}

fn analyze_multiple(
    inputs: &[&str],
    config: &dynamic_template::DynamicTemplateConfig,
    verbose: bool,
    json: bool,
) {
    let input_strings: Vec<String> = inputs.iter().map(|s| s.to_string()).collect();

    println!("═══════════════════════════════════════════════════════════════");
    println!("INPUTS ({} items):", inputs.len());
    println!("═══════════════════════════════════════════════════════════════");
    for input in inputs {
        println!("  • {}", input);
    }
    println!();

    if verbose {
        // Show parsing details for each input
        let parser = Parser::new(config);
        println!("PARSING DETAILS:");
        println!("────────────────────────────────────────────────────────────────");
        for input in inputs {
            let item = match parser.parse(input.to_string()) {
                Ok(item) => item,
                Err(e) => {
                    println!("\n\"{}\": Parse error: {}", input, e);
                    continue;
                }
            };
            println!("\n\"{}\":", input);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&item.metadata).unwrap_or_default()
                );
            } else {
                print_metadata_compact(&item.metadata);
                let groups: Vec<&str> = item
                    .matched_groups
                    .iter()
                    .map(|mg| mg.name.as_str())
                    .collect();
                println!("    matched: {:?}", groups);
                println!("    display: \"{}\"", item.derive_display_name());
            }
        }
        println!();
    }

    // Convert to tracks using organize_into_tracks (default options include expansion)
    println!("TRACKS:");
    println!("────────────────────────────────────────────────────────────────");
    match input_strings.organize_into_tracks(config, None) {
        Ok(tracks) => {
            print_tracks(&tracks);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }
}

fn print_metadata(metadata: &ItemMetadata, verbose: bool) {
    // Always show these
    if let Some(ref group) = metadata.group {
        println!("  group:       {:?}", group);
    }
    if let Some(ref multi_mic) = metadata.multi_mic {
        println!("  multi_mic:   {:?}", multi_mic);
    }
    if let Some(ref effect) = metadata.effect {
        println!("  effect:      {:?}", effect);
    }
    if let Some(ref increment) = metadata.increment {
        println!("  increment:   {:?}", increment);
    }
    if let Some(ref track_type) = metadata.track_type {
        println!("  track_type:  {:?}", track_type);
    }
    if let Some(ref variant) = metadata.variant {
        println!("  variant:     {:?}", variant);
    }
    if let Some(ref layers) = metadata.layers {
        println!("  layers:      {:?}", layers);
    }
    if let Some(ref channel) = metadata.channel {
        println!("  channel:     {:?}", channel);
    }
    if let Some(ref performer) = metadata.performer {
        println!("  performer:   {:?}", performer);
    }
    if let Some(ref arrangement) = metadata.arrangement {
        println!("  arrangement: {:?}", arrangement);
    }
    if let Some(ref section) = metadata.section {
        println!("  section:     {:?}", section);
    }
    if let Some(ref tagged_collection) = metadata.tagged_collection {
        println!("  tagged_coll: {:?}", tagged_collection);
    }

    // Only show in verbose mode
    if verbose {
        if let Some(ref rec_tag) = metadata.rec_tag {
            println!("  rec_tag:     {:?}", rec_tag);
        }
        if let Some(ref playlist) = metadata.playlist {
            println!("  playlist:    {:?}", playlist);
        }
        if let Some(ref file_ext) = metadata.file_extension {
            println!("  file_ext:    {:?}", file_ext);
        }
        if let Some(ref original) = metadata.original_name {
            println!("  original:    {:?}", original);
        }
    }

    // Always show unparsed words if present
    if let Some(ref unparsed) = metadata.unparsed_words {
        if !unparsed.is_empty() {
            println!("  unparsed:    {:?}", unparsed);
        }
    }

    // Show if nothing was parsed
    if metadata.group.is_none()
        && metadata.multi_mic.is_none()
        && metadata.effect.is_none()
        && metadata.increment.is_none()
        && metadata.track_type.is_none()
        && metadata.layers.is_none()
        && metadata.performer.is_none()
        && metadata.arrangement.is_none()
    {
        println!("  (no metadata parsed)");
    }
}

fn print_metadata_compact(metadata: &ItemMetadata) {
    let mut parts = Vec::new();

    if let Some(ref group) = metadata.group {
        parts.push(format!("group={:?}", group));
    }
    if let Some(ref multi_mic) = metadata.multi_mic {
        parts.push(format!("mic={:?}", multi_mic));
    }
    if let Some(ref effect) = metadata.effect {
        parts.push(format!("fx={:?}", effect));
    }
    if let Some(ref increment) = metadata.increment {
        parts.push(format!("inc={}", increment));
    }
    if let Some(ref track_type) = metadata.track_type {
        parts.push(format!("type={}", track_type));
    }
    if let Some(ref unparsed) = metadata.unparsed_words {
        if !unparsed.is_empty() {
            parts.push(format!("unparsed={:?}", unparsed));
        }
    }

    if parts.is_empty() {
        println!("    metadata: (none)");
    } else {
        println!("    metadata: {}", parts.join(", "));
    }
}

fn print_tracks(hierarchy: &daw_proto::TrackHierarchy) {
    use daw_proto::FolderDepthChange;

    let mut depth = 0;

    for track in &hierarchy.tracks {
        let indent_str = "  ".repeat(depth);
        let folder_marker = if track.is_folder { "📁 " } else { "   " };
        let items_info = if track.items.is_empty() {
            String::new()
        } else {
            format!(" [{} item(s)]", track.items.len())
        };

        println!(
            "{}{}{}{}",
            indent_str, folder_marker, track.name, items_info
        );

        // Show items if any
        if !track.items.is_empty() {
            for item in &track.items {
                println!("{}      └─ \"{}\"", indent_str, item);
            }
        }

        // Handle folder depth changes
        match track.folder_depth_change {
            FolderDepthChange::FolderStart => {
                depth += 1;
            }
            FolderDepthChange::ClosesLevels(n) => {
                // n is negative, so we add it (subtracting the absolute value)
                depth = (depth as i32 + n as i32).max(0) as usize;
            }
            FolderDepthChange::Normal => {}
        }
    }
}
