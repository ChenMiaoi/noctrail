//! Pane runtime registry boundary for Noctrail.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub u64);

#[derive(Debug, Default)]
pub struct PaneRuntime;

#[derive(Debug, Default)]
pub struct PaneRuntimeRegistry {
    panes: HashMap<PaneId, PaneRuntime>,
}

impl PaneRuntimeRegistry {
    pub fn new() -> Self {
        Self {
            panes: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }
}
