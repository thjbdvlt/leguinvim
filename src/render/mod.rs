mod context;
mod itemize;

pub use self::context::{CellMetrics, Context, FontFeatures, SubCtx};

use log::warn;

use crate::{
    color,
    cursor::{Cursor, CursorRedrawCb, cursor_rect},
    grid::{Grid, GridMap},
    highlight::HighlightMap,
    pix_grid::{PixLine, PixModel, cursor_x},
    shell::TransparencySettings,
    ui_model::{self, Line},
};

use gtk::{graphene::Rect, prelude::*};

/// A single step in a render plan
#[derive(Clone, Copy)]
struct RenderStep<'a> {
    color: &'a color::Color,
    kind: RenderStepKind,
    len: usize,          // (in cells)
    pos: (usize, usize), // (rows, cols)
}

impl<'a> RenderStep<'a> {
    pub fn new(kind: RenderStepKind, color: &'a color::Color, pos: (usize, usize)) -> Self {
        Self {
            color,
            kind,
            pos,
            len: 1,
        }
    }

    fn to_snapshot(
        self,
        snapshot: &gtk::Snapshot,
        cell_metrics: &CellMetrics,
        line_x: &PixLine,
        start_x: f64,
    ) {
        let (start_row, start_col) = self.pos;
        let x = line_x[start_col] as f64;
        let y = start_row as f64 * cell_metrics.line_height;
        let len = line_x[start_col + self.len] as f64 - x;
        let f = match self.kind {
            RenderStepKind::Background => snapshot_bg,
            RenderStepKind::Strikethrough => snapshot_strikethrough,
            RenderStepKind::Underline => snapshot_underline,
            RenderStepKind::Underdouble => snapshot_underdouble,
            RenderStepKind::Underdot => snapshot_underdot,
            RenderStepKind::Underdash => snapshot_underdash,
        };
        f(snapshot, cell_metrics, self.color, start_x + x, y, len);
    }

    #[inline]
    pub fn extend(&mut self, kind: RenderStepKind, color: &'a color::Color) -> bool {
        if kind == self.kind && *color == *self.color {
            self.len += 1;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderStepKind {
    Background,
    Underline,
    Underdouble,
    Underdot,
    Underdash,
    Strikethrough,
}

pub fn snapshot_all_grids(
    cell_metrics: &CellMetrics,
    mono_metrics: &CellMetrics,
    gridmap: &GridMap,
    hl: &HighlightMap,
) -> Option<gsk::RenderNode> {
    let mut snapshot = gtk::Snapshot::new();
    let grids = gridmap.sorted_visible();
    for (_, grid) in grids.iter() {
        let cm = if grid.monospace {
            mono_metrics
        } else {
            cell_metrics
        };
        snapshot_grid(&mut snapshot, cm, &grid, hl);
    }
    snapshot_pmenu(&mut snapshot, gridmap, cell_metrics, hl);
    snapshot.to_node()
}

fn grid_bg(snapshot: &mut gtk::Snapshot, grid: &Grid, bg: &color::Color) {
    let (start_x, start_y, width, height) = grid.rect;
    snapshot.append_color(&bg.into(), &Rect::new(start_x, start_y, width, height));
}

pub fn grid_border(snapshot: &mut gtk::Snapshot, grid: &Grid, hl: &HighlightMap) {
    const BORDER_WIDTH: f32 = 1.0;
    let (start_x, start_y, width, height) = grid.rect;
    let borders = [
        (0.0, -BORDER_WIDTH, width, BORDER_WIDTH),  // top
        (width, 0.0, BORDER_WIDTH, height),         // right
        (0.0, height, width, BORDER_WIDTH),         // bottom
        (-BORDER_WIDTH, 0.0, BORDER_WIDTH, height), // left
    ];
    for (i, b) in grid.border.iter().enumerate() {
        if *b {
            let (x, y, width, height) = borders[i];
            snapshot.append_color(
                &hl.fg().into(),
                &Rect::new(start_x + x, start_y + y, width, height),
            )
        }
    }
}

fn snapshot_grid(
    snapshot: &mut gtk::Snapshot,
    cell_metrics: &CellMetrics,
    grid: &Grid,
    hl: &HighlightMap,
) {
    if grid.model.rows == 0 {
        return;
    }

    let ui_model = &grid.model;

    grid_bg(snapshot, grid, hl.bg());

    if grid.any_border() {
        grid_border(snapshot, grid, hl);
    }

    // Various operations for text formatting come at the end, so store them in a list until then.
    // We set the capacity to three times the size of the grid, since at most each cell can have
    // strikethrough + a type of underline + the second line in an underdouble. Most of the time
    // though, we optimistically expect this list to be much smaller than iterating through the UI
    // model.
    let mut text_fmt_steps = Vec::with_capacity(ui_model.columns * ui_model.rows * 3);

    // Note that we group each batch of nodes based on their type, since GTK+ does a better job of
    // optimizing contiguous series of similar drawing operations (source: Company)
    let model = ui_model.model();

    let start_x = grid.rect.0 as f64;
    for (row, line) in model.iter().enumerate() {
        let mut pending_bg = None;
        let mut pending_strikethrough = None;
        let mut pending_underline = None;
        let mut pending_underdouble = None;

        let global_row = row + grid.start_row as usize;
        for (col, cell) in line.line.iter().enumerate() {
            let pos = (global_row, col);
            // Plan each step of the process of creating this snapshot as much as possible. We use
            // the term "plan" to describe the process of generating as few snapshot nodes as
            // possible in order to accomplish each step in creating a snapshot for the final image.
            // For example, if multiple adjacent background nodes have identical background colors -
            // our planning phase will generate a single snapshot node for any contiguous group of
            // cells. Where possible, we immediately generate nodes and add them to the snapshot.
            //
            // Currently, the only step of the rendering process we don't do this for is with
            // text nodes - where we rely on the itemization process to have already done this for
            // us. Additionally, all optimizations are limited to each row. We do not for instance,
            // combine the background nodes of multiple identical adjacent rows.
            plan_and_snapshot_cell_bg(
                &snapshot,
                &mut pending_bg,
                hl,
                cell,
                cell_metrics,
                pos,
                &grid.pix.matrix[row],
                start_x,
            );
            plan_underline_strikethrough(
                &mut pending_strikethrough,
                &mut pending_underline,
                &mut pending_underdouble,
                &mut text_fmt_steps,
                hl,
                cell,
                pos,
            );
        }

        // Since background nodes come first, we can add them to the snapshot immediately
        if let Some(pending_bg) = pending_bg {
            pending_bg.to_snapshot(&snapshot, cell_metrics, &grid.pix.matrix[row], start_x);
        }
    }

    snapshot_text(snapshot, cell_metrics, model, grid, &grid.pix, hl);

    for step in text_fmt_steps.into_iter() {
        let row = step.pos.0 - grid.start_row as usize;
        step.to_snapshot(&snapshot, cell_metrics, &grid.pix.matrix[row], start_x);
    }
}

fn snapshot_text(
    snapshot: &mut gtk::Snapshot,
    cell_metrics: &CellMetrics,
    model: &[Line],
    grid: &Grid,
    pix: &PixModel,
    hl: &HighlightMap,
) {
    let line_height = cell_metrics.line_height as f32;
    let (start_x, mut y) = (grid.rect.0, grid.rect.1 + cell_metrics.ascent as f32);
    for (row, line) in model.iter().enumerate() {
        let pix_row = &pix.matrix[row];
        for (col, cell) in line.line.iter().enumerate() {
            snapshot_cell(
                &snapshot,
                &line.item_line[col],
                hl,
                cell,
                start_x + pix_row[col] as f32,
                y,
            );
        }
        y += line_height;
    }
}

fn snapshot_pmenu(
    snapshot: &mut gtk::Snapshot,
    gridmap: &GridMap,
    cell_metrics: &CellMetrics,
    hl: &HighlightMap,
) {
    let pmenu = gridmap.pmenu();
    if pmenu.hidden {
        return;
    }
    let model = pmenu.model.model();
    let pmenu_pix_grid = PixModel::from_grid(
        &pmenu.model,
        cell_metrics.char_width as f32,
        cell_metrics.space_width as f32,
        0,
    );
    grid_bg(snapshot, pmenu, hl.bg());
    snapshot_text(snapshot, cell_metrics, model, pmenu, &pmenu_pix_grid, hl);
    grid_border(snapshot, pmenu, hl);
}

pub fn snapshot_cursor<T: CursorRedrawCb + 'static>(
    snapshot: &gtk::Snapshot,
    cursor: &Cursor<T>,
    font_ctx: &Context,
    mono_ctx: &Context,
    grid: &Grid,
    hl: &HighlightMap,
    transparency: TransparencySettings,
) {
    if !cursor.is_visible() {
        return;
    }

    let ctx = if grid.monospace { mono_ctx } else { font_ctx };

    let cell_metrics = ctx.cell_metrics();

    let ui_model = &grid.model;
    let (cursor_row, cursor_col) = ui_model.get_flushed_cursor();

    let y = cell_metrics.get_y(cursor_row) + (grid.start_row as f64 * cell_metrics.line_height);

    let cursor_line = match ui_model.model().get(cursor_row) {
        Some(cursor_line) => cursor_line,
        None => return,
    };

    let char_width = cell_metrics.char_width as f32;
    let space_width = cell_metrics.space_width as f32;

    let (pixel_width, x, until_x) = cursor_x(
        cursor_line,
        cursor_col,
        char_width,
        space_width,
        grid.sign_column_len(),
    );
    let x = x + grid.rect.0;

    let fade_percentage = cursor.alpha();
    let cell = &cursor_line.line[cursor_col];

    let (clip_y, clip_width, clip_height) =
        cursor_rect(cursor.mode_info(), cell_metrics, y, pixel_width as f64);
    let clip_rect = Rect::new(x, clip_y as f32, clip_width as f32, clip_height as f32);
    let x = x as f64;

    let bg_alpha = transparency.background_alpha;
    let filled_alpha = transparency.filled_alpha;
    let alpha = bg_alpha + ((filled_alpha - bg_alpha) * fade_percentage);
    let is_focused = cursor.is_focused();
    if is_focused {
        snapshot.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0), &clip_rect);
    }

    cursor.snapshot(
        snapshot,
        ctx,
        (x, y),
        cell,
        hl,
        fade_percentage,
        alpha,
        pixel_width as f64,
    );

    // Skip re-rendering cell if it isn't needed
    if !is_focused {
        return;
    }

    let cell_start_col = cursor_line.cell_to_item(cursor_col);
    let fg = hl.actual_cell_fg(cell).fade(hl.bg(), fade_percentage);

    if cell_start_col >= 0 {
        snapshot.push_clip(&clip_rect);
        let cell_start_line_x = x as f64 - until_x as f64;
        for item in &*cursor_line.item_line[cell_start_col as usize] {
            if item.glyphs().is_some() {
                if let Some(ref render_node) = item.new_render_node(
                    &fg,
                    (cell_start_line_x as f32, (y + cell_metrics.ascent) as f32),
                ) {
                    snapshot.append_node(render_node);
                }
            }
        }

        snapshot.pop();
    }

    if cell.hl.strikethrough {
        snapshot_strikethrough(snapshot, cell_metrics, &fg, x, y, clip_width);
    }

    if cell.hl.underdashed {
        snapshot_underdash(
            snapshot,
            cell_metrics,
            &underline_color(cell, hl).fade(hl.bg(), fade_percentage),
            x,
            y,
            clip_width,
        );
    } else if cell.hl.underdotted {
        snapshot_underdot(
            snapshot,
            cell_metrics,
            &underdotted_color(cell, hl).fade(hl.bg(), fade_percentage),
            x,
            y,
            clip_width,
        );
    } else if cell.hl.underline {
        snapshot_underline(
            snapshot,
            cell_metrics,
            &underline_color(cell, hl).fade(hl.bg(), fade_percentage),
            x,
            y,
            clip_width,
        );
    }

    if cell.hl.underdouble {
        snapshot_underdouble(
            snapshot,
            cell_metrics,
            &underline_color(cell, hl).fade(hl.bg(), fade_percentage),
            x,
            y,
            clip_width,
        );
    }
}

fn snapshot_strikethrough(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    x: f64,
    y: f64,
    len: f64,
) {
    snapshot.append_color(
        &color.into(),
        &Rect::new(
            x as f32,
            (y + cell_metrics.strikethrough_position) as f32,
            len as f32,
            cell_metrics.strikethrough_thickness as f32,
        ),
    )
}

fn underline_rect(cell_metrics: &CellMetrics, (x, y): (f64, f64), len: f64) -> Rect {
    Rect::new(
        x as f32,
        (y + cell_metrics.underline_position) as f32,
        len as f32,
        cell_metrics.underline_thickness as f32,
    )
}

fn snapshot_underline(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    x: f64,
    y: f64,
    len: f64,
) {
    snapshot.append_color(&color.into(), &underline_rect(cell_metrics, (x, y), len))
}

fn snapshot_underdouble(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    x: f64,
    y: f64,
    len: f64,
) {
    /* We only need to handle the lower underline, the upper underline will be drawn by
     * snapshot_underline()
     */
    snapshot.append_color(
        &color.into(),
        &underline_rect(cell_metrics, (x, y), len)
            .offset_r(0.0, (cell_metrics.underline_thickness * 2.0) as f32),
    )
}

fn snapshot_underdot(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    x: f64,
    mut y: f64,
    len: f64,
) {
    let CellMetrics {
        underline_position,
        /* Ideally we always want to make sure that the underdot comes out as a distinct set of
         * repeating dots, always equally spaced, and as large as possible within the descent area
         * starting from the underline position to the bottom of the line.
         */
        underline_thickness: diameter,
        ..
    } = *cell_metrics;

    /* We also want to make sure that each dot starts on an X coordinate that's a multiple of the
     * width of the segment that we'll be repeating, in order to avoid the spacing between dots of
     * different colors from ever looking inconsistent. This can mean we'll sometimes only draw a
     * portion of a dot, but that generally looks nicer then the inconsistent alternative.
     */
    let start_x = x - (x % (diameter * 2.0));

    y = (y + underline_position).floor();

    let rect = Rect::new(x as f32, y as f32, len as f32, diameter as f32);
    snapshot.push_repeat(
        &rect,
        Some(&Rect::new(
            start_x as f32,
            y as f32,
            (diameter * 2.0) as f32,
            diameter as f32,
        )),
    );

    let dot = gsk::RoundedRect::from_rect(
        Rect::new(start_x as f32, y as f32, diameter as f32, diameter as f32),
        (diameter / 2.0) as f32,
    );
    snapshot.push_rounded_clip(&dot);
    snapshot.append_color(&color.into(), dot.bounds());
    snapshot.pop();

    snapshot.pop();
}

fn snapshot_underdash(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    x: f64,
    mut y: f64,
    len: f64,
) {
    let CellMetrics {
        underline_position,
        underline_thickness,
        char_width,
        ..
    } = *cell_metrics;

    /* Same trick as in snapshot_underdot(). Note that we need to make sure to round the dash width
     * though, because otherwise subpixel rendering and floating point imprecision will cause dashes
     * to become misaligned as they're repeated
     *
     * It would be nice not to have to round them though 🙄
     */
    let dash_width = (char_width / 3.0).round();
    let pattern_width = dash_width * 2.0;
    let start_x = x - (x % pattern_width);

    y = (y + underline_position).floor();

    let (x, start_x, y, len, underline_thickness) = (
        x as f32,
        start_x as f32,
        y as f32,
        len as f32,
        underline_thickness as f32,
    );

    snapshot.push_repeat(
        &Rect::new(x, y, len, underline_thickness),
        Some(&Rect::new(
            start_x,
            y,
            (dash_width * 2.0) as f32,
            underline_thickness,
        )),
    );
    snapshot.append_color(
        &color.into(),
        &Rect::new(start_x, y, dash_width as f32, underline_thickness),
    );
    snapshot.pop();
}

fn plan_and_snapshot_cell_bg<'a>(
    snapshot: &gtk::Snapshot,
    pending_bg: &mut Option<RenderStep<'a>>,
    hl: &'a HighlightMap,
    cell: &'a ui_model::Cell,
    cell_metrics: &CellMetrics,
    pos: (usize, usize),
    line_x: &PixLine,
    start_x: f64,
) {
    if let Some(cell_bg) = hl.cell_bg(cell).filter(|bg| *bg != hl.bg()) {
        if let Some(cur_pending_bg) = pending_bg {
            if cur_pending_bg.extend(RenderStepKind::Background, cell_bg) {
                return;
            }
            cur_pending_bg.to_snapshot(snapshot, cell_metrics, line_x, start_x);
        }
        *pending_bg = Some(RenderStep::new(RenderStepKind::Background, cell_bg, pos));
    } else if let Some(pending_bg) = pending_bg.take() {
        pending_bg.to_snapshot(snapshot, cell_metrics, line_x, start_x);
    }
}

fn underline_color<'a>(cell: &'a ui_model::Cell, hl: &'a HighlightMap) -> &'a color::Color {
    hl.cell_sp(cell).unwrap_or_else(|| hl.actual_cell_fg(cell))
}

fn underdotted_color<'a>(cell: &'a ui_model::Cell, hl: &'a HighlightMap) -> &'a color::Color {
    hl.cell_sp(cell).unwrap_or(&color::COLOR_RED)
}

fn plan_underline_strikethrough<'a>(
    pending_strikethrough: &mut Option<usize>,
    pending_underline: &mut Option<usize>,
    pending_underdouble: &mut Option<usize>,
    pending_fmt_ops: &mut Vec<RenderStep<'a>>,
    hl: &'a HighlightMap,
    cell: &'a ui_model::Cell,
    pos: (usize, usize),
) {
    if cell.hl.strikethrough {
        let fg = hl.actual_cell_fg(cell);
        let mut extended = false;
        if let Some(idx) = *pending_strikethrough {
            extended = pending_fmt_ops[idx].extend(RenderStepKind::Strikethrough, fg);
        }

        if !extended {
            *pending_strikethrough = Some(pending_fmt_ops.len());
            pending_fmt_ops.push(RenderStep::new(RenderStepKind::Strikethrough, fg, pos));
        }
    } else {
        *pending_strikethrough = None;
    }

    let (kind, color) = if cell.hl.underdashed {
        (RenderStepKind::Underdash, underline_color(cell, hl))
    } else if cell.hl.underdotted {
        (RenderStepKind::Underdot, underdotted_color(cell, hl))
    } else if cell.hl.underline {
        (RenderStepKind::Underline, underline_color(cell, hl))
    } else {
        *pending_underline = None;
        *pending_underdouble = None;
        return;
    };

    let mut extended = false;
    if let Some(idx) = *pending_underline {
        extended = pending_fmt_ops[idx].extend(kind, color);
    }

    if !extended {
        *pending_underline = Some(pending_fmt_ops.len());
        pending_fmt_ops.push(RenderStep::new(kind, color, pos));
    }

    if cell.hl.underdouble {
        if let Some(idx) = *pending_underdouble {
            if pending_fmt_ops[idx].extend(RenderStepKind::Underdouble, color) {
                return;
            }
        }
        *pending_underdouble = Some(pending_fmt_ops.len());
        pending_fmt_ops.push(RenderStep::new(RenderStepKind::Underdouble, color, pos));
    }
}

#[inline]
fn snapshot_bg(
    snapshot: &gtk::Snapshot,
    cell_metrics: &CellMetrics,
    color: &color::Color,
    x: f64,
    y: f64,
    len: f64,
) {
    snapshot.append_color(
        &color.into(),
        &Rect::new(
            x as f32,
            y as f32,
            len as f32,
            cell_metrics.line_height as f32,
        ),
    )
}

/// Generate render nodes for the current cell
fn snapshot_cell(
    snapshot: &gtk::Snapshot,
    items: &[ui_model::Item],
    hl: &HighlightMap,
    cell: &ui_model::Cell,
    x: f32,
    y: f32,
) {
    for item in items {
        let fg = hl.actual_cell_fg(cell);
        if item.glyphs().is_some() {
            if let Some(render_node) = item.render_node(fg, (x, y)) {
                snapshot.append_node(render_node);
            }
        }
    }
}

macro_rules! dirty {
    ($line: expr, $skip: expr) => {
        for cell in $line.line.iter_mut().skip($skip) {
            cell.dirty = true;
        }
    };
}

pub fn shape_dirty(
    ctx: &context::Context,
    alt_ctx: &context::Context,
    sub_ctxs: &mut SubCtx,
    grid: &mut Grid,
    hl: &HighlightMap,
    update_pix: bool,
    monospace: bool,
) {
    let cell_metrics = ctx.cell_metrics();
    let char_width = cell_metrics.char_width as f32;
    let space_width = cell_metrics.space_width as f32;
    let sign_column = grid.sign_column_len();
    let width = (grid.rect.0 + grid.rect.2) as f32;
    for (row, line) in grid.model.model_mut().iter_mut().enumerate() {
        if !line.dirty_line {
            continue;
        }
        shape_dirty_line(line, hl, ctx, alt_ctx);
        line.dirty_line = false;
        if !update_pix {
            continue;
        }
        if monospace {
            grid.pix.update_mono(row, space_width);
            continue;
        }
        let x = grid
            .pix
            .update(line, row, char_width, space_width, sign_column);
        if x <= width {
            continue;
        }
        let mut ratio = 1.0;
        loop {
            ratio -= 0.05;
            let size = sub_ctxs.max_smaller_size(ratio);
            dirty!(line, sign_column);
            shape_dirty_line(line, hl, sub_ctxs.get_or_create(size), alt_ctx);
            let x = grid
                .pix
                .update(line, row, char_width, space_width, sign_column);
            if x <= width {
                dirty!(line, sign_column);
                line.dirty_line = true;
                break;
            }
        }
    }
}

fn shape_dirty_line(line: &mut Line, hl: &HighlightMap, ctx: &Context, alt_ctx: &Context) {
    let styled_line = ui_model::StyledLine::from(line, hl, ctx.font_features());
    let items = ctx.itemize(&styled_line, alt_ctx);
    line.merge(&styled_line, &items);
    for (col, cell) in line.line.iter_mut().enumerate() {
        if cell.dirty {
            for item in &mut *line.item_line[col] {
                let mut glyphs = pango::GlyphString::new();
                {
                    let analysis = item.analysis();
                    let offset = item.item.offset() as usize;
                    let length = item.item.length() as usize;
                    if let Some(line_str) = styled_line.line_str.get(offset..offset + length) {
                        pango::shape(line_str, analysis, &mut glyphs);
                    } else {
                        warn!("Wrong itemize split");
                    }
                }
                item.set_glyphs(glyphs);
            }
        }
        cell.dirty = false;
    }
}
