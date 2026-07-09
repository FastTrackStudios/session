//! Shared action definitions for dynamic-template providers.

actions_proto::define_actions! {
    pub dynamic_template_actions {
        prefix: "fts.dynamic_template",
        title: "Dynamic Template",
        SORT_SELECTED = "sort_selected" {
            name: "Sort Selected",
            description: "Organize selected items into a hierarchical track template",
            category: General,
        }
        SORT_ALL = "sort_all" {
            name: "Sort All",
            description: "Organize all project items into a hierarchical track template",
            category: General,
        }
        IMPORT_AND_SORT = "import_and_sort" {
            name: "Import and Sort",
            description: "Import audio files and organize them into a hierarchical track template",
            category: General,
        }
        ORGANIZE_DEMO = "organize_demo" {
            name: "Organize Demo Inputs",
            description: "Run organizer on a built-in sample input set",
            category: Dev,
            group: "Dev",
        }
        LOG_STATUS = "log_status" {
            name: "Log Status",
            description: "Log dynamic-template runtime status",
            category: Dev,
            group: "Dev",
        }
        LOG_GROUPS = "log_groups" {
            name: "Log Groups",
            description: "Log configured dynamic-template group names",
            category: Dev,
            group: "Dev",
        }
        CREATE_NEW_DRUMS = "create_new_drums" {
            name: "Drums",
            description: "Create a new drums template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_DRUM_KIT = "create_new_drum_kit" {
            name: "Drum Kit",
            description: "Create a new drum kit template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_ELECTRONIC_KIT = "create_new_electronic_kit" {
            name: "Electronic Kit",
            description: "Create a new electronic kit template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_PERCUSSION = "create_new_percussion" {
            name: "Percussion",
            description: "Create a new percussion template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_BASS = "create_new_bass" {
            name: "Bass",
            description: "Create a new bass template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_BASS_GUITAR = "create_new_bass_guitar" {
            name: "Bass Guitar",
            description: "Create a new bass guitar template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_BASS_SYNTH = "create_new_bass_synth" {
            name: "Bass Synth",
            description: "Create a new bass synth template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_UPRIGHT_BASS = "create_new_upright_bass" {
            name: "Upright Bass",
            description: "Create a new upright bass template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_GUITARS = "create_new_guitars" {
            name: "Guitars",
            description: "Create a new guitars template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_ELECTRIC_GUITAR = "create_new_electric_guitar" {
            name: "Electric Guitar",
            description: "Create a new electric guitar template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_ACOUSTIC_GUITAR = "create_new_acoustic_guitar" {
            name: "Acoustic Guitar",
            description: "Create a new acoustic guitar template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_KEYS = "create_new_keys" {
            name: "Keys",
            description: "Create a new keys template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_PIANO = "create_new_piano" {
            name: "Piano",
            description: "Create a new piano template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_ORGAN = "create_new_organ" {
            name: "Organ",
            description: "Create a new organ template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_ELECTRIC_KEYS = "create_new_electric_keys" {
            name: "Electric Keys",
            description: "Create a new electric keys template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_SYNTHS = "create_new_synths" {
            name: "Synths",
            description: "Create a new synths template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_SYNTH_LEAD = "create_new_synth_lead" {
            name: "Synth Lead",
            description: "Create a new synth lead template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_SYNTH_PAD = "create_new_synth_pad" {
            name: "Synth Pad",
            description: "Create a new synth pad template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_SYNTH_ARP = "create_new_synth_arp" {
            name: "Synth Arp",
            description: "Create a new synth arp template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_HORNS = "create_new_horns" {
            name: "Horns",
            description: "Create a new horns template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_TRUMPET = "create_new_trumpet" {
            name: "Trumpet",
            description: "Create a new trumpet template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_TROMBONE = "create_new_trombone" {
            name: "Trombone",
            description: "Create a new trombone template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_SAXOPHONE = "create_new_saxophone" {
            name: "Saxophone",
            description: "Create a new saxophone template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_HARMONICA = "create_new_harmonica" {
            name: "Harmonica",
            description: "Create a new harmonica template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_STRINGS = "create_new_strings" {
            name: "Strings",
            description: "Create a new strings template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_VOCALS = "create_new_vocals" {
            name: "Vocals",
            description: "Create a new vocals template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_LEAD_VOCALS = "create_new_lead_vocals" {
            name: "Lead Vocals",
            description: "Create a new lead vocals template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_BACKGROUND_VOCALS = "create_new_background_vocals" {
            name: "Background Vocals",
            description: "Create a new background vocals template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_CHOIR = "create_new_choir" {
            name: "Choir",
            description: "Create a new choir template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_ORCHESTRA = "create_new_orchestra" {
            name: "Orchestra",
            description: "Create a new orchestra template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_SFX = "create_new_sfx" {
            name: "SFX",
            description: "Create a new SFX template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_GUIDE = "create_new_guide" {
            name: "Guide",
            description: "Create a new guide template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_REFERENCE = "create_new_reference" {
            name: "Reference",
            description: "Create a new reference template group",
            category: General,
            group: "Create",
        }
        CREATE_NEW_STEM_SPLIT = "create_new_stem_split" {
            name: "Stem Split",
            description: "Create a new stem split template group",
            category: General,
            group: "Create",
        }

    }
}
