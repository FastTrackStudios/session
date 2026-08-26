//! Convert a `.musx` file to MusicXML on stdout.
//!
//! Usage: `cargo run -p keyflow-musx --example convert -- <in.musx> [out.musicxml]`

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: convert <in.musx> [out.musicxml]");
    let musx = std::fs::read(&input).expect("read input");
    let musicxml = keyflow_musx::musx_to_musicxml(&musx).expect("convert");
    match args.next() {
        Some(out) => std::fs::write(out, musicxml).expect("write output"),
        None => print!("{musicxml}"),
    }
}
