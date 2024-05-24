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
use std::io::{Stdout, Write};
use unicode_segmentation::UnicodeSegmentation;


type Evaluator<'eval> =
    dyn for<'src> FnMut(&'src str) -> ReplBlockResult<()> + 'eval;

pub struct ReplBuilder<'eval, W: Write> {
    sink: W,
    default_prompt: Vec<StyledContent<char>>,
    continue_prompt: Vec<StyledContent<char>>,
    reverse_search_prompt: Vec<StyledContent<char>>,
    history_filepath: Utf8PathBuf,
    evaluator: Box<Evaluator<'eval>>,
    hello_msg: String,
    goodbye_msg: String,
}

impl<'eval> Default for ReplBuilder<'eval, Stdout> {
    fn default() -> ReplBuilder<'eval, Stdout> {
        #[inline(always)]
        fn nop<'eval>() -> Box<Evaluator<'eval>> {
            Box::new(|_| Ok(()))
        }
        ReplBuilder {
            sink: std::io::stdout(),
            default_prompt:  vec!['■'.yellow(), '>'.green().bold(), ' '.reset()],
            continue_prompt: vec!['.'.yellow(), '.'.yellow(),       ' '.reset()],
            reverse_search_prompt: vec![
                'r'.yellow().italic(),
                'e'.yellow().italic(),
                'v'.yellow().italic(),
                'e'.yellow().italic(),
                'r'.yellow().italic(),
                's'.yellow().italic(),
                'e'.yellow().italic(),
                ' '.reset(),
                's'.yellow().italic(),
                'e'.yellow().italic(),
                'a'.yellow().italic(),
                'r'.yellow().italic(),
                'c'.yellow().italic(),
                'h'.yellow().italic(),
                ':'.blue().italic(),
                ' '.reset(),
            ],
            history_filepath: Utf8PathBuf::from(".repl.history"),
            evaluator: nop(),
            hello_msg: format!("🖐 Press {} to exit.",  "Ctrl-D".magenta()),
            goodbye_msg: "👋".to_string(),
        }
    }
}

impl<'eval, W: Write> ReplBuilder<'eval, W> {
    pub fn sink<S: Write>(self, sink: S) -> ReplBuilder<'eval, S> {
        ReplBuilder {
            sink,
            default_prompt: self.default_prompt,
            continue_prompt: self.continue_prompt,
            reverse_search_prompt: self.reverse_search_prompt,
            history_filepath: self.history_filepath,
            evaluator: self.evaluator,
            hello_msg: self.hello_msg,
            goodbye_msg: self.goodbye_msg,
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

    pub fn reverse_search_prompt(mut self, prompt: Vec<StyledContent<char>>) -> Self {
        self.reverse_search_prompt = prompt;
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

    pub fn hello(mut self, hello_msg: impl Into<String>) -> Self {
        self.hello_msg = hello_msg.into();
        self
    }

    pub fn goodbye(mut self, goodbye_msg: impl Into<String>) -> Self {
        self.goodbye_msg = goodbye_msg.into();
        self
    }

    pub fn build(self) -> ReplBlockResult<Repl<'eval, W>> {
        assert_eq!(
            self.default_prompt.len(), self.continue_prompt.len(),
            "default_prompt.len() != continue_prompt.len()"
        );
        let mut repl = Repl::new(
            self.sink,
            self.history_filepath,
            self.evaluator,
            self.default_prompt,
            self.continue_prompt,
            self.reverse_search_prompt,
            self.hello_msg,
            self.goodbye_msg,
        )?;
        repl.render_default_prompt()?;
        repl.sink.flush()?;
        Ok(repl)
    }
}



pub struct Repl<'eval, W: Write> {
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
    /// The prompt used for reverse history search
    reverse_search_prompt: Vec<StyledContent<char>>,
    hello_msg: String,
    goodbye_msg: String,
}

impl<'eval, W: Write> Repl<'eval, W> {
    fn new(
        mut sink: W,
        history_filepath: impl AsRef<Utf8Path>,
        evaluator: Box<Evaluator<'eval>>,
        default_prompt: Vec<StyledContent<char>>,
        continue_prompt: Vec<StyledContent<char>>,
        reverse_search_prompt: Vec<StyledContent<char>>,
        hello_msg: String,
        goodbye_msg: String,
    ) -> ReplBlockResult<Repl<'eval, W>> {
        sink.flush()?;
        let mut repl = Self {
            sink,
            state: State::Edit(EditState {
                buffer: Cmd::default(),
                cursor: ORIGIN,
            }),
            height: 1,
            history: History::read_from_file(history_filepath.as_ref())?,
            history_filepath: history_filepath.as_ref().to_path_buf(),
            evaluator,
            default_prompt,
            continue_prompt,
            reverse_search_prompt,
            hello_msg,
            goodbye_msg,
        };
        execute!(
            repl.sink,
            cursor::SetCursorStyle::BlinkingBar,
            cursor::MoveToColumn(0),
            style::Print(&repl.hello_msg),
            style::Print("\n"),
        )?;
        Ok(repl)
    }
}

impl<'eval, W: Write> Repl<'eval, W> {
    pub fn start(&mut self) -> ReplBlockResult<()> {
        loop {
            let old_height = self.height;
            self.dispatch_key_event()?; // This might alter `self.height`
            self.render_ui(old_height)?;
        }
    }

    fn dispatch_key_event(&mut self) -> ReplBlockResult<()> {
        terminal::enable_raw_mode()?;
        let event = event::read()?;
        terminal::disable_raw_mode()?;
        match event {
            Event::Key(key!(CONTROL-'c')) => self.cmd_nop()?,

            // Control application lifecycle:
            Event::Key(key!(CONTROL-'d')) => self.cmd_exit_repl()?,
            Event::Key(key!(CONTROL-'g')) => self.cmd_cancel_nav()?,
            Event::Key(key!(@name Enter)) => self.cmd_eval()?,

            // Navigation:
            Event::Key(key!(CONTROL-'p')) => self.cmd_nav_up()?,
            Event::Key(key!(@name Up))    => self.cmd_nav_up()?,
            Event::Key(key!(CONTROL-'n')) => self.cmd_nav_down()?,
            Event::Key(key!(@name Down))  => self.cmd_nav_down()?,
            Event::Key(key!(CONTROL-'b')) => self.cmd_nav_cmd_left()?,
            Event::Key(key!(@name Left))  => self.cmd_nav_cmd_left()?,
            Event::Key(key!(CONTROL-'f')) => self.cmd_nav_cmd_right()?,
            Event::Key(key!(@name Right)) => self.cmd_nav_cmd_right()?,
            Event::Key(key!(CONTROL-'a')) => self.cmd_nav_to_start_of_cmd()?,
            Event::Key(key!(@name Home))  => self.cmd_nav_to_start_of_cmd()?,
            Event::Key(key!(CONTROL-'e')) => self.cmd_nav_to_end_of_cmd()?,
            Event::Key(key!(@name End))   => self.cmd_nav_to_end_of_cmd()?,
            Event::Key(key!(CONTROL-'r')) => self.cmd_reverse_search_history()?,

            // Editing;
            Event::Key(key!(@c))                => self.cmd_insert_char(c)?,
            Event::Key(key!(SHIFT-@c))          => self.cmd_insert_char(c)?,
            // FIXME `SHIFT+Enter` doesn't work for...reasons(??),
            //       yet `CONTROL-o` works as expected:
            Event::Key(key!(@name SHIFT-Enter)) => self.cmd_insert_newline()?,
            Event::Key(key!(CONTROL-'o'))       => self.cmd_insert_newline()?,
            Event::Key(key!(@name Backspace))   => self.cmd_rm_grapheme_before_cursor()?,
            Event::Key(key!(@name Delete))      => self.cmd_rm_grapheme_at_cursor()?,

            _event => {/* ignore the event */},
        }
        Ok(())
    }

    fn render_ui(&mut self, old_input_area_height: u16) -> ReplBlockResult<()> {
        let dims = self.input_area_dims()?;
        let prompt_len = self.prompt_len();

        let calculate_uncursor = |cmd: &Cmd, uncompressed: &Cmd, cursor: Coords| {
            let prev_unlines: Vec<Vec<Line>> = (0..cursor.y)
                .map(|y| cmd[y].uncompress(dims.width, prompt_len))
                .collect();
            let mut uncursor = Coords {
                x: cursor.x,
                y: prev_unlines.iter()
                    .map(|unline| unline.len())
                    .sum::<usize>() as u16,
            };
            let line = &cmd[cursor.y];
            let unlines_for_line = line.uncompress(dims.width, prompt_len);
            for unline in unlines_for_line.iter() {
                let unline_len = unline.count_graphemes();
                let width = std::cmp::min(dims.width, unline_len);
                if uncursor.x > width {
                    uncursor.x -= width;
                    uncursor.y += 1;
                } else {
                    break;
                }
            }
            if uncompressed[uncursor.y].is_start() {
                uncursor.x += prompt_len;
            }
            uncursor
        };

        macro_rules! render {
            ($cmd:expr, $cursor:expr) => {{
                let (cmd, cursor): (&Cmd, Coords) = ($cmd, $cursor);
                let uncompressed = cmd.uncompress(dims.width, prompt_len);

                // Adjust the height of the input area
                let num_unlines = uncompressed.count_lines() as u16;
                let content_height = num_unlines;
                self.height = std::cmp::max(self.height, content_height);

                // Obtain an `uncompressed` version of `cursor`
                let uncursor = calculate_uncursor(cmd, &uncompressed, cursor);

                // Scroll up the old output *BEFORE* clearing the input area
                for _ in old_input_area_height..content_height {
                    queue!(self.sink, terminal::ScrollUp(1))?;
                }

                // execute!(
                //     self.sink,
                //     cursor::MoveUp(terminal::size().unwrap().1),
                //     cursor::MoveToColumn(0),
                //     terminal::Clear(ClearType::All),
                //     style::Print(format!("CMD: {cmd:#?}\n")),
                //     style::Print(format!("UNCOMPRESSED: {uncompressed:#?}\n")),
                //     style::Print(format!("CURSOR: {cursor}\n")),
                //     style::Print(format!("UNCURSOR: {uncursor}\n")),
                //     style::Print(format!("TERM DIMS: {:?}\n", terminal::size()?)),
                //     style::Print(format!("INPUT AREA DIMS: {dims:?}\n")),
                //     cursor::MoveDown(terminal::size().unwrap().1),
                // )?;

                self.clear_input_area()?;
                self.move_cursor_to_origin()?;
                self.render_cmd(&uncompressed)?;

                // Render the uncursor
                let o = self.origin()?;
                queue!(self.sink, cursor::MoveToColumn(o.x + uncursor.x))?;
                queue!(self.sink, cursor::MoveToRow(o.y + uncursor.y))?;

                ReplBlockResult::Ok(())
            }};
        }

        match &self.state {
            State::Edit(EditState { buffer, cursor }) => {
                render!(buffer, *cursor)?;
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                render!(preview, *cursor)?;
            }
            State::Search(SearchState { regex, preview, cursor, .. }) => {
                let (cmd, cursor): (&Cmd, Coords) = (preview, *cursor);
                let uncompressed = cmd.uncompress(dims.width, prompt_len);
                let regex = regex.clone();

                // Adjust the height of the input area
                let num_unlines = uncompressed.count_lines() as u16;
                const SEARCH_PROMPT_LINE: u16 = 1;
                let content_height = num_unlines + SEARCH_PROMPT_LINE;
                self.height = std::cmp::max(self.height, content_height);

                // Scroll up the old output *BEFORE* clearing the input area
                for _ in old_input_area_height..content_height {
                    queue!(self.sink, terminal::ScrollUp(1))?;
                }

                self.clear_input_area()?;
                self.move_cursor_to_origin()?;
                self.render_cmd(&uncompressed)?;
                self.render_reverse_search_prompt()?;

                // Render the reverse search topic
                queue!(self.sink, style::Print(regex))?;

                let o = self.origin()?;
                // Render the search prompt cursor
                queue!(self.sink, cursor::MoveToRow(o.y + cursor.y + self.height))?;
                queue!(self.sink, cursor::MoveToColumn(o.x + cursor.x))?;
            }
        }

        self.sink.flush()?;
        Ok(())
    }

    fn render_cmd(&mut self, uncompressed: &Cmd, ) -> ReplBlockResult<()> {
        for (ulidx, unline) in uncompressed.lines().iter().enumerate() {
            if ulidx == 0 {
                self.render_default_prompt()?;
                queue!(self.sink, style::Print(unline))?;
                queue!(self.sink, cursor::MoveDown(1))?;
                queue!(self.sink, cursor::MoveToColumn(0))?;
            } else if unline.is_start() {
                self.render_continue_prompt()?;
                queue!(self.sink, style::Print(unline))?;
                queue!(self.sink, cursor::MoveDown(1))?;
                // queue!(self.sink, cursor::MoveToColumn(0))?;
            } else {
                queue!(self.sink, style::Print(unline))?;
                queue!(self.sink, cursor::MoveDown(1))?;
                queue!(self.sink, cursor::MoveToColumn(0))?;
            }
        }
        Ok(())
    }

    fn render_default_prompt(
        &mut self,
    ) -> ReplBlockResult<&mut Self> {
        queue!(self.sink, cursor::MoveToColumn(0))?;
        for &c in &self.default_prompt {
            queue!(self.sink, style::Print(c))?;
        }
        Ok(self)
    }

    fn render_continue_prompt(
        &mut self,
    ) -> ReplBlockResult<()> {
        queue!(self.sink, cursor::MoveToColumn(0))?;
        for &c in &self.continue_prompt {
            queue!(self.sink, style::Print(c))?;
        }
        Ok(())
    }

    fn render_reverse_search_prompt(
        &mut self,
    ) -> ReplBlockResult<()> {
        let origin = self.origin()?;
        // Position the cursor to write the reverse search prompt
        queue!(self.sink, cursor::MoveTo(origin.x, origin.y + self.height))?;
        // Render the reverse search prompt
        for c in &self.reverse_search_prompt {
            queue!(self.sink, style::Print(c))?;
        }
        Ok(())
    }

    fn move_cursor_to_origin(
        &mut self,
    ) -> ReplBlockResult<()> {
        let origin = self.origin()?;
        queue!(self.sink, cursor::MoveTo(origin.x, origin.y))?;
        Ok(())
    }

    fn clear_input_area(
        &mut self,
    ) -> ReplBlockResult<()> {
        self.move_cursor_to_origin()?;
        for _ in 0..self.height {
            queue!(self.sink, terminal::Clear(ClearType::CurrentLine))?;
            queue!(self.sink, cursor::MoveDown(1))?;
        }
        self.move_cursor_to_origin()?;
        Ok(())
    }


    /// Return the global (col, row)-coordinates of the top-left corner of `self`.
    fn origin(&self) -> ReplBlockResult<Coords> {
        let (_term_width, term_height) = terminal::size()?;
        Ok(Coords { x: 0, y: term_height - self.height })
    }

    /// Return the (width, height) dimensions of `self`.
    /// The top left cell is represented `(1, 1)`.
    fn input_area_dims(&self) -> ReplBlockResult<Dims> {
        let (term_width, _term_height) = terminal::size()?;
        Ok(Dims { width: term_width, height: self.height })
    }

    fn prompt_len(&self) -> u16 {
        assert_eq!(
            self.default_prompt.len(), self.continue_prompt.len(),
            "default_prompt.len() != continue_prompt.len()"
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
            style::Print(&self.goodbye_msg),
            terminal::Clear(ClearType::FromCursorDown),
        )?;
        self.sink.flush()?;
        std::process::exit(0);
    }

    fn cmd_cancel_nav(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { .. }) => {
                // NOP
            }
            State::Navigate(NavigateState { backup, .. }) => {
                self.state = State::Edit(EditState {
                    cursor: backup.end_of_cmd(),
                    buffer: std::mem::take(backup),
                });
            }
            State::Search(SearchState { backup, .. }) => {
                self.state = State::Edit(EditState {
                    cursor: backup.end_of_cmd(),
                    buffer: std::mem::take(backup),
                });
            }
        }
        Ok(())
    }

    fn cmd_nav_up(&mut self) -> ReplBlockResult<()> {
        let is_at_top_line = |cursor: Coords| cursor.y == ORIGIN.x;
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                if is_at_top_line(*cursor) {
                    self.cmd_nav_history_up()?;
                } else {
                    cursor.y -= 1;
                    let line_len = buffer[cursor.y].count_graphemes();
                    cursor.x = std::cmp::min(cursor.x, line_len);
                }
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                if is_at_top_line(*cursor) {
                    self.cmd_nav_history_up()?;
                } else {
                    cursor.y -= 1;
                    let line_len = preview[cursor.y].count_graphemes();
                    cursor.x = std::cmp::min(cursor.x, line_len);
                }
            }
            State::Search(SearchState { preview, cursor, .. }) => {
                if is_at_top_line(*cursor) {
                    self.cmd_nav_history_up()?;
                } else {
                    cursor.y -= 1;
                    let line_len = preview[cursor.y].count_graphemes();
                    cursor.x = std::cmp::min(cursor.x, line_len);
                }
            }
        }
        Ok(())
    }

    fn cmd_nav_down(&mut self) -> ReplBlockResult<()> {
        let is_at_bottom_line = |cursor: Coords, cmd: &Cmd| cursor.y == cmd.count_lines() - 1;
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                if is_at_bottom_line(*cursor, buffer) {
                    self.cmd_nav_history_down()?;
                } else {
                    cursor.y += 1;
                    let line_len = buffer[cursor.y].count_graphemes();
                    cursor.x = std::cmp::min(cursor.x, line_len);
                }
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                if is_at_bottom_line(*cursor, preview) {
                    self.cmd_nav_history_down()?;
                } else {
                    cursor.y += 1;
                    let line_len = preview[cursor.y].count_graphemes();
                    cursor.x = std::cmp::min(cursor.x, line_len);
                }
            }
            State::Search(SearchState { preview, cursor, .. }) => {
                if is_at_bottom_line(*cursor, preview) {
                    self.cmd_nav_history_down()?;
                } else {
                    cursor.y += 1;
                    let line_len = preview[cursor.y].count_graphemes();
                    cursor.x = std::cmp::min(cursor.x, line_len);
                }
            }
        }
        Ok(())
    }

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
            State::Navigate(NavigateState { hidx, preview, cursor, .. }) => {
                let min_hidx = HistIdx(0);
                if *hidx == min_hidx {
                    // NOP, at the top of the History
                } else {
                    *hidx -= 1;
                    *preview = self.history[*hidx].clone(); // update
                    *cursor = preview.end_of_cmd();
                }
            }
            State::Search(SearchState { preview, matches, current, .. }) => {
                if *current >= matches.len() - 1 {
                    // NOP
                } else {
                    *current += 1;
                    *preview = if matches.is_empty() {
                        Cmd::default()
                    } else {
                        let hidx = matches[*current];
                        self.history[hidx].clone()
                    };
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
                    *cursor = preview.end_of_cmd();
                }
            }
            State::Search(SearchState { preview, matches, current, .. }) => {
                if *current == 0 {
                    // NOP
                } else {
                    *current -= 1;
                    *preview = if matches.is_empty() {
                        Cmd::default()
                    } else {
                        let hidx = matches[*current];
                        self.history[hidx].clone()
                    };
                }
            }
        }
        Ok(())
    }

    fn cmd_nav_cmd_left(&mut self) -> ReplBlockResult<()> {
        let update_cursor = |cmd: &Cmd, cursor: &mut Coords| {
            if *cursor == ORIGIN {
                // NOP
            } else {
                let is_start_of_cursor_line = cursor.x == ORIGIN.x;
                let has_prev_line = cursor.y >= 1;
                if is_start_of_cursor_line && has_prev_line {
                    *cursor = Coords {
                        x: cmd[cursor.y - 1].count_graphemes(),
                        y: cursor.y - 1,
                    };
                } else if is_start_of_cursor_line && !has_prev_line {
                    // NOP
                } else { // not at the start of a line
                    cursor.x -= 1;
                }
            }
        };
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                update_cursor(buffer, cursor);
            },
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                update_cursor(preview, cursor);
            },
            State::Search(SearchState { cursor, .. }) => {
                let prompt_len = self.reverse_search_prompt.len() as u16;
                if cursor.x <= prompt_len {
                    cursor.x = prompt_len; // bound here
                } else {
                    cursor.x -= 1;
                }
            },
        }
        Ok(())
    }

    fn cmd_nav_cmd_right(&mut self) -> ReplBlockResult<()> {
        let update_cursor = |cmd: &Cmd, cursor: &mut Coords| {
            if *cursor == cmd.end_of_cmd() {
                // NOP
            } else {
                let is_end_of_cursor_line =
                    cursor.x == cmd[cursor.y].count_graphemes();
                let has_next_line = cursor.y + 1 < cmd.count_lines();
                if is_end_of_cursor_line && has_next_line {
                    *cursor = Coords {
                        x: ORIGIN.x,
                        y: cursor.y + 1,
                    };
                } else if is_end_of_cursor_line && !has_next_line {
                    // NOP
                } else { // not the end of the line
                    cursor.x += 1;
                }
            }
        };
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                update_cursor(buffer, cursor);
            },
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                update_cursor(preview, cursor);
            },
            State::Search(SearchState { regex, cursor, .. }) => {
                let prompt_len = self.reverse_search_prompt.len() as u16;
                let regex_line_len = regex.graphemes(true).count() as u16;
                if cursor.x >= prompt_len + regex_line_len {
                    cursor.x = prompt_len + regex_line_len; // bound here
                } else {
                    cursor.x += 1;
                }
            },
        }
        Ok(())
    }

    /// Navigate to the start of the current Cmd
    fn cmd_nav_to_start_of_cmd(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { cursor, .. }) => {
                *cursor = ORIGIN;
            },
            State::Navigate(NavigateState { cursor, .. }) => {
                *cursor = ORIGIN;
            },
            State::Search(SearchState { cursor, .. }) => {
                let prompt_len = self.reverse_search_prompt.len() as u16;
                cursor.x = prompt_len;
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
            State::Search(SearchState { regex, cursor, .. }) => {
                let prompt_len = self.reverse_search_prompt.len() as u16;
                let regex_line_len = regex.graphemes(true).count() as u16;
                cursor.x = prompt_len + regex_line_len;
            },
        }
        Ok(())
    }

    fn cmd_reverse_search_history(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                self.state = State::Search(SearchState {
                    regex: String::new(),
                    backup: std::mem::take(buffer),
                    preview: Cmd::default(),
                    cursor: *cursor,
                    matches: vec![],
                    current: 0,
                });
                self.cmd_reverse_search_history()?;
            }
            State::Navigate(NavigateState { hidx: _, backup, preview, cursor }) => {
                self.state = State::Search(SearchState {
                    regex: String::new(),
                    backup: std::mem::take(backup),
                    preview: std::mem::take(preview),
                    cursor: *cursor,
                    matches: vec![],
                    current: 0,
                });
                self.cmd_reverse_search_history()?;
            }
            State::Search(SearchState {
                regex,
                backup: _,
                preview,
                cursor,
                matches,
                current,
            }) => {
                *matches = self.history.reverse_search(regex);
                *current = 0;
                *preview = if matches.is_empty() {
                    Cmd::default()
                } else {
                    self.history[matches[*current]].clone()
                };
                let prompt_len = self.reverse_search_prompt.len() as u16;
                *cursor = Coords { x: prompt_len, y: ORIGIN.y };
            }
        }
        Ok(())
    }

    /// Insert a char into the current cmd at cursor position.
    fn cmd_insert_char(&mut self, c: char) -> ReplBlockResult<()> {
        let dims = self.input_area_dims()?;
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
            State::Search(SearchState {
                regex,
                backup: _,
                preview,
                cursor,
                matches,
                current,
            }) => {
                let prompt_len = self.reverse_search_prompt.len();
                if regex.len() >= dims.width as usize - prompt_len - 1 {
                    return Ok(()); // NOP
                }
                let mut re: Vec<&str> = regex.graphemes(true).collect();
                let c = c.to_string();
                re.insert(cursor.x as usize - prompt_len, &c);
                *regex = re.into_iter().collect::<String>();
                cursor.x += 1;
                *matches = self.history.reverse_search(regex);
                *current = 0;
                *preview = if matches.is_empty() {
                    Cmd::default()
                } else {
                    let hidx = matches[*current];
                    self.history[hidx].clone()
                };
            }
        }
        Ok(())
    }

    /// Add a newline to the current cmd
    fn cmd_insert_newline(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                buffer.insert_empty_line(*cursor);
                *cursor = Coords {
                    x: ORIGIN.x,
                    y: cursor.y + 1
                };
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_insert_newline()?;
            }
            State::Search(SearchState { .. }) => {
                // NOP
            }
        }
        Ok(())
    }

    /// Delete the grapheme before the cursor in of the current cmd
    /// Do nothing if there if there is no grapheme before the cursor.
    fn cmd_rm_grapheme_before_cursor(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                if cursor.y == 0 && cursor.x == 0 {
                    // NOP
                } else if cursor.y == 0 && cursor.x > 0 {
                    buffer.rm_grapheme_before(*cursor);
                    cursor.x -= 1;
                } else if cursor.y > 0 && cursor.x == 0 {
                    let old_len = buffer[cursor.y - 1].count_graphemes();
                    buffer.rm_grapheme_before(*cursor);
                    *cursor = Coords { x: old_len, y: cursor.y - 1 };
                } else if cursor.y > 0 && cursor.x > 0 {
                    buffer.rm_grapheme_before(*cursor);
                    cursor.x -= 1;
                } else {
                    let tag = "cmd_rm_grapheme_before_cursor";
                    unreachable!("[{tag}] cursor={cursor:?}");
                }
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_rm_grapheme_before_cursor()?;
            }
            State::Search(SearchState {
                regex,
                backup: _,
                preview,
                cursor,
                matches,
                current,
            }) => {
                let prompt_len = self.reverse_search_prompt.len();
                let rmidx = cursor.x as usize - prompt_len;
                if regex.len() == 0 || rmidx == 0 {
                    return Ok(()); // NOP
                }
                let mut re: Vec<&str> = regex.graphemes(true).collect();
                re.remove(cursor.x as usize - prompt_len - 1);
                *regex = re.into_iter().collect::<String>();
                cursor.x -= 1;
                *matches = self.history.reverse_search(regex);
                *preview = if matches.is_empty() {
                    Cmd::default()
                } else {
                    let hidx = matches[*current];
                    self.history[hidx].clone()
                };
            },
        }
        Ok(())
    }

    /// Delete the grapheme at the position of the cursor in of the current cmd.
    /// Do nothing if there if there is no grapheme at the cursor.
    fn cmd_rm_grapheme_at_cursor(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                let is_end_of_line = cursor.x == buffer[cursor.y].count_graphemes();
                let has_next_line = cursor.y + 1 < buffer.count_lines();
                if is_end_of_line && has_next_line {
                    buffer.rm_grapheme_at(*cursor);
                } else if is_end_of_line && !has_next_line {
                    // NOP
                } else if !is_end_of_line {
                    buffer.rm_grapheme_at(*cursor);
                } else {
                    let tag = "cmd_rm_grapheme_at_cursor";
                    unreachable!("[{tag}] cursor={cursor:?}");
                }
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_rm_grapheme_at_cursor()?;
            }
            State::Search(SearchState {
                regex,
                backup: _,
                preview,
                cursor,
                matches,
                current,
            }) => {
                let prompt_len = self.reverse_search_prompt.len();
                let rmidx = cursor.x as usize - prompt_len;
                let is_end_of_regex_line = rmidx == regex.graphemes(true).count();
                if regex.len() == 0 || is_end_of_regex_line {
                    return Ok(()); // NOP
                }
                let mut re: Vec<&str> = regex.graphemes(true).collect();
                re.remove(cursor.x as usize - prompt_len);
                *regex = re.into_iter().collect::<String>();
                *matches = self.history.reverse_search(regex);
                *preview = if matches.is_empty() {
                    Cmd::default()
                } else {
                    let hidx = matches[*current];
                    self.history[hidx].clone()
                };
            }
        }
        Ok(())
    }

    /// Execute the current cmd
    fn cmd_eval(&mut self) -> ReplBlockResult<()> {
        match &mut self.state {
            State::Edit(EditState { buffer, cursor }) => {
                let source_code = buffer.to_source_code();
                if source_code.is_empty() {
                    return Ok(());
                }
                let cmd = std::mem::take(buffer);
                let _hidx = self.history.add_cmd(cmd);
                self.history.write_to_file(&self.history_filepath)?;
                (*self.evaluator)(source_code.as_str())?;
                self.height = 1; // reset
                *cursor = ORIGIN;
            }
            State::Navigate(NavigateState { preview, cursor, .. }) => {
                self.state = State::Edit(EditState {
                    buffer: std::mem::take(preview),
                    cursor: *cursor,
                });
                self.cmd_eval()?;
            }
            State::Search(SearchState { preview, cursor, .. }) => {
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

pub(crate) const ORIGIN: Coords = Coords { x: 0, y: 0 };

impl std::fmt::Display for Coords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}


#[derive(Debug)]
enum State {
    Edit(EditState),
    Navigate(NavigateState),
    Search(SearchState),
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

/// Searching backwards through the History for entries that match a regex
#[derive(Debug)]
struct SearchState {
    /// The regex being searched for
    regex: String,
    /// A buffer containing the Cmd that was last edited
    backup: Cmd,
    /// The `History` entry being previewed
    preview: Cmd,
    /// The cursor position within the Cmd buffer
    cursor: Coords,
    /// The `History` entries that match `regex`
    matches: Vec<HistIdx>,
    /// The current entry in `self.matches`
    current: usize,
}
