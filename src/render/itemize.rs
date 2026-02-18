use unicode_segmentation::*;

#[derive(Debug, PartialEq, Eq)]
pub struct ItemizeResult {
    pub offset: usize,
    pub len: usize,
    pub avoid_break: bool,
}

impl ItemizeResult {
    pub fn new(offset: usize, len: usize, avoid_break: bool) -> Self {
        Self {
            offset,
            len,
            avoid_break,
        }
    }
}

pub struct ItemizeIterator<'a> {
    grapheme_iter: GraphemeIndices<'a>,
    line: &'a str,
    prev_grapheme: Option<(usize, &'a str)>,

    alt_font: &'a [bool],
    last_alt_font: usize,
}

impl<'a> ItemizeIterator<'a> {
    pub fn new(line: &'a str, alt_font: &'a [bool], last_alt_font: usize) -> Self {
        ItemizeIterator {
            grapheme_iter: line.grapheme_indices(true),
            line,
            prev_grapheme: None,
            alt_font,
            last_alt_font,
        }
    }

    #[inline]
    fn is_alt_font(&self, i: usize) -> bool {
        i <= self.last_alt_font && self.alt_font[i] != self.alt_font[i - 1]
    }
}

/*
 * Iterates through a line of text while itemizing it into the largest possible clusters of non-whitespace characters that can be drawn at once without risking column misalignment from ambiguous width characters.
 * This means for ASCII where the size of non-whitespace is essentially guaranteed to be consistent, items will ideally be per-word to speed up rendering.
 * For Unicode, items will be per-grapheme to ensure correct monospaced display.
 */
impl Iterator for ItemizeIterator<'_> {
    type Item = ItemizeResult;

    fn next(&mut self) -> Option<Self::Item> {
        let mut start_index = None;
        let avoid_break = false;

        let end_index = loop {
            let grapheme_index = self
                .prev_grapheme
                .take()
                .or_else(|| self.grapheme_iter.next());
            if let Some((index, grapheme)) = grapheme_index {
                // Figure out if this grapheme is whitespace and/or ASCII in one iteration
                let mut is_whitespace = true;
                for c in grapheme.chars() {
                    if is_whitespace {
                        if c.is_whitespace() {
                            continue;
                        }
                        is_whitespace = false;
                    }
                }

                if start_index.is_none() {
                    if !is_whitespace {
                        start_index = Some(index);
                    }
                } else if is_whitespace || self.is_alt_font(index) {
                    self.prev_grapheme = grapheme_index;
                    break index;
                }
            } else {
                break self.line.len();
            }
        };

        start_index.map(|start_index| {
            ItemizeResult::new(start_index, end_index - start_index, avoid_break)
        })
    }
}
