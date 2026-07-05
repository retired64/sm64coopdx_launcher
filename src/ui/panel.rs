use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::video::Window;

use crate::assets::FontCache;

const PANEL_W_RATIO: f32 = 0.62;
const PANEL_H_RATIO: f32 = 0.72;
const HEADER_H: u32 = 48;
const FOOTER_H: u32 = 36;
const BORDER_W: u32 = 2;

const PANEL_BG: Color = Color::RGBA(10, 5, 30, 230);
const PANEL_BORDER: Color = Color::RGBA(120, 70, 220, 100);
const HEADER_BG: Color = Color::RGBA(15, 8, 40, 200);
const HEADER_TEXT: Color = Color::RGB(200, 160, 255);
const FOOTER_TEXT: Color = Color::RGB(140, 120, 180);
const CORNER_RADIUS: i32 = 16;

pub const SLIDE_SPEED: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubScreenType {
    ModManager,
    DynosPacks,
    Network,
    DownloadBrowser,
    Profiles,
    #[allow(dead_code)]
    ProfileDetail,
}

impl SubScreenType {
    pub fn header_label(&self) -> &'static str {
        match self {
            SubScreenType::ModManager => "MOD MANAGER",
            SubScreenType::DynosPacks => "DYNOS PACKS",
            SubScreenType::Network => "NETWORK",
            SubScreenType::DownloadBrowser => "DOWNLOAD MODS",
            SubScreenType::Profiles => "PROFILES",
            SubScreenType::ProfileDetail => "PROFILE",
        }
    }
}

/// Map arc‑menu button index → SubScreenType.
pub fn subscreen_for_menu_index(idx: usize) -> Option<SubScreenType> {
    match idx {
        0 => Some(SubScreenType::ModManager),
        1 => Some(SubScreenType::DynosPacks),
        2 => Some(SubScreenType::Network),
        3 => Some(SubScreenType::DownloadBrowser),
        4 => Some(SubScreenType::Profiles),
        _ => None,
    }
}

pub struct PanelState {
    pub slide_x: f64,
    target_x: f64,
    pub active: Option<SubScreenType>,
    pub header_extra: Option<String>,
    pub footer_hint: Option<String>,
    // Cached header/footer textures — rendered once on open, reused per frame
    header_tex: Option<sdl2::render::Texture>,
    header_w: u32,
    header_h: u32,
    footer_tex: Option<sdl2::render::Texture>,
    footer_w: u32,
    footer_h: u32,
    // Rounded‑corner background texture (step 30). Regenerated on window resize.
    bg_tex: Option<sdl2::render::Texture>,
    bg_tex_w: u32,
    bg_tex_h: u32,
}

impl PanelState {
    pub fn new(win_w: u32) -> Self {
        let hidden = win_w as f64;
        Self {
            slide_x: hidden,
            target_x: hidden,
            active: None,
            header_extra: None,
            footer_hint: None,
            header_tex: None,
            header_w: 0,
            header_h: 0,
            footer_tex: None,
            footer_w: 0,
            footer_h: 0,
            bg_tex: None,
            bg_tex_w: 0,
            bg_tex_h: 0,
        }
    }

    fn visible_x(win_w: u32, panel_w: u32) -> f64 {
        ((win_w - panel_w) as f64 / 2.0) + 30.0
    }

    pub fn open(&mut self, sub_type: SubScreenType, win_w: u32) {
        self.active = Some(sub_type);
        let pw = (win_w as f32 * PANEL_W_RATIO) as u32;
        self.target_x = Self::visible_x(win_w, pw);
        self.header_extra = None;
        self.footer_hint = None;
        // Invalidate cached textures so they are re‑rendered for new sub‑type
        self.header_tex = None;
        self.footer_tex = None;
    }

    pub fn close(&mut self, win_w: u32) {
        self.target_x = win_w as f64;
    }

    pub fn invalidate_cache(&mut self) {
        self.header_tex = None;
        self.footer_tex = None;
    }

    pub fn update(&mut self, dt: f64, win_w: u32) {
        self.slide_x += (self.target_x - self.slide_x) * (SLIDE_SPEED * dt).min(1.0);
        if !self.is_visible(win_w) && self.target_x == win_w as f64 {
            // Fully hidden — clear active screen so we stop rendering
            self.active = None;
        }
        if self.target_x != win_w as f64 && self.slide_x.approx_eq(win_w as f64) {
            // Snap to exact hidden value when opening
            self.slide_x = self.target_x;
        }
    }

    pub fn is_visible(&self, win_w: u32) -> bool {
        self.slide_x < (win_w as i32 - 10) as f64
    }

    #[allow(dead_code)]
    pub fn progress(&self, win_w: u32) -> f32 {
        let pw = (win_w as f32 * PANEL_W_RATIO) as u32;
        let hidden = win_w as f64;
        let visible = Self::visible_x(win_w, pw);
        let range = hidden - visible;
        if range <= 0.0 {
            return 0.0;
        }
        ((hidden - self.slide_x) / range).clamp(0.0, 1.0) as f32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render<'ttf>(
        &mut self,
        canvas: &mut Canvas<Window>,
        font_cache: &mut FontCache<'ttf>,
        ttf: &'ttf sdl2::ttf::Sdl2TtfContext,
        font_path: &std::path::Path,
        font_size_hdr: u16,
        font_size_ftr: u16,
        win_w: u32,
        win_h: u32,
    ) -> Result<(), String> {
        if self.active.is_none() {
            return Ok(());
        }
        let sub_type = self.active.unwrap();

        let pw = (win_w as f32 * PANEL_W_RATIO) as u32;
        let ph = (win_h as f32 * PANEL_H_RATIO) as u32;
        let px = self.slide_x as i32;
        let py = (win_h as i32 - ph as i32) / 2;

        if !self.is_visible(win_w) {
            return Ok(());
        }

        // ── Rounded‑corner panel background (step 30) ──
        // Pre‑render once per resize; just copy the cached texture each frame.
        if self.bg_tex.is_none() || self.bg_tex_w != pw || self.bg_tex_h != ph {
            self.bg_tex = Some(make_rounded_bg(canvas, pw, ph, CORNER_RADIUS)?);
            self.bg_tex_w = pw;
            self.bg_tex_h = ph;
        }
        if let Some(ref bg) = self.bg_tex {
            canvas
                .copy(bg, None, Some(sdl2::rect::Rect::new(px, py, pw, ph)))
                .map_err(|e| e.to_string())?;
        }

        // Rounded border 2px (approximation via inset rounded rect outlines)
        canvas.set_draw_color(PANEL_BORDER);
        for o in 0..BORDER_W as i32 {
            draw_rounded_rect_outline(
                canvas,
                px + o,
                py + o,
                pw as i32 - o * 2,
                ph as i32 - o * 2,
                CORNER_RADIUS - o.max(0),
            )?;
        }

        // Header background
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(HEADER_BG);
        canvas
            .fill_rect(sdl2::rect::Rect::new(
                px + BORDER_W as i32,
                py + BORDER_W as i32,
                pw - BORDER_W * 2,
                HEADER_H,
            ))
            .map_err(|e| e.to_string())?;
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        // Header title — cache texture on first render call
        if self.header_tex.is_none() {
            let font_hdr = font_cache.get_font(ttf, font_path, font_size_hdr)?;
            let mut label = sub_type.header_label().to_string();
            if let Some(ref extra) = self.header_extra {
                label.push(' ');
                label.push_str(extra);
            }
            let surf = font_hdr
                .render(&label)
                .blended(HEADER_TEXT)
                .map_err(|e| e.to_string())?;
            self.header_w = surf.width();
            self.header_h = surf.height();
            self.header_tex = Some(
                canvas
                    .texture_creator()
                    .create_texture_from_surface(&surf)
                    .map_err(|e| e.to_string())?,
            );
        }
        if let Some(ref ht) = self.header_tex {
            let tx = px + 16;
            let ty = py + (HEADER_H as i32 - self.header_h as i32) / 2;
            canvas
                .copy(
                    ht,
                    None,
                    Some(sdl2::rect::Rect::new(tx, ty, self.header_w, self.header_h)),
                )
                .map_err(|e| e.to_string())?;
        }

        // Footer hint — cache texture on first render call
        if self.footer_tex.is_none() {
            let font_ftr = font_cache.get_font(ttf, font_path, font_size_ftr)?;
            let hint = self
                .footer_hint
                .as_deref()
                .unwrap_or("ENTER: Select  |  ESC: Back");
            let surf = font_ftr
                .render(hint)
                .blended(FOOTER_TEXT)
                .map_err(|e| e.to_string())?;
            self.footer_w = surf.width();
            self.footer_h = surf.height();
            self.footer_tex = Some(
                canvas
                    .texture_creator()
                    .create_texture_from_surface(&surf)
                    .map_err(|e| e.to_string())?,
            );
        }
        if let Some(ref ft) = self.footer_tex {
            let tx = px + (pw as i32 - self.footer_w as i32) / 2;
            let ty =
                py + ph as i32 - FOOTER_H as i32 + (FOOTER_H as i32 - self.footer_h as i32) / 2;
            canvas
                .copy(
                    ft,
                    None,
                    Some(sdl2::rect::Rect::new(tx, ty, self.footer_w, self.footer_h)),
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Software rounded‑corner helpers (step 30 — no SDL2_gfx dependency)
// ═══════════════════════════════════════════════════════════════════════

/// Pre‑render a rounded‑rectangle background texture.
///
/// Creates an ARGB surface, fills it with PANEL_BG, then sets corner pixels
/// outside the radius to fully transparent. The resulting texture is cached
/// in PanelState and only regenerated on window resize.
fn make_rounded_bg(
    canvas: &Canvas<Window>,
    pw: u32,
    ph: u32,
    radius: i32,
) -> Result<sdl2::render::Texture, String> {
    use sdl2::pixels::PixelFormatEnum;
    use sdl2::surface::Surface;

    let mut surface = Surface::new(pw, ph, PixelFormatEnum::ARGB8888)
        .map_err(|e| format!("rounded bg surface: {e}"))?;
    surface
        .fill_rect(None, PANEL_BG)
        .map_err(|e| format!("rounded bg fill: {e}"))?;

    // Punch transparent corners using midpoint‑circle test
    let w = pw as i32;
    let h = ph as i32;
    let r2 = radius * radius;

    surface.with_lock_mut(|pixels: &mut [u8]| {
        let pitch = (pw * 4) as usize;
        // 4 corners: TL=(r,r), TR=(w-r,r), BL=(r,h-r), BR=(w-r,h-r)
        let corners: [(i32, i32); 4] = [
            (radius, radius),
            (w - radius, radius),
            (radius, h - radius),
            (w - radius, h - radius),
        ];
        for (cx, cy) in &corners {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy > r2 {
                        let px = cx + dx;
                        let py = cy + dy;
                        if px >= 0 && px < w && py >= 0 && py < h {
                            let idx = (py as usize) * pitch + (px as usize) * 4 + 3; // alpha byte
                            pixels[idx] = 0;
                        }
                    }
                }
            }
        }
    });

    canvas
        .texture_creator()
        .create_texture_from_surface(&surface)
        .map_err(|e| format!("rounded bg tex: {e}"))
}

/// Draw a 1‑pixel rounded‑rectangle outline using the midpoint circle
/// algorithm on each corner. Only draws the straight segments and the
/// corner arcs (no fill).
pub(crate) fn draw_rounded_rect_outline(
    canvas: &mut Canvas<Window>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
) -> Result<(), String> {
    if w <= 0 || h <= 0 || radius <= 0 {
        canvas
            .draw_rect(sdl2::rect::Rect::new(x, y, w as u32, h as u32))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let r = radius;
    let r2 = r * r;

    // Straight segments: top, bottom, left, right (between corner arcs)
    // Top edge (x+r .. x+w-r)
    canvas
        .draw_line((x + r, y), (x + w - r, y))
        .map_err(|e| e.to_string())?;
    // Bottom edge
    canvas
        .draw_line((x + r, y + h), (x + w - r, y + h))
        .map_err(|e| e.to_string())?;
    // Left edge
    canvas
        .draw_line((x, y + r), (x, y + h - r))
        .map_err(|e| e.to_string())?;
    // Right edge
    canvas
        .draw_line((x + w, y + r), (x + w, y + h - r))
        .map_err(|e| e.to_string())?;

    // Corner arcs (quarter circles) — draw point per pixel
    let corners: [(i32, i32); 4] = [
        (x + r, y + r),         // TL
        (x + w - r, y + r),     // TR
        (x + r, y + h - r),     // BL
        (x + w - r, y + h - r), // BR
    ];

    for (cx, cy) in &corners {
        // Midpoint circle: iterate over one octant, mirror
        let mut dx = 0i32;
        let mut dy = r;
        let mut d = 1 - r;
        while dx <= dy {
            // Plot 8 symmetric points (but only those in this corner quadrant)
            let pts = [
                (*cx - dy, *cy - dx), // TL corner
                (*cx - dx, *cy - dy),
                (*cx + dx, *cy - dy), // TR corner
                (*cx + dy, *cy - dx),
                (*cx + dy, *cy + dx), // BR corner
                (*cx + dx, *cy + dy),
                (*cx - dx, *cy + dy), // BL corner
                (*cx - dy, *cy + dx),
            ];
            for (px, py) in &pts {
                // Only draw if this point is in the correct quadrant for this corner
                let in_quadrant = {
                    let rel_x = px - cx;
                    let rel_y = py - cy;
                    rel_x * rel_x + rel_y * rel_y <= r2
                };
                if in_quadrant && *px >= 0 && *py >= 0 {
                    canvas.draw_point((*px, *py)).map_err(|e| e.to_string())?;
                }
            }
            dx += 1;
            if d < 0 {
                d += 2 * dx + 1;
            } else {
                dy -= 1;
                d += 2 * (dx - dy) + 1;
            }
        }
    }

    Ok(())
}

/// Compute the body rectangle (area below header, above footer) for item
/// selector rendering. Used by sub‑screens that need to draw item lists.
pub fn panel_body_rect(win_w: u32, win_h: u32, slide_x: f64) -> sdl2::rect::Rect {
    let pw = (win_w as f32 * PANEL_W_RATIO) as u32;
    let ph = (win_h as f32 * PANEL_H_RATIO) as u32;
    let px = slide_x as i32;
    let py = (win_h as i32 - ph as i32) / 2;
    sdl2::rect::Rect::new(
        px + BORDER_W as i32,
        py + HEADER_H as i32 + BORDER_W as i32,
        pw - BORDER_W * 2,
        ph - HEADER_H - FOOTER_H - BORDER_W * 2,
    )
}

trait ApproxEq {
    fn approx_eq(&self, other: Self) -> bool;
}

impl ApproxEq for f64 {
    fn approx_eq(&self, other: f64) -> bool {
        (self - other).abs() < 0.5
    }
}
