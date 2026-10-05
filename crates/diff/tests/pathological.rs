//! Inputs that once made the diff pipeline hang.

use ghtui_diff::FileDiff;

/// Found by the property tests: tree-sitter's Rust parser never finished
/// on this (carriage returns, escape sequences, bidi controls, wide and
/// combining characters). Highlighting gives up and the diff completes.
#[test]
fn garbage_rust_still_diffs() {
    let old: &str = "\u{202e} \"}\r;  =\u{202e}\u{202e}\n\ne\u{301}sv\\\u{202e}\u{202e}e\u{301}jo\r漢字;漢字\u{1b}[31m\nl\t}\u{202e}=)m)}( ;r\n\rq\"\u{1b}[31mt=}{\u{b}\t\n \"\u{1b}[31m\r\u{1b}[31m\u{1b}[31mk f;\r\"\r\t\"漢字}p\"}}c\n\r漢字\tq\t\n\"e\u{301}\n\u{1b}[31m/\t漢字;{=\"e\u{301}漢字{\u{202e}\nd\u{202e}\u{202e}e\u{301}\u{202e}};\u{1b}[31mv{=y(\n=q(=漢字=};;e\u{301}\u{1b}[31me\u{301}(\u{1b}[31m\u{202e}\n\r;\u{202e}漢字;y漢字\r)漢字d;n} {%\n\r \u{1b}[31m))\u{202e}\"\u{1b}[31m\u{202e}\"\u{202e}\u{1b}[31m\t\"";
    let new: &str = ";\u{1b}[31m漢字;)\u{1b}[31m\u{1b}[31m\n(\u{1b}[31m((mcq\"u) 漢字c\u{1b}[31mu;\"\"(=);\u{202e}\r\nge\u{301}s\u{109dd9}A\u{1b}[31m\n;\u{202e}\n\"漢字}\ti;'\u{202e};\"\n\r\t\t\u{202e}\n\t =\u{1b}[31ml;\t;\n;\t\"漢字k)\u{1b}[31m\u{1b}[31m\tu\u{202e})\u{9e302}}漢字)\" c¥\ne\u{301}\u{202e}\u{202e}\u{1b}[31m\"\u{1b}[31m\u{1059ff}\"\"\t\u{4eea6}\r)漢字;\u{1b}\tc\r;w\u{1b}[31m j:\ne\u{301}\u{e69f7}(\t\u{7f} u漢字}\r}}\u{202e}e\u{301}\r=Zf\n;e\u{301}漢字\"s\ne\u{301}g \u{1b}[31mse\u{301}\r漢字=;( =\tiq=\"\nѨ;=t==\"\" \r}\r\u{1b}[31m\n\"d}\\h\n\r漢字 (\n}};\u{202e}\r(ue\u{301};\r\" \rv\u{202e};\r漢字\r\r\"*}\t( y\r\\\n==漢字\"\t漢字)(\rx\n漢字漢字\u{1b}[31m\u{c6d35}q; =�)\n\r\n漢字\r=\t\u{202e}\r\u{1b}[31mdj\t\"z\t漢字漢字v漢字e\u{301}\u{202e}\"p}\n e\u{301}漢字o\u{202e}e\u{301}e\u{301}\t;漢字漢字=\u{202e}i\"\r(\r\u{1b}[31ma\tx\u{1b}[31m\n\nl  \u{1b}[31m漢字\u{202e}=漢字漢字(\u{202e}漢字\r\r={\u{202e}=\t=\n\r=\r\u{1b}[31m=\u{1b}[31m;\"漢字;\t\"}漢字\rl\"漢字}}\u{1b}[31my\t{=";
    let start = std::time::Instant::now();
    let diff = FileDiff::compute("src/lib.rs", Some(old.as_bytes()), Some(new.as_bytes()));
    assert!(diff.additions > 0);
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
}
