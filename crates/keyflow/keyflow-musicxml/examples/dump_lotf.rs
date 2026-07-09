fn main() {
    let input = std::env::args().nth(1).unwrap_or_else(|| {
        "examples/png-project-charts/02 LORD OF THE FIGHT Master RS.musicxml".to_string()
    });
    let chart = keyflow_musicxml::import_file(&input).expect("import");
    for (i, s) in chart.sections.iter().enumerate() {
        let total: usize = s.tracks.iter().map(|t| t.measures.len()).sum();
        println!(
            "section[{i}] type={:?} name={:?} measures={total}",
            s.section.section_type, s.section.name
        );
        for (j, m) in s.measures().iter().enumerate() {
            let chords = m
                .chords
                .iter()
                .map(|c| c.full_symbol.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let text = m
                .staff_text
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" | ");
            println!(
                "  local={j:02} source={:?} chords=[{}] text=[{}] volta={:?} start={:?} end={:?}",
                m.source_measure_number,
                chords,
                text,
                m.volta_start.as_ref().map(|v| &v.numbers),
                m.start_repeat,
                m.end_repeat
            );
        }
    }
    println!("initial_time_signature={:?}", chart.initial_time_signature);
}
