//! Configuration boundary for Noctrail.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config;

impl Config {
    pub const fn new() -> Self {
        Self
    }
}
