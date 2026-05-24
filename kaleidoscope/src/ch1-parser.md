# Chapter 1: Parsing Kaleidoscope

This chapter implements a complete `combine`-based parser for the Kaleidoscope
language used throughout the tutorial.

You may skip this chapter if you already have an AST (or know how to build one)
and want to use `pliron` to build a compiler pipeline on top of it. The rest of
the tutorial only requires the AST and not the parser implementation.

The implementation for this chapter lives in `examples/kaleidoscope/ast.rs`

## What is combine?

[combine](https://github.com/Marwes/combine) is a parser combinator library.

"Parser combinators are higher-order functions that accept parsers as input and return a new parser as output." -- Wikipedia.

They allow for the construction of complex parsers by combining simpler ones.

As an example, a parser for an integer literal can be combined with a parser for a plus sign and another integer literal to create a parser for an addition expression.

## Why `combine`?

We use `combine` in this tutorial because `pliron` itself already uses `combine`
for most IR parsing tasks. Learning it here makes it easier to read and extend
parser-related code in the `pliron` codebase.

At the same time, `combine` is not a requirement for your own compiler front
end. If you are already productive with another parser library or approach,
you can use that instead. The rest of the tutorial only requires an AST.

This flexibility is especially useful because `combine` may have a steeper learning
curve than some alternatives.

## The language

Kaleidoscope as used here is a simple integer language.

Example: Fibonacci program in Kaleidoscope:

```text
{{#include ../../examples/kaleidoscope/fibonacci.kal}}
```

Example: Factorial program in Kaleidoscope:

```text
{{#include ../../examples/kaleidoscope/factorial.kal}}
```

Supported constructs:

| Construct | Syntax |
|-----------|--------|
| Integer literal | `42` |
| Arithmetic | `+`, `-`, `*` |
| Comparison | `<`, `>`, `==`, `!=`, `<=`, `>=` |
| Variable | `name` |
| Function call | `name(args...)` |
| If statement | `if cond { stmts } else { stmts }` |
| Variable declaration | `var name;` or `var name = expr;` |
| Assignment | `name = expr;` |
| Return | `return expr;` |
| While loop | `while cond { stmts }` |
| Function definition | `def name(params) { stmts }` |

`if` and `while` are **statements**, so neither directly produces a value.

## Grammar

```text
program  := func_def* eof
func_def := 'def' ident '(' params ')' block
params   := (ident (',' ident)*)?
block    := '{' stmt* '}'
stmt     := 'var' ident ('=' expr)? ';'
           | ident '=' expr ';'
           | 'return' expr ';'
           | 'while' expr block
           | 'if' expr block 'else' block
           | expr ';'
expr     := cmp_expr
cmp_expr := add_expr (cmp_op add_expr)?
add_expr := mul_expr (('+' | '-') mul_expr)*
mul_expr := primary ('*' primary)*
primary  := integer
           | ident '(' (expr (',' expr)*)? ')'
           | ident
           | '(' expr ')'
cmp_op   := '<' | '>' | '==' | '!=' | '<=' | '>='
integer  := [0-9]+
ident    := [a-zA-Z_][a-zA-Z0-9_]*
```

## Parsing into an AST

Before looking at any parsing code, it helps to know what we are building
towards.  The parser's sole job is to turn source text into this tree of Rust
types:

### Expressions
Expressions always produce a value. All values are implicitly 64-bit
integers.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:ast_expr}}
```

### Statements
Statements do not produce a value.  `If` and `While` are the structured
statement forms.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:ast_stmt}}
```

### Functions
A function is just a name, a list of parameter names, and a list of
statements.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:ast_function}}
```

*Note*:
- The AST intentionally does not include types, all values are implicitly
64-bit integers.
- `Expr` and `Stmt` are recursive
  - `BinOp` and `Call` contain nested expression nodes,
  - `If` and `While` statements recursively contain statement blocks.
- This shapes how the parser must be structured (see below).

## Lexical building blocks

Before we dive into the small parsers, here is how to read their signature:

```rust
fn my_parser<Input>() -> impl Parser<Input, Output = T> where Input: Stream<Token = char>, ...
```

Here, `Input` is the input stream type (here, a stream of `char`s), `Output = T`
is the AST/token value the parser produces, and `impl Parser<...>` means
"some concrete parser type" chosen by `combine` and inferred by Rust. The long
`where` clause is mostly plumbing so the parser works with any compatible
character stream; for learning, the key part is just:

  > Calling `my_parser` returns a parser that parses characters and returns a `T` on success.

### Skipping Whitespaces

`combine` provides the `spaces()` parser, which matches zero or more whitespace characters.
We wrap it once so that every token parser can end with `.skip(ws())` and consume trailing
spaces and newlines automatically.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:ws_helper}}
```

### Parsing Identifiers

An identifier starts with a letter or `_`, followed by any number of letters,
digits, or `_`.  combine's `letter()`, `alpha_num()`, and `char('_')` match
single characters; `.or()` tries alternatives; `many()` collects zero or more
into a `String`.  The first character and the rest are sequenced with a tuple
and joined by `.map()`.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:ident_parser}}
```

### Keywords

A plain `string("if")` would match the prefix of `iffy`, leaving `fy` in the
stream — wrong.  `not_followed_by(alpha_num().or(char('_')))` asserts that the
matched text is *not* followed by an identifier character.  `attempt()` wraps
the whole thing so that on failure the cursor is rewound; without it, a partial
match would leave the stream half-consumed.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:keyword_parser}}
```

## Parsing expressions

Expressions are parsed in layers from highest to lowest precedence:
`primary` -> `mul_expr` -> `add_expr` -> `cmp_expr`.  Each layer calls the one
below it, so precedence emerges naturally.

### Primary expressions

A primary is the highest-precedence building block.  `choice!()` tries each
alternative in order and returns the first that succeeds.

A function call and a plain variable both start with an identifier, so the
call branch is wrapped in `attempt()` to allow backtracking when no `(` follows.
`between(tok('('), tok(')'), sep_by(...))` handles the argument list cleanly.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:primary_parser}}
```

### Arithmetic expressions and operator precedence

`mul_expr_()` handles `*` (higher precedence) and `add_expr_()` handles `+`
and `-` on top of it.  Both follow the same fold pattern:

1. Parse the leftmost operand.
2. Collect zero or more `(operator, operand)` pairs with `many()`.
3. Fold left-to-right into nested `Expr::BinOp` nodes, so `1 + 2 + 3`
   becomes `BinOp(Add, BinOp(Add, 1, 2), 3)`.

Multiplication is parsed in its own layer first:

```rust
{{#include ../../examples/kaleidoscope/ast.rs:mul_expr_parser}}
```

and addition/subtraction on top of it:

```rust
{{#include ../../examples/kaleidoscope/ast.rs:add_expr_parser}}
```

### Handling recursion in combine

The grammar is recursive: `expr` -> `primary` -> `if expr` or `(expr)`.
combine's `impl Parser` return type does not allow self-reference, so a named
function pointer breaks the type cycle:

```rust
{{#include ../../examples/kaleidoscope/ast.rs:expr_fn}}
```

`expr_fn` calls `cmp_expr_()` specifically because `cmp_expr` is the top
expression layer in the grammar (`expr := cmp_expr`).  That makes
`expr_fn` the single "parse any expression" entry point while still preserving
the full precedence stack (`cmp` on top of `add` on top of `mul` on top of
`primary`).

Because `fn(&mut Input) -> StdParseResult<O, Input>` implements `Parser<Input>`
in `combine`, `expr_fn::<Input>` can be used directly wherever a parser is
expected.  The concrete function-pointer type is what breaks the infinite type
recursion that chained `impl Parser` returns would otherwise cause.

## Parsing statements

Statements are what a function body contains.  The `while` parser shows a
typical pattern: parse a keyword, parse an expression for the condition, then
parse a brace-delimited block of nested statements.  The nested statements
require the same `stmt_fn` function-pointer trick used for expressions above.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:while_stmt_parser}}
```

The `if` statement parser follows the same block-structured pattern, with both
branches containing nested statements:

```rust
{{#include ../../examples/kaleidoscope/ast.rs:if_stmt_parser}}
```

The other statement kinds (`var` declaration, assignment, `return`,
expression-statement) follow the
same shape.  They are all wired together in a single dispatcher:

```rust
{{#include ../../examples/kaleidoscope/ast.rs:stmt_parser}}
```

## Parsing functions and the full program

A function definition is a `def` keyword followed by a name, a parenthesised
parameter list, and a block.  `sep_by(ident_(), tok(','))` collects the
comma-separated parameter names into a `Vec<String>`.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:func_def_parser}}
```

A program is just zero or more function definitions followed by end-of-file.
`ws().with(...)` skips any leading whitespace before the first definition.

```rust
{{#include ../../examples/kaleidoscope/ast.rs:program_parser}}
```

`parse_program` is the public entry point that drives the parser and converts
combine's error type into a plain `String`:

```rust
{{#include ../../examples/kaleidoscope/ast.rs:parse_program}}
```

## Try it out

```sh
cargo test --example kaleidoscope -- test_ast_fibonacci --show-output
cargo test --example kaleidoscope -- test_ast_factorial --show-output
```

## Next step

Chapter 2 defines the Kaleidoscope dialect operation set in a reusable module
and demonstrates constructing those ops.
