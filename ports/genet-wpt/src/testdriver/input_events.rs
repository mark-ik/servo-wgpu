/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use bitflags::bitflags;
use keyboard_types::{CompositionEvent, Key, KeyState};
use serde::{Deserialize, Serialize};

use super::WebViewPoint;

bitflags! {
    /// Flags representing the state of an [`InputEvent`] after the engine has handled it.
    #[derive(Clone, Copy, Default, Deserialize, PartialEq, Serialize)]
    pub struct InputEventResult: u8 {
        /// Whether or not this input event's default behavior was prevented via script.
        const DefaultPrevented = 1 << 0;
        /// Whether or not the WebView handled this event. Some events have default handlers in
        /// Servo, such as keyboard events that insert characters in `<input>` areas. When these
        /// handlers are triggered, this flag is included. This can be used to prevent triggering
        /// behavior (such as keybindings) when the WebView has already consumed the event for its
        /// own purpose.
        const Consumed = 1 << 1;
        /// Whether or not the input event failed to dispatch. This can happen when an event
        /// is sent while Servo is shutting down or when it is in an intermediate state.
        /// Typically these events should be considered to be consumed.
        const DispatchFailed = 1 << 2;
    }
}

/// An input event the testdriver interpreter emits for one tick.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum InputEvent {
    EditingAction(EditingActionEvent),
    Ime(ImeEvent),
    Keyboard(KeyboardEvent),
    MouseButton(MouseButtonEvent),
    MouseLeftViewport(MouseLeftViewportEvent),
    MouseMove(MouseMoveEvent),
    Touch(TouchEvent),
    Wheel(WheelEvent),
}

/// An editing action that should be performed on a `WebView`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum EditingActionEvent {
    Copy,
    Cut,
    Paste,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct KeyboardEvent {
    pub event: ::keyboard_types::KeyboardEvent,
}

impl KeyboardEvent {
    pub fn new(keyboard_event: ::keyboard_types::KeyboardEvent) -> Self {
        Self {
            event: keyboard_event,
        }
    }

    pub fn from_state_and_key(state: KeyState, key: Key) -> Self {
        Self::new(::keyboard_types::KeyboardEvent {
            state,
            key,
            ..::keyboard_types::KeyboardEvent::default()
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct MouseButtonEvent {
    pub action: MouseButtonAction,
    pub button: MouseButton,
    pub point: WebViewPoint,
}

impl MouseButtonEvent {
    pub fn new(action: MouseButtonAction, button: MouseButton, point: WebViewPoint) -> Self {
        Self {
            action,
            button,
            point,
        }
    }
}

/// The types of mouse buttons.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
    Other(u16),
}

impl<T: Into<u64>> From<T> for MouseButton {
    fn from(value: T) -> Self {
        let value = value.into();
        match value {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            3 => MouseButton::Back,
            4 => MouseButton::Forward,
            _ => MouseButton::Other(value as u16),
        }
    }
}

impl From<MouseButton> for i16 {
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::Back => 3,
            MouseButton::Forward => 4,
            MouseButton::Other(value) => value as i16,
        }
    }
}

/// The types of mouse events.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum MouseButtonAction {
    /// Mouse button down.
    Down,
    /// Mouse button up.
    Up,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct MouseMoveEvent {
    pub point: WebViewPoint,
    #[doc(hidden)]
    // An internal flag used to avoid refreshing the cursor in response to move
    // events for touch devices since they are simulated in Servo using mouse events.
    pub is_compatibility_event_for_touch: bool,
}

impl MouseMoveEvent {
    pub fn new(point: WebViewPoint) -> Self {
        Self {
            point,
            is_compatibility_event_for_touch: false,
        }
    }

}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct MouseLeftViewportEvent {
    pub focus_moving_to_another_iframe: bool,
}

/// The type of input represented by a multi-touch event.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum TouchEventType {
    /// A new touch point came in contact with the screen.
    Down,
    /// An existing touch point changed location.
    Move,
    /// A touch point was removed from the screen.
    Up,
    /// The system stopped tracking a touch point.
    Cancel,
}

/// An opaque identifier for a touch point.
///
/// <http://w3c.github.io/touch-events/#widl-Touch-identifier>
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct TouchId(pub i32);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct TouchEvent {
    pub event_type: TouchEventType,
    pub touch_id: TouchId,
    pub point: WebViewPoint,
    /// cancelable default value is true, once the first move has been processed by script disable it.
    cancelable: bool,
}

impl TouchEvent {
    pub fn new(event_type: TouchEventType, touch_id: TouchId, point: WebViewPoint) -> Self {
        TouchEvent {
            event_type,
            touch_id,
            point,
            cancelable: true,
        }
    }

}

/// Unit of a [`WheelDelta`].
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum WheelMode {
    /// Delta values are specified in pixels.
    DeltaPixel = 0x00,
    /// Delta values are specified in lines.
    DeltaLine = 0x01,
    /// Delta values are specified in pages.
    DeltaPage = 0x02,
}

/// The wheel event deltas for every direction.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct WheelDelta {
    /// Delta in the left/right direction. A positive value means that the view scrolls left,
    /// revealing more content to the left of the current viewport.
    pub x: f64,
    /// Delta in the up/down direction. A positive value means that the view scrolls up, revealing
    /// more content above the current viewport.
    pub y: f64,
    /// Delta in the direction going into/out of the screen
    pub z: f64,
    /// Mode to measure the floats in
    pub mode: WheelMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WheelEvent {
    pub delta: WheelDelta,
    pub point: WebViewPoint,
}

impl WheelEvent {
    pub fn new(delta: WheelDelta, point: WebViewPoint) -> Self {
        WheelEvent { delta, point }
    }
}

/// The types of an input method event.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ImeEvent {
    Composition(CompositionEvent),
    Dismissed,
}
