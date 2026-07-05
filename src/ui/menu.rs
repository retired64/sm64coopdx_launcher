use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::video::Window;

const BTN_W: u32 = 272;
const BTN_H: u32 = 62;
const BTN_GAP: i32 = 8;
const PANEL_PAD_X: i32 = 20;
const PANEL_PAD_Y: i32 = 20;
const PANEL_MARGIN: i32 = 24;
const ARC_MAX: i32 = 56;
const ARC_STEP: i32 = 28;

pub const DOT_SIZE: u16 = 18;
pub const MENU_ITEM_COUNT: usize = 5;

const HIGHLIGHT_W: u32 = 4;
const HIGHLIGHT_COLOR: Color = Color::RGB(160, 100, 240);

#[allow(dead_code)]
const PANEL_BG: Color = Color::RGBA(10, 5, 30, 180);
#[allow(dead_code)]
const PANEL_BORDER: Color = Color::RGBA(120, 70, 220, 100);

pub struct ArcMenuItem {
    pub label: &'static str,
    pub icon: &'static str,
}

pub static MENU_ITEMS: [ArcMenuItem; MENU_ITEM_COUNT] = [
    ArcMenuItem {
        label: "Mod Manager",
        icon: "*",
    },
    ArcMenuItem {
        label: "DynOS Packs",
        icon: "#",
    },
    ArcMenuItem {
        label: "Network",
        icon: "~",
    },
    ArcMenuItem {
        label: "Download Mods",
        icon: "v",
    },
    ArcMenuItem {
        label: "Settings",
        icon: "=",
    },
];

fn panel_dimensions(win_w: u32, win_h: u32) -> (i32, i32, u32, u32) {
    let total_h = MENU_ITEM_COUNT as i32 * BTN_H as i32
        + (MENU_ITEM_COUNT - 1) as i32 * BTN_GAP
        + PANEL_PAD_Y * 2;
    let panel_w = PANEL_PAD_X as u32 * 2 + BTN_W + ARC_MAX as u32;
    let panel_h = total_h as u32;
    let panel_x = win_w as i32 - panel_w as i32 - PANEL_MARGIN;
    let panel_y = (win_h as i32 - panel_h as i32) / 2;
    (panel_x, panel_y, panel_w, panel_h)
}

fn button_arc_x(panel_x: i32, btn_index: usize) -> i32 {
    let center = (MENU_ITEM_COUNT - 1) as f32 / 2.0;
    let dist = (btn_index as f32 - center).abs() as i32;
    panel_x + PANEL_PAD_X + ARC_MAX - ARC_STEP * dist
}

fn button_y(panel_y: i32, btn_index: usize) -> i32 {
    panel_y + PANEL_PAD_Y + btn_index as i32 * (BTN_H as i32 + BTN_GAP)
}

/// Public helper: get the Y coordinate of a menu button (used for easing target).
pub fn menu_target_y(win_h: u32, btn_index: usize) -> f64 {
    let (_, py, _, _) = panel_dimensions(0, win_h);
    button_y(py, btn_index) as f64
}

/// Public helper: hit-test which menu button is under (mouse_x, mouse_y).
pub fn hit_test_menu_button(mouse_x: i32, mouse_y: i32, win_w: u32, win_h: u32) -> Option<usize> {
    let (px, py, _, _) = panel_dimensions(win_w, win_h);
    for i in 0..MENU_ITEM_COUNT {
        let bx = button_arc_x(px, i);
        let by = button_y(py, i);
        let rect = sdl2::rect::Rect::new(bx, by, BTN_W, BTN_H);
        if rect.contains_point((mouse_x, mouse_y)) {
            return Some(i);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn render_arc_menu(
    canvas: &mut Canvas<Window>,
    btn_textures: &mut [Option<sdl2::render::Texture>; MENU_ITEM_COUNT],
    btn_w: &[u32; MENU_ITEM_COUNT],
    btn_h: &[u32; MENU_ITEM_COUNT],
    dot_tex: &mut Option<sdl2::render::Texture>,
    dot_w: u32,
    dot_h: u32,
    selected: usize,
    highlight_y: f64,
    dim: f32,
    win_w: u32,
    win_h: u32,
) -> Result<(), String> {
    let dim_alpha = (255.0 * dim) as u8;
    let (px, py, pw, ph) = panel_dimensions(win_w, win_h);

    // Panel background (dim alpha baked into color)
    let bg_dim = Color::RGBA(10, 5, 30, (180.0 * dim) as u8);
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(bg_dim);
    canvas
        .fill_rect(sdl2::rect::Rect::new(px, py, pw, ph))
        .map_err(|e| e.to_string())?;
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    // 1px rounded border (step 30)
    let border_dim = Color::RGBA(120, 70, 220, (100.0 * dim) as u8);
    canvas.set_draw_color(border_dim);
    crate::ui::panel::draw_rounded_rect_outline(
        canvas, px, py, pw as i32, ph as i32, 16, // CORNER_RADIUS
    )?;

    // Animated highlight bar
    let hl_y = highlight_y as i32;
    let hl_x = button_arc_x(px, selected);
    let hl_dim = Color::RGBA(
        HIGHLIGHT_COLOR.r,
        HIGHLIGHT_COLOR.g,
        HIGHLIGHT_COLOR.b,
        (255.0 * dim) as u8,
    );
    canvas.set_draw_color(hl_dim);
    canvas
        .fill_rect(sdl2::rect::Rect::new(hl_x, hl_y, HIGHLIGHT_W, BTN_H))
        .map_err(|e| e.to_string())?;

    for i in 0..MENU_ITEM_COUNT {
        let bx = button_arc_x(px, i);
        let by = button_y(py, i);

        // Button label texture
        if let Some(bt) = btn_textures[i].as_mut() {
            bt.set_alpha_mod(dim_alpha);
            let tw = btn_w[i];
            let th = btn_h[i];
            let tx = bx + 24;
            let ty = by + (BTN_H as i32 - th as i32) / 2;
            canvas
                .copy(bt, None, Some(sdl2::rect::Rect::new(tx, ty, tw, th)))
                .map_err(|e| e.to_string())?;
            bt.set_alpha_mod(255);
        }

        // Selection dot
        if let Some(dt) = dot_tex.as_mut() {
            let dy = by + (BTN_H as i32 - dot_h as i32) / 2;
            let dx = bx - dot_w as i32 - 12;
            let dot_alpha = if i == selected {
                dim_alpha
            } else {
                (128.0 * dim) as u8
            };
            dt.set_alpha_mod(dot_alpha);
            if i == selected {
                dt.set_color_mod(160, 100, 240);
            } else {
                dt.set_color_mod(100, 70, 180);
            }
            canvas
                .copy(dt, None, Some(sdl2::rect::Rect::new(dx, dy, dot_w, dot_h)))
                .map_err(|e| e.to_string())?;
            dt.set_alpha_mod(255);
        }
    }

    Ok(())
}
