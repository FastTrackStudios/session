//! FastTrackStudio Web App
//!
//! Note: This crate is WASM-only. On native targets, it compiles to an empty binary.

#![cfg_attr(not(target_arch = "wasm32"), allow(unused))]

#[cfg(target_arch = "wasm32")]
use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Navbar)]
    #[route("/")]
    Home {},
    #[route("/blog/:id")]
    Blog { id: i32 },
}

#[cfg(target_arch = "wasm32")]
const FAVICON: Asset = asset!("/assets/favicon.ico");
#[cfg(target_arch = "wasm32")]
const MAIN_CSS: Asset = asset!("/assets/main.css");
#[cfg(target_arch = "wasm32")]
const HEADER_SVG: Asset = asset!("/assets/header.svg");
#[cfg(target_arch = "wasm32")]
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        dioxus::launch(App);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("This binary only runs on wasm32. Use `dx build` to compile for WASM.");
    }
}

#[cfg(target_arch = "wasm32")]
#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        Router::<Route> {}
    }
}

#[cfg(target_arch = "wasm32")]
#[component]
pub fn Hero() -> Element {
    rsx! {
        div {
            id: "hero",
            img { src: HEADER_SVG, id: "header" }
            div { id: "links",
                a { href: "https://dioxuslabs.com/learn/0.7/", "Learn Dioxus" }
                a { href: "https://dioxuslabs.com/awesome", "Awesome Dioxus" }
                a { href: "https://github.com/dioxus-community/", "Community Libraries" }
                a { href: "https://github.com/DioxusLabs/sdk", "Dioxus Development Kit" }
                a { href: "https://marketplace.visualstudio.com/items?itemName=DioxusLabs.dioxus", "VSCode Extension" }
                a { href: "https://discord.gg/XgGxMSkvUM", "Community Discord" }
            }
        }
    }
}

/// Home page
#[cfg(target_arch = "wasm32")]
#[component]
fn Home() -> Element {
    rsx! {
        Hero {}
    }
}

/// Blog page
#[cfg(target_arch = "wasm32")]
#[component]
pub fn Blog(id: i32) -> Element {
    rsx! {
        div {
            id: "blog",

            // Content
            h1 { "This is blog #{id}!" }
            p { "In blog #{id}, we show how the Dioxus router works and how URL parameters can be passed as props to our route components." }

            // Navigation links
            Link {
                to: Route::Blog { id: id - 1 },
                "Previous"
            }
            span { " <---> " }
            Link {
                to: Route::Blog { id: id + 1 },
                "Next"
            }
        }
    }
}

/// Shared navbar component.
#[cfg(target_arch = "wasm32")]
#[component]
fn Navbar() -> Element {
    rsx! {
        div {
            id: "navbar",
            Link {
                to: Route::Home {},
                "Home"
            }
            Link {
                to: Route::Blog { id: 1 },
                "Blog"
            }
        }

        Outlet::<Route> {}
    }
}
