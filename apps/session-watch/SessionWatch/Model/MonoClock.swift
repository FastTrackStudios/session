// The watch's clock: the one every time in a guide feed is in.

import Darwin

/// The continuous monotonic clock, microseconds.
///
/// The phone learns this clock's offset by pinging (the watch answers with
/// a reading of it) and sends every beat already converted into it, so the
/// watch estimates nothing. Continuous — it keeps counting while the watch
/// dozes — so a stamp from before a sleep and one from after still compare.
enum MonoClock {
    static func nowMicros() -> Double {
        Double(clock_gettime_nsec_np(CLOCK_MONOTONIC_RAW)) / 1_000
    }
}
