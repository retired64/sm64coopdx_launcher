use sdl2::render::Canvas;
use sdl2::video::Window;

/// Draw the game logo with breathing animation and drop shadow.
///
/// Returns the bottom Y coordinate of the logo rect (so the caller can
/// position the launch prompt below it).
#[allow(clippy::too_many_arguments)]
pub fn draw_game_logo(
    canvas: &mut Canvas<Window>,
    logo_tex: &mut Option<sdl2::render::Texture>,
    total_time: f64,
    win_w: u32,
    win_h: u32,
    max_logo_w: u32,
    max_logo_h: u32,
    shadow_offset: i32,
) -> Result<Option<i32>, String> {
    let lt = match logo_tex.as_mut() {
        Some(lt) => lt,
        None => return Ok(None),
    };

    let q = lt.query();
    let logo_scale = (max_logo_w as f32 / q.width as f32).min(max_logo_h as f32 / q.height as f32);
    let base_w = q.width as f32 * logo_scale;
    let base_h = q.height as f32 * logo_scale;
    let breath = 1.0 + 0.03 * (total_time as f32 * 0.8).sin();
    let lw = (base_w * breath) as i32;
    let lh = (base_h * breath) as i32;
    let lx = (win_w as i32 - lw) / 2;
    let ly = (win_h as i32 - lh) / 2;

    // Shadow
    lt.set_color_mod(0, 0, 0);
    lt.set_alpha_mod(102);
    canvas
        .copy(
            lt,
            None,
            Some(sdl2::rect::Rect::new(
                lx + shadow_offset,
                ly + shadow_offset,
                lw as u32,
                lh as u32,
            )),
        )
        .map_err(|e| e.to_string())?;

    // Logo
    lt.set_color_mod(255, 255, 255);
    lt.set_alpha_mod(255);
    canvas
        .copy(
            lt,
            None,
            Some(sdl2::rect::Rect::new(lx, ly, lw as u32, lh as u32)),
        )
        .map_err(|e| e.to_string())?;

    Ok(Some(ly + lh))
}
