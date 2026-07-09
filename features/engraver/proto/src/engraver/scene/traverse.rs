//! Scene graph traversal utilities.
//!
//! Provides visitor pattern and iterator implementations for traversing
//! scene graphs. Used by both WGPU renderer and SVG serializer.

use kurbo::Affine;

use super::node::SceneNode;
use super::transform::TransformStack;

/// Visitor trait for scene graph traversal.
///
/// Implement this trait to process scene nodes during traversal.
/// The visitor receives callbacks at enter/exit points with accumulated transforms.
///
/// # Example
///
/// ```ignore
/// struct BoundsCollector {
///     bounds: Vec<Rect>,
/// }
///
/// impl SceneVisitor for BoundsCollector {
///     fn enter_node(&mut self, node: &SceneNode, transform: Affine) -> bool {
///         if node.visible {
///             let world_bounds = transform.transform_rect_bbox(node.bounds);
///             self.bounds.push(world_bounds);
///             true // continue traversal
///         } else {
///             false // skip this subtree
///         }
///     }
/// }
/// ```
pub trait SceneVisitor {
    /// Called when entering a node.
    ///
    /// # Arguments
    /// * `node` - The current scene node
    /// * `transform` - The accumulated world transform at this node
    ///
    /// # Returns
    /// * `true` to continue traversing children
    /// * `false` to skip this node's children
    fn enter_node(&mut self, node: &SceneNode, transform: Affine) -> bool;

    /// Called when exiting a node (after all children processed).
    ///
    /// Default implementation does nothing.
    fn exit_node(&mut self, _node: &SceneNode, _transform: Affine) {}
}

/// Traverse a scene graph depth-first with a visitor.
///
/// # Arguments
/// * `root` - The root node to start traversal from
/// * `visitor` - The visitor to receive callbacks
pub fn traverse<V: SceneVisitor>(root: &SceneNode, visitor: &mut V) {
    let mut stack = TransformStack::new();
    traverse_recursive(root, visitor, &mut stack);
}

/// Traverse with a custom initial transform.
pub fn traverse_with_transform<V: SceneVisitor>(
    root: &SceneNode,
    visitor: &mut V,
    initial_transform: Affine,
) {
    let mut stack = TransformStack::new();
    stack.push(initial_transform);
    traverse_recursive(root, visitor, &mut stack);
}

fn traverse_recursive<V: SceneVisitor>(
    node: &SceneNode,
    visitor: &mut V,
    stack: &mut TransformStack,
) {
    // Push this node's transform
    stack.push(node.transform);
    let current_transform = stack.current();

    // Enter node
    let continue_traversal = visitor.enter_node(node, current_transform);

    // Traverse children if requested
    if continue_traversal {
        for child in &node.children {
            traverse_recursive(child, visitor, stack);
        }
    }

    // Exit node
    visitor.exit_node(node, current_transform);

    // Pop transform
    stack.pop();
}

/// Iterator over all nodes in a scene graph (depth-first).
pub struct NodeIterator<'a> {
    stack: Vec<&'a SceneNode>,
}

impl<'a> NodeIterator<'a> {
    /// Create a new iterator starting from the given root.
    #[must_use]
    pub fn new(root: &'a SceneNode) -> Self {
        Self { stack: vec![root] }
    }
}

impl<'a> Iterator for NodeIterator<'a> {
    type Item = &'a SceneNode;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;

        // Push children in reverse order so first child is processed first
        for child in node.children.iter().rev() {
            self.stack.push(child);
        }

        Some(node)
    }
}

/// Iterator over nodes with their accumulated world transforms.
pub struct TransformIterator<'a> {
    // Stack of (node, depth) pairs
    stack: Vec<(&'a SceneNode, usize)>,
    transform_stack: TransformStack,
    current_depth: usize,
}

impl<'a> TransformIterator<'a> {
    /// Create a new transform iterator starting from the given root.
    #[must_use]
    pub fn new(root: &'a SceneNode) -> Self {
        Self {
            stack: vec![(root, 0)],
            transform_stack: TransformStack::new(),
            current_depth: 0,
        }
    }
}

impl<'a> Iterator for TransformIterator<'a> {
    type Item = (&'a SceneNode, Affine);

    fn next(&mut self) -> Option<Self::Item> {
        let (node, depth) = self.stack.pop()?;

        // Pop transforms until we're at the correct depth
        while self.current_depth > depth {
            self.transform_stack.pop();
            self.current_depth -= 1;
        }

        // Push this node's transform
        self.transform_stack.push(node.transform);
        self.current_depth = depth + 1;

        let transform = self.transform_stack.current();

        // Push children in reverse order
        for child in node.children.iter().rev() {
            self.stack.push((child, depth + 1));
        }

        Some((node, transform))
    }
}

/// Extension trait for SceneNode to provide iteration methods.
pub trait SceneNodeExt {
    /// Iterate over all nodes depth-first.
    fn iter(&self) -> NodeIterator<'_>;

    /// Iterate over nodes with accumulated transforms.
    fn iter_with_transforms(&self) -> TransformIterator<'_>;
}

impl SceneNodeExt for SceneNode {
    fn iter(&self) -> NodeIterator<'_> {
        NodeIterator::new(self)
    }

    fn iter_with_transforms(&self) -> TransformIterator<'_> {
        TransformIterator::new(self)
    }
}

/// Collect all visible nodes with their world transforms.
#[must_use]
pub fn collect_visible_nodes(root: &SceneNode) -> Vec<(&SceneNode, Affine)> {
    root.iter_with_transforms()
        .filter(|(node, _)| node.visible)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::scene::id::SemanticId;
    use kurbo::Point;

    fn create_test_tree() -> SceneNode {
        let mut root = SceneNode::group(SemanticId::page(1));

        let mut system =
            SceneNode::group(SemanticId::system(1)).with_position(Point::new(0.0, 100.0));

        let measure1 =
            SceneNode::group(SemanticId::measure(1)).with_position(Point::new(50.0, 0.0));
        let measure2 =
            SceneNode::group(SemanticId::measure(2)).with_position(Point::new(150.0, 0.0));

        system.add_child(measure1);
        system.add_child(measure2);
        root.add_child(system);

        root
    }

    #[test]
    fn test_node_iterator() {
        let root = create_test_tree();
        let nodes: Vec<_> = root.iter().collect();

        assert_eq!(nodes.len(), 4); // root, system, measure1, measure2
    }

    #[test]
    fn test_transform_iterator() {
        let root = create_test_tree();
        let nodes: Vec<_> = root.iter_with_transforms().collect();

        assert_eq!(nodes.len(), 4);

        // Check that measure1 has correct accumulated transform
        // Root (identity) * System (translate 0,100) * Measure1 (translate 50,0)
        let (measure1, transform) = &nodes[2];
        assert!(measure1.id.is_some());

        let pt = *transform * Point::new(0.0, 0.0);
        assert!((pt.x - 50.0).abs() < 1e-10);
        assert!((pt.y - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_visitor_pattern() {
        struct NodeCounter {
            count: usize,
        }

        impl SceneVisitor for NodeCounter {
            fn enter_node(&mut self, _node: &SceneNode, _transform: Affine) -> bool {
                self.count += 1;
                true
            }
        }

        let root = create_test_tree();
        let mut counter = NodeCounter { count: 0 };
        traverse(&root, &mut counter);

        assert_eq!(counter.count, 4);
    }

    #[test]
    fn test_visitor_skip_children() {
        struct SkipSystemVisitor {
            visited: Vec<String>,
        }

        impl SceneVisitor for SkipSystemVisitor {
            fn enter_node(&mut self, node: &SceneNode, _transform: Affine) -> bool {
                if let Some(id) = &node.id {
                    self.visited.push(id.svg_id());
                }

                // Skip system's children
                !matches!(
                    node.id.as_ref().map(|id| id.element_type),
                    Some(crate::engraver::scene::id::ElementType::System)
                )
            }
        }

        let root = create_test_tree();
        let mut visitor = SkipSystemVisitor {
            visited: Vec::new(),
        };
        traverse(&root, &mut visitor);

        // Should visit page and system, but not measures
        assert_eq!(visitor.visited.len(), 2);
        assert!(visitor.visited.contains(&"page-1".to_string()));
        assert!(visitor.visited.contains(&"system-1".to_string()));
    }

    #[test]
    fn test_invisible_nodes() {
        let mut root = SceneNode::new();
        let visible = SceneNode::group(SemanticId::measure(1));
        let invisible = SceneNode::group(SemanticId::measure(2)).with_visible(false);

        root.add_child(visible);
        root.add_child(invisible);

        let visible_nodes = collect_visible_nodes(&root);

        // Should only include root and visible child
        assert_eq!(visible_nodes.len(), 2);
    }
}
