//!

mod cmd;
mod repl;
mod error;
mod history;
mod macros;

pub mod prelude {
    pub use crate::{
        repl::{Repl, ReplBuilder},
        error::{ReplBlockError, ReplBlockResult},
    };
    pub use crossterm::style::{Color, Stylize};
}
