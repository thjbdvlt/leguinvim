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
    pub columns: usize,
    pub rows: usize,
}

impl PixModel {
    pub fn new(columns: usize, rows: usize) -> Self {
        PixModel {
            columns,
            rows,
            matrix: vec![vec![0.0; columns + 1].into_boxed_slice(); rows].into_boxed_slice(),
        }
    }

    pub fn from_grid(
        model: &UiModel,
        char_width: f32,
        space_width: f32,
        sign_column: usize,
    ) -> Self {
        let (rows, columns) = (model.rows, model.columns);
        let mut pix_model = PixModel::new(columns, rows);
        for (row, line) in model.model().iter().enumerate() {
            pix_model.update(line, row, char_width, space_width, sign_column);
        }
        pix_model
    }

    pub fn update(
        &mut self,
        line: &Line,
        row: usize,
        char_width: f32,
        space_width: f32,
        sign_column: usize,
    ) -> f32 {
        let pix_line = &mut self.matrix[row];
        pix_line.fill(space_width);
        pix_line[0..sign_column].fill(char_width);
        let mut last_non_space: usize = sign_column;
        for col in sign_column..self.columns {
            for item in &line.item_line[col] {
                let glyphs = item.glyphs();
                if glyphs.is_some() {
                    for glyph_string in glyphs.iter() {
                        let n = item.item.num_chars() as usize;
                        pix_line[col] = unscale!(glyph_string.width());
                        last_non_space = col + n;
                        if n > 1 {
                            /* when multiple glyphs are computed as one, we set the
                             * subsequent glyphs length to zero.
                             */
                            pix_line[col + 1..last_non_space].fill(0.0);
                        }
                    }
                }
            }
        }
        self.len_to_pos(row, last_non_space)
    }

    pub fn update_mono(&mut self, row: usize, space_width: f32) -> f32 {
        self.matrix[row].fill(space_width);
        self.len_to_pos(row, self.columns)
    }

    fn len_to_pos(&mut self, row: usize, last_non_space: usize) -> f32 {
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
        pix_line[last_non_space]
    }

    pub fn fit(&self, model: &UiModel) -> bool {
        model.columns == self.columns && model.rows == self.rows
    }
}

fn line_str(line: &Line, start_index: usize, end_index: usize) -> String {
    let mut s = String::new();
    for i in start_index..end_index {
        s.push_str(&line.line[i].ch);
    }
    s
}

fn find_word_start_index(pix_line: &PixLine, cursor_col: usize) -> usize {
    let mut i = cursor_col + 1;
    while i > 0 && pix_line[i] == pix_line[i - 1] {
        i -= 1;
    }
    if i > 0 { i - 1 } else { i }
}

pub fn cursor_x(
    line: &Line,
    pix_line: &PixLine,
    cursor_col: usize,
    space_width: f32,
) -> (f32, f32, f32) {
    let mut cursor_width: f32 = space_width;
    let mut prefix: f32 = 0.0;

    let col = find_word_start_index(pix_line, cursor_col);

    let item_line = &line.item_line[col];
    if item_line.is_empty() {
        return (space_width, pix_line[col], 0.0);
    }

    let item = &item_line[0]; // Never more than 1
    let n = item.item.num_chars() as usize;
    let glyphs = item.glyphs();
    let mut x = pix_line[col];
    if n == 0 || glyphs.is_none() {
        eprintln!("cursor on no glyph"); // this shouldn't happen
        return (space_width, x, 0.0);
    }

    let Some(glyph_string) = glyphs.iter().next() else {
        return (space_width, x, 0.0);
    };

    if col + n > cursor_col {
        let m = cursor_col - col;
        let text = line_str(line, col, col + n);
        let analysis = item.analysis();
        let (start, end) = (
            unscale!(glyph_string.index_to_x(&text, analysis, m as i32, false)),
            unscale!(glyph_string.index_to_x(&text, analysis, m as i32, true)),
        );
        cursor_width = end - start;
        x += start;
        prefix = start;
    }

    (cursor_width, x, prefix)
}
