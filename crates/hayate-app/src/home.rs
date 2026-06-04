//! Home/start screen: shown at launch instead of a deck. Offers "New presentation" (a fresh
//! template deck opened in master-edit mode) and a thumbnailed list of recently-opened files.

use hayate_ir::color::{Color, ThemeColorToken};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::theme::Theme;
use hayate_ir::units::inch_f;
use hayate_model::{edit, History};
use hayate_render::build_slide_scene;
use hayate_render::scene::PxSize;

use crate::{recent, EditScope, HayateApp, LeftTab, RecentThumb};

const NEW_DOC_PATH: &str = "untitled.hayate";

impl HayateApp {
    /// Create a fresh template deck (master with an accent bar + a "Title and Content" layout
    /// carrying Title/Body placeholders + one slide) and open it in master-edit mode, so new
    /// projects start from a template the user can tailor. Leaves the home screen.
    pub(crate) fn new_presentation(&mut self) {
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());

        // Master decoration: a thin accent bar across the top, inherited by every slide.
        let bar = p.add_shape(master);
        p.world
            .frames
            .insert(bar, RectEmu::new(0, 0, p.slide_size.w, inch_f(0.18)));
        p.world.geometries.insert(bar, Geometry::Rect);
        p.world
            .fills
            .insert(bar, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));

        let layout = p.add_layout(master, "Title and Content");
        let slide = p.add_slide(layout);

        self.pres = p;
        self.slide = slide;
        self.history = History::new();
        self.selection = None;
        self.also.clear();
        self.doc_path = NEW_DOC_PATH.to_string();

        // Populate the layout's Title/Body placeholders via the shared preset path.
        self.fill_layout_preset(layout, edit::LayoutPreset::TitleAndContent);

        self.home = false;
        self.left_tab = LeftTab::Master;
        self.enter_layout_scope(layout);
    }

    /// Open a presentation from the recents list (or any path). Returns to slide editing.
    pub(crate) fn open_recent(&mut self, path: String) {
        match hayate_format::load(&path) {
            Ok(p) => {
                self.pres = p;
                self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
                self.history = History::new();
                self.selection = None;
                self.also.clear();
                self.home = false;
                self.left_tab = LeftTab::Slides;
                self.scope = EditScope::Slide(self.slide);
                self.rebuild();
                recent::add(&path);
                self.doc_path = path;
            }
            Err(e) => {
                eprintln!("open error: {e}");
                self.notice = Some(format!("ファイルを開けませんでした\n{path}\n{e}"));
            }
        }
    }

    /// Return to the home screen, forcing the recents list to refresh on next render.
    pub(crate) fn go_home(&mut self) {
        self.home = true;
        self.home_loaded = false;
    }

    /// Build the recent-presentation thumbnails (loads each file and renders its first slide).
    /// Best-effort: unreadable files are skipped.
    pub(crate) fn load_home_recents(&mut self) {
        let mut thumbs = Vec::new();
        for path in recent::load() {
            let Ok(p) = hayate_format::load(&path) else {
                continue;
            };
            let Some(slide) = p.slides().first().copied() else {
                continue;
            };
            let scene = build_slide_scene(&p, slide, PxSize { w: 240.0, h: 135.0 });
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&path)
                .to_string();
            thumbs.push(RecentThumb {
                path,
                name,
                scene,
                media: p.media,
            });
        }
        self.home_recents = thumbs;
        self.home_loaded = true;
    }
}
