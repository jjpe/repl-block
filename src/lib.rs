//!

mod cmd;
mod editor;
mod error;
mod history;
mod macros;

pub mod prelude {
    pub use crate::{
        editor::{Editor, EditorBuilder},
        error::{ReplBlockError, ReplBlockResult},
    };
    pub use crossterm::style::{Color, Stylize};
}
