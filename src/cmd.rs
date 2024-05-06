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
        let mut llines = vec![];
        for (rowidx, line) in self.lines().iter().enumerate() {
            let mut graphemes = line.graphemes().peekable();

            let mut lline0 = Line::new();
            let start = if rowidx == 0 { prompt_len } else { 0 };
            for _ in start..editor_width {
                let Some(g) = graphemes.next() else { break };
                lline0.push_str(g);
            }
            if !lline0.is_empty() {
                llines.push(lline0);
            }

            'other_lines: loop {
                let mut lline = Line::new();
                for _ in 0 .. editor_width {
                    let Some(g) = graphemes.next() else { break };
                    lline.push_str(g);
                }
                if !lline.is_empty() {
                    llines.push(lline);
                }
                if graphemes.peek().is_none() {
                    break 'other_lines;
                }
            }
        }
        llines
    }

    pub fn count_logical_lines(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) -> u16 {
        let mut llines = 0;
        for (rowidx, line) in self.lines().iter().enumerate() {
            let mut graphemes = line.graphemes().peekable();

            let mut lline0_len = 0;
            let start = if rowidx == 0 { prompt_len } else { 0 };
            for _ in start..editor_width {
                let Some(_g) = graphemes.next() else { break };
                lline0_len += 1;
            }
            let is_lline0_empty = lline0_len == 0;
            llines += if is_lline0_empty { 0 } else { 1 };

            'other_lines: loop {
                let mut lline_len = 0;
                for _ in 0..editor_width {
                    let Some(_g) = graphemes.next() else { break };
                    lline_len += 1;
                }
                let is_lline_empty = lline_len == 0;
                llines += if is_lline_empty { 0 } else { 1 };
                if graphemes.peek().is_none() {
                    break 'other_lines;
                }
            }
        }
        llines
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

    pub fn end_of_cmd_cursor(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt on logical line 0
        prompt_len: u16,
    ) -> Coords {
        let llines = self.logical_lines(editor_width, prompt_len);
        if let Some(last) = llines.last() {
            Coords {
                x: if llines.len() == 1 {
                    prompt_len + last.count_graphemes()
                } else {
                    last.count_graphemes()
                },
                y: llines.len() as u16 - 1,
            }
        } else {
            Coords {
                x: prompt_len,
                y: Coords::ORIGIN.y,
            }
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
