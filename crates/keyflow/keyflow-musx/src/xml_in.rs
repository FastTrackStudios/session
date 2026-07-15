//! Read-side helpers over [`roxmltree`] that stand in for the lxml
//! `find` / `xpath` calls in the original converter.
//!
//! EnigmaXML places every element in the Finale default namespace and the
//! metadata document in the NotationMetadata namespace; since each document
//! uses a single namespace throughout, we match purely on local element names
//! (the `f:` / `m:` prefixes in the Python XPath are just that one namespace).

use roxmltree::Node;

pub type XNode<'a, 'i> = Node<'a, 'i>;

/// Direct element children of `node` with the given local name.
pub fn children<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Vec<Node<'a, 'i>> {
    node.children()
        .filter(|c| c.is_element() && c.tag_name().name() == name)
        .collect()
}

/// First direct element child with the given local name — `el.find("f:name")`.
pub fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Whether a direct child element with the given name exists.
pub fn has_child(node: Node, name: &str) -> bool {
    child(node, name).is_some()
}

/// Walk a `/`-style path of local names, taking the first match at each step —
/// `el.find("f:a/f:b/f:c")`.
pub fn find_path<'a, 'i>(node: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
    let mut cur = node;
    for name in path {
        cur = child(cur, name)?;
    }
    Some(cur)
}

/// Text of the first path match, if present.
pub fn path_text(node: Node, path: &[&str]) -> Option<String> {
    find_path(node, path).and_then(text)
}

/// An element's text (the text node before its first child), like lxml `.text`.
pub fn text(node: Node) -> Option<String> {
    node.text().map(|s| s.to_string())
}

/// An attribute value.
pub fn attr(node: Node, name: &str) -> Option<String> {
    node.attribute(name).map(|s| s.to_string())
}

/// All `container/name` grandchildren under `root` — the common
/// `/f:finale/f:<container>/f:<name>` XPath shape.
pub fn grandchildren<'a, 'i>(
    root: Node<'a, 'i>,
    container: &str,
    name: &str,
) -> Vec<Node<'a, 'i>> {
    let mut out = Vec::new();
    for c in children(root, container) {
        out.extend(children(c, name));
    }
    out
}

/// First `container/name` grandchild whose `attr_name` equals `attr_val`.
pub fn find_by_attr<'a, 'i>(
    root: Node<'a, 'i>,
    container: &str,
    name: &str,
    attr_name: &str,
    attr_val: &str,
) -> Option<Node<'a, 'i>> {
    grandchildren(root, container, name)
        .into_iter()
        .find(|n| n.attribute(attr_name) == Some(attr_val))
}
