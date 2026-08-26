use super::*;

fn parse_ok(src: &str) -> Document {
    parse(src).expect("parse failed")
}

#[test]
fn directives_basic() {
    let doc = parse_ok("{title: Twinkle}\n{artist: Trad}\n");
    assert_eq!(doc.title(), Some("Twinkle"));
    assert_eq!(doc.artist(), Some("Trad"));
}

#[test]
fn aliases_canonicalize() {
    let doc = parse_ok("{t: Hi}\n{st: There}\n{c: see this}\n");
    assert!(matches!(
        doc.lines[0],
        Line::Directive(Directive {
            kind: DirectiveKind::Title(_),
            ..
        })
    ));
    assert!(matches!(
        doc.lines[1],
        Line::Directive(Directive {
            kind: DirectiveKind::Subtitle(_),
            ..
        })
    ));
    assert!(matches!(
        doc.lines[2],
        Line::Directive(Directive {
            kind: DirectiveKind::Comment(_),
            ..
        })
    ));
}

#[test]
fn lyric_with_chords_and_annotation() {
    let doc = parse_ok("[C]Twinkle [*pp][F]little [C]star");
    let Line::Lyric { chunks, .. } = &doc.lines[0] else {
        panic!("expected lyric line, got {:?}", doc.lines[0]);
    };
    // Parser emits one chunk per `[…]` marker, so `[*pp][F]little` produces
    // two chunks: one annotation-only with empty text, one chord+text.
    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[0].chord.as_deref(), Some("C"));
    assert_eq!(chunks[0].text, "Twinkle ");
    assert!(chunks[1].annotation.is_some());
    assert!(chunks[1].text.is_empty());
    assert_eq!(chunks[2].chord.as_deref(), Some("F"));
    assert_eq!(chunks[2].text, "little ");
    assert_eq!(chunks[3].chord.as_deref(), Some("C"));
    assert_eq!(chunks[3].text, "star");
}

#[test]
fn environments_chorus_recall_and_verse() {
    let doc = parse_ok(
        "{start_of_verse: V1}\n[C]hello\n{end_of_verse}\n{soc}\n[F]bye\n{eoc}\n{chorus}\n",
    );
    let sections = doc.sections();
    let envs: Vec<_> = sections
        .iter()
        .map(|s| s.environment.as_ref().map(|e| e.as_str()))
        .collect();
    assert!(envs.contains(&Some("verse")));
    assert!(envs.contains(&Some("chorus")));

    // {chorus} (recall) is a directive line, not an environment open.
    let recalls: Vec<_> = doc
        .directives()
        .filter(|d| matches!(d.kind, DirectiveKind::ChorusRecall { .. }))
        .collect();
    assert_eq!(recalls.len(), 1);
}

#[test]
fn meta_dispatch_and_aliases() {
    let doc = parse_ok("{album: My LP}\n{tempo: 120}\n{meta: composer Bach}\n");
    let metas: Vec<_> = doc
        .directives()
        .filter_map(|d| match &d.kind {
            DirectiveKind::Meta(m) => Some((m.item.as_str(), m.value.as_str())),
            _ => None,
        })
        .collect();
    assert!(metas.contains(&("album", "My LP")));
    assert!(metas.contains(&("tempo", "120")));
    assert!(metas.contains(&("composer", "Bach")));
}

#[test]
fn plain_metadata_and_section_headings_parse_as_directives() {
    let doc = parse_ok(
        "\
Title: Build My Life
Artist: Housefires
Key: [G]
Original Key: G
Book: Camp 2022

Verse 1:
[G]Worthy of every so[C/G]ng

Chorus:
[Cmaj9]Holy, there is no one [Am7]like You
",
    );

    assert_eq!(doc.title(), Some("Build My Life"));
    assert_eq!(doc.artist(), Some("Housefires"));
    assert_eq!(doc.key(), Some("G"));

    let metas: Vec<_> = doc
        .directives()
        .filter_map(|d| match &d.kind {
            DirectiveKind::Meta(m) => Some((m.item.as_str(), m.value.as_str())),
            _ => None,
        })
        .collect();
    assert!(metas.contains(&("original_key", "G")));
    assert!(metas.contains(&("book", "Camp 2022")));

    let starts: Vec<_> = doc
        .directives()
        .filter_map(|d| match &d.kind {
            DirectiveKind::StartOfEnvironment { env, label } => {
                Some((env.as_str(), label.as_deref().unwrap_or("")))
            }
            _ => None,
        })
        .collect();
    assert_eq!(starts[0], ("verse", "Verse 1 sync=lines"));
    assert_eq!(starts[1], ("chorus", "Chorus sync=lines"));
}

#[test]
fn line_continuation_joins_next_line() {
    // Continuation strips leading whitespace on the joined line.
    let doc = parse_ok("[C]Twinkle, twinkle, \\\n  [F]little [C]star\n");
    let Line::Lyric { chunks, .. } = &doc.lines[0] else {
        panic!("expected joined lyric line");
    };
    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let joined = texts.join("");
    assert!(joined.contains("Twinkle, twinkle,"));
    assert!(joined.contains("little"));
}

#[test]
fn unicode_escapes_expand() {
    // é = 'é'
    let doc = parse_ok("{title: Caf\\u00e9}\n");
    assert_eq!(doc.title(), Some("Café"));
}

#[test]
fn hash_comment_is_ignored_as_lyric() {
    let doc = parse_ok("# this is a comment\n[C]Hello\n");
    assert!(matches!(doc.lines[0], Line::HashComment { .. }));
    assert!(matches!(doc.lines[1], Line::Lyric { .. }));
}

#[test]
fn conditional_directive_captures_selector() {
    let doc = parse_ok("{title-en: Hello}\n");
    let Line::Directive(d) = &doc.lines[0] else {
        panic!("directive expected")
    };
    assert!(matches!(d.kind, DirectiveKind::Title(_)));
    assert_eq!(d.condition.as_deref(), Some("en"));
}

#[test]
fn define_chord_parses_frets_and_fingers() {
    let doc = parse_ok("{define: D base-fret 1 frets x x 0 2 3 2 fingers - - - 1 3 2}\n");
    let Line::Directive(d) = &doc.lines[0] else {
        panic!("directive expected")
    };
    let DirectiveKind::Define { def, is_define } = &d.kind else {
        panic!("define kind expected, got {:?}", d.kind);
    };
    assert!(*is_define);
    assert_eq!(def.name, "D");
    assert_eq!(def.base_fret, Some(1));
    assert_eq!(def.frets.as_ref().unwrap().len(), 6);
    assert_eq!(def.fingers.as_ref().unwrap().len(), 6);
}

#[test]
fn image_directive_parses_kv_pairs() {
    let doc = parse_ok("{image src=\"foo.png\" width=120 height=80 center}\n");
    let Line::Directive(d) = &doc.lines[0] else {
        panic!("directive expected")
    };
    let DirectiveKind::Image(kvs) = &d.kind else {
        panic!("image kind expected")
    };
    assert!(kvs.iter().any(|(k, v)| k == "src" && v == "foo.png"));
    assert!(kvs.iter().any(|(k, v)| k == "width" && v == "120"));
    assert!(kvs.iter().any(|(k, _)| k == "center"));
}

#[test]
fn unknown_directive_falls_through_to_custom() {
    let doc = parse_ok("{x_keyflow_link: keyflow-block-1}\n");
    let Line::Directive(d) = &doc.lines[0] else {
        panic!("directive")
    };
    let DirectiveKind::Custom { name, value } = &d.kind else {
        panic!("custom expected, got {:?}", d.kind);
    };
    assert_eq!(name, "x_keyflow_link");
    assert_eq!(value.as_deref(), Some("keyflow-block-1"));
}

#[test]
fn font_size_color_directives_classify_as_style() {
    let doc = parse_ok("{textsize: 14}\n{titlefont: Helvetica}\n{tabcolour: blue}\n");
    let styles: Vec<_> = doc
        .directives()
        .filter_map(|d| match &d.kind {
            DirectiveKind::Style { name, value } => Some((name.as_str(), value.as_str())),
            _ => None,
        })
        .collect();
    assert!(styles.iter().any(|(n, v)| *n == "textsize" && *v == "14"));
    assert!(styles
        .iter()
        .any(|(n, v)| *n == "titlefont" && *v == "Helvetica"));
    assert!(styles
        .iter()
        .any(|(n, v)| *n == "tabcolour" && *v == "blue"));
}
