//! Default metadata field patterns for extracting metadata from input strings
//!
//! These patterns are used in a metadata-only group that extracts metadata fields
//! like Section, MultiMic, Arrangement, etc. across all groups without creating
//! structure nodes in the hierarchy.

use crate::item_metadata::prelude::*;

/// Creates a default metadata field patterns group
///
/// This is a metadata-only group that extracts metadata fields like Section, MultiMic,
/// Arrangement, etc. from input strings. It doesn't create a structure node in the
/// hierarchy but provides metadata extraction across all groups.
pub fn default_metadata_field_patterns() -> Group<ItemMetadata> {
    // Define each field's patterns separately for clarity
    // Section patterns for song structure elements
    // NOTE: Longer/compound patterns must come BEFORE shorter patterns
    // so "Pre-Chorus" matches before "Chorus" does
    let section = Group::builder("Section")
        .patterns([
            // Pre/Post modifiers (MUST come before Verse/Chorus/etc.)
            "Pre-Chorus",
            "Pre Chorus",
            "PreChorus",
            "Post-Chorus",
            "Post Chorus",
            "PostChorus",
            "Pre-Verse",
            "Pre Verse",
            "PreVerse",
            // Numbered sections (MUST come before base sections)
            "Verse 1",
            "Verse 2",
            "Verse 3",
            "Verse 4",
            "Chorus 1",
            "Chorus 2",
            "Chorus 3",
            "Bridge 1",
            "Bridge 2",
            "V1",
            "V2",
            "V3",
            "V4",
            "C1",
            "C2",
            "C3",
            // Compound sections
            "Middle Bridge",
            "Middle 8",
            "Middle8",
            // Main sections
            "Intro",
            "Verse",
            "Chorus",
            "Bridge",
            "Outro",
            "Vamp",
            "Coda",
            "Tag",
            // Instrumental sections
            "Instrumental",
            "Solo",
            "Break",
            "Breakdown",
            "Buildup",
            "Build",
            "Drop",
            "Interlude",
            "Turnaround",
            // Other common sections
            "Middle",
            "Hook",
            "Refrain",
            "Transition",
            "Ending",
        ])
        .build();

    let arrangement = Group::builder("Arrangement")
        .patterns([
            "Down",
            "Big",
            "Build",
            // Guitar/instrument technique arrangements
            "Harmonics",
            "Strum",
            "Fingerpick",
            "Tremolo",
        ])
        .build();

    let layers = Group::builder("Layers")
        .patterns([
            "DBL",
            "TPL",
            "Double",
            "Triple",
            "Main",
            "OCT",
            "Duplicate",
            "1",
            "2",
            "3",
            "4",
            "5",
        ])
        .build();

    let channel = Group::builder("Channel")
        .patterns(["L", "C", "R", "Left", "Center", "Right"])
        .build();

    let playlist = Group::builder("Playlist")
        .patterns([".1", ".2", ".3", ".4", ".5"])
        .build();

    let multi_mic = Group::builder("MultiMic")
        .patterns(["Top", "Bottom", "In", "Out", "DI", "Amp", "Amplitube"])
        .build();

    let performer = Group::builder("Performer")
        .patterns([
            // Common first names - add more as needed
            "Aaron", "Adam", "Alex", "Amanda", "Amy", "Andrew", "Angela", "Anna", "Ben", "Beth",
            "Bill", "Bob", "Brad", "Brandon", "Bri", "Brian", "Brittany", "Caleb", "Carlos",
            "Chad", "Chris", "Cody", "Connor", "Corey", "Dakota", "Dan", "Daniel", "Dave", "David",
            "Derek", "Diana", "Dylan", "Ed", "Elena", "Emily", "Emma", "Eric", "Ethan", "Evan",
            "Fernando", "Frank", "Fred", "Gary", "George", "Grace", "Greg", "Hannah", "Harry",
            "Heather", "Henry", "Ian", "Isaac", "Jack", "Jacob", "Jake", "James", "Jamie", "Jason",
            "Jeff", "Jen", "Jennifer", "Jeremy", "Jesse", "Jessica", "Jim", "Joe", "Joey", "John",
            "Johnny", "Johny", "Jon", "Jordan", "Jose", "Josh", "Joshua", "JT", "Juan", "Julia",
            "Justin", "Karen", "Kate", "Katie", "Keith", "Kelly", "Ken", "Kevin", "Kim", "Kyle",
            "Larry", "Laura", "Lauren", "Leo", "Lisa", "Logan", "Lou", "Luke", "Madison", "Marc",
            "Marcus", "Maria", "Mark", "Matt", "Matthew", "Megan", "Melissa", "Michael",
            "Michelle", "Mike", "Mitch", "Morgan", "Nancy", "Nate", "Nathan", "Nick", "Nicole",
            "Noah", "Oliver", "Olivia", "Pat", "Patrick", "Paul", "Perry", "Pete", "Peter", "Phil",
            "Rachel", "Randy", "Ray", "Rebecca", "Richard", "Rick", "Rob", "Robert", "Roger",
            "Ron", "Ryan", "Sam", "Samantha", "Sandra", "Sara", "Sarah", "Scott", "Sean", "Seth",
            "Shane", "Shannon", "Sharon", "Shawn", "Sophia", "Stacy", "Steve", "Steven", "Taylor",
            "Thomas", "Tit", "Tim", "Timothy", "Todd", "Tom", "Tony", "Travis", "Tyler", "Victor",
            "Victoria", "Warren", "Will", "William", "Zach", "Zachary", "Zoe",
        ])
        .build();

    let rec_tag = Group::builder("RecTag")
        .patterns([
            "PASS 1", "PASS 2", "PASS 3", "PASS 4", "PASS-1", "PASS-2", "PASS-3", "PASS-4",
        ])
        .build();

    // Effect patterns - audio processing effects applied to tracks
    // These can appear on any instrument track (e.g., "Snare Verb", "Vocal Comp")
    let effect = Group::builder("Effect")
        .patterns([
            // Reverb variations
            "Verb",
            "Reverb",
            "Rev",
            // Note: "Room" excluded - conflicts with room mics in drums
            "Hall",
            "Plate",
            "Chamber",
            "Spring",
            // Delay variations
            "Delay",
            "Dly",
            "Echo",
            "Slap",
            "Slapback",
            // Compression/dynamics
            "Compressor",
            "Limiter",
            "Lim",
            // Distortion/saturation
            "Dist",
            "Distortion",
            "Fuzz",
            "Overdrive",
            "OD",
            "Saturation",
            "Sat",
            // Modulation effects
            "Chorus",
            "Flanger",
            "Phaser",
            "Tremolo",
            "Vibrato",
            // EQ/filtering
            "EQ",
            "Filter",
            "HPF",
            "LPF",
            // Other common effects
            "Parallel",
            "Crush",
            "Bitcrush",
            "Send",
            "Return",
            "Aux",
            // Note: "FX" intentionally excluded - too generic, matches SFX filenames
        ])
        .build();

    // Combine all field patterns into a single metadata-only group
    Group::builder("MetadataFields")
        .metadata_only()
        .section(section)
        .arrangement(arrangement)
        .layers(layers)
        .channel(channel)
        .playlist(playlist)
        .multi_mic(multi_mic)
        .performer(performer)
        .rec_tag(rec_tag)
        .effect(effect)
        .build()
}
