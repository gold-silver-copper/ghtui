//! [`words!`]: an enum whose variants each have fixed words, declared as
//! one table so a variant and its words are written once.

/// Declares an enum from one table: a row per variant, a column per
/// `&'static str` function. It derives `Debug, Clone, Copy, PartialEq,
/// Eq` (more go above the enum) and gives `ALL`, in the rows' order. A
/// column written `fn slug -> fn from_slug` also gets its reverse
/// lookup, which finds the variant with that word.
///
/// ```text
/// words! {
///     #[derive(Hash)]
///     pub enum Since { pub fn slug -> pub fn from_slug, pub fn label }
///     Daily   "daily"   "Today";
///     Weekly  "weekly"  "This week";
/// }
/// ```
macro_rules! words {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Enum:ident { $($columns:tt)* }
        $($(#[$vmeta:meta])* $V:ident $($word:literal)*;)*
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis enum $Enum {
            $($(#[$vmeta])* $V,)*
        }

        impl $Enum {
            pub const ALL: [$Enum; [$(stringify!($V)),*].len()] = [$($Enum::$V),*];

            words!(@columns [$($columns)*] [$($V [$($word)*])*]);
        }
    };
    // Each column takes its rows' first words, and passes on the rest.
    (@columns [] [$($rows:tt)*]) => {};
    (
        @columns
        [$(#[$cmeta:meta])* $cvis:vis fn $column:ident $(-> $rvis:vis fn $reverse:ident)? $(, $($rest:tt)*)?]
        [$($V:ident [$word:literal $($words:literal)*])*]
    ) => {
        $(#[$cmeta])*
        $cvis fn $column(self) -> &'static str {
            match self {
                $(Self::$V => $word,)*
            }
        }

        $(
            $rvis fn $reverse(word: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|v| v.$column() == word)
            }
        )?

        words!(@columns [$($($rest)*)?] [$($V [$($words)*])*]);
    };
}
