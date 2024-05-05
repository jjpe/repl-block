//!

mod cmd;
mod editor;
mod error;
mod history;
mod macros;

pub mod prelude {
    pub use crate::{
        cmd::{Cmd, Last},
        editor::{Editor, EditorBuilder, FlushPolicy},
        error::{ReplBlockError, ReplBlockResult},
        history::{History, HistIdx},
    };
}
