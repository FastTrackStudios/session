//! Offline batch tool: convert an `.RPP`'s markers/regions into the FTS
//! lane system (see `session::keyflow::offline::auto_organize_regions`) —
//! a whole album can be run in one shot without opening REAPER.
//!
//! ```text
//! convert-markers <in.rpp>... [-o <out.rpp>]
//! ```
//!
//! Without `-o`, each input is rewritten in place (the conversion is
//! idempotent, so re-running it is always safe). `-o` only accepts a
//! single input, matching `dynamic-template --apply-buses`'s convention.

use std::env;

use dawfile_reaper::types::RppSerialize;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        std::process::exit(2);
    }

    let mut output: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "-o" {
            output = Some(iter.next().unwrap_or_else(|| {
                eprintln!("-o requires a path");
                std::process::exit(2);
            }));
        } else {
            inputs.push(arg);
        }
    }

    if inputs.is_empty() {
        print_usage();
        std::process::exit(2);
    }
    if output.is_some() && inputs.len() > 1 {
        eprintln!("-o only accepts a single input");
        std::process::exit(2);
    }

    let mut failed = false;
    for input in &inputs {
        let out = output.clone().unwrap_or_else(|| input.clone());
        if let Err(err) = convert_one(input, &out) {
            eprintln!("error: {input}: {err}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

fn convert_one(input: &str, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut project = dawfile_reaper::io::read_project(input)?;
    let name = std::path::Path::new(input)
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("Song");

    session::keyflow::offline::auto_organize_regions(&mut project, name)?;

    std::fs::write(output, project.to_rpp_string())?;
    println!("{input} → {output}");
    Ok(())
}

fn print_usage() {
    eprintln!(
        "usage: convert-markers <in.rpp>... [-o <out.rpp>]\n\n\
         Converts plain REAPER markers/regions into the FTS lane system:\n\
         section regions recolored via the section-color taxonomy and pinned\n\
         to the Sections lane, a whole-song region added to the Song lane,\n\
         stray point markers swept up, non-FTS lanes hidden.\n\n\
         Without -o, each input is rewritten in place (idempotent — safe to\n\
         re-run). -o only accepts a single input."
    );
}
