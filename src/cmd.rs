//!

use crate::editor::Coords;
use unicode_segmentation::UnicodeSegmentation;


#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
pub struct Cmd { lines: Vec<Line> }

impl Default for Cmd {
    fn default() -> Self {
        Self { lines: vec![Line::new(LineKind::Start)] }
    }
}

impl Cmd {
    pub fn count_lines(&self) -> u16 {
        self.lines.len() as u16
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn insert_char(&mut self, pos: Coords, c: char) {
        if self.lines.is_empty() {
            self.lines.push(Line::new_start());
        }
        self[pos.y].insert_char(pos.x, c);
    }

    pub fn insert_empty_line(&mut self, pos: Coords) {
        self.lines.insert(pos.y as usize + 1, Line {
            content: self[pos.y].graphemes().skip(pos.x as usize).collect(),
            kind: LineKind::Start,
        });
        self[pos.y] = Line {
            content: self[pos.y].graphemes().take(pos.x as usize).collect(),
            kind: self[pos.y].kind,
        };
    }

    /// Remove the grapheme before a given `pos`ition.
    pub fn rm_grapheme_before(&mut self, pos: Coords) {
        if self.is_empty() {
            return; // nothing to remove
        }
        if pos.y == 0 && pos.x == 0 {
            // NOP
        } else if pos.y == 0 && pos.x > 0 {
            self[pos.y].rm_grapheme_before(pos.x);
        } else if pos.y > 0 && pos.x == 0 {
            let removed: Line = self.lines.remove(pos.y as usize);
            self[pos.y - 1].push_str(removed.as_str());
        } else if pos.y > 0 && pos.x > 0 {
            self[pos.y].rm_grapheme_before(pos.x);
        } else {
            let tag = "Cmd::rm_grapheme_before";
            unreachable!("[{tag}] pos={pos:?}");
        }
    }

    /// Remove the grapheme at a given `pos`ition.
    pub fn rm_grapheme_at(&mut self, pos: Coords) {
        if self.is_empty() {
            return; // nothing to remove
        }
        let is_end_of_line = pos.x == self[pos.y].count_graphemes();
        let has_next_line = pos.y + 1 < self.count_lines();
        if is_end_of_line && has_next_line {
            let removed: Line = self.lines.remove(pos.y as usize + 1);
            self[pos.y].push_str(removed.as_str());
        } else if is_end_of_line && !has_next_line {
            // NOP
        } else if !is_end_of_line {
            self[pos.y].rm_grapheme_at(pos.x);
        } else {
            let tag = "Cmd::rm_grapheme_at";
            unreachable!("[{tag}] pos={pos:?}");
        }
    }

    pub fn lines(&self) -> &[Line] {
        self.lines.as_slice()
    }

    #[cfg(test)]
    // Compression here means that all line continuations (which exist for the
    // purpose of line overflow rendering) have been merged with their starting
    // line.
    // Compressed Cmds are used for storage, but also for cleanup after user
    // edits e.g. insertions.
    pub(crate) fn compress(&self) -> Self {
        if self.is_empty() {
            return self.clone();
        }
        let mut clines = vec![];
        for line in self.lines().iter() {
            if line.is_start() {
                clines.push(line.clone());
            } else if line.kind == LineKind::Overflow {
                let prev = clines.last_mut().unwrap();
                prev.push_str(line.as_str());
            } else {
                unreachable!();
            }

            // if lidx == 0 { // nothing to continue
            //     let last = clines.last_mut().unwrap();
            //     last.push_str(line.as_str());
            // } else if line.kind == LineKind::Overflow {
            //     let last = clines.last_mut().unwrap();
            //     last.push_str(line.as_str());
            // } else {
            //     clines.push(line.clone());
            // }

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
        Self {
            lines: self.lines().iter()
                .flat_map(|line| line.uncompress(editor_width, prompt_len))
                .collect()
        }
    }

    pub fn max_line_idx(&self) -> Option<usize> {
        let num_lines = self.count_lines() as usize;
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

    pub fn to_source_code(&self) -> String {
        use itertools::Itertools;
        #[allow(unstable_name_collisions)] // for the .intersperse() call below
        self.lines.iter()
            .filter(|line| !line.is_empty())
            .map(Line::as_str)
            .intersperse("\n")
            .collect()
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


pub(crate) struct Last;

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
    const CAPACITY: usize = 200;

    fn new(kind: LineKind) -> Self {
        Self {
            content: String::with_capacity(Self::CAPACITY),
            kind
        }
    }

    pub(crate) fn new_start() -> Self {
        Self::new(LineKind::Start)
    }

    pub(crate) fn new_overflow() -> Self {
        Self::new(LineKind::Overflow)
    }

    pub fn is_start(&self) -> bool {
        self.kind == LineKind::Start
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

    pub fn push_str(&mut self, s: &str) {
        self.content.push_str(s);
    }

    pub fn as_str(&self) -> &str {
        self.content.as_str()
    }

    pub fn rm_grapheme_before(&mut self, xpos: u16) {
        if xpos == 0 {
            return; // No graphemes to remove
        }
        self.rm_grapheme_at(xpos - 1);
    }

    pub fn rm_grapheme_at(&mut self, xpos: u16) {
        *self = Self {
            content: self.graphemes().enumerate()
                .filter(|&(gidx, _)| gidx != xpos as usize)
                .map(|(_, grapheme)| grapheme)
                .collect(),
            kind: self.kind,
        };
    }

    pub(crate) fn uncompress(
        &self,
        // The width (in columns) of the Editor
        editor_width: u16,
        // The length of the prompt
        prompt_len: u16,
    ) -> Vec<Self> {
        if self.is_empty() {
            return vec![Line::new(self.kind)];
        }

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
            let mut lline = Line::new_overflow();
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
    Overflow
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
                    kind: LineKind::Overflow,
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
                    // length == term_cols - prompt_len
                    content: r#"<xml a="b">hello<?do-it a proc instr?><!--a comment-->world<kid a="b"/><![CDATA[boom bam]]>&lt;&a"#.to_string(),
                    kind: LineKind::Start,
                },
                Line {
                    content: r#"mp;&gt;&#x20;{{more text}}</xml>/descendant-or-self::processing-instruction()"#.to_string(),
                    kind: LineKind::Overflow,
                }
            ]
        };
        assert_eq!(ucmd2, uclines2);

        Ok(())
    }
}
