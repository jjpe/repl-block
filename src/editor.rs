//!

use crate::{
    cmd::{Cmd, Line, LineKind},
    error::ReplBlockResult,
    history::{History, HistIdx},
    macros::key,
};
use camino::{Utf8Path, Utf8PathBuf};
use crossterm::{
    cursor, execute, queue, style, terminal,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{Stylize, StyledContent},
    terminal::ClearType,
};
use itertools::Itertools;
use std::io::{Stdout, Write};


type Evaluator<'eval> =
    dyn for<'src> FnMut(&'src str) -> ReplBlockResult<()> + 'eval;

pub struct EditorBuilder<'eval, W: Write> {
    sink: W,
    default_prompt: Vec<StyledContent<char>>,
    continue_prompt: Vec<StyledContent<char>>,
    history_filepath: Utf8PathBuf,
    evaluator: Box<Evaluator<'eval>>,
}

impl<'eval> Default for EditorBuilder<'eval, Stdout> {
    fn default() -> EditorBuilder<'eval, Stdout> {
        #[inline(always)]
        fn nop<'eval>() -> Box<Evaluator<'eval>> {
            Box::new(|_| Ok(()))
        }
        EditorBuilder {
            sink: std::io::stdout(),
            default_prompt:  vec!['■'.yellow(), '>'.green().bold(), ' '.reset()],
            continue_prompt: vec!['ꞏ'.yellow(), 'ꞏ'.yellow(),       ' '.reset()],
            history_filepath: Utf8PathBuf::new(),
            evaluator: nop(),
        }
    }
}

impl<'eval, W: Write> EditorBuilder<'eval, W> {
    pub fn sink<S: Write>(self, sink: S) -> EditorBuilder<'eval, S> {
        EditorBuilder {
            sink,
            default_prompt: self.default_prompt,
            continue_prompt: self.continue_prompt,
            history_filepath: self.history_filepath,
            evaluator: self.evaluator,
        }
    }

    pub fn default_prompt(mut self, prompt: Vec<StyledContent<char>>) -> Self {
        self.default_prompt = prompt;
        self
    }

    pub fn continue_prompt(mut self, prompt: Vec<StyledContent<char>>) -> Self {
        self.continue_prompt = prompt;
        self
    }

    pub fn history_filepath(mut self, filepath: impl AsRef<Utf8Path>) -> Self {
        self.history_filepath = filepath.as_ref().to_path_buf();
        self
    }

    pub fn evaluator<E>(mut self, evaluator: E) -> Self
    where
        E: for<'src> FnMut(&'src str) -> ReplBlockResult<()> + 'eval
    {
        self.evaluator = Box::new(evaluator);
        self
    }

    pub fn build(self) -> ReplBlockResult<Editor<'eval, W>> {
        assert_eq!(
            self.default_prompt.len(), self.continue_prompt.len(),
            "PRECONDITION FAILED: default_prompt.len() != continue_prompt.len()"
        );
        let mut editor = Editor::new(
            self.sink,
            self.history_filepath,
            self.evaluator,
            self.default_prompt,
            self.continue_prompt,
        )?;
        // The REPL operates in raw mode.  Raw mode is also explicitly turned
        // on before reading input, and turned off again afterwards.
        terminal::enable_raw_mode()?;
        editor.write_default_prompt(FlushPolicy::Flush)?;
        Ok(editor)
    }
}



// #[derive(Debug)] TODO write manual impl
pub struct Editor<'eval, W: Write> {
    sink: W,
    state: State,
    /// The height of the input area, in lines
    height: u16,
    /// The history of cmds
    history: History,
    /// The filepath of the history file
    history_filepath: Utf8PathBuf,
    /// The fn used to perform the Evaluate step of the REPL
    evaluator: Box<Evaluator<'eval>>,
    /// The default command prompt
    default_prompt: Vec<StyledContent<char>>,
    /// The command prompt used for command continuations
    continue_prompt: Vec<StyledContent<char>>,
}

impl<'eval, W: Write> Editor<'eval, W> {
    fn new(
        mut sink: W,
        history_filepath: impl AsRef<Utf8Path>,
        evaluator: Box<Evaluator<'eval>>,
        default_prompt: Vec<StyledContent<char>>,
        continue_prompt: Vec<StyledContent<char>>,
    ) -> ReplBlockResult<Editor<'eval, W>> {
        sink.flush()?;
        let mut editor = Self {
            sink,
            state: State::Edit(EditState {
                buffer: Cmd::default(),
                cursor: Coords::EDITOR_ORIGIN,
            }),
            height: 1,
            history: History::read_from_file(history_filepath.as_ref())?,
            history_filepath: history_filepath.as_ref().to_path_buf(),
            evaluator,
            default_prompt,
            continue_prompt,
        };
        execute!(
            editor.sink,
            cursor::SetCursorStyle::BlinkingBar,
            cursor::MoveToColumn(0),
            style::Print(format!("🖐 Press {} to exit.",  "Ctrl-D".magenta())),
            style::Print("\n"),
        )?;
        Ok(editor)
    }
}

impl<'eval, W: Write> Editor<'eval, W> {
    pub fn run_event_loop(&mut self) -> ReplBlockResult<()> {
        loop {
            let old_height = self.height;
            self.dispatch_key_event()?; // This might alter `self.height`
            self.render_ui(old_height)?;
        }
    }

    fn dispatch_key_event(&mut self) -> ReplBlockResult<()> {
        match event::read()? {
            Event::Key(key!(CONTROL-'c')) => self.cmd_nop()?,

            // Control application lifecycle:
            Event::Key(key!(CONTROL-'d')) => self.cmd_exit_repl()?,
            Event::Key(key!(@name Enter)) => self.cmd_eval()?,

            // Navigation:
            Event::Key(key!(CONTROL-'p')) => self.cmd_nav_history_up()?,
            Event::Key(key!(@name Up))    => self.cmd_nav_history_up()?,
            Event::Key(key!(CONTROL-'n')) => self.cmd_nav_history_down()?,
            Event::Key(key!(@name Down))  => self.cmd_nav_history_down()?,
            Event::Key(key!(CONTROL-'b')) => self.cmd_nav_cmd_left()?,
            Event::Key(key!(@name Left))  => self.cmd_nav_cmd_left()?,
            Event::Key(key!(CONTROL-'f')) => self.cmd_nav_cmd_right()?,
            Event::Key(key!(@name Right)) => self.cmd_nav_cmd_right()?,
            Event::Key(key!(CONTROL-'a')) => self.cmd_nav_to_start_of_cmd()?,
            Event::Key(key!(@name Home))  => self.cmd_nav_to_start_of_cmd()?,
            Event::Key(key!(CONTROL-'e')) => self.cmd_nav_to_end_of_cmd()?,
            Event::Key(key!(@name End))   => self.cmd_nav_to_end_of_cmd()?,

            // TODO remove both key bindings
            // Mainly useful for debugging:
            Event::Key(key!(@name ALT-Up)) => {
                // execute!(self.sink, cursor::MoveUp(1))?;
                execute!(self.sink, terminal::ScrollUp(1))?;
            },
            Event::Key(key!(@name ALT-Down)) => {
                execute!(self.sink, terminal::ScrollDown(1))?;
                // execute!(self.sink, cursor::MoveDown(1))?;
            },

            // Editing;
            Event::Key(key!(@c))                => self.cmd_insert_char(c)?,
            Event::Key(key!(SHIFT-@c))          => self.cmd_insert_char(c)?,
            // FIXME `SHIFT+Enter` doesn't work for...reasons(??),
            //       yet `CONTROL-o` works as expected:
            Event::Key(key!(@name SHIFT-Enter)) => self.cmd_insert_newline()?,
            Event::Key(key!(CONTROL-'o'))       => self.cmd_insert_newline()?,
            Event::Key(key!(@name Backspace)) =>
                self.cmd_rm_grapheme_before_cursor()?,
            Event::Key(key!(@name Delete)) =>
                self.cmd_rm_grapheme_at_cursor()?,

            _event => {/* ignore the event */},
        }
        Ok(())
    }

    fn render_ui(&mut self, old_editor_height: u16) ->ReplBlockResult<()> {
        let editor_dims = self.dimensions()?;
        let prompt_len = self.prompt_len();
        let calculate_uncursor = |cmd: &Cmd, cursor: Coords| {
            if cursor == Coords::EDITOR_ORIGIN {
                return Coords { x: prompt_len, y: Coords::EDITOR_ORIGIN.y };
            }

            let prev_unlines: Vec<Vec<Line>> = (0..cursor.y)
                .map(|y| cmd[y].uncompress(editor_dims.width, prompt_len))
                .collect();
            let mut y = prev_unlines.iter()
                .map(|unline| unline.len())
                .sum::<usize>() as u16;

            let mut x = cursor.x;
            let line = &cmd[cursor.y];
            for unline in line.uncompress(editor_dims.width, prompt_len) {
                // let unline_len = unline.count_graphemes();
                if unline.is_start() {
                    x += prompt_len;
                }
                if x >= editor_dims.width {
                    // x -= unline_len;
                    x -= editor_dims.width;
                    y += 1;
                }
            }
            Coords { x, y }
        };
        match &self.state {
            State::Edit(EditState { buffer, cursor }) => {
                let cursor = *cursor; // Be like water: avoid borrowck complaint
                let uncompressed = buffer.uncompress(editor_dims.width, prompt_len);
                let num_uncompressed_lines = uncompressed.count_lines() as u16;
                self.height = std::cmp::max(self.height, num_uncompressed_lines);

                // Obtain an `uncompressed` version of `cursor`
                let uncursor = calculate_uncursor(buffer, cursor);

                // Scroll up the old output *BEFORE* clearing the input area
                for _ in old_editor_height as usize .. uncompressed.count_lines() {
                    queue!(self.sink, terminal::ScrollUp(1))?;
                }

                terminal::disable_raw_mode()?;
                execute!(
                    self.sink,
                    // cursor::MoveUp(40),
                    cursor::MoveUp(terminal::size().unwrap().1),
                    cursor::MoveToColumn(0),
                    terminal::Clear(ClearType::All),
                    style::Print(format!("BUFFER: {buffer:#?}\n")),
                    style::Print(format!("UNCOMPRESSED: {uncompressed:#?}\n")),
                    style::Print(format!("CURSOR: {cursor}\n")),
                    style::Print(format!("UNCURSOR: {uncursor}\n")),
                    style::Print(format!("TERM DIMS: {:?}\n", terminal::size()?)),
                    style::Print(format!("EDITOR DIMS: {editor_dims:?}\n")),
                    // cursor::MoveDown(40),
                    cursor::MoveDown(terminal::size().unwrap().1),
                )?;
                terminal::enable_raw_mode()?;

                // Clear and prepare the input area
                self.clear_input_area(FlushPolicy::NoFlush)?;
                self.move_cursor_to_origin(FlushPolicy::NoFlush)?;

                // Render the lines of the `uncompressed` cmd
                for (lidx, lline) in uncompressed.lines().iter().enumerate() {
                    if lidx == 0 {
                        self.write_default_prompt(FlushPolicy::NoFlush)?;
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                            cursor::MoveToColumn(0),
                        )?;
                    } else if lline.is_start() {
                        self.write_continue_prompt(FlushPolicy::NoFlush)?;
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                        )?;
                    } else {
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                            cursor::MoveToColumn(0),
                        )?;
                    }
                }

                // Rendser the uncursor
                let o = self.origin()?;
                queue!(
                    self.sink,
                    // cursor::MoveTo(o.x + uncursor.x, o.y + uncursor.y),
                    cursor::MoveToColumn(o.x + uncursor.x),
                    cursor::MoveToRow(o.y + uncursor.y),
                )?;


                // { // Clear and prepare the input area
                //     self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
                //     self.clear_input_area(FlushPolicy::NoFlush)?;
                //     self.write_default_prompt(FlushPolicy::NoFlush)?;
                // }
                // for (lidx, lline) in uncompressed.lines().iter().enumerate() {
                //     if lidx == 0 {
                //         queue!(
                //             self.sink,
                //             style::Print(lline),
                //             // cursor::MoveTo(end_of_cmd.x, end_of_cmd.y),
                //             cursor::MoveDown(1),
                //             // cursor::MoveToColumn(0),
                //         )?;
                //     } else {
                //         if lline.kind == LineKind::Continue {
                //             queue!(
                //                 self.sink,
                //                 cursor::MoveToColumn(0),
                //                 style::Print(lline),
                //                 // style::Print(format!("FOOOOOO: '{}'", lline)),
                //                 // cursor::MoveUp(1),
                //                 // cursor::MoveTo(end_of_cmd.x, end_of_cmd.y),
                //                 cursor::MoveDown(1),
                //                 cursor::MoveToColumn(0),
                //             )?;
                //         } else {
                //             self.write_continue_prompt(FlushPolicy::NoFlush)?;
                //             queue!(
                //                 self.sink,
                //                 style::Print(lline),
                //                 // style::Print(format!("FOOOOOO: '{}'", lline)),
                //                 // cursor::MoveUp(1),
                //                 // cursor::MoveTo(end_of_cmd.x, end_of_cmd.y),
                //                 cursor::MoveDown(1),
                //                 cursor::MoveToColumn(0),
                //                 // cursor::MoveToColumn(0),
                //             )?;
                //         }
                //     }
                // }



                // { // Clear and prepare the input area
                //     self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
                //     self.clear_input_area(FlushPolicy::NoFlush)?;
                //     self.write_default_prompt(FlushPolicy::NoFlush)?;
                // }
                // for (lidx, lline) in llines.iter().enumerate() {
                //     queue!(
                //         self.sink,
                //         style::Print(lline),
                //         cursor::MoveDown(1),
                //         cursor::MoveToColumn(0),
                //     )?;
                // }


                // self.move_cursor_to(FlushPolicy::NoFlush, cursor)?;

            }
            State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {
                let cursor = *cursor; // Be like water: avoid borrowck complaint
                let uncompressed = preview.uncompress(editor_dims.width, prompt_len);
                let num_uncompressed_lines = uncompressed.count_lines() as u16;
                self.height = std::cmp::max(self.height, num_uncompressed_lines);

                // Obtain an `uncompressed` version of `cursor`
                let uncursor = calculate_uncursor(preview, cursor);

                // Scroll up the old output *BEFORE* clearing the input area
                for _ in old_editor_height as usize .. uncompressed.count_lines() {
                    queue!(self.sink, terminal::ScrollUp(1))?;
                }

                terminal::disable_raw_mode()?;
                execute!(
                    self.sink,
                    // cursor::MoveUp(40),
                    cursor::MoveUp(terminal::size().unwrap().1),
                    cursor::MoveToColumn(0),
                    terminal::Clear(ClearType::All),
                    style::Print(format!("PREVIEW: {preview:#?}\n")),
                    style::Print(format!("UNCOMPRESSED: {uncompressed:#?}\n")),
                    style::Print(format!("CURSOR: {cursor}\n")),
                    style::Print(format!("UNCURSOR: {uncursor}\n")),
                    style::Print(format!("TERM DIMS: {:?}\n", terminal::size()?)),
                    style::Print(format!("EDITOR DIMS: {editor_dims:?}\n")),
                    // cursor::MoveDown(40),
                    cursor::MoveDown(terminal::size().unwrap().1),
                )?;
                terminal::enable_raw_mode()?;

                // Clear and prepare the input area
                self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
                self.clear_input_area(FlushPolicy::NoFlush)?;
                // self.write_default_prompt(FlushPolicy::NoFlush)?;

                // for uline in uncompressed.lines().iter() {
                //     queue!(
                //         self.sink,
                //         style::Print(uline),
                //         cursor::MoveDown(1),
                //         cursor::MoveToColumn(0),
                //     )?;
                // }
                for (lidx, lline) in uncompressed.lines().iter().enumerate() {
                    if lidx == 0 {
                        self.write_default_prompt(FlushPolicy::NoFlush)?;
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                            cursor::MoveToColumn(0),
                        )?;
                    } else if lline.is_start() {
                        self.write_continue_prompt(FlushPolicy::NoFlush)?;
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                        )?;
                    } else {
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                            cursor::MoveToColumn(0),
                        )?;
                    }
                }

                // Rendser the uncursor
                let o = self.origin()?;
                queue!(
                    self.sink,
                    // cursor::MoveTo(o.x + uncursor.x, o.y + uncursor.y),
                    cursor::MoveToColumn(o.x + uncursor.x),
                    cursor::MoveToRow(o.y + uncursor.y),
                )?;

                // self.move_cursor_to(FlushPolicy::NoFlush, cursor)?;


                // if let Some(last) = lines.last() {
                //     self.move_cursor_to(FlushPolicy::NoFlush, Coords {
                //         x: if lines.len() == 1 {
                //             prompt_len + last.count_graphemes()
                //         } else {
                //             last.count_graphemes()
                //         },
                //         y: lines.len() as u16 - 1,
                //     })?;
                // }


            }
        }
        self.sink.flush()?;
        Ok(())
    }

    fn write_default_prompt(
        &mut self,
        flush_policy: FlushPolicy,
    ) -> ReplBlockResult<&mut Self> {
        queue!(self.sink, cursor::MoveToColumn(0))?;
        for i in 0..self.default_prompt.len() {
            queue!(self.sink, style::Print(self.default_prompt[i]))?;
        }
        if let FlushPolicy::Flush = flush_policy {
            self.sink.flush()?;
        }
        Ok(self)
    }

    fn write_continue_prompt(
        &mut self,
        flush_policy: FlushPolicy,
    ) -> ReplBlockResult<()> {
        queue!(self.sink, cursor::MoveToColumn(0))?;
        for c in &self.continue_prompt {
            queue!(self.sink, style::Print(c))?;
        }
        if let FlushPolicy::Flush = flush_policy {
            self.sink.flush()?;
        }
        Ok(())
    }

    fn move_cursor_to(
        &mut self,
        flush_policy: FlushPolicy,
        target: Coords,
    ) -> ReplBlockResult<()> {
        let origin = self.origin()?;
        // queue!(self.sink, cursor::MoveTo(origin.x + target.x, origin.y + target.y))?;
        queue!(self.sink, cursor::MoveToColumn(origin.x + target.x))?;
        queue!(self.sink, cursor::MoveToRow(origin.y + target.y))?;
        if let FlushPolicy::Flush = flush_policy {
            self.sink.flush()?;
        }
        Ok(())
    }

    fn move_cursor_to_origin(
        &mut self,
        flush_policy: FlushPolicy,
    ) -> ReplBlockResult<()> {
        let origin = self.origin()?;
        queue!(self.sink, cursor::MoveTo(origin.x, origin.y))?;
        if let FlushPolicy::Flush = flush_policy {
            self.sink.flush()?;
        }
        Ok(())
    }

    fn clear_input_area(
        &mut self,
        flush_policy: FlushPolicy,
    ) -> ReplBlockResult<()> {
        self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
        for _ in 0..self.height {
            queue!(self.sink, terminal::Clear(ClearType::CurrentLine))?;
            queue!(self.sink, cursor::MoveDown(1))?;
        }
        self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
        if let FlushPolicy::Flush = flush_policy {
            self.sink.flush()?;
        }
        Ok(())
    }


    /// Return the global (col, row)-coordinates of the top-left corner of `self`.
    fn origin(&self) -> ReplBlockResult<Coords> {
        let (_term_width, term_height) = terminal::size()?;
        Ok(Coords { x: 0, y: term_height - self.height })
    }

    // /// Return the (col, row)-coordinates of the cursor,
    // /// relative to the top-left corner of `self`.
    // /// The top left cell is represented as `(0, 0)`.
    // fn cursor_position(&self) -> ReplBlockResult<Coords> {
    //     self.cursor_position_relative_to(self.origin()?)
    // }

    // /// Query the global cursor position coordinates, then translate
    // /// them to be relative to the `(x, y)` coordinates.
    // fn cursor_position_relative_to(
    //     &self,
    //     origin: Coords,
    // ) -> ReplBlockResult<Coords> {
    //     let (cx, cy) = cursor::position()?;
    //     Ok(Coords { x: cx - origin.x, y: cy - origin.y })
    // }


    /// Return the (width, height) dimensions of `self`.
    /// The top left cell is represented `(1, 1)`.
    fn dimensions(&self) -> ReplBlockResult<Dims> {
        let (term_width, _term_height) = terminal::size()?;
        Ok(Dims { width: term_width, height: self.height })
    }

    fn prompt_len(&self) -> u16 {
        assert_eq!(
            self.default_prompt.len(), self.continue_prompt.len(),
            "PRECONDITION FAILED: default_prompt.len() != continue_prompt.len()"
        );
        self.default_prompt.len() as u16
    }



    fn cmd_nop(&mut self) -> ReplBlockResult<()> {
        Ok(()) // NOP
    }

    /// Exit the REPL
    fn cmd_exit_repl(&mut self) -> ReplBlockResult<()> {
        execute!(
            self.sink,
            cursor::SetCursorStyle::DefaultUserShape,
            cursor::MoveToColumn(0),
            style::Print("👋"),
            terminal::Clear(ClearType::FromCursorDown),
        )?;
        terminal::disable_raw_mode()?; // Undo raw mode from start of program
        self.sink.flush()?;
        std::process::exit(0);
    }

    /// Navigate up in the History
    fn cmd_nav_history_up(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor: _ }) => {
                let Some(max_hidx) = self.history.max_idx() else {
                    return Ok(()); // NOP: no history to navigate
                };
                self.state = State::Navigate(NavigateState {
                    hidx: max_hidx,
                    backup: std::mem::take(buffer),
                    preview: self.history[max_hidx].clone(),
                    cursor: self.history[max_hidx].end_of_cmd(),
                });
            }
            State::Navigate(NavigateState { hidx, backup: _, preview, cursor }) => {
                let min_hidx = HistIdx(0);
                if *hidx == min_hidx {
                    // NOP, at the top of the History
                } else {
                    *hidx -= 1;
                    *preview = self.history[*hidx].clone(); // update
                    *cursor = preview.end_of_cmd();
                }
            }
        }
        Ok(())
    }

    fn cmd_nav_history_down(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { .. }) => {/* NOP */}
            State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {
                let max_hidx = self.history.max_idx();
                if Some(*hidx) == max_hidx { // bottom-of-history
                    self.state = State::Edit(EditState {
                        cursor: backup.end_of_cmd(),
                        buffer: std::mem::take(backup),
                    });
                } else {
                    *hidx += 1;
                    *preview = self.history[*hidx].clone(); // update
                    //*cursor = preview.end_of_cmd();
                    *cursor = Coords::EDITOR_ORIGIN;
                }
            }
        }
        Ok(())
    }

    fn cmd_nav_cmd_left(&mut self) -> ReplBlockResult<()> {
        let editor_dims = self.dimensions()?;
        let prompt_len = self.prompt_len();
        let update_cursor = |cmd: &Cmd, cursor: &mut Coords| {
            let CursorFlags {
                is_top_cmd_line,
                is_start_of_cmd_line,
                ..
            } = cursor.flags(editor_dims, prompt_len, cmd);
            if is_top_cmd_line && is_start_of_cmd_line {
                // NOP
            } else if is_top_cmd_line && !is_start_of_cmd_line {
                cursor.x -= 1;
            } else if !is_top_cmd_line && is_start_of_cmd_line {
                *cursor = Coords {
                    x: cmd[cursor.y - 1].max_x(),
                    y: cursor.y - 1,
                };
            } else if !is_top_cmd_line && !is_start_of_cmd_line {
                cursor.x -= 1;
            } else {
                panic!("[cmd_nav_cmd_left] Illegal cursor position: {cursor}");
            }
        };
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                update_cursor(buffer, cursor);
            },
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                update_cursor(preview, cursor);
            },
        }
        Ok(())
    }

    fn cmd_nav_cmd_right(&mut self) -> ReplBlockResult<()> {
        let editor_dims = self.dimensions()?;
        let prompt_len = self.prompt_len();
        let update_cursor = |cmd: &Cmd, cursor: &mut Coords| {
            let CursorFlags {
                is_bottom_cmd_line,
                is_end_of_cmd_line,
                ..
            } = cursor.flags(editor_dims, prompt_len, cmd);
            if is_bottom_cmd_line && is_end_of_cmd_line {
                // NOP
            } else if is_bottom_cmd_line && !is_end_of_cmd_line {
                cursor.x += 1;
            } else if !is_bottom_cmd_line && is_end_of_cmd_line {
                *cursor = Coords {
                    x: Coords::EDITOR_ORIGIN.x,
                    y: cursor.y + 1,
                };
            } else if !is_bottom_cmd_line && !is_end_of_cmd_line {
                cursor.x += 1;
            } else {
                panic!("[cmd_nav_cmd_right] Illegal cursor position: {cursor}");
            }
        };
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                update_cursor(buffer, cursor);
            },
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                update_cursor(preview, cursor);
            },
        }
        Ok(())
    }

    /// Navigate to the start of the current Cmd
    fn cmd_nav_to_start_of_cmd(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { cursor, .. }) => {
                *cursor = Coords::EDITOR_ORIGIN;
            },
            State::Navigate(NavigateState { cursor, .. }) => {
                *cursor = Coords::EDITOR_ORIGIN;
            },
        }
        Ok(())
    }

    /// Navigate to the end of the current Cmd
    fn cmd_nav_to_end_of_cmd(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                *cursor = buffer.end_of_cmd();
            },
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                *cursor = preview.end_of_cmd();
            },
        }
        Ok(())
    }

    /// Insert a char into the current cmd at cursor position.
    fn cmd_insert_char(&mut self, c: char) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                buffer.insert_char(*cursor, c);
                cursor.x += 1;
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_insert_char(c)?;
            }
        }
        Ok(())
    }

    /// Add a newline to the current cmd
    fn cmd_insert_newline(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                buffer.push_empty_line(LineKind::Start);
                *cursor = Coords {
                    x: Coords::EDITOR_ORIGIN.x,
                    y: cursor.y + 1
                };
            }
            State::Navigate(NavigateState { preview, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: preview.end_of_cmd(),
                });
                self.cmd_insert_newline()?;
            }
        }
        Ok(())
    }

    /// Delete the grapheme before the cursor in of the current cmd
    /// Do nothing if there if there is no grapheme before the cursor.
    fn cmd_rm_grapheme_before_cursor(&mut self) -> ReplBlockResult<()> {
        let editor_dims = self.dimensions()?;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                if buffer.is_empty() {
                    return Ok(()); // NOP
                }
                buffer.rm_grapheme_before(
                    *cursor,
                    editor_dims,
                    prompt_len,
                );
                self.cmd_nav_cmd_left()?;
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_rm_grapheme_before_cursor()?;
            }
        }
        Ok(())
    }

    /// Delete the grapheme at the position of the cursor in of the current cmd.
    /// Do nothing if there if there is no grapheme at the cursor.
    fn cmd_rm_grapheme_at_cursor(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                if buffer.is_empty() {
                    return Ok(()); // NOP
                }
                buffer.rm_grapheme_at(*cursor);
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_rm_grapheme_at_cursor()?;
            }
        }
        Ok(())
    }

    /// Execute the current cmd
    fn cmd_eval(&mut self) -> ReplBlockResult<()> {
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                #[allow(unstable_name_collisions)]
                let source_code = buffer.lines().iter()
                    .filter(|line| !line.is_empty())
                    .map(Line::as_str)
                    .intersperse("\n")
                    .collect::<String>();
                if source_code.is_empty() {
                    return Ok(());
                }
                let cmd = std::mem::take(buffer);
                let _hidx = self.history.add_cmd(cmd);
                self.history.write_to_file(&self.history_filepath)?;
                {   // Evaluation can and usually does produce some output,
                    // and that will be garbled if written in raw mode
                    terminal::disable_raw_mode()?;
                    (*self.evaluator)(source_code.as_str())?;
                    terminal::enable_raw_mode()?;
                }
                self.height = 1; // reset
                *cursor = Coords { x: prompt_len, y: 0 };
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_eval()?;
            }
        }
        Ok(())
    }

}

#[derive(Clone, Copy,  Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dims { pub width: u16, pub height: u16 }

#[derive(Clone, Copy,  Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coords { pub x: u16, pub y: u16 }

impl Coords {
    pub(crate) const EDITOR_ORIGIN: Self = Self { x: 0, y: 0 };

    pub fn is_origin(&self) -> bool {
        *self == Self::EDITOR_ORIGIN
    }

    pub fn flags(
        &self,
        editor_dims: Dims,
        prompt_len: u16,
        cmd: &Cmd,
    ) -> CursorFlags {
        let is_top_cmd_line = self.y == 0;
        let is_bottom_cmd_line = self.y == cmd.count_lines() as u16 - 1;
        // let offset = if is_top_cmd_line || cmd[self.y].is_start() {
        //     prompt_len
        // } else {
        //     0
        // };
        let offset = 0;
        let is_start_of_cmd_line = self.x == offset;
        // let is_end_of_cmd_line   = self.x == offset + cmd[self.y].count_graphemes();
        let is_end_of_cmd_line = *self == cmd.end_of_cmd();

        const ORIGIN: Coords = Coords::EDITOR_ORIGIN;
        let is_top_editor_row = self.y == ORIGIN.y;
        let is_bottom_editor_row = self.y == ORIGIN.y + editor_dims.height;
        let is_leftmost_editor_column = self.x == ORIGIN.x;
        let is_rightmost_editor_column = self.x == ORIGIN.x + editor_dims.width - 1;

        // terminal::disable_raw_mode().unwrap();
        // println!();
        // println!("cursor={self:?}");
        // println!("is_top_cmd_line={is_top_cmd_line}");
        // println!("is_bottom_cmd_line={is_bottom_cmd_line}");
        // println!("line kine={}", cmd[self.y].kind);
        // println!("is_start_of_cmd_line={is_start_of_cmd_line}");
        // println!("is_end_of_cmd_line={is_end_of_cmd_line}");
        // println!("is_top_editor_row={is_top_editor_row}");
        // println!("is_bottom_editor_row={is_bottom_editor_row}");
        // println!("is_leftmost_editor_column={is_leftmost_editor_column}");
        // println!("is_rightmost_editor_column={is_rightmost_editor_column}");
        // terminal::enable_raw_mode().unwrap();

        CursorFlags {
            is_top_cmd_line,
            is_bottom_cmd_line,
            is_start_of_cmd_line,
            is_end_of_cmd_line,
            is_top_editor_row,
            is_bottom_editor_row,
            is_leftmost_editor_column,
            is_rightmost_editor_column,
            // is_continuation: cmd[self.y].is_continue(),
        }
    }
}

impl std::fmt::Display for Coords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

impl std::ops::Add<Self> for Coords {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}

impl std::ops::Sub<Self> for Coords {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self { x: self.x - rhs.x, y: self.y - rhs.y }
    }
}


pub struct CursorFlags {
    /// Set to `true` iff. the Coords are in the top Cmd line.
    pub(crate) is_top_cmd_line: bool,
    /// Set to `true` iff. the Coords are in the bottom Cmd line.
    pub(crate) is_bottom_cmd_line: bool,
    /// Set to `true` iff. the Coords are at the start of a Cmd line.
    pub(crate) is_start_of_cmd_line: bool,
    /// Set to `true` iff. the Coords are at the end of a Cmd line.
    pub(crate) is_end_of_cmd_line: bool,
    /// Set to `true` iff. the Coords are in the top editor row.
    pub(crate) is_top_editor_row: bool,
    /// Set to `true` iff. the Coords are in the bottom editor row.
    pub(crate) is_bottom_editor_row: bool,
    /// Set to `true` iff. the Coords are in the leftmost editor column.
    pub(crate) is_leftmost_editor_column: bool,
    /// Set to `true` iff. the Coords are in the rightmost editor column.
    pub(crate) is_rightmost_editor_column: bool,
    // pub(crate) is_continuation: bool,
}

#[derive(Debug)]
enum State {
    Edit(EditState),
    Navigate(NavigateState),
}

impl State {
    fn as_edit(&self) -> ReplBlockResult<&EditState> {
        match &self {
            Self::Edit(es) => Ok(es),
            Self::Navigate(_) => panic!("Expected State::Edit(_); Got {self:?}"),
        }
    }

    fn as_navigate(&self) -> ReplBlockResult<&NavigateState> {
        match &self {
            Self::Edit(_) => panic!("Expected State::Nsvigate(_); Got {self:?}"),
            Self::Navigate(ns) => Ok(ns),
        }
    }
}

/// Editing a `Cmd`
#[derive(Debug)]
struct EditState {
    /// A buffer containing the cmd being edited
    buffer: Cmd,
    /// The cursor position within the Cmd buffer
    cursor: Coords,
}

/// Navigating through the `History`
#[derive(Debug)]
struct NavigateState {
    /// Points to the History cmd being previewed
    hidx: HistIdx,
    /// A buffer containing the cmd that was last edited
    backup: Cmd,
    /// The `History` entry being previewed
    preview: Cmd,
    /// The cursor position within the Cmd preview buffer
    cursor: Coords,
}


#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlushPolicy {
    Flush,
    NoFlush,
}
