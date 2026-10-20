// SPDX-License-Identifier: GPL-2.0-or-later

use gilrs::{Button, EventType, Gilrs};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Previous,
    Next,
    Left,
    Right,
    Activate,
    Back,
}

/// gilrs observes device arrival/removal continuously; no restart is needed.
pub struct Controllers {
    backend: Option<Gilrs>,
    pub error: Option<String>,
}

impl Controllers {
    pub fn new() -> Self {
        match Gilrs::new() {
            Ok(backend) => Self {
                backend: Some(backend),
                error: None,
            },
            Err(error) => Self {
                backend: None,
                error: Some(error.to_string()),
            },
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.backend.as_ref().map_or_else(Vec::new, |backend| {
            backend
                .gamepads()
                .filter(|(_, pad)| pad.is_connected())
                .map(|(_, pad)| pad.name().to_owned())
                .collect()
        })
    }

    pub fn poll(&mut self, enabled: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(backend) = &mut self.backend {
            while let Some(event) = backend.next_event() {
                if enabled && let EventType::ButtonPressed(button, _) = event.event {
                    let action = match button {
                        Button::DPadUp => Some(Action::Previous),
                        Button::DPadDown => Some(Action::Next),
                        Button::DPadLeft => Some(Action::Left),
                        Button::DPadRight => Some(Action::Right),
                        Button::South => Some(Action::Activate),
                        Button::East => Some(Action::Back),
                        _ => None,
                    };
                    actions.extend(action);
                }
            }
        }
        actions
    }
}
