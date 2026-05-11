//! Document-adapter-facing types shared across DOCX, PPTX, and future
//! format adapters.
//!
//! These types are deliberately format-agnostic: a "run" is whatever
//! contiguous text-with-formatting unit an adapter chooses to emit. The
//! [`RunRef`] carries enough location data to feed an audit log, and the
//! [`RunVisitor`] decides per-run whether the text and font should be
//! rewritten.

/// A single text run handed to a [`RunVisitor`].
///
/// Adapter-specific identifiers are exposed as optional fields so the same
/// type works for OOXML formats with different nesting depths (DOCX has
/// paragraphs and runs; PPTX adds a slide layer).
pub struct RunRef<'a> {
    pub paragraph_index: usize,
    pub run_index: usize,
    pub slide_index: Option<usize>,
    pub font_name: Option<&'a str>,
    pub text: &'a str,
}

/// Decision returned by a [`RunVisitor`].
pub enum RunAction {
    /// Leave the run's text and font unchanged.
    Keep,
    /// Rewrite the run's text and (optionally) its font.
    Replace {
        new_text: String,
        new_font: Option<String>,
    },
}

/// A per-run callback. Adapters call [`RunVisitor::visit`] once for each
/// extracted run and apply the returned [`RunAction`].
pub trait RunVisitor {
    fn visit(&mut self, run: RunRef<'_>) -> RunAction;
}

impl<F: FnMut(RunRef<'_>) -> RunAction> RunVisitor for F {
    fn visit(&mut self, run: RunRef<'_>) -> RunAction {
        (self)(run)
    }
}

use crate::policy::{convert_run, ConvertOptions};

/// The default classify-then-transliterate visitor used by every host that
/// has no audit/explain plumbing of its own (WASM bindings, future
/// LibreOffice / Apache OpenOffice connector, anyone embedding the adapter
/// crates directly).
///
/// The CLI keeps its own visitor in `banglakit-cli/src/main.rs` because that
/// one also writes per-run JSONL audit entries and `--explain` output to
/// stderr; both are concerns that have no place in `banglakit-core`.
pub struct DefaultRunVisitor<'a> {
    pub opts: ConvertOptions<'a>,
    pub any_change: bool,
    pub runs_converted: usize,
}

impl<'a> DefaultRunVisitor<'a> {
    pub fn new(opts: ConvertOptions<'a>) -> Self {
        Self { opts, any_change: false, runs_converted: 0 }
    }
}

impl<'a> RunVisitor for DefaultRunVisitor<'a> {
    fn visit(&mut self, run: RunRef<'_>) -> RunAction {
        let r = convert_run(run.text, run.font_name, &self.opts);
        if r.changed {
            self.any_change = true;
            self.runs_converted += 1;
            RunAction::Replace {
                new_text: r.text,
                new_font: r.font.map(str::to_string),
            }
        } else {
            RunAction::Keep
        }
    }
}
