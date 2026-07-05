use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::Canvas;
use sdl2::video::Window;

const KEY_W: u32 = 48;
const KEY_H: u32 = 38;
const KEY_GAP: i32 = 7;
const BOTTOM_MARGIN: i32 = 18;
const SPACE_W_MULT: u32 = 4;
const WIDE_W_MULT: u32 = 2;

const KB_BG: Color = Color::RGBA(40, 25, 80, 230);
const KEY_NORMAL: Color = Color::RGBA(40, 25, 80, 200);
const KEY_SELECTED_BORDER: Color = Color::RGB(180, 140, 255);
const KEY_TEXT: Color = Color::RGB(255, 255, 255);
const KEY_SPECIAL_BG: Color = Color::RGBA(60, 40, 100, 200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    Space,
    Backspace,
    Confirm,
}

#[derive(Debug, Clone, Copy)]
struct KeyDef {
    label: &'static str,
    width_mult: u32,
    action: KeyAction,
}

const LAYOUT: &[&[KeyDef]] = &[
    // Row 0
    &[
        KeyDef {
            label: "Q",
            width_mult: 1,
            action: KeyAction::Char('Q'),
        },
        KeyDef {
            label: "W",
            width_mult: 1,
            action: KeyAction::Char('W'),
        },
        KeyDef {
            label: "E",
            width_mult: 1,
            action: KeyAction::Char('E'),
        },
        KeyDef {
            label: "R",
            width_mult: 1,
            action: KeyAction::Char('R'),
        },
        KeyDef {
            label: "T",
            width_mult: 1,
            action: KeyAction::Char('T'),
        },
        KeyDef {
            label: "Y",
            width_mult: 1,
            action: KeyAction::Char('Y'),
        },
        KeyDef {
            label: "U",
            width_mult: 1,
            action: KeyAction::Char('U'),
        },
        KeyDef {
            label: "I",
            width_mult: 1,
            action: KeyAction::Char('I'),
        },
        KeyDef {
            label: "O",
            width_mult: 1,
            action: KeyAction::Char('O'),
        },
        KeyDef {
            label: "P",
            width_mult: 1,
            action: KeyAction::Char('P'),
        },
    ],
    // Row 1
    &[
        KeyDef {
            label: "A",
            width_mult: 1,
            action: KeyAction::Char('A'),
        },
        KeyDef {
            label: "S",
            width_mult: 1,
            action: KeyAction::Char('S'),
        },
        KeyDef {
            label: "D",
            width_mult: 1,
            action: KeyAction::Char('D'),
        },
        KeyDef {
            label: "F",
            width_mult: 1,
            action: KeyAction::Char('F'),
        },
        KeyDef {
            label: "G",
            width_mult: 1,
            action: KeyAction::Char('G'),
        },
        KeyDef {
            label: "H",
            width_mult: 1,
            action: KeyAction::Char('H'),
        },
        KeyDef {
            label: "J",
            width_mult: 1,
            action: KeyAction::Char('J'),
        },
        KeyDef {
            label: "K",
            width_mult: 1,
            action: KeyAction::Char('K'),
        },
        KeyDef {
            label: "L",
            width_mult: 1,
            action: KeyAction::Char('L'),
        },
    ],
    // Row 2
    &[
        KeyDef {
            label: "Z",
            width_mult: 1,
            action: KeyAction::Char('Z'),
        },
        KeyDef {
            label: "X",
            width_mult: 1,
            action: KeyAction::Char('X'),
        },
        KeyDef {
            label: "C",
            width_mult: 1,
            action: KeyAction::Char('C'),
        },
        KeyDef {
            label: "V",
            width_mult: 1,
            action: KeyAction::Char('V'),
        },
        KeyDef {
            label: "B",
            width_mult: 1,
            action: KeyAction::Char('B'),
        },
        KeyDef {
            label: "N",
            width_mult: 1,
            action: KeyAction::Char('N'),
        },
        KeyDef {
            label: "M",
            width_mult: 1,
            action: KeyAction::Char('M'),
        },
    ],
    // Row 3
    &[
        KeyDef {
            label: "1",
            width_mult: 1,
            action: KeyAction::Char('1'),
        },
        KeyDef {
            label: "2",
            width_mult: 1,
            action: KeyAction::Char('2'),
        },
        KeyDef {
            label: "3",
            width_mult: 1,
            action: KeyAction::Char('3'),
        },
        KeyDef {
            label: "4",
            width_mult: 1,
            action: KeyAction::Char('4'),
        },
        KeyDef {
            label: "5",
            width_mult: 1,
            action: KeyAction::Char('5'),
        },
        KeyDef {
            label: "6",
            width_mult: 1,
            action: KeyAction::Char('6'),
        },
        KeyDef {
            label: "7",
            width_mult: 1,
            action: KeyAction::Char('7'),
        },
        KeyDef {
            label: "8",
            width_mult: 1,
            action: KeyAction::Char('8'),
        },
        KeyDef {
            label: "9",
            width_mult: 1,
            action: KeyAction::Char('9'),
        },
        KeyDef {
            label: "0",
            width_mult: 1,
            action: KeyAction::Char('0'),
        },
    ],
    // Row 4
    &[
        KeyDef {
            label: ".",
            width_mult: 1,
            action: KeyAction::Char('.'),
        },
        KeyDef {
            label: "-",
            width_mult: 1,
            action: KeyAction::Char('-'),
        },
        KeyDef {
            label: "@",
            width_mult: 1,
            action: KeyAction::Char('@'),
        },
        KeyDef {
            label: "SPACE",
            width_mult: SPACE_W_MULT,
            action: KeyAction::Space,
        },
        KeyDef {
            label: "\u{232B}",
            width_mult: WIDE_W_MULT,
            action: KeyAction::Backspace,
        },
        KeyDef {
            label: "OK",
            width_mult: WIDE_W_MULT,
            action: KeyAction::Confirm,
        },
    ],
];

pub struct VirtualKeyboard {
    pub active: bool,
    /// Cursor row (0..LAYOUT.len())
    pub kb_row: usize,
    /// Cursor column within the current row (0..row_len-1)
    pub kb_col: usize,
}

impl VirtualKeyboard {
    pub fn new() -> Self {
        Self {
            active: false,
            kb_row: 0,
            kb_col: 0,
        }
    }

    pub fn open(&mut self) {
        self.active = true;
        self.kb_row = 0;
        self.kb_col = 0;
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    /// Navigation: wraparound IS enabled for both row and column.
    /// This matches the arc menu pattern — the keyboard is a closed set
    /// and wrapping from last row back to first is intuitive.
    pub fn move_up(&mut self) {
        if self.kb_row == 0 {
            self.kb_row = LAYOUT.len() - 1;
        } else {
            self.kb_row -= 1;
        }
        self.clamp_col();
    }

    pub fn move_down(&mut self) {
        if self.kb_row >= LAYOUT.len() - 1 {
            self.kb_row = 0;
        } else {
            self.kb_row += 1;
        }
        self.clamp_col();
    }

    pub fn move_left(&mut self) {
        let row_len = LAYOUT[self.kb_row].len();
        if self.kb_col == 0 {
            self.kb_col = row_len - 1;
        } else {
            self.kb_col -= 1;
        }
    }

    pub fn move_right(&mut self) {
        let row_len = LAYOUT[self.kb_row].len();
        if self.kb_col >= row_len - 1 {
            self.kb_col = 0;
        } else {
            self.kb_col += 1;
        }
    }

    fn clamp_col(&mut self) {
        let row_len = LAYOUT[self.kb_row].len();
        if self.kb_col >= row_len {
            self.kb_col = row_len - 1;
        }
    }

    /// Get the action of the currently selected key.
    pub fn selected_key(&self) -> KeyAction {
        LAYOUT[self.kb_row][self.kb_col].action
    }

    /// Apply the selected key action to a buffer. Returns true if the
    /// action was Confirm (caller should commit and close the keyboard).
    pub fn apply_selected(&self, buffer: &mut String) -> bool {
        match self.selected_key() {
            KeyAction::Char(ch) => {
                buffer.push(ch);
                false
            }
            KeyAction::Space => {
                buffer.push(' ');
                false
            }
            KeyAction::Backspace => {
                buffer.pop();
                false
            }
            KeyAction::Confirm => true,
        }
    }

    /// Fill the buffer with the character from the selected key (for
    /// physical ENTER/A-button press on the keyboard).
    pub fn confirm_if_active(&self, buffer: &mut String) -> bool {
        if !self.active {
            return false;
        }
        self.apply_selected(buffer)
    }
}

/// Compute the total width of a keyboard row.
fn row_width(row: &[KeyDef]) -> u32 {
    let mut w: u32 = 0;
    for (i, k) in row.iter().enumerate() {
        w += KEY_W * k.width_mult;
        if i + 1 < row.len() {
            w += KEY_GAP as u32;
        }
    }
    w
}

/// Compute the starting x of a row so it's centered.
fn row_x(row: &[KeyDef], win_w: u32) -> i32 {
    ((win_w as i32 - row_width(row) as i32) / 2).max(0)
}

/// Render the virtual keyboard at the bottom of the screen.
pub fn render_keyboard(
    canvas: &mut Canvas<Window>,
    kb: &VirtualKeyboard,
    font: &sdl2::ttf::Font<'_, 'static>,
    win_w: u32,
    win_h: u32,
) -> Result<(), String> {
    if !kb.active {
        return Ok(());
    }

    let total_h = LAYOUT.len() as u32 * KEY_H + (LAYOUT.len() - 1) as u32 * KEY_GAP as u32;
    let base_y = win_h as i32 - total_h as i32 - BOTTOM_MARGIN;

    // Background strip
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(KB_BG);
    canvas
        .fill_rect(Rect::new(0, base_y - 8, win_w, total_h + 16))
        .map_err(|e| e.to_string())?;
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    for (ri, row) in LAYOUT.iter().enumerate() {
        let rx = row_x(row, win_w);
        let ry = base_y + ri as i32 * (KEY_H as i32 + KEY_GAP);
        let mut cx = rx;

        for (ci, key) in row.iter().enumerate() {
            let kw = KEY_W * key.width_mult;
            let key_rect = Rect::new(cx, ry, kw, KEY_H);

            // Key background
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            if key.width_mult > 1 {
                canvas.set_draw_color(KEY_SPECIAL_BG);
            } else {
                canvas.set_draw_color(KEY_NORMAL);
            }
            canvas.fill_rect(key_rect).map_err(|e| e.to_string())?;
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            // Selection highlight
            if ri == kb.kb_row && ci == kb.kb_col {
                canvas.set_draw_color(KEY_SELECTED_BORDER);
                canvas.draw_rect(key_rect).map_err(|e| e.to_string())?;
            }

            // Key label
            let surf = font
                .render(key.label)
                .blended(KEY_TEXT)
                .map_err(|e| format!("kb label: {e}"))?;
            let tc = canvas.texture_creator();
            let tex = tc
                .create_texture_from_surface(&surf)
                .map_err(|e| format!("kb tex: {e}"))?;
            let tw = surf.width();
            let th = surf.height();
            let tx = cx + (kw as i32 - tw as i32) / 2;
            let ty = ry + (KEY_H as i32 - th as i32) / 2;
            canvas
                .copy(&tex, None, Some(Rect::new(tx, ty, tw, th)))
                .map_err(|e| format!("kb copy: {e}"))?;

            cx += kw as i32 + KEY_GAP;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_cursor_is_valid() {
        let kb = VirtualKeyboard::new();
        assert!(kb.kb_row < LAYOUT.len());
        assert!(kb.kb_col < LAYOUT[kb.kb_row].len());
    }

    #[test]
    fn move_right_wraps() {
        let mut kb = VirtualKeyboard::new();
        let len = LAYOUT[0].len();
        // Move to end, then one more wraps
        kb.kb_col = len - 1;
        kb.move_right();
        assert_eq!(kb.kb_col, 0);
    }

    #[test]
    fn move_left_wraps() {
        let mut kb = VirtualKeyboard::new();
        kb.kb_col = 0;
        kb.move_left();
        let len = LAYOUT[0].len();
        assert_eq!(kb.kb_col, len - 1);
    }

    #[test]
    fn move_up_wraps() {
        let mut kb = VirtualKeyboard::new();
        kb.kb_row = 0;
        kb.move_up();
        assert_eq!(kb.kb_row, LAYOUT.len() - 1);
    }

    #[test]
    fn move_down_wraps() {
        let mut kb = VirtualKeyboard::new();
        kb.kb_row = LAYOUT.len() - 1;
        kb.move_down();
        assert_eq!(kb.kb_row, 0);
    }

    #[test]
    fn backspace_on_empty_buffer_does_not_panic() {
        let _kb = VirtualKeyboard::new();
        let mut buf = String::new();
        // Select the backspace key (row 4, col 4)
        let bs: &VirtualKeyboard = &VirtualKeyboard {
            active: true,
            kb_row: 4,
            kb_col: 4,
        };
        assert!(!bs.apply_selected(&mut buf));
        assert!(buf.is_empty());
    }

    #[test]
    fn space_writes_space() {
        let bs = VirtualKeyboard {
            active: true,
            kb_row: 4,
            kb_col: 3,
        };
        let mut buf = String::from("abc");
        assert!(!bs.apply_selected(&mut buf));
        assert_eq!(buf, "abc ");
    }

    #[test]
    fn confirm_returns_true() {
        let bs = VirtualKeyboard {
            active: true,
            kb_row: 4,
            kb_col: 5,
        };
        let mut buf = String::from("hello");
        assert!(bs.apply_selected(&mut buf));
        assert_eq!(buf, "hello"); // confirm doesn't modify buffer
    }
}
