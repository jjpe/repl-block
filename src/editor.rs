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
        sink: W,
        history_filepath: impl AsRef<Utf8Path>,
        evaluator: Box<Evaluator<'eval>>,
        default_prompt: Vec<StyledContent<char>>,
        continue_prompt: Vec<StyledContent<char>>,
    ) -> ReplBlockResult<Editor<'eval, W>> {
        let mut editor = Self {
            sink,
            state: State::Edit(EditState { buffer: Cmd::default() }),
            height: 1,
            history: History::read_from_file(history_filepath.as_ref())?,
            history_filepath: history_filepath.as_ref().to_path_buf(),
            evaluator,
            default_prompt,
            continue_prompt,
        };
        editor.sink.flush()?;
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

// This is a macro rather than a method of Editor due to the borrowck
// issues that would ensue when trying to use that method while also
// matching on Editor state.
macro_rules! repaint_input_area {
    // NOTE: The `$edtitor` macro var is expanded more than
    //       once to keep borrows as short as possible.
    (in $editor:expr, old: $old:expr, new: $new:expr $(,)?) => {{
        /* OUT WITH THE OLD ... */
        let (old, new): (&Cmd, &Cmd) = ({ $old }, { $new });
        let origin = $editor.origin()?;
        let editor_width = $editor.dimensions()?.width;
        let prompt_len = $editor.prompt_len();
        let num_lines_old = old.count_logical_lines(editor_width, prompt_len);
        let num_lines_new = new.count_logical_lines(editor_width, prompt_len);
        let lines_new = new.logical_lines(editor_width, prompt_len);
        // The editor height can grow but never shrinks until a Cmd is evaluated
        $editor.height = std::cmp::max($editor.height, num_lines_new);
        for offset in 0..num_lines_old { // Clear all the old lines
            queue!(
                $editor.sink,
                cursor::MoveToColumn(origin.x),
                cursor::MoveToRow(origin.y),
                cursor::MoveDown(offset),
                terminal::Clear(ClearType::CurrentLine),
            )?;
        }
        queue!( // Go to the editor's origin
            $editor.sink,
            cursor::MoveToColumn(origin.x),
            cursor::MoveToRow(origin.y),
        )?;
        $editor.write_default_prompt(FlushPolicy::NoFlush)?;
        queue!( // If the new Cmd is empty, clear the rest of the (single) line
            $editor.sink,
            cursor::SavePosition,
            terminal::Clear(ClearType::UntilNewLine),
        )?;
        for line in &lines_new { // Write the new lines
            queue!(
                $editor.sink,
                style::Print(line),
                cursor::SavePosition,
                terminal::Clear(ClearType::UntilNewLine),
            )?;
        }
        queue!($editor.sink, cursor::RestorePosition)?;
        $editor.sink.flush()?;
        ReplBlockResult::Ok(())
    }};
}

impl<'eval, W: Write> Editor<'eval, W> {
    pub fn run_event_loop(&mut self) -> ReplBlockResult<()> {
        loop { match event::read()? {
            Event::Key(key!(CONTROL-'c')) => self.cmd_nop()?,

            // Control application lifecycle:
            Event::Key(key!(CONTROL-'d'))    => self.cmd_exit_repl()?,
            Event::Key(key!(@special Enter)) => self.cmd_eval()?,

            // Navigation:
            Event::Key(key!(CONTROL-'p'))    => self.cmd_navigate_up()?,
            Event::Key(key!(@special Up))    => self.cmd_navigate_up()?,
            Event::Key(key!(CONTROL-'n'))    => self.cmd_navigate_down()?,
            Event::Key(key!(@special Down))  => self.cmd_navigate_down()?,
            Event::Key(key!(CONTROL-'b'))    => self.cmd_navigate_left()?,
            Event::Key(key!(@special Left))  => self.cmd_navigate_left()?,
            Event::Key(key!(CONTROL-'f'))    => self.cmd_navigate_right()?,
            Event::Key(key!(@special Right)) => self.cmd_navigate_right()?,
            Event::Key(key!(CONTROL-'a'))    => self.cmd_navigate_line_start()?,
            Event::Key(key!(@special Home))  => self.cmd_navigate_line_start()?,
            Event::Key(key!(CONTROL-'e'))    => self.cmd_navigate_line_end()?,
            Event::Key(key!(@special End))   => self.cmd_navigate_line_end()?,

            // TODO remove both key bindings
            // Mainly useful for debugging:
            Event::Key(key!(@special ALT-Up)) => {
                execute!(self.sink, terminal::ScrollUp(1))?;
            },
            Event::Key(key!(@special ALT-Down)) => {
                execute!(self.sink, terminal::ScrollDown(1))?;
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
        }}
    }

    pub fn write_default_prompt(
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

    pub fn write_continue_prompt(
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


    /// Return the global (col, row)-coordinates of the top-left corner of `self`.
    fn origin(&self) -> ReplBlockResult<Coords> {
        let (_term_width, term_height) = terminal::size()?;
        Ok(Coords { x: 0, y: term_height - self.height })
    }

    // /// Return the global (col, row)-coordinates of the top-left corner of `self`.
    // /// The top left cell is represented `(1, 1)`.
    // fn content_origin(&self) -> ReplResult<Coords> {
    //     let (_term_width, term_height) = terminal::size()?;
    //     Ok(Coords { x: *self.prompt_len(), y: term_height - self.height })
    // }

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

    fn cmd_navigate_left(&mut self) -> ReplBlockResult<()> {
        fn move_left<W: Write>(
            sink: &mut W,
            cmd: &Cmd,
            _origin: Coords,
            cursor: Coords,
            editor_dims: Dims,
            prompt_len: u16
        ) -> ReplBlockResult<()> {
            if cmd.is_empty() {
                return Ok(()); // NOP, row does not exist
            }
            let llines = cmd.logical_lines(editor_dims.width, prompt_len);
            let cur_lline: &Line = &llines[cursor.y as usize];
            if cur_lline.is_empty() {
                return Ok(()); // NOP, col does not exist
            }
            let (min_x, min_y) = (0, 0);
            if cursor.x == min_x && cursor.y == min_y {
                // NOP: at origin
            } else if cursor.x == prompt_len && cursor.y == min_y {
                // NOP: at leftmost point of content area
            } else if cursor.x == min_x && cursor.y != min_y {
                let prev_lline = &llines[cursor.y as usize - 1];
                let last = prompt_len + prev_lline.count_graphemes();
                queue!(sink, cursor::MoveUp(1))?;
                queue!(sink, cursor::MoveToColumn(last - 1))?;
                sink.flush()?;
            } else {
                execute!(sink, cursor::MoveLeft(1))?;
            }
            Ok(())
        }
        let editor_dims = self.dimensions()?;
        let cursor = self.cursor_position()?;
        let origin = self.origin()?;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer }) =>
                move_left(
                    &mut self.sink,
                    buffer,
                    origin,
                    cursor,
                    editor_dims,
                    prompt_len
                )?,
            State::Navigate(NavigateState { nav: _, backup: _, preview }) =>
                move_left(
                    &mut self.sink,
                    preview,
                    origin,
                    cursor,
                    editor_dims,
                    prompt_len
                )?,
        }
        Ok(())
    }

    fn cmd_navigate_right(&mut self) -> ReplBlockResult<()> {
        fn move_right<W: Write>(
            sink: &mut W,
            cmd: &Cmd,
            origin: Coords,
            cursor: Coords,
            editor_dims: Dims,
            prompt_len: u16
        ) -> ReplBlockResult<()> {
            if cmd.is_empty() {
                return Ok(()); // NOP, row does not exist
            }
            let llines = cmd.logical_lines(editor_dims.width, prompt_len);
            let cur_lline: &Line = &llines[cursor.y as usize];
            if cur_lline.is_empty() {
                return Ok(()); // NOP, col does not exist
            }
            let max_x = editor_dims.width - 1;
            let max_y = origin.y + editor_dims.height;
            if cursor.x == max_x && cursor.y == max_y {
                // NOP: cursor is at bottom-right point
            }
            // else if
            //     cursor.y == 0 &&
            //     cursor.x >= prompt_len + cur_lline.count_graphemes()
            //     // cursor.x == editor_dims.width
            // {
            //     // NOP: cursor is at the end of the text on the top line
            // }
            else if
                cursor.y > 0 &&
                cursor.x >= cur_lline.count_graphemes()
            {
                // NOP: cursor is at the end of the cmd on a non-top line
            }
            else if cursor.x == max_x && cursor.y != max_y {
                if cursor.y as usize + 1 < llines.len() { // there is a next line
                    queue!(sink, cursor::MoveDown(1))?;
                    queue!(sink, cursor::MoveToColumn(0))?;
                    sink.flush()?;
                } else {
                    // NOP: no next line to navigate to
                }
            } else if
                cursor.x < editor_dims.width &&
                cursor.x < prompt_len + cur_lline.count_graphemes()
            {
                execute!(sink, cursor::MoveRight(1))?;
            } else {
                // NOP
            }
            Ok(())
        }
        let origin = self.origin()?;
        let editor_dims = self.dimensions()?;
        let cursor = self.cursor_position()?;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer }) =>
                move_right(
                    &mut self.sink,
                    buffer,
                    origin,
                    cursor,
                    editor_dims,
                    prompt_len,
                )?,
            State::Navigate(NavigateState { nav: _, backup: _, preview }) =>
                move_right(
                    &mut self.sink,
                    preview,
                    origin,
                    cursor,
                    editor_dims,
                    prompt_len,
                )?,
        }
        Ok(())
    }

    fn cmd_navigate_up(&mut self) -> ReplBlockResult<()> {
        // let editor_width = self.dimensions()?.width;
        // let cursor = self.cursor_position()?;
        match &mut self.state {
            State::Edit(EditState { buffer }) => {
                let Some(max_hidx) = self.history.max_idx()
                else { return Ok(()); }; // NOP: no history to navigate
                self.state = State::Navigate(NavigateState {
                    nav: Navigator::new(max_hidx),
                    backup: std::mem::take(buffer),
                    preview: self.history[max_hidx].clone(),
                });
                repaint_input_area!(
                    in self,
                    old: &self.state.as_navigate()?.backup, // i.e. old buffer
                    new: &self.state.as_navigate()?.preview,
                )?;
            }
            State::Navigate(NavigateState { nav, backup: _, preview }) => {
                let min_hidx = HistIdx(0);
                if nav.hidx == min_hidx { // top-of-history
                    // NOP
                } else {
                    nav.hidx -= 1;
                    let old_preview = std::mem::take(preview);
                    *preview = self.history[nav.hidx].clone(); // update
                    repaint_input_area!(
                        in self,
                        old: &old_preview,
                        new: &self.state.as_navigate()?.preview,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn cmd_navigate_down(&mut self) -> ReplBlockResult<()> {
        // let editor_width = self.dimensions()?.width;
        // let cursor = self.cursor_position()?;
        match &mut self.state {
            State::Edit(EditState { .. }) => {/* NOP */}
            State::Navigate(NavigateState { nav, backup, preview }) => {
                let max_hidx = self.history.max_idx();
                if Some(nav.hidx) == max_hidx { // bottom-of-history
                    let nav_hidx = nav.hidx;
                    self.state = State::Edit(EditState {
                        buffer: std::mem::take(backup),
                    });
                    repaint_input_area!(
                        in self,
                        old: &self.history[nav_hidx],
                        new: &self.state.as_edit()?.buffer,
                    )?;
                } else {
                    nav.hidx += 1;
                    let old_preview = std::mem::take(preview);
                    *preview = self.history[nav.hidx].clone(); // update
                    repaint_input_area!(
                        in self,
                        old: &old_preview,
                        new: &self.state.as_navigate()?.preview,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Navigate to the start of the line containing the cursor
    fn cmd_navigate_line_start(&mut self) -> ReplBlockResult<()> {
        let origin = self.origin()?;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { .. }) =>
                execute!(
                    self.sink,
                    cursor::MoveToRow(origin.y),
                    cursor::MoveToColumn(origin.x),
                    cursor::MoveRight(prompt_len),
                )?,
            State::Navigate(NavigateState { .. }) =>
                execute!(
                    self.sink,
                    cursor::MoveToRow(origin.y),
                    cursor::MoveToColumn(origin.x),
                    cursor::MoveRight(prompt_len),
                )?,
        }
        Ok(())
    }

    /// Navigate to the end of the line containing the cursor
    fn cmd_navigate_line_end(&mut self) -> ReplBlockResult<()> {
        fn mv_cursor(
            sink: &mut impl Write,
            origin: Coords,
            cmd: &Cmd,
            editor_dims: Dims,
            prompt_len: u16,
        ) -> ReplBlockResult<()> {
            let llines = cmd.logical_lines(editor_dims.width, prompt_len);
            if llines.is_empty() {
                return Ok(());
            }
            queue!(sink, cursor::MoveTo(origin.x + prompt_len, origin.y))?;
            if llines.len() >= 2 {
                let n_down = llines.len().saturating_sub(1);
                if n_down > 0 {
                    queue!(sink, cursor::MoveDown(n_down as u16))?;
                }
                queue!(sink, cursor::MoveToColumn(origin.x))?;
            }
            let n_right = llines.last()
                .map(|last: &Line| last.count_graphemes())
                .unwrap_or(0);
            queue!(sink, cursor::MoveRight(n_right))?;
            sink.flush()?;
            Ok(())
        }
        let origin = self.origin()?;
        let editor_dims = self.dimensions()?;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer }) =>
                mv_cursor(
                    &mut self.sink,
                    origin,
                    buffer,
                    editor_dims,
                    prompt_len,
                )?,
            State::Navigate(NavigateState { nav: _, backup: _, preview }) =>
                mv_cursor(
                    &mut self.sink,
                    origin,
                    preview,
                    editor_dims,
                    prompt_len,
                )?,
        }
        Ok(())
    }

    /// Add a char to the current line of the current cmd
    fn cmd_insert_char(&mut self, c: char) -> ReplBlockResult<()> {
        let editor_dims = self.dimensions()?;
        let cursor = self.cursor_position()?;
        let prompt_len = self.prompt_len();
        match &mut self.state {
            State::Edit(EditState { buffer }) => {
                let old_buffer = buffer.clone();
                let coords = Coords {
                    // x: cursor.x - prompt_len,
                    x: cursor.x,
                    y: cursor.y,
                };
                terminal::disable_raw_mode().unwrap();
                buffer.insert_char(coords, c, editor_dims.width, prompt_len);
                terminal::enable_raw_mode().unwrap();
                repaint_input_area!(
                    in self,
                    old: &old_buffer,
                    new: &self.state.as_edit()?.buffer,
                )?;
                execute!(self.sink, cursor::MoveToColumn(cursor.x + 1))?;
            }
            State::Navigate(NavigateState { nav: _, backup: _, preview }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                });
                self.cmd_insert_char(c)?;
            }
        }
        Ok(())
    }

    /// Add a newline to the current cmd
    fn cmd_insert_newline(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer }) => {
                buffer.push_empty_line();
                execute!(self.sink, style::Print("\n"))?;
                self.write_continue_prompt(FlushPolicy::Flush)?;
            }
            State::Navigate(NavigateState { nav: _, backup: _, preview }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                });
                self.cmd_insert_newline()?;
            }
        }
        Ok(())
    }

    /// Delete the last char on the current line of the current cmd
    fn cmd_rm_char(&mut self, Coords { x, y }: Coords) -> ReplBlockResult<()> {
        let cursor = self.cursor_position()?;
        match &mut self.state {
            State::Edit(EditState { buffer }) => {
                if buffer.is_empty() {
                    return Ok(()); // NOP
                }
                buffer.rm_char(Coords { x: cursor.x - 1, ..cursor });
                let line = buffer[cursor.y as usize].to_string();
                { // Repaint the entire line
                    queue!(
                        self.sink,
                        cursor::MoveToColumn(0), // also clear the prompt
                    )?;
                    self.write_default_prompt(FlushPolicy::NoFlush)?;
                    queue!(
                        self.sink,
                        style::Print(line),
                        terminal::Clear(terminal::ClearType::UntilNewLine),
                    )?;
                    self.sink.flush()?;
                }
            }
            State::Navigate(NavigateState { nav: _, backup: _, preview: _ }) => {
                terminal::disable_raw_mode()?;
                todo!("[cmd_rm_char") // TODO
            }
        }
        Ok(())
    }

    /// Execute the current cmd
    fn cmd_eval(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer }) => {
                execute!(self.sink, style::Print("\n"))?;
                #[allow(unstable_name_collisions)]
                let source_code = buffer.lines().iter()
                    .filter(|line| !line.is_empty())
                    .map(Line::as_str)
                    .intersperse("\n")
                    .collect::<String>();
                if source_code.is_empty() {
                    self.write_default_prompt(FlushPolicy::Flush)?;
                    // Prepare for listening to input:
                    terminal::enable_raw_mode()?;
                    return Ok(());
                }
                let cmd = std::mem::take(buffer);
                let _hidx = self.history.add_cmd(cmd);
                // TODO: use hidx
                self.history.write_to_file(&self.history_filepath)?;
                terminal::disable_raw_mode()?;
                (*self.evaluator)(source_code.as_str())?;
                self.height = 1; // reset
                self.write_default_prompt(FlushPolicy::Flush)?;
                terminal::enable_raw_mode()?;
            }
            State::Navigate(NavigateState { nav: _, backup: _, preview }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                });
                self.cmd_eval()?;
            }
        }
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
    const ORIGIN: Self = Self { x: 0, y: 0 };

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
}

/// Navigating through the `History`
#[derive(Debug)]
struct NavigateState {
    /// Keeps track of history navigation coordinates
    nav: Navigator,
    /// A buffer containing the cmd that was last edited
    backup: Cmd,
    /// The `History` entry being previewed
    preview: Cmd,
}

#[derive(Debug)]
struct Navigator {
    /// Points to a Cmd in the History
    hidx: HistIdx,
    /// Points to a line within the pointee Cmd of `self.hidx`
    line: u16,
}

impl Navigator {
    pub fn new(hidx: HistIdx) -> Self {
        Self {
            hidx,
            line: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlushPolicy {
    Flush,
    NoFlush,
}
