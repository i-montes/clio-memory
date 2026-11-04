//! The agent layer: the places of the shared file, project detection and what an agent may do.

pub mod memory;
pub mod project;

pub use memory::{Actors, Draft, EmbedModel, Memory, Place, default_data};
