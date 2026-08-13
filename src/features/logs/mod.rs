pub mod controller;
pub mod model;
pub mod processor;
pub mod view;

pub use controller::{LogAction, LogsController};
pub use model::LogsModel;
pub use view::render;
