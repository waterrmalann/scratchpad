//! Text editing engine: buffer, cursor, selection, history and Markdown decorations. Free of any UI framework.

mod buffer;
mod coords;
mod editor;
mod graphemes;
mod selection;

pub use buffer::{Buffer, LineEnding, TextChange};
pub use coords::{Bias, ByteOffset, Point, Utf16Offset};
pub use editor::Editor;
pub use selection::{Goal, Selection};
