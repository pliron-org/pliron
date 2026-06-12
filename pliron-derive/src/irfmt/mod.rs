use proc_macro2::{Span, TokenStream};
use quote::format_ident;
use syn::{
    self, Data, DataEnum, DeriveInput, Fields, Ident, LitStr, Type,
    parse::{Parse, ParseStream},
};

mod parser;

pub(crate) enum FmtData {
    Struct(Struct),
    Enum(Enum),
}

impl Parse for FmtData {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let input = DeriveInput::parse(input)?;
        Self::try_from(input)
    }
}

impl TryFrom<DeriveInput> for FmtData {
    type Error = syn::Error;

    fn try_from(input: DeriveInput) -> syn::Result<Self> {
        match input.data {
            Data::Struct(ref data) => {
                Struct::from_syn(input.ident, &data.fields, false).map(FmtData::Struct)
            }
            Data::Enum(ref data) => Enum::from_syn(input.ident, data).map(FmtData::Enum),
            Data::Union(_) => Err(syn::Error::new_spanned(
                &input,
                "Format can only be derived for structs and enums",
            )),
        }
    }
}

/// Enum format data.
#[derive(Clone)]
pub(crate) struct Enum {
    pub name: Ident,
    pub variants: Vec<(Format, Struct)>,
}

/// Struct format data.
#[derive(Clone)]
pub(crate) struct Struct {
    pub name: Ident,
    pub fields: Vec<Field>,
    // Whether the struct is for an enum variant
    pub is_enum_variant: bool,
}

impl Struct {
    fn from_syn(name: Ident, fields: &Fields, is_enum_variant: bool) -> syn::Result<Self> {
        let fields = fields
            .iter()
            .enumerate()
            .map(|(i, f)| match f.ident {
                Some(ref ident) => Field {
                    ty: f.ty.clone(),
                    ident: FieldIdent::Named(ident.to_string()),
                },
                None => Field {
                    ty: f.ty.clone(),
                    ident: FieldIdent::Unnamed(i),
                },
            })
            .collect();

        Ok(Self {
            name,
            fields,
            is_enum_variant,
        })
    }
}

impl Enum {
    fn from_syn(name: Ident, data: &DataEnum) -> syn::Result<Self> {
        let variants =
            data.variants
                .iter()
                .map(|v| {
                    // parse the variant's format string given as an attribute
                    let fmt_str = v
                        .attrs
                        .iter()
                        .find_map(|attr| {
                            if attr.path().is_ident("format") {
                                Some(attr.parse_args::<LitStr>())
                            } else {
                                None
                            }
                        })
                        .transpose()?;
                    let r#struct = Struct::from_syn(v.ident.clone(), &v.fields, true)?;

                    let format = match fmt_str {
                        Some(str) => Format::parse(&str.value())
                            .map_err(|e| syn::Error::new_spanned(str, e))?,
                        None => canonical_format_for_structs(
                            &FmtData::Struct(r#struct.clone()),
                            name.span(),
                        )?,
                    };
                    Ok((format, r#struct))
                })
                .collect::<Result<_, syn::Error>>()?;

        Ok(Self { name, variants })
    }
}

#[derive(Clone)]
pub(crate) struct Field {
    pub(crate) ty: Type,
    pub(crate) ident: FieldIdent,
}

/// Field identifier of an IR entity that can be used a variable in a format string.
/// We extract struct fields as field identifiers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FieldIdent {
    Named(String),
    Unnamed(usize),
}

impl From<FieldIdent> for Elem {
    fn from(value: FieldIdent) -> Self {
        match value {
            FieldIdent::Named(name) => Elem::new_var(name),
            FieldIdent::Unnamed(index) => Elem::new_unnamed_var(index),
        }
    }
}

impl From<&FieldIdent> for Elem {
    fn from(value: &FieldIdent) -> Self {
        match value {
            FieldIdent::Named(name) => Elem::new_var(name),
            FieldIdent::Unnamed(index) => Elem::new_unnamed_var(*index),
        }
    }
}

impl From<&str> for FieldIdent {
    fn from(s: &str) -> Self {
        Self::Named(s.to_string())
    }
}

impl From<String> for FieldIdent {
    fn from(s: String) -> Self {
        Self::Named(s)
    }
}

impl From<usize> for FieldIdent {
    fn from(i: usize) -> Self {
        Self::Unnamed(i)
    }
}

impl quote::ToTokens for FieldIdent {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Named(name) => {
                let ident = format_ident!("{}", name);
                ident.to_tokens(tokens);
            }
            Self::Unnamed(index) => {
                let ident = syn::Index::from(*index);
                ident.to_tokens(tokens);
            }
        }
    }
}

/// A parsed format string.
///
/// The format string is a sequence of literals, variables, and directives.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Format {
    pub elems: Vec<Elem>,
}

impl From<Vec<Elem>> for Format {
    fn from(elems: Vec<Elem>) -> Self {
        Self { elems }
    }
}

impl Format {
    pub fn is_empty(&self) -> bool {
        self.elems.is_empty()
    }

    pub fn prepend(&mut self, elem: impl Into<Elem>) {
        self.elems.insert(0, elem.into());
    }

    pub fn append(&mut self, elem: impl Into<Elem>) {
        self.elems.push(elem.into());
    }

    pub fn enclose(&mut self, open: impl Into<Elem>, close: impl Into<Elem>) {
        self.prepend(open);
        self.append(close);
    }
}

impl Format {
    /// Parse a format string.
    pub fn parse(input: &str) -> parser::Result<Self> {
        parser::parse(input)
    }
}

/// A single element term in a format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Elem {
    /// Literal is a custom string enclosed in backticks. For example `lit` or `(`.
    Lit(Lit),

    /// Varialbes are custom identifiers starting with a dollar sign. For example $var.
    Var(Var),

    /// Unnamed variables are custom identifiers starting with a dollar sign and a number.
    /// For example $0.
    UnnamedVar(UnnamedVar),

    /// Directives are builtin identifiers. Some directives may have optional arguments enclosed
    /// in parens. For example `attr-dict` or `directive($arg1, other-directive)`.
    Directive(Directive),
}

impl Default for Elem {
    fn default() -> Self {
        Self::Lit(Lit::new(""))
    }
}

impl Elem {
    pub fn new_lit(s: impl Into<String>) -> Self {
        Self::Lit(Lit::new(s))
    }

    pub fn new_lit_at(pos: usize, s: impl Into<String>) -> Self {
        Self::Lit(Lit::new_at(pos, s))
    }

    pub fn new_var(s: impl Into<String>) -> Self {
        Self::Var(Var::new(s))
    }

    pub fn new_var_at(pos: usize, s: impl Into<String>) -> Self {
        Self::Var(Var::new_at(pos, s.into()))
    }

    pub fn new_unnamed_var(index: usize) -> Self {
        Self::UnnamedVar(UnnamedVar::new(index))
    }

    pub fn new_unnamed_var_at(pos: usize, index: usize) -> Self {
        Self::UnnamedVar(UnnamedVar::new_at(pos, index))
    }

    #[allow(dead_code)] // used in tests.
    pub fn new_directive_at(pos: usize, name: impl Into<String>) -> Self {
        Self::Directive(Directive::new_at(pos, name))
    }

    #[allow(dead_code)]
    pub fn new_directive_with_args(name: impl Into<String>, args: Vec<Elem>) -> Self {
        Self::Directive(Directive::new_with_args(name, args))
    }

    pub fn new_directive_with_args_at(
        pos: usize,
        name: impl Into<String>,
        args: Vec<Elem>,
    ) -> Self {
        Self::Directive(Directive::new_with_args_at(pos, name, args))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lit {
    pub pos: Option<usize>,
    pub lit: String,
}

impl From<Lit> for Elem {
    fn from(lit: Lit) -> Self {
        Self::Lit(lit)
    }
}

impl From<&str> for Lit {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl Lit {
    pub fn new(s: impl Into<String>) -> Self {
        Self {
            pos: None,
            lit: s.into(),
        }
    }

    pub fn new_at(pos: usize, s: impl Into<String>) -> Self {
        Self {
            pos: Some(pos),
            lit: s.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Var {
    pub pos: Option<usize>,
    pub name: String,
}

impl Var {
    pub fn new(s: impl Into<String>) -> Self {
        Self {
            pos: None,
            name: s.into(),
        }
    }

    pub fn new_at(pos: usize, s: impl Into<String>) -> Self {
        Self {
            pos: Some(pos),
            name: s.into(),
        }
    }
}

impl From<Var> for Elem {
    fn from(lit: Var) -> Self {
        Self::Var(lit)
    }
}

impl From<&str> for Var {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnnamedVar {
    pub pos: Option<usize>,
    pub index: usize,
}

impl From<UnnamedVar> for Elem {
    fn from(var: UnnamedVar) -> Self {
        Self::UnnamedVar(var)
    }
}

impl From<usize> for UnnamedVar {
    fn from(index: usize) -> Self {
        Self::new(index)
    }
}

impl UnnamedVar {
    pub fn new(index: usize) -> Self {
        Self { pos: None, index }
    }

    pub fn new_at(pos: usize, index: usize) -> Self {
        Self {
            pos: Some(pos),
            index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Directive {
    pub pos: Option<usize>,
    pub name: String,
    pub args: Vec<Elem>,
}

impl Directive {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            pos: None,
            name: name.into(),
            args: vec![],
        }
    }

    pub fn new_at(pos: usize, name: impl Into<String>) -> Self {
        Self {
            pos: Some(pos),
            name: name.into(),
            args: vec![],
        }
    }

    pub fn new_with_args(name: impl Into<String>, args: Vec<Elem>) -> Self {
        Self {
            pos: None,
            name: name.into(),
            args,
        }
    }

    pub fn new_with_args_at(pos: usize, name: impl Into<String>, args: Vec<Elem>) -> Self {
        Self {
            pos: Some(pos),
            name: name.into(),
            args,
        }
    }
}

impl From<Directive> for Elem {
    fn from(directive: Directive) -> Self {
        Self::Directive(directive)
    }
}

struct FmtValue(Vec<Elem>);

impl From<Elem> for FmtValue {
    fn from(elem: Elem) -> Self {
        Self(vec![elem])
    }
}

impl From<Vec<Elem>> for FmtValue {
    fn from(elems: Vec<Elem>) -> Self {
        Self(elems)
    }
}

impl From<Directive> for FmtValue {
    fn from(d: Directive) -> Self {
        Self(vec![Elem::Directive(d)])
    }
}

impl From<FmtValue> for Vec<Elem> {
    fn from(value: FmtValue) -> Self {
        value.0
    }
}

impl From<FmtValue> for Format {
    fn from(value: FmtValue) -> Self {
        Self { elems: value.0 }
    }
}

pub(crate) fn canonical_op_format() -> Format {
    Format {
        elems: vec![Directive::new("canonical").into()],
    }
}
/// Enums have just one preset format, which is:
/// "Variant <field1 = ..., field2 = ... >".
pub(crate) fn canonical_format_for_enums() -> Format {
    Format {
        elems: vec![Directive::new("canonical").into()],
    }
}

/// Describe a canonical syntax for types / attributes defined by a struct.
/// This is just "<field1 = ..., field2 = ... >".
pub(crate) fn canonical_format_for_structs(input: &FmtData, span: Span) -> syn::Result<Format> {
    // TODO: add support for per field attributes?

    let FmtData::Struct(data) = input else {
        return Err(syn::Error::new(
            span,
            "Format can only be derived for structs",
        ));
    };

    let mut elems = vec![];
    let mut is_named = false;
    for (i, field) in data.fields.iter().enumerate() {
        match &field.ident {
            FieldIdent::Named(field) => {
                is_named = true;
                if i > 0 {
                    elems.push(Elem::new_lit(","));
                }
                elems.push(Elem::new_lit(field));
                elems.push(Elem::new_lit("=".to_string()));
                elems.push(Elem::new_var(field));
            }
            FieldIdent::Unnamed(field) => {
                elems.push(Elem::new_unnamed_var(*field));
            }
        }
    }

    let mut format = Format { elems };
    if !format.is_empty() {
        if is_named {
            format.enclose(Elem::Lit("{".into()), Elem::Lit("}".into()));
        } else {
            format.enclose(Elem::Lit("(".into()), Elem::Lit(")".into()));
        }
    }
    Ok(format)
}
