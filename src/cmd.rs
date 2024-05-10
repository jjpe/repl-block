//!

use crate::editor::{Coords, CursorFlags, Dims};
use unicode_segmentation::UnicodeSegmentation;


#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
pub struct Cmd { lines: Vec<Line> }

impl Default for Cmd {
    fn default() -> Self {
        Self { lines: vec![Line::with_capacity(Self::LINE_CAPACITY)] }
    }
}

impl Cmd {
    const LINE_CAPACITY: usize = 200;

    pub fn count_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn insert_char(&mut self, pos: Coords, c: char) {
        if self.lines.is_empty() {
            self.push_empty_line(LineKind::Start);
        }
        self[pos.y].insert_char(pos.x, c);
    }

    pub fn push_empty_line(&mut self, kind: LineKind) {
        self.lines.push(Line::with_capacity(Self::LINE_CAPACITY));
        self[Last].kind = kind;
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

    /// Remove the grapheme before a given `pos`ition.
    pub fn rm_grapheme_before(
        &mut self,
        pos: Coords,
        // The width (in columns) of the Editor
        editor_dims: Dims,
        // The length of the prompt
        prompt_len: u16,
    ) {
        if self.is_empty() {
            return; // nothing to remove
        }

        let CursorFlags {
            is_top_cmd_line,
            is_bottom_cmd_line,
            is_start_of_cmd_line,
            is_end_of_cmd_line,
            is_top_editor_row,
            is_bottom_editor_row,
            is_leftmost_editor_column,
            is_rightmost_editor_column,
        } = pos.flags(editor_dims, prompt_len, self);

        let is_continuation = self[pos.y].is_continue();
        let is_prompt_line = pos.y == 0;
        let is_start_of_prompt_line = pos.x <= prompt_len;
        let is_start_of_nonprompt_line = if is_continuation {
            pos.x == 0
        } else {
            pos.x <= prompt_len
        };
        if is_prompt_line && is_start_of_prompt_line {
            // NOP: no graphemes to remove before origin
        } else if is_prompt_line {
            self[pos.y].rm_grapheme_before(pos.x);
        } else { // not on the prompt line
            if is_start_of_nonprompt_line {
                let removed: Line = self.lines.remove(pos.y as usize);
                self[pos.y - 1].push_str(removed.as_str());
            } else { // not at the start of the line
                self[pos.y].rm_grapheme_before(pos.x);
            }
        }

        // else if is_continuation {
        //     if is_start_of_nonprompt_line {
        //     } else {
        //     }
        // }
        // else { // pos.x > 0
        //     self.lines[pos.y as usize].rm_grapheme_before(pos.x);
        // }

    }

    /// Remove the grapheme at a given `pos`ition.
    pub fn rm_grapheme_at(
        &mut self,
        pos: Coords,
        // // The width (in columns) of the Editor
        // editor_width: u16,
        // // The length of the prompt
        // prompt_len: u16,
    ) {
        if self.is_empty() {
            return; // nothing to remove
        }
        self[pos.y].rm_grapheme_at(pos.x);

        // let is_prompt_line = pos.x == 0;
        // let is_start_of_prompt_line = pos.y == prompt_len;
        // let is_start_of_nonprompt_line = pos.y == 0;
        // if pos.x == 0 && pos.y == 0 {
        //     self[pos.y].rm_grapheme_at(pos.x);
        // } else if pos.x == 0 {
        //     let removed: Line = self.lines.remove(pos.y as usize);
        //     self[pos.y - 1].push_str(removed.as_str());
        // } else { // pos.x > 0
        //     self[pos.y].rm_grapheme_at(pos.x);
        // }

    }

    pub fn lines(&self) -> &[Line] {
        self.lines.as_slice()
    }

    // Compression here means that all line continuations (which exist for the
    // purpose of line overflow rendering) have been merged with their starting
    // line.
    // Compressed Cmds are used for storage, but also for cleanup after user
    // edits e.g. insertions.
    pub(crate) fn compress(&self) -> Self {
        if self.is_empty() {
            return self.clone();
        }
        let mut clines = vec![Line::new_start()];
        for (lidx, line) in self.lines().iter().enumerate() {
            if lidx == 0 { // nothing to continue
                let last = clines.last_mut().unwrap();
                last.push_str(line.as_str());
            } else if line.kind == LineKind::Continue {
                let last = clines.last_mut().unwrap();
                last.push_str(line.as_str());
            } else {
                clines.push(line.clone());
            }
        }
        Self { lines: clines }
    }

    // Uncompression here means that `self` has been been through a layout
    // process.
    // Uncompressed Cmds are used for rendering, but also result from user
    // edits e.g. insertions.
    pub(crate) fn uncompress(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt
        prompt_len: u16,
    ) -> Self {
        let mut ulines = vec![];
        for line in self.lines().iter() {
            if line.is_empty() {
                ulines.push(line.clone());
            } else {
                ulines.extend(line.uncompress(editor_width, prompt_len));
            }
        }
        if let Some((lidx, last)) = ulines.iter().enumerate().last() {
            if last.fills_editor_width(editor_width, prompt_len, lidx) {
                ulines.push(Line::new_continue());
            }
        }
        Self { lines: ulines }
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

    pub fn end_of_cmd(&self) -> Coords {
        self.lines.last()
            .map(|last| Coords {
                x: last.count_graphemes(),
                y: self.max_line_idx().unwrap() as u16,
            })
            .unwrap_or(Coords::EDITOR_ORIGIN)
    }

}

impl std::fmt::Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Alignment::Right;
        const INDENT: &str = "  ";
        match &self.lines[..] {
            [] => {},
            [lines@.., last] => {
                if let (Some(Right), Some(width)) = (f.align(), f.width()) {
                    for line in lines {
                        for _ in 0..width { write!(f, "{INDENT}")?; }
                        writeln!(f, "{line}")?;
                    }
                    for _ in 0..width { write!(f, "{INDENT}")?; }
                    write!(f, "{last}")?;
                } else {
                    for line in lines {
                        writeln!(f, "{line}")?;
                    }
                    write!(f, "{last}")?;
                }
            }
        }
        Ok(())
    }
}

impl std::ops::Index<u16> for Cmd {
    type Output = Line;

    fn index(&self, index: u16) -> &Self::Output {
        &self.lines[index as usize]
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

impl std::ops::IndexMut<u16> for Cmd {
    fn index_mut(&mut self, index: u16) -> &mut Self::Output {
        &mut self.lines[index as usize]
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
#[serde(transparent)]
pub struct Line {
    content: String,
    #[serde(skip)]
    // #[serde(skip_deserializing)]
    #[serde(default)]
    pub(crate) kind: LineKind,
}

impl Line {
    fn new(kind: LineKind) -> Self {
        Self { content: String::new(), kind }
    }

    fn new_start() -> Self {
        Self::new(LineKind::Start)
    }

    fn new_continue() -> Self {
        Self::new(LineKind::Continue)
    }

    fn with_capacity(cap: usize) -> Self {
        Self {
            content: String::with_capacity(cap),
            kind: LineKind::Start,
        }
    }

    pub fn is_start(&self) -> bool {
        self.kind == LineKind::Start
    }

    pub fn is_continue(&self) -> bool {
        self.kind == LineKind::Continue
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    pub fn insert_char(&mut self, x_pos: u16, c: char) {
        let mut graphemes = self.graphemes();
        let mut content = String::new();
        for _ in 0..x_pos {
            let Some(g) = graphemes.next() else { break };
            content.push_str(g);
        }
        content.push(c);
        while let Some(g) = graphemes.next() {
            content.push_str(g);
        }
        drop(graphemes);
        self.content = content;
    }

    pub fn insert_str(&mut self, x_pos: u16, s: &str) {
        let mut graphemes = self.graphemes();
        let mut content = String::new();
        for _ in 0..x_pos {
            let Some(g) = graphemes.next() else { break };
            content.push_str(g);
        }
        content.push_str(s);
        while let Some(g) = graphemes.next() {
            content.push_str(g);
        }
        drop(graphemes);
        self.content = content;
    }

    pub fn graphemes(&self) -> impl Iterator<Item = &str> + '_ {
        self.content.graphemes(true)
    }

    pub fn grapheme_indices(&self) -> impl Iterator<Item = (usize, &str)> {
        self.content.grapheme_indices(true)
    }

    pub fn max_x(&self) -> u16 {
        self.count_graphemes().saturating_sub(1)
    }

    pub fn count_graphemes(&self) -> u16 {
        self.content.graphemes(true).count() as _
    }

    // pub fn count_chars(&self) -> u16 {
    //     self.content.chars().count() as _
    // }

    // pub fn count_bytes(&self) -> usize {
    //     self.content.len() as _
    // }

    pub fn push_char(&mut self, c: char) {
        self.content.push(c);
    }

    pub fn push_str(&mut self, s: &str) {
        self.content.push_str(s);
    }

    pub fn pop(&mut self) -> Option<char> {
        self.content.pop()
    }

    pub fn as_str(&self) -> &str {
        self.content.as_str()
    }

    pub fn rm_grapheme_before(&mut self, xpos: u16) {
        if xpos == 0 {
            return; // No graphemes to remove before the start of `self`
        }
        self.rm_grapheme_at(xpos - 1);
    }

    pub fn rm_grapheme_at(&mut self, xpos: u16) {
        // println!("removing grapheme, x_pos={xpos} line={self:?}");
        *self = Self {
            content: self.graphemes().enumerate()
                .filter(|&(gidx, _)| gidx != xpos as usize)
                .map(|(_, grapheme)| grapheme)
                .collect(),
            kind: self.kind,
        };
        // println!("removed grapheme, line={self:?}");
    }

    fn fills_editor_width(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt
        prompt_len: u16,
        line_idx: usize,
    ) -> bool {
        if line_idx == 0 {
            self.count_graphemes() == editor_width - prompt_len
        } else if self.is_start() {
            self.count_graphemes() == editor_width - prompt_len
        } else {
            self.count_graphemes() == editor_width
        }
    }

    pub(crate) fn uncompress(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt
        prompt_len: u16,
    ) -> Vec<Self> {
        let mut ulines = vec![];
        let mut graphemes = self.graphemes().peekable();

        let mut lline0 = Line::new(self.kind);
        let start = if self.is_start() { prompt_len } else { 0 };
        for _ in start..editor_width {
            let Some(g) = graphemes.next() else { break };
            lline0.push_str(g);
        }
        if !lline0.is_empty() {
            ulines.push(lline0);
        }

        'continue_lines: loop {
            let mut lline = Line::new_continue();
            for _ in 0 .. editor_width {
                let Some(g) = graphemes.next() else { break };
                lline.push_str(g);
            }
            if !lline.is_empty() {
                ulines.push(lline);
            }

            if graphemes.peek().is_none() {
                break 'continue_lines;
            }
        }

        ulines
    }

}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
pub enum LineKind {
    #[default]
    Start,
    Continue
}


#[cfg(test)]
mod test {
    use crate::error::ReplBlockResult;
    use super::*;

    #[test]
    fn compression_reversibility() -> ReplBlockResult<()> {
        let cmd = Cmd {
            lines: vec![
                Line {
                    // This line is intentionally very long without line breaks.
                    content: r#"<xml a="b">hello<?do-it a proc instr?><!--a comment-->world<kid a="b"/><![CDATA[boom bam]]>&lt;&amp;&gt;&#x20;{{more text}}</xml>/descendant-or-self::processing-instruction()"#.to_string(),
                    kind: LineKind::Start,
                }
            ]
        };

        // Plausible, but otherwise unspecial values:
        let term_cols = 100;
        let prompt_len = 3;

        let uclines = cmd.uncompress(term_cols, prompt_len);
        println!("uclines={uclines:#?}");
        let ucmd = Cmd {
            lines: vec![
                Line {
                    // length == term_cols- prompt_len
                    content: r#"<xml a="b">hello<?do-it a proc instr?><!--a comment-->world<kid a="b"/><![CDATA[boom bam]]>&lt;&a"#.to_string(),
                    kind: LineKind::Start,
                },
                Line {
                    content: r#"mp;&gt;&#x20;{{more text}}</xml>/descendant-or-self::processing-instruction()"#.to_string(),
                    kind: LineKind::Continue,
                }
            ]
        };
        assert_eq!(ucmd, uclines);

        let clines = uclines.compress();
        println!("clines={clines:#?}");
        assert_eq!(cmd, clines);
        let ccmd = Cmd {
            lines: vec![
                Line {
                    // This line is intentionally very long without line breaks.
                    content: r#"<xml a="b">hello<?do-it a proc instr?><!--a comment-->world<kid a="b"/><![CDATA[boom bam]]>&lt;&amp;&gt;&#x20;{{more text}}</xml>/descendant-or-self::processing-instruction()"#.to_string(),
                    kind: LineKind::Start,
                }
            ]
        };
        assert_eq!(ccmd, clines);
        assert_eq!(ccmd, cmd);

        let uclines2 = cmd.uncompress(term_cols, prompt_len);
        println!("uclines2={uclines2:#?}");
        assert_eq!(uclines, uclines2);
        let ucmd2 = Cmd {
            lines: vec![
                Line {
                    // length == term_cols- prompt_len
                    content: r#"<xml a="b">hello<?do-it a proc instr?><!--a comment-->world<kid a="b"/><![CDATA[boom bam]]>&lt;&a"#.to_string(),
                    kind: LineKind::Start,
                },
                Line {
                    content: r#"mp;&gt;&#x20;{{more text}}</xml>/descendant-or-self::processing-instruction()"#.to_string(),
                    kind: LineKind::Continue,
                }
            ]
        };
        assert_eq!(ucmd2, uclines2);

        Ok(())
    }
}
