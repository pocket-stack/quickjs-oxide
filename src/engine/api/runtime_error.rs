//! Errors exposed by runtime and context operations.

use crate::engine::api::error::Error;
use crate::engine::atom::AtomError;
use crate::engine::heap::HeapError;
use crate::engine::object::property::PropertyDefinitionError;
use crate::engine::object::shape::ShapeError;
use crate::engine::value::JsStringError;
use std::error::Error as StdError;
use std::fmt;

/// Checked failures at the public runtime-domain boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    WrongRuntime(&'static str),
    WrongContext(&'static str),
    /// A module identity escaped a construction or resolution transaction
    /// which later rolled back. Its append-only identity is stable but no
    /// longer executable.
    AbortedModule,
    /// Resolution was observed through host re-entry before construction and
    /// its request table completed, or a host callback failed after QuickJS's
    /// one-shot latch was set. The graph cannot yet be linked safely.
    IncompleteModuleResolution,
    Invariant(&'static str),
    Exception,
    Engine(Error),
    Atom(AtomError),
    Heap(HeapError),
    Shape(ShapeError),
    Property(PropertyDefinitionError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime(kind) => write!(formatter, "{kind} belongs to another runtime"),
            Self::WrongContext(kind) => write!(formatter, "{kind} belongs to another context"),
            Self::AbortedModule => {
                formatter.write_str("module construction or resolution was rolled back")
            }
            Self::IncompleteModuleResolution => {
                formatter.write_str("module resolution is incomplete and cannot be linked safely")
            }
            Self::Invariant(message) => write!(formatter, "runtime invariant failed: {message}"),
            Self::Exception => formatter.write_str("JavaScript exception"),
            Self::Engine(error) => error.fmt(formatter),
            Self::Atom(error) => error.fmt(formatter),
            Self::Heap(error) => error.fmt(formatter),
            Self::Shape(error) => error.fmt(formatter),
            Self::Property(error) => error.fmt(formatter),
        }
    }
}

impl StdError for RuntimeError {}

impl From<Error> for RuntimeError {
    fn from(error: Error) -> Self {
        Self::Engine(error)
    }
}

impl From<JsStringError> for RuntimeError {
    fn from(error: JsStringError) -> Self {
        Self::Engine(Error::from(error))
    }
}

impl From<AtomError> for RuntimeError {
    fn from(error: AtomError) -> Self {
        Self::Atom(error)
    }
}

impl From<HeapError> for RuntimeError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

impl From<ShapeError> for RuntimeError {
    fn from(error: ShapeError) -> Self {
        Self::Shape(error)
    }
}

impl From<PropertyDefinitionError> for RuntimeError {
    fn from(error: PropertyDefinitionError) -> Self {
        Self::Property(error)
    }
}
