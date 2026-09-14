//! One line of work, and the several of them a browser can hold.
//!
//! A pane is a thread, the repository it runs in, and the task being written
//! into it. Everything else — the runs recorded here, the record being read,
//! whatever dialog is open — belongs to the workspace rather than to a pane,
//! and stays where it is.
//!
//! **Side by side where there is room, switched between where there is not.**
//! Two transcripts on an eighty-column terminal are two transcripts nobody can
//! read, so below 160 columns one pane has the screen and the rest are a row at
//! the top. From 160 columns each gets a column of its own. Where the columns
//! go, and how wide each is, is the operator's to arrange.

use crate::tui::thread::Thread;
use crate::workspace::Repository;

/// The share of the width a pane starts with, and the most it can be given.
/// A pane at the most beside one at the least still leaves the narrow one a
/// column wide enough to read, because every column is given its minimum first.
pub const WEIGHT: u16 = 3;
pub const MAX_WEIGHT: u16 = 9;

pub struct Pane {
    pub thread: Thread,
    /// The repository this line of work is in. `None` until one is chosen.
    pub repository: Option<Repository>,
    /// The task being written here.
    pub prompt: String,
    /// Where typing lands in the task, as a byte offset. `None` is the end,
    /// which is where it goes back to whenever the task is replaced whole — so
    /// nothing that writes `prompt` directly can leave it pointing nowhere.
    pub cursor: Option<usize>,
    /// How far back into what has been asked here the box has been walked.
    pub history_at: Option<usize>,
    /// Whether the transcript sticks to the bottom as the run writes to it.
    pub follow: bool,
    /// This pane's share of the width, when the panes are side by side.
    pub weight: u16,
    /// Where this pane's transcript was scrolled to while another pane had the
    /// keys. The pane in front scrolls with the screen; a column beside it
    /// keeps its own place rather than jumping whenever the keys move.
    pub scroll: u16,
}

impl Default for Pane {
    fn default() -> Self {
        Self {
            thread: Thread::default(),
            repository: None,
            prompt: String::new(),
            cursor: None,
            history_at: None,
            follow: true,
            weight: WEIGHT,
            scroll: 0,
        }
    }
}

impl Pane {
    pub fn new(repository: Option<Repository>) -> Self {
        Self {
            repository,
            ..Self::default()
        }
    }

    /// What the pane is called on the bar: the repository it works in, or the
    /// task it is working on where that says more.
    pub fn title(&self) -> String {
        if let Some(repo) = &self.repository {
            return repo.name.clone();
        }
        match self.thread.turns.first() {
            Some(turn) => turn.prompt.clone(),
            None => "new".to_string(),
        }
    }

    pub fn running(&self) -> bool {
        self.thread.running()
    }

    /// Where typing lands, held on a character boundary inside the task
    /// whatever has been done to the task since the cursor was put there.
    pub fn at(&self) -> usize {
        let Some(at) = self.cursor else {
            return self.prompt.len();
        };
        let mut at = at.min(self.prompt.len());
        while !self.prompt.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// Replaces the task and puts the cursor after it.
    pub fn replace(&mut self, task: String) {
        self.prompt = task;
        self.cursor = None;
    }

    pub fn insert(&mut self, c: char) {
        if self.cursor.is_none() {
            self.prompt.push(c);
            return;
        }
        let at = self.at();
        self.prompt.insert(at, c);
        let next = at + c.len_utf8();
        self.cursor = (next < self.prompt.len()).then_some(next);
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        let at = self.at();
        let Some(c) = self.prompt[..at].chars().next_back() else {
            return;
        };
        let from = at - c.len_utf8();
        self.prompt.remove(from);
        if self.cursor.is_some() {
            self.cursor = Some(from);
        }
    }

    /// One character back. A mark that takes no cell of its own is stepped
    /// over with its letter, so the cursor never sits between the two where
    /// nothing on the screen could show it.
    pub fn left(&mut self) {
        let mut at = self.at();
        while let Some(c) = self.prompt[..at].chars().next_back() {
            at -= c.len_utf8();
            if !mark(c) {
                break;
            }
        }
        self.cursor = (at < self.prompt.len()).then_some(at);
    }

    /// One character forward, with any marks that belong to it.
    pub fn right(&mut self) {
        let mut at = self.at();
        if let Some(c) = self.prompt[at..].chars().next() {
            at += c.len_utf8();
            while let Some(c) = self.prompt[at..].chars().next().filter(|c| mark(*c)) {
                at += c.len_utf8();
            }
        }
        self.cursor = (at < self.prompt.len()).then_some(at);
    }

    /// The row of the task the cursor is on, and how far into that row it is,
    /// in bytes.
    pub fn cursor_row(&self) -> (usize, usize) {
        let before = &self.prompt[..self.at()];
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        (before.matches('\n').count(), before.len() - start)
    }

    /// Puts the cursor on the cell a click landed on: the character in that
    /// column of that row, or the end of the row where the click is past it.
    pub fn place(&mut self, row: usize, column: u16) {
        let rows: Vec<&str> = self.prompt.split('\n').collect();
        let row = row.min(rows.len() - 1);
        let start: usize = rows[..row].iter().map(|r| r.len() + 1).sum();
        let line = rows[row];
        let mut cells = 0u16;
        let mut offset = line.len();
        for (i, c) in line.char_indices() {
            if mark(c) {
                continue;
            }
            let width = cells_of(c);
            if column < cells + width {
                offset = i;
                break;
            }
            cells += width;
        }
        let at = start + offset;
        self.cursor = (at < self.prompt.len()).then_some(at);
    }
}

/// How many cells a character takes on the screen, measured the way the
/// screen measures it.
fn cells_of(c: char) -> u16 {
    ratatui::text::Span::raw(c.to_string()).width() as u16
}

/// A character drawn onto the one before it rather than into a cell of its own.
fn mark(c: char) -> bool {
    c != '\n' && cells_of(c) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repository(name: &str) -> Repository {
        Repository {
            name: name.to_string(),
            path: PathBuf::from("/p").join(name),
        }
    }

    #[test]
    fn a_pane_is_named_for_the_repository_it_works_in() {
        assert_eq!(Pane::new(Some(repository("scratch"))).title(), "scratch");
    }

    fn written(task: &str) -> Pane {
        let mut pane = Pane::default();
        for c in task.chars() {
            pane.insert(c);
        }
        pane
    }

    #[test]
    fn typing_lands_where_the_cursor_is() {
        let mut pane = written("hello world");
        for _ in 0.." world".len() {
            pane.left();
        }
        pane.insert(',');
        assert_eq!(pane.prompt, "hello, world");
        pane.backspace();
        pane.backspace();
        assert_eq!(pane.prompt, "hell world");
    }

    #[test]
    fn the_cursor_stops_at_either_end() {
        let mut pane = written("ab");
        for _ in 0..5 {
            pane.left();
        }
        assert_eq!(pane.at(), 0);
        pane.backspace();
        assert_eq!(
            pane.prompt, "ab",
            "backspace at the start deleted something"
        );
        for _ in 0..5 {
            pane.right();
        }
        assert_eq!(pane.cursor, None, "the end is the end, not an offset");
    }

    #[test]
    fn a_task_replaced_whole_puts_the_cursor_after_it() {
        // History and clearing replace the task; a cursor left in the middle
        // of the last one would type into the middle of this one.
        let mut pane = written("a long first task");
        pane.left();
        pane.replace("short".to_string());
        pane.insert('!');
        assert_eq!(pane.prompt, "short!");

        // And a task written straight into the field cannot strand it.
        pane.cursor = Some(40);
        pane.prompt = "tiny".to_string();
        assert_eq!(pane.at(), 4);
    }

    #[test]
    fn a_mark_moves_with_its_letter() {
        // Thai: a consonant with a tone mark above it is one cell. A cursor
        // between the two could not be drawn anywhere.
        let mut pane = written("\u{0e01}\u{0e48}x");
        pane.left();
        pane.left();
        assert_eq!(pane.at(), 0);
        pane.right();
        assert_eq!(pane.at(), "\u{0e01}\u{0e48}".len());
    }

    #[test]
    fn a_click_finds_the_character_in_that_cell() {
        let mut pane = written("first\nsecond row");
        pane.place(1, 3);
        assert_eq!(pane.cursor_row(), (1, 3));
        // Past the end of a row is the end of that row.
        pane.place(0, 60);
        assert_eq!(pane.cursor_row(), (0, 5));
        // Past the last row is the last row.
        pane.place(9, 0);
        assert_eq!(pane.cursor_row(), (1, 0));
        // A wide character takes two cells, and both of them are it.
        let mut wide = written("\u{4f60}\u{597d}x");
        wide.place(0, 3);
        assert_eq!(wide.at(), "\u{4f60}".len());
        wide.place(0, 4);
        assert_eq!(wide.at(), "\u{4f60}\u{597d}".len());
    }

    #[test]
    fn a_cursor_at_the_end_is_always_spelled_as_the_end() {
        // `None` is the end. A `Some` equal to the length would be a second
        // spelling of the same place, and code that tests for `None` would miss
        // it.
        let mut pane = Pane::default();
        pane.left();
        assert_eq!(pane.cursor, None);
        pane.insert('a');
        assert_eq!(pane.cursor, None);
        pane.left();
        pane.insert('b');
        assert_eq!(pane.prompt, "ba");
        assert_eq!(pane.cursor, Some(1));
    }

    #[test]
    fn a_pane_with_nowhere_to_work_yet_says_so_rather_than_being_blank() {
        // A bar of unnamed panes is a bar nobody can navigate.
        assert_eq!(Pane::new(None).title(), "new");
    }
}
