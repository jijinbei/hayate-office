//! File I/O actions: PPTX import/export, SVG export, and native save/open.

use hayate_model::History;

use crate::HayateApp;

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

    /// Save to the current document path.
    pub(crate) fn save(&self) {
        let path = self.doc_path.clone();
        match hayate_format::save(&self.pres, &path) {
            Ok(()) => eprintln!("saved to {path}"),
            Err(e) => eprintln!("save error: {e}"),
        }
    }

    /// Export every slide to a multi-page PDF next to the document (raster pages at ~2x so text
    /// stays crisp). The output file is the document path with a `.pdf` extension.
    pub(crate) fn export_pdf(&self) {
        let bytes = hayate_render::export_pdf(&self.pres, 2.0);
        let path = pdf_path(&self.doc_path);
        match std::fs::write(&path, bytes) {
            Ok(()) => eprintln!("exported {path}"),
            Err(e) => eprintln!("pdf export error: {e}"),
        }
    }

    pub(crate) fn open(&mut self) {
        let path = self.doc_path.clone();
        match hayate_format::load(&path) {
            Ok(p) => {
                self.pres = p;
                self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
                self.history = History::new();
                self.selection = None;
                self.rebuild();
                eprintln!("opened {path}");
            }
            Err(e) => eprintln!("open error: {e}"),
        }
    }
}

/// Replace (or append) a `.pdf` extension on the document path.
fn pdf_path(doc: &str) -> String {
    match doc.rsplit_once('.') {
        Some((stem, _ext)) => format!("{stem}.pdf"),
        None => format!("{doc}.pdf"),
    }
}
