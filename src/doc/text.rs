use super::RectF;

#[derive(Clone, Copy, Debug)]
pub struct TextChar {
    pub c: char,
    pub rect: RectF,
}

/// Plain text of a page in reading order, one rectangle (in page points) per character.
/// Line breaks are `'\n'` entries whose rectangle repeats the previous character's.
#[derive(Clone, Debug, Default)]
pub struct PageText {
    pub chars: Vec<TextChar>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStyle {
    Glyph,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_words: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

fn fold(c: char, case_sensitive: bool) -> char {
    let c = if c == '\n' { ' ' } else { c };
    if case_sensitive { c } else { c.to_lowercase().next().unwrap_or(c) }
}

impl PageText {
    pub fn string(&self) -> String {
        self.chars.iter().map(|c| c.c).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.iter().all(|c| c.c.is_whitespace())
    }

    fn lines(&self) -> Vec<(usize, usize)> {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, c) in self.chars.iter().enumerate() {
            if c.c == '\n' {
                lines.push((start, i));
                start = i + 1;
            }
        }
        if start < self.chars.len() {
            lines.push((start, self.chars.len()));
        }
        lines
    }

    /// The character offset closest to a point, in reading order.
    pub fn offset_at(&self, x: f32, y: f32) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        let mut best_dist = f32::MAX;
        for (s, e) in self.lines() {
            if s >= e {
                continue;
            }
            let bbox = self.chars[s..e].iter().fold(self.chars[s].rect, |a, c| a.union(&c.rect));
            let dy = if y < bbox.y0 { bbox.y0 - y } else if y > bbox.y1 { y - bbox.y1 } else { 0.0 };
            let dx = if x < bbox.x0 { bbox.x0 - x } else if x > bbox.x1 { x - bbox.x1 } else { 0.0 };
            let d = dy * 4.0 + dx;
            if d < best_dist {
                best_dist = d;
                best = Some((s, e));
            }
        }
        let (s, e) = best?;
        for i in s..e {
            let r = &self.chars[i].rect;
            if x < (r.x0 + r.x1) / 2.0 {
                return Some(i);
            }
        }
        Some(e)
    }

    /// The character under a point, if any.
    pub fn char_at(&self, x: f32, y: f32) -> Option<usize> {
        self.chars.iter().position(|c| c.c != '\n' && c.rect.contains(x, y))
    }

    fn expand(&self, style: SelectionStyle, mut a: usize, mut b: usize) -> (usize, usize) {
        let n = self.chars.len();
        match style {
            SelectionStyle::Glyph => {}
            SelectionStyle::Word => {
                while a > 0 && a <= n && is_word_char(self.chars[a - 1].c) {
                    a -= 1;
                }
                while b < n && is_word_char(self.chars[b].c) {
                    b += 1;
                }
            }
            SelectionStyle::Line => {
                while a > 0 && self.chars[a - 1].c != '\n' {
                    a -= 1;
                }
                while b < n && self.chars[b].c != '\n' {
                    b += 1;
                }
            }
        }
        (a, b)
    }

    /// The selected character range between two points.
    pub fn range_between(&self, style: SelectionStyle, from: (f32, f32), to: (f32, f32)) -> Option<(usize, usize)> {
        let a = self.offset_at(from.0, from.1)?;
        let b = self.offset_at(to.0, to.1)?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (a, b) = self.expand(style, a, b);
        (a < b).then_some((a, b))
    }

    pub fn range_text(&self, a: usize, b: usize) -> String {
        let s: String = self.chars[a.min(self.chars.len())..b.min(self.chars.len())].iter().map(|c| c.c).collect();
        s
    }

    /// One rectangle per line covered by the range.
    pub fn range_rects(&self, a: usize, b: usize) -> Vec<RectF> {
        let mut out: Vec<RectF> = Vec::new();
        let mut cur: Option<RectF> = None;
        for i in a..b.min(self.chars.len()) {
            let ch = &self.chars[i];
            if ch.c == '\n' {
                if let Some(r) = cur.take() {
                    out.push(r);
                }
                continue;
            }
            let r = ch.rect;
            cur = Some(match cur {
                Some(cr) if (r.y0 - cr.y0).abs() < (cr.height() * 0.5).max(1.0) && r.x0 >= cr.x0 - 1.0 => cr.union(&r),
                Some(cr) => {
                    out.push(cr);
                    r
                }
                None => r,
            });
        }
        if let Some(r) = cur {
            out.push(r);
        }
        out
    }

    /// Finds all occurrences of `needle`, returning character ranges.
    pub fn find(&self, needle: &str, opts: FindOptions) -> Vec<(usize, usize)> {
        let needle: Vec<char> = needle.chars().map(|c| fold(c, opts.case_sensitive)).collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let hay: Vec<char> = self.chars.iter().map(|c| fold(c.c, opts.case_sensitive)).collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == needle[..] {
                let end = i + needle.len();
                let ok = !opts.whole_words
                    || ((i == 0 || !is_word_char(hay[i - 1])) && (end == hay.len() || !is_word_char(hay[end])));
                if ok {
                    out.push((i, end));
                    i = end;
                    continue;
                }
            }
            i += 1;
        }
        out
    }
}

/// Builds reading-order text from glyph boxes that arrive in content-stream order:
/// inserts spaces at gaps and line breaks when the baseline moves.
#[derive(Default)]
pub struct TextBuilder {
    pub chars: Vec<TextChar>,
    last: Option<(RectF, f32)>,
}

impl TextBuilder {
    /// Adds a glyph. `baseline` is the y of the glyph origin, `size` the font size, both in points.
    pub fn push(&mut self, s: &str, rect: RectF, baseline: f32, size: f32) {
        if let Some((prev, prev_base)) = self.last {
            let size = size.max(1.0);
            let same_line = (baseline - prev_base).abs() < size * 0.5;
            if !same_line {
                if self.chars.last().is_some_and(|c| c.c != '\n') {
                    self.chars.push(TextChar { c: '\n', rect: prev });
                }
            } else {
                let gap = rect.x0 - prev.x1;
                if gap > size * 0.15 && !s.starts_with(' ') && self.chars.last().is_some_and(|c| c.c != ' ') {
                    self.chars.push(TextChar { c: ' ', rect: RectF::new(prev.x1, prev.y0, rect.x0, prev.y1) });
                } else if gap < -size * 2.0 {
                    self.chars.push(TextChar { c: '\n', rect: prev });
                }
            }
        }
        let n = s.chars().count().max(1) as f32;
        let w = rect.width() / n;
        for (i, c) in s.chars().enumerate() {
            if c.is_control() {
                continue;
            }
            let r = RectF::new(rect.x0 + w * i as f32, rect.y0, rect.x0 + w * (i + 1) as f32, rect.y1);
            self.chars.push(TextChar { c, rect: r });
        }
        self.last = Some((rect, baseline));
    }

    pub fn finish(self) -> PageText {
        PageText { chars: self.chars }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PageText {
        let mut b = TextBuilder::default();
        b.push("Hello", RectF::new(0.0, 0.0, 50.0, 10.0), 8.0, 10.0);
        b.push("world", RectF::new(55.0, 0.0, 105.0, 10.0), 8.0, 10.0);
        b.push("Next", RectF::new(0.0, 20.0, 40.0, 30.0), 28.0, 10.0);
        b.finish()
    }

    #[test]
    fn builds_spaces_and_lines() {
        assert_eq!(sample().string(), "Hello world\nNext");
    }

    #[test]
    fn finds_across_case() {
        let t = sample();
        assert_eq!(t.find("WORLD", FindOptions::default()).len(), 1);
        assert_eq!(t.find("WORLD", FindOptions { case_sensitive: true, ..Default::default() }).len(), 0);
        assert_eq!(t.find("o", FindOptions::default()).len(), 2);
    }

    #[test]
    fn word_selection() {
        let t = sample();
        let (a, b) = t.range_between(SelectionStyle::Word, (12.0, 5.0), (12.0, 5.0)).unwrap();
        assert_eq!(t.range_text(a, b), "Hello");
    }
}
