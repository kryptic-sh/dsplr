//! The backend thread's recent messages, and the window's notes about the
//! desktop's changes (edits dropped with an output that went), which the bar
//! shows one at a time: kept so the window can show them all again
//! (`docs/plan/06-gui.md`).

use std::collections::VecDeque;

/// How many lines are kept: the oldest go first.
pub const MESSAGE_HISTORY: usize = 12;

/// The last [`MESSAGE_HISTORY`] lines of the backend thread's messages,
/// oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Messages {
    lines: VecDeque<String>,
}

impl Messages {
    /// Keeps `message`, a line each of its lines that has anything on it.
    pub fn push(&mut self, message: &str) {
        for line in message
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
        {
            self.lines.push_back(line.to_owned());
        }
        while self.lines.len() > MESSAGE_HISTORY {
            self.lines.pop_front();
        }
    }

    /// The lines kept, oldest first.
    pub fn lines(&self) -> impl ExactSizeIterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::{MESSAGE_HISTORY, Messages};

    #[test]
    fn each_line_is_kept_and_only_the_last_few() {
        let mut messages = Messages::default();
        messages.push("applied\nread back: nothing differs\n\n");
        assert_eq!(
            messages.lines().collect::<Vec<_>>(),
            ["applied", "read back: nothing differs"]
        );
        for n in 0..MESSAGE_HISTORY {
            messages.push(&format!("line {n}"));
        }
        assert_eq!(messages.lines().len(), MESSAGE_HISTORY);
        assert_eq!(messages.lines().next(), Some("line 0"), "the oldest went");
        assert_eq!(
            messages.lines().last(),
            Some(format!("line {}", MESSAGE_HISTORY - 1).as_str())
        );
    }
}
