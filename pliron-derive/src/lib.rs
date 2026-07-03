mod derive_attr;
mod derive_entity;
mod derive_format;
mod derive_op;
mod derive_type;
mod interfaces;
mod irfmt;
mod verify_succ;

use proc_macro::TokenStream;
use syn::parse_quote;

use derive_format::DeriveIRObject;

/// `#[def_attribute(...)]`: Annotate a Rust struct as a new IR attribute.
///
/// *Note*: It is suggested to use the [pliron_attr] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_attr]'s `name` field expands
///         to this macro.
///
/// The argument to the macro is the fully qualified name of the attribute in the form of
/// `"dialect.attribute_name"`.
///
/// The macro will leave the struct definition unchanged, but it will generate an implementation of
/// the pliron::Attribute trait and implements other internal traits and types resources required
/// to use the IR attribute.
///
/// **Note**: pre-requisite traits for `Attribute` must already be implemented.
///         Additionaly, [Eq] and [Hash](core::hash::Hash) must be implemented by the type.
///
/// Usage:
///
/// ```
/// use pliron::derive::{def_attribute, format_attribute, verify_succ};
///
/// #[verify_succ]
/// #[def_attribute("my_dialect.attribute")]
/// #[format_attribute]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// pub struct StringAttr(String);
/// # use pliron::{printable::{State, Printable}, context::Context};
/// ```
#[proc_macro_attribute]
pub fn def_attribute(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_attr::def_attribute(args, input))
}

/// `#[def_type(...)]`: Annotate a Rust struct as a new IR type.
///
/// *Note*: It is suggested to use the [pliron_type] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_type]'s `name` field expands
///         to this macro.
///
/// The argument to the macro is the fully qualified name of the type in the form of
/// `"dialect.type_name"`.
///
/// The macro will leave the struct definition unchanged, but it will generate an implementation of
/// the pliron::Type trait and implements other internal traits and types resources required
/// to use the IR type.
///
/// **Note**: pre-requisite traits for `Type` must already be implemented.
///         Additionaly, [Hash](core::hash::Hash) and [Eq] must be implemented by the rust type.
///
/// Usage:
///
/// ```
/// use pliron::derive::{def_type, format_type, verify_succ};
/// #[verify_succ]
/// #[def_type("my_dialect.unit")]
/// #[format_type]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// pub struct UnitType;
/// ```
#[proc_macro_attribute]
pub fn def_type(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_type::def_type(args, input))
}

/// Derive get methods for types that retrieve interned types.
///
/// *Note*: It is suggested to use the [pliron_type] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_type]'s `generate_get` field
///         expands to this macro.
///
/// This macro generates a `get` method that returns a uniqued instance of the type.
/// For unit structs (no fields), it takes only a `Context` parameter.
/// For structs with fields, it takes a `Context` parameter plus a parameter for each field.
///
/// ## Examples
///
/// ### Named fields struct:
/// ```
/// use pliron::derive::{def_type, derive_type_get, format_type, verify_succ};
/// use pliron::context::Context;
///
/// #[verify_succ]
/// #[def_type("my_dialect.vector_type")]
/// #[format_type]
/// #[derive_type_get]  // Auto-generates get method
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// pub struct VectorType {
///     elem_ty: u32,
///     num_elems: u32,
/// }
///
/// // Usage of the auto-generated get method:
/// # fn example(ctx: &Context) {
/// let vector_type = VectorType::get(ctx, 42, 8); // get(ctx, elem_ty, num_elems)
/// # }
/// ```
///
/// ### Tuple struct:
/// ```
/// use pliron::derive::{def_type, derive_type_get, format_type, verify_succ};
/// use pliron::context::Context;
///
/// #[verify_succ]
/// #[def_type("my_dialect.tuple_type")]
/// #[format_type]
/// #[derive_type_get]  // Auto-generates get method
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// pub struct TupleType(u32, String, bool);
///
/// // Usage of the auto-generated get method:
/// # fn example(ctx: &Context) {
/// let tuple_type = TupleType::get(ctx, 42, "hello".to_string(), true); // get(ctx, field_0, field_1, field_2)
/// # }
/// ```
///
/// ### Unit struct:
/// ```
/// use pliron::derive::{def_type, derive_type_get, format_type, verify_succ};
/// use pliron::context::Context;
///
/// #[verify_succ]
/// #[def_type("my_dialect.unit_type")]
/// #[format_type]
/// #[derive_type_get]  // Auto-generates get method
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// pub struct UnitType;
///
/// // Usage of the auto-generated get method:
/// # fn example(ctx: &Context) {
/// let unit_type = UnitType::get(ctx); // get(ctx) - no additional parameters
/// # }
/// ```
#[proc_macro_attribute]
pub fn derive_type_get(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_type::derive_type_get(args, input))
}

/// `#[verify_succ]`: Implement [Verify](../pliron/common_traits/trait.Verify.html)
/// for a Rust struct or enum with a verifier that always succeeds.
///
/// This leaves the original item unchanged and adds:
/// `impl Verify for T { fn verify(...) -> Result<()> { Ok(()) } }`.
///
/// Usage:
///
/// ```
/// use pliron::derive::verify_succ;
/// use pliron::{common_traits::Verify, context::Context};
///
/// #[verify_succ]
/// struct AlwaysValid;
///
/// let ctx = Context::new();
/// assert!(AlwaysValid.verify(&ctx).is_ok());
/// ```
#[proc_macro_attribute]
pub fn verify_succ(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(verify_succ::verify_succ_impl(args.into(), input.into()))
}

/// `#[def_op(...)]`: Create a new IR operation.
///
/// *Note*: It is suggested to use the [pliron_op] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_op]'s `name` field expands
///         to this macro.
///
/// The argument to the macro is the fully qualified name of the operation in the form of
/// `"dialect.op_name"`.
///
/// The macro assumes an empty struct and will add the `op: Ptr<Operation>` field used to access
/// the underlying Operation in the context.
///
/// The macro will automatically derive the `Clone`, `Copy`, `Hash`, `PartialEq` and `Eq` traits
/// for the new struct definition.
///
/// **Note**: pre-requisite traits for `Op` (Printable, Verify etc) must already be implemented
///
/// Usage:
///
/// ```
/// use pliron::derive::{def_op, format_op, verify_succ};
///
/// #[verify_succ]
/// #[def_op("my_dialect.op")]
/// #[format_op]
/// pub struct MyOp;
/// ```
/// The example will create a struct definition equivalent to:
///
/// ```
/// #[derive(Clone, Copy, PartialEq, Eq, Hash)]
/// pub struct MyOp {
///   op: Ptr<Operation>,
/// }
/// # use pliron::{context::Ptr, operation::Operation};
/// ```
#[proc_macro_attribute]
pub fn def_op(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_op::def_op(args, input))
}

/// Derive getter and setters for operation attributes listed as arguments.
///
/// *Note*: It is suggested to use the [pliron_op] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_op]'s `attributes` field
///         expands to this macro.
///
/// The arguments are a comma separated list of attribute names
/// (which must be an [Identifier](../pliron/identifier/struct.Identifier.html)),
/// each of which may have an optional concrete Rust type specified,
/// denoting the [Attribute](../pliron/attribute/trait.Attribute.html)'s concrete type.
///
/// ```
/// # use pliron::derive::{def_op, derive_attr_get_set, format_op, verify_succ};
/// // A test for the `derive_attr_get_set` macro.
/// #[verify_succ]
/// #[def_op("llvm.with_attrs")]
/// #[format_op]
/// #[derive_attr_get_set(name1_any_attr, name2_ty_attr : pliron::builtin::attributes::TypeAttr)]
/// pub struct WithAttrsOp {}
/// ```
///
/// This expands to add the following getter / setter items:
/// ```Rust
/// # use pliron::derive::{def_op, format_op, derive_attr_get_set};
/// # use core::cell::Ref;
/// # use pliron::dict_key;
/// # use pliron::{attribute::AttrObj, context::Context};
/// # use pliron::{builtin::attributes::TypeAttr};
/// # use pliron::derive::verify_succ;
/// # #[verify_succ]
/// #[format_op]
/// #[def_op("llvm.with_attrs")]
/// pub struct WithAttrsOp {}
/// dict_key!(ATTR_KEY_NAME1_ANY_ATTR, "name1_any_attr");
/// dict_key!(ATTR_KEY_NAME2_TY_ATTR, "name2_ty_attr");
/// impl WithAttrsOp {
///   pub fn get_attr_name1_any_attr<'a>
///     (&self, ctx: &'a Context)-> Option<Ref<'a, AttrObj>> { todo!() }
///   pub fn set_attr_name1_any_attr(&self, ctx: &Context, value: AttrObj) { todo!() }
///   pub fn get_attr_name2_ty_attr<'a>
///     (&self, ctx: &'a Context) -> Option<Ref<'a, TypeAttr>> { todo!() }
///   pub fn set_attr_name2_ty_attr(&self, ctx: &Context, value: TypeAttr) { todo!() }
/// }
/// ```
#[proc_macro_attribute]
pub fn derive_attr_get_set(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_op::derive_attr_get_set(args, input))
}

/// Derive getter methods and / or operand type interfaces for operation operands.
///
/// *Note*: It is suggested to use the [pliron_op] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_op]'s `operands` field expands
///         to this macro.
///
/// The arguments are a comma-separated list where each entry is:
/// - `name` or `name: Type` for a named operand getter `get_operand_<name>()`.
/// - `_` or `_: Type` to skip getter generation for that position.
///
/// When `Type` is provided, this macro derives
/// [OperandNOfType](../pliron/builtin/op_interfaces/trait.OperandNOfType.html)
/// for the corresponding operand index.
///
/// The op is allowed to have more operands than those specified in the macro arguments.
/// They just won't have getters or type interfaces generated for them.
///
/// ```
/// use pliron::derive::{def_op, format_op, operands, verify_succ};
/// use pliron::builtin::types::{IntegerType, UnitType};
///
/// #[verify_succ]
/// #[def_op("dialect.with_operands")]
/// #[format_op]
/// #[operands(lhs: IntegerType, _, rhs, _: UnitType)]
/// pub struct WithOperandsOp {}
/// ```
#[proc_macro_attribute]
pub fn operands(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_op::operands(args, input))
}

/// Derive getter methods and / or result type interfaces for operation results.
///
/// *Note*: It is suggested to use the [pliron_op] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_op]'s `results` field expands
///         to this macro.
///
/// The arguments are a comma-separated list where each entry is:
/// - `name` or `name: Type` for a named result getter `get_result_<name>()`.
/// - `_` or `_: Type` to skip getter generation for that position.
///
/// When `Type` is provided, this macro derives
/// [ResultNOfType](../pliron/builtin/op_interfaces/trait.ResultNOfType.html)
/// for the corresponding result index.
///
/// The op is allowed to have more results than those specified in the macro arguments.
/// They just won't have getters or type interfaces generated for them.
///
/// ```
/// use pliron::derive::{def_op, format_op, results, verify_succ};
/// use pliron::builtin::types::{IntegerType, UnitType};
///
/// #[verify_succ]
/// #[def_op("dialect.with_results")]
/// #[format_op]
/// #[results(out: IntegerType, _: UnitType)]
/// pub struct WithResultsOp {}
/// ```
#[proc_macro_attribute]
pub fn results(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_op::results(args, input))
}

/// Derive [Printable](../pliron/printable/trait.Printable.html) and
/// [Parsable](../pliron/parsable/trait.Parsable.html) for Rust types.
/// Use this for types other than `Op`, `Type` and `Attribute`s.
///
/// A format string can be specified as an argument to the macro, to customize the syntax.
/// Without a format string, the default syntax is used. For enums, no format string is allowed
/// on the enum itself, but it is allowed on its variants.
///
/// Primarily, the following two are used to refer to fields in a struct or tuple:
///   1. A named variable `$name` specifies a named struct field.
///   2. An unnamed variable `$i` specifies the i'th field of a tuple struct.
///
/// Struct (or tuple) fields that are either [Option] or [Vec] (or an array) must to be specified
/// using the `opt` and `vec` directives respectively (i.e., a format string is mandatory).
///
/// The `opt` directive takes one mandatory argument, a variable specifying the field name with
/// type `Option`. It also supports optional `label` and `delimiters` directives:
/// 1. `label($name)`: uses `name :` as a prefix for the optional value.
/// 2. `delimiters(`open`, `close`)`: wraps the optional value with the given delimiters.
///
/// The `vec` directive takes two arguments, the first is a variable specifying the field name
/// with type `Vec` (or array) and the second is another directive to specify a
/// [ListSeparator](../pliron/printable/enum.ListSeparator.html).
///
/// The following directives are supported:
///   1. `NewLine`: takes no argument, and specifies a newline to be used as list separator.
///   2. ``CharNewline(`c`)``: takes a single character argument that will be followed by a newline.
///   3. ``Char(`c`)``: takes a single character argument that will be used as separator.
///   4. ``CharSpace(`c`)``: takes a single character argument that will be followed by a space.
///
/// Generic structs and enums are supported. The macro preserves generic parameters on the
/// generated `Printable` and `Parsable` impls, but it does not synthesize trait bounds.
/// Any generic field that is parsed or printed through the format must therefore carry explicit
/// bounds on the type itself. In practice, this usually means a bound like
/// `T: Printable + Parsable<Arg = (), Parsed = T>`.
///
/// Examples:
/// 1. Derive for a struct, with no format string (default format):
///    (Note that the field u64 has both `Printable` and `Parsable` implemented).
/// ```
/// use pliron::derive::format;
/// #[format]
/// struct IntWrapper {
///    inner: u64,
/// }
/// ```
/// 2. An example with a custom format string:
/// ```
/// use pliron::derive::format;
/// #[format("`BubbleWrap` `[` $inner `]`")]
/// struct IntWrapperCustom {
///   inner: u64,
/// }
/// ```
/// 3. An example for an enum (custom format strings are allowed for the variants only).
/// ```
/// use pliron::derive::format;
/// use pliron::{builtin::types::IntegerType, r#type::TypedHandle};
/// #[format]
/// enum Enum {
///     A(TypedHandle<IntegerType>),
///     B {
///         one: TypedHandle<IntegerType>,
///         two: u64,
///     },
///     C,
///     #[format("`<` $upper `/` $lower `>`")]
///     Op {
///         upper: u64,
///         lower: u64,
///     },
/// }
/// ```
/// 4. An example with `Option` and `Vec` fields
/// ```
/// use pliron::derive::format;
/// #[format("`<` opt($a) `;` vec($b, Char(`,`)) `>`")]
/// struct OptAndVec {
///    a: Option<u64>,
///    b: Vec<u64>,
///}
/// ```
/// 5. An example with a generic field and explicit bounds:
/// ```
/// use pliron::derive::format;
/// use pliron::{parsable::Parsable, printable::Printable};
///
/// #[format]
/// struct Wrapper<T>
/// where
///     T: Printable + Parsable<Arg = (), Parsed = T>,
/// {
///     value: T,
/// }
/// ```
/// 6. An example with an optional field using `label` and `delimiters` in `opt`:
/// ```
/// use pliron::derive::format;
/// #[format("`<` opt($a, label($value), delimiters(`(`, `)`)) `>`")]
/// struct OptionalField {
///    a: Option<u64>,
/// }
/// ```
#[proc_macro_attribute]
pub fn format(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_format::derive(
        args,
        input,
        DeriveIRObject::AnyOtherRustType,
    ))
}

/// Derive [Printable](../pliron/printable/trait.Printable.html) and
/// [Parsable](../pliron/parsable/trait.Parsable.html) for [Op](../pliron/op/trait.Op.html)s
///
/// *Note*: It is suggested to use the [pliron_op] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_op]'s `format` field
///         expands to this macro.
///
/// This derive only supports a syntax in which results appear before the opid:
///   `res1, ... = opid ...`
/// The format string specifies what comes after the opid.
///   1. A named variable `$name` specifies a named attribute of the operation.
///      This cannot be combined with the [attr_dict](#attr_dict) directive.
///   2. An unnamed variable `$i` specifies `operands[i]`, except when inside some directives.
///      This cannot be combined with the "operands" directive.
///   3. The "type" directive specifies that a type must be parsed. It takes one argument,
///      which is an unnamed variable `$i` with `i` specifying `result[i]`. This cannot be
///      combined with the "types" directive.
///   4. The "region" directive specifies that a region must be parsed. It takes one argument,
///      which is an unnamed variable `$i` with `i` specifying `region[i]`. This cannot be
///      combined with the "regions" directive.
///   5. The <a name="attr"></a> "attr" directive can be used to specify attribute on an `Op` when
///      the attribute's rust type is fixed at compile time. It takes two mandatory and two optional
///      arguments.
///
///      1. The first operand is a named variable `$name` which is used as a key into the
///         operation's attribute dictionary
///      2. The second is the concrete rust type of the attribute. This second argument can be a
///         named variable `$name` (with `name` being in scope) or a literal string denoting the path
///         to a rust type (e.g. `` `::pliron::builtin::attributes::IntegerAttr` ``).
///      3. Two additional optional arguments can be specified:
///         * The "label" directive, with one argument, a named variable `$label`, which
///           specifies the label to be used while printing / parsing the attribute.
///         * The "delimiters" directive, which takes two literal arguments,
///           specifying the opening and closing delimiters to be used while printing / parsing.
///
///      The advantage over specifying an attribute using the [attr](#attr) directive (as against
///      just using a named variable) is that the attribute-id is not a part of the syntax
///      here (because the type is statically known, allowing us to be able to parse it),
///      thus allowing it to be more succinct. This cannot be combined with the [attr_dict](#attr_dict)
///      directive.
///   6. The "succ" directive specifies an operation's successor. It takes one argument,
///      which is an unnamed variable `$i` with `i` specifying `successor[i]`.
///   7. The "operands" directive specifies all the operands of an operation. It takes one argument
///      which is a directive specifying the separator between operands. This cannot be combined
///      with using unnamed variables `$i` to refer to operands.
///      The following directives are supported:
///        1. `NewLine`: takes no argument, and specifies a newline to be used as list separator.
///        2. ``CharNewline(`c`)``: takes a single character argument that will be followed by a newline.
///        3. ``Char(`c`)``: takes a single character argument that will be used as separator.
///        4. ``CharSpace(`c`)``: takes a single character argument that will be followed by a space.
///   8. The "successors" directive specifies all the successors of an operation. It takes one argument
///      which is a directive specifying the separator between successors. The separator directive is
///      same as that for "operands" above. This cannot be combined with the "succ" directive.
///   9. The "regions" directive specifies all the regions of an operation. It takes one argument
///      which is a directive specifying the separator between regions. The separator directive is same
///      as that for "operands" above. This cannot be combined with the "region" directive.
///  10. The <a name="attr_dict"></a> "attr_dict" directive specifies an
///      [AttributeDict](../pliron/attribute/struct.AttributeDict.html).
///      It cannot be combined with any of [attr](#attr), [opt_attr](#opt_attr) directives or
///      a named variable (`$name`).
///  11. The "types" directive specifies all the result types of an operation. It takes one argument
///      which is a directive specifying the separator between result types. The separator directive is
///      same as that for "operands" above. This cannot be combined with the "type" directive.
///  12. The "typesig" directive prints the full type signature of an operation as
///      `(operand_types) -> (result_types)`. It takes no arguments. When parsing, the operand
///      types are consumed and ignored; only the result types are used to build the operation.
///      This cannot be combined with the "type" or "types" directives.
///  13. The <a name="opt_attr"></a> "opt_attr" directive specifies an optional attribute on an `Op`.
///      It takes two or more arguments, which are same as those of the [attr](#attr) directive.
///      This cannot be combined with the [attr_dict](#attr_dict) directive.
///
/// Examples:
/// 1. Derive for a struct, with no format string (default format):
/// ```
/// use pliron::derive::{def_op, format_op, verify_succ};
/// #[verify_succ]
/// #[format_op]
/// #[def_op("test.myop")]
/// struct MyOp;
/// ```
/// 2. An example with a custom format string:
/// ```
/// use pliron::derive::{def_op, derive_op_interface_impl, format_op, verify_succ};
/// use pliron::{op::Op, builtin::op_interfaces::
///     {OneOpdInterface, OneResultInterface, NOpdsInterface, NResultsInterface}};
/// #[verify_succ]
/// #[format_op("$0 `<` $attr `>` `:` type($0)")]
/// #[def_op("test.one_result_one_operand")]
/// #[derive_op_interface_impl
///     (OneOpdInterface, OneResultInterface, NOpdsInterface<1>, NResultsInterface<1>)]
/// struct OneResultOneOperandOp;
/// ```
/// More examples can be seen in the tests for this macro in `pliron-derive/tests/format_op.rs`.
#[proc_macro_attribute]
pub fn format_op(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_format::derive(args, input, DeriveIRObject::Op))
}

/// Derive [Printable](../pliron/printable/trait.Printable.html) and
/// [Parsable](../pliron/parsable/trait.Parsable.html) for
/// [Attribute](../pliron/attribute/trait.Attribute.html)s
///
/// *Note*: It is suggested to use the [pliron_attr] macro instead of using this macro directly.
///
/// Refer to [macro@format] for the syntax specification and examples.
#[proc_macro_attribute]
pub fn format_attribute(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_format::derive(
        args,
        input,
        DeriveIRObject::Attribute,
    ))
}

/// Derive [Printable](../pliron/printable/trait.Printable.html) and
/// [Parsable](../pliron/parsable/trait.Parsable.html) for
/// [Type](../pliron/type/trait.Type.html)s
///
/// *Note*: It is suggested to use the [pliron_type] macro instead of using this macro directly.
///
/// Refer to [macro@format] for the syntax specification and examples.
#[proc_macro_attribute]
pub fn format_type(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_format::derive(args, input, DeriveIRObject::Type))
}

pub(crate) fn to_token_stream(res: syn::Result<proc_macro2::TokenStream>) -> TokenStream {
    let tokens = match res {
        Ok(tokens) => tokens,
        Err(error) => {
            let error = error.to_compile_error();
            quote::quote!(
                #error
            )
        }
    };
    TokenStream::from(tokens)
}

/// Declare an [Op](../pliron/op/trait.Op.html) interface, which can be implemented
/// by any `Op`.
///
/// If the interface requires any other interface to be already implemented,
/// they can be specified super-traits.
///
/// When an `Op` is verified, its interfaces are also automatically verified,
/// with guarantee that a super-interface is verified before an interface itself is.
///
/// Example: Here `SameOperandsAndResultType` and `SymbolOpInterface` are super interfaces
/// for the new interface `MyOpIntr`.
/// ```
/// # use pliron::builtin::op_interfaces::{SameOperandsAndResultType, SymbolOpInterface};
/// # use pliron::derive::{op_interface};
/// # use pliron::{op::Op, context::Context, result::Result};
///   /// MyOpIntr is my first op interface.
///   #[op_interface]
///   trait MyOpIntr: SameOperandsAndResultType + SymbolOpInterface {
///       fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
///       where Self: Sized,
///       {
///           Ok(())
///       }
///   }
/// ```
#[proc_macro_attribute]
pub fn op_interface(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let supertrait = parse_quote! { ::pliron::op::Op };
    let verifier_type = parse_quote! { ::pliron::op::OpInterfaceVerifier };
    let target_marker_trait = parse_quote! { ::pliron::op::OpInterfaceMarker };

    to_token_stream(interfaces::interface_define(
        item,
        supertrait,
        verifier_type,
        true,
        target_marker_trait,
    ))
}

/// Implement [Op](../pliron/op/trait.Op.html) Interface for an Op. The interface trait must define
/// a `verify` function with type [OpInterfaceVerifier](../pliron/op/type.OpInterfaceVerifier.html)
///
/// Usage:
/// ```
/// # use pliron::derive::{def_op, format_op, op_interface, op_interface_impl, verify_succ};
///
/// #[verify_succ]
/// #[def_op("dialect.name")]
/// #[format_op]
/// struct MyOp;
///
/// #[op_interface]
/// pub trait MyOpInterface {
///     fn gubbi(&self);
///     fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
///     where Self: Sized,
///     {
///         Ok(())
///     }
/// }
///
/// #[op_interface_impl]
/// impl MyOpInterface for MyOp {
///     fn gubbi(&self) { println!("gubbi"); }
/// }
/// # use pliron::{
/// #     op::Op, context::Context, result::Result, common_traits::Verify
/// # };
/// ```
#[proc_macro_attribute]
pub fn op_interface_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let interface_verifiers_slice = parse_quote! { ::pliron::op::OP_INTERFACE_VERIFIERS };
    let all_verifiers_fn_type = parse_quote! { ::pliron::op::OpInterfaceAllVerifiers };
    to_token_stream(interfaces::interface_impl(
        item,
        interface_verifiers_slice,
        all_verifiers_fn_type,
    ))
}

/// `#[pliron_type(...)]`: Unified macro for defining IR types.
///
/// This macro provides a simplified, unified syntax for defining IR types by expanding
/// into the existing type definition macros. It supports the following configuration options:
///
/// - `name = "dialect.type_name"`: The fully qualified name of the type (required).\
///   Expands to [def_type].
/// - `format = "format_string"`: Custom format string for printing/parsing (optional).\
///   Expands to [format_type].
/// - `verifier = "succ"`: Verifier implementation, currently only "succ" is supported (optional).\
///   Expands to [macro@verify_succ].
/// - `generate_get = true/false`: Whether to generate a get method for the type (optional, default: false).\
///   Expands to [derive_type_get].
///
/// ## Examples
///
/// ### Basic type definition:
/// ```
/// use pliron::derive::pliron_type;
///
/// #[pliron_type(name = "test.unit_type", format, verifier = "succ")]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// pub struct UnitType;
/// ```
///
/// ### Type with custom format:
/// ```
/// use pliron::derive::pliron_type;
///
/// #[pliron_type(
///     name = "test.flags_type",
///     format = "`type` `{` $flags `}`",
///     verifier = "succ"
/// )]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// struct FlagsType {
///     flags: u32,
/// }
/// ```
///
/// ### Type with get method generation:
/// ```
/// use pliron::derive::pliron_type;
///
/// #[pliron_type(
///     name = "test.vector_type",
///     generate_get = true,
///     format,
///     verifier = "succ"
/// )]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// struct VectorType {
///     elem_ty: u32,
///     num_elems: u32,
/// }
/// ```
#[proc_macro_attribute]
pub fn pliron_type(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_entity::pliron_type(args, input))
}

/// `#[pliron_attr(...)]`: Unified macro for defining IR attributes.
///
/// This macro provides a simplified, unified syntax for defining IR attributes by expanding
/// into the existing attribute definition macros. It supports the following configuration options:
///
/// - `name = "dialect.attribute_name"`: The fully qualified name of the attribute (required).\
///   Expands to [def_attribute].
/// - `format = "format_string"`: Custom format string for printing/parsing (optional).\
///   Expands to [format_attribute].
/// - `verifier = "succ"`: Verifier implementation, currently only "succ" is supported (optional).\
///   Expands to [macro@verify_succ].
///
/// ## Examples
///
/// ### Basic attribute definition:
/// ```
/// use pliron::derive::pliron_attr;
///
/// #[pliron_attr(name = "test.string_attr", format, verifier = "succ")]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// struct StringAttr {
///     value: String,
/// }
/// ```
///
/// ### Attribute with custom format:
/// ```
/// use pliron::derive::pliron_attr;
///
/// #[pliron_attr(
///     name = "test.string_attr",
///     format = "`attr` `(` $value `)`",
///     verifier = "succ"
/// )]
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// struct StringAttr {
///     value: String,
/// }
/// ```
#[proc_macro_attribute]
pub fn pliron_attr(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_entity::pliron_attr(args, input))
}

/// `#[pliron_op(...)]`: Unified macro for defining IR operations.
///
/// This macro provides a simplified, unified syntax for defining IR operations by expanding
/// into the existing operation definition macros. It supports the following configuration options:
///
/// - `name = "dialect.op_name"`: The fully qualified name of the operation (required).\
///   Expands to [def_op].
/// - `format = "format_string"`: Custom format string for printing/parsing (optional).\
///   Expands to [format_op].
/// - `interfaces = [Interface1, Interface2, ...]`: List of interfaces to implement (optional).\
///   Expands to [derive_op_interface_impl].
/// - `attributes = (attr_name: AttrType, ...)`: List of attributes with their types (optional).\
///   Expands to [derive_attr_get_set], generating getter and setter methods.
/// - `operands = (name, name: Type, _, _: Type, ...)`: List of operand specs (optional).\
///   Expands to [operands].
/// - `results = (name, name: Type, _, _: Type, ...)`: List of result specs (optional).\
///   Expands to [results].
/// - `verifier = "succ"`: Verifier implementation, currently only "succ" is supported (optional).\
///   Expands to [macro@verify_succ].
///
/// ## Examples
///
/// ### Basic operation definition:
/// ```
/// use pliron::derive::pliron_op;
///
/// #[pliron_op(name = "test.my_op", format, verifier = "succ")]
/// struct MyOp;
/// ```
///
/// ### Operation with custom format and interfaces:
/// ```
/// use pliron::derive::pliron_op;
/// use pliron::builtin::op_interfaces::NRegionsInterface;
///
/// #[pliron_op(
///     name = "test.if_op",
///     format = "`(`$0`)` region($0)",
///     interfaces = [ NRegionsInterface<1> ],
///     verifier = "succ"
/// )]
/// struct IfOp;
/// ```
///
/// ### Operation with specified attributes:
/// ```
/// use pliron::derive::pliron_op;
/// use pliron::builtin::attributes::{UnitAttr, IntegerAttr};
///
/// #[pliron_op(
///     name = "dialect.test",
///     format,
///     attributes = (attr1: UnitAttr, attr2: IntegerAttr),
///     verifier = "succ"
/// )]
/// struct CallOp;
/// ```
///
/// ### Operation with specified operands:
/// ```
/// use pliron::derive::pliron_op;
/// use pliron::builtin::types::{IntegerType, UnitType};
///
/// #[pliron_op(
///     name = "dialect.with_operands",
///     format,
///     operands = (lhs: IntegerType, _, rhs, _: UnitType),
///     verifier = "succ"
/// )]
/// struct WithOperandsOp;
/// ```
///
/// ### Operation with specified results:
/// ```
/// use pliron::derive::pliron_op;
/// use pliron::builtin::types::{IntegerType, UnitType};
///
/// #[pliron_op(
///     name = "dialect.with_results",
///     format,
///     results = (out: IntegerType, _: UnitType),
///     verifier = "succ"
/// )]
/// struct WithResultsOp;
/// ```
#[proc_macro_attribute]
pub fn pliron_op(args: TokenStream, input: TokenStream) -> TokenStream {
    to_token_stream(derive_entity::pliron_op(args, input))
}

/// Derive implementation of an [Op](../pliron/op/trait.Op.html) Interface for an Op.
/// Note that an impl can be derived only for those interfaces that do not require any
/// methods to be defined during the impl.
///
/// *Note*: It is suggested to use the [pliron_op] macro instead of using this macro directly.
///         The documention here is useful though, because [pliron_op]'s `interfaces` field
///         expands to this macro.
///
/// Usage:
/// ```
/// # use pliron::derive::{derive_op_interface_impl, format_op, op_interface, verify_succ};
///
/// #[verify_succ]
/// #[def_op("dialect.name")]
/// #[format_op]
/// #[derive_op_interface_impl(MyOpInterface)]
/// struct MyOp;
///
/// #[op_interface]
/// pub trait MyOpInterface {
///     fn gubbi(&self) { println!("gubbi"); }
///     fn verify(op: &dyn Op, ctx: &Context) -> Result<()>
///     where Self: Sized,
///     {
///         Ok(())
///     }
/// }
/// # use pliron::derive::def_op;
/// # use pliron::{
/// #     op::Op, context::Context, result::Result,
/// #     common_traits::Verify
/// # };
/// ```
#[proc_macro_attribute]
pub fn derive_op_interface_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    to_token_stream(interfaces::derive_op_interface_impl(attr, item))
}

/// Declare an [Attribute](../pliron/attribute/trait.Attribute.html) interface,
/// which can be implemented by any `Attribute`.
///
/// If the interface requires any other interface to be already implemented,
/// they can be specified super-traits.
///
/// When an `Attribute` is verified, its interfaces are also automatically verified,
/// with guarantee that a super-interface is verified before an interface itself is.
///
/// Example: Here `Super1` and `Super2` are super interfaces for the interface `MyAttrIntr`.
/// ```
/// # use pliron::{attribute::Attribute, context::Context, result::Result};
/// use pliron::derive::attr_interface;
///
/// #[attr_interface]
/// trait Super1 {
///     fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
///     where
///         Self: Sized,
///     {
///         Ok(())
///     }
/// }
///
/// #[attr_interface]
/// trait Super2 {
///     fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
///     where
///         Self: Sized,
///     {
///         Ok(())
///     }
/// }
///
/// // MyAttrIntr is my best attribute interface.
/// #[attr_interface]
/// trait MyAttrIntr: Super1 + Super2 {
///     fn verify(_attr: &dyn Attribute, _ctx: &Context) -> Result<()>
///     where
///         Self: Sized,
///     {
///         Ok(())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn attr_interface(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let supertrait = parse_quote! { ::pliron::attribute::Attribute };
    let verifier_type = parse_quote! { ::pliron::attribute::AttrInterfaceVerifier };
    let target_marker_trait = parse_quote! { ::pliron::attribute::AttrInterfaceMarker };

    to_token_stream(interfaces::interface_define(
        item,
        supertrait,
        verifier_type,
        true,
        target_marker_trait,
    ))
}

/// Implement [Attribute](../pliron/attribute/trait.Attribute.html) Interface for an Attribute.
/// The interface trait must define a `verify` function with type
/// [AttrInterfaceVerifier](../pliron/attribute/type.AttrInterfaceVerifier.html).
///
/// Usage:
/// ```
/// use pliron::derive::{attr_interface, attr_interface_impl, def_attribute, format_attribute, verify_succ};
///
/// #[verify_succ]
/// #[def_attribute("dialect.name")]
/// #[format_attribute]
/// #[derive(PartialEq, Eq, Clone, Debug, Hash)]
/// struct MyAttr { }
///
///     /// My first attribute interface.
/// #[attr_interface]
/// trait MyAttrInterface {
///     fn monu(&self);
///     fn verify(attr: &dyn Attribute, ctx: &Context) -> Result<()>
///     where Self: Sized,
///     {
///          Ok(())
///     }
/// }
///
/// #[attr_interface_impl]
/// impl MyAttrInterface for MyAttr
/// {
///     fn monu(&self) { println!("monu"); }
/// }
/// # use pliron::{
/// #     printable::{self, Printable},
/// #     context::Context, result::Result, common_traits::Verify,
/// #     attribute::Attribute
/// # };
#[proc_macro_attribute]
pub fn attr_interface_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let interface_verifiers_slice = parse_quote! { ::pliron::attribute::ATTR_INTERFACE_VERIFIERS };
    let all_verifiers_fn_type = parse_quote! { ::pliron::attribute::AttrInterfaceAllVerifiers };
    to_token_stream(interfaces::interface_impl(
        item,
        interface_verifiers_slice,
        all_verifiers_fn_type,
    ))
}

/// Declare a [Type](../pliron/type/trait.Type.html) interface,
/// which can be implemented by any `Type`.
///
/// If the interface requires any other interface to be already implemented,
/// they can be specified super-traits.
///
/// When an `Attribute` is verified, its interfaces are also automatically verified,
/// with guarantee that a super-interface is verified before an interface itself is.
///
/// Example: Here `Super1` and `Super2` are super interfaces for the interface `MyTypeIntr`.
/// ```
/// use pliron::derive::type_interface;
/// # use pliron::{r#type::Type, context::Context, result::Result};
/// #[type_interface]
/// trait Super1 {
///     fn verify(_type: &dyn Type, _ctx: &Context) -> Result<()>
///     where
///         Self: Sized,
///     {
///         Ok(())
///     }
/// }
///
/// #[type_interface]
/// trait Super2 {
///     fn verify(_type: &dyn Type, _ctx: &Context) -> Result<()>
///     where
///         Self: Sized,
///     {
///         Ok(())
///     }
/// }
///
/// #[type_interface]
/// // MyTypeIntr is my best type interface.
/// trait MyTypeIntr: Super1 + Super2 {
///     fn verify(_type: &dyn Type, _ctx: &Context) -> Result<()>
///     where
///         Self: Sized,
///     {
///         Ok(())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn type_interface(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let supertrait = parse_quote! { ::pliron::r#type::Type };
    let verifier_type = parse_quote! { ::pliron::r#type::TypeInterfaceVerifier };
    let target_marker_trait = parse_quote! { ::pliron::r#type::TypeInterfaceMarker };

    to_token_stream(interfaces::interface_define(
        item,
        supertrait,
        verifier_type,
        false,
        target_marker_trait,
    ))
}

/// Implement [Type](../pliron/type/trait.Type.html) Interface for a Type.
/// The interface trait must define a `verify` function with type
/// [TypeInterfaceVerifier](../pliron/type/type.TypeInterfaceVerifier.html).
///
/// Usage:
/// ```
/// use pliron::derive::{def_type, format_type, type_interface, type_interface_impl, verify_succ};
///
/// #[verify_succ]
/// #[def_type("dialect.name")]
/// #[format_type]
/// #[derive(PartialEq, Eq, Clone, Debug, Hash)]
/// struct MyType { }
///
/// #[type_interface]
/// /// My first type interface.
/// trait MyTypeInterface {
///     fn monu(&self);
///     fn verify(r#type: &dyn Type, ctx: &Context) -> Result<()>
///     where Self: Sized,
///     {
///          Ok(())
///     }
/// }
///
/// #[type_interface_impl]
/// impl MyTypeInterface for MyType
/// {
///     fn monu(&self) { println!("monu"); }
/// }
/// # use pliron::{
/// #     printable::{self, Printable},
/// #     context::Context, result::Result, common_traits::Verify,
/// #     r#type::Type
/// # };
#[proc_macro_attribute]
pub fn type_interface_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let interface_verifiers_slice = parse_quote! { ::pliron::r#type::TYPE_INTERFACE_VERIFIERS };
    let all_verifiers_fn_type = parse_quote! { ::pliron::r#type::TypeInterfaceAllVerifiers };
    to_token_stream(interfaces::interface_impl(
        item,
        interface_verifiers_slice,
        all_verifiers_fn_type,
    ))
}
