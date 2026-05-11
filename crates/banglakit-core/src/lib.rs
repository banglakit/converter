//! # banglakit-core
//!
//! Pure-function ANSI Bengali → Unicode transliterator and classifier.
//!
//! The crate is encoding-family-aware via the [`Encoding`] enum; v0.1.0
//! implements the Bijoy/SutonnyMJ family. Adding Boishakhi or Lekhoni later
//! is a matter of dropping a new mapping TOML file under `data/<family>/`
//! and registering a new [`Encoding`] variant — no architectural changes.
//!
//! ## Public surface
//!
//! - [`transliterate`] — convert an ANSI Bengali string to Unicode Bengali.
//! - [`classify`] — score a string for ANSI encoding likelihood.
//! - [`Classification`], [`Decision`], [`Mode`] — types returned and consumed
//!   by the API.
//! - [`Encoding`], [`registry`] — encoding-family enum and its data registry.

pub mod classifier;
pub mod encoding;
pub mod fonts;
pub mod visitor;
mod english;
mod mapping;
mod normalize;
mod transliterate;

pub use classifier::{classify, Classification, Decision, Mode, Signal, Stage};
pub use encoding::{registry, Encoding, EncodingRegistry};
pub use transliterate::{transliterate, transliterate_with_audit, SpanMap, SpanMapping};
pub use visitor::{RunAction, RunRef, RunVisitor};
