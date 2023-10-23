//! Contains some types used for domain specific entities

use std::result;


/// Our `Result` type, with a standardized error, for brevity
pub type Result<T> = result::Result<T, Box<dyn std::error::Error>>;
