//! File I/O actions: PPTX import/export, SVG export, and native save/open.

use hayate_model::History;

use crate::{HayateApp, DOC_PATH};

impl HayateApp {
    pub(crate) fn export_pptx(&self) {
        match hayate_format_pptx::export_pptx(&self.pres, "hayate-deck.pptx") {
            Ok(()) => eprintln!("exported hayate-deck.pptx"),
            Err(e) => eprintln!("pptx export error: {e}"),
        }
    }

    pub(crate) fn import_pptx(&mut self) {
        match hayate_format_pptx::import_pptx("hayate-deck.pptx") {
            Ok(p) => {
                self.pres = p;
                self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
                self.history = History::new();
                self.selection = None;
                self.also.clear();
                self.rebuild();
                eprintln!("imported hayate-deck.pptx");
            }
            Err(e) => eprintln!("pptx import error: {e}"),
        }
    }

    /// Export the current slide to an SVG file next to the app.
    pub(crate) fn export_svg(&self) {
        let svg = hayate_render::export_svg(&self.scene);
        match std::fs::write("hayate-slide.svg", svg) {
            Ok(()) => eprintln!("exported hayate-slide.svg"),
            Err(e) => eprintln!("svg export error: {e}"),
        }
    }

    pub(crate) fn save(&self) {
        match hayate_format::save(&self.pres, DOC_PATH) {
            Ok(()) => eprintln!("saved to {DOC_PATH}"),
            Err(e) => eprintln!("save error: {e}"),
        }
    }

    pub(crate) fn open(&mut self) {
        match hayate_format::load(DOC_PATH) {
            Ok(p) => {
                self.pres = p;
                self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
                self.history = History::new();
                self.selection = None;
                self.rebuild();
                eprintln!("opened {DOC_PATH}");
            }
            Err(e) => eprintln!("open error: {e}"),
        }
    }
}
