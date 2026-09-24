// GENERATED — do not edit. Mirrors the facet shapes in
// crates/session/proto/src/watch.rs (the guide feed the iPhone relays).
// Regenerate: cargo run -p session-proto --example gen_watch_swift -- guide
//   > apps/session-watch/SessionWatch/Generated/WatchGuide.generated.swift

import Foundation

public enum WatchAccent: String, Codable, Equatable, Sendable {
    case beat = "Beat"
    case downbeat = "Downbeat"
    case countIn = "CountIn"
}

public struct WatchBeat: Codable, Equatable, Sendable {
    public var index: UInt32
    public var atUs: Double
    public var position: Double
    public var progress: Float
    public var bar: Int32
    public var beat: UInt32
    public var beatsPerBar: UInt32
    public var bpm: Float
    public var section: Int32
    public var sectionBar: UInt32
    public var sectionBars: UInt32
    public var count: UInt32
    public var cue: String
    public var accent: WatchAccent

    public init(
        index: UInt32,
        atUs: Double,
        position: Double,
        progress: Float,
        bar: Int32,
        beat: UInt32,
        beatsPerBar: UInt32,
        bpm: Float,
        section: Int32,
        sectionBar: UInt32,
        sectionBars: UInt32,
        count: UInt32,
        cue: String,
        accent: WatchAccent
    ) {
        self.index = index
        self.atUs = atUs
        self.position = position
        self.progress = progress
        self.bar = bar
        self.beat = beat
        self.beatsPerBar = beatsPerBar
        self.bpm = bpm
        self.section = section
        self.sectionBar = sectionBar
        self.sectionBars = sectionBars
        self.count = count
        self.cue = cue
        self.accent = accent
    }

    enum CodingKeys: String, CodingKey {
        case index = "index"
        case atUs = "at_us"
        case position = "position"
        case progress = "progress"
        case bar = "bar"
        case beat = "beat"
        case beatsPerBar = "beats_per_bar"
        case bpm = "bpm"
        case section = "section"
        case sectionBar = "section_bar"
        case sectionBars = "section_bars"
        case count = "count"
        case cue = "cue"
        case accent = "accent"
    }
}

public struct WatchGuideFeed: Codable, Equatable, Sendable {
    public var revision: UInt64
    public var run: UInt64
    public var setTitle: String
    public var songTitle: String
    public var songIndex: Int32
    public var songCount: UInt32
    public var playing: Bool
    public var clockLocked: Bool
    public var clockErrorUs: Double
    public var sections: [WatchSection]
    public var here: WatchBeat
    public var beats: [WatchBeat]

    public init(
        revision: UInt64,
        run: UInt64,
        setTitle: String,
        songTitle: String,
        songIndex: Int32,
        songCount: UInt32,
        playing: Bool,
        clockLocked: Bool,
        clockErrorUs: Double,
        sections: [WatchSection],
        here: WatchBeat,
        beats: [WatchBeat]
    ) {
        self.revision = revision
        self.run = run
        self.setTitle = setTitle
        self.songTitle = songTitle
        self.songIndex = songIndex
        self.songCount = songCount
        self.playing = playing
        self.clockLocked = clockLocked
        self.clockErrorUs = clockErrorUs
        self.sections = sections
        self.here = here
        self.beats = beats
    }

    enum CodingKeys: String, CodingKey {
        case revision = "revision"
        case run = "run"
        case setTitle = "set_title"
        case songTitle = "song_title"
        case songIndex = "song_index"
        case songCount = "song_count"
        case playing = "playing"
        case clockLocked = "clock_locked"
        case clockErrorUs = "clock_error_us"
        case sections = "sections"
        case here = "here"
        case beats = "beats"
    }
}

public struct WatchMessage: Codable, Equatable, Sendable {
    public var ping: WatchPing?
    public var feed: WatchGuideFeed?

    public init(
        ping: WatchPing?,
        feed: WatchGuideFeed?
    ) {
        self.ping = ping
        self.feed = feed
    }

    enum CodingKeys: String, CodingKey {
        case ping = "ping"
        case feed = "feed"
    }
}

public struct WatchPing: Codable, Equatable, Sendable {
    public var sentUs: Double

    public init(
        sentUs: Double
    ) {
        self.sentUs = sentUs
    }

    enum CodingKeys: String, CodingKey {
        case sentUs = "sent_us"
    }
}

public struct WatchPong: Codable, Equatable, Sendable {
    public var sentUs: Double
    public var receivedUs: Double
    public var repliedUs: Double

    public init(
        sentUs: Double,
        receivedUs: Double,
        repliedUs: Double
    ) {
        self.sentUs = sentUs
        self.receivedUs = receivedUs
        self.repliedUs = repliedUs
    }

    enum CodingKeys: String, CodingKey {
        case sentUs = "sent_us"
        case receivedUs = "received_us"
        case repliedUs = "replied_us"
    }
}

public struct WatchSection: Codable, Equatable, Sendable {
    public var name: String
    public var start: Double
    public var end: Double
    public var color: UInt32
    public var bars: UInt32

    public init(
        name: String,
        start: Double,
        end: Double,
        color: UInt32,
        bars: UInt32
    ) {
        self.name = name
        self.start = start
        self.end = end
        self.color = color
        self.bars = bars
    }

    enum CodingKeys: String, CodingKey {
        case name = "name"
        case start = "start"
        case end = "end"
        case color = "color"
        case bars = "bars"
    }
}

