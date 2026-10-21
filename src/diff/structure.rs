//! Structural diffs: which tokens changed, judged by the syntax tree rather
//! than by lines, so reformatting, wrapped arguments and code wrapped in a
//! new block don't read as wholly changed lines.
//!
//! Each side is parsed with tree-sitter and seen as lists and atoms, as
//! difftastic sees it: a node's children form a list, the brackets it opens
//! and closes are the list's delimiters, and leaves, comments and strings
//! are atoms. Two sequences of nodes are matched by their subtrees' hashes
//! first (the same code, wherever its whitespace and line breaks fall).
//! What's left is paired: lists of the same kind and brackets by how much
//! they share, recursing into them; code wrapped in a new list (an `if`, a
//! closure, a call) or unwrapped from one, inside it; comments and strings
//! by their words. Anything unmatched is novel.
//!
//! The matching ([`Matching`]) says which token each token is on the other
//! side. Each alignment of the file's lines then reads it for itself
//! ([`Matching::apply`]): a match is kept only where it agrees with the
//! alignment's context lines, so what's emphasised never hides a change the
//! lines show.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use imara_diff::{Algorithm, Diff, InternedInput};
use tree_sitter::{Node as TsNode, Tree};

use crate::diff::blocks::Fnv;
use crate::diff::file::{Alignment, TextDiff};
use crate::diff::hunks::DiffLine;
use crate::diff::intraline::{
    IntraLine, best_pairing, changed_tokens, dice, greedy_pairing, similarity,
};
use crate::diff::sat_u32;
use crate::diff::text::Text;

/// Why a file is shown by its lines only, with structure asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// No grammar for its language.
    NoGrammar,
    /// Too big to parse, or too many tokens to compare.
    TooBig,
    /// Comparing took more work than a file is allowed.
    TooSlow,
    /// A side didn't parse in time.
    Unparsed,
}

impl Fallback {
    /// Why, in a few words, for the file's header.
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoGrammar => "no grammar for this language",
            Self::TooBig => "too big to compare by syntax",
            Self::TooSlow => "too slow to compare by syntax",
            Self::Unparsed => "didn't parse in time",
        }
    }
}

/// Tokens on a side above this are compared by lines only.
const MAX_TOKENS: usize = 200_000;
/// Units of work (nodes hashed against, tokens compared) one file may
/// take; past it, the file is compared by lines. Counted rather than
/// timed, so the same diff always falls back the same way.
const MAX_WORK: u64 = 30_000_000;
/// Nesting deeper than this is compared as a flat run of tokens.
const MAX_DEPTH: usize = 160;
/// Lists with the same brackets (or of one kind) this alike, by their
/// tokens, are paired.
const MIN_LIST_SIMILARITY: f64 = 0.3;
/// Code this alike to a list's contents counts as wrapped in it.
const MIN_WRAP_SIMILARITY: f64 = 0.5;
/// Comments and strings this alike (character bigrams) are paired.
const MIN_TEXT_SIMILARITY: f64 = 0.5;
/// Pairings over more candidate pairs than this are made greedily.
const MAX_PAIRING: usize = 40_000;
/// Change blocks with more lines a side than this keep positional rows.
const MAX_PAIRED_LINES: usize = 400;

/// A token: its byte range in the source.
#[derive(Debug, Clone, Copy)]
struct Token {
    start: u32,
    end: u32,
}

#[derive(Debug)]
enum Node {
    /// A token; `text` for a comment or string, compared by its words.
    Atom { token: u32, kind: u16, text: bool },
    /// A node's children, with the brackets around them if it has some.
    List {
        kind: u16,
        open: Option<u32>,
        close: Option<u32>,
        children: Vec<u32>,
    },
}

/// One side's tree as lists and atoms.
#[derive(Debug, Default)]
struct Side {
    tokens: Vec<Token>,
    token_hash: Vec<u64>,
    nodes: Vec<Node>,
    /// Each node's hash: its tokens and how they nest, not their spacing.
    hash: Vec<u64>,
    roots: Vec<u32>,
}

/// Brackets that delimit a list.
const DELIMITERS: [(&str, &str); 3] = [("(", ")"), ("[", "]"), ("{", "}")];

/// A node read whole, as one atom compared by its words: comments,
/// strings and prose, and any token over several lines (a fenced code
/// block, a block scalar).
fn is_text(n: TsNode<'_>, source: &str) -> bool {
    let kind = n.kind();
    let multiline = || {
        n.child_count() == 0
            && source
                .get(n.start_byte()..n.end_byte())
                .is_some_and(|s| s.contains('\n'))
    };
    kind.contains("comment")
        || kind.contains("string")
        || kind.ends_with("char_literal")
        || kind.ends_with("rune_literal")
        || kind.ends_with("scalar")
        || kind == "heredoc_body"
        || kind == "inline"
        || multiline()
}

impl Side {
    fn build(tree: &Tree, source: &str) -> Result<Self, Fallback> {
        let mut side = Self::default();
        let root = tree.root_node();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if let Some(id) = side.node(child, source, 0) {
                side.roots.push(id);
            }
            if side.tokens.len() > MAX_TOKENS {
                return Err(Fallback::TooBig);
            }
        }
        Ok(side)
    }

    fn text<'s>(&self, source: &'s str, token: u32) -> &'s str {
        self.tokens
            .get(token as usize)
            .and_then(|t| source.get(t.start as usize..t.end as usize))
            .unwrap_or_default()
    }

    fn token(&mut self, n: TsNode<'_>, source: &str, text: bool) -> u32 {
        let (start, end) = (sat_u32(n.start_byte()), sat_u32(n.end_byte()));
        let slice = source.get(start as usize..end as usize).unwrap_or_default();
        let mut h = Fnv::new();
        if text {
            // Words, so re-indenting a comment's lines isn't a change.
            for word in slice.split_whitespace() {
                h.write(word.as_bytes());
                h.write(b" ");
            }
        } else {
            h.write(slice.as_bytes());
        }
        self.tokens.push(Token { start, end });
        self.token_hash.push(h.0);
        sat_u32(self.tokens.len() - 1)
    }

    fn push(&mut self, node: Node) -> u32 {
        let mut h = Fnv::new();
        let mut put = |x: u64| h.write(&x.to_le_bytes());
        match &node {
            Node::Atom { token, .. } => {
                put(self.token_hash.get(*token as usize).copied().unwrap_or(0));
            }
            Node::List {
                open,
                close,
                children,
                ..
            } => {
                let token =
                    |t: &Option<u32>| t.and_then(|t| self.token_hash.get(t as usize).copied());
                put(1);
                put(token(open).unwrap_or(2));
                for c in children {
                    put(self.hash.get(*c as usize).copied().unwrap_or(0));
                }
                put(token(close).unwrap_or(3));
            }
        }
        self.nodes.push(node);
        self.hash.push(h.0);
        sat_u32(self.nodes.len() - 1)
    }

    /// `n` as a node, or `None` when it has no text (a missing token).
    fn node(&mut self, n: TsNode<'_>, source: &str, depth: usize) -> Option<u32> {
        if n.start_byte() >= n.end_byte() {
            return None;
        }
        let text = is_text(n, source);
        if n.child_count() == 0 || text {
            let token = self.token(n, source, text);
            return Some(self.push(Node::Atom {
                token,
                kind: n.kind_id(),
                text,
            }));
        }
        let kind = n.kind_id();
        if depth >= MAX_DEPTH {
            let children = self.leaves(n, source);
            return Some(self.list(kind, None, None, children));
        }
        let mut cursor = n.walk();
        let kids: Vec<TsNode<'_>> = n.children(&mut cursor).collect();
        let leaf = |i: usize| {
            kids.get(i)
                .filter(|k| k.child_count() == 0)
                .and_then(|k| source.get(k.start_byte()..k.end_byte()))
        };
        let delimited = (0..kids.len()).find_map(|i| {
            let close = DELIMITERS.iter().find(|(o, _)| leaf(i) == Some(o))?.1;
            let j = (i + 1..kids.len())
                .rev()
                .find(|&j| leaf(j) == Some(close))?;
            Some((i, j))
        });
        let convert = |side: &mut Self, range: std::ops::Range<usize>| -> Vec<u32> {
            let nodes = kids.get(range).unwrap_or_default();
            nodes
                .iter()
                .filter_map(|k| side.node(*k, source, depth + 1))
                .collect()
        };
        let Some((i, j)) = delimited else {
            let children = convert(self, 0..kids.len());
            return Some(self.list(kind, None, None, children));
        };
        let before = convert(self, 0..i);
        let open = kids.get(i).map(|k| self.token(*k, source, false));
        // A trailing comma comes and goes with wrapping, so it isn't
        // compared (except a Python one-element tuple's, which makes it one).
        let trailing = j > i + 2
            && j.checked_sub(1).is_some_and(|c| leaf(c) == Some(","))
            && !(n.kind() == "tuple" && j == i + 3);
        let between = convert(self, i + 1..if trailing { j - 1 } else { j });
        let close = kids.get(j).map(|k| self.token(*k, source, false));
        let after = convert(self, j + 1..kids.len());
        let inner = self.push(Node::List {
            kind,
            open,
            close,
            children: between,
        });
        if before.is_empty() && after.is_empty() {
            return Some(inner);
        }
        let children = before.into_iter().chain([inner]).chain(after).collect();
        Some(self.list(kind, None, None, children))
    }

    /// A list without brackets; one child stands for itself.
    fn list(
        &mut self,
        kind: u16,
        open: Option<u32>,
        close: Option<u32>,
        children: Vec<u32>,
    ) -> u32 {
        match children.as_slice() {
            [only] if open.is_none() && close.is_none() => *only,
            _ => self.push(Node::List {
                kind,
                open,
                close,
                children,
            }),
        }
    }

    /// Every leaf under `n`, as atoms, in order.
    fn leaves(&mut self, n: TsNode<'_>, source: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cursor = n.walk();
        'walk: loop {
            let node = cursor.node();
            let text = is_text(node, source);
            if (node.child_count() == 0 || text) && node.start_byte() < node.end_byte() {
                let token = self.token(node, source, text);
                out.push(self.push(Node::Atom {
                    token,
                    kind: node.kind_id(),
                    text,
                }));
            } else if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.node() == n {
                    break 'walk;
                }
                if cursor.goto_next_sibling() {
                    continue 'walk;
                }
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        out
    }
}

/// Which token on the other side each token is, per side.
#[derive(Debug)]
pub(crate) struct Matching {
    sides: [Side; 2],
    partner: [Vec<Option<u32>>; 2],
    /// Changed byte ranges (from the token's start) inside paired comments
    /// and strings.
    inner: [HashMap<u32, Vec<(u32, u32)>>; 2],
}

/// Compares the trees of `old` and `new`.
pub(crate) fn compare(old: (&Tree, &Text), new: (&Tree, &Text)) -> Result<Matching, Fallback> {
    compare_within(old, new, MAX_WORK)
}

/// [`compare`], giving up past `budget` units of work.
fn compare_within(
    old: (&Tree, &Text),
    new: (&Tree, &Text),
    budget: u64,
) -> Result<Matching, Fallback> {
    let l = Side::build(old.0, &old.1.source)?;
    let r = Side::build(new.0, &new.1.source)?;
    let mut m = Matcher {
        partner: [vec![None; l.tokens.len()], vec![None; r.tokens.len()]],
        inner: Default::default(),
        sources: [&old.1.source, &new.1.source],
        bags: Default::default(),
        work: 0,
        budget,
        sides: [&l, &r],
    };
    m.seq(&l.roots, &r.roots)?;
    let (partner, inner) = (m.partner, m.inner);
    Ok(Matching {
        sides: [l, r],
        partner,
        inner,
    })
}

struct Matcher<'a> {
    sides: [&'a Side; 2],
    sources: [&'a str; 2],
    partner: [Vec<Option<u32>>; 2],
    inner: [HashMap<u32, Vec<(u32, u32)>>; 2],
    /// Each list's tokens' hashes, sorted, as they're asked for.
    bags: [HashMap<u32, Rc<Vec<u64>>>; 2],
    work: u64,
    budget: u64,
}

/// A side: 0 old, 1 new.
type S = usize;

impl<'a> Matcher<'a> {
    fn spend(&mut self, units: usize) -> Result<(), Fallback> {
        self.work = self.work.saturating_add(units as u64);
        if self.work > self.budget {
            return Err(Fallback::TooSlow);
        }
        Ok(())
    }

    fn node(&self, s: S, n: u32) -> Option<&'a Node> {
        self.sides.get(s).copied()?.nodes.get(n as usize)
    }

    fn hash(&self, s: S, n: u32) -> u64 {
        (self.sides.get(s))
            .and_then(|side| side.hash.get(n as usize))
            .copied()
            .unwrap_or(0)
    }

    fn pair(&mut self, l: Option<u32>, r: Option<u32>) {
        let (Some(l), Some(r)) = (l, r) else { return };
        let [lp, rp] = &mut self.partner;
        if let (Some(a), Some(b)) = (lp.get_mut(l as usize), rp.get_mut(r as usize)) {
            *a = Some(r);
            *b = Some(l);
        }
    }

    /// Matches two sequences of sibling nodes.
    fn seq(&mut self, l: &[u32], r: &[u32]) -> Result<(), Fallback> {
        if l.is_empty() || r.is_empty() {
            return Ok(());
        }
        self.spend(l.len() + r.len())?;
        let mut input = InternedInput::default();
        input.update_before(l.iter().map(|&n| self.hash(0, n)));
        input.update_after(r.iter().map(|&n| self.hash(1, n)));
        let diff = Diff::compute(Algorithm::Histogram, &input);
        let (mut i, mut j) = (0, 0);
        let end = (
            sat_u32(l.len())..sat_u32(l.len()),
            sat_u32(r.len())..sat_u32(r.len()),
        );
        for (before, after) in diff.hunks().map(|h| (h.before, h.after)).chain([end]) {
            let same = (i..before.start as usize).zip(j..after.start as usize);
            for (a, b) in same {
                if let (Some(&a), Some(&b)) = (l.get(a), r.get(b)) {
                    self.same(a, b);
                }
            }
            let (ls, rs) = (
                l.get(before.start as usize..before.end as usize)
                    .unwrap_or_default(),
                r.get(after.start as usize..after.end as usize)
                    .unwrap_or_default(),
            );
            self.hunk(ls, rs)?;
            (i, j) = (before.end as usize, after.end as usize);
        }
        Ok(())
    }

    /// Pairs the tokens of two equal subtrees.
    fn same(&mut self, a: u32, b: u32) {
        let (x, y) = match (self.node(0, a), self.node(1, b)) {
            (Some(Node::Atom { token: x, .. }), Some(Node::Atom { token: y, .. })) => {
                (Some(*x), Some(*y))
            }
            (
                Some(Node::List {
                    open: lo,
                    close: lc,
                    children: lk,
                    ..
                }),
                Some(Node::List {
                    open: ro,
                    close: rc,
                    children: rk,
                    ..
                }),
            ) if lk.len() == rk.len() => {
                self.pair(*lo, *ro);
                self.pair(*lc, *rc);
                for (&x, &y) in lk.iter().zip(rk) {
                    self.same(x, y);
                }
                return;
            }
            // Equal hashes, different shapes: a collision; leave it novel.
            _ => return,
        };
        self.pair(x, y);
    }

    /// Matches what a sequence diff left unmatched between two anchors.
    fn hunk(&mut self, ls: &[u32], rs: &[u32]) -> Result<(), Fallback> {
        if ls.is_empty() || rs.is_empty() {
            return Ok(());
        }
        let pairs = self.pair_up(ls, rs)?;
        let (mut pi, mut pj) = (0, 0);
        let end = (ls.len(), rs.len());
        for &(i, j) in pairs.iter().chain([&end]) {
            let (gl, gr) = (
                ls.get(pi..i).unwrap_or_default(),
                rs.get(pj..j).unwrap_or_default(),
            );
            self.wrapped(gl, gr)?;
            if let (Some(&a), Some(&b)) = (ls.get(i), rs.get(j)) {
                self.inside(a, b)?;
            }
            (pi, pj) = (i + 1, j + 1);
        }
        Ok(())
    }

    /// Matches inside two paired nodes.
    fn inside(&mut self, a: u32, b: u32) -> Result<(), Fallback> {
        match (self.node(0, a), self.node(1, b)) {
            (
                Some(Node::List {
                    open: lo,
                    close: lc,
                    children: lk,
                    ..
                }),
                Some(Node::List {
                    open: ro,
                    close: rc,
                    children: rk,
                    ..
                }),
            ) => {
                self.pair(*lo, *ro);
                self.pair(*lc, *rc);
                self.seq(lk, rk)
            }
            (Some(Node::Atom { token: x, .. }), Some(Node::Atom { token: y, .. })) => {
                let (x, y) = (*x, *y);
                self.pair(Some(x), Some(y));
                let texts = [self.text(0, x), self.text(1, y)];
                self.spend(texts[0].len() + texts[1].len())?;
                let (old, new) = changed_tokens(texts[0], texts[1]);
                let [li, ri] = &mut self.inner;
                li.insert(x, old);
                ri.insert(y, new);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn text(&self, s: S, token: u32) -> &'a str {
        match (self.sides.get(s), self.sources.get(s)) {
            (Some(side), Some(&source)) => side.text(source, token),
            _ => "",
        }
    }

    /// How much two nodes are alike, if they may pair at all.
    fn score(&mut self, a: u32, b: u32) -> Result<Option<f64>, Fallback> {
        let open = |s: S, t: &Option<u32>| t.map(|t| self.text(s, t));
        let lists = match (self.node(0, a), self.node(1, b)) {
            (
                Some(Node::List {
                    kind: k1,
                    open: o1,
                    close: c1,
                    ..
                }),
                Some(Node::List {
                    kind: k2,
                    open: o2,
                    close: c2,
                    ..
                }),
            ) => {
                // Bracketed lists pair by their brackets, whatever the node
                // (`(a)` and the tuple `(a,)`); others by their kind.
                let (o1, o2) = (open(0, o1), open(1, o2));
                (k1 == k2 || o1.is_some()) && o1 == o2 && open(0, c1) == open(1, c2)
            }
            (
                Some(Node::Atom {
                    token: x,
                    kind: k1,
                    text: true,
                }),
                Some(Node::Atom {
                    token: y,
                    kind: k2,
                    text: true,
                }),
            ) if k1 == k2 => {
                let (x, y) = (*x, *y);
                let (tx, ty) = (self.text(0, x), self.text(1, y));
                let cost = tx.len() + ty.len();
                let s = similarity(tx, ty);
                self.spend(cost)?;
                return Ok(Some(s).filter(|s| *s >= MIN_TEXT_SIMILARITY));
            }
            _ => false,
        };
        if !lists {
            return Ok(None);
        }
        let (x, y) = (self.bag(0, a), self.bag(1, b));
        self.spend(x.len() + y.len())?;
        Ok(Some(dice(&x, &y)).filter(|s| *s >= MIN_LIST_SIMILARITY))
    }

    /// The sorted hashes of every token under `n`.
    fn bag(&mut self, s: S, n: u32) -> Rc<Vec<u64>> {
        if let Some(bag) = self.bags.get(s).and_then(|b| b.get(&n)) {
            return Rc::clone(bag);
        }
        let mut out = Vec::new();
        let mut stack = vec![n];
        while let Some(n) = stack.pop() {
            let side = self.sides.get(s);
            let hash = |t: &u32| {
                side.and_then(|side| side.token_hash.get(*t as usize))
                    .copied()
            };
            match self.node(s, n) {
                Some(Node::Atom { token, .. }) => out.extend(hash(token)),
                Some(Node::List {
                    open,
                    close,
                    children,
                    ..
                }) => {
                    out.extend(open.iter().chain(close).filter_map(hash));
                    stack.extend(children.iter().rev());
                }
                None => {}
            }
        }
        out.sort_unstable();
        let bag = Rc::new(out);
        if let Some(bags) = self.bags.get_mut(s) {
            bags.insert(n, Rc::clone(&bag));
        }
        bag
    }

    /// The bags of several nodes, merged.
    fn bag_of(&mut self, s: S, ns: &[u32]) -> Vec<u64> {
        let mut out = Vec::new();
        for &n in ns {
            out.extend_from_slice(&self.bag(s, n));
        }
        out.sort_unstable();
        out
    }

    /// Which nodes of `ls` and `rs` pair, in order: the pairing with the
    /// greatest total likeness, weighted by size, or greedily when there
    /// are too many candidates.
    fn pair_up(&mut self, ls: &[u32], rs: &[u32]) -> Result<Vec<(usize, usize)>, Fallback> {
        let weight = |me: &mut Self, i: usize, j: usize| -> Result<Option<f64>, Fallback> {
            let (Some(&a), Some(&b)) = (ls.get(i), rs.get(j)) else {
                return Ok(None);
            };
            let Some(s) = me.score(a, b)? else {
                return Ok(None);
            };
            let size = me.bag(0, a).len() + me.bag(1, b).len();
            Ok(Some(s * size as f64))
        };
        let (n, m) = (ls.len(), rs.len());
        if n.saturating_mul(m) > MAX_PAIRING {
            greedy_pairing(n, m, |i, j| weight(self, i, j))
        } else {
            best_pairing(n, m, |i, j| weight(self, i, j))
        }
    }

    /// Unpaired nodes between two anchors: code wrapped in a new list on
    /// one side (or unwrapped from one) is matched against its contents.
    fn wrapped(&mut self, ls: &[u32], rs: &[u32]) -> Result<(), Fallback> {
        if ls.is_empty() || rs.is_empty() {
            return Ok(());
        }
        let (lbag, rbag) = (self.bag_of(0, ls), self.bag_of(1, rs));
        self.spend(lbag.len() + rbag.len())?;
        // The list most like everything on the other side; of lists as
        // alike, the biggest, which explains the most.
        let mut best: Option<(f64, usize, S, u32)> = None;
        for (s, nodes, other) in [(1, rs, &lbag), (0, ls, &rbag)] {
            for &n in nodes {
                if !matches!(self.node(s, n), Some(Node::List { .. })) {
                    continue;
                }
                let bag = self.bag(s, n);
                self.spend(bag.len() + other.len())?;
                let like = dice(&bag, other);
                let size = bag.len();
                let better = |(b, bs, ..): (f64, usize, S, u32)| {
                    like.total_cmp(&b).then(size.cmp(&bs)).is_gt()
                };
                if like >= MIN_WRAP_SIMILARITY && best.is_none_or(better) {
                    best = Some((like, size, s, n));
                }
            }
        }
        let Some((.., s, n)) = best else {
            // A run of tokens replaced one for one: each pairs with its
            // replacement, so only the characters that differ stand out.
            if ls.len() == rs.len() {
                for (&a, &b) in ls.iter().zip(rs) {
                    let same_kind = match (self.node(0, a), self.node(1, b)) {
                        (Some(Node::Atom { kind: x, .. }), Some(Node::Atom { kind: y, .. })) => {
                            x == y
                        }
                        _ => false,
                    };
                    if same_kind {
                        self.inside(a, b)?;
                    }
                }
            }
            return Ok(());
        };
        let Some(Node::List { children, .. }) = self.node(s, n) else {
            return Ok(());
        };
        if s == 1 {
            self.seq(ls, children)
        } else {
            self.seq(children, rs)
        }
    }
}

/// The 1-based line holding byte `at` of `text`.
fn line_at(text: &Text, at: u32) -> u32 {
    sat_u32(text.lines.partition_point(|&(start, _)| start <= at)).max(1)
}

/// `(line, start, end)` pieces of byte range `range` of `text`, one per
/// line it covers, in that line's own offsets.
fn per_line(text: &Text, range: (u32, u32)) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
    let first = line_at(text, range.0);
    (first..)
        .map_while(move |line| {
            let &(start, end) = text.lines.get(line as usize - 1)?;
            (start < range.1 || line == first).then_some((line, start, end))
        })
        .filter_map(move |(line, start, end)| {
            let (a, b) = (range.0.max(start), range.1.min(end));
            (a < b).then(|| (line, a - start, b - start))
        })
}

impl Matching {
    /// The structural reading of `line`, an alignment of `text`'s lines:
    /// the same lines and blocks, with emphasis on just the novel tokens,
    /// blocks with none folded as formatting, and the removed and added
    /// lines of a block paired by the tokens they share.
    pub(crate) fn apply(&self, text: &TextDiff, line: &Alignment) -> Alignment {
        let [old, new] = [
            text.side_text(crate::diff::anchor::Side::Left),
            text.side_text(crate::diff::anchor::Side::Right),
        ];
        // Each side's context lines, by the line they show on the other.
        let mut counterpart = [vec![0u32; old.len() + 1], vec![0u32; new.len() + 1]];
        for l in &line.lines {
            if let DiffLine::Context { old: o, new: n } = *l {
                let [co, cn] = &mut counterpart;
                if let Some(c) = co.get_mut(o as usize) {
                    *c = n;
                }
                if let Some(c) = cn.get_mut(n as usize) {
                    *c = o;
                }
            }
        }
        let texts = [old, new];
        // Novel ranges, lines with a token that isn't, and matched lines,
        // by side and line.
        let mut novel: [HashMap<u32, Vec<(u32, u32)>>; 2] = Default::default();
        let mut keeps: [HashSet<u32>; 2] = Default::default();
        let mut shared: HashMap<(u32, u32), usize> = HashMap::new();
        for s in 0..2 {
            let (Some(side), Some(&text), Some(partners)) =
                (self.sides.get(s), texts.get(s), self.partner.get(s))
            else {
                continue;
            };
            let other = 1 - s;
            for (t, token) in side.tokens.iter().enumerate() {
                let here = line_at(text, token.start);
                let there = partners.get(t).copied().flatten().and_then(|p| {
                    let tok = self.sides.get(other)?.tokens.get(p as usize)?;
                    Some(line_at(texts.get(other)?, tok.start))
                });
                // A match must agree with the context lines: a context
                // line's tokens are its counterpart's.
                let agrees = there.is_some_and(|there| {
                    let ctx = |s: usize, l: u32| {
                        counterpart
                            .get(s)
                            .and_then(|c| c.get(l as usize))
                            .copied()
                            .unwrap_or(0)
                    };
                    let (mine, theirs) = (ctx(s, here), ctx(other, there));
                    (mine == 0 || mine == there) && (theirs == 0 || theirs == here)
                });
                let ranges: Vec<(u32, u32)> = if agrees {
                    if let Some(keeps) = keeps.get_mut(s) {
                        keeps.extend(per_line(text, (token.start, token.end)).map(|p| p.0));
                    }
                    if s == 0
                        && let Some(there) = there
                    {
                        *shared.entry((here, there)).or_default() += 1;
                    }
                    let inner = self.inner.get(s).and_then(|i| i.get(&sat_u32(t)));
                    inner
                        .into_iter()
                        .flatten()
                        .map(|&(a, b)| (token.start + a, token.start + b))
                        .collect()
                } else {
                    vec![(token.start, token.end)]
                };
                if let Some(map) = novel.get_mut(s) {
                    for (l, a, b) in ranges.into_iter().flat_map(|r| per_line(text, r)) {
                        map.entry(l).or_default().push((a, b));
                    }
                }
            }
        }
        let mut out = Alignment {
            lines: line.lines.clone(),
            intraline: IntraLine::new(),
            blocks: line.blocks.clone(),
            pairs: Vec::new(),
            kept: HashSet::new(),
        };
        for block in &mut out.blocks {
            let entries = block.entries.clone();
            // Each changed line's novel ranges and whether that's all of it.
            let mut lines = Vec::new();
            for e in entries.clone() {
                let Some(l) = line.lines.get(e as usize) else {
                    continue;
                };
                let pos = l.shown();
                let s = usize::from(pos.side == crate::diff::anchor::Side::Right);
                let code = text.line(pos);
                let ranges = novel
                    .get(s)
                    .and_then(|m| m.get(&pos.line))
                    .map(|r| merged(code, r))
                    .unwrap_or_default();
                // Wholly new or gone: every token on it is.
                let whole = !keeps.get(s).is_some_and(|k| k.contains(&pos.line));
                lines.push((e, ranges, whole));
            }
            let any_novel = lines.iter().any(|(_, r, _)| !r.is_empty());
            let touches_end = text.final_newline_changed()
                && entries
                    .clone()
                    .filter_map(|e| line.lines.get(e as usize))
                    .flat_map(|l| l.lines())
                    .any(|p| text.is_last(p));
            // What lines call formatting stays so: a tokenizer can split
            // the same code differently (`x.0 .1` and `x.0.1`).
            block.formatting_only |= !any_novel && !touches_end;
            // A wholly new or gone line is plain, as by lines; a line with
            // nothing new or gone is kept.
            for (e, ranges, whole) in lines {
                if ranges.is_empty() || block.formatting_only {
                    out.kept.insert(e);
                } else if !whole {
                    out.intraline.insert(e, ranges);
                }
            }
            out.pairs.extend(pair_lines(&line.lines, entries, &shared));
        }
        out
    }
}

/// `ranges` of `code`, sorted and joined across whitespace, trimmed of it.
fn merged(code: &str, ranges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (a, b) in sorted {
        let Some(piece) = code.get(a as usize..b as usize) else {
            continue;
        };
        let lead = sat_u32(piece.len() - piece.trim_start().len());
        let trail = sat_u32(piece.len() - piece.trim_end().len());
        let (a, b) = (a + lead, b.saturating_sub(trail));
        if a >= b {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if code
                    .get(last.1.min(a) as usize..a as usize)
                    .is_some_and(|gap| gap.trim().is_empty()) =>
            {
                last.1 = last.1.max(b);
            }
            _ => out.push((a, b)),
        }
    }
    out
}

/// Which removed and added entries of the block at `entries` sit side by
/// side: the in-order pairing sharing the most matched tokens.
fn pair_lines(
    lines: &[DiffLine],
    entries: std::ops::Range<u32>,
    shared: &HashMap<(u32, u32), usize>,
) -> Vec<(u32, u32)> {
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    for e in entries {
        match lines.get(e as usize) {
            Some(DiffLine::Removed(l)) => removed.push((e, *l)),
            Some(DiffLine::Added(l)) => added.push((e, *l)),
            _ => {}
        }
    }
    if removed.is_empty()
        || added.is_empty()
        || removed.len() > MAX_PAIRED_LINES
        || added.len() > MAX_PAIRED_LINES
    {
        return Vec::new();
    }
    let weight = |i: usize, j: usize| {
        let shared = match (removed.get(i), added.get(j)) {
            (Some(&(_, o)), Some(&(_, n))) => shared.get(&(o, n)).copied().unwrap_or(0),
            _ => 0,
        };
        Ok::<_, std::convert::Infallible>((shared > 0).then_some(shared as f64))
    };
    let Ok(pairs) = best_pairing(removed.len(), added.len(), weight);
    (pairs.into_iter())
        .filter_map(|(i, j)| Some((removed.get(i)?.0, added.get(j)?.0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::file::tests::text_diff;
    use crate::diff::highlight::{Language, parse};
    use crate::diff::hunks::Whitespace;

    #[test]
    fn a_file_without_a_grammar_is_compared_by_lines() {
        let text = text_diff("notes.txt", "a b\n", "a c\n");
        assert_eq!(text.fallback(), Some(Fallback::NoGrammar));
        assert_eq!(
            text.view(Whitespace::Exact, true),
            text.alignment(Whitespace::Exact)
        );
        assert_eq!(text_diff("a.rs", "f(a);\n", "f(b);\n").fallback(), None);
    }

    /// Past its budget a file is compared by lines, the same way every
    /// time.
    #[test]
    fn past_its_budget_a_file_is_compared_by_lines() {
        let (old, new) = (
            Text::new(b"fn f() { a(1, 2); }\n"),
            Text::new(b"fn f() { b(1, 3); }\n"),
        );
        let (a, b) = (
            parse(Language::Rust, &old).unwrap(),
            parse(Language::Rust, &new).unwrap(),
        );
        compare_within((&a, &old), (&b, &new), 1_000_000).unwrap();
        assert_eq!(
            compare_within((&a, &old), (&b, &new), 5).err(),
            Some(Fallback::TooSlow)
        );
    }
}
