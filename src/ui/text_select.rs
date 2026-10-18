//! Mouse text selection and copying for the row-based, custom-painted text views
//! (diffs and file contents). Text is monospace with tabs drawn as four columns.

use egui::{Id, Pos2, Rect, Response, Vec2};

/// A position: row index and byte offset into that row's text.
type Cell = (usize, usize);

#[derive(Clone, Copy, Default)]
struct State {
    anchor: Cell,
    head: Cell,
    /// Whether anything is selected (the anchor may equal the head while dragging).
    active: bool,
    dragging: bool,
}

/// Which selector owns the selection, so only one view shows (and copies) one.
fn owner_id() -> Id {
    Id::new("text_select_owner")
}

pub struct Selector {
    id: Id,
    state: State,
    row_h: f32,
    char_w: f32,
    /// Screen x of the text start and y of row 0, from a painted row.
    origin: Option<Pos2>,
    copy_requested: bool,
    copy_text: Option<String>,
}

const TAB_COLS: usize = 4;

fn cols_of(s: &str) -> usize {
    s.chars()
        .map(|c| if c == '\t' { TAB_COLS } else { 1 })
        .sum()
}

/// Byte offset of the character boundary nearest to display column `col`.
fn byte_at_col(s: &str, col: f32) -> usize {
    let mut at = 0.0;
    for (i, c) in s.char_indices() {
        let w = if c == '\t' { TAB_COLS as f32 } else { 1.0 };
        if col < at + w / 2.0 {
            return i;
        }
        at += w;
    }
    s.len()
}

fn clamp_byte(s: &str, mut b: usize) -> usize {
    b = b.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte range of the word (or run of other non-space characters) around `b`.
fn word_at(s: &str, b: usize) -> (usize, usize) {
    let b = clamp_byte(s, b);
    let Some(c) = s[b..].chars().next().or_else(|| s[..b].chars().next_back()) else {
        return (b, b);
    };
    let same = |x: char| {
        if is_word(c) {
            is_word(x)
        } else if c.is_whitespace() {
            x.is_whitespace()
        } else {
            !is_word(x) && !x.is_whitespace()
        }
    };
    let start = s[..b]
        .char_indices()
        .rev()
        .take_while(|&(_, x)| same(x))
        .last()
        .map_or(b, |(i, _)| i);
    let end = s[b..]
        .char_indices()
        .find(|&(_, x)| !same(x))
        .map_or(s.len(), |(i, _)| b + i);
    (start, end)
}

impl Selector {
    pub fn new(ui: &egui::Ui, id: Id, row_h: f32, char_w: f32) -> Self {
        let owned = ui.data(|d| d.get_temp::<Id>(owner_id())) == Some(id);
        let state = if owned {
            ui.data(|d| d.get_temp::<State>(id)).unwrap_or_default()
        } else {
            State::default()
        };
        Self {
            id,
            state,
            row_h,
            char_w,
            origin: None,
            copy_requested: false,
            copy_text: None,
        }
    }

    fn range(&self) -> Option<(Cell, Cell)> {
        let s = &self.state;
        if !s.active || s.anchor == s.head {
            return None;
        }
        Some(if s.anchor <= s.head {
            (s.anchor, s.head)
        } else {
            (s.head, s.anchor)
        })
    }

    pub fn has_selection(&self) -> bool {
        self.range().is_some()
    }

    fn cell_at(
        &self,
        pos: Pos2,
        origin: Pos2,
        rows: usize,
        text: &dyn Fn(usize) -> Option<String>,
    ) -> Cell {
        if rows == 0 {
            return (0, 0);
        }
        let row = (pos.y - origin.y) / self.row_h;
        if row < 0.0 {
            return (0, 0);
        }
        let row = row as usize;
        if row >= rows {
            let last = rows - 1;
            return (last, text(last).map_or(0, |t| t.len()));
        }
        let col = ((pos.x - origin.x) / self.char_w).max(0.0);
        (row, text(row).map_or(0, |t| byte_at_col(&t, col)))
    }

    /// Handles input on one painted row and returns the selection highlight to paint
    /// behind its text. `text_left` is the screen x where the row's text starts.
    pub fn row(
        &mut self,
        ui: &egui::Ui,
        index: usize,
        rect: Rect,
        resp: &Response,
        text_left: f32,
        text: Option<&str>,
    ) -> Option<Rect> {
        let origin = Pos2::new(text_left, rect.top() - index as f32 * self.row_h);
        self.origin = Some(origin);
        if text.is_some() && resp.hover_pos().is_some_and(|p| p.x >= text_left) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        if resp.drag_started()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let col = ((pos.x - text_left) / self.char_w).max(0.0);
            let cell = (index, text.map_or(0, |t| byte_at_col(t, col)));
            let shift = ui.input(|i| i.modifiers.shift);
            if !(shift && self.state.active) {
                self.state.anchor = cell;
            }
            self.state.head = cell;
            self.state.active = true;
            self.state.dragging = true;
            ui.data_mut(|d| d.insert_temp(owner_id(), self.id));
        } else if resp.double_clicked()
            && let (Some(t), Some(pos)) = (text, resp.interact_pointer_pos())
        {
            let col = ((pos.x - text_left) / self.char_w).max(0.0);
            let (a, b) = word_at(t, byte_at_col(t, col));
            self.state = State {
                anchor: (index, a),
                head: (index, b),
                active: true,
                dragging: false,
            };
            ui.data_mut(|d| d.insert_temp(owner_id(), self.id));
        } else if resp.clicked() {
            self.state.active = false;
        }
        if let Some(t) = text {
            resp.context_menu(|ui| {
                if ui
                    .add_enabled(self.has_selection(), egui::Button::new("Copy"))
                    .clicked()
                {
                    self.copy_requested = true;
                    ui.close();
                }
                if ui.button("Copy Line").clicked() {
                    self.copy_text = Some(t.to_owned());
                    ui.close();
                }
            });
        }
        self.highlight(index, rect, text_left, text?)
    }

    fn highlight(&self, index: usize, rect: Rect, text_left: f32, text: &str) -> Option<Rect> {
        let ((r0, b0), (r1, b1)) = self.range()?;
        if index < r0 || index > r1 {
            return None;
        }
        let start = if index == r0 { clamp_byte(text, b0) } else { 0 };
        let (end, newline) = if index == r1 {
            (clamp_byte(text, b1), false)
        } else {
            (text.len(), true)
        };
        let c0 = cols_of(&text[..start.min(end)]);
        let c1 = cols_of(&text[..end]) + newline as usize;
        (c1 > c0).then(|| {
            Rect::from_min_size(
                Pos2::new(text_left + c0 as f32 * self.char_w, rect.top()),
                Vec2::new((c1 - c0) as f32 * self.char_w, rect.height()),
            )
        })
    }

    /// Call after painting the rows (inside the scroll area): extends a drag selection,
    /// scrolls while dragging past the edge and copies on ⌘/Ctrl C.
    /// `text(row)` returns a row's copyable text (`None` for rows like hunk headers).
    pub fn finish(
        mut self,
        ui: &egui::Ui,
        rows: usize,
        auto_scroll: bool,
        text: impl Fn(usize) -> Option<String>,
    ) {
        if self.state.dragging {
            let (pos, down) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down()));
            if !down {
                self.state.dragging = false;
            } else if let (Some(pos), Some(origin)) = (pos, self.origin) {
                self.state.head = self.cell_at(pos, origin, rows, &text);
                let clip = ui.clip_rect();
                if auto_scroll && (pos.y < clip.top() || pos.y > clip.bottom()) {
                    let dy = if pos.y < clip.top() {
                        clip.top() - pos.y
                    } else {
                        clip.bottom() - pos.y
                    };
                    ui.scroll_with_delta(Vec2::new(0.0, dy.clamp(-40.0, 40.0)));
                    ui.ctx().request_repaint();
                }
            }
        }
        let copy_key = self.has_selection()
            && ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if (copy_key || self.copy_requested)
            && let Some(((r0, b0), (r1, b1))) = self.range()
        {
            let mut out = Vec::new();
            for r in r0..=r1.min(rows.saturating_sub(1)) {
                let Some(t) = text(r) else { continue };
                let start = if r == r0 { clamp_byte(&t, b0) } else { 0 };
                let end = if r == r1 { clamp_byte(&t, b1) } else { t.len() };
                out.push(t[start.min(end)..end].to_owned());
            }
            self.copy_text = Some(out.join("\n"));
        }
        if let Some(t) = self.copy_text.take() {
            ui.ctx().copy_text(t);
        }
        ui.data_mut(|d| d.insert_temp(self.id, self.state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_and_bytes() {
        assert_eq!(byte_at_col("\tab", 0.0), 0);
        assert_eq!(byte_at_col("\tab", 3.0), 1);
        assert_eq!(byte_at_col("\tab", 4.0), 1);
        assert_eq!(byte_at_col("\tab", 5.0), 2);
        assert_eq!(byte_at_col("éa", 1.0), 2);
        assert_eq!(byte_at_col("ab", 9.0), 2);
        assert_eq!(cols_of("\tab"), 6);
    }

    #[test]
    fn words() {
        let s = "let foo_bar = 1;";
        assert_eq!(word_at(s, 5), (4, 11));
        assert_eq!(word_at(s, 12), (12, 13));
        assert_eq!(word_at(s, 99), (15, 16));
    }
}
