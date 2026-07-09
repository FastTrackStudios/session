//! Syntax-highlighted editor for Keyflow notation.
//!
//! A transparent textarea overlaid on highlighted code display,
//! providing real-time syntax highlighting as the user types.

use crate::prelude::*;
use dioxus::dioxus_core::Task;
use keyflow::text::highlighting::{Highlighter, Renderer, Theme};
use keyflow::text::ide::{self, Severity};

/// Highlighted editor component.
///
/// Renders a textarea with a syntax-highlighted code display underneath.
/// The textarea is transparent so the highlighting shows through, and
/// scroll positions are synchronized between the two layers.
///
/// The textarea is kept **uncontrolled** for normal user input — the browser
/// manages cursor position and scroll state natively. The Dioxus `value` prop
/// is only set on the initial render; subsequent external changes (e.g. loading
/// an example) are pushed to the DOM via JS eval, which preserves cursor state.
#[component]
pub fn HighlightedEditor(
    /// The current source text.
    source: String,
    /// Callback when source text changes.
    on_change: EventHandler<String>,
) -> Element {
    let theme = use_memo(Theme::default_dark);

    // Local text state for immediate textarea responsiveness.
    // This updates on every keystroke so the user sees their typing instantly,
    // while the debounced on_change only fires after 150ms of inactivity.
    let mut local_text = use_signal(|| source.clone());

    // Track the source prop to detect external changes (e.g. example loaded).
    let mut last_source = use_signal(|| source.clone());
    // Track whether the textarea DOM needs a programmatic value update.
    let mut needs_dom_sync = use_signal(|| false);

    if *last_source.read() != source {
        last_source.set(source.clone());
        // Only push to textarea if the external change differs from the user's local text
        if *local_text.read() != source {
            local_text.set(source.clone());
            needs_dom_sync.set(true);
        }
    }

    // When an external source change arrives, push it to the textarea DOM via JS.
    // This avoids Dioxus re-setting the `value` attribute (which resets cursor position).
    use_effect(move || {
        if *needs_dom_sync.read() {
            let text = local_text.read().clone();
            // Escape for JS string literal (backslash, backtick, ${)
            let escaped = text
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace("${", "\\${");
            let js = format!(
                r#"(function() {{
                    var ta = document.getElementById('editor-textarea');
                    if (ta) {{ ta.value = `{escaped}`; }}
                }})();"#
            );
            dioxus::prelude::document::eval(&js);

            needs_dom_sync.set(false);
        }
    });

    // Debounce task handle — cancelled and replaced on each keystroke
    let mut debounce_task: Signal<Option<Task>> = use_signal(|| None);

    // Use local_text for highlighting so it updates immediately on keystroke
    let highlighted_html = use_memo(move || {
        let source = local_text.read().clone();
        let theme = &*theme.read();
        let mut html = String::with_capacity(source.len() * 3);

        for line in source.lines() {
            let spans = Highlighter::highlight_line(line);
            html.push_str(&Renderer::to_html_inline(line, &spans, theme));
            html.push('\n');
        }

        // Ensure trailing newline for cursor positioning
        if source.ends_with('\n') || source.is_empty() {
            html.push('\n');
        }

        html
    });

    // Live diagnostics — feeds the squiggle overlay and the status footer.
    // `analyze` is sub-millisecond for typical chart sizes, so we re-run it
    // on every edit (already gated by the local_text signal updates above).
    let analysis_diags = use_memo(move || {
        let source = local_text.read().clone();
        let analysis = ide::analyze(&source);
        analysis.diagnostics
    });

    // Squiggle-overlay HTML: transparent text everywhere, with wavy-underline
    // wrappers around each diagnostic range so the underline lines up with
    // the same monospace cells as the highlighted text below.
    let diagnostics_overlay_html = use_memo(move || {
        let source = local_text.read().clone();
        let diags = analysis_diags.read().clone();
        render_squiggle_overlay(&source, &diags)
    });

    // Status footer summary: "3 errors · 1 warning" (or empty when clean).
    let (error_count, warning_count, first_message): (usize, usize, Option<String>) = {
        let diags = analysis_diags.read();
        let errors = diags
            .iter()
            .filter(|d| matches!(d.severity, Severity::Error))
            .count();
        let warnings = diags
            .iter()
            .filter(|d| matches!(d.severity, Severity::Warning))
            .count();
        let first = diags.first().map(|d| d.message.clone());
        (errors, warnings, first)
    };
    let maybe_s_err = if error_count == 1 { "" } else { "s" };
    let maybe_s_warn = if warning_count == 1 { "" } else { "s" };

    // Syntax theme foreground color for unhighlighted text
    let fg_color = {
        let theme = theme.read();
        theme.foreground.to_css()
    };

    // Initial value for the textarea — only used on first render.
    // After that, user input keeps the DOM in sync naturally (uncontrolled),
    // and external changes go through the JS eval effect above.
    let initial_value = local_text.read().clone();

    rsx! {
        div {
            class: "relative flex-1 overflow-hidden flex flex-col",
            style: "font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, monospace; font-size: 14px; line-height: 21px;",

            // Stacked editor layers: highlight display -> squiggle overlay -> textarea.
            div {
                class: "relative flex-1 overflow-hidden",

            // Highlighted code display (underneath)
            pre {
                class: "absolute inset-0 overflow-auto pointer-events-none m-0 p-3 whitespace-pre bg-card",
                id: "highlight-display",
                style: "color: {fg_color}; tab-size: 4;",
                dangerous_inner_html: "{highlighted_html}"
            }

            // Diagnostics overlay (between highlights and textarea). Transparent
            // text; only the wavy underlines on diagnostic ranges paint.
            pre {
                class: "absolute inset-0 overflow-auto pointer-events-none m-0 p-3 whitespace-pre",
                id: "diagnostic-overlay",
                style: "color: transparent; tab-size: 4;",
                dangerous_inner_html: "{diagnostics_overlay_html}"
            }

            // Transparent textarea (on top, receives input)
            textarea {
                class: "absolute inset-0 w-full h-full resize-none outline-none m-0 p-3 whitespace-pre",
                id: "editor-textarea",
                style: "background: transparent; color: transparent; caret-color: var(--foreground); tab-size: 4; font: inherit; line-height: inherit;",
                spellcheck: false,
                // Set initial value only — textarea is uncontrolled after mount
                initial_value: "{initial_value}",
                oninput: move |evt| {
                    let value = evt.value().clone();
                    // Update local state immediately for responsive typing & highlighting
                    local_text.set(value.clone());

                    // Cancel any pending debounce task
                    if let Some(prev) = *debounce_task.read() {
                        prev.cancel();
                    }

                    // Spawn a new debounced task — fires on_change after 150ms of inactivity
                    let task = spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                        on_change.call(value);
                    });
                    debounce_task.set(Some(task));
                },
                // Synchronize scroll position with highlight display
                onscroll: move |_| {
                    #[cfg(feature = "web")]
                    {
                        let js = r#"
                            (function() {
                                var ta = document.getElementById('editor-textarea');
                                var hl = document.getElementById('highlight-display');
                                var ov = document.getElementById('diagnostic-overlay');
                                if (ta && hl) {
                                    hl.scrollTop = ta.scrollTop;
                                    hl.scrollLeft = ta.scrollLeft;
                                }
                                if (ta && ov) {
                                    ov.scrollTop = ta.scrollTop;
                                    ov.scrollLeft = ta.scrollLeft;
                                }
                            })();
                        "#;
                        dioxus::prelude::document::eval(js);
                    }
                }
            }
            } // end stacked editor layers div

            // Status footer — error/warning summary + first message tooltip.
            // Composed from fts-ui Text primitives (uses theme tokens for the
            // muted-foreground / destructive / warning colors).
            div {
                class: "border-t bg-card px-3 py-1 flex gap-3 items-center",
                style: "min-height: 22px; line-height: 18px; font-family: inherit;",
                {
                    if error_count == 0 && warning_count == 0 {
                        rsx! {
                            Text { variant: TextVariant::Muted, "no problems" }
                        }
                    } else {
                        rsx! {
                            Text {
                                variant: TextVariant::Small,
                                class: "text-destructive".to_string(),
                                "{error_count} error{maybe_s_err}"
                            }
                            Text {
                                variant: TextVariant::Small,
                                class: "text-warning".to_string(),
                                "{warning_count} warning{maybe_s_warn}"
                            }
                            if let Some(msg) = first_message.as_ref() {
                                Text {
                                    variant: TextVariant::Muted,
                                    class: "truncate".to_string(),
                                    "· {msg}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Render the source as transparent text with a wavy underline applied to
/// each diagnostic range. The output sits on a `<pre>` overlay positioned
/// identically to the highlight layer, so the underlines hit the same
/// monospace cells as the visible code.
fn render_squiggle_overlay(source: &str, diagnostics: &[keyflow::text::ide::Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    // Sort + clip ranges to the source bounds; merge overlaps so we can do a
    // simple sweep below.
    let len = source.len();
    let mut ranges: Vec<(usize, usize, Severity)> = diagnostics
        .iter()
        .filter_map(|d| {
            let start = d.range.start.min(len);
            let end = d.range.end().min(len);
            (start < end).then_some((start, end, d.severity))
        })
        .collect();
    ranges.sort_by_key(|&(s, _, _)| s);

    let mut out = String::with_capacity(source.len() + diagnostics.len() * 64);
    let mut cursor = 0usize;
    for (start, end, severity) in ranges {
        if start < cursor {
            // Overlap with prior range — skip.
            continue;
        }
        if cursor < start {
            push_escaped(&mut out, &source[cursor..start]);
        }
        let class = match severity {
            Severity::Error => "kf-diag kf-diag-error",
            Severity::Warning => "kf-diag kf-diag-warning",
            Severity::Info => "kf-diag kf-diag-info",
            Severity::Hint => "kf-diag kf-diag-hint",
        };
        // Use the theme's CSS custom properties so squiggle colors track
        // light/dark/custom themes set via `fts_ui::ThemeProvider`. The
        // tokens are defined in `fts-ui`'s theme stylesheet.
        let color = match severity {
            Severity::Error => "var(--destructive)",
            Severity::Warning => "var(--warning)",
            Severity::Info => "var(--info)",
            Severity::Hint => "var(--muted-foreground)",
        };
        out.push_str(&format!(
            "<span class=\"{class}\" style=\"text-decoration: underline wavy {color}; \
             text-decoration-skip-ink: none; text-underline-offset: 3px;\">"
        ));
        push_escaped(&mut out, &source[start..end]);
        out.push_str("</span>");
        cursor = end;
    }
    if cursor < source.len() {
        push_escaped(&mut out, &source[cursor..]);
    }
    out
}

fn push_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyflow::text::ide::Diagnostic as IdeDiagnostic;
    use keyflow::text::parsing::TextSpan;

    #[test]
    fn squiggle_overlay_empty_when_no_diagnostics() {
        assert_eq!(render_squiggle_overlay("Cmaj7 | G7", &[]), "");
    }

    #[test]
    fn squiggle_overlay_wraps_diagnostic_range_only() {
        let source = "Cmaj7 BAD G7";
        let diag = IdeDiagnostic::error("kf001-parse-failed", "boom", TextSpan::new(6, 3));
        let html = render_squiggle_overlay(source, &[diag]);
        assert!(html.contains("Cmaj7 "));
        assert!(html.contains("BAD"));
        assert!(html.contains("kf-diag-error"));
        // Suffix preserved untouched.
        assert!(html.ends_with("G7"));
    }

    #[test]
    fn squiggle_overlay_escapes_html_specials() {
        let source = "<<bad>>";
        let diag = IdeDiagnostic::error("kf001", "x", TextSpan::new(0, source.len()));
        let html = render_squiggle_overlay(source, &[diag]);
        assert!(html.contains("&lt;&lt;bad&gt;&gt;"));
        assert!(!html.contains("<<bad>>"));
    }
}
