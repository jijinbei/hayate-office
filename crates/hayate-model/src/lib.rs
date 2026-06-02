//! HayateOffice editing layer (DESIGN 6.10): the uniform four-kind `Operation`, grouped
//! into transactions, with an undo/redo `History`. gpui-free. All document mutation flows
//! through `apply`, which keeps undo and (later) CRDT/serialization tractable.

pub mod align;
pub mod edit;
pub mod history;
pub mod op;

pub use align::{Align, Axis};
pub use edit::*;
pub use history::{History, Transaction};
pub use op::Operation;
