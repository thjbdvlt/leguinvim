use std::ops::{Index, IndexMut};
use std::rc::Rc;

use fnv::FnvHashMap;

use crate::highlight::{Highlight, HighlightMap};
use crate::pix_grid::{GRID_WIDTH_RATIO, PixModel};
use crate::render::CellMetrics;
use crate::ui_model::{ModelRect, UiModel};
use nvim_rs::Value;

pub const STATUS_GRID: u64 = 1;

pub struct GridMap {
    pub grids: FnvHashMap<u64, Grid>,
    pub pmenu: Grid,
}

impl Index<u64> for GridMap {
    type Output = Grid;

    fn index(&self, idx: u64) -> &Grid {
        &self.grids[&idx]
    }
}

impl IndexMut<u64> for GridMap {
    fn index_mut(&mut self, idx: u64) -> &mut Grid {
        self.grids.get_mut(&idx).unwrap()
    }
}

impl GridMap {
    pub fn new() -> Self {
        GridMap {
            grids: FnvHashMap::default(),
            pmenu: Grid::new_pmenu(),
        }
    }

    pub fn get(&self, grid_id: u64) -> Option<&Grid> {
        self.grids.get(&grid_id)
    }

    pub fn get_model_mut(&mut self, grid_id: u64) -> Option<&mut UiModel> {
        self.grids.get_mut(&grid_id).map(|g| &mut g.model)
    }

    pub fn get_model(&self, grid_id: u64) -> Option<&UiModel> {
        self.grids.get(&grid_id).map(|g| &g.model)
    }

    pub fn get_or_create(&mut self, idx: u64) -> &mut Grid {
        if self.grids.contains_key(&idx) {
            return self.grids.get_mut(&idx).unwrap();
        }

        self.grids.insert(idx, Grid::new(idx));
        self.grids.get_mut(&idx).unwrap()
    }

    pub fn destroy(&mut self, idx: u64) {
        self.grids.remove(&idx);
    }

    pub fn clear_glyphs(&mut self) {
        for grid in self.grids.values_mut() {
            grid.model.clear_glyphs();
        }
    }

    pub fn pmenu_mut(&mut self) -> &mut Grid {
        &mut self.pmenu
    }

    pub fn pmenu(&self) -> &Grid {
        &self.pmenu
    }

    pub fn get_grid_pos(&self, grid_id: u64) -> Option<(i64, i64, i64, i64)> {
        let grid = self.get(grid_id)?;
        Some((
            grid.start_row,
            grid.start_col,
            grid.rows() as i64,
            grid.columns() as i64,
        ))
    }

    pub fn sorted_visible(&self) -> Vec<(&u64, &Grid)> {
        let mut grids: Vec<(&u64, &Grid)> = self.iter_visible().collect();
        grids.sort_by_key(|(id, g)| (g.zindex, *id));
        grids
    }

    pub fn iter_visible(&self) -> impl Iterator<Item = (&u64, &Grid)> {
        self.grids.iter().filter(|(_, g)| !g.hidden)
    }

    /// Flush any pending cursor position updates (e.g. updates we received before getting a 'flush'
    /// event)
    pub fn flush_cursor(&mut self) {
        for grid in self.grids.values_mut() {
            grid.flush_cursor();
        }
    }
}

#[derive(Default, Debug)]
pub struct Grid {
    pub model: UiModel,
    pub id: u64,
    pub hidden: bool,
    pub start_row: i64,
    pub start_col: i64,

    pub floating: bool,
    pub zindex: u64,
    pub compindex: u64,
    pub anchor: String,
    pub anchor_grid_id: u64,
    pub anchor_pos: (i64, i64),
    pub border_removed: bool,
    pub monospace: bool,

    pub border: [bool; 4],
    pub pix: PixModel,
    pub rect: (f32, f32, f32, f32),
}

impl Grid {
    pub fn new(id: u64) -> Self {
        Grid {
            id,
            ..Grid::default()
        }
    }

    pub fn new_pmenu() -> Self {
        Grid {
            hidden: true,
            floating: true,
            zindex: 1000,
            border_removed: true,
            border: [true, true, true, true],
            ..Grid::default()
        }
    }

    pub fn set_pos(&mut self, start_row: i64, start_col: i64) {
        self.start_row = start_row;
        self.start_col = start_col;
        self.border = [start_row > 0, false, false, start_col > 0];
    }

    pub fn set_float_pos(
        &mut self,
        row: i64,
        col: i64,
        zindex: u64,
        anchor_grid: u64,
        monospace: bool,
    ) {
        self.start_row = row;
        self.start_col = col;
        self.zindex = zindex;
        self.floating = true;
        self.border.fill(true);
        self.anchor_grid_id = anchor_grid;
        self.monospace = monospace;
    }

    pub fn get_cursor(&self) -> (usize, usize) {
        self.model.get_real_cursor()
    }

    pub fn flush_cursor(&mut self) {
        self.model.flush_cursor();
    }

    pub fn cur_point(&self) -> ModelRect {
        self.model.cur_real_point()
    }

    pub fn columns(&self) -> usize {
        self.model.columns
    }

    pub fn rows(&self) -> usize {
        self.model.rows
    }

    pub fn any_border(&self) -> bool {
        for b in self.border {
            if b {
                return true;
            }
        }
        false
    }

    pub fn resize(&mut self, columns: usize, rows: usize) {
        if self.model.columns != columns || self.model.rows != rows {
            self.model = self.model.resized(columns, rows);
            self.pix = PixModel::new(columns, rows);
        }
    }

    pub fn clear_content(&mut self) {
        self.model = UiModel::new(self.model.rows as u64, self.model.columns as u64);
    }

    pub fn cursor_goto(&mut self, row: usize, col: usize) {
        self.model.set_cursor(row, col)
    }

    pub fn clear(&mut self, default_hl: &Rc<Highlight>) {
        self.model.clear(default_hl);
    }

    pub fn start_x(&self, cell_metrics: &CellMetrics) -> f64 {
        if self.monospace {
            self.start_col as f64 * cell_metrics.char_width
        } else {
            self.start_col as f64 * cell_metrics.char_width / GRID_WIDTH_RATIO
        }
    }

    pub fn start_y(&self, cell_metrics: &CellMetrics) -> f64 {
        self.start_row as f64 * cell_metrics.line_height
    }

    pub fn width(&self, cell_metrics: &CellMetrics) -> f64 {
        if self.monospace {
            self.model.columns as f64 * cell_metrics.char_width
        } else {
            self.model.columns as f64 * cell_metrics.char_width / GRID_WIDTH_RATIO
        }
    }

    pub fn set_rect(&mut self, cell_metrics: &CellMetrics) {
        self.rect = (
            self.start_x(cell_metrics) as f32,
            self.start_y(cell_metrics) as f32,
            self.width(cell_metrics) as f32,
            self.height(cell_metrics) as f32,
        );
    }

    pub fn height(&self, cell_metrics: &CellMetrics) -> f64 {
        self.model.rows as f64 * cell_metrics.line_height
    }

    pub fn sign_column_len(&self) -> usize {
        if self.id == STATUS_GRID {
            0
        } else {
            self.model.sign_column_len()
        }
    }

    #[allow(clippy::get_first)] // get(0), get(1), get(2) more consistent than .first(), .get(1)
    pub fn line(
        &mut self,
        row: usize,
        col_start: usize,
        cells: Vec<Vec<Value>>,
        highlights: &HighlightMap,
    ) {
        let mut hl_id = None;
        let mut col_end = col_start;

        for cell in cells {
            let ch = cell.get(0).unwrap().as_str().unwrap_or("");
            hl_id = cell.get(1).and_then(|h| h.as_u64()).or(hl_id);
            let repeat = cell.get(2).and_then(|r| r.as_u64()).unwrap_or(1) as usize;

            self.model.put(
                row,
                col_end,
                ch,
                ch.is_empty(),
                repeat,
                highlights.get(hl_id),
            );
            col_end += repeat;
        }
    }

    pub fn complete_items(
        &mut self,
        items: &[[String; 4]],
        selected: i64,
        reverse: bool,
        n_items: usize,
        hl: &HighlightMap,
    ) {
        let (mut row, step, selected) = if reverse {
            let last_item = n_items as i64 - 1;
            (last_item, -1, last_item - selected)
        } else {
            (0, 1, selected)
        };
        for item in items.iter() {
            self.model.put(
                row as usize,
                2, // padding
                // TODO completion: put also other fields?
                // maybe it doesn't matter, since for prose writing
                // we're not so much interested in "types" such as "method" or "const".
                &item[0],
                false,
                1,
                if row == selected {
                    hl.pmenu_sel.clone()
                } else {
                    hl.pmenu.clone()
                },
            );
            row += step;
        }
    }

    pub fn scroll(
        &mut self,
        top: u64,
        bot: u64,
        left: u64,
        right: u64,
        rows: i64,
        _: i64,
        default_hl: &Rc<Highlight>,
    ) {
        self.model.scroll(
            top as i64,
            bot as i64 - 1,
            left as usize,
            right as usize - 1,
            rows,
            default_hl,
        )
    }
}
