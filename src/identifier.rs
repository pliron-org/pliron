// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! [Identifier]s are strings used to name entities in programming languages.
//! In pliron, they must satisfy the regex `[a-zA-Z_][a-zA-Z0-9_]*`.

use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
};
use core::{fmt::Display, ops::Add, str::FromStr};
use thiserror::Error;

use crate::{
    arg_err_noloc,
    builtin::attributes::StringAttr,
    combine::{Parser, token},
    impl_printable_for_display,
    parsable::{self, Parsable, ParseResult},
    result::{self, Result},
    utils::table::HMap,
};

#[derive(Clone, Hash, PartialEq, Eq, Debug, PartialOrd, Ord)]
/// An [Identifier] is a string that must satisfy the regex `[a-zA-Z_][a-zA-Z0-9_]*`.
pub struct Identifier(Cow<'static, str>);

impl Identifier {
    /// Checks if a string is a valid [Identifier].
    pub const fn is_valid(s: &str) -> bool {
        let b = s.as_bytes();
        if b.is_empty() {
            return false;
        }
        if !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
            return false;
        }
        let mut i = 1;
        while i < b.len() {
            if !(b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Attempt to construct a new [Identifier] from a [String].
    /// Examples:
    /// ```
    /// use pliron::identifier::Identifier;
    /// let _: Identifier = "hi12".try_into().expect("Identifier creation error");
    /// let _: Identifier = "A12ab".try_into().expect("Identifier creation error");
    /// TryInto::<Identifier>::try_into("hi12.").expect_err("Malformed identifier not caught");
    /// TryInto::<Identifier>::try_into("12ab").expect_err("Malformed identifier not caught");
    /// TryInto::<Identifier>::try_into(".a12ab").expect_err("Malformed identifier not caught");
    /// ```
    pub fn try_new(value: String) -> Result<Self> {
        if Identifier::is_valid(&value) {
            Ok(Identifier(Cow::Owned(value)))
        } else {
            arg_err_noloc!(MalformedIdentifierErr(value.clone()))
        }
    }

    /// Construct an [Identifier] from a static string.
    ///
    /// **Panics** if `!Identifier::is_valid(value)`.
    ///
    /// It is suggested to use
    ///   - The [ident!](crate::ident) macro, which wraps the value in a `const` scope,
    ///     thus statically validating it.
    ///   - [Self::try_new] when value is not known statically.
    #[track_caller]
    pub const fn new(value: &'static str) -> Self {
        if Identifier::is_valid(value) {
            Identifier(Cow::Borrowed(value))
        } else {
            panic!("value is not a valid Identifier");
        }
    }
}

/// Construct an [Identifier] from a string literal, validating it at compile time.
///
/// For dynamically sourced strings, use [Identifier::try_new].
///
/// # Examples
///
/// ```
/// use pliron::ident;
///
/// let identifier = ident!("my_identifier_1");
/// assert_eq!(identifier.as_ref(), "my_identifier_1");
/// ```
///
/// Invalid identifiers are rejected at compile time:
///
/// ```compile_fail
/// use pliron::ident;
///
/// let _ = ident!("not-an-identifier");
/// ```
#[macro_export]
macro_rules! ident {
    ($value:literal) => {{
        const IDENT: $crate::identifier::Identifier = $crate::identifier::Identifier::new($value);
        IDENT
    }};
}

impl Add for Identifier {
    type Output = Identifier;

    fn add(self, rhs: Self) -> Self::Output {
        let mut result = self.0.into_owned();
        result.push_str(rhs.as_ref());
        Identifier(Cow::Owned(result))
    }
}

impl_printable_for_display!(Identifier);

impl Display for Identifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

impl TryFrom<String> for Identifier {
    type Error = result::Error;

    fn try_from(value: String) -> Result<Self> {
        Self::try_new(value)
    }
}

impl TryFrom<&str> for Identifier {
    type Error = result::Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::try_new(value.to_string())
    }
}

impl FromStr for Identifier {
    type Err = result::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::try_new(value.to_string())
    }
}

impl TryFrom<StringAttr> for Identifier {
    type Error = result::Error;

    fn try_from(value: StringAttr) -> Result<Self> {
        Self::try_new(value.into())
    }
}

impl From<Identifier> for String {
    fn from(value: Identifier) -> Self {
        value.0.into_owned()
    }
}

impl AsRef<str> for Identifier {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Error)]
#[error("Malformed identifier {0}")]
struct MalformedIdentifierErr(String);

impl Parsable for Identifier {
    type Arg = ();
    type Parsed = Identifier;

    fn parse<'a>(
        state_stream: &mut parsable::StateStream<'a>,
        _arg: Self::Arg,
    ) -> ParseResult<'a, Self::Parsed> {
        use crate::combine::{many, parser::char};
        let parser = (char::letter().or(token('_')))
            .and(many::<String, _, _>(char::alpha_num().or(char::char('_'))))
            .map(|(c, rest)| c.to_string() + &rest);

        parser
            .map(|str| {
                str.try_into()
                    .expect("Something is wrong in our Identifier parser")
            })
            .parse_stream(state_stream)
            .into()
    }
}

/// A utility to safely (i.e., without collisions) legalise identifiers.
/// Generated [Identifier]s are unique only within an instance of this legaliser.
/// ```
/// use pliron::identifier::{Legaliser, Identifier};
/// let mut legaliser = Legaliser::default();
/// let id1 = legaliser.legalise("hello_");
/// assert_eq!(id1.as_ref(), "hello_");
/// assert_eq!(legaliser.source_name(&id1).unwrap(), "hello_");
/// let id2 = legaliser.legalise("hello.");
/// assert_eq!(id2.as_ref(), "hello__0");
/// assert_eq!(legaliser.source_name(&id2).unwrap(), "hello.");
/// let id3 = legaliser.legalise("hello__0");
/// assert_eq!(id3.as_ref(), "hello__0_1");
/// assert_eq!(legaliser.source_name(&id3).unwrap(), "hello__0");
/// let id4 = legaliser.legalise("");
/// assert_eq!(id4.as_ref(), "_");
/// assert_eq!(legaliser.source_name(&id4).unwrap(), "");
/// let id5 = legaliser.legalise("_");
/// assert_eq!(id5.as_ref(), "__2");
/// assert_eq!(legaliser.source_name(&id5).unwrap(), "_");
///
/// let mut another_legaliser = Legaliser::default();
/// let id6 = another_legaliser.legalise("_");
/// assert_eq!(id6.as_ref(), "_");
/// assert_eq!(another_legaliser.source_name(&id6).unwrap(), "_");
/// let id7 = another_legaliser.legalise("");
/// assert_eq!(id7.as_ref(), "__0");
/// assert_eq!(another_legaliser.source_name(&id7).unwrap(), "");
///
/// ```
#[derive(Default)]
pub struct Legaliser {
    /// A map from the source strings to [Identifier]s.
    str_to_id: HMap<String, Identifier>,
    /// Reverse map from [Identifier]s to their source string.
    rev_str_to_id: HMap<String, String>,
    /// A counter to generate unique (within this object) ids.
    counter: usize,
}

impl Legaliser {
    /// Replace illegal characters with '_'.
    fn replace_illegal_chars(name: &str) -> String {
        if TryInto::<Identifier>::try_into(name).is_ok() {
            return name.to_string();
        }

        if name.is_empty() {
            return String::from("_");
        }

        let mut char_iter = name.chars();
        let first_char = char_iter.next().unwrap();
        let mut result = if first_char.is_alphabetic() {
            String::from(first_char)
        } else {
            String::from("_")
        };

        let rest = char_iter.map(|c| if c.is_ascii_alphanumeric() { c } else { '_' });
        result.extend(rest);

        result
    }

    /// Get a legal [Identifier] for input name.
    pub fn legalise(&mut self, name: &str) -> Identifier {
        // If we've already mapped this before, just return that.
        if let Some(id) = self.str_to_id.get(name) {
            return id.clone();
        }

        let legal_name = Self::replace_illegal_chars(name);
        let mut legal_name_unique = legal_name.clone();
        // Until this is not already a mapped identifier, create unique ones.
        while self.rev_str_to_id.contains_key(&legal_name_unique) {
            legal_name_unique = legal_name.clone() + &format!("_{}", self.counter);
            self.counter += 1;
        }

        let legal_name_id = Identifier(Cow::Owned(legal_name_unique.clone()));
        self.str_to_id
            .insert(name.to_string(), legal_name_id.clone());
        self.rev_str_to_id
            .insert(legal_name_unique, name.to_string());

        legal_name_id
    }

    /// Get the source name from which this [Identifier] was mapped to.
    pub fn source_name(&self, id: &Identifier) -> Option<String> {
        self.rev_str_to_id.get(id.as_ref()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cmp::Ordering;

    /// Ensure equivalence of two [Identifier]s holding the same text, irrespective
    /// of how they are stored internally (`'static str` or an owned [String]).
    #[test]
    fn const_and_owned_are_equivalent() {
        let konst = ident!("foo");
        let owned = Identifier::try_new("foo".to_string()).unwrap();

        assert_eq!(konst, owned);
        assert_eq!(konst.cmp(&owned), Ordering::Equal);
        assert!(ident!("z") > Identifier::try_new("a".to_string()).unwrap());

        // Equal keys must also hash equally.
        let mut map = HMap::<Identifier, u32>::default();
        map.insert(owned.clone(), 42);
        assert_eq!(map.get(&konst), Some(&42));

        // Debug output must not show how the value was built.
        assert_eq!(format!("{konst:?}"), format!("{owned:?}"));
    }
}
