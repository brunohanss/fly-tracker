#![forbid(unsafe_code)]

pub mod app;
pub mod event;
pub mod layout;
mod terminal;
pub mod theme;
pub mod viewport;
pub mod views;

pub use terminal::run;
pub use terminal::run_connected;
