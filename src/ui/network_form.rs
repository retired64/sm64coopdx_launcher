use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::ttf::Font;
use sdl2::video::Window;

use crate::managers::network_manager::{NetworkConfig, NetworkMode};
use crate::ui::common::{ACCENT_W, ROW_H, ROW_STEP};

const FIELD_LABEL_X: i32 = 16;
const FIELD_VALUE_X: i32 = 200;
const FIELD_LABEL_COLOR: Color = Color::RGB(185, 175, 210);
const FIELD_VALUE_COLOR: Color = Color::RGB(255, 255, 255);
const FIELD_EDIT_COLOR: Color = Color::RGB(160, 255, 160);
const HIGHLIGHT_COLOR: Color = Color::RGBA(80, 35, 180, 70);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Int,
    Cycle,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct NetworkField {
    pub label: &'static str,
    pub field_type: FieldType,
    pub index: usize,
    pub visible: fn(NetworkMode) -> bool,
}

const ALWAYS: fn(NetworkMode) -> bool = |_| true;
const ONLY_CLIENT: fn(NetworkMode) -> bool = |m| matches!(m, NetworkMode::Client);
const ONLY_SERVER: fn(NetworkMode) -> bool = |m| matches!(m, NetworkMode::Server);
const ONLY_SERVER_OR_COOP: fn(NetworkMode) -> bool =
    |m| matches!(m, NetworkMode::Server | NetworkMode::CoopNet);
const ONLY_COOP: fn(NetworkMode) -> bool = |m| matches!(m, NetworkMode::CoopNet);

pub static NETWORK_FIELDS: &[NetworkField] = &[
    NetworkField {
        label: "Player Name",
        field_type: FieldType::Text,
        index: 0,
        visible: ALWAYS,
    },
    NetworkField {
        label: "Mode",
        field_type: FieldType::Cycle,
        index: 1,
        visible: ALWAYS,
    },
    NetworkField {
        label: "Join IP",
        field_type: FieldType::Text,
        index: 2,
        visible: ONLY_CLIENT,
    },
    NetworkField {
        label: "Join Port",
        field_type: FieldType::Int,
        index: 3,
        visible: ONLY_CLIENT,
    },
    NetworkField {
        label: "Host Port",
        field_type: FieldType::Int,
        index: 4,
        visible: ONLY_SERVER,
    },
    NetworkField {
        label: "Max Players",
        field_type: FieldType::Int,
        index: 5,
        visible: ONLY_SERVER_OR_COOP,
    },
    NetworkField {
        label: "Password",
        field_type: FieldType::Text,
        index: 6,
        visible: ONLY_COOP,
    },
];

/// Cached textures for network form labels and values.
/// Labels are rendered once (they never change).
/// Values are cached and invalidated on config write or editing state change.
pub struct NetworkFormState {
    pub config: NetworkConfig,
    pub selected_field: usize,
    pub editing_field: Option<usize>,
    // Texture caches (filled on first render call)
    pub label_tex: [Option<sdl2::render::Texture>; 7],
    pub label_w: [u32; 7],
    pub label_h: [u32; 7],
    pub value_tex: [Option<sdl2::render::Texture>; 7],
    pub value_w: [u32; 7],
    pub value_h: [u32; 7],
    /// Config version counter — bump to invalidate all value caches.
    config_version: u32,
}

impl NetworkFormState {
    pub fn new(config: NetworkConfig) -> Self {
        Self {
            config,
            selected_field: 0,
            editing_field: None,
            label_tex: Default::default(),
            label_w: [0; 7],
            label_h: [0; 7],
            value_tex: Default::default(),
            value_w: [0; 7],
            value_h: [0; 7],
            config_version: 0,
        }
    }

    pub fn visible_fields(&self) -> Vec<NetworkField> {
        NETWORK_FIELDS
            .iter()
            .filter(|f| (f.visible)(self.config.mode))
            .copied()
            .collect()
    }

    pub fn field_value(&self, field: &NetworkField) -> String {
        match field.index {
            0 => self.config.player_name.clone(),
            1 => self.config.mode.label().to_string(),
            2 => self.config.join_ip.clone(),
            3 => self.config.join_port.to_string(),
            4 => self.config.host_port.to_string(),
            5 => self.config.max_players.to_string(),
            6 => self.config.coopnet_password.clone(),
            _ => String::new(),
        }
    }

    pub fn field_value_for_index(&self, idx: usize) -> String {
        if idx >= NETWORK_FIELDS.len() {
            return String::new();
        }
        self.field_value(&NETWORK_FIELDS[idx])
    }

    pub fn commit_edit(&mut self, buffer: &str) {
        let Some(editing_idx) = self.editing_field else {
            return;
        };
        let field = &NETWORK_FIELDS[editing_idx];
        match field.index {
            0 => self.config.player_name = buffer.to_string(),
            2 => self.config.join_ip = buffer.to_string(),
            3 => {
                if let Ok(v) = buffer.parse() {
                    self.config.join_port = v;
                }
            }
            4 => {
                if let Ok(v) = buffer.parse() {
                    self.config.host_port = v;
                }
            }
            5 => {
                if let Ok(v) = buffer.parse() {
                    self.config.max_players = v;
                }
            }
            6 => self.config.coopnet_password = buffer.to_string(),
            _ => {}
        }
        self.editing_field = None;
        self.config_version = self.config_version.wrapping_add(1);
    }

    pub fn append_char(&mut self, ch: char, buffer: &mut String) {
        if self.editing_field.is_some() {
            buffer.push(ch);
        }
    }

    pub fn invalidate_cache(&mut self) {
        self.config_version = self.config_version.wrapping_add(1);
    }

    /// Check if value cache for a field index is stale.
    fn value_cache_stale(&self, idx: usize) -> bool {
        self.value_tex[idx].is_none() || self.editing_field == Some(idx)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_network_form(
    canvas: &mut Canvas<Window>,
    state: &mut NetworkFormState,
    font: &Font<'_, 'static>,
    body_x: i32,
    body_y: i32,
    body_w: i32,
    _body_h: i32,
    show_cursor: bool,
) -> Result<(), String> {
    let visible = state.visible_fields();
    if visible.is_empty() {
        return Ok(());
    }

    for (vi, field) in visible.iter().enumerate() {
        let actual_idx = field.index;
        let row_y = body_y + vi as i32 * ROW_STEP as i32;

        // Highlight
        if actual_idx == state.selected_field {
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            canvas.set_draw_color(HIGHLIGHT_COLOR);
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, body_w as u32, ROW_H))
                .map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            canvas.set_draw_color(Color::RGB(160, 100, 240));
            canvas
                .fill_rect(sdl2::rect::Rect::new(body_x, row_y, ACCENT_W, ROW_H))
                .map_err(|e| e.to_string())?;
        }

        // Label texture (cache, never changes)
        if state.label_tex[actual_idx].is_none() {
            let surf = font
                .render(field.label)
                .blended(FIELD_LABEL_COLOR)
                .map_err(|e| format!("label render: {e}"))?;
            state.label_w[actual_idx] = surf.width();
            state.label_h[actual_idx] = surf.height();
            state.label_tex[actual_idx] = Some(
                canvas
                    .texture_creator()
                    .create_texture_from_surface(&surf)
                    .map_err(|e| format!("label tex: {e}"))?,
            );
        }
        if let Some(ref lt) = state.label_tex[actual_idx] {
            let ty = row_y + (ROW_H as i32 - state.label_h[actual_idx] as i32) / 2;
            canvas
                .copy(
                    lt,
                    None,
                    Some(sdl2::rect::Rect::new(
                        body_x + FIELD_LABEL_X,
                        ty,
                        state.label_w[actual_idx],
                        state.label_h[actual_idx],
                    )),
                )
                .map_err(|e| e.to_string())?;
        }

        // Value texture (cache, invalidate on config change or editing)
        let is_editing = state.editing_field == Some(actual_idx);
        if state.value_cache_stale(actual_idx) || is_editing {
            let mut val = state.field_value(field);
            if is_editing && show_cursor {
                val.push('_');
            }
            let val_color = if is_editing {
                FIELD_EDIT_COLOR
            } else {
                FIELD_VALUE_COLOR
            };
            let surf = font
                .render(&val)
                .blended(val_color)
                .map_err(|e| format!("val render: {e}"))?;
            state.value_w[actual_idx] = surf.width();
            state.value_h[actual_idx] = surf.height();
            state.value_tex[actual_idx] = Some(
                canvas
                    .texture_creator()
                    .create_texture_from_surface(&surf)
                    .map_err(|e| format!("val tex: {e}"))?,
            );
        }
        if let Some(ref vt) = state.value_tex[actual_idx] {
            let ty = row_y + (ROW_H as i32 - state.value_h[actual_idx] as i32) / 2;
            canvas
                .copy(
                    vt,
                    None,
                    Some(sdl2::rect::Rect::new(
                        body_x + FIELD_VALUE_X,
                        ty,
                        state.value_w[actual_idx],
                        state.value_h[actual_idx],
                    )),
                )
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}
