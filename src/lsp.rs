//! Parse instrumentation for language tooling (pliron-lsp).
//!
//! When enabled through [`parse_recorded`], the parser records the source
//! span of every entity it parses (operations, op names, results, operand
//! uses, successors, block labels and arguments, types, attributes,
//! attribute keys, format keywords and regions) and can optionally recover
//! from errors: a failing operation is recorded, the input is skipped to the
//! next `;`, block header or closing `}`, and parsing continues.
//!
//! None of this is active during normal parsing: the recorder lives in the
//! parser's name tracker and is `None` unless [`parse_recorded`] installs it.

use alloc::{boxed::Box, format, string::String, string::ToString, vec::Vec};

use crate::{
    basic_block::BasicBlock,
    builtin::{op_interfaces::OneResultInterface, ops::ForwardRefOp},
    combine::{
        Parser, Positioned, StreamOnce,
        stream::{ResetStream, position::SourcePosition},
    },
    context::{Context, Ptr},
    identifier::Identifier,
    irfmt::parsers::spaced,
    location::{Location, Source},
    operation::Operation,
    parsable::{ParseResult, State, StateStream, state_stream_from_iterator},
    region::Region,
    result::Result,
    r#type::TypeHandle,
    value::Value,
};

/// Options for [`parse_recorded`].
#[derive(Clone, Copy, Debug)]
pub struct RecordOptions {
    /// Recover from errors inside blocks and regions.
    pub recover: bool,
    /// Do not overwrite parsed locations with locations recorded in the
    /// `outlined_attributes:` section (keeps positions pointing into the
    /// text being parsed).
    pub keep_parsed_locations: bool,
}

impl Default for RecordOptions {
    fn default() -> Self {
        RecordOptions {
            recover: true,
            keep_parsed_locations: true,
        }
    }
}

/// A recorded parse event. Positions are `combine` source positions
/// (1-based line, 1-based column counted in chars). `end` positions are
/// exclusive.
#[derive(Clone, Debug)]
pub enum Event {
    /// An operation was parsed successfully.
    Op {
        op: Ptr<Operation>,
        start: SourcePosition,
        opid_start: SourcePosition,
        opid_end: SourcePosition,
        end: SourcePosition,
    },
    /// An operation failed to parse; recovery skipped to `end`.
    OpError {
        start: SourcePosition,
        end: SourcePosition,
    },
    /// Definition of an op result.
    ResultDef {
        value: Value,
        start: SourcePosition,
        end: SourcePosition,
    },
    /// Use of an SSA value as an operand.
    OperandUse {
        value: Value,
        start: SourcePosition,
        end: SourcePosition,
    },
    /// Use of a block as a successor (`^label`).
    SuccessorUse {
        block: Ptr<BasicBlock>,
        start: SourcePosition,
        end: SourcePosition,
    },
    /// Definition of a block label (`^label` in a block header).
    BlockLabel {
        block: Ptr<BasicBlock>,
        start: SourcePosition,
        end: SourcePosition,
    },
    /// Definition of a block argument.
    BlockArgDef {
        value: Value,
        start: SourcePosition,
        end: SourcePosition,
    },
    /// A type (`dialect.type` and its contents).
    Type {
        ty: TypeHandle,
        start: SourcePosition,
        id_end: SourcePosition,
        end: SourcePosition,
    },
    /// An attribute (`dialect.attr` and its contents).
    Attr {
        start: SourcePosition,
        id_end: SourcePosition,
        end: SourcePosition,
    },
    /// A key in an attribute dictionary.
    AttrKey {
        start: SourcePosition,
        end: SourcePosition,
    },
    /// A literal keyword of a declarative (derived) format.
    Keyword {
        start: SourcePosition,
        end: SourcePosition,
    },
    /// A token marked by a hand-written parser with [`token`]; `kind` is a
    /// semantic token type name (`"keyword"`, `"number"`, ...).
    Token {
        kind: &'static str,
        start: SourcePosition,
        end: SourcePosition,
    },
    /// A region (`{ ... }`).
    Region {
        region: Option<Ptr<Region>>,
        open: SourcePosition,
        close: Option<SourcePosition>,
    },
}

/// An error recorded during recovering parsing.
#[derive(Clone, Debug)]
pub struct RecordedError {
    pub pos: SourcePosition,
    pub message: String,
}

/// Everything recorded during [`parse_recorded`].
#[derive(Default, Debug)]
pub struct Recording {
    pub events: Vec<Event>,
    pub errors: Vec<RecordedError>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) events: Vec<Event>,
    pub(crate) errors: Vec<RecordedError>,
    pub(crate) recover: bool,
    pub(crate) keep_parsed_locations: bool,
    /// Result names of the most recent operation that failed to parse (set
    /// by `Operation::parse`, consumed by recovery to define placeholders).
    pub(crate) failed_results: Option<Vec<(Identifier, Location)>>,
    /// Values that stand in for results of operations that failed to parse.
    pub(crate) placeholders: Vec<Value>,
    /// The input text, to answer "is this position at the start of a line?"
    /// independently of how much whitespace parsers consumed.
    pub(crate) lines: Vec<Vec<char>>,
}

impl Recorder {
    pub(crate) fn push(&mut self, ev: Event) {
        self.events.push(ev);
    }

    /// A forward reference was resolved: retarget recorded uses.
    pub(crate) fn forward_value_resolved(&mut self, fref: Value, def: Value) {
        for ev in &mut self.events {
            if let Event::OperandUse { value, .. } = ev
                && *value == fref
            {
                *value = def;
            }
        }
    }

    /// A forward block reference was resolved: retarget recorded uses.
    pub(crate) fn forward_block_resolved(&mut self, fref: Ptr<BasicBlock>, def: Ptr<BasicBlock>) {
        for ev in &mut self.events {
            if let Event::SuccessorUse { block, .. } = ev
                && *block == fref
            {
                *block = def;
            }
        }
    }

    /// Positions where `value` is used (for diagnostics).
    pub(crate) fn uses_of_value(&self, v: Value) -> Vec<SourcePosition> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::OperandUse { value, start, .. } if *value == v => Some(*start),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn uses_of_block(&self, b: Ptr<BasicBlock>) -> Vec<SourcePosition> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::SuccessorUse { block, start, .. } if *block == b => Some(*start),
                _ => None,
            })
            .collect()
    }
}

/// The active recorder, if any.
pub(crate) fn recorder<'s>(state: &'s mut State<'_>) -> Option<&'s mut Recorder> {
    state.name_tracker.recorder.as_deref_mut()
}

/// Record an event if recording is active.
pub(crate) fn record(state: &mut State<'_>, ev: impl FnOnce() -> Event) {
    if let Some(r) = recorder(state) {
        r.push(ev());
    }
}

/// Is recovering parsing active?
pub(crate) fn recovering(state: &State<'_>) -> bool {
    state
        .name_tracker
        .recorder
        .as_ref()
        .is_some_and(|r| r.recover)
}

/// Should parsed locations be kept (not overwritten by outlined ones)?
pub(crate) fn keep_parsed_locations(state: &State<'_>) -> bool {
    state
        .name_tracker
        .recorder
        .as_ref()
        .is_some_and(|r| r.keep_parsed_locations)
}

/// Parse `input` as a top-level operation while recording parse events.
///
/// Returns the parse result (in recovering mode this is `Ok` as long as the
/// top-level operation itself could be parsed) and the recording.
pub fn parse_recorded(
    ctx: &mut Context,
    input: &str,
    opts: RecordOptions,
) -> (Result<Ptr<Operation>>, Recording) {
    let mut state = State::new(ctx, Source::InMemory);
    state.name_tracker.recorder = Some(Box::new(Recorder {
        recover: opts.recover,
        keep_parsed_locations: opts.keep_parsed_locations,
        lines: input.split('\n').map(|l| l.chars().collect()).collect(),
        ..Recorder::default()
    }));
    let mut stream = state_stream_from_iterator(input.chars(), state);
    let res = spaced(Operation::top_level_parser())
        .parse_stream(&mut stream)
        .into_result();
    let rec = stream
        .state
        .name_tracker
        .recorder
        .take()
        .map(|r| *r)
        .unwrap_or_default();
    let recording = Recording {
        events: rec.events,
        errors: rec.errors,
    };
    let res = match res {
        Ok((op, _)) => Ok(op),
        Err(err) => {
            let (pos, msg) = describe_error(err.into_inner().error);
            let loc = Location::SrcPos {
                src: Source::InMemory,
                pos,
            };
            crate::input_err!(loc, ParseFailure(msg))
        }
    };
    (res, recording)
}

/// A combine parse failure, as a pliron error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ParseFailure(pub String);

/// A parser that returns the current position and consumes nothing.
pub fn position<'a>()
-> impl Parser<StateStream<'a>, Output = SourcePosition, PartialState = ()> + 'a {
    crate::combine::parser(|state_stream: &mut StateStream<'a>| {
        crate::combine::ParseResult::PeekOk(state_stream.position()).into()
    })
}

/// Parse a literal keyword (like `combine::parser::char::string`) and record
/// it as a [`Event::Keyword`]. Used by derived (declarative) formats.
pub fn keyword<'a>(
    lit: &'static str,
) -> impl Parser<StateStream<'a>, Output = &'static str, PartialState = ()> + 'a {
    crate::combine::parser(move |state_stream: &mut StateStream<'a>| {
        let start = state_stream.position();
        let res = crate::combine::parser::char::string(lit)
            .parse_stream(state_stream)
            .into_result();
        if res.is_ok() {
            let end = state_stream.position();
            record(&mut state_stream.state, || Event::Keyword { start, end });
        }
        res
    })
}

/// Run `parser` and record what it consumed as an [`Event::Token`] of the
/// given semantic kind. For hand-written parsers that want their syntax
/// highlighted; behaves exactly like `parser` otherwise.
pub fn token<'a, P>(
    kind: &'static str,
    mut parser: P,
) -> impl Parser<StateStream<'a>, Output = P::Output, PartialState = ()> + 'a
where
    P: Parser<StateStream<'a>> + 'a,
{
    crate::combine::parser(move |state_stream: &mut StateStream<'a>| {
        let start = state_stream.position();
        let res = parser.parse_stream(state_stream).into_result();
        if res.is_ok() {
            let end = state_stream.position();
            record(&mut state_stream.state, || Event::Token {
                kind,
                start,
                end,
            });
        }
        res
    })
}

/// Record an error during recovering parsing.
pub(crate) fn record_error(
    state_stream: &mut StateStream<'_>,
    pos: SourcePosition,
    message: String,
) {
    if let Some(r) = recorder(&mut state_stream.state) {
        // Avoid duplicates at the same position.
        if r.errors
            .iter()
            .any(|e| e.pos == pos && e.message == message)
        {
            return;
        }
        r.errors.push(RecordedError { pos, message });
    }
}

/// Turn a combine error into (position, message).
pub(crate) fn describe_error(
    err: crate::combine::easy::Errors<char, char, SourcePosition>,
) -> (SourcePosition, String) {
    let pos = err.position;
    let mut msg = err.to_string();
    // combine prefixes "Parse error at line: L, column: C"; drop it.
    if let Some(rest) = msg.strip_prefix("Parse error at ")
        && let Some(nl) = rest.find('\n')
    {
        msg = rest[nl + 1..].trim().to_string();
    }
    // pliron errors carry their own location prefix.
    let msg = msg
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    let msg = match msg.find("] ") {
        Some(i) if msg.starts_with("[<in-memory>") => msg[i + 2..].to_string(),
        _ => msg,
    };
    (pos, msg)
}

/// Where recovery stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sync {
    /// A `;` was consumed; the next operation follows.
    Semicolon,
    /// A new operation starts at the beginning of a line (no `;`).
    NewOp,
    /// A block header (`^...`) starts at the beginning of a line.
    BlockHeader,
    /// A `}` that closes the enclosing region (not consumed).
    CloseBrace,
    /// End of input.
    Eof,
}

fn peek(s: &mut StateStream<'_>) -> Option<char> {
    let cp = s.checkpoint();
    let c = s.uncons().ok();
    let _ = s.reset(cp);
    c
}

fn bump(s: &mut StateStream<'_>) {
    let _ = s.uncons();
}

/// Skip input after an error until a synchronisation point.
///
/// `at_line_start` tells whether the current position is at the start of a
/// line (only whitespace before it). Braces are tracked so that nested
/// regions of a broken operation are skipped as a whole; a `}` in the middle
/// of a line that does not match an opened `{` is assumed to close a group
/// opened before the error and is ignored.
pub(crate) fn skip_to_sync(
    s: &mut StateStream<'_>,
    mut at_line_start: bool,
    stop_at_new_op: bool,
) -> Sync {
    let mut depth = 0u32;
    loop {
        let Some(c) = peek(s) else {
            return Sync::Eof;
        };
        if c == '\n' {
            bump(s);
            at_line_start = true;
            continue;
        }
        if c.is_whitespace() {
            bump(s);
            continue;
        }
        if at_line_start && depth == 0 {
            if c == '}' {
                return Sync::CloseBrace;
            }
            if c == '^' {
                return Sync::BlockHeader;
            }
            if stop_at_new_op && (c.is_alphabetic() || c == '_') {
                return Sync::NewOp;
            }
        }
        at_line_start = false;
        bump(s);
        match c {
            '"' => {
                // Skip a string literal.
                while let Some(c) = peek(s) {
                    bump(s);
                    if c == '\\' {
                        bump(s);
                    } else if c == '"' {
                        break;
                    }
                }
            }
            ';' if depth == 0 => return Sync::Semicolon,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
}

/// Skip whitespace; returns whether a newline was crossed (or the position
/// was already at the start of a line).
pub(crate) fn skip_spaces(s: &mut StateStream<'_>) -> bool {
    let mut crossed = s.position().column == 1;
    while let Some(c) = peek(s) {
        if !c.is_whitespace() {
            break;
        }
        if c == '\n' {
            crossed = true;
        }
        bump(s);
    }
    crossed
}

/// Is the current position preceded only by whitespace on its line?
pub(crate) fn at_line_start(s: &mut StateStream<'_>) -> bool {
    let pos = s.position();
    let Some(r) = s.state.name_tracker.recorder.as_deref() else {
        return pos.column == 1;
    };
    let Some(line) = r.lines.get((pos.line - 1).max(0) as usize) else {
        return true;
    };
    line.iter()
        .take((pos.column - 1).max(0) as usize)
        .all(|c| c.is_whitespace())
}

/// Peek the next non-space character without consuming it.
pub(crate) fn peek_char(s: &mut StateStream<'_>) -> Option<char> {
    peek(s)
}

/// Define placeholder values for the results of an operation that failed to
/// parse, so that later uses do not cascade into "unresolved" errors.
pub(crate) fn define_placeholders(state_stream: &mut StateStream<'_>) {
    let Some(results) = recorder(&mut state_stream.state).and_then(|r| r.failed_results.take())
    else {
        return;
    };
    for name_loc in results {
        let ctx = &mut *state_stream.state.ctx;
        let placeholder = ForwardRefOp::new(ctx).get_result(ctx);
        if state_stream
            .state
            .name_tracker
            .ssa_def(state_stream.state.ctx, &name_loc, placeholder)
            .is_ok()
            && let Some(r) = recorder(&mut state_stream.state)
        {
            r.placeholders.push(placeholder);
        }
    }
}

/// Run `parse` for a list item; on error, record it, recover, and report
/// where recovery stopped. On success returns `Ok(output)`.
pub(crate) fn recover_from<'a, T>(
    state_stream: &mut StateStream<'a>,
    res: ParseResult<'a, T>,
    stop_at_new_op: bool,
) -> core::result::Result<T, Sync> {
    match res {
        Ok((v, _)) => Ok(v),
        Err(e) => {
            let (pos, msg) = describe_error(e.into_inner().error);
            record_error(state_stream, pos, msg);
            // Never stop right where we are (that would not make progress).
            Err(skip_to_sync(state_stream, false, stop_at_new_op))
        }
    }
}

/// Format an unresolved-reference message.
pub(crate) fn unresolved_message(id: &Identifier, is_label: bool) -> String {
    if is_label {
        format!("Block label ^{id} is not defined in this region")
    } else {
        format!("Value {id} is not defined in this scope")
    }
}
