use sdl2::render::Canvas;
use sdl2::video::Window;

/// Draw the vinyl disc with rotation, pulse animation, track name, and
/// track counter. All text textures must be pre‑rendered by the caller.
#[allow(clippy::too_many_arguments)]
pub fn draw_vinyl_player(
    canvas: &mut Canvas<Window>,
    vinyl_tex: &mut Option<sdl2::render::Texture>,
    total_time: f64,
    vinyl_margin: i32,
    vinyl_rot_speed: f64,
    track_name_tex: &mut Option<sdl2::render::Texture>,
    track_name_w: u32,
    track_name_h: u32,
    track_counter_tex: &mut Option<sdl2::render::Texture>,
    track_counter_w: u32,
    track_counter_h: u32,
) -> Result<(), String> {
    let vt = match vinyl_tex.as_mut() {
        Some(vt) => vt,
        None => return Ok(()),
    };

    let q = vt.query();
    let pulse = 1.0 + 0.03 * (total_time as f32 * 3.0).sin();
    let scaled_w = (q.width as f32 * pulse) as u32;
    let scaled_h = (q.height as f32 * pulse) as u32;
    let angle = (total_time * vinyl_rot_speed) % 360.0;

    let dst = sdl2::rect::Rect::new(vinyl_margin, vinyl_margin, scaled_w, scaled_h);
    let center = sdl2::rect::Point::new((scaled_w / 2) as i32, (scaled_h / 2) as i32);
    canvas
        .copy_ex(vt, None, Some(dst), angle, Some(center), false, false)
        .map_err(|e| e.to_string())?;

    let text_h = vinyl_margin + scaled_h as i32 + 10;

    // Track name (white text with dark shadow)
    if let Some(tn) = track_name_tex.as_mut() {
        let tx = vinyl_margin;
        let ty = text_h;
        tn.set_color_mod(0, 0, 0);
        tn.set_alpha_mod(100);
        canvas
            .copy(
                tn,
                None,
                Some(sdl2::rect::Rect::new(
                    tx + 1,
                    ty + 1,
                    track_name_w,
                    track_name_h,
                )),
            )
            .map_err(|e| e.to_string())?;
        tn.set_color_mod(255, 255, 255);
        tn.set_alpha_mod(255);
        canvas
            .copy(
                tn,
                None,
                Some(sdl2::rect::Rect::new(tx, ty, track_name_w, track_name_h)),
            )
            .map_err(|e| e.to_string())?;
    }

    // Counter (purple text with dark shadow)
    if let Some(tc) = track_counter_tex.as_mut() {
        let cy = text_h + track_name_h as i32 + 4;
        tc.set_color_mod(0, 0, 0);
        tc.set_alpha_mod(100);
        canvas
            .copy(
                tc,
                None,
                Some(sdl2::rect::Rect::new(
                    vinyl_margin + 1,
                    cy + 1,
                    track_counter_w,
                    track_counter_h,
                )),
            )
            .map_err(|e| e.to_string())?;
        tc.set_color_mod(200, 160, 255);
        tc.set_alpha_mod(255);
        canvas
            .copy(
                tc,
                None,
                Some(sdl2::rect::Rect::new(
                    vinyl_margin,
                    cy,
                    track_counter_w,
                    track_counter_h,
                )),
            )
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}
