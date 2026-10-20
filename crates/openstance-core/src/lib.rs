//! openstance core (bootstrapping; replaced below).
#![warn(missing_docs)]
pub mod clock;
pub mod config;
pub mod db;
pub mod error;
pub mod json;
pub use error::{Error, Result};
