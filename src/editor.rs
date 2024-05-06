//!

use crate::{
    cmd::{Cmd, Line},
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

pub struct EditorBuilder<'eval, const N: usize, W: Write> {
    sink: W,
    default_prompt: [StyledContent<char>; N],
    continue_prompt: [StyledContent<char>; N],
    history_filepath: Utf8PathBuf,
    evaluator: Box<Evaluator<'eval>>,
}

impl<'eval> Default for EditorBuilder<'eval, 3, Stdout> {
    fn default() -> EditorBuilder<'eval, 3, Stdout> {
        #[inline(always)]
        fn nop<'eval>() -> Box<Evaluator<'eval>> {
            Box::new(|_| Ok(()))
        }
        EditorBuilder {
            sink: std::io::stdout(),
            default_prompt:  ['■'.yellow(), '>'.green().bold(), ' '.reset()],
            continue_prompt: ['ꞏ'.yellow(), 'ꞏ'.yellow(),       ' '.reset()],
            history_filepath: Utf8PathBuf::new(),
            evaluator: nop(),
        }
    }
}

impl<'eval, const N: usize, W: Write> EditorBuilder<'eval, N, W> {
    pub fn sink<S: Write>(self, sink: S) -> EditorBuilder<'eval, N, S> {
        EditorBuilder {
            sink,
            default_prompt: self.default_prompt,
            continue_prompt: self.continue_prompt,
            history_filepath: self.history_filepath,
            evaluator: self.evaluator,
        }
    }

    pub fn default_prompt(mut self, prompt: [StyledContent<char>; N]) -> Self {
        self.default_prompt = prompt;
        self
    }

    pub fn continue_prompt(mut self, prompt: [StyledContent<char>; N]) -> Self {
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

    pub fn build(self) -> ReplBlockResult<Editor<'eval, N, W>> {
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
pub struct Editor<'eval, const N: usize, W: Write> {
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
    default_prompt: [StyledContent<char>; N],
    /// The command prompt used for command continuations
    continue_prompt: [StyledContent<char>; N],
}

impl<'eval, const N: usize, W: Write> Editor<'eval, N, W> {
    fn new(
        mut sink: W,
        history_filepath: impl AsRef<Utf8Path>,
        evaluator: Box<Evaluator<'eval>>,
        default_prompt: [StyledContent<char>; N],
        continue_prompt: [StyledContent<char>; N],
    ) -> ReplBlockResult<Editor<'eval, N, W>> {
        sink.flush()?;
        let mut editor = Self {
            sink,
            state: State::Edit(EditState {
                buffer: Cmd::default(),
                cursor: Coords::ORIGIN,
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
            style::Print(format!("Press {} to exit.",  "Ctrl-D".magenta())),
            style::Print("\n"),
        )?;
        Ok(editor)
    }
}

impl<'eval, const N: usize, W: Write> Editor<'eval, N, W> {
    pub fn run_event_loop(&mut self) -> ReplBlockResult<()> {
        loop {
            let old_height = self.height;
            match event::read()? {
                Event::Key(key!(CONTROL-'c'))    => self.cmd_nop()?,

                // Control application lifecycle:
                Event::Key(key!(CONTROL-'d'))    => self.cmd_exit_repl()?,
                Event::Key(key!(@special Enter)) => self.cmd_eval()?,

                // Navigation:
                Event::Key(key!(CONTROL-'p'))    => self.cmd_nav_history_up()?,
                Event::Key(key!(@special Up))    => self.cmd_nav_history_up()?,
                Event::Key(key!(CONTROL-'n'))    => self.cmd_nav_history_down()?,
                Event::Key(key!(@special Down))  => self.cmd_nav_history_down()?,
                Event::Key(key!(CONTROL-'b'))    => self.cmd_nav_cmd_left()?,
                Event::Key(key!(@special Left))  => self.cmd_nav_cmd_left()?,
                Event::Key(key!(CONTROL-'f'))    => self.cmd_nav_cmd_right()?,
                Event::Key(key!(@special Right)) => self.cmd_nav_cmd_right()?,
                Event::Key(key!(CONTROL-'a'))    => self.cmd_nav_to_start_of_cmd()?,
                Event::Key(key!(@special Home))  => self.cmd_nav_to_start_of_cmd()?,
                Event::Key(key!(CONTROL-'e'))    => self.cmd_nav_to_end_of_cmd()?,
                Event::Key(key!(@special End))   => self.cmd_nav_to_end_of_cmd()?,

                // TODO remove both key bindings
                // Mainly useful for debugging:
                Event::Key(key!(@special ALT-Up)) => {
                    // execute!(self.sink, cursor::MoveUp(1))?;
                    execute!(self.sink, terminal::ScrollUp(1))?;
                },
                Event::Key(key!(@special ALT-Down)) => {
                    execute!(self.sink, terminal::ScrollDown(1))?;
                    // execute!(self.sink, cursor::MoveDown(1))?;
                },

                // Editing;
                Event::Key(key!(@c))       => self.cmd_insert_char(c)?,
                Event::Key(key!(SHIFT-@c)) => self.cmd_insert_char(c)?,
                // FIXME `SHIFT+Enter` doesn't work for...reasons(??),
                //       yet `CONTROL-o` works as expected:
                Event::Key(key!(CONTROL-'o')) => self.cmd_insert_newline()?,
                Event::Key(key!(@special Backspace)) => {
                    let cursor_position = self.cursor_position()?;
                    self.cmd_rm_char(cursor_position)?
                },

                _event => {
                    //     execute!(
                    //         self.sink,
                    //         style::Print(format!("event={event:#?}")),
                    //         style::Print("\n"),
                    //     )?;
                }
            }

            let editor_width = self.dimensions()?.width;
            let prompt_len = self.prompt_len();
            match &self.state {
                State::Edit(EditState { buffer, cursor }) => {
                    let lines = buffer.logical_lines(editor_width, prompt_len);
                    let cursor = *cursor;

                    self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
                    self.clear_input_area(FlushPolicy::NoFlush)?;
                    self.write_default_prompt(FlushPolicy::NoFlush)?;

                    for line in &lines {
                        queue!(
                            self.sink,
                            style::Print(line),
                            cursor::MoveDown(1),
                            cursor::MoveToColumn(0),
                            //cursor::SavePosition,
                        )?;
                    }
                    // queue!(self.sink, cursor::RestorePosition)?;

                    self.move_cursor_to(FlushPolicy::NoFlush, cursor)?;

                }
                State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {
                    let llines = preview.logical_lines(editor_width, prompt_len);
                    let cursor = *cursor;

                    // Scroll up the old output *BEFORE* clearing the input area
                    for _ in old_height as usize .. llines.len() {
                        queue!(self.sink, terminal::ScrollUp(1))?;
                    }

                    self.move_cursor_to_origin(FlushPolicy::NoFlush)?;
                    self.clear_input_area(FlushPolicy::NoFlush)?;
                    self.write_default_prompt(FlushPolicy::NoFlush)?;

                    for lline in &llines {
                        queue!(
                            self.sink,
                            style::Print(lline),
                            cursor::MoveDown(1),
                            cursor::MoveToColumn(0),
                        )?;
                    }

                    self.move_cursor_to(FlushPolicy::NoFlush, cursor)?;

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
        }
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
        for i in 0..self.continue_prompt.len() {
            queue!(self.sink, style::Print(self.continue_prompt[i]))?;
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

    /// Return the (col, row)-coordinates of the cursor,
    /// relative to the top-left corner of `self`.
    /// The top left cell is represented as `(0, 0)`.
    fn cursor_position(&self) -> ReplBlockResult<Coords> {
        self.cursor_position_relative_to(self.origin()?)
    }

    /// Query the global cursor position coordinates, then translate
    /// them to be relative to the `(x, y)` coordinates.
    fn cursor_position_relative_to(
        &self,
        origin: Coords,
    ) -> ReplBlockResult<Coords> {
        let (cx, cy) = cursor::position()?;
        Ok(Coords { x: cx - origin.x, y: cy - origin.y })
    }


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
            style::Print("Exiting"),
        )?;
        terminal::disable_raw_mode()?; // Undo raw mode from start of program
        self.sink.flush()?;
        std::process::exit(0);
    }

    /// Navigate up in the History
    fn cmd_nav_history_up(&mut self) -> ReplBlockResult<()> {
        let editor_width = self.dimensions()?.width;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer, cursor: _ }) => {
                let Some(max_hidx) = self.history.max_idx() else {
                    return Ok(()); // NOP: no history to navigate
                };
                self.state = State::Navigate(NavigateState {
                    hidx: max_hidx,
                    backup: std::mem::take(buffer),
                    preview: self.history[max_hidx].clone(),
                    cursor: self.history[max_hidx]
                        .end_of_cmd_cursor(editor_width, prompt_len),
                });
                let height = self.state.as_navigate()?.preview
                    .count_logical_lines(editor_width, prompt_len);
                self.height = std::cmp::max(self.height, height); // update
            }
            State::Navigate(NavigateState { hidx, backup: _, preview, cursor }) => {
                let min_hidx = HistIdx(0);
                if *hidx == min_hidx {
                    // NOP, at the top of the History
                } else {
                    *hidx -= 1;
                    *preview = self.history[*hidx].clone(); // update
                    *cursor = self.history[*hidx]
                        .end_of_cmd_cursor(editor_width, prompt_len); // reset
                    let height = preview
                        .count_logical_lines(editor_width, prompt_len);
                    self.height = std::cmp::max(self.height, height); // update
                }
            }
        }
        Ok(())
    }

    fn cmd_nav_history_down(&mut self) -> ReplBlockResult<()> {
        let prompt_len = self.prompt_len();
        let editor_width = self.dimensions()?.width;
        match &mut self.state {
            State::Edit(EditState { .. }) => {/* NOP */}
            State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {
                let max_hidx = self.history.max_idx();
                if Some(*hidx) == max_hidx { // bottom-of-history
                    self.state = State::Edit(EditState {
                        cursor: backup.end_of_cmd_cursor(editor_width, prompt_len),
                        buffer: std::mem::take(backup),
                    });
                    let buffer_height = self.state.as_edit()?.buffer
                        .count_logical_lines(editor_width, prompt_len);
                    self.height = std::cmp::max(self.height, buffer_height);
                } else {
                    *hidx += 1;
                    *preview = self.history[*hidx].clone(); // update
                    *cursor = self.history[*hidx]
                        .end_of_cmd_cursor(editor_width, prompt_len); // reset
                    let preview_height = preview
                        .count_logical_lines(editor_width, prompt_len);
                    self.height = std::cmp::max(self.height, preview_height);
                }
            }
        }
        Ok(())
    }

    fn cmd_nav_cmd_left(&mut self) -> ReplBlockResult<()> {
        let editor_width = self.dimensions()?.width;
        let prompt_len = self.prompt_len();
        let origin = self.origin()?;
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {

                todo!("[cmd_nav_cmd_left]");

                // if cursor.y == 0 && cursor.x == prompt_len {
                //     // NOP: Thou shalt not pass!
                // } else if cursor.y > 0 && cursor.x == 0 {
                //     // NOP: Thou shalt not pass!
                // } else {
                //     cursor.x = cursor.x.saturating_sub(1);
                // }

            },
            State::Navigate(NavigateState { hidx: _, backup: _, preview, cursor }) => {
                let is_prompt_line = cursor.y == 0;
                if is_prompt_line && cursor.x == prompt_len {
                    // NOP: At the start of the prompt line
                } else if !is_prompt_line && cursor.x == origin.x {
                    let llines = preview.logical_lines(editor_width, prompt_len);
                    let dst_y = cursor.y.saturating_sub(1);
                    let offset = if dst_y == 0 { prompt_len } else { 0 };
                    let dst_x = offset + llines.get(dst_y as usize)
                        .map(|prev| prev.count_graphemes().saturating_sub(1))
                        .unwrap_or(0);
                    let end_of_prev_line = Coords { x: dst_x, y: dst_y };
                    *cursor = end_of_prev_line;
                } else {
                    cursor.x = cursor.x.saturating_sub(1);
                }
            },
        }
        Ok(())
    }

    fn cmd_nav_cmd_right(&mut self) -> ReplBlockResult<()> {
        let editor_width = self.dimensions()?.width;
        let prompt_len = self.prompt_len();
        let origin = self.origin()?;
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {

                todo!("[cmd_nav_cmd_right]");

            },
            State::Navigate(NavigateState { hidx: _, backup: _, preview, cursor }) => {
                let llines = preview.logical_lines(editor_width, prompt_len);
                let max_x = llines.get(cursor.y as usize)
                    .map(|line| line.count_graphemes().saturating_sub(1))
                    .unwrap_or(0);
                let max_y = llines.len().saturating_sub(1) as u16;
                let is_1_liner = max_y == 0;
                if is_1_liner {
                    let is_end_of_prompt_line = cursor.x > prompt_len + max_x;
                    if is_end_of_prompt_line {
                        // NOP: At the end of 1-liner content
                    } else {
                        cursor.x = cursor.x + 1;
                    }
                } else {
                    let is_prompt_line = cursor.y == 0;
                    let is_last_line = cursor.y == max_y;
                    let is_end_of_prompt_line = cursor.x >= prompt_len + max_x;
                    let is_end_of_non_prompt_line = cursor.x >= max_x;
                    let start_of_next_line = Coords { y: cursor.y + 1, ..origin };
                    if is_last_line && cursor.x == max_x {
                        cursor.x = cursor.x + 1; // edge case: end of last line
                    } else if is_prompt_line && is_end_of_prompt_line {
                        *cursor = start_of_next_line;
                    } else if !is_prompt_line && is_end_of_non_prompt_line {
                        if cursor.y >= max_y {
                            // NOP: At the end of multiliner content
                        } else {
                            *cursor = start_of_next_line;
                        }
                    } else { // The cursor is in the middle of a line
                        cursor.x = cursor.x + 1;
                    }
                }
            },
        }
        Ok(())
    }

    /// Navigate to the start of the current Cmd
    fn cmd_nav_to_start_of_cmd(&mut self) -> ReplBlockResult<()> {
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { cursor, .. }) => {
                *cursor = Coords { x: prompt_len, y: 0 };
            },
            State::Navigate(NavigateState { cursor, .. }) => {
                *cursor = Coords { x: prompt_len, y: 0 };
            },
        }
        Ok(())
    }

    /// Navigate to the end of the current Cmd
    fn cmd_nav_to_end_of_cmd(&mut self) -> ReplBlockResult<()> {
        let editor_width = self.dimensions()?.width;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {

                todo!("[cmd_nav_to_end_of_cmd]"); // TODO

            },
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                let llines = preview.logical_lines(editor_width, prompt_len);
                let max_y = llines.len().saturating_sub(1);
                let offset = if max_y == 0 { prompt_len } else { 0 };
                let max_x = offset + llines.get(max_y)
                    .map(|last| last.count_graphemes())
                    .unwrap_or(0);
                *cursor = Coords { x: max_x, y: max_y as u16 };
            },
        }
        Ok(())
    }

    /// Add a char to the current line of the current cmd
    fn cmd_insert_char(&mut self, c: char) -> ReplBlockResult<()> {
        // let editor_dims = self.dimensions()?;
        // let cursor = self.cursor_position()?;
        // let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                // let old_buffer = buffer.clone();
                // let coords = Coords {
                //     // x: cursor.x - prompt_len,
                //     x: cursor.x,
                //     y: cursor.y,
                // };
                // terminal::disable_raw_mode().unwrap();
                // buffer.insert_char(coords, c, editor_dims.width, prompt_len);
                // terminal::enable_raw_mode().unwrap();
                // repaint_input_area!(
                //     in self,
                //     old: &old_buffer,
                //     new: &self.state.as_edit()?.buffer,
                // )?;
                // execute!(self.sink, cursor::MoveToColumn(cursor.x + 1))?;
            }
            State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {
                // self.state = State::Edit(EditState {
                //     buffer: std::mem::take(preview),
                // });
                // self.cmd_insert_char(c)?;
            }
        }

        todo!("[cmd_insert_char]"); // TODO
        Ok(())
    }

    /// Add a newline to the current cmd
    fn cmd_insert_newline(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {

                // buffer.push_empty_line();
                // execute!(self.sink, style::Print("\n"))?;
                // self.write_continue_prompt(FlushPolicy::Flush)?;

            }
            State::Navigate(NavigateState { hidx, backup, preview, cursor, }) => {

                // self.state = State::Edit(EditState {
                //     buffer: std::mem::take(preview),
                // });
                // self.cmd_insert_newline()?;

            }
        }
        todo!("[cmd_insert_newline]"); // TODO
        Ok(())
    }

    /// Delete the last char on the current line of the current cmd
    fn cmd_rm_char(&mut self, Coords { x, y }: Coords) -> ReplBlockResult<()> {
        // let cursor = self.cursor_position()?;
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {

                // if buffer.is_empty() {
                //     return Ok(()); // NOP
                // }
                // buffer.rm_char(Coords { x: cursor.x - 1, ..cursor });
                // let line = buffer[cursor.y as usize].to_string();
                // { // Repaint the entire line
                //     queue!(
                //         self.sink,
                //         cursor::MoveToColumn(0), // also clear the prompt
                //     )?;
                //     self.write_default_prompt(FlushPolicy::NoFlush)?;
                //     queue!(
                //         self.sink,
                //         style::Print(line),
                //         terminal::Clear(terminal::ClearType::UntilNewLine),
                //     )?;
                //     self.sink.flush()?;
                // }

            }
            State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {

            }
        }
        todo!("[cmd_rm_char"); // TODO
        Ok(())
    }

    /// Execute the current cmd
    fn cmd_eval(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {

                // execute!(self.sink, style::Print("\n"))?;
                // #[allow(unstable_name_collisions)]
                // let source_code = buffer.lines().iter()
                //     .filter(|line| !line.is_empty())
                //     .map(Line::as_str)
                //     .intersperse("\n")
                //     .collect::<String>();
                // if source_code.is_empty() {
                //     self.write_default_prompt(FlushPolicy::Flush)?;
                //     // Prepare for listening to input:
                //     terminal::enable_raw_mode()?;
                //     return Ok(());
                // }
                // let cmd = std::mem::take(buffer);
                // let _hidx = self.history.add_cmd(cmd);
                // // TODO: use hidx
                // self.history.write_to_file(&self.history_filepath)?;
                // terminal::disable_raw_mode()?;
                // (*self.evaluator)(source_code.as_str())?;
                // self.height = 1; // reset
                // self.write_default_prompt(FlushPolicy::Flush)?;
                // terminal::enable_raw_mode()?;

            }
            State::Navigate(NavigateState { hidx, backup, preview, cursor }) => {

                // self.state = State::Edit(EditState {
                //     buffer: std::mem::take(preview),
                // });
                // self.cmd_eval()?;

            }
        }
        todo!("[cmd_eval]"); // TODO
        Ok(())
    }

    // fn cmd_(&mut self) -> ReplResult<()> {
    //     // TODO
    //     Ok(())
    // }

}

#[derive(Clone, Copy,  Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dims { pub width: u16, pub height: u16 }

#[derive(Clone, Copy,  Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coords { pub x: u16, pub y: u16 }

impl Coords {
    pub(crate) const ORIGIN: Self = Self { x: 0, y: 0 };

    pub fn is_origin(&self) -> bool {
        *self == Self::ORIGIN
    }
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

    // fn as_edit_mut(&mut self) -> ReplResult<&mut EditState> {
    //     match self {
    //         Self::Edit(es) => Ok(es),
    //         Self::Navigate(_) => panic!("Expected State::Edit(_); Got {self:?}"),
    //     }
    // }

    fn as_navigate(&self) -> ReplBlockResult<&NavigateState> {
        match &self {
            Self::Edit(_) => panic!("Expected State::Nsvigate(_); Got {self:?}"),
            Self::Navigate(ns) => Ok(ns),
        }
    }

    // fn as_navigate_mut(&mut self) -> ReplResult<&mut NavigateState> {
    //     match self {
    //         Self::Edit(_) => panic!("Expected State::Nsvigate(_); Got {self:?}"),
    //         Self::Navigate(ns) => Ok(ns),
    //     }
    // }
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
