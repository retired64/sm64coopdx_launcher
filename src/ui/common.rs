use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::video::Window;

pub const ROW_H: u32 = 46;
pub const ROW_GAP: u32 = 3;
pub const ROW_STEP: u32 = ROW_H + ROW_GAP;
pub const DEFAULT_VISIBLE_ROWS: usize = 9;
#[allow(dead_code)]
pub const DL_VISIBLE_ROWS: usize = 6;
pub const DL_PAGE_SIZE: usize = 15;
pub const ACCENT_W: u32 = 4;

pub const SCROLLBAR_W: u32 = 6;
pub const SCROLLBAR_PAD: i32 = 4;

const HIGHLIGHT_COLOR: Color = Color::RGBA(80, 35, 180, 70);
/// Lighter highlight for mouse‑hover feedback (step 30).
const HOVER_COLOR: Color = Color::RGBA(80, 35, 180, 30);
const ACCENT_COLOR: Color = Color::RGB(160, 100, 240);
#[allow(dead_code)]
const SELECTED_TEXT: Color = Color::RGB(255, 255, 255);
#[allow(dead_code)]
const NORMAL_TEXT: Color = Color::RGB(185, 175, 210);
const SCROLL_TRACK: Color = Color::RGBA(30, 20, 60, 100);
const SCROLL_THUMB: Color = Color::RGBA(140, 100, 220, 150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ItemType {
    Toggle,
    Text,
    Action,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UiItem {
    pub name: String,
    pub rel_path: String,
    pub enabled: bool,
    pub item_type: ItemType,
    pub value: String,
}

/// Selection navigation: wraparound is DISABLED for item lists.
///
/// Unlike the arc menu (which wraps because it's a small circular set of 5
/// buttons), item lists have a natural top and bottom. Wraparound here would
/// jump from item 0 to the last item, which is confusing in a potentially
/// long scrollable list. The spec does not mention wraparound for the item
/// selector; this matches standard desktop UI patterns.
#[allow(dead_code)]
pub fn clamp_selection(items: &[UiItem], selected: usize) -> usize {
    if items.is_empty() {
        return 0;
    }
    selected.min(items.len() - 1)
}

#[allow(dead_code)]
pub fn clamp_scroll(items: &[UiItem], scroll: usize, visible_rows: usize) -> usize {
    if items.is_empty() {
        return 0;
    }
    let max_scroll = items.len().saturating_sub(visible_rows);
    scroll.min(max_scroll)
}

pub fn ensure_selection_visible(selected: usize, scroll: usize, visible_rows: usize) -> usize {
    if selected < scroll {
        selected
    } else if selected >= scroll + visible_rows {
        selected - visible_rows + 1
    } else {
        scroll
    }
}

/// Draw the generic item selector inside a panel body rectangle.
///
/// All text textures and icon textures must be pre‑rendered by the caller.
/// This function performs zero allocations — only `canvas.copy()` and
/// `canvas.fill_rect()` calls.
#[allow(clippy::too_many_arguments)]
pub fn draw_item_selector(
    canvas: &mut Canvas<Window>,
    items: &[UiItem],
    selected: usize,
    scroll: usize,
    visible_rows: usize,
    show_toggle: bool,
    // Pre‑rendered item name textures (index‑aligned with items)
    item_tex: &[Option<sdl2::render::Texture>],
    item_w: &[u32],
    item_h: &[u32],
    // Pre‑rendered toggle icon textures
    icon_check: &sdl2::render::Texture,
    icon_cross: &sdl2::render::Texture,
    icon_plus: &sdl2::render::Texture,
    icon_w: u32,
    icon_h: u32,
    // Panel body rectangle
    body_x: i32,
    body_y: i32,
    body_w: i32,
    body_h: i32,
    // Hovered row (None = mouse is outside the list)
    hovered: Option<usize>,
) -> Result<(), String> {
    let total = items.len();
    if total == 0 {
        return Ok(());
    }

    // ── Rows ──
    let end = (scroll + visible_rows).min(total);
    for idx in scroll..end {
        let item = &items[idx];
        let row_y = body_y + (idx - scroll) as i32 * ROW_STEP as i32;

        // Hover highlight (lighter, only when not already selected — step 30)
        if let Some(h) = hovered
            && h == idx
            && idx != selected
        {
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(HOVER_COLOR);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, body_w as u32, ROW_H))
                .map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);
        }

        // Selected highlight
        if idx == selected {
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(HIGHLIGHT_COLOR);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, body_w as u32, ROW_H))
                .map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            // Accent bar
            canvas.set_draw_color(ACCENT_COLOR);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, ACCENT_W, ROW_H))
                .map_err(|e| e.to_string())?;
        }

        // Toggle / action icon
        if show_toggle {
            let ix = body_x + 12;
            let iy = row_y + (ROW_H as i32 - icon_h as i32) / 2;
            let i_tex = match item.item_type {
                ItemType::Action => icon_plus,
                ItemType::Toggle if item.enabled => icon_check,
                _ => icon_cross,
            };
            canvas
                .copy(
                    i_tex,
                    None,
                    Some(sdl2::rect::Rect::new(ix, iy, icon_w, icon_h)),
                )
                .map_err(|e| e.to_string())?;
        }

        // Item name texture
        if let Some(tex) = item_tex.get(idx).and_then(|t| t.as_ref()) {
            let tw = item_w[idx];
            let th = item_h[idx];
            let tx = body_x + if show_toggle { 40 } else { 16 };
            let ty = row_y + (ROW_H as i32 - th as i32) / 2;
            canvas
                .copy(tex, None, Some(sdl2::rect::Rect::new(tx, ty, tw, th)))
                .map_err(|e| e.to_string())?;
        }
    }

    // ── Scrollbar (only if total > visible) ──
    if total > visible_rows {
        let sb_x = body_x + body_w - SCROLLBAR_W as i32 - SCROLLBAR_PAD;
        let thumb_h = (body_h as f32 * visible_rows as f32 / total as f32).max(16.0) as u32;
        let max_scroll = (total - visible_rows) as f64;
        let thumb_y = if max_scroll > 0.0 {
            body_y + ((body_h - thumb_h as i32) as f64 * scroll as f64 / max_scroll) as i32
        } else {
            body_y
        };

        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(SCROLL_TRACK);
        canvas
            .fill_rect(sdl2::rect::Rect::new(
                sb_x,
                body_y,
                SCROLLBAR_W,
                body_h as u32,
            ))
            .map_err(|e| e.to_string())?;
        canvas.set_draw_color(SCROLL_THUMB);
        canvas
            .fill_rect(sdl2::rect::Rect::new(sb_x, thumb_y, SCROLLBAR_W, thumb_h))
            .map_err(|e| e.to_string())?;
        canvas.set_blend_mode(sdl2::render::BlendMode::None);
    }

    Ok(())
}

/// Hit-test which item row is under (mouse_x, mouse_y) within the panel body.
/// Returns `Some(index)` into the full items list, or `None` if the click
/// is outside any visible row.
#[allow(clippy::too_many_arguments)]
pub fn hit_test_item_row(
    mouse_x: i32,
    mouse_y: i32,
    body_x: i32,
    body_y: i32,
    body_w: i32,
    scroll: usize,
    visible_rows: usize,
    total_items: usize,
) -> Option<usize> {
    let end = (scroll + visible_rows).min(total_items);
    for i in scroll..end {
        let row_y = body_y + (i - scroll) as i32 * ROW_STEP as i32;
        let rect = sdl2::rect::Rect::new(body_x, row_y, body_w as u32, ROW_H);
        if rect.contains_point((mouse_x, mouse_y)) {
            return Some(i);
        }
    }
    None
}

pub fn render_text_centered(
    canvas: &mut Canvas<Window>,
    font: &sdl2::ttf::Font<'_, 'static>,
    text: &str,
    y: i32,
    color: Color,
    window_w: u32,
) -> Result<(), String> {
    let surface = font
        .render(text)
        .blended(color)
        .map_err(|e| e.to_string())?;
    let tc = canvas.texture_creator();
    let texture = tc
        .create_texture_from_surface(&surface)
        .map_err(|e| e.to_string())?;
    let q = texture.query();
    let dst = sdl2::rect::Rect::new(
        (window_w as i32 - q.width as i32) / 2,
        y - q.height as i32 / 2,
        q.width,
        q.height,
    );
    canvas
        .copy(&texture, None, Some(dst))
        .map_err(|e| e.to_string())?;
    Ok(())
}
