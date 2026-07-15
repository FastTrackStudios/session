//! A tiny mutable XML tree, mirroring the subset of the lxml `Element` API that
//! the converter relies on (`SubElement`, `.text`, `.set`, `.remove`,
//! `reorder_children`) plus a pretty-printing serializer.

use std::cell::RefCell;
use std::rc::Rc;

/// A shared, mutable element handle — the analogue of an lxml `Element`.
pub type El = Rc<RefCell<Elem>>;

pub struct Elem {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: Option<String>,
    pub children: Vec<El>,
}

/// Create a detached root element with the given attributes.
pub fn element(tag: &str, attrs: &[(&str, &str)]) -> El {
    Rc::new(RefCell::new(Elem {
        tag: tag.to_string(),
        attrs: attrs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        text: None,
        children: Vec::new(),
    }))
}

/// `SubElement(parent, tag)` — create a child, append it, return it.
pub fn sub(parent: &El, tag: &str) -> El {
    let child = element(tag, &[]);
    parent.borrow_mut().children.push(child.clone());
    child
}

/// `SubElement(parent, tag, **attrs)`.
pub fn sub_attrs(parent: &El, tag: &str, attrs: &[(&str, &str)]) -> El {
    let child = element(tag, attrs);
    parent.borrow_mut().children.push(child.clone());
    child
}

/// `SubElement(parent, tag).text = text` in one shot.
pub fn sub_text(parent: &El, tag: &str, text: &str) -> El {
    let child = sub(parent, tag);
    child.borrow_mut().text = Some(text.to_string());
    child
}

/// `el.text = text`.
pub fn set_text(el: &El, text: &str) {
    el.borrow_mut().text = Some(text.to_string());
}

/// `el.set(key, value)` — set (or replace) an attribute, preserving order.
pub fn set_attr(el: &El, key: &str, value: &str) {
    let mut e = el.borrow_mut();
    if let Some(slot) = e.attrs.iter_mut().find(|(k, _)| k == key) {
        slot.1 = value.to_string();
    } else {
        e.attrs.push((key.to_string(), value.to_string()));
    }
}

/// `parent.remove(child)` — remove by identity.
pub fn remove_child(parent: &El, child: &El) {
    parent
        .borrow_mut()
        .children
        .retain(|c| !Rc::ptr_eq(c, child));
}

/// Number of children — the analogue of `len(el.getchildren())`.
pub fn child_count(el: &El) -> usize {
    el.borrow().children.len()
}

/// Port of `helper.reorder_children`: stable-group children by tag, then emit
/// them in `order`, appending any tags not listed at the end.
pub fn reorder_children(parent: &El, order: &[&str]) {
    let existing: Vec<El> = std::mem::take(&mut parent.borrow_mut().children);

    let mut ordered: Vec<El> = Vec::with_capacity(existing.len());
    for tag in order {
        for child in &existing {
            if child.borrow().tag == *tag {
                ordered.push(child.clone());
            }
        }
    }
    for child in &existing {
        let tag = child.borrow().tag.clone();
        if !order.contains(&tag.as_str()) {
            ordered.push(child.clone());
        }
    }
    parent.borrow_mut().children = ordered;
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

fn write_indented(el: &El, depth: usize, out: &mut String) {
    let e = el.borrow();
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(&e.tag);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        out.push_str(&escape_attr(v));
        out.push('"');
    }

    if e.children.is_empty() {
        // Match lxml: text set (even to "") renders open/close; unset self-closes.
        match &e.text {
            Some(t) => {
                out.push('>');
                out.push_str(&escape_text(t));
                out.push_str("</");
                out.push_str(&e.tag);
                out.push('>');
            }
            None => out.push_str("/>"),
        }
    } else {
        out.push('>');
        out.push('\n');
        for child in &e.children {
            write_indented(child, depth + 1, out);
            out.push('\n');
        }
        out.push_str(&indent);
        out.push_str("</");
        out.push_str(&e.tag);
        out.push('>');
    }
}

/// Serialize `root` as a full MusicXML document: XML declaration, the
/// score-partwise DOCTYPE, then the pretty-printed tree.
pub fn serialize_document(root: &El) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" \
\"http://www.musicxml.org/dtds/partwise.dtd\">\n",
    );
    write_indented(root, 0, &mut out);
    out.push('\n');
    out
}
