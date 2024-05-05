//!

use std::io::Write;

use crate::{
    error::ReplBlockResult,
    editor::Coords,
};
use crossterm::terminal;
use itertools::Itertools;
use unicode_segmentation::UnicodeSegmentation;


#[derive(Default, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
pub struct Cmd { lines: Vec<Line> }

impl Cmd {
    const LINE_CAPACITY: usize = 100;

    pub fn count_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn insert_char(
        &mut self,
        pos: Coords,
        c: char,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) {
        if self.lines().is_empty() {
            self.push_empty_line();
        }

        let mut llines: Vec<Line> = self.logical_lines(editor_width, prompt_len);

        println!();
        println!("pos={pos:?}");
        println!("c='{c}'");
        println!("cmd={self:#?}--------------");
        println!();
        std::io::stdout().flush().unwrap();
        // self.lines[pos.y as usize].insert_char(pos.x, c);
        llines[pos.y as usize].insert_char(pos.x, c);
        self.lines = llines;
        println!();
        println!("pos={pos:?}");
        println!("c='{c}'");
        println!("cmd={self:#?}--------------");
        println!("\n\n\n\n");

        self.rebalance(editor_width, prompt_len);
    }

    pub fn push_char(
        &mut self,
        c: char,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) {
        if self.lines.is_empty() {
            self.push_empty_line();
        }
        self[Last].push_char(c);
        self.rebalance(editor_width, prompt_len);
    }

    fn rebalance(
        &mut self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) {
        for lidx in 0..self.lines.len() {
            let line = &mut self[lidx];
            if line.count_graphemes() > editor_width {
                let spillover = line.graphemes()
                    .skip(editor_width as usize)
                    .collect::<String>();
                // Truncate the `line` to the first `editor_width` graphemes:
                *line = line.graphemes()
                    .take(editor_width as usize)
                    .collect::<Line>();
                if spillover.is_empty() {
                    continue
                }
                if self.lines.get(lidx + 1).is_none() {
                    self.push_empty_line();
                }
                if let Some(next) = self.lines.get_mut(lidx + 1) {
                    next.insert_str(0, &spillover);
                }
            }
        }
    }

    pub fn push_empty_line(&mut self) {
        self.lines.push(Line::with_capacity(Self::LINE_CAPACITY));
    }

    pub fn pop_char(&mut self) {
        if self.lines.is_empty() {
            // NOP
        } else if self[Last].is_empty() {
            self.lines.pop().unwrap();
        } else {
            self[Last].pop().unwrap();
        }
    }

    pub fn pop_chars(&mut self, num: usize) {
        for _ in 0..num {
            self.pop_char();
        }
    }

    pub fn rm_char(&mut self, pos: Coords) {
        if self.is_empty() {
            return; // nothing to remove
        }
        if pos.x == 0 && pos.y == 0 {
            self.lines[pos.y as usize].remove_grapheme(pos.x as usize);
        } else if pos.x == 0 {
            let removed: Line = self.lines.remove(pos.y as usize);
            let prev = pos.y as usize - 1;
            self.lines[prev].push_str(removed.as_str());
        } else { // pos.x > 0
            self.lines[pos.y as usize].remove_grapheme(pos.x as usize);
        }
    }

    pub fn lines(&self) -> &[Line] {
        self.lines.as_slice()
    }

    pub fn logical_lines(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) -> Vec<Line> {
        self.lines.iter()
            .flat_map(|line| line.logical_lines(editor_width, prompt_len))
            .collect()
    }

    pub fn count_logical_lines(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) -> u16 {
        let mut num = 0;
        for line in self.lines().iter() {
            num += line.count_logical_lines(editor_width, prompt_len);
        }
        num
    }

    pub fn rm_line(&mut self, lineno: usize) {
        self.lines.remove(lineno);
    }

    pub fn max_line_idx(&self) -> Option<usize> {
        let num_lines = self.count_lines();
        if num_lines > 0 {
            Some(num_lines - 1)
        } else {
            None
        }
    }

}

impl std::fmt::Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Alignment::Right;
        const INDENT: &str = "  ";
        match &self.lines[..] {
            [] => {},
            [lines@.., last] => {
                for line in lines {
                    if let (Some(Right), Some(width)) = (f.align(), f.width()) {
                        for _ in 0..width { write!(f, "{INDENT}")?; }
                    }
                    writeln!(f, "{line}")?;
                }
                if let (Some(Right), Some(width)) = (f.align(), f.width()) {
                    for _ in 0..width { write!(f, "{INDENT}")?; }
                }
                write!(f, "{last}")?;
            }
        }
        Ok(())
    }
}

impl std::ops::Index<usize> for Cmd {
    type Output = Line;

    fn index(&self, index: usize) -> &Self::Output {
        &self.lines[index]
    }
}


pub struct Last;

impl std::ops::Index<Last> for Cmd {
    type Output = Line;

    fn index(&self, _: Last) -> &Self::Output {
        let last = self.lines.len() - 1;
        &self.lines[last]
    }
}

impl std::ops::IndexMut<usize> for Cmd {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.lines[index]
    }
}

impl std::ops::IndexMut<Last> for Cmd {
    fn index_mut(&mut self, _: Last) -> &mut Self::Output {
        let last = self.lines.len() - 1;
        &mut self.lines[last]
    }
}


#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
#[derive(derive_more::From)]
pub struct Line(String);

impl Line {
    pub fn new() -> Self {
        Self(String::new())
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(String::with_capacity(cap))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn insert_char(&mut self, x_pos: u16, c: char) {
        let byte_idx = self.grapheme_indices().enumerate()
            .find(|(i, (_byte_idx, _grapheme))| *i == x_pos as usize)
            .map(|(_i, (byte_idx, _grapheme))| byte_idx);
        let Some(byte_idx) = byte_idx else {
            if x_pos == self.count_graphemes() {
                self.0.push(c);
                return;
            } else {
                panic!("Index is out of bounds: x_pox={x_pos},  line={self:?}");
            }
        };
        self.0.insert(byte_idx, c);
    }

    pub fn insert_str(&mut self, x_pos: u16, s: &str) {
        let byte_idx = self.grapheme_indices().enumerate()
            .find(|(i, (_byte_idx, _grapheme))| *i == x_pos as usize)
            .map(|(_i, (byte_idx, _grapheme))| byte_idx);
        let Some(byte_idx) = byte_idx else {
            if x_pos == self.count_graphemes() {
                self.0.push_str(s);
                return;
            } else {
                panic!("Index is out of bounds: idx={x_pos},  line={self:?}");
            }
        };
        for s in s.graphemes(true) {
            self.0.insert_str(byte_idx, s);
        }
    }

    pub fn does_overflow(&self) -> ReplBlockResult<bool> {
        // !matches!(self.num_logical_lines(), Ok(1))
        let num_graphemes = self.count_graphemes();
        let (num_cols, _) = terminal::size()?;
        Ok(num_graphemes > num_cols)
    }

    pub fn logical_lines(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) -> Vec<Line> {
        let mut graphemes = self.graphemes();
        let mut first = Line::new();
        for _ in 0 .. editor_width - prompt_len {
            match graphemes.next() {
                Some(g) => first.push_str(g),
                None => return vec![first], // End of the first & only line
            }
        }
        let lines: Vec<Line> = std::iter::once(first)
            .chain(
                graphemes
                    .chunks(editor_width as usize).into_iter()
                    .map(|chunk| chunk.collect::<Line>())
            )
            .collect();
        assert!(lines.len() >= 1);
        lines
    }

    pub fn count_logical_lines(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) -> u16 {

        let mut num_lines = 1;
        let mut num_graphemes = self.count_graphemes();
        let line_len = std::cmp::min(
            editor_width - prompt_len, // line overflow
            num_graphemes, // no line overflow
        );
        num_graphemes -= line_len; // first line
        while num_graphemes > 0 {
            let line_len = std::cmp::min(
                editor_width,  // line overflow
                num_graphemes, // no line overflow
            );
            num_graphemes -= line_len; // rest of the lines
            num_lines += 1;
        }
        assert!(num_lines >= 1);
        num_lines

        // let line_len = prompt_len + num_graphemes;
        // 1 /*remaining non-full row*/ + (line_len / editor_width)

    }

    pub fn graphemes(&self) -> impl Iterator<Item = &str> + '_ {
        self.0.graphemes(true)
    }

    pub fn grapheme_indices(&self) -> impl Iterator<Item = (usize, &str)> {
        self.0.grapheme_indices(true)
    }

    pub fn count_graphemes(&self) -> u16 {
        self.0.graphemes(true).count() as _
    }

    pub fn count_chars(&self) -> u16 {
        self.0.chars().count() as _
    }

    pub fn count_bytes(&self) -> usize {
        self.0.len() as _
    }

    pub fn push_char(&mut self, c: char) {
        self.0.push(c);
    }

    pub fn push_str(&mut self, s: &str) {
        self.0.push_str(s);
    }

    pub fn pop(&mut self) -> Option<char> {
        self.0.pop()
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn remove_grapheme(&mut self, grapheme_idx: usize) {
        *self = self.graphemes().enumerate()
            .filter(|&(gidx, _)| gidx != grapheme_idx)
            .map(|(_, grapheme)| grapheme)
            .collect();
    }
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'iter> FromIterator<&'iter str> for Line {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = &'iter str>,
    {
        Self(iter.into_iter().collect())
    }
}
