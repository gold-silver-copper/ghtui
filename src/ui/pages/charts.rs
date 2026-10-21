//! Charts as a terminal draws them: bars and sparklines made of block
//! characters, always beside the numbers they stand for. A bar's height
//! is rounded to an eighth of a cell, and any count above zero shows,
//! however small next to the largest.

use super::*;

/// Eighths of a cell, from one up.
const EIGHTHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Eighths of a cell across, from one up.
const ACROSS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

/// `value` of `max` in eighths of `cells`, at least one when it's above
/// zero.
fn eighths(value: u64, max: u64, cells: usize) -> usize {
    if value == 0 || max == 0 {
        return 0;
    }
    let full = cells.saturating_mul(8);
    let n = (u128::from(value) * full as u128).div_ceil(u128::from(max));
    usize::try_from(n).unwrap_or(full).clamp(1, full)
}

/// A horizontal bar `cells` wide at most: `value` of `max`.
pub fn bar(value: u64, max: u64, cells: usize) -> String {
    let n = eighths(value, max, cells);
    let mut out = "█".repeat(n / 8);
    if let Some(c) = (n % 8).checked_sub(1).and_then(|i| ACROSS.get(i)) {
        out.push(*c);
    }
    out
}

/// One character per value, its height a share of the largest; a space
/// for none.
pub fn sparkline(values: &[u64]) -> String {
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&v| match eighths(v, max, 1) {
            0 => ' ',
            n => EIGHTHS.get(n - 1).copied().unwrap_or('█'),
        })
        .collect()
}

/// `values` summed into `n` buckets or fewer, in order, each the same
/// number of values (the last may have fewer): what each column of a
/// narrow chart counts. The size of a bucket, and the sums.
pub fn buckets(values: &[u32], n: usize) -> (usize, Vec<u64>) {
    let size = values.len().div_ceil(n.max(1)).max(1);
    let sums = values
        .chunks(size)
        .map(|c| c.iter().map(|&v| u64::from(v)).sum())
        .collect();
    (size, sums)
}

/// Columns of bars `height` rows tall, one per value, each `width`
/// cells wide, with the largest value written at the top of the axis
/// and zero at its foot.
pub fn columns(page: &mut Page, values: &[u64], width: usize, height: usize) {
    let max = values.iter().copied().max().unwrap_or(0);
    let label = |n: u64| format!("{:>6} ", compact(n));
    for row in (0..height).rev() {
        let mut line = String::new();
        for &v in values {
            let filled = eighths(v, max, height);
            let below = row * 8;
            let cell = match filled.saturating_sub(below) {
                0 => ' ',
                n if n >= 8 => '█',
                n => EIGHTHS.get(n - 1).copied().unwrap_or('█'),
            };
            line.extend(std::iter::repeat_n(cell, width.max(1)));
        }
        let axis = if row + 1 == height {
            label(max)
        } else {
            " ".repeat(7)
        };
        body(
            page,
            vec![meta(format!("{axis}│")), Seg::new(line, Role::Accent)],
        );
    }
    body(
        page,
        vec![meta(format!(
            "{}└{}",
            label(0),
            "─".repeat(values.len() * width.max(1))
        ))],
    );
}

/// The dates under [`columns`]: the first column's and the last's.
pub fn column_dates(page: &mut Page, columns: usize, first: &str, last: &str) {
    let room = columns.saturating_sub(crate::ui::text::width(first));
    let gap = room.saturating_sub(crate::ui::text::width(last)).max(1);
    body(
        page,
        vec![meta(format!(
            "{}{first}{}{last}",
            " ".repeat(8),
            " ".repeat(gap)
        ))],
    );
}

/// "Oct 5, 2026" for unix seconds.
pub fn date(secs: u64) -> String {
    day(&crate::ui::time::iso8601(secs))
}

/// "Oct 2026" for unix seconds.
pub fn month(secs: u64) -> String {
    let d = date(secs);
    match d.split_once(' ') {
        Some((m, rest)) => format!("{m} {}", rest.rsplit(' ').next().unwrap_or_default()),
        None => d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_count_shows_and_the_largest_fills() {
        assert_eq!(bar(0, 10, 4), "");
        assert_eq!(bar(1, 1000, 4), "▏");
        assert_eq!(bar(10, 10, 4), "████");
        assert_eq!(bar(5, 10, 4), "██");
        assert_eq!(sparkline(&[0, 1, 8, 4]), " ▁█▄");
    }

    #[test]
    fn buckets_sum_in_order() {
        assert_eq!(buckets(&[1, 2, 3, 4, 5], 2), (3, vec![6, 9]));
        assert_eq!(buckets(&[1, 2], 10), (1, vec![1, 2]));
    }
}
