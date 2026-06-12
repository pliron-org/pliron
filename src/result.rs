//! Utilities for error handling
//!
//! - [ErrorKind] describes the kind of error.
//! - [struct@Error] is the main error type used throughout the compiler. This `struct` contains
//!   the field `err` which holds the actual error that implements [AnyError]. [AnyError] is an
//!   extension trait to enforce [core::error::Error] + [Downcast].
//! - [Result] is an alias for `std::result::Result` with the error type set to [struct@Error].
//!   [Result] implements [ExpectOk], which provides a method [expect_ok](ExpectOk::expect_ok)
//!   to `unwrap` the result, and if that fails, panics with the error message printed using
//!   [Printable].
//!
//! The following macros are provided to conveniently create [struct@Error] and [Result] objects.
//!   - [input_error_noloc]: Create [struct@Error] for input errors without location information
//!   - [input_error]: Create [struct@Error] for input errors with location information
//!   - [input_err_noloc]: Create [Result] for input errors without location information
//!   - [input_err]: Create [Result] for input errors with location information
//!   - [verify_error_noloc]: Create [struct@Error] for verification errors without location information
//!   - [verify_error]: Create [struct@Error] for verification errors with location information
//!   - [verify_err_noloc]: Create [Result] for verification errors without location information
//!   - [verify_err]: Create [Result] for verification errors with location information
//!   - [arg_error_noloc]: Create [struct@Error] for argument errors without location information
//!   - [arg_error]: Create [struct@Error] for argument errors with location information
//!   - [arg_err_noloc]: Create [Result] for argument errors without location information
//!   - [arg_err]: Create [Result] for argument errors with location information
//!
//! The inner `err` when constructing [struct@Error] is typically a custom error type
//! that derives [core::error::Error] using [thiserror].
//!
//! ```
//! use thiserror::Error;
//! use expect_test::expect;
//! use pliron::{
//!     context::Context,
//!     input_error_noloc, result::Error
//! };
//!
//! #[derive(Error, Debug)]
//! #[error("This is my error: {0}")]
//! pub struct MyError(String);
//!
//! let err: Error = input_error_noloc!(MyError("Something went wrong".to_string()));
//! expect![[r#"
//!     Compilation error: invalid input program.
//!     This is my error: Something went wrong"#
//! ]].assert_eq(&err.to_string());
//! ```
//!
//! The [struct@Error] type can be printed without [Context], as in the example above.
//! For more informational error messages (such as including a source location),
//! it should be printed using [Printable], passing the [Context].
//!
//! Sometimes the error object may need [Context] for better / more convenient
//! formatting. In such cases, they can implement [Printable] and mark themselves
//! using [type_to_trait]. [struct@Error]'s [Printable] implementation will check if
//! the inner error implements [Printable] and use it if available.
//!
//! ```rust
//! use core::fmt::Display;
//! use expect_test::expect;
//! use thiserror::Error;
//! use pliron::{
//!     context::Context,
//!     printable::{State, Printable},
//!     type_to_trait, input_error_noloc, result::Error
//! };
//!
//! #[derive(Debug, Error)]
//! #[error("Error displayed using core::fmt::Display: {0}")]
//! pub struct PrintableErr(String);

//! impl Printable for PrintableErr {
//!     fn fmt(
//!         &self,
//!         ctx: &Context,
//!         state: &State,
//!         f: &mut core::fmt::Formatter<'_>,
//!     ) -> core::fmt::Result {
//!         write!(f, "Error printed using Printable: {}", self.0)
//!     }
//! }
//!
//! /// Marking `PrintableErr` as implementing `Printable` enables
//! /// `dyn AnyError` to be downcasted to `dyn Printable`.
//! type_to_trait!(PrintableErr, Printable);
//!
//! let ctx = &mut Context::new();
//! let res: Error = input_error_noloc!(PrintableErr("Test error".to_string()));
//! expect![[r#"
//!     Compilation error: invalid input program.
//!     Error printed using Printable: Test error"#
//! ]].assert_eq(&res.disp(ctx).to_string());
//! ```

use core::fmt::Display;

#[cfg(doc)]
use crate::{
    arg_err, arg_err_noloc, arg_error, arg_error_noloc, input_err, input_err_noloc, input_error,
    input_error_noloc, type_to_trait, verify_err, verify_err_noloc, verify_error,
    verify_error_noloc,
};

use alloc::{boxed::Box, string::String};
use downcast_rs::{Downcast, impl_downcast};
use thiserror::Error;

use crate::{
    context::Context,
    deps::backtrace::{Backtrace, BacktraceStatus},
    location::{Located, Location},
    printable::{Printable, State},
    utils::trait_cast::any_to_trait,
};

/// A wrapper trait combining [`core::error::Error`] and [`downcast_rs::Downcast`]
/// to allow upcasting to [Any](core::any::Any), so that, we can downcast it
/// to the [Printable] trait if the error implements it.
pub trait AnyError: core::error::Error + Send + Sync + 'static + Downcast {}
impl<T: core::error::Error + Send + Sync + 'static> AnyError for T {}
impl_downcast!(AnyError);

/// The kinds of errors we have during compilation.
#[derive(Debug, Error)]
pub enum ErrorKind {
    /// Something's wrong with the program being compiled
    #[error("invalid input program")]
    InvalidInput,
    /// The IR was found to be inconsistent or invalid during verification
    #[error("verification failed")]
    VerificationFailed,
    /// Inconsistent or invalid argument(s) passed to a pliron function.
    #[error("invalid argument")]
    InvalidArgument,
}

/// An error object that can hold any [core::error::Error].
#[derive(Debug)]
pub struct Error {
    /// The kind of error this is
    pub kind: ErrorKind,
    /// The actual error object describing the error
    pub err: Box<dyn AnyError>,
    /// Location of this error in the code being compiled
    pub loc: Location,
    /// Details of how this error occurred
    pub backtrace: Backtrace,
}

impl core::error::Error for Error {}

/// This does not print [Location] or [Backtrace]. Use [Printable::disp] for that.
impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Compilation error: {}.\n{}", self.kind, self.err)
    }
}

impl Printable for Error {
    fn fmt(
        &self,
        ctx: &Context,
        _state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        if self.loc.is_unknown() {
            writeln!(f, "Compilation error: {}.", self.kind,)
        } else {
            writeln!(
                f,
                "[{}] Compilation error: {}.",
                self.loc.disp(ctx),
                self.kind,
            )
        }?;

        if let Some(self_val) = self.err.downcast_ref::<Error>() {
            write!(f, "{}", self_val.disp(ctx))?;
        } else if let Some(self_val) = any_to_trait::<dyn Printable>((*self.err).as_any()) {
            write!(f, "{}", self_val.disp(ctx))?;
        } else {
            write!(f, "{}", self.err)?;
            if self.backtrace.status() == BacktraceStatus::Captured {
                write!(f, "\nError backtrace:\n{}", self.backtrace)?;
            }
        }

        Ok(())
    }
}

impl Located for Error {
    fn loc(&self) -> Location {
        self.loc.clone()
    }

    fn set_loc(&mut self, loc: Location) {
        self.loc = loc;
    }
}

/// Type alias for [core::result::Result] with the error type set to [struct@Error]
pub type Result<T> = core::result::Result<T, Error>;

impl<T: Printable> Printable for Result<T> {
    fn fmt(
        &self,
        ctx: &Context,
        state: &State,
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        match self {
            Ok(val) => val.fmt(ctx, state, f),
            Err(err) => Printable::fmt(err, ctx, state, f),
        }
    }
}

/// Same as `unwrap`, but with the panic message printed using [Printable].
pub trait ExpectOk<T> {
    /// Unwraps the result, panicking if it is an error.
    #[track_caller]
    fn expect_ok(self, ctx: &Context) -> T;
}

impl<T> ExpectOk<T> for Result<T> {
    fn expect_ok(self, ctx: &Context) -> T {
        match self {
            Ok(val) => val,
            Err(err) => {
                panic!("{}", err.disp(ctx))
            }
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Error)]
#[error("{0}")]
pub struct StringError(pub String);

/// Specify [ErrorKind] and create [struct@Error] from any [core::error::Error] object.
/// To create [Result], use [create_err!](crate::create_err) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// It may be shorter to just use [verify_error!](crate::verify_error),
/// [input_error!](crate::input_error) or [arg_error!](crate::arg_error) instead.
#[macro_export]
macro_rules! create_error {
    ($loc: expr, $kind: expr, $str: literal $($t:tt)*) => {
        $crate::create_error!($loc, $kind, $crate::result::StringError($crate::alloc::format!($str $($t)*)))
    };
    ($loc: expr, $kind: expr, $err: expr) => {
        $crate::result::Error {
            kind: $kind,
            err: $crate::alloc::boxed::Box::new($err),
            loc: $loc,
            backtrace: $crate::deps::backtrace::Backtrace::capture(),
        }
    };
}

/// Specify [ErrorKind] and create [Result] from any [core::error::Error] object.
/// To create [struct@Error], use [create_error!](crate::create_error) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// It may be shorter to just use [verify_err!](crate::verify_err),
/// [input_err!](crate::input_err) or [arg_err!](crate::arg_err) instead.
#[macro_export]
macro_rules! create_err {
    ($loc: expr, $kind: expr, $str: literal $($t:tt)*) => {
        $crate::create_err!($loc, $kind, $crate::result::StringError($crate::alloc::format!($str $($t)*)))
    };
    ($loc: expr, $kind: expr, $err: expr) => {
        Err($crate::create_error!($loc, $kind, $err))
    };
}

// Create [ErrorKind::VerificationFailed] [struct@Error] from any [core::error::Error] object.
/// To create [Result], use [verify_err!](crate::verify_err) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// ```rust
/// use thiserror::Error;
/// use pliron::{verify_error, result::{Result, ErrorKind, Error}, location::Location};
///
/// #[derive(Error, Debug)]
/// #[error("sample error")]
/// pub struct SampleErr;
///
/// assert!(
///     matches!(
///         verify_error!(Location::Unknown, SampleErr),
///         Error {
///            kind: ErrorKind::VerificationFailed,
///            err,
///            ..
///         } if err.is::<SampleErr>()
/// ));
///
/// let res_msg: Error = verify_error!(Location::Unknown, "Some formatted {}", 0);
/// assert_eq!(
///     res_msg.err.to_string(),
///     "Some formatted 0"
/// );
/// ```
#[macro_export]
macro_rules! verify_error {
    ($loc: expr, $($t:tt)*) => {
        $crate::create_error!($loc, $crate::result::ErrorKind::VerificationFailed, $($t)*)
    }
}

/// Create [ErrorKind::VerificationFailed] [Result] from any [core::error::Error] object.
/// To create [struct@Error], use [verify_error!](crate::verify_error) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// ```rust
/// use thiserror::Error;
/// use pliron::{verify_err, result::{Result, ErrorKind, Error}, location::Location};
///
/// #[derive(Error, Debug)]
/// #[error("sample error")]
/// pub struct SampleErr;
///
/// assert!(
///     matches!(
///         verify_err!(Location::Unknown, SampleErr),
///         Result::<()>::Err(Error {
///            kind: ErrorKind::VerificationFailed,
///            err,
///            ..
///         }) if err.is::<SampleErr>()
/// ));
///
/// let res_msg: Result<()> = verify_err!(Location::Unknown, "Some formatted {}", 0);
/// assert_eq!(
///     res_msg.unwrap_err().err.to_string(),
///     "Some formatted 0"
/// );
/// ```
#[macro_export]
macro_rules! verify_err {
    ($loc: expr, $($t:tt)*) => {
        $crate::create_err!($loc, $crate::result::ErrorKind::VerificationFailed, $($t)*)
    }
}

/// Create [ErrorKind::InvalidInput] [struct@Error] from any [core::error::Error] object.
/// To create [Result], use [input_err!](crate::input_err) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// ```rust
/// use thiserror::Error;
/// use pliron::{input_error, result::{Result, ErrorKind, Error}, location::Location};
///
/// #[derive(Error, Debug)]
/// #[error("sample error")]
/// pub struct SampleErr;
///
/// assert!(
///     matches!(
///         input_error!(Location::Unknown, SampleErr),
///         Error {
///            kind: ErrorKind::InvalidInput,
///            err,
///            ..
///         } if err.is::<SampleErr>()
/// ));
///
/// let res_msg: Error = input_error!(Location::Unknown, "Some formatted {}", 0);
/// assert_eq!(
///     res_msg.err.to_string(),
///     "Some formatted 0"
/// );
/// ```
#[macro_export]
macro_rules! input_error {
    ($loc: expr, $($t:tt)*) => {
        $crate::create_error!($loc, $crate::result::ErrorKind::InvalidInput, $($t)*)
    }
}

/// Create [ErrorKind::InvalidInput] [Result] from any [core::error::Error] object.
/// To create [struct@Error], use [input_error!](crate::input_error) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// ```rust
/// use thiserror::Error;
/// use pliron::{input_err, result::{Result, ErrorKind, Error}, location::Location};
///
/// #[derive(Error, Debug)]
/// #[error("sample error")]
/// pub struct SampleErr;
///
/// assert!(
///     matches!(
///         input_err!(Location::Unknown, SampleErr),
///         Result::<()>::Err(Error {
///            kind: ErrorKind::InvalidInput,
///            err,
///            ..
///         }) if err.is::<SampleErr>()
/// ));
///
/// let res_msg: Result<()> = input_err!(Location::Unknown, "Some formatted {}", 0);
/// assert_eq!(
///     res_msg.unwrap_err().err.to_string(),
///     "Some formatted 0"
/// );
/// ```
#[macro_export]
macro_rules! input_err {
    ($loc: expr, $($t:tt)*) => {
        $crate::create_err!($loc, $crate::result::ErrorKind::InvalidInput, $($t)*)
    }
}

/// Create [ErrorKind::InvalidArgument] [struct@Error] from any [core::error::Error] object.
/// To create [Result], use [arg_err!](crate::arg_err) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// ```rust
/// use thiserror::Error;
/// use pliron::{arg_error, result::{Result, ErrorKind, Error}, location::Location};
///
/// #[derive(Error, Debug)]
/// #[error("sample error")]
/// pub struct SampleErr;
///
/// assert!(
///     matches!(
///         arg_error!(Location::Unknown, SampleErr),
///         Error {
///            kind: ErrorKind::InvalidArgument,
///            err,
///            ..
///         } if err.is::<SampleErr>()
/// ));
///
/// let res_msg: Error = arg_error!(Location::Unknown, "Some formatted {}", 0);
/// assert_eq!(
///     res_msg.err.to_string(),
///     "Some formatted 0"
/// );
/// ```
#[macro_export]
macro_rules! arg_error {
    ($loc: expr, $($t:tt)*) => {
        $crate::create_error!($loc, $crate::result::ErrorKind::InvalidArgument, $($t)*)
    }
}

/// Create [ErrorKind::InvalidArgument] [Result] from any [core::error::Error] object.
/// To create [struct@Error], use [arg_error!](crate::arg_error) instead.
/// The macro also accepts [format!](alloc::format) like arguments to create one-off errors.
/// ```rust
/// use thiserror::Error;
/// use pliron::{arg_err, result::{Result, ErrorKind, Error}, location::Location};
///
/// #[derive(Error, Debug)]
/// #[error("sample error")]
/// pub struct SampleErr;
///
/// assert!(
///     matches!(
///         arg_err!(Location::Unknown, SampleErr),
///         Result::<()>::Err(Error {
///            kind: ErrorKind::InvalidArgument,
///            err,
///            ..
///         }) if err.is::<SampleErr>()
/// ));
///
/// let res_msg: Result<()> = arg_err!(Location::Unknown, "Some formatted {}", 0);
/// assert_eq!(
///     res_msg.unwrap_err().err.to_string(),
///     "Some formatted 0"
/// );
/// ```
#[macro_export]
macro_rules! arg_err {
    ($loc: expr, $($t:tt)*) => {
        $crate::create_err!($loc, $crate::result::ErrorKind::InvalidArgument, $($t)*)
    }
}

/// Same as [verify_error] but when no location is known.
#[macro_export]
macro_rules! verify_error_noloc {
    ($($t:tt)*) => {
        $crate::create_error!($crate::location::Location::Unknown, $crate::result::ErrorKind::VerificationFailed, $($t)*)
    }
}

/// Same as [verify_err] but when no location is known.
#[macro_export]
macro_rules! verify_err_noloc {
    ($($t:tt)*) => {
        $crate::create_err!($crate::location::Location::Unknown, $crate::result::ErrorKind::VerificationFailed, $($t)*)
    }
}

/// Same as [input_error] but when no location is known.
#[macro_export]
macro_rules! input_error_noloc {
    ($($t:tt)*) => {
        $crate::create_error!($crate::location::Location::Unknown, $crate::result::ErrorKind::InvalidInput, $($t)*)
    }
}

/// Same as [input_err] but when no location is known.
#[macro_export]
macro_rules! input_err_noloc {
    ($($t:tt)*) => {
        $crate::create_err!($crate::location::Location::Unknown, $crate::result::ErrorKind::InvalidInput, $($t)*)
    }
}

/// Same as [arg_error] but when no location is known.
#[macro_export]
macro_rules! arg_error_noloc {
    ($($t:tt)*) => {
        $crate::create_error!($crate::location::Location::Unknown, $crate::result::ErrorKind::InvalidArgument, $($t)*)
    }
}

/// Same as [arg_err] but when no location is known.
#[macro_export]
macro_rules! arg_err_noloc {
    ($($t:tt)*) => {
        $crate::create_err!($crate::location::Location::Unknown, $crate::result::ErrorKind::InvalidArgument, $($t)*)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use expect_test::expect;
    use thiserror::Error;

    use crate::{
        combine::stream::position::{Positioner, SourcePosition},
        context::Context,
        location::{Location, Source},
        printable::Printable,
        type_to_trait,
    };

    #[derive(Debug, Error)]
    #[error("Test error")]
    pub struct TestErr;

    #[derive(Debug, Error)]
    pub struct PrintableErr(String);

    impl core::fmt::Display for PrintableErr {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "Display: {}", self.0)
        }
    }

    impl Printable for PrintableErr {
        fn fmt(
            &self,
            _ctx: &Context,
            _state: &crate::printable::State,
            f: &mut core::fmt::Formatter<'_>,
        ) -> core::fmt::Result {
            write!(f, "Printable: {}", self.0)
        }
    }
    // We want to be able to downcast `PrintableErr` as `Any` to `Printable`
    type_to_trait!(PrintableErr, Printable);

    #[test]
    fn wrapped_err() {
        let ctx = &mut Context::new();
        let src = Source::new_from_file(ctx, "/tmp/test.pliron");

        let pos1 = SourcePosition::default();
        let loc1 = Location::SrcPos { src, pos: pos1 };

        let mut pos2 = SourcePosition::default();
        pos2.update(&' ');
        let loc2 = Location::SrcPos { pos: pos2, src };

        let res = input_error!(loc2, TestErr);
        let wrapped_res = input_error!(loc1, res);
        let expected_err_msg = expect![[r#"
            ["/tmp/test.pliron": line: 1, column: 1] Compilation error: invalid input program.
            ["/tmp/test.pliron": line: 1, column: 2] Compilation error: invalid input program.
            Test error"#]];

        let actual_err = wrapped_res.disp(ctx).to_string();
        expected_err_msg.assert_eq(&actual_err);
    }

    #[test]
    fn printable_err() {
        let ctx = &mut Context::new();
        let src = Source::new_from_file(ctx, "/tmp/test.pliron");

        let pos = SourcePosition::default();
        let loc = Location::SrcPos { src, pos };

        let res = input_error!(loc, PrintableErr("Test error".to_string()));

        let expected_err_msg = expect![[r#"
            ["/tmp/test.pliron": line: 1, column: 1] Compilation error: invalid input program.
            Printable: Test error"#]];

        let actual_err = res.disp(ctx).to_string();
        expected_err_msg.assert_eq(&actual_err);
    }
}
