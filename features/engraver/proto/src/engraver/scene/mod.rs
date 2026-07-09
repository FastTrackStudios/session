//! Scene graph for music notation rendering.
//!
//! The scene graph provides a hierarchical structure for rendering music scores
//! to multiple backends (WGPU, SVG). Each node can have:
//! - A semantic ID linking to source music elements
//! - Paint commands for rendering
//! - Child nodes for hierarchical structure
//! - Transforms for positioning
//!
//! # Architecture
//!
//! ```text
//! SceneNode (root)
//!     ├── SemanticId (page-1)
//!     ├── transform: Affine
//!     ├── commands: Vec<PaintCommand>
//!     └── children: Vec<SceneNode>
//!         ├── SceneNode (system-1)
//!         │   └── SceneNode (measure-1)
//!         │       └── SceneNode (chord-1)
//!         │           ├── PaintCommand::Glyph (notehead)
//!         │           └── PaintCommand::Line (stem)
//!         └── ...
//! ```
//!
//! # Modules
//!
//! - [`id`] - Semantic identification for SVG data attributes
//! - [`node`] - Scene graph node structure
//! - [`paint`] - Backend-agnostic paint commands
//! - [`transform`] - Affine transform utilities
//! - [`traverse`] - Visitor pattern and iterators

// region:    --- Modules

pub mod id;
pub mod node;
pub mod paint;
pub mod transform;
pub mod traverse;

// endregion: --- Modules

// region:    --- Re-exports

// Re-export main types for convenience
pub use id::{ElementType, SemanticId};
pub use node::{GlyphInfo, GlyphType, SceneNode, metadata_keys};
pub use paint::{
    FillRule, FontStyle, FontWeight, LineCap, LineJoin, PaintCommand, TextAnchor, color_to_svg,
    path_to_svg_d,
};
pub use transform::{
    TransformStack, affine_to_svg_transform, get_scale, get_translation, is_identity, is_scale,
    is_translation, position_at, rotate_around, rotation_angle, scale_around,
};
pub use traverse::{
    NodeIterator, SceneNodeExt, SceneVisitor, TransformIterator, collect_visible_nodes, traverse,
    traverse_with_transform,
};

// endregion: --- Re-exports
