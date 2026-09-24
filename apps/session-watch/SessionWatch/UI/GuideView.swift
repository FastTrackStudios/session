// The guide, glanceable: the section in big type, the bar within it, the
// beat as dots, what comes next, and the whole song as a coloured strip.
// Everything shown is a field of the beat last sounded — computed on the
// phone by the desktop's own guide code; nothing is worked out here.

import SwiftUI

struct GuideView: View {
    @Environment(GuideStore.self) private var store

    var body: some View {
        if let feed = store.feed, let beat = store.beat {
            Guide(feed: feed, beat: beat, stale: store.stale)
        } else {
            VStack(spacing: 6) {
                Image(systemName: "iphone.gen3.radiowaves.left.and.right")
                    .font(.title2)
                    .foregroundStyle(.secondary)
                Text("Open Session on your iPhone and join a set")
                    .font(.footnote)
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.secondary)
            }
            .padding()
        }
    }
}

private struct Guide: View {
    let feed: WatchGuideFeed
    let beat: WatchBeat
    let stale: Bool

    private var section: WatchSection? {
        feed.sections.indices.contains(Int(beat.section)) ? feed.sections[Int(beat.section)] : nil
    }
    private var next: WatchSection? {
        let i = Int(beat.section) + 1
        return feed.sections.indices.contains(i) ? feed.sections[i] : nil
    }
    private var countingIn: Bool { beat.accent == .countIn && feed.playing }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            header
            if countingIn {
                CountIn(beat: beat, into: next?.name ?? feed.songTitle)
            } else {
                Text(section?.name ?? feed.songTitle)
                    .font(.system(size: 26, weight: .heavy, design: .rounded))
                    .foregroundStyle(section?.tint ?? .primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                HStack(alignment: .firstTextBaseline) {
                    if beat.sectionBars > 0 {
                        Text("\(beat.sectionBar)")
                            .font(.system(size: 22, weight: .bold, design: .rounded).monospacedDigit())
                        + Text("/\(beat.sectionBars)")
                            .font(.system(size: 14, weight: .semibold, design: .rounded).monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text("♩\(Int(beat.bpm.rounded()))")
                        .font(.system(size: 14, weight: .semibold, design: .rounded).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                BeatDots(beat: beat, playing: feed.playing)
            }
            Spacer(minLength: 0)
            NextUp(beat: beat, next: next)
            SongStrip(feed: feed, beat: beat)
        }
        .padding(.horizontal, 4)
        .opacity(stale ? 0.45 : 1)
    }

    private var header: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(stale ? .gray : feed.clockLocked ? .green : .orange)
                .frame(width: 6, height: 6)
            Text(feed.songTitle)
                .font(.system(size: 12, weight: .semibold))
                .lineLimit(1)
            if feed.songCount > 1 {
                Text("\(feed.songIndex + 1)/\(feed.songCount)")
                    .font(.system(size: 11).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if !feed.playing {
                Image(systemName: "stop.fill").font(.system(size: 9)).foregroundStyle(.secondary)
            }
        }
        // The corner clock is always drawn top-right; keep clear of it.
        .padding(.trailing, 36)
    }
}

/// The count into the song: the number, huge.
private struct CountIn: View {
    let beat: WatchBeat
    let into: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The guide's spoken count in yellow; the bar before it (which
            // the guide leaves silent for the announcement) as plain beats.
            Text("\(beat.count > 0 ? beat.count : beat.beat)")
                .font(.system(size: 64, weight: .black, design: .rounded).monospacedDigit())
                .foregroundStyle(beat.count > 0 ? Color.yellow : Color.white.opacity(0.35))
                .contentTransition(.numericText())
            Text(beat.cue.isEmpty ? "into \(into)" : beat.cue)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

/// One dot a beat of the bar; the one sounding filled, beat one larger.
private struct BeatDots: View {
    let beat: WatchBeat
    let playing: Bool

    var body: some View {
        HStack(spacing: 5) {
            ForEach(1...Int(max(beat.beatsPerBar, 1)), id: \.self) { n in
                let on = playing && n == Int(beat.beat)
                Circle()
                    .fill(on ? (n == 1 ? Color.orange : Color.white) : Color.white.opacity(0.18))
                    .frame(width: n == 1 ? 13 : 10, height: n == 1 ? 13 : 10)
            }
            if beat.count > 0 {
                Text("\(beat.count)")
                    .font(.system(size: 13, weight: .black, design: .rounded))
                    .foregroundStyle(.yellow)
            }
        }
    }
}

/// What comes next, and how soon.
private struct NextUp: View {
    let beat: WatchBeat
    let next: WatchSection?

    var body: some View {
        if let next {
            let bars = Int(beat.sectionBars) - Int(beat.sectionBar) + 1
            HStack(spacing: 4) {
                Image(systemName: "arrow.right").font(.system(size: 10, weight: .bold))
                Text(next.name)
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(next.tint)
                    .lineLimit(1)
                Spacer(minLength: 0)
                if bars > 0 {
                    Text(bars == 1 ? "this bar" : "in \(bars)")
                        .font(.system(size: 12).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// The song as its sections, with the playhead.
private struct SongStrip: View {
    let feed: WatchGuideFeed
    let beat: WatchBeat

    var body: some View {
        GeometryReader { geo in
            let start = feed.sections.first?.start ?? 0
            let end = max(feed.sections.last?.end ?? 1, start + 0.001)
            let x = { (t: Double) in CGFloat((t - start) / (end - start)) * geo.size.width }
            ZStack(alignment: .leading) {
                ForEach(Array(feed.sections.enumerated()), id: \.offset) { i, s in
                    Rectangle()
                        .fill(s.tint.opacity(i == Int(beat.section) ? 1 : 0.45))
                        .frame(width: max(1, x(s.end) - x(s.start) - 1), height: 6)
                        .offset(x: x(s.start))
                }
                Rectangle()
                    .fill(.white)
                    .frame(width: 2, height: 10)
                    .offset(x: CGFloat(beat.progress) * geo.size.width - 1)
            }
            .frame(height: 10)
        }
        .frame(height: 10)
    }
}
