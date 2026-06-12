//! [Identifier]s are strings used to name entities in programming languages.

use alloc::{
    format,
    string::{String, ToString},
};
use core::{
    fmt::Display,
    ops::{Add, Deref},
};
use rustc_hash::FxHashMap;
use thiserror::Error;

use crate::{
    builtin::attributes::StringAttr,
    combine::{Parser, token},
    impl_printable_for_display,
    parsable::{self, Parsable, ParseResult},
    result::{self, Result},
    verify_err_noloc,
};

#[derive(Clone, Hash, PartialEq, Eq, Debug, PartialOrd, Ord)]
/// An [Identifier] must satisfy the regex `[a-zA-Z_][a-zA-Z0-9_]*`.
/// Also see [module description](module@crate::identifier).
pub struct Identifier(String);

impl Identifier {
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
        let mut chars_iter = value.chars();
        match chars_iter.next() {
            Some(first_char) if (first_char.is_ascii_alphabetic() || first_char == '_') => {
                if !chars_iter.all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return verify_err_noloc!(MalformedIdentifierErr(value.clone()));
                }
            }
            _ => {
                return verify_err_noloc!(MalformedIdentifierErr(value.clone()));
            }
        }
        Ok(Identifier(value))
    }
}

impl Add for Identifier {
    type Output = Identifier;

    fn add(self, rhs: Self) -> Self::Output {
        Identifier(self.0 + &rhs.0)
    }
}

impl_printable_for_display!(Identifier);

impl Display for Identifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for Identifier {
    type Error = result::Error;

    fn try_from(value: String) -> core::result::Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl TryFrom<&str> for Identifier {
    type Error = result::Error;

    fn try_from(value: &str) -> core::result::Result<Self, Self::Error> {
        Self::try_new(value.to_string())
    }
}

impl TryFrom<StringAttr> for Identifier {
    type Error = result::Error;

    fn try_from(value: StringAttr) -> core::result::Result<Self, Self::Error> {
        Self::try_new(value.into())
    }
}

impl From<Identifier> for String {
    fn from(value: Identifier) -> Self {
        value.0
    }
}

impl Deref for Identifier {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A fast way to get just the "_" character as a string.
pub fn underscore() -> Identifier {
    Identifier("_".to_string())
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
/// Generated [Identifier]s are unique only within this the this object.
/// ```
/// use pliron::identifier::{Legaliser, Identifier};
/// let mut legaliser = Legaliser::default();
/// let id1 = legaliser.legalise("hello_");
/// assert_eq!(*id1, "hello_");
/// assert_eq!(legaliser.source_name(&id1).unwrap(), "hello_");
/// let id2 = legaliser.legalise("hello.");
/// assert_eq!(*id2, "hello__0");
/// assert_eq!(legaliser.source_name(&id2).unwrap(), "hello.");
/// let id3 = legaliser.legalise("hello__0");
/// assert_eq!(*id3, "hello__0_1");
/// assert_eq!(legaliser.source_name(&id3).unwrap(), "hello__0");
/// let id4 = legaliser.legalise("");
/// assert_eq!(*id4, "_");
/// assert_eq!(legaliser.source_name(&id4).unwrap(), "");
/// let id5 = legaliser.legalise("_");
/// assert_eq!(*id5, "__2");
/// assert_eq!(legaliser.source_name(&id5).unwrap(), "_");
///
/// let mut another_legaliser = Legaliser::default();
/// let id6 = another_legaliser.legalise("_");
/// assert_eq!(*id6, "_");
/// assert_eq!(another_legaliser.source_name(&id6).unwrap(), "_");
/// let id7 = another_legaliser.legalise("");
/// assert_eq!(*id7, "__0");
/// assert_eq!(another_legaliser.source_name(&id7).unwrap(), "");
///
/// ```
#[derive(Default)]
pub struct Legaliser {
    /// A map from the source strings to [Identifier]s.
    str_to_id: FxHashMap<String, Identifier>,
    /// Reverse map from [Identifier]s to their source string.
    rev_str_to_id: FxHashMap<String, String>,
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

        let legal_name_id = Identifier(legal_name_unique.clone());
        self.str_to_id
            .insert(name.to_string(), legal_name_id.clone());
        self.rev_str_to_id
            .insert(legal_name_unique.clone(), name.to_string());

        legal_name_id
    }

    /// Get the source name from which this [Identifier] was mapped to.
    pub fn source_name(&self, id: &Identifier) -> Option<String> {
        self.rev_str_to_id.get(&id.0).cloned()
    }
}
