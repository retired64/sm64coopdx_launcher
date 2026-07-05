use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::video::Window;

use crate::config::{
    FADE_DURATION_MS, PROGRESS_B, PROGRESS_BAR_H, PROGRESS_BAR_W, PROGRESS_G, PROGRESS_R,
    SPLASH_MIN_MS, WINDOW_H, WINDOW_W,
};
use crate::ui::common;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashTransition {
    None,
    ToGame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FadePhase {
    FadingIn,
    Loading,
    Holding,
    FadingOut,
}

pub struct SplashState {
    start_time_ms: f64,
    progress: f32,
    loaded: bool,
    fade_phase: FadePhase,
}

impl SplashState {
    pub fn new() -> Self {
        Self {
            start_time_ms: 0.0,
            progress: 0.0,
            loaded: false,
            fade_phase: FadePhase::FadingIn,
        }
    }

    pub fn set_start(&mut self, time_ms: f64) {
        self.start_time_ms = time_ms;
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    pub fn mark_loaded(&mut self) {
        self.loaded = true;
        self.progress = 1.0;
    }

    pub fn update(&mut self, now_ms: f64) -> SplashTransition {
        let elapsed = now_ms - self.start_time_ms;
        let fade_ms = FADE_DURATION_MS as f64;

        if elapsed < fade_ms {
            self.fade_phase = FadePhase::FadingIn;
        } else if !self.loaded {
            self.fade_phase = FadePhase::Loading;
        } else if elapsed < SPLASH_MIN_MS as f64 + fade_ms {
            self.fade_phase = FadePhase::Holding;
        } else if elapsed < SPLASH_MIN_MS as f64 + fade_ms * 2.0 {
            self.fade_phase = FadePhase::FadingOut;
        } else {
            return SplashTransition::ToGame;
        }

        SplashTransition::None
    }

    pub fn fade_alpha(&self, now_ms: f64) -> u8 {
        let elapsed = now_ms - self.start_time_ms;
        let fade_ms = FADE_DURATION_MS as f64;

        match self.fade_phase {
            FadePhase::FadingIn => ((elapsed / fade_ms).clamp(0.0, 1.0) * 255.0) as u8,
            FadePhase::Loading | FadePhase::Holding => 255,
            FadePhase::FadingOut => {
                let fade_elapsed = elapsed - (SPLASH_MIN_MS as f64 + fade_ms);
                ((1.0 - (fade_elapsed / fade_ms).clamp(0.0, 1.0)) * 255.0) as u8
            }
        }
    }

    pub fn render(
        &self,
        canvas: &mut Canvas<Window>,
        splash_tex: &mut sdl2::render::Texture,
        font: &sdl2::ttf::Font<'_, 'static>,
        now_ms: f64,
    ) -> Result<(), String> {
        let alpha = self.fade_alpha(now_ms);
        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas.clear();

        splash_tex.set_alpha_mod(alpha);
        let q = splash_tex.query();
        let w_ratio = WINDOW_W as f32 / q.width as f32;
        let h_ratio = WINDOW_H as f32 / q.height as f32;
        let scale = w_ratio.min(h_ratio);
        let dw = (q.width as f32 * scale) as u32;
        let dh = (q.height as f32 * scale) as u32;
        let dst = sdl2::rect::Rect::new(
            (WINDOW_W as i32 - dw as i32) / 2,
            (WINDOW_H as i32 - dh as i32) / 2,
            dw,
            dh,
        );
        canvas
            .copy(splash_tex, None, Some(dst))
            .map_err(|e| e.to_string())?;

        if self.fade_phase != FadePhase::FadingOut || alpha > 30 {
            let progress_bar_y = (WINDOW_H as f32 * 0.88) as i32;
            let bar_x = ((WINDOW_W - PROGRESS_BAR_W) / 2) as i32;

            canvas.set_draw_color(Color::RGB(30, 20, 70));
            canvas
                .fill_rect(sdl2::rect::Rect::new(
                    bar_x,
                    progress_bar_y,
                    PROGRESS_BAR_W,
                    PROGRESS_BAR_H,
                ))
                .map_err(|e| e.to_string())?;

            let fill_w = (PROGRESS_BAR_W as f32 * self.progress) as u32;
            if fill_w > 0 {
                canvas.set_draw_color(Color::RGB(PROGRESS_R, PROGRESS_G, PROGRESS_B));
                canvas
                    .fill_rect(sdl2::rect::Rect::new(
                        bar_x,
                        progress_bar_y,
                        fill_w,
                        PROGRESS_BAR_H,
                    ))
                    .map_err(|e| e.to_string())?;
            }

            let label_color = Color::RGB(200, 180, 240);
            let pct = (self.progress * 100.0) as u32;
            let label = format!("Loading... {pct}%");
            common::render_text_centered(
                canvas,
                font,
                &label,
                progress_bar_y - 24,
                label_color,
                WINDOW_W,
            )?;
        }

        Ok(())
    }
}
