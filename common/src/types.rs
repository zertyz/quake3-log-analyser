//! Contains some types used for domain specific entities

use std::result;


pub type Result<T> = result::Result<T, Box<dyn std::error::Error>>;
