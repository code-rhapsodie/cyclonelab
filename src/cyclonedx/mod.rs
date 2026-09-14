//! CycloneDX 1.7 data model.
//!
//! The model does not aim to cover 100% of the schema from the start: each
//! struct explicitly types the fields this crate's commands need, and
//! captures the rest in an `extra` field (`#[serde(flatten)]`). This allows
//! loading an SBOM produced by another tool, modifying a small part of it,
//! and rewriting it without losing any information — including fields
//! added by future versions of the CycloneDX schema.
//!
//! To add a new piece of the model (e.g. `dependencies`,
//! `vulnerabilities`...), create a dedicated file in this module following
//! the same principle.

mod component;
mod hash;
mod license;
pub mod validation;

pub use component::Component;
pub use license::LicenseChoice;
