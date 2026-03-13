use crate::ui_model::{Line, UiModel};

pub const GRID_WIDTH_RATIO: f64 = 0.9;

pub type PixLine = Box<[f32]>;
type PixMatrix = Box<[PixLine]>;

macro_rules! unscale {
    ($e: expr) => {
        ($e / pango::SCALE) as f32
    };
}

/// Store informations about positions and length of characters in pixels
#[derive(Default, Debug)]
pub struct PixModel {
    /// pixel 2d array
    pub matrix: PixMatrix,
    /// number of columns in neovim grid
    pub columns: usize,
    /// number of rows in neovim grid
    pub rows: usize,
}

impl PixModel {
    /// Create an empty PixModel at a specific size
    pub fn new(columns: usize, rows: usize) -> Self {
        PixModel {
            columns,
            rows,
            matrix: vec![vec![0.0; columns + 1].into_boxed_slice(); rows].into_boxed_slice(),
        }
    }

    /// Create and populate a PixModel from a UiModel and font informations
    pub fn from_grid(
        model: &UiModel,
        char_width: f32,
        space_width: f32,
        sign_column: usize,
    ) -> Self {
        let mut pix_model = PixModel::new(model.columns, model.rows);
        for (row, line) in model.model().iter().enumerate() {
            pix_model.update(line, row, char_width, space_width, sign_column);
        }
        pix_model
    }

    /// Update all characters' pixel positions in a line
    pub fn update(
        &mut self,
        line: &Line,
        row: usize,
        char_width: f32,
        space_width: f32,
        sign_column: usize,
    ) -> f32 {
        let pix_line = &mut self.matrix[row];
        // we first fill with space width because... spaces aren't characters, so we would never be able to compute their width.
        pix_line.fill(space_width);
        pix_line[0..sign_column].fill(char_width);
        // we need to know the last non-space's position to draw backgrounds
        let mut last_non_space: usize = sign_column;
        for col in sign_column..self.columns {
            // TODO: ensure that there's no issue with this loop on `iteml_line[col].` because `pix_line[col]` seems to overwrite on last iteration.
            for item in &line.item_line[col] {
                if let Some(glyph_string) = item.glyphs().as_ref() {
                    let n = item.item.num_chars() as usize;
                    pix_line[col] = unscale!(glyph_string.width());
                    last_non_space = col + n;
                    if n > 1 {
                        // when multiple glyphs are computed as one, we set the subsequent glyphs length to zero.
                        pix_line[col + 1..last_non_space].fill(0.0);
                    }
                }
            }
        }
        // the previous loop computes characters width. we now need to additionate these width to get pixel indexes.
        self.len_to_pos(row, last_non_space)
    }

    /// Update pixel position of monospace font line. This is obviously simplier.
    pub fn update_mono(&mut self, row: usize, space_width: f32) -> f32 {
        self.matrix[row].fill(space_width);
        self.len_to_pos(row, self.columns)
    }

    /// Compute the horizontal position in pixel of each cell.
    fn len_to_pos(&mut self, row: usize, last_non_space: usize) -> f32 {
        // we just reuse the same array here, because the previous one is not usefull anymore.
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

    /// Does this PixModel fits this UiModel, i.e. do they have the exact same size?
    pub fn fit(&self, model: &UiModel) -> bool {
        model.columns == self.columns && model.rows == self.rows
    }
}

fn find_word_start_index(pix_line: &PixLine, cursor_col: usize) -> usize {
    let mut i = cursor_col + 1;
    while i > 0 && pix_line[i] == pix_line[i - 1] {
        i -= 1;
    }
    if i > 0 { i - 1 } else { i }
}

/// Get cursor x position in pixels.
/// This function returns three values:
/// - cursor width
/// - cursor x position in pixel
/// - the x position in pixel of the word the cursor is in
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
        eprintln!("cursor on no glyph"); // there's no reason for this to ever happen
        return (space_width, x, 0.0);
    }

    let Some(glyph_string) = glyphs.iter().next() else {
        // empty line
        return (space_width, x, 0.0);
    };

    if col + n > cursor_col {
        let analysis = item.analysis();

        // We can't use the same technique as in PixLine::update, because items are typically grouped in words and here we need to analyse position and width inside a word.
        // TODO: optimize
        let mut s = String::new();
        for cell in line.line[col..cursor_col].iter() {
            s.push_str(&cell.ch);
        }
        let start = unscale!(glyph_string.index_to_x(&s, analysis, s.len() as i32, false));
        let mut glyphs = pango::GlyphString::new();
        pango::shape(&line.line[cursor_col].ch, analysis, &mut glyphs);
        cursor_width = unscale!(glyphs.width());
        x += start;
        prefix = start;
    }

    (cursor_width, x, prefix)
}
