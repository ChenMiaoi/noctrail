use std::fmt;

pub struct ClipboardBridge {
    system: Option<arboard::Clipboard>,
    fallback: String,
}

impl fmt::Debug for ClipboardBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardBridge")
            .field("system_available", &self.system.is_some())
            .field("fallback_len", &self.fallback.len())
            .finish()
    }
}

impl Default for ClipboardBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardBridge {
    pub fn new() -> Self {
        Self {
            system: arboard::Clipboard::new().ok(),
            fallback: String::new(),
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.fallback = text.clone();

        if let Some(system) = self.system.as_mut() {
            let _ = system.set_text(text);
        }
    }

    pub fn get_text(&mut self) -> Option<String> {
        if let Some(system) = self.system.as_mut()
            && let Ok(text) = system.get_text()
        {
            self.fallback = text.clone();
            return Some(text);
        }

        if self.fallback.is_empty() {
            None
        } else {
            Some(self.fallback.clone())
        }
    }
}
