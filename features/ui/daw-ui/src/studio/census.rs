//! What the window actually built, counted once.
//!
//! Optimising a DOM you have not counted is guesswork, and the guesses
//! are usually wrong by an order of magnitude. This reports the node
//! census a few seconds after the project lands — total, SVG, and the
//! split between the track panel and the lanes — so "the TCP is heavy"
//! is a number rather than a hunch.
//!
//! One shot, not a poll: `querySelectorAll('*')` walks the whole tree,
//! which is exactly the kind of work this module exists to find.
//!
//! `RUST_LOG=daw_ui::studio::census=info`.

use crate::prelude::*;

/// Counts the rendered tree once, and logs it.
#[component]
pub fn Census() -> Element {
    let mut eval = use_hook(|| {
        document::eval(
            r"
            // After the project has landed and settled — counting an
            // empty window measures nothing.
            const wait = (ms) => new Promise((r) => setTimeout(r, ms));
            (async () => {
                for (let i = 0; i < 120; i++) {
                    if (document.querySelector('.studio-tcp .studio-row')) { break; }
                    await wait(500);
                }
                await wait(2000);
                const count = (sel) => document.querySelectorAll(sel).length;
                dioxus.send([
                    count('*'),
                    count('svg'),
                    count('svg *'),
                    count('.studio-tcp *'),
                    count('.studio-lanes *'),
                    count('.studio-item'),
                ]);
            })();
            ",
        )
    });
    use_future(move || async move {
        if let Ok([total, svgs, svg_children, tcp, lanes, items]) = eval.recv::<[f64; 6]>().await {
            tracing::info!(
                dom.total = total,
                dom.svg_roots = svgs,
                dom.svg_nodes = svg_children,
                dom.tcp_nodes = tcp,
                dom.lane_nodes = lanes,
                dom.items = items,
                "dom census"
            );
        }
    });
    rsx! {}
}
