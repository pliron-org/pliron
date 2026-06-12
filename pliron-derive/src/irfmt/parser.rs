use combine::{
    Parser, between, choice, many, one_of, optional,
    parser::{
        char::spaces,
        range::{recognize, take_while1},
        repeat::escaped,
    },
    position, sep_by,
    stream::position::IndexPositioner,
    token,
};

use super::{Elem, Format};

type Stream<'a> = combine::stream::position::Stream<&'a str, IndexPositioner>;

pub(crate) type Result<T, E = Error> = core::result::Result<T, E>;

pub(crate) type Error = Box<dyn core::error::Error>;

/// Parse a format string.
///
/// A format string is a sequence of literals, variables, and directives. The terms are optionally
/// separated by whitespaces, which will be ignored in the parsed format.
///
/// * Literal strings are enclosed in backticks, and may contain escaped characters.
/// * Variables (named / unnamed) begin with a dollar sign and are followed by an
///   identifier (named) or a number (unnamed).
/// * Directives are identifiers followed by an optional list of arguments enclosed in parentheses.
/// * In case no arguments are provided, the parentheses may be omitted.
pub(crate) fn parse(input: &str) -> Result<Format> {
    let input = Stream::with_positioner(input, IndexPositioner::new());
    let (elems, _rest) = match parse_fmt_elems().parse(input) {
        Ok(elems) => elems,
        Err(err) => {
            let msg = format!("{err}");
            return Err(msg.into());
        }
    };
    Ok(Format { elems })
}

fn parse_fmt_elems<'a>() -> impl Parser<Stream<'a>, Output = Vec<Elem>> {
    many(parse_fmt_elem())
}

combine::parser! {
    fn parse_fmt_elem['a]()(Stream<'a>) -> Elem where [] {
        parse_fmt_elem_()
    }
}

fn parse_fmt_elem_<'a>() -> impl Parser<Stream<'a>, Output = Elem> {
    spaces()
        .with(choice((parse_lit(), parse_var(), parse_directive())))
        .skip(spaces())
}

fn parse_lit<'a>() -> impl Parser<Stream<'a>, Output = Elem> {
    let body = recognize(escaped(
        take_while1(|c| c != '`' && c != '\\'),
        '\\',
        one_of(r#"`nrt\"#.chars()),
    ));
    let lit = between(token('`'), token('`'), body);
    (position(), lit).map(|(pos, s)| Elem::new_lit_at(pos, s))
}

fn parse_var<'a>() -> impl Parser<Stream<'a>, Output = Elem> {
    let tok = token('$').with(take_while1(|c: char| c.is_alphanumeric() || c == '_'));
    (position(), tok).map(|(pos, s): (_, &str)| match s.parse::<usize>() {
        Ok(n) => Elem::new_unnamed_var_at(pos, n),
        Err(_) => Elem::new_var_at(pos, s),
    })
}

fn parse_directive<'a>() -> impl Parser<Stream<'a>, Output = Elem> {
    let name = take_while1(|c: char| c.is_alphanumeric() || c == '-' || c == '_').skip(spaces());
    let args = between(token('('), token(')'), sep_by(parse_fmt_elem(), token(',')));
    (position(), name, optional(args)).map(|(pos, name, args)| {
        Elem::new_directive_with_args_at(pos, name, args.unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_literal() {
        let input = "`lit`";
        let want = vec![Elem::new_lit_at(0, "lit")];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }

    #[test]
    fn literal_with_escaped_chars() {
        let input = r#"`hello\n \`world\``"#;
        let want = vec![Elem::new_lit_at(0, r#"hello\n \`world\`"#)];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }

    #[test]
    fn simple_variable() {
        let input = "$var";
        let want = vec![Elem::new_var_at(0, "var")];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }

    #[test]
    fn simple_unnamed_variable() {
        let input = "$1";
        let want = vec![Elem::new_unnamed_var_at(0, 1)];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }

    #[test]
    fn simple_directive() {
        let input = "directive";
        let want = vec![Elem::new_directive_at(0, "directive")];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }

    #[test]
    fn directive_with_empty_args() {
        let input = "directive()";
        let want = vec![Elem::new_directive_at(0, "directive")];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }

    #[test]
    fn directive_with_args() {
        let input = "directive($arg1, other-directive)";
        let want = vec![Elem::new_directive_with_args_at(
            0,
            "directive",
            vec![
                Elem::new_var_at(10, "arg1"),
                Elem::new_directive_at(17, "other-directive"),
            ],
        )];
        let got = parse(input).unwrap();
        assert_eq!(got.elems, want);
    }
}
