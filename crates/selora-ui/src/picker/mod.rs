//! Dropdown-style pickers (the chat sidebar's model picker, UI_SPEC §4.1).

#![forbid(unsafe_code)]

pub mod model_picker;

pub use model_picker::{ModelPicker, ModelPickerEvent, PickerGeometry};
