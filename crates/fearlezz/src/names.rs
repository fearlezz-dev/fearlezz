//! Words config uses to pick one of a fixed set, and the list an error gives for a word
//! that names none.

/// One of a fixed set that config names by a word.
pub(crate) trait Named: Copy + PartialEq + 'static {
    /// Each one with its name, in the order an error lists them.
    const NAMES: &'static [(&'static str, Self)];

    fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, t)| *t == self)
            .map_or("", |(n, _)| n)
    }

    /// The one called `name`, or else every name for the error to list.
    fn named(name: &str) -> Result<Self, String> {
        find(Self::NAMES, name)
    }
}

/// The value called `name` in `table`, or else every name in it, joined by ", ".
pub(crate) fn find<T: Copy>(table: &[(&str, T)], name: &str) -> Result<T, String> {
    table
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| *t)
        .ok_or_else(|| table.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "))
}
