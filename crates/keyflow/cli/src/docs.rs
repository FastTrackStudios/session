//! `kf docs` — render ` ```kf ` fenced blocks in a markdown content tree to
//! inline SVG, writing a generated mirror tree that a stock dodeca (`ddc`) build
//! consumes.
//!
//! This keeps the docs toolchain decoupled from dodeca: authors write ` ```kf `
//! blocks in the source content; this pass rewrites each one to
//! `<figure class="kf-chart">…svg…</figure>` in a parallel output tree, and
//! dodeca builds that. Nothing in dodeca changes — it passes the raw inline SVG
//! through like any other HTML in markdown.
//!
//! The actual chart→SVG rendering is injected as a closure (`render`) so this
//! module stays free of the engraver: the caller passes a function producing a
//! content-cropped, **font-less** SVG (`Some(svg)`), or `None` when the body
//! doesn't parse / is empty. `font_css` is the `@font-face` block injected once
//! per chart-bearing page so the engraving fonts load a single time rather than
//! being embedded in every chart.

use std::path::Path;
use std::time::SystemTime;

/// Chart-text → font-less SVG. `None` means "couldn't render" (parse error or
/// empty), in which case the block is left as its original fenced source.
pub type RenderFn<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// Filename of the shared engraving-font stylesheet written into the output
/// tree, and the URL chart pages link it at. Marked `stable_assets` in
/// `.config/dodeca.styx` so dodeca serves it verbatim at this exact path (no
/// cache-busting). It's CSS, not a font file, so dodeca's font subsetter never
/// touches it — the SMuFL glyphs stay whole.
const FONT_CSS_NAME: &str = "kf-fonts.css";

/// `<link>` injected once into each chart-bearing page, in place of inlining the
/// multi-MB `@font-face` block per page. The browser fetches `kf-fonts.css` a
/// single time and caches it across the whole site.
const FONT_CSS_LINK: &str = "<link rel=\"stylesheet\" href=\"/kf-fonts.css\">";

/// Render every `.md` under `in_dir` into `out_dir`, transforming ` ```kf `
/// blocks; copy every other file verbatim. The output tree is rebuilt from
/// scratch each call (so deletions in the source propagate). Returns the number
/// of charts rendered.
pub fn build(
    in_dir: &Path,
    out_dir: &Path,
    render: &RenderFn<'_>,
    font_css: &str,
) -> Result<usize, String> {
    if !in_dir.is_dir() {
        return Err(format!("input is not a directory: {}", in_dir.display()));
    }

    // Write the shared font stylesheet once. Chart pages link it (see
    // `FONT_CSS_LINK`) instead of each inlining the same ~4MB of base64, so the
    // browser caches the fonts a single time site-wide.
    //
    // dodeca only serves files from its `static/` dir (a SIBLING of the content
    // dir, `content_dir.parent()/static`) — non-`.md` files inside the content
    // tree are ignored. So the stylesheet goes there, and is served at the
    // top-level `/kf-fonts.css` that `FONT_CSS_LINK` points to.
    let static_dir = out_dir.parent().unwrap_or(out_dir).join("static");
    write_file(&static_dir.join(FONT_CSS_NAME), font_css.as_bytes())?;

    // Write the mirror tree IN PLACE (overwrite files, don't delete the dir).
    // `ddc serve` keeps inotify watches on the files under `out_dir`; wiping and
    // recreating the tree invalidates those watches, so live reload stops until
    // the server is restarted. Overwriting in place keeps the watches alive.
    // (Source deletions leave orphans in the mirror — rare; clear `out_dir`
    // manually if it matters.)
    let mut rendered = 0;
    walk(in_dir, in_dir, out_dir, render, &mut rendered)?;
    Ok(rendered)
}

fn walk(
    root: &Path,
    dir: &Path,
    out_root: &Path,
    render: &RenderFn<'_>,
    rendered: &mut usize,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let out_path = out_root.join(rel);

        if path.is_dir() {
            walk(root, &path, out_root, render, rendered)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let src = std::fs::read_to_string(&path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let (transformed, n) = transform_markdown(&src, render);
            *rendered += n;
            write_file(&out_path, transformed.as_bytes())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&path, &out_path).map_err(|e| format!("copy {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Transform one markdown document: render its ` ```kf ` blocks and, if any
/// rendered, inject the shared font `<link>` once after the frontmatter.
/// Returns the new document and the number of charts rendered.
fn transform_markdown(src: &str, render: &RenderFn<'_>) -> (String, usize) {
    let lines: Vec<&str> = src.lines().collect();

    // Preserve a leading `+++`/`---` frontmatter block untouched — the font
    // `<style>` must go AFTER it, or dodeca's frontmatter parser chokes.
    let mut prefix = String::new();
    let mut idx = 0;
    if let Some(&first) = lines.first() {
        if first == "+++" || first == "---" {
            let delim = first;
            prefix.push_str(first);
            prefix.push('\n');
            idx = 1;
            while idx < lines.len() {
                let line = lines[idx];
                prefix.push_str(line);
                prefix.push('\n');
                idx += 1;
                if line == delim {
                    break;
                }
            }
        }
    }

    let (body, count) = transform_fences(&lines[idx..], render);

    let mut out = prefix;
    if count > 0 {
        out.push('\n');
        out.push_str(FONT_CSS_LINK);
        out.push_str("\n\n");
    }
    out.push_str(&body);
    (out, count)
}

/// Scan markdown lines, replacing each ` ```kf ` fenced block with its rendered
/// SVG and each ` ```kf-src ` block with syntax-highlighted HTML. A ` ```kf `
/// block whose body fails to render (parse error / empty) is left as the
/// original fenced code so a typo shows the source, not a hard failure.
fn transform_fences(lines: &[&str], render: &RenderFn<'_>) -> (String, usize) {
    let mut out = String::new();
    let mut count = 0;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if let Some(fence) = open_fence(line) {
            if fence.lang == "kf-src" {
                // Collect the block body up to the matching closing fence,
                // stripping the fence's own indentation (CommonMark: content
                // of an indented fence is dedented by the fence indent).
                let indent = line.len() - line.trim_start_matches(' ').len();
                let mut j = i + 1;
                let mut body = Vec::new();
                while j < lines.len() && !is_close_fence(lines[j], &fence) {
                    body.push(dedent(lines[j], indent));
                    j += 1;
                }
                out.push_str(&line[..indent]);
                out.push_str(&highlight_block(&body));
                i = j + 1; // skip past the closing fence
                continue;
            }
            if fence.lang == "kf" {
                // Collect the block body up to the matching closing fence.
                let mut j = i + 1;
                let mut chart = String::new();
                while j < lines.len() && !is_close_fence(lines[j], &fence) {
                    chart.push_str(lines[j]);
                    chart.push('\n');
                    j += 1;
                }
                let close_idx = j; // index of the closing fence (or lines.len())

                match render(&chart) {
                    Some(svg) if !svg.trim().is_empty() => {
                        // Inline SVG in an HTML page must start at `<svg>`: a
                        // leading `<?xml …?>` prolog / DOCTYPE is parsed as a
                        // bogus comment and the browser drops the chart. Strip
                        // anything before the first `<svg`.
                        let inline = svg.find("<svg").map_or(svg.as_str(), |p| &svg[p..]);
                        out.push_str("<figure class=\"kf-chart\">\n");
                        out.push_str(inline);
                        out.push_str("\n</figure>\n");
                        count += 1;
                    }
                    _ => {
                        // Fallback: reproduce the original block verbatim.
                        for line in &lines[i..=close_idx.min(lines.len().saturating_sub(1))] {
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
                i = close_idx + 1; // skip past the closing fence
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
        i += 1;
    }
    (out, count)
}

/// A recognized opening code fence: the run of fence characters and the first
/// info-string token (the language).
struct Fence {
    ch: char,
    len: usize,
    lang: String,
}

/// Recognize an opening fence line (` ``` ` or `~~~`, up to 3 leading spaces of
/// indent per CommonMark) and pull out its fence run + language token.
fn open_fence(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None; // 4+ spaces = indented code, not a fence
    }
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|&c| c == ch).count();
    if len < 3 {
        return None;
    }
    let info = trimmed[len..].trim();
    // Backtick info strings may not contain backticks (CommonMark).
    if ch == '`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next().unwrap_or("").to_string();
    Some(Fence { ch, len, lang })
}

/// Strip up to `n` leading spaces from `line` (CommonMark fence dedent).
fn dedent(line: &str, n: usize) -> &str {
    let strip = line.chars().take(n).take_while(|&c| c == ' ').count();
    &line[strip..]
}

/// Render a ` ```kf-src ` block body as syntax-highlighted HTML.
///
/// Produces the same wrapper markup dodeca emits for fenced code (so the site
/// stylesheet and copy-button JS apply unchanged), with a `kf-src` marker class
/// and a `kf` language header. Highlighting is the editor's own line
/// highlighter; spans carry CSS classes only (`kf-root`, `kf-quality`, …) and
/// the palette lives in the site stylesheet.
///
/// The whole block is emitted as a SINGLE line, with source newlines encoded
/// as `&#10;` inside the `<code>`. Raw HTML in markdown is a CommonMark
/// "type 6" block that a blank line terminates — one physical line sidesteps
/// that entirely, and lets the block sit inside list items at any indent.
fn highlight_block(body: &[&str]) -> String {
    use keyflow::text::highlighting::{Highlighter, Renderer};

    let mut out = String::from(
        "<div class=\"code-block kf-src\"><div class=\"code-header\">kf</div><pre><code>",
    );
    for (n, line) in body.iter().enumerate() {
        if n > 0 {
            out.push_str("&#10;");
        }
        if !line.trim().is_empty() {
            let spans = Highlighter::highlight_line(line);
            out.push_str(&Renderer::to_html_classes(line, &spans));
        }
    }
    out.push_str("</code></pre><button class=\"copy-btn\">Copy</button></div>\n");
    out
}

/// Whether `line` closes an open `fence`: only fence characters of the same
/// kind, at least as long as the opener, with nothing after.
fn is_close_fence(line: &str, fence: &Fence) -> bool {
    let trimmed = line.trim_start_matches(' ').trim_end();
    if line.len() - line.trim_start_matches(' ').len() > 3 {
        return false;
    }
    !trimmed.is_empty()
        && trimmed.chars().all(|c| c == fence.ch)
        && trimmed.chars().count() >= fence.len
}

/// Cheap fingerprint of a directory tree for `--watch`: (file count, newest
/// mtime). Any edit bumps an mtime; any add/remove changes the count.
pub fn tree_fingerprint(dir: &Path) -> (usize, u128) {
    fn nanos(t: SystemTime) -> u128 {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
    fn visit(dir: &Path, count: &mut usize, newest: &mut u128) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, count, newest);
            } else if let Ok(meta) = entry.metadata() {
                *count += 1;
                if let Ok(m) = meta.modified() {
                    *newest = (*newest).max(nanos(m));
                }
            }
        }
    }
    let mut count = 0;
    let mut newest = 0;
    visit(dir, &mut count, &mut newest);
    (count, newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub renderer: echoes the chart body inside a fake `<svg>` so the
    /// transform logic can be tested without the engraver. Returns `None` for a
    /// body containing "BAD" to exercise the fallback path.
    fn stub(body: &str) -> Option<String> {
        if body.contains("BAD") {
            None
        } else {
            Some(format!("<svg data-chart=\"{}\"></svg>", body.trim()))
        }
    }

    #[test]
    fn renders_kf_block_and_injects_fonts_after_frontmatter() {
        let md = "+++\ntitle = \"X\"\n+++\n\nIntro\n\n```kf\n1 4 6 5\n```\n\nOutro\n";
        let (out, n) = transform_markdown(md, &stub);
        assert_eq!(n, 1, "one chart rendered");
        assert!(
            out.starts_with("+++\ntitle = \"X\"\n+++\n"),
            "frontmatter kept on top"
        );
        assert!(
            out.contains("href=\"/kf-fonts.css\""),
            "font link injected once"
        );
        assert!(
            out.contains("<figure class=\"kf-chart\">\n<svg"),
            "svg figure emitted"
        );
        assert!(!out.contains("```kf"), "no kf fence remains: {out:.200}");
        assert!(
            out.contains("Intro") && out.contains("Outro"),
            "prose preserved"
        );
    }

    #[test]
    fn non_kf_fences_and_plain_pages_untouched() {
        let md = "Some text\n\n```rust\nfn main() {}\n```\n";
        let (out, n) = transform_markdown(md, &stub);
        assert_eq!(n, 0);
        assert!(out.contains("```rust"), "other languages pass through");
        assert!(
            !out.contains("kf-fonts.css"),
            "no font link when no kf block"
        );
    }

    #[test]
    fn unrenderable_kf_block_falls_back_to_source() {
        let md = "```kf\nBAD chart\n```\n";
        let (out, n) = transform_markdown(md, &stub);
        assert_eq!(n, 0, "render returned None");
        assert!(
            out.contains("```kf"),
            "fallback keeps the original fence: {out}"
        );
        assert!(out.contains("BAD chart"), "fallback keeps the body");
    }

    #[test]
    fn strips_xml_prolog_so_svg_embeds_inline() {
        // The engraver emits a standalone SVG with an `<?xml …?>` prolog; inline
        // in HTML that prolog kills rendering, so the transform must drop it.
        let render = |body: &str| {
            Some(format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg>{}</svg>",
                body.trim()
            ))
        };
        let md = "```kf\n1 4 6 5\n```\n";
        let (out, n) = transform_markdown(md, &render);
        assert_eq!(n, 1);
        assert!(!out.contains("<?xml"), "xml prolog must be stripped: {out}");
        assert!(
            out.contains("<figure class=\"kf-chart\">\n<svg>"),
            "figure starts at <svg>"
        );
    }

    #[test]
    fn kf_src_fence_becomes_highlighted_html() {
        let md = "Before\n\n```kf-src\nVS 4\nGm7 | C7\n\nCH 4\n```\n\nAfter\n";
        let (out, n) = transform_markdown(md, &stub);
        assert_eq!(n, 0, "kf-src renders no chart");
        assert!(
            out.contains("<div class=\"code-block kf-src\">"),
            "wrapper emitted: {out}"
        );
        assert!(!out.contains("```"), "fence consumed");
        assert!(
            out.contains("class=\"kf-"),
            "highlight spans present: {out}"
        );
        // single-line emission: source newlines become &#10; entities, so a
        // blank body line can't terminate the CommonMark HTML block
        let html_line = out
            .lines()
            .find(|l| l.contains("code-block kf-src"))
            .unwrap();
        assert!(html_line.contains("&#10;&#10;"), "blank line encoded");
        assert!(html_line.ends_with("</div>"), "block is one physical line");
        assert!(
            !out.contains("kf-fonts.css"),
            "no font link for source-only blocks"
        );
    }

    #[test]
    fn kf_src_fence_indented_in_list_keeps_indent() {
        let md = "- item:\n  ```kf-src\n  m{ 1 2 3 4 }\n  ```\n- next\n";
        let (out, _) = transform_markdown(md, &stub);
        assert!(
            out.contains("\n  <div class=\"code-block kf-src\">"),
            "block keeps the fence's list indent: {out}"
        );
        assert!(
            out.contains("<pre><code>m<span"),
            "body dedented (no leading spaces) and highlighted: {out}"
        );
    }

    #[test]
    fn handles_tilde_fences_and_multiple_blocks() {
        let md = "~~~kf\n1 5\n~~~\n\ntext\n\n```kf\n4 1\n```\n";
        let (out, n) = transform_markdown(md, &stub);
        assert_eq!(n, 2, "both fence styles render");
        assert!(out.matches("kf-chart").count() == 2);
    }
}
