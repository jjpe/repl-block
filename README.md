# repl-block

[![crates.io](https://img.shields.io/crates/v/repl-block?label=repl-block)](https://crates.io/crates/repl-block)
[![Documentation](https://docs.rs/repl-block/badge.svg)](https://docs.rs/repl-block/latest)
![Rust](https://github.com/jjpe/repl-block/workflows/Rust/badge.svg)
![](https://img.shields.io/badge/rustc-1.68.2+-red.svg)
![](https://img.shields.io/crates/l/repl-block)

## Synopsis

This crate provides a simple and easy way to build a `Read-Eval-Print-Loop`,
a.k.a. REPL.

## Usage

Add a dependency on this crate to your project's `Cargo.toml`:
``` toml
[dependencies]
repl-block= "0.7.1"
```

Then one can use the `ReplBuilder` type to build an start a REPL like this:
```rust
use repl_block::prelude::{ReplBuilder, ReplBlockResult, Utf8PathBuf};

fn main() -> ReplBlockResult<()> {
    let mut evaluator = /* initialize your evaluator */;
    let path = Utf8PathBuf::try_from(env::current_dir()?)?.join(".repl.history");
    ReplBuilder::default()
        // Explicitly register .repl.history as the history file:
        .history_filepath(path)
        // Register the evaluator; the default evaluator fn is NOP
        .evaluator(|query: &str| {
            match evaluator.evaluate(query) {
                Ok(value) => println!("{value}"),
                Err(err)  => println!("{err}"),
            }
            Ok(())
        })
        .build()? // Use `self` to build a REPL
        .start()?;
    Ok(())
}

```
