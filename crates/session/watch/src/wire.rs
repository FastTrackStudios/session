//! The bytes on the `WatchConnectivity` link: JSON, the shapes of
//! [`session_proto::watch`] — the watch decodes them with the `Codable`
//! structs `gen_watch_swift` generates from the same shapes.

use facet::Facet;

/// One message as JSON, or `None` (with a warning) if it cannot be.
#[must_use]
pub fn encode<'a, T: Facet<'a>>(message: &T) -> Option<Vec<u8>> {
    match facet_json::to_string(message) {
        Ok(json) => Some(json.into_bytes()),
        Err(e) => {
            tracing::warn!(watch.encode_error = %e, "watch relay: a message could not be encoded");
            None
        }
    }
}

/// One message from the watch; `None` if it is not one.
#[must_use]
pub fn decode<T: Facet<'static>>(bytes: &[u8]) -> Option<T> {
    let text = std::str::from_utf8(bytes).ok()?;
    facet_json::from_str::<T>(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use session_proto::watch::{WatchAccent, WatchBeat, WatchGuideFeed, WatchMessage, WatchPong};

    /// The field names and the enum's spelling are what the Swift side
    /// decodes: `snake_case` keys, a unit variant as its name.
    #[test]
    fn the_wire_is_what_swift_reads() {
        let feed = WatchGuideFeed {
            beats: vec![WatchBeat {
                at_us: 1.5,
                accent: WatchAccent::CountIn,
                ..WatchBeat::default()
            }],
            ..WatchGuideFeed::default()
        };
        let bytes = encode(&WatchMessage {
            ping: None,
            feed: Some(feed.clone()),
        })
        .unwrap();
        let json = String::from_utf8(bytes.clone()).unwrap();
        assert!(json.contains("\"at_us\":1.5"), "{json}");
        assert!(json.contains("\"accent\":\"CountIn\""), "{json}");
        assert!(json.contains("\"ping\":null"), "{json}");
        let back: WatchMessage = decode(&bytes).unwrap();
        assert_eq!(back.feed, Some(feed));
        let pong: WatchPong = decode(br#"{"sent_us":1,"received_us":2.5,"replied_us":3}"#).unwrap();
        assert!((pong.received_us - 2.5).abs() < f64::EPSILON);
    }
}
