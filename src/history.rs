//! Undo/redo history for a project's editable document state.
//!
//! State is captured as a [`ProjectSnapshot`] (see `snapshot.rs`). A baseline
//! snapshot tracks the last committed state; when the document differs from it
//! (checked once the user settles, not mid-drag) the baseline is pushed onto the
//! undo stack. Selection/active-image changes are part of a snapshot but are not
//! treated as document changes, so merely clicking around doesn't fill history.

use crate::snapshot::ProjectSnapshot;

/// How many undo steps to retain.
const MAX_UNDO: usize = 100;

#[derive(Default)]
pub struct History {
    /// The current committed state (what new changes are diffed against).
    baseline: Option<ProjectSnapshot>,
    undo: Vec<ProjectSnapshot>,
    redo: Vec<ProjectSnapshot>,
}

impl History {
    /// Drops all history and adopts `current` as the baseline.
    pub fn reset(&mut self, current: ProjectSnapshot) {
        self.baseline = Some(current);
        self.undo.clear();
        self.redo.clear();
    }

    /// Records an undo step if `current`'s document differs from the baseline.
    /// Returns `true` if a change was committed.
    pub fn commit(&mut self, current: ProjectSnapshot) -> bool {
        let changed = match &self.baseline {
            Some(b) => !b.same_document(&current),
            None => true,
        };
        if !changed {
            return false;
        }
        if let Some(b) = self.baseline.take() {
            self.undo.push(b);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.baseline = Some(current);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Steps back one state; returns the snapshot to restore, if any.
    pub fn undo(&mut self) -> Option<ProjectSnapshot> {
        let prev = self.undo.pop()?;
        if let Some(b) = self.baseline.take() {
            self.redo.push(b);
        }
        self.baseline = Some(prev.clone());
        Some(prev)
    }

    /// Steps forward one state; returns the snapshot to restore, if any.
    pub fn redo(&mut self) -> Option<ProjectSnapshot> {
        let next = self.redo.pop()?;
        if let Some(b) = self.baseline.take() {
            self.undo.push(b);
        }
        self.baseline = Some(next.clone());
        Some(next)
    }
}
