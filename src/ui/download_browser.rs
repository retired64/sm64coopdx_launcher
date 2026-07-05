use std::collections::{HashMap, HashSet};

use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::ttf::Font;
use sdl2::video::Window;

use crate::managers::download_manager::{
    DbFile, SearchIndex, autocomplete_authors, filter_mods_combined, top_tags,
};
use crate::ui::common::{
    ACCENT_W, DL_PAGE_SIZE, DL_VISIBLE_ROWS, ROW_H, ROW_STEP, SCROLLBAR_PAD, SCROLLBAR_W,
};

use crate::managers::download_manager::DownloadProgress;

const HIGHLIGHT_COLOR: Color = Color::RGBA(80, 35, 180, 70);
const ACCENT_COLOR: Color = Color::RGB(160, 100, 240);
const TITLE_NORM: Color = Color::RGB(185, 175, 210);
const AUTHOR_COLOR: Color = Color::RGB(140, 130, 180);
const STATS_COLOR: Color = Color::RGB(120, 160, 220);
const SCROLL_TRACK: Color = Color::RGBA(30, 20, 60, 100);
const SCROLL_THUMB: Color = Color::RGBA(140, 100, 220, 150);

/// Per‑row cached textures for the download browser.
///
/// Step 30: title textures are pre‑rendered in both selected (white) and
/// unselected (TITLE_NORM) variants so `draw_download_browser` never calls
/// `set_color_mod` per‑frame.
#[derive(Default)]
struct RenderedRow {
    title_tex: Option<sdl2::render::Texture>,
    title_sel_tex: Option<sdl2::render::Texture>,
    title_w: u32,
    title_h: u32,
    author_tex: Option<sdl2::render::Texture>,
    author_w: u32,
    author_h: u32,
    stats_tex: Option<sdl2::render::Texture>,
    stats_w: u32,
    stats_h: u32,
}

/// Which part of the download browser has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlFocusMode {
    /// UP/DOWN navigate the mod list
    ModList,
    /// LEFT/RIGHT navigate tag chips
    TagChips,
    /// Author dropdown / autocomplete
    AuthorInput,
}

/// Full state for the download browser sub‑screen.
pub struct DownloadBrowserState {
    pub db: Option<DbFile>,
    pub index: Option<SearchIndex>,
    pub sorted: Vec<usize>,
    pub search_text: String,
    pub search_active: bool,
    pub filtered: Option<Vec<usize>>,
    pub selected: usize,
    pub scroll: usize,
    pub page: usize,
    row_cache: HashMap<usize, RenderedRow>,

    // ── Step 21: tags + author filters ──
    pub active_tags: Vec<String>,
    pub active_author: Option<String>,
    pub focus_mode: DlFocusMode,
    pub tag_chip_selected: usize,
    pub top_tags: Vec<(String, usize)>,
    pub author_dropdown_open: bool,
    pub author_dropdown_text: String,
    pub author_filtered: Vec<String>,
    pub author_list_selected: usize,

    /// Composite cache key: (query, tags, author).
    filter_key: Option<(String, Vec<String>, Option<String>)>,

    // ── Installed detection (step 24) ──
    pub installed_ids: HashSet<String>,

    // ── Perf: texture caches ──
    /// Maps "prefix:text" → (texture, w, h). Prefixes: "chip:", "author_btn:",
    /// "dd:" (dropdown), "clear_all:", "arrow:", "cross:".
    tex_cache: HashMap<String, (sdl2::render::Texture, u32, u32)>,
    /// Last author button label — invalidate when author selection changes.
    cached_author_label: String,
    /// Last clear-all button text — invalidate when filter combo changes.
    cached_clear_all_key: String,
}

impl DownloadBrowserState {
    pub fn new() -> Self {
        Self {
            db: None,
            index: None,
            sorted: Vec::new(),
            search_text: String::new(),
            search_active: false,
            filtered: None,
            selected: 0,
            scroll: 0,
            page: 0,
            row_cache: HashMap::new(),
            active_tags: Vec::new(),
            active_author: None,
            focus_mode: DlFocusMode::ModList,
            tag_chip_selected: 0,
            top_tags: Vec::new(),
            author_dropdown_open: false,
            author_dropdown_text: String::new(),
            author_filtered: Vec::new(),
            author_list_selected: 0,
            filter_key: None,
            tex_cache: HashMap::new(),
            cached_author_label: String::new(),
            cached_clear_all_key: String::new(),
            installed_ids: HashSet::new(),
        }
    }

    /// Called when the DB is first loaded (lazy, on first DownloadBrowser open).
    /// Builds sorted list (not‑installed first, then rating desc) and cache top 9 tags.
    pub fn init_index(&mut self) {
        if let Some(ref idx) = self.index {
            // Build sorted indices: not‑installed first, then rating descending
            let mut pairs: Vec<(usize, f64, bool)> = idx
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let installed = self.installed_ids.contains(&e.mod_id);
                    (i, e.rating, installed)
                })
                .collect();
            pairs.sort_by(|a, b| {
                b.2.cmp(&a.2) // installed first (true > false)
                    .then_with(|| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            });
            self.sorted = pairs.into_iter().map(|(i, _, _)| i).collect();

            // Cache top 9 tags (computed once per session)
            self.top_tags = top_tags(idx, 9);
        }
    }

    pub fn total(&self) -> usize {
        self.filtered
            .as_ref()
            .map_or(self.sorted.len(), |f| f.len())
    }

    pub fn entry_idx(&self, list_idx: usize) -> Option<usize> {
        if let Some(ref f) = self.filtered {
            f.get(list_idx).copied()
        } else {
            self.sorted.get(list_idx).copied()
        }
    }

    /// Reset selection + scroll + page, called whenever the filter changes.
    fn reset_position(&mut self) {
        self.selected = 0;
        self.scroll = 0;
        self.page = 0;
    }

    /// Total number of pages given current list size.
    pub fn total_pages(&self) -> usize {
        let total = self.total();
        if total == 0 {
            return 1;
        }
        (total).div_ceil(DL_PAGE_SIZE)
    }

    /// Current page (clamped). 0-indexed.
    pub fn current_page(&self) -> usize {
        self.page.min(self.total_pages().saturating_sub(1))
    }

    /// Jump to a specific page (clamped). Updates scroll to start of page.
    pub fn jump_to_page(&mut self, page: usize) {
        let tp = self.total_pages();
        self.page = page.min(tp.saturating_sub(1));
        self.selected = self.page * DL_PAGE_SIZE;
        let total = self.total();
        if self.selected >= total {
            self.selected = total.saturating_sub(1);
        }
        self.scroll = self.selected;
    }

    /// Navigate page: delta = -1 or +1.
    pub fn change_page(&mut self, delta: isize) {
        let cur = self.current_page() as isize;
        let new = (cur + delta).max(0);
        self.jump_to_page(new as usize);
    }

    /// Composite filter: recomputes `filtered` only when (query, tags, author)
    /// actually changed. Returns true if filtered changed.
    pub fn update_filter(&mut self) -> bool {
        let tags_clone = self.active_tags.clone();
        let author_clone = self.active_author.clone();
        let new_key = (self.search_text.clone(), tags_clone, author_clone);

        if self.filter_key.as_ref() == Some(&new_key) {
            return false;
        }

        // Compute
        let has_text = !self.search_text.is_empty();
        let has_tags = !self.active_tags.is_empty();
        let has_author = self.active_author.is_some();

        let new_filtered = if has_text || has_tags || has_author {
            self.index.as_ref().map(|idx| {
                filter_mods_combined(
                    idx,
                    &self.search_text,
                    &self.active_tags,
                    self.active_author.as_deref(),
                )
            })
        } else {
            // No filters active → show full list
            None
        };

        self.filtered = new_filtered;
        self.filter_key = Some(new_key);
        self.reset_position();
        true
    }

    /// Clear all filters (text, tags, author) in one action.
    pub fn clear_all_filters(&mut self) {
        self.search_text.clear();
        self.search_active = false;
        self.active_tags.clear();
        self.active_author = None;
        self.tag_chip_selected = 0;
        self.author_dropdown_open = false;
        self.author_dropdown_text.clear();
        self.author_filtered.clear();
        self.author_list_selected = 0;
        self.focus_mode = DlFocusMode::ModList;
        self.filtered = None;
        self.filter_key = None;
        self.reset_position();
        // Invalidate cached author button + clear‑all labels
        self.cached_author_label.clear();
        self.cached_clear_all_key.clear();
    }

    /// Toggle a tag chip on/off.
    pub fn toggle_tag(&mut self, tag: &str) {
        let lower = tag.to_lowercase();
        if let Some(pos) = self
            .active_tags
            .iter()
            .position(|t| t.to_lowercase() == lower)
        {
            self.active_tags.remove(pos);
        } else {
            self.active_tags.push(tag.to_string());
        }
        self.cached_clear_all_key.clear();
    }

    /// Is a given tag currently active?
    pub fn is_tag_active(&self, tag: &str) -> bool {
        let lower = tag.to_lowercase();
        self.active_tags.iter().any(|t| t.to_lowercase() == lower)
    }

    /// Open/close the author dropdown, refresh the autocomplete list.
    pub fn toggle_author_dropdown(&mut self) {
        self.author_dropdown_open = !self.author_dropdown_open;
        if self.author_dropdown_open {
            self.refresh_author_autocomplete();
        }
    }

    /// Update author autocomplete list based on current dropdown text.
    pub fn refresh_author_autocomplete(&mut self) {
        if let Some(ref idx) = self.index {
            if self.author_dropdown_text.is_empty() {
                self.author_filtered = idx.authors_sorted.clone();
            } else {
                self.author_filtered = autocomplete_authors(idx, &self.author_dropdown_text);
            }
            self.author_list_selected = 0;
        }
    }

    /// Select an author from the dropdown. `None` clears the author filter.
    pub fn select_author(&mut self, author: Option<String>) {
        self.active_author = author;
        self.author_dropdown_open = false;
        self.author_dropdown_text.clear();
        self.author_filtered.clear();
        self.author_list_selected = 0;
        // Invalidate author button label texture
        self.cached_author_label.clear();
        self.cached_clear_all_key.clear();
    }
}

const SEARCH_BAR_H: u32 = 32;
const TAG_ROW_H: u32 = 28;
const AUTHOR_ROW_H: u32 = 28;
const FILTERS_GAP: i32 = 2;

const SEARCH_BAR_COLOR: Color = Color::RGBA(40, 30, 70, 200);
const SEARCH_TEXT_COLOR: Color = Color::RGB(200, 200, 220);
const SEARCH_PLACEHOLDER: Color = Color::RGB(100, 90, 130);

const CHIP_BG_INACT: Color = Color::RGBA(50, 40, 80, 150);
const CHIP_BG_ACT: Color = Color::RGBA(120, 70, 200, 220);
const CHIP_TEXT: Color = Color::RGB(200, 195, 230);
const CHIP_COUNT: Color = Color::RGB(150, 140, 190);
const CHIP_SEL_BORDER: Color = Color::RGB(180, 150, 240);

const AUTHOR_BTN_BG: Color = Color::RGBA(50, 40, 80, 180);
const AUTHOR_BTN_TEXT: Color = Color::RGB(180, 170, 210);
const AUTHOR_BTN_ACT: Color = Color::RGB(160, 120, 230);
const AUTHOR_DD_BG: Color = Color::RGBA(30, 20, 55, 235);
const AUTHOR_DD_SEL: Color = Color::RGBA(100, 60, 180, 180);
const AUTHOR_DD_TEXT: Color = Color::RGB(200, 195, 230);

const CLEAR_BTN_BG: Color = Color::RGBA(60, 30, 30, 140);
const CLEAR_BTN_TEXT: Color = Color::RGB(200, 140, 140);

/// Ensure a row's textures are cached. Called before drawing a visible row.
fn cache_row(
    state: &mut DownloadBrowserState,
    canvas: &Canvas<Window>,
    font_title: &Font<'_, 'static>,
    font_small: &Font<'_, 'static>,
    entry_idx: usize,
) -> Result<(), String> {
    if state.row_cache.contains_key(&entry_idx) {
        return Ok(());
    }

    let index = state.index.as_ref().ok_or("no index")?;
    let entry = &index.entries[entry_idx];
    let title = &entry.title_display;
    let author = entry.author_display.as_deref().unwrap_or("Unknown");
    let rating = entry.rating;
    let downloads = entry.downloads;
    let stats = format!("{rating:.1}  {downloads}");

    let mut row = RenderedRow::default();

    // Pre‑render title in both unselected and selected colours (step 30)
    {
        let surf = font_title
            .render(title)
            .blended(TITLE_NORM)
            .map_err(|e| format!("title: {e}"))?;
        row.title_w = surf.width();
        row.title_h = surf.height();
        row.title_tex = Some(
            canvas
                .texture_creator()
                .create_texture_from_surface(&surf)
                .map_err(|e| format!("title tex: {e}"))?,
        );
    }
    {
        let surf_sel = font_title
            .render(title)
            .blended(Color::RGB(255, 255, 255))
            .map_err(|e| format!("title sel: {e}"))?;
        row.title_sel_tex = Some(
            canvas
                .texture_creator()
                .create_texture_from_surface(&surf_sel)
                .map_err(|e| format!("title sel tex: {e}"))?,
        );
    }
    {
        let surf = font_small
            .render(author)
            .blended(AUTHOR_COLOR)
            .map_err(|e| format!("author: {e}"))?;
        row.author_w = surf.width();
        row.author_h = surf.height();
        row.author_tex = Some(
            canvas
                .texture_creator()
                .create_texture_from_surface(&surf)
                .map_err(|e| format!("author tex: {e}"))?,
        );
    }
    {
        let surf = font_small
            .render(&stats)
            .blended(STATS_COLOR)
            .map_err(|e| format!("stats: {e}"))?;
        row.stats_w = surf.width();
        row.stats_h = surf.height();
        row.stats_tex = Some(
            canvas
                .texture_creator()
                .create_texture_from_surface(&surf)
                .map_err(|e| format!("stats tex: {e}"))?,
        );
    }

    state.row_cache.insert(entry_idx, row);
    Ok(())
}

/// Helper: ensure a cached text texture exists. Returns (w, h) after creation.
/// Use `state.tex_cache[key].0` to get the texture for copying.
fn ensure_tex_cache(
    state: &mut DownloadBrowserState,
    canvas: &mut Canvas<Window>,
    font: &Font<'_, 'static>,
    key: &str,
    text: &str,
    color: Color,
) -> Result<(u32, u32), String> {
    if let Some((_, w, h)) = state.tex_cache.get(key) {
        return Ok((*w, *h));
    }
    let surf = font
        .render(text)
        .blended(color)
        .map_err(|e| format!("cache '{key}': {e}"))?;
    let w = surf.width();
    let h = surf.height();
    let tex = canvas
        .texture_creator()
        .create_texture_from_surface(&surf)
        .map_err(|e| format!("cache tex '{key}': {e}"))?;
    state.tex_cache.insert(key.to_string(), (tex, w, h));
    Ok((w, h))
}

/// Layout helper: returns the Y position where mod rows start and their height.
fn mod_area(body_y: i32, body_h: i32) -> (i32, i32) {
    let top = body_y
        + SEARCH_BAR_H as i32
        + FILTERS_GAP
        + TAG_ROW_H as i32
        + FILTERS_GAP
        + AUTHOR_ROW_H as i32
        + FILTERS_GAP;
    let h = body_h - (top - body_y);
    (top, h)
}

/// Draw the download browser inside a panel body rectangle.
///
/// Layout:
///   [search bar 32px] [tag chips 28px] [author row 28px] [mod rows ...]
///
/// Returns the Y and height of the mod area so the caller can use it for
/// click‑hit testing.
#[allow(clippy::too_many_arguments)]
pub fn draw_download_browser(
    canvas: &mut Canvas<Window>,
    state: &mut DownloadBrowserState,
    font_small: &Font<'_, 'static>,
    font_title: &Font<'_, 'static>,
    body_x: i32,
    body_y: i32,
    body_w: i32,
    body_h: i32,
) -> Result<(), String> {
    // ── Search bar ──
    {
        let sb_y = body_y;
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(SEARCH_BAR_COLOR);
        canvas
            .fill_rect(sdl2::rect::Rect::new(
                body_x,
                sb_y,
                body_w as u32,
                SEARCH_BAR_H,
            ))
            .map_err(|e| e.to_string())?;
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        let search_display = if state.search_text.is_empty() {
            "/ Search...".to_string()
        } else {
            format!("/ {}", state.search_text)
        };
        let txt_color = if state.search_text.is_empty() {
            SEARCH_PLACEHOLDER
        } else {
            SEARCH_TEXT_COLOR
        };
        {
            let surf = font_small
                .render(&search_display)
                .blended(txt_color)
                .map_err(|e| format!("search text: {e}"))?;
            let tc = canvas.texture_creator();
            let tex = tc
                .create_texture_from_surface(&surf)
                .map_err(|e| format!("search tex: {e}"))?;
            let tw = surf.width();
            let th = surf.height();
            let tx = body_x + 32;
            let ty = sb_y + (SEARCH_BAR_H as i32 - th as i32) / 2;
            canvas
                .copy(&tex, None, Some(sdl2::rect::Rect::new(tx, ty, tw, th)))
                .map_err(|e| format!("search copy: {e}"))?;
        }

        if state.search_active && !state.search_text.is_empty() {
            let (cw, ch) = ensure_tex_cache(
                state,
                canvas,
                font_small,
                "static:cancel_x",
                "×",
                Color::RGB(180, 100, 100),
            )?;
            let tex = &state.tex_cache["static:cancel_x"].0;
            let cx = body_x + body_w - cw as i32 - 16;
            let cy = sb_y + (SEARCH_BAR_H as i32 - ch as i32) / 2;
            canvas
                .copy(tex, None, Some(sdl2::rect::Rect::new(cx, cy, cw, ch)))
                .map_err(|e| format!("clear copy: {e}"))?;
        }
    }

    // ── Tag chips row ──
    {
        let chip_y = body_y + SEARCH_BAR_H as i32 + FILTERS_GAP;
        let chip_h = TAG_ROW_H;
        let mut cx = body_x + 8;
        let chip_pad_h: i32 = 6;
        let chip_pad_v: i32 = 4;

        let top_tags = state.top_tags.clone();
        let chip_selected = state.tag_chip_selected;
        for (ci, (tag, count)) in top_tags.iter().enumerate() {
            let active = state.is_tag_active(tag);
            let is_sel = state.focus_mode == DlFocusMode::TagChips && ci == chip_selected;

            let display = format!("{tag} ({count})");
            let cache_key = format!("chip:{display}");

            let (tw, th) =
                ensure_tex_cache(state, canvas, font_small, &cache_key, &display, CHIP_TEXT)?;
            let w = tw as i32 + chip_pad_h * 2;
            let h = chip_h as i32;

            // Don't draw if it would go past body edge
            if cx + w > body_x + body_w - 8 {
                break;
            }

            let bg = if active { CHIP_BG_ACT } else { CHIP_BG_INACT };

            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(bg);
            canvas
                .fill_rect(sdl2::rect::Rect::new(cx, chip_y, w as u32, h as u32))
                .map_err(|e| e.to_string())?;

            if is_sel {
                canvas.set_draw_color(CHIP_SEL_BORDER);
                canvas
                    .draw_rect(sdl2::rect::Rect::new(
                        cx - 1,
                        chip_y - 1,
                        w as u32 + 2,
                        h as u32 + 2,
                    ))
                    .map_err(|e| e.to_string())?;
            }

            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            let tex = &state.tex_cache[&cache_key].0;
            canvas
                .copy(
                    tex,
                    None,
                    Some(sdl2::rect::Rect::new(
                        cx + chip_pad_h,
                        chip_y + chip_pad_v,
                        tw,
                        th,
                    )),
                )
                .map_err(|e| format!("chip copy: {e}"))?;

            cx += w + 4;
        }

        // "no tags" placeholder (one‑time render)
        #[allow(clippy::collapsible_if)]
        if state.top_tags.is_empty() {
            if state.index.is_some() {
                let (nw, nh) = ensure_tex_cache(
                    state,
                    canvas,
                    font_small,
                    "static:no_tags",
                    "(no tags)",
                    CHIP_COUNT,
                )?;
                let tex = &state.tex_cache["static:no_tags"].0;
                canvas
                    .copy(
                        tex,
                        None,
                        Some(sdl2::rect::Rect::new(cx, chip_y + 4, nw, nh)),
                    )
                    .map_err(|e| format!("no tags copy: {e}"))?;
            }
        }
    }

    // ── Author row ──
    {
        let author_y = body_y + SEARCH_BAR_H as i32 + FILTERS_GAP + TAG_ROW_H as i32 + FILTERS_GAP;
        let author_h = AUTHOR_ROW_H as i32;

        // Background
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(AUTHOR_BTN_BG);
        canvas
            .fill_rect(sdl2::rect::Rect::new(
                body_x,
                author_y,
                body_w as u32,
                author_h as u32,
            ))
            .map_err(|e| e.to_string())?;
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        let is_focused = state.focus_mode == DlFocusMode::AuthorInput;
        let focus_border = if state.author_dropdown_open {
            AUTHOR_BTN_ACT
        } else if is_focused {
            CHIP_SEL_BORDER
        } else {
            AUTHOR_BTN_BG
        };
        canvas.set_draw_color(focus_border);
        canvas
            .draw_rect(sdl2::rect::Rect::new(
                body_x,
                author_y,
                body_w as u32,
                author_h as u32,
            ))
            .map_err(|e| e.to_string())?;

        let label = if let Some(ref a) = state.active_author {
            format!("Author: {a}")
        } else {
            "Author: All".into()
        };
        let lbl_color = if state.active_author.is_some() {
            AUTHOR_BTN_ACT
        } else {
            AUTHOR_BTN_TEXT
        };
        // Cache author button label (only re‑render on author change)
        if label != state.cached_author_label {
            // Clear old cached texture so ensure_tex_cache creates a new one
            state.tex_cache.remove("author_btn:label");
            state.cached_author_label = label.clone();
        }
        let _cache_key = format!("author_btn:{label}");
        {
            // Render with correct colour (key includes author name only, colour is per‑frame)
            // We cache both colours as separate keys
            let colour_key = if state.active_author.is_some() {
                "author_btn:active:"
            } else {
                "author_btn:inactive:"
            };
            let full_key = format!("{colour_key}{label}");
            let (tw, th) =
                ensure_tex_cache(state, canvas, font_small, &full_key, &label, lbl_color)?;
            let tex = &state.tex_cache[&full_key].0;
            let ty = author_y + (author_h - th as i32) / 2;
            canvas
                .copy(
                    tex,
                    None,
                    Some(sdl2::rect::Rect::new(body_x + 8, ty, tw, th)),
                )
                .map_err(|e| format!("author label copy: {e}"))?;
        }

        // × button to clear author filter (cached)
        if state.active_author.is_some() {
            let (cw, ch) = ensure_tex_cache(
                state,
                canvas,
                font_small,
                "static:author_clear_x",
                "×",
                CLEAR_BTN_TEXT,
            )?;
            let tex = &state.tex_cache["static:author_clear_x"].0;
            let cx = body_x + body_w - 30;
            let cy = author_y + (author_h - ch as i32) / 2;
            canvas
                .copy(tex, None, Some(sdl2::rect::Rect::new(cx, cy, cw, ch)))
                .map_err(|e| format!("auth clear copy: {e}"))?;
        }

        // ▼ indicator (cached)
        {
            let (aw, ah) = ensure_tex_cache(
                state,
                canvas,
                font_small,
                "static:dropdown_arrow",
                "▼",
                AUTHOR_BTN_TEXT,
            )?;
            let tex = &state.tex_cache["static:dropdown_arrow"].0;
            let ax = body_x + body_w - 60;
            let ay = author_y + (author_h - ah as i32) / 2;
            canvas
                .copy(tex, None, Some(sdl2::rect::Rect::new(ax, ay, aw, ah)))
                .map_err(|e| format!("arrow copy: {e}"))?;
        }

        // ── Author dropdown overlay ──
        if state.author_dropdown_open {
            let dd_h = 200u32.min((state.author_filtered.len().max(1) as u32) * 22 + 8);
            let dd_y = author_y + author_h;
            let dd_w = (body_w as u32 / 2).max(180);
            let dd_x = body_x + 8;

            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(AUTHOR_DD_BG);
            canvas
                .fill_rect(sdl2::rect::Rect::new(dd_x, dd_y, dd_w, dd_h))
                .map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            let vis = ((dd_h as i32 - 4) / 22).max(1) as usize;
            let dd_scroll = state
                .author_list_selected
                .saturating_sub(vis.saturating_sub(1))
                .min(state.author_list_selected);
            let selected_in_dd = state.author_list_selected;
            let author_filtered = state.author_filtered.clone();

            for (i, author) in author_filtered.iter().skip(dd_scroll).take(vis).enumerate() {
                let idx = dd_scroll + i;
                let row_y = dd_y + 4 + i as i32 * 22;
                let is_sel = idx == selected_in_dd;

                if is_sel {
                    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
                    canvas.set_draw_color(AUTHOR_DD_SEL);
                    canvas
                        .fill_rect(sdl2::rect::Rect::new(dd_x, row_y, dd_w, 22))
                        .map_err(|e| e.to_string())?;
                    canvas.set_blend_mode(sdl2::render::BlendMode::None);
                }

                let dd_key = format!("dd:{author}");
                let (aw, ah) =
                    ensure_tex_cache(state, canvas, font_small, &dd_key, author, AUTHOR_DD_TEXT)?;
                let tex = &state.tex_cache[&dd_key].0;
                canvas
                    .copy(
                        tex,
                        None,
                        Some(sdl2::rect::Rect::new(dd_x + 4, row_y + 2, aw, ah)),
                    )
                    .map_err(|e| format!("dd author copy: {e}"))?;
            }

            if state.author_filtered.is_empty() {
                let (ew, eh) = ensure_tex_cache(
                    state,
                    canvas,
                    font_small,
                    "static:no_author_match",
                    "(no matches)",
                    AUTHOR_COLOR,
                )?;
                let tex = &state.tex_cache["static:no_author_match"].0;
                canvas
                    .copy(
                        tex,
                        None,
                        Some(sdl2::rect::Rect::new(dd_x + 4, dd_y + 4, ew, eh)),
                    )
                    .map_err(|e| format!("dd empty copy: {e}"))?;
            }
        }
    }

    // ── Mod rows ──
    let (row_y_offset, row_body_h) = mod_area(body_y, body_h);

    let total = state.total();
    if total == 0 {
        return Ok(());
    }

    let visible = DL_VISIBLE_ROWS.min(total);
    let end = (state.scroll + visible).min(total);

    for list_idx in state.scroll..end {
        if let Some(entry_idx) = state.entry_idx(list_idx) {
            cache_row(state, canvas, font_title, font_small, entry_idx)?;
        }
    }

    for (vi, list_idx) in (state.scroll..end).enumerate() {
        let entry_idx = match state.entry_idx(list_idx) {
            Some(ei) => ei,
            None => continue,
        };
        let row_y = row_y_offset + vi as i32 * ROW_STEP as i32;
        let is_sel = list_idx == state.selected && state.focus_mode == DlFocusMode::ModList;

        if is_sel {
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(HIGHLIGHT_COLOR);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, body_w as u32, ROW_H))
                .map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            canvas.set_draw_color(ACCENT_COLOR);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, ACCENT_W, ROW_H))
                .map_err(|e| e.to_string())?;
        }

        let row = state.row_cache.get_mut(&entry_idx);
        if let Some(row) = row {
            // Step 30: use pre‑rendered selected/unselected textures
            let title_tex = if is_sel {
                row.title_sel_tex.as_ref()
            } else {
                row.title_tex.as_ref()
            };
            if let Some(tt) = title_tex {
                canvas
                    .copy(
                        tt,
                        None,
                        Some(sdl2::rect::Rect::new(
                            body_x + 16,
                            row_y + 4,
                            row.title_w,
                            row.title_h,
                        )),
                    )
                    .map_err(|e| e.to_string())?;
            }

            if let Some(ref at) = row.author_tex {
                canvas
                    .copy(
                        at,
                        None,
                        Some(sdl2::rect::Rect::new(
                            body_x + 16,
                            row_y + ROW_H as i32 - row.author_h as i32 - 4,
                            row.author_w,
                            row.author_h,
                        )),
                    )
                    .map_err(|e| e.to_string())?;
            }

            if let Some(ref st) = row.stats_tex {
                canvas
                    .copy(
                        st,
                        None,
                        Some(sdl2::rect::Rect::new(
                            body_x + body_w - row.stats_w as i32 - 16,
                            row_y + (ROW_H as i32 - row.stats_h as i32) / 2,
                            row.stats_w,
                            row.stats_h,
                        )),
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    // ── Scrollbar ──
    if total > visible {
        let sb_x = body_x + body_w - SCROLLBAR_W as i32 - SCROLLBAR_PAD;
        let thumb_h = (row_body_h as f32 * visible as f32 / total as f32).max(16.0) as u32;
        let max_scroll = total.saturating_sub(visible) as f64;
        let thumb_y = if max_scroll > 0.0 {
            row_y_offset
                + ((row_body_h - thumb_h as i32) as f64 * state.scroll as f64 / max_scroll) as i32
        } else {
            row_y_offset
        };

        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(SCROLL_TRACK);
        canvas
            .fill_rect(sdl2::rect::Rect::new(
                sb_x,
                row_y_offset,
                SCROLLBAR_W,
                row_body_h as u32,
            ))
            .map_err(|e| e.to_string())?;
        canvas.set_draw_color(SCROLL_THUMB);
        canvas
            .fill_rect(sdl2::rect::Rect::new(sb_x, thumb_y, SCROLLBAR_W, thumb_h))
            .map_err(|e| e.to_string())?;
        canvas.set_blend_mode(sdl2::render::BlendMode::None);
    }

    // ── Footer: Clear‑all (left) + pagination (right) ──
    {
        let footer_y = body_y + body_h - 24;
        let has_filters =
            state.search_active || !state.active_tags.is_empty() || state.active_author.is_some();

        // ── Clear all button (left, only when filters active) ──
        if has_filters {
            let clear_label = format!(
                "Clear all (text:{} tags:{} author:{})",
                if state.search_text.is_empty() {
                    "–"
                } else {
                    "×"
                },
                state.active_tags.len(),
                if state.active_author.is_some() {
                    "×"
                } else {
                    "–"
                },
            );
            let btn_key = format!("clear_all:{clear_label}");
            let (tw, th) = ensure_tex_cache(
                state,
                canvas,
                font_small,
                &btn_key,
                &clear_label,
                CLEAR_BTN_TEXT,
            )?;
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(CLEAR_BTN_BG);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, footer_y, tw + 16, 22))
                .map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);
            let tex = &state.tex_cache[&btn_key].0;
            canvas
                .copy(
                    tex,
                    None,
                    Some(sdl2::rect::Rect::new(body_x + 8, footer_y + 2, tw, th)),
                )
                .map_err(|e| format!("clear all copy: {e}"))?;
        }

        // ── Pagination (right) ──
        let tp = state.total_pages();
        let cp = state.current_page();
        let page_text = format!("Page {} / {tp}", cp + 1);
        let page_key = format!("page:{page_text}");
        let (pw, ph) = ensure_tex_cache(
            state,
            canvas,
            font_small,
            &page_key,
            &page_text,
            AUTHOR_BTN_TEXT,
        )?;
        let px = body_x + body_w - pw as i32 - 16;
        let py = footer_y + 2;
        let pt = &state.tex_cache[&page_key].0;
        canvas
            .copy(pt, None, Some(sdl2::rect::Rect::new(px, py, pw, ph)))
            .map_err(|e| format!("page copy: {e}"))?;

        // Navigation arrows (static — cached once)
        {
            let nav_chars = ["◀◀", "◀", "▶", "▶▶"];
            let nav_keys = [
                "static:nav_first",
                "static:nav_prev",
                "static:nav_next",
                "static:nav_last",
            ];
            // ◀◀ (first page) + ◀ (prev page), left of page text
            let mut nav_x = body_x + body_w - (pw as i32 + 16 + 20 + 20 + 20 + 20);
            for (i, &ch) in nav_chars.iter().enumerate() {
                let (nw, nh) =
                    ensure_tex_cache(state, canvas, font_small, nav_keys[i], ch, AUTHOR_BTN_TEXT)?;
                let nt = &state.tex_cache[nav_keys[i]].0;
                let ny = footer_y + (22 - nh as i32) / 2;
                canvas
                    .copy(nt, None, Some(sdl2::rect::Rect::new(nav_x, ny, nw, nh)))
                    .map_err(|e| format!("nav copy {i}: {e}"))?;
                if i == 1 {
                    // After ◀◀ and ◀ → page text
                    nav_x += nw as i32 + 4;
                } else if i == 2 {
                    // After ▶ → ▶▶
                    nav_x += nw as i32 + 4;
                } else if i == 0 {
                    nav_x += nw as i32 + 4;
                } // i == 3 (last): just trailing
            }
        }
    }

    Ok(())
}

/// Check if a mouse click at (mx, my) hits the clear‑all button.
pub fn hit_clear_all(
    mx: i32,
    my: i32,
    body_x: i32,
    body_y: i32,
    _body_w: i32,
    body_h: i32,
    has_filters: bool,
) -> bool {
    if !has_filters {
        return false;
    }
    let footer_y = body_y + body_h - 24;
    // Approximate: button is roughly 240px wide from left
    let btn_w: i32 = 240;
    mx >= body_x && mx <= body_x + btn_w && my >= footer_y && my <= footer_y + 22
}

/// Check if a mouse click at (mx, my) hits the author dropdown button.
pub fn hit_author_btn(
    mx: i32,
    my: i32,
    body_x: i32,
    body_y: i32,
    body_w: i32,
    body_h: i32,
) -> bool {
    let _ = body_h;
    let author_y = body_y + SEARCH_BAR_H as i32 + FILTERS_GAP + TAG_ROW_H as i32 + FILTERS_GAP;
    mx >= body_x && mx <= body_x + body_w && my >= author_y && my <= author_y + AUTHOR_ROW_H as i32
}

/// Check if a mouse click at (mx, my) hits the author clear (×) button.
pub fn hit_author_clear(mx: i32, my: i32, body_x: i32, body_y: i32, body_w: i32) -> bool {
    let author_y = body_y + SEARCH_BAR_H as i32 + FILTERS_GAP + TAG_ROW_H as i32 + FILTERS_GAP;
    let cx = body_x + body_w - 30;
    mx >= cx - 4 && mx <= cx + 20 && my >= author_y && my <= author_y + AUTHOR_ROW_H as i32
}

/// Check if a mouse click at (mx, my) hits a specific tag chip.
/// Returns the chip index if hit.
pub fn hit_tag_chip(
    mx: i32,
    my: i32,
    body_x: i32,
    body_y: i32,
    body_w: i32,
    state: &DownloadBrowserState,
    font_small: &Font<'_, 'static>,
) -> Option<usize> {
    let chip_y = body_y + SEARCH_BAR_H as i32 + FILTERS_GAP;
    if my < chip_y || my > chip_y + TAG_ROW_H as i32 {
        return None;
    }

    let mut cx = body_x + 8;
    for (ci, (tag, count)) in state.top_tags.iter().enumerate() {
        let display = format!("{tag} ({count})");
        // Estimate chip width from rendered text size
        let w_est = if let Ok((tw, _)) = font_small.size_of(&display) {
            tw as i32 + 12
        } else {
            (display.len() as i32) * 9 + 12
        };
        let chip_w = w_est;
        if cx + chip_w > body_x + body_w - 8 {
            break;
        }
        if mx >= cx && mx <= cx + chip_w {
            return Some(ci);
        }
        cx += chip_w + 4;
    }
    None
}

/// Simple progress bar overlay for downloads. Rendered on top of the panel
/// body when a download is active.
///
/// Caller should lock `progress` before calling.
pub fn draw_progress_overlay(
    canvas: &mut Canvas<Window>,
    font: &Font<'_, 'static>,
    progress: &DownloadProgress,
    win_w: u32,
    win_h: u32,
    is_cancelling: bool,
) -> Result<(), String> {
    let bar_w: u32 = 400;
    let bar_h: u32 = 24;
    let bar_x = (win_w - bar_w) as i32 / 2;
    let bar_y = win_h as i32 - 80;
    let pct = progress.pct();

    // Background
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(20, 15, 40, 220));
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            bar_x - 4,
            bar_y - 4,
            bar_w + 8,
            bar_h + 8 + 20,
        ))
        .map_err(|e| e.to_string())?;
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    // Track
    canvas.set_draw_color(Color::RGBA(60, 50, 100, 180));
    canvas
        .fill_rect(sdl2::rect::Rect::new(bar_x, bar_y, bar_w, bar_h))
        .map_err(|e| e.to_string())?;

    // Fill
    let fill_color = if is_cancelling {
        Color::RGBA(200, 80, 80, 220)
    } else {
        Color::RGBA(120, 80, 220, 220)
    };
    let fill_w = (bar_w as u64 * pct as u64 / 100) as u32;
    canvas.set_draw_color(fill_color);
    canvas
        .fill_rect(sdl2::rect::Rect::new(bar_x, bar_y, fill_w, bar_h))
        .map_err(|e| e.to_string())?;

    // Label
    let label = if let Some(ref err) = progress.error {
        format!("Error: {err}")
    } else if progress.done && progress.extracted > 0 {
        format!(
            "{} mod(s) installed — reopen Mod Manager to load",
            progress.extracted
        )
    } else if progress.done {
        "Download complete!".to_string()
    } else if is_cancelling {
        "Cancelling...".to_string()
    } else if let Some(total) = progress.total {
        let down_mb = progress.bytes as f64 / (1024.0 * 1024.0);
        let total_mb = total as f64 / (1024.0 * 1024.0);
        format!("{down_mb:.1} / {total_mb:.1} MB  ({pct}%)")
    } else {
        let down_mb = progress.bytes as f64 / (1024.0 * 1024.0);
        format!("{down_mb:.1} MB")
    };
    let surf = font
        .render(&label)
        .blended(Color::RGB(220, 210, 240))
        .map_err(|e| format!("progress label: {e}"))?;
    let tc = canvas.texture_creator();
    let tex = tc
        .create_texture_from_surface(&surf)
        .map_err(|e| format!("progress tex: {e}"))?;
    let lx = bar_x + (bar_w as i32 - surf.width() as i32) / 2;
    let ly = bar_y + bar_h as i32 + 2;
    canvas
        .copy(
            &tex,
            None,
            Some(sdl2::rect::Rect::new(lx, ly, surf.width(), surf.height())),
        )
        .map_err(|e| format!("progress copy: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(count: usize) -> DownloadBrowserState {
        let mut s = DownloadBrowserState::new();
        s.sorted = (0..count).collect();
        s
    }

    #[test]
    fn total_pages_empty_is_1() {
        let s = make_state(0);
        assert_eq!(s.total_pages(), 1);
    }

    #[test]
    fn total_pages_divisions() {
        assert_eq!(make_state(15).total_pages(), 1);
        assert_eq!(make_state(16).total_pages(), 2);
        assert_eq!(make_state(30).total_pages(), 2);
        assert_eq!(make_state(31).total_pages(), 3);
    }

    #[test]
    fn page_nav_clamps() {
        let mut s = make_state(50); // 4 pages (0–3)
        s.change_page(1);
        assert_eq!(s.current_page(), 1);
        s.change_page(5); // would be page 6 → clamped to 3
        assert_eq!(s.current_page(), 3);
        s.change_page(-10); // would be -7 → clamped to 0
        assert_eq!(s.current_page(), 0);
    }

    #[test]
    fn page_resets_on_filter_change() {
        let mut s = make_state(50);
        s.change_page(2);
        assert_eq!(s.current_page(), 2);
        // Simulate filter change: update_filter resets page via reset_position
        s.active_tags.push("test".into());
        s.filter_key = None;
        s.update_filter();
        assert_eq!(s.current_page(), 0);
        assert_eq!(s.scroll, 0);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn filtered_total_pages() {
        // With filtered list of 10 items: ceil(10/15) = 1 page
        let mut s = make_state(100);
        s.filtered = Some((0..10).collect());
        assert_eq!(s.total_pages(), 1);
    }
}
