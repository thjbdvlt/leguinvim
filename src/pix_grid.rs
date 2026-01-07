use crate::ui_model::{Line, UiModel};

pub const GRID_WIDTH_RATIO: f64 = 0.9;

pub type PixLine = Box<[f32]>;
type PixMatrix = Box<[PixLine]>;

macro_rules! unscale {
    ($e: expr) => {
        ($e / pango::SCALE) as f32
    };
}

#[derive(Default, Debug)]
pub struct PixModel {
    pub matrix: PixMatrix,
    columns: usize,
    rows: usize,
}

impl PixModel {
    pub fn new(columns: usize, rows: usize) -> Self {
        PixModel {
            columns,
            rows,
            matrix: vec![vec![0.0; columns + 1].into_boxed_slice(); rows].into_boxed_slice(),
        }
    }

    pub fn from_grid(model: &UiModel, space_width: f32, sign_column: usize) -> Self {
        let model = model;
        let (rows, columns) = (model.rows, model.columns);
        let mut pix_model = PixModel::new(columns, rows);
        for (row, line) in model.model().iter().enumerate() {
            pix_model.update(line, row, space_width, sign_column);
        }
        pix_model
    }

    pub fn update(&mut self, line: &Line, row: usize, space_width: f32, sign_column: usize) -> f32 {
        let pix_line = &mut self.matrix[row];
        pix_line.fill(space_width);
        for col in sign_column..self.columns {
            for item in &line.item_line[col] {
                let glyphs = item.glyphs();
                if glyphs.is_some() {
                    for glyph_string in glyphs.iter() {
                        let n = glyph_string.num_glyphs();
                        pix_line[col] = unscale!(glyph_string.width());
                        if n > 1 {
                            /* when multiple glyphs are computed as one, we set the
                             * subsequent glyphs length to zero.
                             */
                            pix_line[col + 1..col + n as usize].fill(0.0);
                        }
                    }
                }
            }
        }
        self.len_to_pos(row)
    }

    pub fn update_mono(&mut self, row: usize, space_width: f32) -> f32 {
        self.matrix[row].fill(space_width);
        self.len_to_pos(row)
    }

    fn len_to_pos(&mut self, row: usize) -> f32 {
        /* compute the horizontal position in pixel of each cell
         * we just reuse the same array here, because the previous one
         * is not usefull anymore.
         */
        let pix_line = &mut self.matrix[row];
        let mut x: f32 = 0.0;
        for i in 0..self.columns {
            let len = pix_line[i];
            pix_line[i] = x;
            x += len;
        }
        pix_line[self.columns] = pix_line[self.columns - 1];
        x
    }

    pub fn fit(&self, model: &UiModel) -> bool {
        model.columns == self.columns && model.rows == self.rows
    }
}

fn width_word_first_char(line: &Line, col: usize) -> Option<i32> {
    let glyphs = line.item_line[col].iter().nth(0)?.glyphs();
    if !glyphs.is_some() {
        return None;
    }
    Some(
        glyphs
            .iter()
            .nth(0)?
            .glyph_info()
            .iter()
            .nth(0)?
            .geometry()
            .width(),
    )
}

pub fn cursor_x(
    line: &Line,
    cursor_col: usize,
    space_width: f32,
    sign_column_len: usize,
) -> (f32, f32, f32) {
    let mut x: f32 = sign_column_len as f32 * space_width;
    let mut n_glyphs: usize = 0;
    let mut col: usize = sign_column_len;
    let mut cursor_width: f32 = space_width;
    let mut word_width_until: f32 = 0.0;
    'l: loop {
        for item in &line.item_line[col] {
            let glyphs = item.glyphs();
            if glyphs.is_some() {
                for glyph_string in glyphs.iter() {
                    let n = glyph_string.num_glyphs() as usize;
                    if col + n > cursor_col {
                        let m = cursor_col - col;
                        if m > 0 {
                            let mut new_glyph = glyph_string.clone();
                            new_glyph.set_size(m as i32);
                            let w = unscale!(new_glyph.width());
                            x += w;
                            word_width_until = w;
                            n_glyphs += m;
                            if let Some(c) = glyph_string.glyph_info().iter().nth(m) {
                                cursor_width = unscale!(c.geometry().width());
                            }
                        }
                        break 'l;
                    } else {
                        n_glyphs += n;
                        /* calling "/ pango::SCALE" each time is suboptimal
                         * but ensure that it's computed the same way as it is for
                         * line drawing
                         * */
                        x += unscale!(glyph_string.width());
                    }
                }
            }
        }
        col += 1;
        if col >= cursor_col {
            if col < line.item_line.len() {
                if let Some(width) = width_word_first_char(line, col) {
                    cursor_width = unscale!(width);
                }
            }
            break 'l;
        }
    }
    (
        cursor_width,
        x + ((cursor_col - n_glyphs - sign_column_len) as f32 * space_width),
        word_width_until,
    )
}
