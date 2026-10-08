//! Undo history of one frame: the parameters after each edit and where that
//! edit went (the roll or the frame). Undoing a roll-wide edit puts the roll
//! back, whichever scope is selected at the time.

use nv_process::ProcessParams;
use serde::{Deserialize, Serialize};

/// Where inspector edits go: the roll's defaults or the frame's overrides.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    #[default]
    Roll,
    Frame,
}

/// Longest history kept per frame.
const MAX_STEPS: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Step {
    params: ProcessParams,
    scope: Scope,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    steps: Vec<Step>,
    /// The step that is current.
    at: usize,
}

impl History {
    pub fn new(params: ProcessParams, scope: Scope) -> Self {
        Self { steps: vec![Step { params, scope }], at: 0 }
    }

    pub fn current(&self) -> Option<&ProcessParams> {
        self.steps.get(self.at).map(|s| &s.params)
    }

    /// The last `n` steps, for saving: history that outlives the session
    /// need not outgrow `roll.json`.
    pub fn trimmed(&self, n: usize) -> History {
        let n = n.max(1);
        let from = self.steps.len().saturating_sub(n).min(self.at);
        History { steps: self.steps[from..].to_vec(), at: self.at - from }
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Record an edit made in `scope`. Anything that could have been redone is gone.
    pub fn push(&mut self, params: ProcessParams, scope: Scope) {
        if !self.steps.is_empty() {
            self.steps.truncate(self.at + 1);
        }
        self.steps.push(Step { params, scope });
        if self.steps.len() > MAX_STEPS {
            self.steps.remove(0);
        }
        self.at = self.steps.len() - 1;
    }

    /// Make `params` current, as an edit in `scope` unless it already is.
    pub fn ensure_current(&mut self, params: &ProcessParams, scope: Scope) {
        if self.current() != Some(params) {
            self.push(params.clone(), scope);
        }
    }

    /// Step back. Returns the parameters to show and the scope of the edit
    /// that was undone.
    pub fn undo(&mut self) -> Option<(ProcessParams, Scope)> {
        if self.at == 0 || self.steps.is_empty() {
            return None;
        }
        let undone = self.steps[self.at].scope;
        self.at -= 1;
        Some((self.steps[self.at].params.clone(), undone))
    }

    /// Step forward. Returns the parameters to show and the scope of the edit
    /// that was redone.
    pub fn redo(&mut self) -> Option<(ProcessParams, Scope)> {
        if self.at + 1 >= self.steps.len() {
            return None;
        }
        self.at += 1;
        Some((self.steps[self.at].params.clone(), self.steps[self.at].scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nv_process::default_params_for;

    fn p(exposure: f32) -> ProcessParams {
        let mut p = default_params_for(&nv_film::builtin_profiles()[0]);
        p.conversion.exposure = exposure;
        p
    }

    #[test]
    fn undo_and_redo_walk_the_steps_and_report_the_scope() {
        let mut h = History::new(p(0.0), Scope::Roll);
        h.push(p(1.0), Scope::Frame);
        h.push(p(2.0), Scope::Roll);
        assert_eq!(h.undo().map(|(x, s)| (x.conversion.exposure, s)), Some((1.0, Scope::Roll)));
        assert_eq!(h.undo().map(|(x, s)| (x.conversion.exposure, s)), Some((0.0, Scope::Frame)));
        assert!(h.undo().is_none());
        assert_eq!(h.redo().map(|(x, s)| (x.conversion.exposure, s)), Some((1.0, Scope::Frame)));
        assert_eq!(h.redo().map(|(x, s)| (x.conversion.exposure, s)), Some((2.0, Scope::Roll)));
        assert!(h.redo().is_none());
    }

    #[test]
    fn a_new_edit_drops_the_redo_branch() {
        let mut h = History::new(p(0.0), Scope::Roll);
        h.push(p(1.0), Scope::Roll);
        h.undo();
        h.push(p(5.0), Scope::Roll);
        assert!(h.redo().is_none());
        assert_eq!(h.current().unwrap().conversion.exposure, 5.0);
    }

    #[test]
    fn ensure_current_only_pushes_a_change() {
        let mut h = History::new(p(0.0), Scope::Roll);
        h.ensure_current(&p(0.0), Scope::Roll);
        assert!(h.undo().is_none(), "pushed a duplicate");
        h.ensure_current(&p(3.0), Scope::Roll);
        assert_eq!(h.undo().map(|(x, _)| x.conversion.exposure), Some(0.0));
    }

    #[test]
    fn the_history_is_capped() {
        let mut h = History::new(p(0.0), Scope::Roll);
        for i in 1..=MAX_STEPS + 20 {
            h.push(p(i as f32), Scope::Roll);
        }
        let mut n = 0;
        while h.undo().is_some() {
            n += 1;
        }
        assert_eq!(n, MAX_STEPS - 1);
    }

    #[test]
    fn trimming_keeps_the_end_and_the_position() {
        let mut h = History::new(p(0.0), Scope::Roll);
        for i in 1..=10 {
            h.push(p(i as f32), Scope::Roll);
        }
        h.undo();
        h.undo();
        let t = h.trimmed(4);
        assert_eq!(t.current().unwrap().conversion.exposure, 8.0);
        let mut t = t;
        assert_eq!(t.redo().map(|(x, _)| x.conversion.exposure), Some(9.0));
        assert_eq!(t.redo().map(|(x, _)| x.conversion.exposure), Some(10.0));
        assert!(t.redo().is_none());
        assert_eq!(h.trimmed(1000), h);
    }

    #[test]
    fn it_survives_json() {
        let mut h = History::new(p(0.0), Scope::Roll);
        h.push(p(1.5), Scope::Frame);
        let back: History = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back, h);
    }
}
