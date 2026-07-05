use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod assets;
mod config;
mod error;
mod game;
mod managers;
mod ui;

use sdl2::image::LoadSurface;

use config::{
    BACKGROUND_PNG, BG_B, BG_G, BG_R, FADE_DURATION_MS, FONT_PATH, FONT_SIZE_DEFAULT,
    FONT_SIZE_MEDIUM, FONT_SIZE_SMALL, ICON_PNG, LOGO_PNG, NAV_WAV, OGG_DIR, SPLASH_PNG,
    SPLASH_WAV, VINYL_PNG, WINDOW_H, WINDOW_W,
};
use error::LauncherResult;
use managers::download_manager::DownloadProgress;
use managers::{download_manager, dynos_manager, mod_manager, network_manager, profile_manager};
use ui::common::{self, ItemType, UiItem};
use ui::download_browser::{DlFocusMode, DownloadBrowserState};
use ui::keyboard::VirtualKeyboard;
use ui::network_form::NetworkFormState;
use ui::panel::{PanelState, SubScreenType, subscreen_for_menu_index};

/// Gamepad -> logical action mapping (testable pure function, no SDL2 dependency).
/// Used by JoyButtonDown, JoyHatMotion, AND debug keyboard keys — all three
/// dispatch through the same pure functions, so the unit tests cover real
/// event‑loop behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpAction {
    Confirm,  // A / J → ENTER
    Cancel,   // B / K → ESC
    Activate, // X / L → Space
    NavUp,
    NavDown,
    NavLeft,
    NavRight,
    PagePrev, // L1 / U
    PageNext, // R1 / O
    Menu,     // Start / I → TAB
    None,
}

fn joy_button_to_action(button: u8) -> GpAction {
    match button {
        0 => GpAction::Confirm,
        1 => GpAction::Cancel,
        2 => GpAction::Activate,
        4 => GpAction::PagePrev,
        5 => GpAction::PageNext,
        7 => GpAction::Menu,
        _ => GpAction::None,
    }
}

fn hat_to_action(state: sdl2::joystick::HatState) -> GpAction {
    use sdl2::joystick::HatState;
    match state {
        HatState::Up | HatState::RightUp | HatState::LeftUp => GpAction::NavUp,
        HatState::Down | HatState::RightDown | HatState::LeftDown => GpAction::NavDown,
        HatState::Left => GpAction::NavLeft,
        HatState::Right => GpAction::NavRight,
        _ => GpAction::None,
    }
}

#[cfg(test)]
mod gp_tests {
    use super::*;
    use sdl2::joystick::HatState;

    #[test]
    fn joy_a_is_confirm() {
        assert_eq!(joy_button_to_action(0), GpAction::Confirm);
    }
    #[test]
    fn joy_b_is_cancel() {
        assert_eq!(joy_button_to_action(1), GpAction::Cancel);
    }
    #[test]
    fn joy_x_is_activate() {
        assert_eq!(joy_button_to_action(2), GpAction::Activate);
    }
    #[test]
    fn joy_unknown_is_none() {
        assert_eq!(joy_button_to_action(99), GpAction::None);
    }
    #[test]
    fn hat_up_is_nav_up() {
        assert_eq!(hat_to_action(HatState::Up), GpAction::NavUp);
    }
    #[test]
    fn hat_left_is_nav_left() {
        assert_eq!(hat_to_action(HatState::Left), GpAction::NavLeft);
    }
    #[test]
    fn hat_centered_is_none() {
        assert_eq!(hat_to_action(HatState::Centered), GpAction::None);
    }
    #[test]
    fn joy_l1_is_page_prev() {
        assert_eq!(joy_button_to_action(4), GpAction::PagePrev);
    }
    #[test]
    fn joy_r1_is_page_next() {
        assert_eq!(joy_button_to_action(5), GpAction::PageNext);
    }
    #[test]
    fn joy_start_is_menu() {
        assert_eq!(joy_button_to_action(7), GpAction::Menu);
    }
}

use ui::splash::SplashState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppState {
    Splash,
    Game,
    Menu,
    SubScreen,
    Launching,
    ShuttingDown,
}

// Decision: assets are loaded sequentially on the main thread during the
// splash screen, NOT in a background thread. AGENTS.md concurrency rules
// apply to user‑interactive states (Game, Menu, SubScreen). During splash
// there is no user input — loading in‑line gives identical visual feedback
// (progress bar) without the complexity of Arc<Mutex<Vec<u8>>> for textures.
//
// unsafe_textures feature is used so Texture is 'static — avoids borrow‑ck
// issues with canvas vs texture lifetimes. This is the standard approach in
// production SDL2 Rust apps (the renderer owns the internal C texture).

const MAX_LOGO_W: u32 = 900;
const MAX_LOGO_H: u32 = 450;
const SHADOW_OFFSET: i32 = 8;
const PROMPT_GAP: i32 = 60;
const VINYL_MARGIN: i32 = 30;
const VINYL_ROT_SPEED: f64 = 90.0;
const MUSIC_VOLUME: i32 = 28; // 0.22 × 128
const TRACK_COOLDOWN_S: f64 = 1.0;

const CREATOR_FONT_SIZE: u16 = 20;
const CREATOR_MARGIN: i32 = 32;
const CREATOR_HOVER_OFFSET: i32 = 4;
const CREATOR_URL: &str = "https://www.youtube.com/@Retired64";
const RAINBOW_COLORS: [(u8, u8, u8); 6] = [
    (255, 50, 50),
    (255, 220, 50),
    (50, 255, 100),
    (50, 150, 255),
    (255, 150, 50),
    (200, 50, 255),
];

// ── Music auto‑advance flag ──
//
// SDL2_mixer::Music::hook_finished fires from the audio thread, NOT the main
// thread. Per AGENTS.md §'Modelo de concurrencia', a non‑main thread MUST NOT
// mutate UI state directly. Instead the callback sets this AtomicBool; the main
// loop checks it each frame and performs the actual track change safely.
static TRACK_FINISHED: AtomicBool = AtomicBool::new(false);

/// Set by the ctrlc signal handler — main loop checks this each frame.
static SHUTDOWN_REQUEST: AtomicBool = AtomicBool::new(false);

fn on_music_finished() {
    TRACK_FINISHED.store(true, Ordering::Release);
}

/// Open a URL in the system browser using std::process::Command.
///
/// We do NOT use the `open` crate (not in spec §23 Cargo.toml). Instead we
/// shell out to platform‑specific commands, which is zero‑cost in deps and
/// avoids pulling a new transitive dependency tree for a single URL open.
fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}

fn bg_cover_rect(tex_w: u32, tex_h: u32, win_w: u32, win_h: u32) -> sdl2::rect::Rect {
    let w_ratio = win_w as f32 / tex_w as f32;
    let h_ratio = win_h as f32 / tex_h as f32;
    let scale = w_ratio.max(h_ratio);
    let dw = (tex_w as f32 * scale) as u32;
    let dh = (tex_h as f32 * scale) as u32;
    sdl2::rect::Rect::new(
        (win_w as i32 - dw as i32) / 2,
        (win_h as i32 - dh as i32) / 2,
        dw,
        dh,
    )
}

/// Dispatch config write to the correct manager based on active SubScreen.
fn write_active_config(active: &Option<SubScreenType>, config_path: &Path, names: &[String]) {
    let result = match active {
        Some(SubScreenType::ModManager) => mod_manager::write_enabled_mods(config_path, names),
        Some(SubScreenType::DynosPacks) => dynos_manager::write_enabled_packs(config_path, names),
        _ => return,
    };
    result.unwrap_or_else(|e| {
        log::error!("Failed to write config: {e}");
    });
}

/// Calculate shutdown progress percentage from B‑button hold duration.
/// 0ms → 0%, 5000ms → 100%.
#[must_use]
pub fn shutdown_progress(hold_ms: u64) -> u32 {
    ((hold_ms as f64 / 5000.0).clamp(0.0, 1.0) * 100.0) as u32
}

/// Auto‑scroll helper for download browser (same logic as common::ensure_selection_visible).
fn ensure_dl_visible(selected: usize, scroll: usize, _total: usize, visible: usize) -> usize {
    if selected < scroll {
        selected
    } else if selected >= scroll + visible {
        selected - visible + 1
    } else {
        scroll
    }
}

fn main() -> LauncherResult<()> {
    env_logger::init();

    // Parse --game-path CLI argument (if present)
    let cli_game_path: Option<String> = {
        let args: Vec<String> = std::env::args().collect();
        let mut path = None;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--game-path" && i + 1 < args.len() {
                path = Some(args[i + 1].clone());
                break;
            }
            i += 1;
        }
        path
    };

    log::info!("Initializing SDL2 subsystems");

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let ttf = sdl2::ttf::init()?;
    let _image = sdl2::image::init(sdl2::image::InitFlag::PNG)?;
    let _audio = sdl.audio()?;

    sdl2::mixer::open_audio(44100, sdl2::mixer::AUDIO_S16LSB, 2, 512)?;
    sdl2::mixer::allocate_channels(8);
    log::info!("SDL2_mixer initialized");

    // Joystick for gamepad B‑button hold shutdown
    let joystick = sdl.joystick()?;
    joystick.set_event_state(true);

    // Ctrl+C graceful shutdown — only sets atomic flag (same pattern as hook_finished)
    ctrlc::set_handler(|| {
        log::info!("Ctrl+C received — requesting shutdown");
        SHUTDOWN_REQUEST.store(true, Ordering::Release);
    })
    .map_err(|e| format!("Failed to set ctrlc handler: {e}"))?;

    let target_hz: u32 = if let Ok(mode) = video.current_display_mode(0) {
        let hz = mode.refresh_rate.max(30) as u32;
        log::info!("Display mode: {}×{} @ {hz}Hz", mode.w, mode.h);
        hz
    } else {
        log::warn!("Could not query display mode, defaulting to 60Hz");
        60
    };
    let target_frame_time = Duration::from_secs_f64(1.0 / target_hz as f64);

    let window = video
        .window("SM64 Coop DX Launcher", WINDOW_W, WINDOW_H)
        .resizable()
        .build()?;

    let mut canvas = window.into_canvas().accelerated().present_vsync().build()?;
    canvas.set_draw_color(sdl2::pixels::Color::RGB(BG_R, BG_G, BG_B));

    let splash_surface = sdl2::surface::Surface::from_file(&*SPLASH_PNG)
        .map_err(|e| format!("Failed to load splash.png: {e}"))?;
    let mut splash_tex = canvas
        .texture_creator()
        .create_texture_from_surface(&splash_surface)
        .map_err(|e| format!("Failed to create splash texture: {e}"))?;
    log::info!("Splash texture loaded");

    let font_path = FONT_PATH.as_path();
    let mut font_cache = assets::FontCache::new(
        &ttf,
        font_path,
        &[
            FONT_SIZE_SMALL,
            CREATOR_FONT_SIZE,
            FONT_SIZE_MEDIUM,
            FONT_SIZE_DEFAULT,
        ],
    )?;

    let mut _nav_sound: Option<sdl2::mixer::Chunk> = None;
    let mut _splash_sound_chunk: Option<sdl2::mixer::Chunk> = None;
    let mut vinyl_tex: Option<sdl2::render::Texture> = None;
    let mut logo_tex: Option<sdl2::render::Texture> = None;
    let mut background_tex: Option<sdl2::render::Texture> = None;
    let mut _icon_surface: Option<sdl2::surface::Surface<'_>> = None;
    let mut music_track_paths: Vec<PathBuf> = Vec::new();

    // Music / track state
    let mut _current_music: Option<sdl2::mixer::Music<'_>> = None;
    let mut current_track_index: usize = 0;
    let mut last_track_change: f64 = 0.0;
    let mut requested_track_change: Option<isize> = None;

    // Game launch state
    let game_exe = game::resolve_game_path(cli_game_path.as_deref(), None);
    let mut launch_start = Instant::now();
    let mut game_launched: bool = false;
    let mut launch_tex: Option<sdl2::render::Texture> = None;
    let mut launch_tex_w: u32 = 0;
    let mut launch_tex_h: u32 = 0;
    let mut launch_error: Option<String> = None;

    // Shutdown B‑hold state (gamepad button 1 or debug key K)
    let mut b_hold_start: Option<Instant> = None;
    let mut shutdown_cancel_start: Option<Instant> = None;

    // Cached rendering data
    let mut cached_win_size: (u32, u32) = (WINDOW_W, WINDOW_H);
    let mut cached_bg_rect: Option<sdl2::rect::Rect> = None;
    let mut prompt_tex: Option<sdl2::render::Texture> = None;
    let mut prompt_tex_w: u32 = 0;
    let mut prompt_tex_h: u32 = 0;
    let mut track_name_tex: Option<sdl2::render::Texture> = None;
    let mut track_name_w: u32 = 0;
    let mut track_name_h: u32 = 0;
    let mut track_counter_tex: Option<sdl2::render::Texture> = None;
    let mut track_counter_w: u32 = 0;
    let mut track_counter_h: u32 = 0;

    // Creator button state
    let mut creator_tex: Option<sdl2::render::Texture> = None;
    let mut creator_tex_w: u32 = 0;
    let mut creator_tex_h: u32 = 0;
    let mut creator_rect: sdl2::rect::Rect = sdl2::rect::Rect::new(0, 0, 0, 0);
    let mut controls_hint_tex: Option<sdl2::render::Texture> = None;
    let mut controls_hint_w: u32 = 0;
    let mut controls_hint_h: u32 = 0;
    let mut mouse_x: i32 = 0;
    let mut mouse_y: i32 = 0;
    let mut hovered_row: Option<usize> = None;

    // Fullscreen toggle state
    let mut is_fullscreen: bool = false;

    // Sub‑screen panel state
    let mut panel_state = PanelState::new(WINDOW_W);
    let mut menu_dim: f32 = 1.0;

    // XDG data directory (mods, config, profiles — per spec §22)
    let data_dir = dirs::data_dir()
        .ok_or("XDG data dir not available")?
        .join("sm64coopdx");
    let config_path = data_dir.join("sm64config.txt");

    // Item selector state (reused across ModManager, Dynos, Profiles, etc.)
    let mut real_items: Vec<UiItem> = Vec::new();
    let mut item_tex: Vec<Option<sdl2::render::Texture>> = Vec::new();
    let mut item_tex_w: Vec<u32> = Vec::new();
    let mut item_tex_h: Vec<u32> = Vec::new();
    let mut item_selected: usize = 0;
    let mut item_scroll: usize = 0;
    // Pre‑rendered toggle icon textures
    let mut icon_check_tex: Option<sdl2::render::Texture> = None;
    let mut icon_cross_tex: Option<sdl2::render::Texture> = None;
    let mut icon_plus_tex: Option<sdl2::render::Texture> = None;
    let mut icon_w: u32 = 0;
    let mut icon_h: u32 = 0;
    let mut needs_item_setup: bool = false;

    // Network form state
    let mut network_state = NetworkFormState::new(network_manager::NetworkConfig::default());
    let mut network_edit_buffer: String = String::new();
    let mut virtual_kb = VirtualKeyboard::new();

    // Profile editing state (step 26)
    let mut profile_edit_buffer: String = String::new();
    let mut prof_edit_action: Option<String> = None; // "create" or "rename"
    let mut prof_edit_target: Option<String> = None; // profile being renamed
    let mut prof_delete_hold_start: Option<Instant> = None;
    let mut prof_detail_profile: Option<String> = None;

    // Download browser state (lazy‑loaded on first open)
    let mut dl_state = DownloadBrowserState::new();
    let dl_progress: Arc<Mutex<DownloadProgress>> =
        Arc::new(Mutex::new(DownloadProgress::new(None)));
    let dl_cancel = Arc::new(AtomicBool::new(false));
    let mut dl_download_active = false;
    let mut dl_download_handle: Option<std::thread::JoinHandle<()>> = None;

    // Arc menu state
    let mut menu_btn_tex: [Option<sdl2::render::Texture>; 5] = Default::default();
    let mut menu_btn_w: [u32; 5] = [0; 5];
    let mut menu_btn_h: [u32; 5] = [0; 5];
    let mut menu_dot_tex: Option<sdl2::render::Texture> = None;
    let mut menu_dot_w: u32 = 0;
    let mut menu_dot_h: u32 = 0;
    let mut menu_selected: usize = 0;
    let mut highlight_y: f64 = 0.0;

    let mut app_state = AppState::Splash;
    let mut splash = SplashState::new();
    let splash_start = Instant::now();

    const PHASE_DELTAS: [f32; 7] = [0.12, 0.10, 0.16, 0.19, 0.10, 0.10, 0.18];
    let mut load_phase: usize = 0;
    let mut current_progress: f32 = 0.05;
    let mut loading_finished = false;

    let mut event_pump = sdl.event_pump()?;
    let fps_start = Instant::now();
    let mut last_frame = Instant::now();
    let mut fps_timer = Instant::now();
    let mut frame_count: u64 = 0;

    log::info!("Entering main loop");

    'main: loop {
        let frame_start = Instant::now();
        let dt = (frame_start - last_frame).as_secs_f64().min(0.1);
        last_frame = frame_start;

        for event in event_pump.poll_iter() {
            match event {
                sdl2::event::Event::Quit { .. } => break 'main,
                sdl2::event::Event::KeyDown {
                    keycode: Some(key), ..
                } => match key {
                    sdl2::keyboard::Keycode::Escape => {
                        if dl_download_active {
                            // Cancel active download
                            dl_cancel.store(true, Ordering::Relaxed);
                        } else if app_state == AppState::ShuttingDown {
                            shutdown_cancel_start = Some(Instant::now());
                        } else if dl_state.author_dropdown_open {
                            dl_state.author_dropdown_open = false;
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::ProfileDetail)
                        {
                            panel_state.close(cached_win_size.0);
                            let profiles_dir = data_dir.join("profiles");
                            let active_name =
                                profile_manager::get_active_profile_name(&profiles_dir)
                                    .unwrap_or_else(|| "Default".into());
                            if let Ok(items) =
                                profile_manager::scan_profiles(&profiles_dir, &active_name)
                            {
                                real_items = items;
                                let count = real_items.len().saturating_sub(1);
                                panel_state.header_extra = Some(format!("{count} profiles"));
                            }
                            panel_state.footer_hint = Some(
                                "ENTER: Config  |  Space: Activate  |  N: New  |  R: Rename  |  DEL: Delete  |  ESC: Back"
                                    .to_string(),
                            );
                            panel_state.invalidate_cache();
                            panel_state.active = Some(SubScreenType::Profiles);
                            needs_item_setup = true;
                            app_state = AppState::SubScreen;
                        } else if dl_state.search_active {
                            // Clear search first, panel close on second ESC
                            dl_state.search_text.clear();
                            dl_state.search_active = false;
                        } else if app_state == AppState::SubScreen {
                            panel_state.close(cached_win_size.0);
                            app_state = AppState::Game;
                        } else if app_state == AppState::Menu {
                            app_state = AppState::Game;
                        } else {
                            break 'main;
                        }
                    }
                    sdl2::keyboard::Keycode::Tab => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.focus_mode = match dl_state.focus_mode {
                                DlFocusMode::ModList => DlFocusMode::TagChips,
                                DlFocusMode::TagChips => DlFocusMode::AuthorInput,
                                DlFocusMode::AuthorInput => DlFocusMode::ModList,
                            };
                            dl_state.author_dropdown_open = false;
                        } else {
                            app_state = if app_state == AppState::Menu {
                                AppState::Game
                            } else if app_state == AppState::Game {
                                AppState::Menu
                            } else {
                                app_state
                            };
                        }
                    }
                    sdl2::keyboard::Keycode::W | sdl2::keyboard::Keycode::Up => {
                        if virtual_kb.active {
                            virtual_kb.move_up();
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::AuthorInput
                            && dl_state.author_dropdown_open
                            && !dl_state.author_filtered.is_empty()
                        {
                            dl_state.author_list_selected =
                                dl_state.author_list_selected.saturating_sub(1);
                        } else if app_state == AppState::Menu {
                            if menu_selected == 0 {
                                menu_selected = ui::menu::MENU_ITEM_COUNT - 1;
                            } else {
                                menu_selected -= 1;
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Network)
                        {
                            let vis = network_state.visible_fields();
                            if !vis.is_empty() {
                                let current = network_state.selected_field;
                                network_state.selected_field = if current == 0 {
                                    vis.len() - 1
                                } else {
                                    current - 1
                                };
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::ModList
                        {
                            dl_state.selected = dl_state.selected.saturating_sub(1);
                            dl_state.scroll = ensure_dl_visible(
                                dl_state.selected,
                                dl_state.scroll,
                                dl_state.total(),
                                common::DL_VISIBLE_ROWS,
                            );
                        } else if app_state == AppState::SubScreen {
                            item_selected = item_selected.saturating_sub(1);
                            item_scroll = common::ensure_selection_visible(
                                item_selected,
                                item_scroll,
                                common::DEFAULT_VISIBLE_ROWS,
                            );
                        } else {
                            requested_track_change = Some(-1);
                        }
                    }
                    sdl2::keyboard::Keycode::S | sdl2::keyboard::Keycode::Down => {
                        if virtual_kb.active {
                            virtual_kb.move_down();
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::AuthorInput
                            && dl_state.author_dropdown_open
                            && !dl_state.author_filtered.is_empty()
                        {
                            let max = dl_state.author_filtered.len().saturating_sub(1);
                            if dl_state.author_list_selected < max {
                                dl_state.author_list_selected += 1;
                            }
                        } else if app_state == AppState::Menu {
                            if menu_selected >= ui::menu::MENU_ITEM_COUNT - 1 {
                                menu_selected = 0;
                            } else {
                                menu_selected += 1;
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Network)
                        {
                            let vis = network_state.visible_fields();
                            if !vis.is_empty() {
                                let current = network_state.selected_field;
                                network_state.selected_field = if current >= vis.len() - 1 {
                                    0
                                } else {
                                    current + 1
                                };
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::ModList
                            && dl_state.total() > 0
                        {
                            let max = dl_state.total().saturating_sub(1);
                            if dl_state.selected < max {
                                dl_state.selected += 1;
                            }
                            dl_state.scroll = ensure_dl_visible(
                                dl_state.selected,
                                dl_state.scroll,
                                dl_state.total(),
                                common::DL_VISIBLE_ROWS,
                            );
                        } else if app_state == AppState::SubScreen {
                            let max = if real_items.is_empty() {
                                0
                            } else {
                                real_items.len() - 1
                            };
                            if item_selected < max {
                                item_selected += 1;
                            }
                            item_scroll = common::ensure_selection_visible(
                                item_selected,
                                item_scroll,
                                common::DEFAULT_VISIBLE_ROWS,
                            );
                        } else {
                            requested_track_change = Some(1);
                        }
                    }
                    sdl2::keyboard::Keycode::Return | sdl2::keyboard::Keycode::Space => {
                        if virtual_kb.active && prof_edit_action.is_some() {
                            let confirmed = virtual_kb.confirm_if_active(&mut profile_edit_buffer);
                            if confirmed {
                                let profiles_dir = data_dir.join("profiles");
                                let action = prof_edit_action.take().unwrap();
                                virtual_kb.close();
                                if action == "create" {
                                    let name = profile_edit_buffer.trim().to_string();
                                    let parent_cfg = data_dir.join("sm64config.txt");
                                    if let Err(e) = profile_manager::create_profile(
                                        &profiles_dir,
                                        &name,
                                        &parent_cfg,
                                    ) {
                                        log::error!("Create profile failed: {e}");
                                    }
                                } else if action == "rename" {
                                    let old_name = prof_edit_target.take().unwrap_or_default();
                                    let new_name = profile_edit_buffer.trim().to_string();
                                    if let Err(e) = profile_manager::rename_profile(
                                        &profiles_dir,
                                        &old_name,
                                        &new_name,
                                    ) {
                                        log::error!("Rename profile failed: {e}");
                                    }
                                } else if action == "playername" {
                                    let name = prof_detail_profile.clone().unwrap_or_default();
                                    let new_name = profile_edit_buffer.trim().to_string();
                                    if let Ok(config) = profile_manager::update_profile_playername(
                                        &profiles_dir,
                                        &name,
                                        &new_name,
                                    ) {
                                        real_items =
                                            profile_manager::build_profile_detail_items(&config);
                                        panel_state.invalidate_cache();
                                    }
                                }
                                profile_edit_buffer.clear();
                                // Re‑scan profiles
                                let active_name =
                                    profile_manager::get_active_profile_name(&profiles_dir)
                                        .unwrap_or_else(|| "Default".into());
                                if let Ok(items) =
                                    profile_manager::scan_profiles(&profiles_dir, &active_name)
                                {
                                    real_items = items;
                                    let count = real_items.len().saturating_sub(1);
                                    panel_state.header_extra = Some(format!("{count} profiles"));
                                }
                                panel_state.footer_hint = Some(
                                    "ENTER: Config  |  Space: Activate  |  N: New  |  R: Rename  |  DEL: Delete  |  ESC: Back"
                                        .to_string(),
                                );
                                panel_state.invalidate_cache();
                            }
                        } else if virtual_kb.active {
                            let confirmed = virtual_kb.confirm_if_active(&mut network_edit_buffer);
                            if confirmed {
                                network_state.commit_edit(&network_edit_buffer);
                                network_edit_buffer.clear();
                                virtual_kb.close();
                                network_manager::write_network_config(
                                    &config_path,
                                    &network_state.config,
                                )
                                .ok();
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::ModList
                            && dl_state.total() > 0
                            && !dl_download_active
                        {
                            // Start download of selected mod
                            if let Some(entry_idx) = dl_state.entry_idx(dl_state.selected)
                                && let Some(ref db) = dl_state.db
                                && let Some(ref idx) = dl_state.index
                            {
                                let mod_id = &idx.entries[entry_idx].mod_id;
                                if let Some(mod_info) = db.mods.get(mod_id)
                                    && let Some(url) = mod_info.download_urls.first()
                                {
                                    let url = url.clone();
                                    let dest_dir = data_dir.clone();
                                    let mod_id_clone = mod_id.clone();
                                    let progress = dl_progress.clone();
                                    let cancel = dl_cancel.clone();

                                    cancel.store(false, Ordering::Relaxed);
                                    {
                                        let mut p = progress.lock().unwrap();
                                        *p = DownloadProgress::new(None);
                                    }

                                    dl_download_active = true;
                                    log::info!("Starting download: {} from {}", mod_id_clone, url);
                                    // dest_dir is data_dir (mods root).
                                    // Download goes to a temp subdir so ZIP can be
                                    // extracted into data_dir cleanly.
                                    let dl_temp = dest_dir.join("_downloads");
                                    dl_download_handle = Some(std::thread::spawn(move || {
                                        let _ = (|| -> Result<(), String> {
                                            let zip_path = download_manager::download_mod_file(
                                                &url,
                                                &dl_temp,
                                                &mod_id_clone,
                                                &progress,
                                                &cancel,
                                            )?;
                                            // Extract ZIP into dest_dir (mods root)
                                            let extracted = download_manager::extract_mod_zip(
                                                &zip_path, &dest_dir,
                                            )?;
                                            log::info!(
                                                "Extracted {} mod(s) into {}",
                                                extracted.len(),
                                                dest_dir.display()
                                            );
                                            // Update progress with extraction count
                                            if let Ok(mut p) = progress.lock() {
                                                p.extracted = extracted.len() as u32;
                                            }
                                            // Delete the ZIP after successful extraction
                                            let _ = std::fs::remove_file(&zip_path);
                                            Ok(())
                                        })();
                                    }));
                                }
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::TagChips
                            && !dl_state.top_tags.is_empty()
                        {
                            let ci = dl_state.tag_chip_selected;
                            if ci < dl_state.top_tags.len() {
                                let tag = dl_state.top_tags[ci].0.clone();
                                dl_state.toggle_tag(&tag);
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::AuthorInput
                        {
                            if dl_state.author_dropdown_open {
                                // Select highlighted author (or clear if selected)
                                if dl_state.author_filtered.is_empty() {
                                    dl_state.select_author(None);
                                } else if dl_state.author_list_selected
                                    < dl_state.author_filtered.len()
                                {
                                    let author = dl_state.author_filtered
                                        [dl_state.author_list_selected]
                                        .clone();
                                    dl_state.select_author(Some(author));
                                }
                            } else {
                                dl_state.toggle_author_dropdown();
                            }
                        } else if app_state == AppState::Game {
                            // Launch the game
                            if game_exe.exists() {
                                log::info!("Launching game: {}", game_exe.display());
                                launch_error = None;
                                app_state = AppState::Launching;
                                launch_start = Instant::now();
                                game_launched = false;
                                sdl2::mixer::Music::set_volume(3); // 0.02 × 128
                            } else {
                                log::error!("Game binary not found: {}", game_exe.display());
                                launch_error = Some(format!(
                                    "Game binary not found: {}\n\
                                     Configure the game path in ~/.config/sm64coopdx/launcher.toml\n\
                                     or set SM64COOPDX_PATH environment variable.",
                                    game_exe.display()
                                ));
                            }
                        } else if app_state == AppState::Menu {
                            if let Some(st) = subscreen_for_menu_index(menu_selected) {
                                log::info!("Opening sub-screen: {:?}", st);
                                panel_state.open(st, cached_win_size.0);
                                app_state = AppState::SubScreen;
                                needs_item_setup = true;
                                item_selected = 0;
                                item_scroll = 0;
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Network)
                        {
                            // Toggle editing on selected text/int field
                            if network_state.editing_field.is_some() {
                                network_state.commit_edit(&network_edit_buffer);
                                network_edit_buffer.clear();
                                virtual_kb.close();
                                network_manager::write_network_config(
                                    &config_path,
                                    &network_state.config,
                                )
                                .ok();
                            } else {
                                network_state.editing_field = Some(network_state.selected_field);
                                network_edit_buffer.push_str(
                                    &network_state
                                        .field_value_for_index(network_state.selected_field),
                                );
                                virtual_kb.open();
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Profiles)
                            && !real_items.is_empty()
                        {
                            #[allow(clippy::collapsible_if)]
                            if item_selected < real_items.len() {
                                let item = &real_items[item_selected];
                                if item.rel_path == "__new__" {
                                    prof_edit_action = Some("create".into());
                                    prof_edit_target = None;
                                    profile_edit_buffer.clear();
                                    virtual_kb.open();
                                } else {
                                    prof_detail_profile = Some(item.name.clone());
                                    panel_state
                                        .open(SubScreenType::ProfileDetail, cached_win_size.0);
                                    app_state = AppState::SubScreen;
                                    needs_item_setup = true;
                                }
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::ProfileDetail)
                            && !real_items.is_empty()
                        {
                            #[allow(clippy::collapsible_if)]
                            if item_selected < real_items.len() {
                                let item = &real_items[item_selected];
                                if item.item_type == ItemType::Text {
                                    prof_edit_action = Some("playername".into());
                                    profile_edit_buffer.clear();
                                    profile_edit_buffer.push_str(&item.value);
                                    virtual_kb.open();
                                } else {
                                    let profiles_dir = data_dir.join("profiles");
                                    let name = prof_detail_profile.clone().unwrap_or_default();
                                    if let Ok(config) = profile_manager::toggle_profile_config(
                                        &profiles_dir,
                                        &name,
                                        &item.rel_path,
                                    ) {
                                        real_items =
                                            profile_manager::build_profile_detail_items(&config);
                                        panel_state.invalidate_cache();
                                    }
                                }
                            }
                        } else if app_state == AppState::SubScreen
                            && (panel_state.active == Some(SubScreenType::ModManager)
                                || panel_state.active == Some(SubScreenType::DynosPacks))
                            && !real_items.is_empty()
                        {
                            #[allow(clippy::collapsible_if)]
                            if item_selected < real_items.len() {
                                real_items[item_selected].enabled =
                                    !real_items[item_selected].enabled;
                                let names: Vec<String> = real_items
                                    .iter()
                                    .filter(|i| i.enabled)
                                    .map(|i| i.rel_path.clone())
                                    .collect();
                                write_active_config(&panel_state.active, &config_path, &names);
                                let active = real_items.iter().filter(|i| i.enabled).count();
                                panel_state.header_extra =
                                    Some(format!("{active}/{} active", real_items.len()));
                                panel_state.invalidate_cache();
                            }
                        }
                    }
                    sdl2::keyboard::Keycode::F => {
                        let win = canvas.window_mut();
                        is_fullscreen = !is_fullscreen;
                        if is_fullscreen {
                            win.set_fullscreen(sdl2::video::FullscreenType::Desktop)
                                .ok();
                        } else {
                            win.set_fullscreen(sdl2::video::FullscreenType::Off).ok();
                        }
                    }
                    sdl2::keyboard::Keycode::C => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.clear_all_filters();
                        }
                    }
                    sdl2::keyboard::Keycode::N => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Profiles)
                            && prof_edit_action.is_none()
                        {
                            prof_edit_action = Some("create".into());
                            prof_edit_target = None;
                            profile_edit_buffer.clear();
                            virtual_kb.open();
                        }
                    }
                    sdl2::keyboard::Keycode::R => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Profiles)
                            && prof_edit_action.is_none()
                            && !real_items.is_empty()
                        {
                            let item = &real_items[item_selected];
                            if item.rel_path != "__new__" {
                                prof_edit_action = Some("rename".into());
                                prof_edit_target = Some(item.name.clone());
                                profile_edit_buffer.clear();
                                profile_edit_buffer.push_str(&item.name);
                                virtual_kb.open();
                            }
                        }
                    }
                    sdl2::keyboard::Keycode::Delete => {
                        // Delete profile with hold‑to‑confirm (same pattern as shutdown, step 16)
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Profiles)
                            && !real_items.is_empty()
                        {
                            let item = &real_items[item_selected];
                            if item.rel_path != "__new__" {
                                prof_delete_hold_start = Some(Instant::now());
                            }
                        }
                    }
                    sdl2::keyboard::Keycode::PageUp => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.change_page(-1);
                        }
                    }
                    sdl2::keyboard::Keycode::PageDown => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.change_page(1);
                        }
                    }
                    sdl2::keyboard::Keycode::Home => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.jump_to_page(0);
                        }
                    }
                    sdl2::keyboard::Keycode::End => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            let tp = dl_state.total_pages();
                            dl_state.jump_to_page(tp.saturating_sub(1));
                        }
                    }
                    // Debug: simulate B‑button hold with K key (no gamepad required)
                    sdl2::keyboard::Keycode::K => {
                        if app_state == AppState::Game {
                            b_hold_start = Some(Instant::now());
                            app_state = AppState::ShuttingDown;
                        }
                    }
                    sdl2::keyboard::Keycode::Left => {
                        if virtual_kb.active {
                            virtual_kb.move_left();
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::TagChips
                            && !dl_state.top_tags.is_empty()
                        {
                            dl_state.tag_chip_selected =
                                dl_state.tag_chip_selected.saturating_sub(1);
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Network)
                        {
                            network_state.config.mode = network_state.config.mode.prev();
                            network_state.invalidate_cache();
                            network_manager::write_network_config(
                                &config_path,
                                &network_state.config,
                            )
                            .ok();
                        }
                    }
                    sdl2::keyboard::Keycode::Right => {
                        if virtual_kb.active {
                            virtual_kb.move_right();
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            && dl_state.focus_mode == DlFocusMode::TagChips
                            && !dl_state.top_tags.is_empty()
                        {
                            let max = dl_state.top_tags.len().saturating_sub(1);
                            if dl_state.tag_chip_selected < max {
                                dl_state.tag_chip_selected += 1;
                            }
                        } else if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Network)
                        {
                            network_state.config.mode = network_state.config.mode.next();
                            network_state.invalidate_cache();
                            network_manager::write_network_config(
                                &config_path,
                                &network_state.config,
                            )
                            .ok();
                        }
                    }
                    sdl2::keyboard::Keycode::Backspace => {
                        if network_state.editing_field.is_some() {
                            network_edit_buffer.pop();
                        }
                        if prof_edit_action.is_some() {
                            profile_edit_buffer.pop();
                        }
                        if dl_state.author_dropdown_open
                            && !dl_state.author_dropdown_text.is_empty()
                        {
                            dl_state.author_dropdown_text.pop();
                            dl_state.refresh_author_autocomplete();
                        }
                        if dl_state.search_active {
                            dl_state.search_text.pop();
                        }
                    }
                    sdl2::keyboard::Keycode::Slash => {
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.search_active = true;
                        }
                    }
                    // ── Debug keys: simulate gamepad buttons (step 30) ──
                    // These dispatch through the same GpAction path as a real
                    // gamepad, so the joy_button_to_action unit tests cover the
                    // actual event‑loop behaviour.
                    sdl2::keyboard::Keycode::L => {
                        // Simulate gamepad X button → Activate
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::Profiles)
                            && !real_items.is_empty()
                        {
                            let profiles_dir = data_dir.join("profiles");
                            let name = &real_items[item_selected].name;
                            if name != "+ New Profile" {
                                profile_manager::set_active_profile(&profiles_dir, name).ok();
                                let an = profile_manager::get_active_profile_name(&profiles_dir)
                                    .unwrap_or_else(|| "Default".into());
                                real_items = profile_manager::scan_profiles(&profiles_dir, &an)
                                    .unwrap_or_default();
                                let count = real_items.len().saturating_sub(1);
                                panel_state.header_extra = Some(format!("{count} profiles"));
                                panel_state.invalidate_cache();
                            }
                        }
                    }
                    sdl2::keyboard::Keycode::U => {
                        // Simulate gamepad L1 → PagePrev (download browser)
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.change_page(-1);
                        }
                    }
                    sdl2::keyboard::Keycode::O => {
                        // Simulate gamepad R1 → PageNext (download browser)
                        if app_state == AppState::SubScreen
                            && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        {
                            dl_state.change_page(1);
                        }
                    }
                    _ => {}
                },
                sdl2::event::Event::Window {
                    win_event: sdl2::event::WindowEvent::Resized(w, h),
                    ..
                } => {
                    cached_win_size = (w as u32, h as u32);
                    cached_bg_rect = None;
                    creator_rect = sdl2::rect::Rect::new(0, 0, 0, 0);
                }
                sdl2::event::Event::MouseMotion { x, y, .. } => {
                    mouse_x = x;
                    mouse_y = y;
                    // Hover snap: if in Game or Menu state, snap selection to hovered button
                    #[allow(clippy::collapsible_if)]
                    if app_state == AppState::Game || app_state == AppState::Menu {
                        if let Some(idx) = ui::menu::hit_test_menu_button(
                            mouse_x,
                            mouse_y,
                            cached_win_size.0,
                            cached_win_size.1,
                        ) {
                            menu_selected = idx;
                            highlight_y = ui::menu::menu_target_y(cached_win_size.1, idx);
                        }
                    }
                    // Hover row in item selector (step 30)
                    if app_state == AppState::SubScreen
                        && panel_state.is_visible(cached_win_size.0)
                        && !real_items.is_empty()
                    {
                        let body = ui::panel::panel_body_rect(
                            cached_win_size.0,
                            cached_win_size.1,
                            panel_state.slide_x,
                        );
                        hovered_row = common::hit_test_item_row(
                            x,
                            y,
                            body.x,
                            body.y,
                            body.w,
                            item_scroll,
                            common::DEFAULT_VISIBLE_ROWS,
                            real_items.len(),
                        );
                    } else {
                        hovered_row = None;
                    }
                }
                sdl2::event::Event::MouseButtonDown {
                    mouse_btn: sdl2::mouse::MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    // Creator button click
                    if creator_rect.w > 0 && creator_rect.contains_point((x, y)) {
                        log::info!("Creator button clicked — opening {CREATOR_URL}");
                        open_url(CREATOR_URL);
                    }
                    // Menu button click (Game or Menu state)
                    #[allow(clippy::collapsible_if)]
                    if app_state == AppState::Game || app_state == AppState::Menu {
                        if let Some(idx) = ui::menu::hit_test_menu_button(
                            x,
                            y,
                            cached_win_size.0,
                            cached_win_size.1,
                        ) {
                            menu_selected = idx;
                            highlight_y = ui::menu::menu_target_y(cached_win_size.1, idx);
                            if let Some(st) = subscreen_for_menu_index(idx) {
                                log::info!("Menu click: opening {:?}", st);
                                panel_state.open(st, cached_win_size.0);
                                app_state = AppState::SubScreen;
                                needs_item_setup = true;
                                item_selected = 0;
                                item_scroll = 0;
                            }
                        }
                    }
                    // Item selector row click (toggle in ModManager / Dynos)
                    if app_state == AppState::SubScreen && !real_items.is_empty() {
                        let body = ui::panel::panel_body_rect(
                            cached_win_size.0,
                            cached_win_size.1,
                            panel_state.slide_x,
                        );
                        if let Some(idx) = common::hit_test_item_row(
                            x,
                            y,
                            body.x,
                            body.y,
                            body.w,
                            item_scroll,
                            common::DEFAULT_VISIBLE_ROWS,
                            real_items.len(),
                        ) {
                            item_selected = idx;
                            real_items[idx].enabled = !real_items[idx].enabled;
                            let names: Vec<String> = real_items
                                .iter()
                                .filter(|i| i.enabled)
                                .map(|i| i.rel_path.clone())
                                .collect();
                            write_active_config(&panel_state.active, &config_path, &names);
                            let active = real_items.iter().filter(|i| i.enabled).count();
                            panel_state.header_extra =
                                Some(format!("{active}/{} active", real_items.len()));
                            panel_state.invalidate_cache();
                        }
                    }
                    // Download browser click handling
                    if app_state == AppState::SubScreen
                        && panel_state.active == Some(SubScreenType::DownloadBrowser)
                    {
                        let body = ui::panel::panel_body_rect(
                            cached_win_size.0,
                            cached_win_size.1,
                            panel_state.slide_x,
                        );
                        // Check clear all button
                        let has_filters = dl_state.search_active
                            || !dl_state.active_tags.is_empty()
                            || dl_state.active_author.is_some();
                        if ui::download_browser::hit_clear_all(
                            x,
                            y,
                            body.x,
                            body.y,
                            body.w,
                            body.h,
                            has_filters,
                        ) {
                            dl_state.clear_all_filters();
                        }
                        // Check author dropdown button
                        if ui::download_browser::hit_author_btn(
                            x, y, body.x, body.y, body.w, body.h,
                        ) {
                            dl_state.focus_mode = DlFocusMode::AuthorInput;
                            dl_state.toggle_author_dropdown();
                        }
                        // Check author × clear button
                        if dl_state.active_author.is_some()
                            && ui::download_browser::hit_author_clear(x, y, body.x, body.y, body.w)
                        {
                            dl_state.select_author(None);
                        }
                        // Check tag chips
                        {
                            let font_sm =
                                font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL).ok();
                            #[allow(clippy::collapsible_if)]
                            if let Some(fsm) = font_sm {
                                if let Some(ci) = ui::download_browser::hit_tag_chip(
                                    x, y, body.x, body.y, body.w, &dl_state, fsm,
                                ) {
                                    dl_state.focus_mode = DlFocusMode::TagChips;
                                    dl_state.tag_chip_selected = ci;
                                    let tag = dl_state.top_tags[ci].0.clone();
                                    dl_state.toggle_tag(&tag);
                                }
                            }
                        }
                    }
                }
                sdl2::event::Event::MouseWheel { y, .. } => {
                    if app_state == AppState::SubScreen
                        && panel_state.active == Some(SubScreenType::DownloadBrowser)
                    {
                        let max_scroll = dl_state.total().saturating_sub(common::DL_VISIBLE_ROWS);
                        if y > 0 {
                            dl_state.scroll = dl_state.scroll.saturating_sub(3);
                        } else if y < 0 {
                            dl_state.scroll = (dl_state.scroll + 3).min(max_scroll);
                        }
                    } else if app_state == AppState::SubScreen && !real_items.is_empty() {
                        let max_scroll = real_items
                            .len()
                            .saturating_sub(common::DEFAULT_VISIBLE_ROWS);
                        if y > 0 {
                            item_scroll = item_scroll.saturating_sub(3);
                        } else if y < 0 {
                            item_scroll = (item_scroll + 3).min(max_scroll);
                        }
                    }
                }
                sdl2::event::Event::JoyButtonDown { button_idx, .. } => {
                    // Dispatch through pure, testable function
                    match joy_button_to_action(button_idx) {
                        GpAction::Cancel => {
                            if app_state == AppState::Game {
                                b_hold_start = Some(Instant::now());
                                app_state = AppState::ShuttingDown;
                            } else if dl_state.author_dropdown_open {
                                dl_state.author_dropdown_open = false;
                            } else if dl_state.search_active {
                                dl_state.search_text.clear();
                                dl_state.search_active = false;
                            } else if app_state == AppState::SubScreen {
                                panel_state.close(cached_win_size.0);
                                app_state = AppState::Game;
                            } else if app_state == AppState::Menu {
                                app_state = AppState::Game;
                            }
                        }
                        GpAction::Confirm => {
                            if app_state == AppState::SubScreen && !real_items.is_empty() {
                                let item = &real_items[item_selected];
                                if panel_state.active == Some(SubScreenType::ModManager)
                                    || panel_state.active == Some(SubScreenType::DynosPacks)
                                {
                                    real_items[item_selected].enabled =
                                        !real_items[item_selected].enabled;
                                    let names: Vec<String> = real_items
                                        .iter()
                                        .filter(|i| i.enabled)
                                        .map(|i| i.rel_path.clone())
                                        .collect();
                                    write_active_config(&panel_state.active, &config_path, &names);
                                    let active = real_items.iter().filter(|i| i.enabled).count();
                                    panel_state.header_extra =
                                        Some(format!("{active}/{} active", real_items.len()));
                                    panel_state.invalidate_cache();
                                } else if panel_state.active == Some(SubScreenType::ProfileDetail) {
                                    if item.item_type == ItemType::Text {
                                        prof_edit_action = Some("playername".into());
                                        profile_edit_buffer.clear();
                                        profile_edit_buffer.push_str(&item.value);
                                        virtual_kb.open();
                                    } else {
                                        let profiles_dir = data_dir.join("profiles");
                                        let name = prof_detail_profile.clone().unwrap_or_default();
                                        if let Ok(config) = profile_manager::toggle_profile_config(
                                            &profiles_dir,
                                            &name,
                                            &item.rel_path,
                                        ) {
                                            real_items =
                                                profile_manager::build_profile_detail_items(
                                                    &config,
                                                );
                                            panel_state.invalidate_cache();
                                        }
                                    }
                                } else if panel_state.active == Some(SubScreenType::Profiles)
                                    && item.rel_path != "__new__"
                                {
                                    prof_detail_profile = Some(item.name.clone());
                                    panel_state
                                        .open(SubScreenType::ProfileDetail, cached_win_size.0);
                                    needs_item_setup = true;
                                }
                            } else if app_state == AppState::Game && game_exe.exists() {
                                launch_error = None;
                                app_state = AppState::Launching;
                                launch_start = Instant::now();
                                game_launched = false;
                                sdl2::mixer::Music::set_volume(3);
                            } else if app_state == AppState::Menu
                                && let Some(st) = subscreen_for_menu_index(menu_selected)
                            {
                                panel_state.open(st, cached_win_size.0);
                                app_state = AppState::SubScreen;
                                needs_item_setup = true;
                                item_selected = 0;
                                item_scroll = 0;
                            }
                        }
                        GpAction::Activate => {
                            if app_state == AppState::SubScreen
                                && panel_state.active == Some(SubScreenType::Profiles)
                                && !real_items.is_empty()
                            {
                                let profiles_dir = data_dir.join("profiles");
                                let name = &real_items[item_selected].name;
                                if name != "+ New Profile" {
                                    profile_manager::set_active_profile(&profiles_dir, name).ok();
                                    let an =
                                        profile_manager::get_active_profile_name(&profiles_dir)
                                            .unwrap_or_else(|| "Default".into());
                                    real_items = profile_manager::scan_profiles(&profiles_dir, &an)
                                        .unwrap_or_default();
                                    let count = real_items.len().saturating_sub(1);
                                    panel_state.header_extra = Some(format!("{count} profiles"));
                                    panel_state.invalidate_cache();
                                }
                            }
                        }
                        GpAction::PagePrev => {
                            if app_state == AppState::SubScreen
                                && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            {
                                dl_state.change_page(-1);
                            }
                        }
                        GpAction::PageNext => {
                            if app_state == AppState::SubScreen
                                && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            {
                                dl_state.change_page(1);
                            }
                        }
                        GpAction::Menu => {
                            if app_state == AppState::SubScreen
                                && panel_state.active == Some(SubScreenType::DownloadBrowser)
                            {
                                dl_state.focus_mode = match dl_state.focus_mode {
                                    DlFocusMode::ModList => DlFocusMode::TagChips,
                                    DlFocusMode::TagChips => DlFocusMode::AuthorInput,
                                    DlFocusMode::AuthorInput => DlFocusMode::ModList,
                                };
                                dl_state.author_dropdown_open = false;
                            } else {
                                app_state = if app_state == AppState::Menu {
                                    AppState::Game
                                } else if app_state == AppState::Game {
                                    AppState::Menu
                                } else {
                                    app_state
                                };
                            }
                        }
                        _ => {}
                    }
                }
                sdl2::event::Event::JoyButtonUp { button_idx, .. } => {
                    if joy_button_to_action(button_idx) == GpAction::Cancel
                        && app_state == AppState::ShuttingDown
                    {
                        shutdown_cancel_start = Some(Instant::now());
                    }
                }
                sdl2::event::Event::JoyHatMotion {
                    hat_idx: 0, state, ..
                } => {
                    let action = hat_to_action(state);
                    if virtual_kb.active {
                        match action {
                            GpAction::NavUp => virtual_kb.move_up(),
                            GpAction::NavDown => virtual_kb.move_down(),
                            GpAction::NavLeft => virtual_kb.move_left(),
                            GpAction::NavRight => virtual_kb.move_right(),
                            _ => {}
                        }
                    } else if app_state == AppState::SubScreen
                        && panel_state.active == Some(SubScreenType::DownloadBrowser)
                    {
                        let dl = &mut dl_state;
                        match action {
                            GpAction::NavUp if dl.focus_mode == DlFocusMode::ModList => {
                                dl.selected = dl.selected.saturating_sub(1);
                                dl.scroll = ensure_dl_visible(
                                    dl.selected,
                                    dl.scroll,
                                    dl.total(),
                                    common::DL_VISIBLE_ROWS,
                                );
                            }
                            GpAction::NavDown if dl.focus_mode == DlFocusMode::ModList => {
                                let max = dl.total().saturating_sub(1);
                                if dl.selected < max {
                                    dl.selected += 1;
                                }
                                dl.scroll = ensure_dl_visible(
                                    dl.selected,
                                    dl.scroll,
                                    dl.total(),
                                    common::DL_VISIBLE_ROWS,
                                );
                            }
                            GpAction::NavLeft if dl.focus_mode == DlFocusMode::TagChips => {
                                dl.tag_chip_selected = dl.tag_chip_selected.saturating_sub(1);
                            }
                            GpAction::NavRight if dl.focus_mode == DlFocusMode::TagChips => {
                                let max = dl.top_tags.len().saturating_sub(1);
                                if dl.tag_chip_selected < max {
                                    dl.tag_chip_selected += 1;
                                }
                            }
                            GpAction::NavUp
                                if dl.focus_mode == DlFocusMode::AuthorInput
                                    && dl.author_dropdown_open =>
                            {
                                dl.author_list_selected = dl.author_list_selected.saturating_sub(1);
                            }
                            GpAction::NavDown
                                if dl.focus_mode == DlFocusMode::AuthorInput
                                    && dl.author_dropdown_open =>
                            {
                                let max = dl.author_filtered.len().saturating_sub(1);
                                if dl.author_list_selected < max {
                                    dl.author_list_selected += 1;
                                }
                            }
                            _ => {}
                        }
                    } else if app_state == AppState::SubScreen
                        && panel_state.active == Some(SubScreenType::Network)
                    {
                        match action {
                            GpAction::NavUp | GpAction::NavDown => {
                                let vis = network_state.visible_fields();
                                if !vis.is_empty() {
                                    network_state.selected_field = if action == GpAction::NavUp {
                                        if network_state.selected_field == 0 {
                                            vis.len() - 1
                                        } else {
                                            network_state.selected_field - 1
                                        }
                                    } else {
                                        let c = network_state.selected_field;
                                        if c >= vis.len() - 1 { 0 } else { c + 1 }
                                    };
                                }
                            }
                            GpAction::NavLeft => {
                                network_state.config.mode = network_state.config.mode.prev();
                                network_state.invalidate_cache();
                                network_manager::write_network_config(
                                    &config_path,
                                    &network_state.config,
                                )
                                .ok();
                            }
                            GpAction::NavRight => {
                                network_state.config.mode = network_state.config.mode.next();
                                network_state.invalidate_cache();
                                network_manager::write_network_config(
                                    &config_path,
                                    &network_state.config,
                                )
                                .ok();
                            }
                            _ => {}
                        }
                    } else if app_state == AppState::SubScreen && !real_items.is_empty() {
                        match action {
                            GpAction::NavUp => {
                                item_selected = item_selected.saturating_sub(1);
                                item_scroll = common::ensure_selection_visible(
                                    item_selected,
                                    item_scroll,
                                    common::DEFAULT_VISIBLE_ROWS,
                                );
                            }
                            GpAction::NavDown => {
                                let max = real_items.len().saturating_sub(1);
                                if item_selected < max {
                                    item_selected += 1;
                                }
                                item_scroll = common::ensure_selection_visible(
                                    item_selected,
                                    item_scroll,
                                    common::DEFAULT_VISIBLE_ROWS,
                                );
                            }
                            _ => {}
                        }
                    } else if app_state == AppState::Menu {
                        match action {
                            GpAction::NavUp => {
                                if menu_selected == 0 {
                                    menu_selected = ui::menu::MENU_ITEM_COUNT - 1;
                                } else {
                                    menu_selected -= 1;
                                }
                            }
                            GpAction::NavDown => {
                                if menu_selected >= ui::menu::MENU_ITEM_COUNT - 1 {
                                    menu_selected = 0;
                                } else {
                                    menu_selected += 1;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        match action {
                            GpAction::NavUp => requested_track_change = Some(-1),
                            GpAction::NavDown => requested_track_change = Some(1),
                            _ => {}
                        }
                    }
                }
                sdl2::event::Event::JoyDeviceAdded { .. } => {
                    log::info!("Gamepad connected");
                }
                sdl2::event::Event::JoyDeviceRemoved { .. } => {
                    log::info!("Gamepad disconnected");
                }
                sdl2::event::Event::KeyUp {
                    keycode: Some(sdl2::keyboard::Keycode::K),
                    ..
                } => {
                    if app_state == AppState::ShuttingDown {
                        shutdown_cancel_start = Some(Instant::now());
                    }
                }
                sdl2::event::Event::KeyUp {
                    keycode: Some(sdl2::keyboard::Keycode::Delete),
                    ..
                } => {
                    prof_delete_hold_start = None;
                }
                sdl2::event::Event::TextInput { text, .. } => {
                    if network_state.editing_field.is_some() {
                        for ch in text.chars() {
                            if ch.is_alphanumeric() || ". -_@".contains(ch) {
                                network_state.append_char(ch, &mut network_edit_buffer);
                            }
                        }
                    }
                    if prof_edit_action.is_some() {
                        for ch in text.chars() {
                            if ch.is_alphanumeric() || ch == ' ' || ". -_".contains(ch) {
                                profile_edit_buffer.push(ch);
                            }
                        }
                    }
                    // Author dropdown autocomplete typing
                    if app_state == AppState::SubScreen
                        && panel_state.active == Some(SubScreenType::DownloadBrowser)
                        && dl_state.focus_mode == DlFocusMode::AuthorInput
                        && dl_state.author_dropdown_open
                    {
                        for ch in text.chars() {
                            if ch.is_alphanumeric() || ch == ' ' || ". -_".contains(ch) {
                                dl_state.author_dropdown_text.push(ch);
                                dl_state.refresh_author_autocomplete();
                            }
                        }
                    }
                    if dl_state.search_active {
                        for ch in text.chars() {
                            if ch.is_alphanumeric() || ch == ' ' || ". -_".contains(ch) {
                                dl_state.search_text.push(ch);
                            }
                        }
                    } else if app_state == AppState::SubScreen
                        && panel_state.active == Some(SubScreenType::DownloadBrowser)
                    {
                        // Auto-activate search on typing
                        dl_state.search_active = true;
                        for ch in text.chars() {
                            if ch.is_alphanumeric() || ch == ' ' || ". -_".contains(ch) {
                                dl_state.search_text.push(ch);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let total_time = fps_start.elapsed().as_secs_f64();
        let elapsed_ms = frame_start.duration_since(splash_start).as_secs_f64() * 1000.0;

        // Ctrl+C graceful shutdown
        if SHUTDOWN_REQUEST.load(Ordering::Acquire) {
            log::info!("Shutdown via Ctrl+C");
            sdl2::mixer::Music::halt();
            break 'main;
        }

        // ── Profile delete hold‑to‑confirm (1.5s) ──
        if let Some(hold_start) = prof_delete_hold_start {
            let hold_ms = hold_start.elapsed().as_millis() as u64;
            if hold_ms >= 1500 {
                prof_delete_hold_start = None;
                if app_state == AppState::SubScreen
                    && panel_state.active == Some(SubScreenType::Profiles)
                    && !real_items.is_empty()
                    && item_selected < real_items.len()
                {
                    let profile_name = real_items[item_selected].name.clone();
                    let profiles_dir = data_dir.join("profiles");
                    let result = profile_manager::delete_profile(&profiles_dir, &profile_name);
                    if let Err(e) = result {
                        log::error!("Delete profile failed: {e}");
                    } else {
                        let active_name = profile_manager::get_active_profile_name(&profiles_dir)
                            .unwrap_or_else(|| "Default".into());
                        if let Ok(items) =
                            profile_manager::scan_profiles(&profiles_dir, &active_name)
                        {
                            real_items = items;
                            let count = real_items.len().saturating_sub(1);
                            panel_state.header_extra = Some(format!("{count} profiles"));
                            if item_selected >= real_items.len() {
                                item_selected = real_items.len().saturating_sub(1);
                            }
                            panel_state.footer_hint = Some(
                                "ENTER: Config  |  Space: Activate  |  N: New  |  R: Rename  |  DEL: Delete  |  ESC: Back"
                                    .to_string(),
                            );
                            panel_state.invalidate_cache();
                        }
                    }
                }
            }
        }

        if app_state == AppState::ShuttingDown {
            let hold_ms = b_hold_start
                .map(|s| s.elapsed().as_millis() as u64)
                .unwrap_or(0);

            // Gradual cancel fade: when user releases B/K, alpha eases to 0
            // over FADE_DURATION_MS (500ms), then transitions back to Game.
            let alpha: u8 = if let Some(cancel_start) = shutdown_cancel_start {
                let cancel_elapsed = cancel_start.elapsed().as_millis() as u64;
                if cancel_elapsed >= FADE_DURATION_MS {
                    app_state = AppState::Game;
                    b_hold_start = None;
                    shutdown_cancel_start = None;
                    continue;
                }
                let fade_progress = 1.0 - (cancel_elapsed as f64 / FADE_DURATION_MS as f64);
                // Use the hold progress as the base, multiplied by fade_progress
                let pct = shutdown_progress(hold_ms) as f64;
                ((pct * 2.0).min(255.0) * fade_progress) as u8
            } else {
                let pct = shutdown_progress(hold_ms);
                (pct * 2).min(255) as u8
            };

            canvas.set_draw_color(sdl2::pixels::Color::RGB(0, 0, 0));
            canvas.clear();
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(sdl2::pixels::Color::RGBA(0, 0, 0, alpha));
            canvas
                .fill_rect(sdl2::rect::Rect::new(0, 0, WINDOW_W, WINDOW_H))
                .ok();
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            if shutdown_cancel_start.is_none() {
                let pct = shutdown_progress(hold_ms);
                let font_sm = font_cache.get_font(&ttf, font_path, FONT_SIZE_MEDIUM)?;
                let text = format!("Closing\u{2026} {pct}%");
                let surf = font_sm
                    .render(&text)
                    .blended(sdl2::pixels::Color::RGB(255, 255, 255))
                    .map_err(|e| format!("sd text: {e}"))?;
                let tc = canvas.texture_creator();
                let tex = tc
                    .create_texture_from_surface(&surf)
                    .map_err(|e| format!("sd tex: {e}"))?;
                let tw = surf.width();
                let th = surf.height();
                canvas
                    .copy(
                        &tex,
                        None,
                        Some(sdl2::rect::Rect::new(
                            (WINDOW_W as i32 - tw as i32) / 2,
                            (WINDOW_H as i32 - th as i32) / 2,
                            tw,
                            th,
                        )),
                    )
                    .map_err(|e| format!("sd copy: {e}"))?;
            }
            canvas.present();

            if hold_ms >= 5000 {
                log::info!("Shutdown complete");
                sdl2::mixer::Music::halt();
                break 'main;
            }
        } else if app_state == AppState::Splash {
            if load_phase == 0 && !loading_finished {
                splash.set_start(elapsed_ms);
            }

            let fade_ms = FADE_DURATION_MS as f64;
            if elapsed_ms >= fade_ms && !loading_finished {
                match load_phase {
                    0 => {
                        _nav_sound = sdl2::mixer::Chunk::from_file(&*NAV_WAV).ok();
                        log::debug!("Loaded nav sound");
                    }
                    1 => {
                        _splash_sound_chunk = sdl2::mixer::Chunk::from_file(&*SPLASH_WAV).ok();
                        log::debug!("Loaded splash sound");
                    }
                    2 => {
                        if let Ok(surf) = sdl2::surface::Surface::from_file(&*VINYL_PNG) {
                            vinyl_tex = canvas
                                .texture_creator()
                                .create_texture_from_surface(&surf)
                                .ok();
                        }
                        log::debug!("Loaded vinyl texture");
                    }
                    3 => {
                        if let Ok(surf) = sdl2::surface::Surface::from_file(&*LOGO_PNG) {
                            logo_tex = canvas
                                .texture_creator()
                                .create_texture_from_surface(&surf)
                                .ok();
                        }
                        log::debug!("Loaded logo texture");
                    }
                    4 => {
                        _icon_surface = sdl2::surface::Surface::from_file(&*ICON_PNG).ok();
                        log::debug!("Loaded icon surface");
                    }
                    5 => {
                        if let Ok(entries) = std::fs::read_dir(&*OGG_DIR) {
                            music_track_paths = entries
                                .filter_map(|e| e.ok())
                                .map(|e| e.path())
                                .filter(|p| p.extension().is_some_and(|ext| ext == "ogg"))
                                .collect();
                        }
                        log::debug!("Scanned {} music tracks", music_track_paths.len());
                    }
                    6 => {
                        if let Ok(surf) =
                            sdl2::surface::Surface::from_file(&*BACKGROUND_PNG)
                        {
                            background_tex = canvas
                                .texture_creator()
                                .create_texture_from_surface(&surf)
                                .ok();
                        }
                        log::debug!("Loaded background texture");
                    }
                    _ => {
                        loading_finished = true;
                        splash.mark_loaded();
                        log::info!("All assets loaded");
                    }
                }

                if load_phase < PHASE_DELTAS.len() {
                    current_progress += PHASE_DELTAS[load_phase];
                    splash.set_progress(current_progress);
                }
                load_phase += 1;
            }

            let font = font_cache.get_font(&ttf, font_path, FONT_SIZE_DEFAULT)?;
            let transition = splash.update(elapsed_ms);
            splash.render(&mut canvas, &mut splash_tex, font, elapsed_ms)?;
            canvas.present();

            if transition == ui::splash::SplashTransition::ToGame {
                app_state = AppState::Game;
                cached_bg_rect = None;
                creator_rect = sdl2::rect::Rect::new(0, 0, 0, 0);
                last_track_change = total_time;

                // Register music‑finished callback
                sdl2::mixer::Music::hook_finished(on_music_finished);

                // Load and play first music track
                if let Some(first_track) = music_track_paths.first() {
                    match sdl2::mixer::Music::from_file(first_track) {
                        Ok(music) => {
                            sdl2::mixer::Music::set_volume(MUSIC_VOLUME);
                            music.play(0)?;
                            _current_music = Some(music);
                            current_track_index = 0;
                            log::info!("Playing track 1/{}", music_track_paths.len());
                        }
                        Err(e) => {
                            log::warn!("Failed to load music track {:?}: {e}", first_track);
                        }
                    }
                } else {
                    log::warn!("No .ogg tracks found in assets/ogg-sounds/");
                }

                // Pre‑render prompt texture
                {
                    let font_med = font_cache.get_font(&ttf, font_path, FONT_SIZE_MEDIUM)?;
                    let ps = font_med
                        .render("Press ENTER / Button (A)")
                        .blended(sdl2::pixels::Color::RGB(255, 255, 255))
                        .map_err(|e| format!("Prompt render: {e}"))?;
                    prompt_tex_w = ps.width();
                    prompt_tex_h = ps.height();
                    prompt_tex = Some(
                        canvas
                            .texture_creator()
                            .create_texture_from_surface(&ps)
                            .map_err(|e| format!("Prompt texture: {e}"))?,
                    );
                }

                // Pre‑render creator button (rainbow text, per‑letter colors)
                {
                    let font_cr = font_cache.get_font(&ttf, font_path, CREATOR_FONT_SIZE)?;
                    let text = "By Retired64";
                    let mut total_w: u32 = 0;
                    let mut total_h: u32 = 0;
                    let mut char_surfs: Vec<sdl2::surface::Surface<'_>> = Vec::new();

                    for (i, ch) in text.chars().enumerate() {
                        let (r, g, b) = RAINBOW_COLORS[i % RAINBOW_COLORS.len()];
                        let ch_str = ch.to_string();
                        let surf = font_cr
                            .render(&ch_str)
                            .blended(sdl2::pixels::Color::RGB(r, g, b))
                            .map_err(|e| format!("Creator char render: {e}"))?;
                        total_w += surf.width();
                        total_h = total_h.max(surf.height());
                        char_surfs.push(surf);
                    }

                    let mut composite = sdl2::surface::Surface::new(
                        total_w,
                        total_h,
                        sdl2::pixels::PixelFormatEnum::ARGB8888,
                    )
                    .map_err(|e| format!("Creator composite surface: {e}"))?;
                    composite
                        .fill_rect(None, sdl2::pixels::Color::RGBA(0, 0, 0, 0))
                        .map_err(|e| format!("Creator fill: {e}"))?;

                    let mut cx: i32 = 0;
                    for surf in &char_surfs {
                        let dr = sdl2::rect::Rect::new(
                            cx,
                            (total_h - surf.height()) as i32 / 2,
                            surf.width(),
                            surf.height(),
                        );
                        surf.blit(None, &mut composite, Some(dr))
                            .map_err(|e| format!("Creator blit: {e}"))?;
                        cx += surf.width() as i32;
                    }

                    creator_tex_w = total_w;
                    creator_tex_h = total_h;
                    creator_tex = Some(
                        canvas
                            .texture_creator()
                            .create_texture_from_surface(&composite)
                            .map_err(|e| format!("Creator texture: {e}"))?,
                    );
                }

                // Pre‑render arc menu button labels
                for (i, item) in ui::menu::MENU_ITEMS.iter().enumerate() {
                    let font_cr = font_cache.get_font(&ttf, font_path, CREATOR_FONT_SIZE)?;
                    let label = format!("{}  {}", item.icon, item.label);
                    let surf = font_cr
                        .render(&label)
                        .blended(sdl2::pixels::Color::RGB(185, 175, 210))
                        .map_err(|e| format!("Menu btn {i} render: {e}"))?;
                    menu_btn_w[i] = surf.width();
                    menu_btn_h[i] = surf.height();
                    menu_btn_tex[i] = Some(
                        canvas
                            .texture_creator()
                            .create_texture_from_surface(&surf)
                            .map_err(|e| format!("Menu btn {i} texture: {e}"))?,
                    );
                }

                // Pre‑render selection dot "●"
                {
                    let font_dot = font_cache.get_font(&ttf, font_path, ui::menu::DOT_SIZE)?;
                    let dot_surf = font_dot
                        .render("●")
                        .blended(sdl2::pixels::Color::RGB(160, 100, 240))
                        .map_err(|e| format!("Dot render: {e}"))?;
                    menu_dot_w = dot_surf.width();
                    menu_dot_h = dot_surf.height();
                    menu_dot_tex = Some(
                        canvas
                            .texture_creator()
                            .create_texture_from_surface(&dot_surf)
                            .map_err(|e| format!("Dot texture: {e}"))?,
                    );
                }

                // Pre‑render track name + counter textures
                re_render_track_textures(
                    &mut canvas,
                    &mut font_cache,
                    &ttf,
                    font_path,
                    &music_track_paths,
                    current_track_index,
                    &mut track_name_tex,
                    &mut track_name_w,
                    &mut track_name_h,
                    &mut track_counter_tex,
                    &mut track_counter_w,
                    &mut track_counter_h,
                )?;

                // Pre‑render controls hint (bottom‑left)
                {
                    let font_hint = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                    let hint = "ESC / B(hold 5s) : Exit\nW/S or D-Pad : Change Track";
                    let surf = font_hint
                        .render(hint)
                        .blended(sdl2::pixels::Color::RGB(160, 140, 180))
                        .map_err(|e| format!("controls hint: {e}"))?;
                    controls_hint_w = surf.width();
                    controls_hint_h = surf.height();
                    controls_hint_tex = Some(
                        canvas
                            .texture_creator()
                            .create_texture_from_surface(&surf)
                            .map_err(|e| format!("controls tex: {e}"))?,
                    );
                }

                log::info!("Transitioning to Game state");
                // Initialize menu highlight to first button
                highlight_y = ui::menu::menu_target_y(WINDOW_H, 0);
            }
        } else if app_state == AppState::Launching {
            // ────── Launch fade + spawn ──────
            let fade_elapsed = launch_start.elapsed().as_secs_f64();
            let fade_progress = (fade_elapsed / 1.5).min(1.0) as f32;
            let alpha = ((1.0 - fade_progress) * 255.0) as u8;

            // Dark overlay with fade
            canvas.set_draw_color(sdl2::pixels::Color::RGB(0, 0, 0));
            canvas.clear();
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(sdl2::pixels::Color::RGBA(0, 0, 0, alpha));
            canvas
                .fill_rect(sdl2::rect::Rect::new(0, 0, WINDOW_W, WINDOW_H))
                .ok();
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            // "Iniciando..." text (pre‑render once, reuse per frame)
            if launch_tex.is_none() {
                let font_lg = font_cache.get_font(&ttf, font_path, FONT_SIZE_DEFAULT)?;
                let surf = font_lg
                    .render("Iniciando SM64 Coop DX\u{2026}")
                    .blended(sdl2::pixels::Color::RGB(255, 255, 255))
                    .map_err(|e| format!("launch text: {e}"))?;
                launch_tex_w = surf.width();
                launch_tex_h = surf.height();
                launch_tex = Some(
                    canvas
                        .texture_creator()
                        .create_texture_from_surface(&surf)
                        .map_err(|e| format!("launch tex: {e}"))?,
                );
            }
            if let Some(ref lt) = launch_tex {
                let tx = (WINDOW_W as i32 - launch_tex_w as i32) / 2;
                let ty = (WINDOW_H as i32 - launch_tex_h as i32) / 2;
                canvas
                    .copy(
                        lt,
                        None,
                        Some(sdl2::rect::Rect::new(tx, ty, launch_tex_w, launch_tex_h)),
                    )
                    .map_err(|e| format!("launch copy: {e}"))?;
            }

            canvas.present();

            // After 1500ms fade → validate and spawn the game
            if !game_launched && fade_elapsed >= 1.5 {
                game_launched = true;
                let enabled: Vec<String> = real_items
                    .iter()
                    .filter(|i| i.enabled)
                    .map(|i| i.rel_path.clone())
                    .collect();

                // Load active profile for args + optional binary override
                let profiles_dir = data_dir.join("profiles");
                let active_profile = profile_manager::load_active_profile(&profiles_dir);
                let profile_override = active_profile
                    .as_ref()
                    .and_then(|(config, _)| config.game_path.as_deref());

                // Re‑resolve game path with profile_override (tier 2.5)
                let game_exe = game::resolve_game_path(cli_game_path.as_deref(), profile_override);

                // ── Pre‑launch validation ──
                let game_dir = game_exe.parent().map(Path::new).unwrap_or(Path::new("."));
                let mut abort = false;

                // Validate game installation files
                if let Err(e) = game::validate_game_installation(game_dir, &data_dir) {
                    log::error!("Game validation failed: {e}");
                    launch_error = Some(e);
                    abort = true;
                }

                // Ensure ROM is present in savepath
                if !abort
                    && let Err(e) = game::ensure_rom(&data_dir, game_dir, &data_dir)
                {
                    log::error!("ROM check failed: {e}");
                    launch_error = Some(e);
                    abort = true;
                }

                if !abort {
                    let all_args = game::build_all_game_args(
                        &enabled,
                        &network_state.config,
                        active_profile.as_ref().map(|(c, _)| c),
                        active_profile.as_ref().map(|(_, d)| d.as_path()),
                        &data_dir,
                    );
                    match game::spawn_game(&game_exe, &all_args, &data_dir) {
                        Ok(child) => {
                            log::info!("Game spawned: {}", game_exe.display());
                            launch_error = None;
                            game::spawn_monitor(child);
                        }
                        Err(e) => {
                            log::error!("{e}");
                            launch_error = Some(e);
                        }
                    }
                }
                app_state = AppState::Game;
            }
        } else {
            // ══════════ Game / Menu / SubScreen ══════════
            let (win_w, win_h) = cached_win_size;

            // Restore music volume when game exits
            if game::GAME_EXITED.swap(false, Ordering::Acquire) {
                sdl2::mixer::Music::set_volume(MUSIC_VOLUME);
                log::info!("Game exited — music volume restored");
            }

            // Panel slide animation (update every frame when opening/closing)
            panel_state.update(dt, win_w);

            // Menu dimming easing: 1.0 when no sub‑screen, 0.35 when sub‑screen active
            let dim_target = if app_state == AppState::SubScreen {
                0.35
            } else {
                1.0
            };
            menu_dim += (dim_target - menu_dim) * (8.0 * dt as f32).min(1.0);

            // ── Track change processing ──
            let needs_auto_advance = TRACK_FINISHED.swap(false, Ordering::Acquire);
            let can_change = total_time - last_track_change >= TRACK_COOLDOWN_S;
            let mut change_dir: Option<isize> = None;

            if can_change && !music_track_paths.is_empty() {
                if needs_auto_advance {
                    change_dir = Some(1);
                } else if let Some(dir) = requested_track_change.take() {
                    change_dir = Some(dir);
                }
            }

            if let Some(dir) = change_dir {
                let len = music_track_paths.len();
                current_track_index =
                    ((len as isize + current_track_index as isize + dir) as usize) % len;
                last_track_change = total_time;

                sdl2::mixer::Music::halt();
                let path = &music_track_paths[current_track_index];
                match sdl2::mixer::Music::from_file(path) {
                    Ok(music) => {
                        sdl2::mixer::Music::set_volume(MUSIC_VOLUME);
                        music.play(0)?;
                        _current_music = Some(music);
                        log::info!("Playing track {}/{}", current_track_index + 1, len);
                    }
                    Err(e) => {
                        log::warn!("Failed to load track {:?}: {e}", path);
                    }
                }

                re_render_track_textures(
                    &mut canvas,
                    &mut font_cache,
                    &ttf,
                    font_path,
                    &music_track_paths,
                    current_track_index,
                    &mut track_name_tex,
                    &mut track_name_w,
                    &mut track_name_h,
                    &mut track_counter_tex,
                    &mut track_counter_w,
                    &mut track_counter_h,
                )?;
            }

            canvas.set_draw_color(sdl2::pixels::Color::RGB(BG_R, BG_G, BG_B));
            canvas.clear();

            // Background (cover, cached rect)
            if let Some(ref bg) = background_tex {
                if cached_bg_rect.is_none() {
                    let q = bg.query();
                    cached_bg_rect = Some(bg_cover_rect(q.width, q.height, win_w, win_h));
                }
                canvas.copy(bg, None, cached_bg_rect).ok();
            }

            // Semi-transparent dark overlay
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(sdl2::pixels::Color::RGBA(0, 0, 0, 140));
            canvas
                .fill_rect(sdl2::rect::Rect::new(0, 0, win_w, win_h))
                .ok();
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            // ── Vinyl disc ──
            ui::vinyl::draw_vinyl_player(
                &mut canvas,
                &mut vinyl_tex,
                total_time,
                VINYL_MARGIN,
                VINYL_ROT_SPEED,
                &mut track_name_tex,
                track_name_w,
                track_name_h,
                &mut track_counter_tex,
                track_counter_w,
                track_counter_h,
            )?;

            // ── Logo with breathing animation + drop shadow ──
            let logo_bottom = ui::logo::draw_game_logo(
                &mut canvas,
                &mut logo_tex,
                total_time,
                win_w,
                win_h,
                MAX_LOGO_W,
                MAX_LOGO_H,
                SHADOW_OFFSET,
            )?;

            // ── Launch prompt (below logo, alpha pulse) ──
            if let Some(ref mut pt) = prompt_tex {
                let alpha = (128.0 + 127.0 * (total_time as f32 * 2.5).sin()) as u8;
                pt.set_alpha_mod(alpha);
                let px = (win_w as i32 - prompt_tex_w as i32) / 2;
                let logo_h = logo_bottom.unwrap_or(0);
                canvas
                    .copy(
                        pt,
                        None,
                        Some(sdl2::rect::Rect::new(
                            px,
                            logo_h + PROMPT_GAP,
                            prompt_tex_w,
                            prompt_tex_h,
                        )),
                    )
                    .ok();
            }

            // ── Controls hint (bottom‑left) ──
            if let Some(ref ch) = controls_hint_tex {
                let cx = 30;
                let cy = win_h as i32 - controls_hint_h as i32 - 30;
                canvas
                    .copy(
                        ch,
                        None,
                        Some(sdl2::rect::Rect::new(
                            cx,
                            cy,
                            controls_hint_w,
                            controls_hint_h,
                        )),
                    )
                    .ok();
            }

            // ── Creator button (bottom‑right, rainbow per‑letter, hover + click) ──
            if let Some(ref ct) = creator_tex {
                // Cache hit-test rect; recalc on resize (rect.w == 0) or first frame
                if creator_rect.w == 0 {
                    let cx = win_w as i32 - creator_tex_w as i32 - CREATOR_MARGIN;
                    let cy = win_h as i32 - creator_tex_h as i32 - CREATOR_MARGIN;
                    creator_rect = sdl2::rect::Rect::new(cx, cy, creator_tex_w, creator_tex_h);
                }

                let hover = creator_rect.contains_point((mouse_x, mouse_y));
                let dy = if hover { -CREATOR_HOVER_OFFSET } else { 0 };
                let dx = creator_rect.x;
                let dy_pos = creator_rect.y + dy;

                canvas
                    .copy(
                        ct,
                        None,
                        Some(sdl2::rect::Rect::new(
                            dx,
                            dy_pos,
                            creator_tex_w,
                            creator_tex_h,
                        )),
                    )
                    .ok();

                if hover {
                    let ul_y = dy_pos + creator_tex_h as i32 + 2;
                    canvas.set_draw_color(sdl2::pixels::Color::RGB(255, 255, 255));
                    canvas
                        .fill_rect(sdl2::rect::Rect::new(dx, ul_y, creator_tex_w, 2))
                    .ok();
            }

            // ── Launch error (shown below prompt when game failed to start) ──
            if let Some(ref err) = launch_error {
                let logo_h = logo_bottom.unwrap_or(0);
                let mut y = logo_h + PROMPT_GAP + PROMPT_GAP; // leave room above
                let font_err = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                for line in err.lines() {
                    if line.is_empty() {
                        y += (FONT_SIZE_SMALL as i32) / 2;
                        continue;
                    }
                    let surf = font_err
                        .render(line)
                        .blended(sdl2::pixels::Color::RGB(255, 100, 100))
                        .map_err(|e| format!("error line: {e}"))?;
                    let lw = surf.width();
                    let lh = surf.height();
                    let tex = canvas
                        .texture_creator()
                        .create_texture_from_surface(&surf)
                        .map_err(|e| format!("error tex: {e}"))?;
                    let lx = (win_w as i32 - lw as i32) / 2;
                    canvas
                        .copy(
                            &tex,
                            None,
                            Some(sdl2::rect::Rect::new(lx, y, lw, lh)),
                        )
                        .ok();
                    y += lh as i32 + 4;
                }
            }
            }

            // ── Arc menu (right side, dims when sub‑screen is active) ──
            // Mouse hover snaps instantly (set in MouseMotion); keyboard nav eases.
            let target_y = ui::menu::menu_target_y(win_h, menu_selected);
            highlight_y += (target_y - highlight_y) * (12.0 * dt).min(1.0);

            ui::menu::render_arc_menu(
                &mut canvas,
                &mut menu_btn_tex,
                &menu_btn_w,
                &menu_btn_h,
                &mut menu_dot_tex,
                menu_dot_w,
                menu_dot_h,
                menu_selected,
                highlight_y,
                menu_dim,
                win_w,
                win_h,
            )?;

            // ── Sub‑screen panel (slide‑in from right) ──
            if panel_state.is_visible(win_w) || panel_state.active.is_some() {
                panel_state.render(
                    &mut canvas,
                    &mut font_cache,
                    &ttf,
                    font_path,
                    FONT_SIZE_MEDIUM,
                    FONT_SIZE_SMALL,
                    win_w,
                    win_h,
                )?;

                // Rebuild item list when a sub‑screen opens
                if needs_item_setup {
                    needs_item_setup = false;

                    if panel_state.active == Some(SubScreenType::ModManager) {
                        let mods_dir = data_dir.join("mods");
                        real_items = mod_manager::scan_mods(&mods_dir)?;
                        let enabled = mod_manager::parse_enabled_mods(&config_path)?;
                        mod_manager::apply_enabled_state(&mut real_items, &enabled);
                        let active_count = real_items.iter().filter(|i| i.enabled).count();
                        panel_state.header_extra =
                            Some(format!("{active_count}/{} active", real_items.len()));
                        panel_state.footer_hint = Some("ENTER: Toggle  |  ESC: Back".to_string());
                        panel_state.invalidate_cache();
                    } else if panel_state.active == Some(SubScreenType::DynosPacks) {
                        let packs_dir = data_dir.join("dynos").join("packs");
                        real_items = dynos_manager::scan_packs(&packs_dir)?;
                        let enabled = dynos_manager::parse_enabled_packs(&config_path)?;
                        dynos_manager::apply_enabled_state(&mut real_items, &enabled);
                        let active_count = real_items.iter().filter(|i| i.enabled).count();
                        panel_state.header_extra =
                            Some(format!("{active_count}/{} active", real_items.len()));
                        panel_state.footer_hint = Some("ENTER: Toggle  |  ESC: Back".to_string());
                        panel_state.invalidate_cache();
                    } else if panel_state.active == Some(SubScreenType::Network) {
                        network_state = NetworkFormState::new(
                            network_manager::parse_network_config(&config_path)?,
                        );
                        network_edit_buffer.clear();
                        panel_state.header_extra = None;
                        panel_state.footer_hint = Some(
                            "ENTER: Edit  |  \u{2190}\u{2192}: Cycle mode  |  ESC: Back"
                                .to_string(),
                        );
                        panel_state.invalidate_cache();
                    } else if panel_state.active == Some(SubScreenType::DownloadBrowser) {
                        // Lazy‑load database + build index on first open
                        if dl_state.db.is_none() {
                            let db_path = crate::config::DATABASE_JSON.as_path();
                            dl_state.db =
                                Some(crate::managers::download_manager::load_database(db_path)?);
                            dl_state.index =
                                Some(crate::managers::download_manager::build_search_index(
                                    dl_state.db.as_ref().unwrap(),
                                ));
                            log::info!(
                                "DB loaded: {} mods, {} tags, {} authors",
                                dl_state.index.as_ref().map_or(0, |i| i.entries.len()),
                                dl_state.index.as_ref().map_or(0, |i| i.tag_frequency.len()),
                                dl_state
                                    .index
                                    .as_ref()
                                    .map_or(0, |i| i.authors_sorted.len()),
                            );
                        }

                        // ── Scan for installed mods (step 24) ──
                        let installed_map_path = data_dir.join("launcher_downloads.json");
                        let installed_map = crate::managers::download_manager::load_installed_map(
                            &installed_map_path,
                        );
                        if let Some(ref idx) = dl_state.index {
                            let mods_dir = &data_dir;
                            if mods_dir.exists() {
                                let mut installed_dirs: Vec<std::path::PathBuf> = Vec::new();
                                if let Ok(entries) = std::fs::read_dir(mods_dir) {
                                    for entry in entries.filter_map(|e| e.ok()) {
                                        let p = entry.path();
                                        if p.is_dir() {
                                            installed_dirs.push(p);
                                        }
                                    }
                                }
                                dl_state.installed_ids =
                                    crate::managers::download_manager::detect_installed_mods(
                                        &installed_dirs,
                                        idx,
                                        &installed_map,
                                    );
                            }
                        }

                        // Save any new auto‑detected mappings
                        if let Some(ref idx) = dl_state.index {
                            let mut new_map = installed_map.clone();
                            let mut changed = false;
                            let mods_dir = &data_dir;
                            if mods_dir.exists()
                                && let Ok(entries) = std::fs::read_dir(mods_dir)
                            {
                                for entry in entries.filter_map(|e| e.ok()) {
                                    let p = entry.path();
                                    if let Some(folder) = p.file_name().and_then(|n| n.to_str())
                                        && !new_map.contains_key(folder)
                                    {
                                        // Check if this folder matches any installed mod
                                        // (via normalized match — re-use simple matching)
                                        let norm =
                                            crate::managers::download_manager::normalize(folder);
                                        for e in &idx.entries {
                                            let nt = crate::managers::download_manager::normalize(
                                                &e.title_display,
                                            );
                                            let ni = crate::managers::download_manager::normalize(
                                                &e.mod_id,
                                            );
                                            if norm == nt || norm == ni {
                                                new_map
                                                    .insert(folder.to_string(), e.mod_id.clone());
                                                changed = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if changed {
                                let _ = crate::managers::download_manager::save_installed_map(
                                    &installed_map_path,
                                    &new_map,
                                );
                            }
                        }

                        dl_state.init_index();
                        dl_state.clear_all_filters();
                        panel_state.header_extra = Some(format!(
                            "{} / {} mods",
                            dl_state.total(),
                            dl_state.sorted.len()
                        ));
                        panel_state.footer_hint = Some(
                            "ESC: Back  |  /: Search  |  Tab: Tags/Author  |  C: Clear all"
                                .to_string(),
                        );
                        panel_state.invalidate_cache();
                    } else if panel_state.active == Some(SubScreenType::Profiles) {
                        let profiles_dir = data_dir.join("profiles");
                        profile_manager::ensure_default_profile(&profiles_dir)?;
                        let active_name = profile_manager::get_active_profile_name(&profiles_dir)
                            .unwrap_or_else(|| "Default".into());
                        real_items = profile_manager::scan_profiles(&profiles_dir, &active_name)?;
                        let profile_count = real_items.len().saturating_sub(1);
                        panel_state.header_extra = Some(format!("{profile_count} profiles"));
                        panel_state.footer_hint = Some(
                            "ENTER: Config  |  Space: Activate  |  N: New  |  R: Rename  |  DEL: Delete  |  ESC: Back"
                                .to_string(),
                        );
                        panel_state.invalidate_cache();
                    } else if panel_state.active == Some(SubScreenType::ProfileDetail)
                        && let Some(ref profile_name) = prof_detail_profile
                    {
                        let profiles_dir = data_dir.join("profiles");
                        let profile_dir = profiles_dir.join(profile_name);
                        if let Ok(config) = profile_manager::load_profile_config(&profile_dir) {
                            real_items = profile_manager::build_profile_detail_items(&config);
                            panel_state.header_extra = Some(format!("Profile: {profile_name}"));
                            panel_state.footer_hint =
                                Some("ENTER: Toggle / Edit  |  ESC: Back".to_string());
                            panel_state.invalidate_cache();
                        }
                    }

                    // Pre‑render item name textures
                    item_tex.clear();
                    item_tex_w.clear();
                    item_tex_h.clear();
                    if !real_items.is_empty() {
                        let font_sm = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                        for item in &real_items {
                            let surf = font_sm
                                .render(&item.name)
                                .blended(sdl2::pixels::Color::RGB(185, 175, 210))
                                .map_err(|e| format!("item tex: {e}"))?;
                            item_tex_w.push(surf.width());
                            item_tex_h.push(surf.height());
                            item_tex.push(Some(
                                canvas
                                    .texture_creator()
                                    .create_texture_from_surface(&surf)
                                    .map_err(|e| format!("item tex: {e}"))?,
                            ));
                        }
                    }

                    // Pre‑render toggle icons (✓ — +) once per session
                    if icon_check_tex.is_none() {
                        let font_icon = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                        let check = font_icon
                            .render("✓")
                            .blended(sdl2::pixels::Color::RGB(0, 220, 100))
                            .map_err(|e| format!("check icon: {e}"))?;
                        let cross = font_icon
                            .render("—")
                            .blended(sdl2::pixels::Color::RGB(100, 100, 120))
                            .map_err(|e| format!("cross icon: {e}"))?;
                        let plus = font_icon
                            .render("+")
                            .blended(sdl2::pixels::Color::RGB(0, 255, 255))
                            .map_err(|e| format!("plus icon: {e}"))?;
                        icon_w = check.width();
                        icon_h = check.height();
                        icon_check_tex = Some(
                            canvas
                                .texture_creator()
                                .create_texture_from_surface(&check)
                                .map_err(|e| format!("check tex: {e}"))?,
                        );
                        icon_cross_tex = Some(
                            canvas
                                .texture_creator()
                                .create_texture_from_surface(&cross)
                                .map_err(|e| format!("cross tex: {e}"))?,
                        );
                        icon_plus_tex = Some(
                            canvas
                                .texture_creator()
                                .create_texture_from_surface(&plus)
                                .map_err(|e| format!("plus tex: {e}"))?,
                        );
                    }
                }

                // Draw item selector in the panel body area
                if app_state == AppState::SubScreen && !real_items.is_empty() {
                    let body = ui::panel::panel_body_rect(win_w, win_h, panel_state.slide_x);
                    if let (Some(check), Some(cross), Some(plus)) =
                        (&icon_check_tex, &icon_cross_tex, &icon_plus_tex)
                    {
                        common::draw_item_selector(
                            &mut canvas,
                            &real_items,
                            item_selected,
                            item_scroll,
                            common::DEFAULT_VISIBLE_ROWS,
                            true,
                            &item_tex,
                            &item_tex_w,
                            &item_tex_h,
                            check,
                            cross,
                            plus,
                            icon_w,
                            icon_h,
                            body.x,
                            body.y,
                            body.w,
                            body.h,
                            hovered_row,
                        )?;
                    }
                }

                if app_state == AppState::SubScreen
                    && panel_state.active == Some(SubScreenType::Network)
                {
                    let body = ui::panel::panel_body_rect(win_w, win_h, panel_state.slide_x);
                    let font_net = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                    // Virtual keyboard handles input when editing; physical
                    // keyboard works in parallel via TextInput events.
                    let cursor_visible = (total_time as u64 / 530).is_multiple_of(2);
                    ui::network_form::render_network_form(
                        &mut canvas,
                        &mut network_state,
                        font_net,
                        body.x,
                        body.y,
                        body.w,
                        body.h,
                        cursor_visible,
                    )?;
                }

                if app_state == AppState::SubScreen
                    && panel_state.active == Some(SubScreenType::DownloadBrowser)
                {
                    dl_state.update_filter();
                    let body = ui::panel::panel_body_rect(win_w, win_h, panel_state.slide_x);
                    let font_sm = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                    let font_title = font_sm; // reuse 18pt for title
                    ui::download_browser::draw_download_browser(
                        &mut canvas,
                        &mut dl_state,
                        font_sm,
                        font_title,
                        body.x,
                        body.y,
                        body.w,
                        body.h,
                    )?;
                }
            }

            // ── Download progress overlay ──
            if dl_download_active || dl_download_handle.is_some() {
                // Draw progress bar (may show "Complete!" or error on last frame)
                {
                    let progress = dl_progress.lock().unwrap();
                    let font_dl = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                    ui::download_browser::draw_progress_overlay(
                        &mut canvas,
                        font_dl,
                        &progress,
                        win_w,
                        win_h,
                        dl_cancel.load(Ordering::Relaxed),
                    )?;
                }
                // Clean up finished thread after drawing its final state
                if let Some(ref handle) = dl_download_handle
                    && handle.is_finished()
                {
                    dl_download_active = false;
                    dl_download_handle = None;
                }
            }

            // ── Virtual keyboard (renders on top of everything) ──
            {
                let font_kb = font_cache.get_font(&ttf, font_path, FONT_SIZE_SMALL)?;
                ui::keyboard::render_keyboard(&mut canvas, &virtual_kb, font_kb, win_w, win_h)?;
            }

            canvas.present();
        }

        frame_count += 1;
        let elapsed = frame_start.elapsed();
        if elapsed < target_frame_time {
            std::thread::sleep(target_frame_time - elapsed);
        }

        if fps_timer.elapsed() >= Duration::from_secs(1) {
            let total_elapsed = fps_start.elapsed().as_secs_f64();
            log::debug!("FPS: {frame_count} (t={total_elapsed:.1}s, state={app_state:?})");
            frame_count = 0;
            fps_timer = Instant::now();
        }
    }

    log::info!("Shutdown complete");
    Ok(())
}

// ── Helper: re‑render track name and counter textures after a track change ──
#[allow(clippy::too_many_arguments)]
fn re_render_track_textures<'ttf>(
    canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
    font_cache: &mut assets::FontCache<'ttf>,
    ttf: &'ttf sdl2::ttf::Sdl2TtfContext,
    font_path: &Path,
    track_paths: &[PathBuf],
    index: usize,
    name_tex: &mut Option<sdl2::render::Texture>,
    name_w: &mut u32,
    name_h: &mut u32,
    counter_tex: &mut Option<sdl2::render::Texture>,
    counter_w: &mut u32,
    counter_h: &mut u32,
) -> Result<(), String> {
    if track_paths.is_empty() {
        *name_tex = None;
        *counter_tex = None;
        return Ok(());
    }

    let path = &track_paths[index];
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown");

    // Track name
    {
        let font_sm = font_cache.get_font(ttf, font_path, config::FONT_SIZE_SMALL)?;
        let ts = font_sm
            .render(name)
            .blended(sdl2::pixels::Color::RGB(255, 255, 255))
            .map_err(|e| format!("Track name render: {e}"))?;
        *name_w = ts.width();
        *name_h = ts.height();
        *name_tex = Some(
            canvas
                .texture_creator()
                .create_texture_from_surface(&ts)
                .map_err(|e| format!("Track name texture: {e}"))?,
        );
    }

    // Counter "N / total"
    {
        let font_sm = font_cache.get_font(ttf, font_path, config::FONT_SIZE_SMALL)?;
        let counter_str = format!("{} / {}", index + 1, track_paths.len());
        let cs = font_sm
            .render(&counter_str)
            .blended(sdl2::pixels::Color::RGB(200, 160, 255))
            .map_err(|e| format!("Counter render: {e}"))?;
        *counter_w = cs.width();
        *counter_h = cs.height();
        *counter_tex = Some(
            canvas
                .texture_creator()
                .create_texture_from_surface(&cs)
                .map_err(|e| format!("Counter texture: {e}"))?,
        );
    }

    Ok(())
}

#[cfg(test)]
mod shutdown_tests {
    use super::shutdown_progress;

    #[test]
    fn at_zero() {
        assert_eq!(shutdown_progress(0), 0);
    }

    #[test]
    fn at_half() {
        assert_eq!(shutdown_progress(2500), 50);
    }

    #[test]
    fn at_full() {
        assert_eq!(shutdown_progress(5000), 100);
    }

    #[test]
    fn clamped_at_max() {
        assert_eq!(shutdown_progress(10000), 100);
    }
}
