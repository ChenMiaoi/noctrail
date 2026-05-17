//! Render plan and backend boundary for Noctrail.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    Gpu,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderPlan;

impl RenderPlan {
    pub const fn new() -> Self {
        Self
    }
}

#[derive(Debug, Default)]
pub struct RenderSurface;
