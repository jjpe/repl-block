//!

mod cmd;
mod editor;
mod error;
mod history;
mod macros;

pub mod prelude {
    pub use crate::{
        cmd::Cmd,
        editor::Editor,
        history::{History, HistIdx},
    };
}
