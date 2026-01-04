use nvim_rs::Value;

use crate::grid::Grid;
use crate::highlight::HighlightMap;

macro_rules! next_str {
    ($iter: expr) => {
        String::from(($iter).next()?.as_str()?)
    };
}

fn into_completion_item(item: &Value) -> Option<[String; 4]> {
    let mut iter = item.as_array()?.into_iter();
    let arr: [String; 4] = [
        next_str!(iter), // word
        next_str!(iter), // kind
        next_str!(iter), // menu
        next_str!(iter), // info
    ];
    Some(arr)
}

pub struct Pmenu {
    pub items: Vec<[String; 4]>,
    pub sel: i64,
    pub anchor_grid_id: u64,
}

fn longest(items: &[[String; 4]]) -> usize {
    let mut longest: usize = 0;
    for i in items {
        let len = i[0].len();
        if len > longest {
            longest = len;
        }
    }
    longest + (longest / 2) + 4 // two space left, two space right
}

impl Pmenu {
    pub fn new() -> Self {
        Pmenu {
            items: Vec::new(),
            sel: -1,
            anchor_grid_id: 0,
        }
    }

    pub fn parse_items(&mut self, items: &Value) {
        let Some(items) = items.as_array() else {
            self.items = Vec::new();
            return;
        };
        self.items = items.into_iter()
            // we don't filter items, else "selected" index could be wrong.
            .map(|i| into_completion_item(i).unwrap_or([
                String::from(""),
                String::from(""),
                String::from(""),
                String::from(""),
            ]))
            .collect();
    }

    pub fn set_anchor_grid_id(&mut self, grid: u64) {
        self.anchor_grid_id = grid;
    }

    pub fn set_sel(&mut self, sel: i64) {
        self.sel = sel;
    }

    pub fn get_items(&self, rows: usize) -> Option<(i64, &[[String; 4]])> {
        let n_items = self.len();
        let sel = std::cmp::max(0, self.sel) as usize;
        let after_sel = n_items - sel;
        if n_items <= rows {
            Some((self.sel, &self.items))
        } else if after_sel < rows {
            let first = sel - (rows - after_sel);
            Some(((sel - first) as i64, &self.items[first..first + rows]))
        } else {
            Some((0, &self.items[sel..sel + rows]))
        }
    }

    pub fn put(&self, pmenu_grid: &mut Grid, rows: usize, hl: &HighlightMap) {
        let rows = rows - pmenu_grid.start_row as usize;
        if let Some((sel_idx, items)) = self.get_items(rows) {
            let columns = longest(items);
            pmenu_grid.resize(columns, items.len());
            pmenu_grid.complete_items(items, sel_idx, hl);
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}
