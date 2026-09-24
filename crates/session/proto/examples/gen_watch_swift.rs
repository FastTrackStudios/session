//! Generate a watch app's Swift `Codable` mirror of its wire DTOs from
//! their facet shapes — Rust stays the source of truth.
//!
//! ```bash
//! # the remote's session page (the `/watch/v1` bridge)
//! cargo run -p session-proto --example gen_watch_swift \
//!     > apps/desktop/watchos/FTSWatch/Generated/WatchSession.generated.swift
//! # the Session watch app's guide feed (relayed by the iPhone)
//! cargo run -p session-proto --example gen_watch_swift -- guide \
//!     > apps/session-watch/SessionWatch/Generated/WatchGuide.generated.swift
//! ```
//!
//! The walker covers exactly what the watch DTOs use — structs of
//! primitives, `String`, `Vec<T>`, `Option<T>`, and enums of unit variants
//! — and fails loudly on anything else so a proto change can't silently
//! drift from Swift.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use facet::{Def, Facet, Shape, StructKind, Type, UserType};
use session_proto::watch::{WatchMessage, WatchPong, WatchSessionState};

fn main() -> Result<(), String> {
    let which = std::env::args().nth(1).unwrap_or_else(|| "session".into());
    let (roots, source, output): (&[&'static Shape], &str, &str) = match which.as_str() {
        "session" => (
            &[WatchSessionState::SHAPE],
            "the `/watch/v1` wire DTOs",
            "apps/desktop/watchos/FTSWatch/Generated/WatchSession.generated.swift",
        ),
        "guide" => (
            &[WatchMessage::SHAPE, WatchPong::SHAPE],
            "the guide feed the iPhone relays",
            "apps/session-watch/SessionWatch/Generated/WatchGuide.generated.swift",
        ),
        other => return Err(format!("gen_watch_swift: unknown set {other} (session | guide)")),
    };
    let mut items = BTreeMap::new();
    for root in roots {
        collect(root, &mut items)?;
    }

    let mut out = String::new();
    let args = if which == "session" { String::new() } else { format!(" -- {which}") };
    let _ = write!(
        out,
        "// GENERATED — do not edit. Mirrors the facet shapes in\n\
         // crates/session/proto/src/watch.rs ({source}).\n\
         // Regenerate: cargo run -p session-proto --example gen_watch_swift{args}\n\
         //   > {output}\n\n\
         import Foundation\n\n",
    );
    for (name, item) in &items {
        match item {
            Item::Enum(variants) => {
                // facet-json writes a unit variant as its name.
                let _ = writeln!(out, "public enum {name}: String, Codable, Equatable, Sendable {{");
                for variant in variants {
                    let _ = writeln!(out, "    case {} = \"{variant}\"", lower_first(variant));
                }
                out.push_str("}\n\n");
            }
            Item::Struct(body) => write_struct(&mut out, name, body),
        }
    }
    print!("{out}");
    Ok(())
}

fn write_struct(out: &mut String, name: &str, body: &StructBody) {
    let _ = writeln!(out, "public struct {name}: Codable, Equatable, Sendable {{");
    for (field, ty) in body {
        let _ = writeln!(out, "    public var {}: {ty}", camel(field));
    }
    // Memberwise init (public structs don't get one across module
    // boundaries for free).
    out.push_str("\n    public init(\n");
    let params: Vec<String> = body
        .iter()
        .map(|(field, ty)| format!("        {}: {ty}", camel(field)))
        .collect();
    out.push_str(&params.join(",\n"));
    out.push_str("\n    ) {\n");
    for (field, _) in body {
        let f = camel(field);
        let _ = writeln!(out, "        self.{f} = {f}");
    }
    out.push_str("    }\n\n");
    // The wire is snake_case (facet-json uses the Rust field names).
    out.push_str("    enum CodingKeys: String, CodingKey {\n");
    for (field, _) in body {
        let _ = writeln!(out, "        case {} = \"{field}\"", camel(field));
    }
    out.push_str("    }\n}\n\n");
}

/// Field list in declaration order: (`rust_name`, `swift_type`).
type StructBody = Vec<(String, String)>;

/// A Swift declaration to emit.
enum Item {
    Struct(StructBody),
    /// Variant names, in declaration order.
    Enum(Vec<String>),
}

/// Recursively collect every user struct and enum reachable from `shape`.
fn collect(shape: &'static Shape, out: &mut BTreeMap<String, Item>) -> Result<(), String> {
    if out.contains_key(shape.type_identifier) {
        return Ok(());
    }
    match &shape.ty {
        Type::User(UserType::Struct(st)) => {
            let mut body = StructBody::new();
            for field in st.fields {
                body.push((field.name.to_string(), swift_type(field.shape(), out)?));
            }
            out.insert(shape.type_identifier.to_string(), Item::Struct(body));
        }
        Type::User(UserType::Enum(en)) => {
            let mut variants = Vec::new();
            for variant in en.variants {
                if variant.data.kind != StructKind::Unit {
                    return Err(format!(
                        "gen_watch_swift: {}::{} carries data — only unit variants map to Swift",
                        shape.type_identifier, variant.name
                    ));
                }
                variants.push(variant.name.to_string());
            }
            out.insert(shape.type_identifier.to_string(), Item::Enum(variants));
        }
        _ => {
            return Err(format!(
                "gen_watch_swift: expected a struct or enum shape, got {}",
                shape.type_identifier
            ));
        }
    }
    Ok(())
}

/// Map a facet shape to its Swift spelling, recursing into user structs.
fn swift_type(shape: &'static Shape, out: &mut BTreeMap<String, Item>) -> Result<String, String> {
    match &shape.def {
        Def::List(l) => return Ok(format!("[{}]", swift_type(l.t, out)?)),
        Def::Option(o) => return Ok(format!("{}?", swift_type(o.t, out)?)),
        _ => {}
    }
    Ok(match shape.type_identifier {
        "String" => "String".into(),
        "bool" => "Bool".into(),
        "f32" => "Float".into(),
        "f64" => "Double".into(),
        "u8" => "UInt8".into(),
        "u16" => "UInt16".into(),
        "u32" => "UInt32".into(),
        "u64" => "UInt64".into(),
        "i8" => "Int8".into(),
        "i16" => "Int16".into(),
        "i32" => "Int32".into(),
        "i64" => "Int64".into(),
        other => {
            if let Type::User(UserType::Struct(_) | UserType::Enum(_)) = &shape.ty {
                collect(shape, out)?;
                other.into()
            } else {
                return Err(format!(
                    "gen_watch_swift: unsupported type {other} — extend the walker"
                ));
            }
        }
    })
}

/// `snake_case` → camelCase (Swift field convention).
fn camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut upper_next = false;
    for c in s.chars() {
        if c == '_' {
            upper_next = true;
        } else if upper_next {
            result.extend(c.to_uppercase());
            upper_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// `Downbeat` → `downbeat`, `CountIn` → `countIn` (Swift case convention).
fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect())
}
