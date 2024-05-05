//!

use camino::Utf8Path;
use crate::{
    cmd::{Cmd, Last},
    error::ReplBlockResult,
};
use itertools::Itertools;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct History {
    /// A list of commands
    cmds: VecDeque<Cmd>,
}

impl Default for History {
    fn default() -> Self {
        Self { cmds: VecDeque::with_capacity(Self::UPPER_LIMIT) }
    }
}

impl History {
    const UPPER_LIMIT: usize = 1000;

    pub fn read_from_file(filepath: impl AsRef<Utf8Path>) -> ReplBlockResult<Self> {
        let filepath = filepath.as_ref();
        let mut file = if filepath.exists() {
            File::open(filepath)?
        } else {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(true)
                .open(&filepath)?;
            file.write_all(&[])?;
            file.flush()?;
            file
        };
        let mut contents = String::with_capacity(8 * 1024);
        let read_bytes = file.read_to_string(&mut contents)?;
        if read_bytes == 0 { // emtpy file
            Ok(Self::default())
        } else {
            Ok(serde_json::from_str::<Self>(&contents)?)
        }
    }

    pub fn write_to_file(&self, path: impl AsRef<Utf8Path>) -> ReplBlockResult<()> {
        let mut file = OpenOptions::new()
            .truncate(true)
            .write(true)
            .open(path.as_ref())?;
        let json: String = serde_json::to_string_pretty(&self)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn add_cmd(&mut self, cmd: Cmd) -> HistIdx {
        let idx = HistIdx(self.cmds.len());
        self.cmds.push_back(cmd);
        idx
    }

    pub fn last_cmd(&self) -> Option<&Cmd> {
        self.cmds.back()
    }

    // TODO: use this
    pub fn trim_cmds(&mut self) {
        // while self.cmds.len() > UPPER_LIMIT {
        //     self.cmds.pop_front();
        // }
        self.cmds = self.cmds.drain(..)
            .rev()
            .unique() // purge the non-newest non-unique cmds
            .rev()
            .take(Self::UPPER_LIMIT)
            .collect();
    }

    pub fn count_cmds(&self) -> usize {
        self.cmds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn max_idx(&self) -> Option<HistIdx> {
        let num_cmds = self.count_cmds();
        if num_cmds > 0 {
            Some(HistIdx::from(num_cmds - 1))
        } else {
            None
        }
    }
}

impl std::fmt::Display for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "History:")?;
        for cmd in &self.cmds {
            writeln!(f, "{cmd:>1}")?;
        }
        Ok(())
    }
}

impl std::ops::Index<HistIdx> for History {
    type Output = Cmd;

    fn index(&self, index: HistIdx) -> &Self::Output {
        &self.cmds[index.0]
    }
}

impl std::ops::IndexMut<HistIdx> for History {
    fn index_mut (&mut self, index: HistIdx) -> &mut Self::Output {
        &mut self.cmds[index.0]
    }
}

impl std::ops::Index<Last> for History {
    type Output = Cmd;

    fn index(&self, _: Last) -> &Self::Output {
        let hidx = HistIdx(self.cmds.len() - 1);
        &self[hidx]
    }
}

impl std::ops::IndexMut<Last> for History {
    fn index_mut(&mut self, _: Last) -> &mut Self::Output {
        let hidx = HistIdx(self.cmds.len() - 1);
        &mut self[hidx]
    }
}


#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize, derive_more::From)]
pub struct HistIdx(pub(crate) usize);

impl std::ops::Add<usize> for HistIdx {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl std::ops::AddAssign<usize> for HistIdx {
    fn add_assign(&mut self, rhs: usize) {
        *self = *self + rhs;
    }
}

impl std::ops::Sub<usize> for HistIdx {
    type Output = Self;

    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0 - rhs)
    }
}

impl std::ops::SubAssign<usize> for HistIdx {
    fn sub_assign(&mut self, rhs: usize) {
        *self = *self - rhs;
    }
}
