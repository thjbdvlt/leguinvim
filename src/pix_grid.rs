use crate::grid::{Grid, GridMap, STATUS_GRID};
use crate::render::CellMetrics;
use crate::ui_model::Line;

use fnv::FnvHashMap;

pub const GRID_WIDTH_RATIO: f64 = 0.9;

type PixLine = Box<[i32]>;
type PixModel = Box<[PixLine]>;

pub struct PixGrid {
    pub start_x: f64,
    pub start_y: f64,
    pub width: f64,
    pub height: f64,
    pub matrix: PixModel,
}

fn new_pix_model(columns: usize, rows: usize, space_width: i32) -> PixModel {
    vec![vec![space_width; columns + 1].into_boxed_slice(); rows].into_boxed_slice()
}

impl PixGrid {
    pub fn new(grid: &Grid, cell_metrics: &CellMetrics, start_x: f64) -> PixGrid {
        let model = grid.model.model();

        /* get infos about grid position and size */
        let (rows, columns) = (grid.model.rows, grid.model.columns);
        let space_width = cell_metrics.char_width as i32;
        let start_x = start_x + grid.start_x(cell_metrics);
        let start_y = grid.start_y(cell_metrics);
        let width = grid.width(cell_metrics);
        let height = grid.height(cell_metrics);

        /* generate an empty matrix */
        let mut matrix = new_pix_model(columns, rows, space_width);

        /* get the sign column length */
        let sign_column = if rows > 0 && columns > 0 && grid.id != STATUS_GRID {
            model[0].sign_column_len()
        } else {
            0
        };

        /* compute the length for each non-space cell in each line */
        for (row, line) in model.iter().enumerate() {
            let pix_line = &mut matrix[row];
            for col in sign_column..columns {
                for item in &line.item_line[col] {
                    let glyphs = item.glyphs();
                    if glyphs.is_some() {
                        for glyph_string in glyphs.iter() {
                            let n = glyph_string.num_glyphs();
                            pix_line[col] = glyph_string.width() / pango::SCALE;
                            if n > 1 {
                                /* when multiple glyphs computed as one, we set the
                                 * subsequent glyphs length to zero
                                 * */
                                for i in 1..n as usize {
                                    pix_line[col + i] = 0;
                                }
                            }
                        }
                    }
                }
            }
            /* compute the horizontal position in pixel of each cell
             * we just reuse the same array here, because the previous one
             * is not usefull anymore
             * */
            let mut x: i32 = start_x as i32;
            for i in 0..columns {
                let len = pix_line[i];
                pix_line[i] = x;
                x += len;
            }
            pix_line[columns] = pix_line[columns - 1];
        }
        PixGrid {
            matrix,
            start_x,
            start_y,
            width,
            height,
        }
    }
}

pub struct PixGridMap {
    pub grids: FnvHashMap<u64, PixGrid>,
}

impl PixGridMap {
    pub fn get(&self, id: &u64) -> Option<&PixGrid> {
        self.grids.get(id)
    }
    pub fn new() -> Self {
        PixGridMap {
            grids: FnvHashMap::default(),
        }
    }
    pub fn insert(&mut self, grid: &Grid, cell_metrics: &CellMetrics) {
        self.grids
            .insert(grid.id, PixGrid::new(grid, cell_metrics, 0.0));
    }
    pub fn from_gridmap(gridmap: &GridMap, cell_metrics: &CellMetrics) -> Self {
        let mut pm = Self::new();
        for (_, grid) in gridmap.grids.iter().filter(|(_, g)| !g.hidden) {
            pm.insert(grid, cell_metrics);
        }
        pm
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
    space_width: i32,
    sign_column_len: usize,
) -> (i32, i32, i32) {
    let mut x: i32 = sign_column_len as i32 * space_width;
    let mut n_glyphs: usize = 0;
    let mut col: usize = sign_column_len;
    let mut cursor_width: i32 = space_width;
    let mut word_width_until: i32 = 0;
    'l: loop {
        for item in &line.item_line[col] {
            let glyphs = item.glyphs();
            if glyphs.is_some() {
                for glyph_string in glyphs.iter() {
                    let n = glyph_string.num_glyphs() as usize;
                    if col + n > cursor_col {
                        let m = cursor_col - col;
                        if m as i32 > 0 {
                            let mut new_glyph = glyph_string.clone();
                            new_glyph.set_size(m as i32);
                            x += new_glyph.width() / pango::SCALE;
                            word_width_until = new_glyph.width() / pango::SCALE;
                            n_glyphs += m;
                            if let Some(c) = glyph_string.glyph_info().iter().nth(m) {
                                cursor_width = c.geometry().width() / pango::SCALE;
                            }
                        }
                        break 'l;
                    } else {
                        n_glyphs += n;
                        /* calling "/ pango::SCALE" each time is suboptimal
                         * but ensure that it's computed the same way as it is for
                         * line drawing
                         * */
                        x += glyph_string.width() / pango::SCALE;
                    }
                }
            }
        }
        col += 1;
        if col >= cursor_col {
            if col < line.item_line.len() {
                if let Some(width) = width_word_first_char(line, col) {
                    cursor_width = width / pango::SCALE;
                }
            }
            break 'l;
        }
    }
    (
        cursor_width,
        x + ((cursor_col - n_glyphs - sign_column_len) as i32 * space_width),
        word_width_until,
    )
}
