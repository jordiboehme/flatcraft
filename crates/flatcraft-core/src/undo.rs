//! Snapshot-based undo stack.

/// Default number of undo steps kept.
pub const DEFAULT_LIMIT: usize = 100;

/// Undo and redo over whole-value snapshots.
#[derive(Debug, Clone)]
pub struct UndoStack<T: Clone> {
    undo: Vec<T>,
    redo: Vec<T>,
    limit: usize,
}

impl<T: Clone> UndoStack<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    /// Records the state before a change. Clears redo and drops the oldest
    /// entry beyond the limit.
    pub fn record(&mut self, before: T) {
        self.redo.clear();
        self.undo.push(before);
        if self.undo.len() > self.limit {
            let excess = self.undo.len() - self.limit;
            self.undo.drain(..excess);
        }
    }

    /// Returns the previous state; `current` becomes available for redo.
    pub fn undo(&mut self, current: T) -> Option<T> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        Some(previous)
    }

    /// Returns the next state; `current` becomes available for undo.
    pub fn redo(&mut self, current: T) -> Option<T> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Number of steps that can be undone.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
}

impl<T: Clone> Default for UndoStack<T> {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}
