//! Parity dump — mirrors reference-data/css-orchestrator/test/test_engine.lua
//! output so the Rust port can be diffed against the Lua engine.
//!
//! cargo run -p keyflow-orchestra --example dump -- <score> <part-substr> [profile]

use keyflow_orchestra::{Config, ProfileKind, process_part};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("score path");
    let part_sub = args.next().expect("part name substring");
    let profile = args.next().unwrap_or_else(|| "strings".into());

    let score = keyflow_orchestra::score::load(&path).expect("parse");
    println!("Parsed {} parts", score.parts.len());
    let part = score
        .parts
        .iter()
        .find(|p| p.name.to_lowercase().contains(&part_sub.to_lowercase()))
        .expect("part not found");

    let mut cfg = Config::default();
    cfg.profile = match profile.as_str() {
        "strings" => ProfileKind::Strings,
        "woodwinds" => ProfileKind::Woodwinds,
        "brass" => ProfileKind::Brass,
        "brass_trumpet" => ProfileKind::BrassTrumpet,
        "harp" => ProfileKind::Harp,
        _ => ProfileKind::Generic,
    };
    let res = process_part(part, &cfg);

    let channels: Vec<String> = res.channels.iter().map(|c| c.to_string()).collect();
    println!(
        "=== {} [profile={}] ===\n  notes={}  ccs={}  channels={{{}}}  span QN {:.2}..{:.2}",
        part.name,
        profile,
        res.notes.len(),
        res.ccs.len(),
        channels.join(","),
        res.start_qn,
        res.end_qn
    );
    println!("  first notes (startQN, end, ch, pitch, vel, artic, scriptOffsetMs):");
    for n in res.notes.iter().take(14) {
        println!(
            "    {:8.3} -> {:7.3}  ch{:<2}  p{:<3} v{:<3}  {:<13} lead={:+}ms",
            n.start_qn, n.end_qn, n.chan, n.pitch, n.vel, n.artic, n.lead_ms as i64
        );
    }
    let mut hist: std::collections::BTreeMap<u8, usize> = Default::default();
    for n in &res.notes {
        *hist.entry(n.chan).or_default() += 1;
    }
    let hs: Vec<String> = hist.iter().map(|(c, k)| format!("ch{c}={k}")).collect();
    println!("  channel note counts: {}", hs.join("  "));

    if let Some(&ch) = res.channels.first() {
        for cc in [1u8, 2, 58] {
            let mut line = format!("  CC{cc} (ch{ch}):");
            let mut count = 0;
            for e in &res.ccs {
                if e.cc == cc && e.chan == ch {
                    line.push_str(&format!(" {:.2}={}", e.qn, e.val));
                    count += 1;
                    if count >= 16 {
                        break;
                    }
                }
            }
            println!("{line}");
        }
    }
}
