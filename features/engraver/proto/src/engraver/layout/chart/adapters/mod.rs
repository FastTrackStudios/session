//! Layout adapters for different rendering modes.
//!
//! This module contains implementations of `LayoutAdapter` for:
//! - `PaginatedAdapter`: Page-based layout with page breaks
//! - `ContinuousAdapter`: Infinite scroll layout

mod continuous;
mod paginated;

pub use continuous::ContinuousAdapter;
pub use paginated::PaginatedAdapter;
