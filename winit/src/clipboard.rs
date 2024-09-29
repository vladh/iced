#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

//! Access the clipboard.

use crate::core::clipboard::Kind;
use std::sync::Arc;
use winit::window::{Window, WindowId};

/// A buffer for short-term storage and transfer within and between
/// applications.
#[allow(missing_debug_implementations)]
pub struct Clipboard {
    state: State,
}

enum State {
    Unavailable,
}

impl Clipboard {
    /// Creates a new [`Clipboard`] for the given window.
    pub fn connect(window: Arc<Window>) -> Clipboard {
        Clipboard {
            state: State::Unavailable,
        }
    }

    /// Creates a new [`Clipboard`] that isn't associated with a window.
    /// This clipboard will never contain a copied value.
    pub fn unconnected() -> Clipboard {
        Clipboard {
            state: State::Unavailable,
        }
    }

    /// Reads the current content of the [`Clipboard`] as text.
    pub fn read(&self, kind: Kind) -> Option<String> {
        None
    }

    /// Writes the given text contents to the [`Clipboard`].
    pub fn write(&mut self, kind: Kind, contents: String) {
    }

    /// Returns the identifier of the window used to create the [`Clipboard`], if any.
    pub fn window_id(&self) -> Option<WindowId> {
        None
    }
}

impl crate::core::Clipboard for Clipboard {
    fn read(&self, kind: Kind) -> Option<String> {
        self.read(kind)
    }

    fn write(&mut self, kind: Kind, contents: String) {
        self.write(kind, contents);
    }
}
