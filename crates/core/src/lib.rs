//! Máquina de estados y modelo de requerimientos de `micli`.
//!
//! Este crate no depende de ningún otro crate del workspace. Las transiciones
//! de estado viven únicamente en [`state::transition`]; el resto del sistema
//! consume esa función y nunca muta estado por su cuenta.

pub mod requirement;
pub mod state;

pub use requirement::{Requirement, RequirementId};
pub use state::{ActiveState, Event, State, TransitionError, transition};
