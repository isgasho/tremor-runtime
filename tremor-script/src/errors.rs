// Copyright 2018-2019, Wayfair GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// NOTE: We node this because of error_chain
#![allow(clippy::large_enum_variant)]
#![allow(deprecated)]
#![allow(unused_imports)]

use crate::ast::{self, BaseExpr, Expr};
use crate::errors;
use crate::lexer;
use crate::pos;
use crate::pos::{Location, Range};
use crate::registry::Context;
use base64;
use dissect;
use error_chain::error_chain;
use glob;
use grok;
use lalrpop_util;
use lalrpop_util::ParseError as LalrpopError;
use regex;
use serde_json;
pub use simd_json::ValueType;
use simd_json::{BorrowedValue as Value, ValueTrait};
use std::num;
use std::ops::Range as IRange;

#[cfg(test)]
impl PartialEq for Error {
    fn eq(&self, _other: &Error) -> bool {
        // This might be Ok since we try to compare Result in tets
        false
    }
}

type ParserError<'screw_lalrpop> =
    lalrpop_util::ParseError<pos::Location, lexer::Token<'screw_lalrpop>, errors::Error>;

impl<'screw_lalrpop> From<ParserError<'screw_lalrpop>> for Error {
    fn from(error: ParserError<'screw_lalrpop>) -> Self {
        match error {
            LalrpopError::UnrecognizedToken {
                token: Some((start, token, end)),
                expected,
            } => ErrorKind::UnrecognizedToken(
                (start.move_up_lines(2), end.move_down_lines(2)).into(),
                (start, end).into(),
                token.to_string(),
                expected.iter().map(|s| s.replace('"', "`")).collect(),
            )
            .into(),
            LalrpopError::ExtraToken {
                token: (start, token, end),
            } => ErrorKind::ExtraToken(
                (start.move_up_lines(2), end.move_down_lines(2)).into(),
                (start, end).into(),
                token.to_string(),
            )
            .into(),
            LalrpopError::InvalidToken { location: start } => {
                let mut end = start;
                end.column.0 += 1;
                ErrorKind::InvalidToken(
                    (start.move_up_lines(2), end.move_down_lines(2)).into(),
                    (start, end).into(),
                )
                .into()
            }
            _ => ErrorKind::ParserError(format!("{:?}", error)).into(),
        }
    }
}

// We need this since we call objects records
fn t2s(t: ValueType) -> &'static str {
    match t {
        ValueType::Null => "null",
        ValueType::Bool => "boolean",
        ValueType::String => "string",
        ValueType::I64 => "integer",
        ValueType::F64 => "float",
        ValueType::Array => "array",
        ValueType::Object => "record",
    }
}

pub fn best_hint(given: &str, options: &[String], max_dist: usize) -> Option<(usize, String)> {
    options
        .iter()
        .filter_map(|option| {
            let d = distance::damerau_levenshtein(given, &option);
            if d <= max_dist {
                Some((d, option))
            } else {
                None
            }
        })
        .min()
        .map(|(d, s)| (d, s.clone()))
}
pub type ErrorLocation = (Option<Range>, Option<Range>);
// FIXME: Option<(Location, Location, Option<(Location, Location)>)>
impl ErrorKind {
    pub fn expr(&self) -> ErrorLocation {
        use ErrorKind::*;
        match self {
            ArrayOutOfRange(expr, inner, _) => (Some(*expr), Some(*inner)),
            AssignIntoArray(expr, inner) => (Some(*expr), Some(*inner)),
            BadAccessInEvent(expr, inner, _) => (Some(*expr), Some(*inner)),
            BadAccessInGlobal(expr, inner, _) => (Some(*expr), Some(*inner)),
            BadAccessInLocal(expr, inner, _) => (Some(*expr), Some(*inner)),
            BadArity(expr, inner, _, _, _, _) => (Some(*expr), Some(*inner)),
            BadType(expr, inner, _, _, _) => (Some(*expr), Some(*inner)),
            BinaryDrop(expr, inner) => (Some(*expr), Some(*inner)),
            BinaryEmit(expr, inner) => (Some(*expr), Some(*inner)),
            ExtraToken(outer, inner, _) => (Some(*outer), Some(*inner)),
            InsertKeyExists(expr, inner, _) => (Some(*expr), Some(*inner)),
            InvalidAssign(expr, inner) => (Some(*expr), Some(*inner)),
            InvalidBinary(expr, inner, _, _, _) => (Some(*expr), Some(*inner)),
            InvalidDrop(expr, inner) => (Some(*expr), Some(*inner)),
            InvalidEmit(expr, inner) => (Some(*expr), Some(*inner)),
            InvalidExtractor(expr, inner, _, _, _) => (Some(*expr), Some(*inner)),
            InvalidFloatLiteral(expr, inner) => (Some(*expr), Some(*inner)),
            InvalidHexLiteral(expr, inner) => (Some(*expr), Some(*inner)),
            InvalidIntLiteral(expr, inner) => (Some(*expr), Some(*inner)),
            InvalidToken(outer, inner) => (Some(*outer), Some(*inner)),
            InvalidUnary(expr, inner, _, _) => (Some(*expr), Some(*inner)),
            MergeTypeConflict(expr, inner, _, _) => (Some(*expr), Some(*inner)),
            MissingEffectors(expr, inner) => (Some(*expr), Some(*inner)),
            MissingFunction(expr, inner, _, _, _) => (Some(*expr), Some(*inner)),
            MissingModule(expr, inner, _, _) => (Some(*expr), Some(*inner)),
            NoClauseHit(expr) => (Some(expr.expand_lines(2)), Some(*expr)),
            Oops(expr) => (Some(expr.expand_lines(2)), Some(*expr)),
            OverwritingLocal(expr, inner, _) => (Some(*expr), Some(*inner)),
            RuntimeError(expr, inner, _, _, _, _) => (Some(*expr), Some(*inner)),
            TypeConflict(expr, inner, _, _) => (Some(*expr), Some(*inner)),
            UnexpectedCharacter(expr, inner, _) => (Some(*expr), Some(*inner)),
            UnexpectedEscapeCode(expr, inner, _, _) => (Some(*expr), Some(*inner)),
            UnrecognizedToken(outer, inner, _, _) => (Some(*outer), Some(*inner)),
            UnterminatedExtractor(expr, inner, _) => (Some(*expr), Some(*inner)),
            UnterminatedIdentLiteral(expr, inner, _) => (Some(*expr), Some(*inner)),
            UnterminatedStringLiteral(expr, inner, _) => (Some(*expr), Some(*inner)),
            UpdateKeyMissing(expr, inner, _) => (Some(*expr), Some(*inner)),

            // Special cases
            EmptyScript
            | Grok(_)
            | InvalidInfluxData(_)
            | Io(_)
            | Msg(_)
            | ParseIntError(_)
            | ParserError(_)
            | UnexpectedEndOfStream
            | Utf8Error(_) => (Some(Range::default()), None),
            ErrorKind::__Nonexhaustive { .. } => (Some(Range::default()), None),
        }
    }
    pub fn token(&self) -> Option<String> {
        use ErrorKind::*;
        match self {
            UnterminatedExtractor(_, _, token) => Some(token.to_string()),
            UnterminatedStringLiteral(_, _, token) => Some(token.to_string()),
            UnterminatedIdentLiteral(_, _, token) => Some(token.to_string()),
            UnexpectedEscapeCode(_, _, s, _) => Some(s.to_string()),
            _ => None,
        }
    }
    pub fn hint(&self) -> Option<String> {
        use ErrorKind::*;
        match self {
            UnrecognizedToken(outer, inner, t, _) if t == "" && inner.0.absolute == outer.1.absolute => Some("It looks like a `;` is missing at the end of the script".into()),
            BadAccessInLocal(_, _, key) if distance::damerau_levenshtein(key, "event") <= 2 => {
                Some("Did you mean to use event?".to_owned())
            }
            TypeConflict(_, _, ValueType::F64, expected) => match expected.as_slice() {
                [ValueType::I64] => Some(
                    "You can use math::trunc() and related functions to ensure numbers are integers."
                        .to_owned(),
                ),
                _ => None
            },
            MissingModule(_, _, m, _) if m == "object" => Some("Did you mean to use the `record` module".into()),
            MissingModule(_, _, _, Some((_, suggestion))) => Some(format!("Did you mean `{}`", suggestion)),
            MissingFunction(_, _, _, _, Some((_, suggestion))) => Some(format!("Did you mean `{}`", suggestion)),
            UnrecognizedToken(_, _, t, l) if t == "event" && l.contains(&("`<ident>`".to_string())) => Some("It looks like you tried to use the key 'event' as parth of a path, consider quoting it as `event` to make it an ident oruse array like acces such as [\"event\"].".into()),
            UnrecognizedToken(_, _, t, l) if t == "-" && l.contains(&("`(`".to_string())) => Some("Try wrapping this expression in parentheses `(` ... `)`".into()),
            NoClauseHit(_) => Some("Consider adding a `default => null` clause at the end of your match or validate full coverage beforehand.".into()),
            Oops(_) => Some("Please take the error output script and idealy data and open a ticket, this should not happen.".into()),
            _ => None,
        }
    }
}

impl Error {
    pub fn context(&self) -> ErrorLocation {
        self.0.expr()
    }
    pub fn hint(&self) -> Option<String> {
        self.0.hint()
    }
    pub fn token(&self) -> Option<String> {
        self.0.token()
    }
}

fn choices<T>(choices: &[T]) -> String
where
    T: ToString,
{
    if choices.len() == 1 {
        choices[0].to_string()
    } else {
        format!(
            "one of {}",
            choices
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}

error_chain! {
    foreign_links {
        Io(std::io::Error);
        Grok(grok::Error);
        Utf8Error(std::str::Utf8Error);
        ParseIntError(num::ParseIntError);
    }
    errors {
        /*
         * ParserError
         */
        UnrecognizedToken(range: Range, loc: Range, token: String, expected: Vec<String>) {
            description("Unrecognized token")
                display("Found the token `{}` but expected {}", token, choices(expected))
        }
        ExtraToken(range: Range, loc: Range, token: String) {
            description("Extra token")
                display("Found an extra token: `{}` that does not belong there", token)
        }
        InvalidToken(range: Range, loc: Range) {
            description("Invalid token")
                display("Invalid token")
        }
        /*
         * Generic
         */
        EmptyScript {
            description("No expressions were found in the script")
                display("No expressions were found in the script")
        }
        TypeConflict(expr: Range, inner: Range, got: ValueType, expected: Vec<ValueType>) {
            description("Conflicting types")
                display("Conflicting types, got {} but expected {}", t2s(*got), choices(&expected.iter().map(|v| t2s(*v).to_string()).collect::<Vec<String>>()))
        }
        Oops(expr: Range) {
            description("Something went wrong and we're not sure what it was")
                display("Something went wrong and we're not sure what it was")
        }
        /*
         * Functions
         */
        BadArity(expr: Range, inner: Range, m: String, f: String, a: usize, calling_a: usize) {
            description("Bad arity for function")
                display("Bad arity for function {}::{}/{} but was called with {} arguments", m, f, a, calling_a)
        }
        MissingModule(expr: Range, inner: Range, m: String, suggestion: Option<(usize, String)>) {
            description("Call to undefined module")
                display("Call to undefined module {}", m)
        }
        MissingFunction(expr: Range, inner: Range, m: String, f: String, suggestion: Option<(usize, String)>) {
            description("Call to undefined function")
                display("Call to undefined function {}::{}", m, f)
        }
        BadType(expr: Range, inner: Range, m: String, f: String, a: usize) {
            description("Bad type passed to function")
                display("Bad type passed to function {}::{}/{}", m, f, a)
        }
        RuntimeError(expr: Range, inner: Range, m: String, f: String,  a: usize, c: String) {
            description("Runtime error in function")
                display("Runtime error in function {}::{}/{}: {}", m, f, a, c)
        }
        /*
         * Lexer and Parser
         */
        UnterminatedExtractor(expr: Range, inner: Range, extractor: String) {
            description("Unterminated extractor")
                display("It looks like you forgot to terminate an extractor")
        }

        UnterminatedStringLiteral(expr: Range, inner: Range, extractor: String) {
            description("Unterminated string")
                display("It looks like you forgot to terminate a string")
        }

        UnterminatedIdentLiteral(expr: Range, inner: Range, extractor: String)
        {
            description("Unterminated ident")
                display("It looks like you forgot to terminate an ident")

        }

        UnexpectedCharacter(expr: Range, inner: Range, found: char){
            description("An unexpected character was found")
                display("An unexpected character '{}' was found", found)
        }


        UnexpectedEscapeCode(expr: Range, inner: Range, token: String, found: char){
            description("An unexpected escape code was found")
                display("An unexpected escape code '{}' was found", found)

        }

        InvalidHexLiteral(expr: Range, inner: Range){
            description("An invalid hexadecimal")
                display("An invalid hexadecimal")

        }

        InvalidIntLiteral(expr: Range, inner: Range) {
            description("An invalid integer literal")
                display("An invalid integer literal")

        }
        InvalidFloatLiteral(expr: Range, inner: Range) {
            description("An invalid float literal")
                display("An invalid float literal")

        }

        UnexpectedEndOfStream {
            description("An unexpected end of stream was found")
                display("An unexpected end of stream was found")

        }

        /*
         * Parser
         */
        ParserError(pos: String) {
            description("Parser user error")
                display("Parser user error: {}", pos)
        }
        /*
         * Resolve / Assign path walking
         */

        BadAccessInLocal(expr: Range, inner: Range, key: String ) {
            description("Trying to access a non existing local key")
                display("Trying to access a non existing local key `{}`", key)
        }
        BadAccessInGlobal(expr: Range, inner: Range, key: String) {
            description("Trying to access a non existing global key")
                display("Trying to access a non existing global key `{}`", key)
        }
        BadAccessInEvent(expr: Range, inner: Range, key: String) {
            description("Trying to access a non existing event key")
                display("Trying to access a non existing event key `{}`", key)
        }
        ArrayOutOfRange(expr: Range, inner: Range, r: IRange<usize>) {
            description("Trying to access an index that is out of range")
                display("Trying to access an index that is out of range {}", if r.start == r.end {
                    format!("{}", r.start)
                } else {
                    format!("{}..{}", r.start, r.end)
                })
        }
        AssignIntoArray(expor: Range, inner: Range) {
            description("Can not assign tinto a array")
                display("It is not supported to assign value into an array")
        }
        InvalidAssign(expor: Range, inner: Range) {
            description("You can not assign that")
                display("You are trying to assing to a value that isn't valid")
        }
        /*
         * Emit & Drop
         */
        InvalidEmit(expr: Range, inner: Range) {
            description("Can not emit from this location")
                display("Can not emit from this location")

        }
        InvalidDrop(expr: Range, inner: Range) {
            description("Can not drop from this location")
                display("Can not drop from this location")

        }
        BinaryEmit(expr: Range, inner: Range) {
            description("Please enclose the value you want to emit")
                display("The expression can be read as a binary expression, please put the value you wan to emit in parentheses.")
        }
        BinaryDrop(expr: Range, inner: Range) {
            description("Please enclose the value you want to rop")
                display("The expression can be read as a binary expression, please put the value you wan to drop in parentheses.")
        }
        /*
         * Operators
         */
        InvalidUnary(expr: Range, inner: Range, op: ast::UnaryOpKind, val: ValueType) {
            description("Invalid ynary operation")
                display("The unary operation operation `{}` is not defined for the type `{}`", op, t2s(*val))
        }
        InvalidBinary(expr: Range, inner: Range, op: ast::BinOpKind, left: ValueType, right: ValueType) {
            description("Invalid ynary operation")
                display("The binary operation operation `{}` is not defined for the type `{}` and `{}`", op, t2s(*left), t2s(*right))
        }

        /*
         * match
         */
        InvalidExtractor(expr: Range, inner: Range, name: String, pattern: String, error: String){
            description("Invalid tilde predicate pattern")
                display("Invalid tilde predicate pattern: {}", error)
        }
        NoClauseHit(expr: Range){
            description("A match expression executed bu no clause matched")
                display("A match expression executed bu no clause matched")
        }
        MissingEffectors(expr: Range, inner: Range) {
            description("The clause has no effectors")
                display("The clause is missing a body")
        }
        /*
         * Patch
         */
        InsertKeyExists(expr: Range, inner: Range, key: String) {
            description("The key that is suposed to be inserted already exists")
                display("The key that is suposed to be inserted already exists: {}", key)
        }
        UpdateKeyMissing(expr: Range, inner: Range, key:String) {
            description("The key that is suposed to be updated does not exists")
                display("The key that is suposed to be updated does not exists: {}", key)
        }

        MergeTypeConflict(expr: Range, inner: Range, key:String, val: ValueType) {
            description("Merge can only be performed on keys that either do not exist or are records")
                display("Merge can only be performed on keys that either do not exist or are records but the key '{}' has the type {}", key, t2s(*val))
        }

        OverwritingLocal(expr: Range, inner: Range, val: String) {
            description("Trying to overwrite a local variable in a comprehension case")
                display("Trying to overwrite the local variable `{}` in a comprehension case", val)
        }
        InvalidInfluxData(s: String) {
            description("Invalid Influx Line Protocol data")
                display("Invalid Influx Line Protocol data: {}", s)
        }

    }
}

impl<Ctx: Context> Expr<Ctx> {
    pub fn error_array_out_of_bound<T>(
        &self,
        inner: &Expr<Ctx>,
        path: &ast::Path<Ctx>,
        r: IRange<usize>,
    ) -> Result<T> {
        let expr: Range = self.into();
        Err(match path {
            ast::Path::Local(_path) => ErrorKind::ArrayOutOfRange(expr, inner.into(), r).into(),
            ast::Path::Meta(_path) => ErrorKind::ArrayOutOfRange(expr, inner.into(), r).into(),
            ast::Path::Event(_path) => ErrorKind::ArrayOutOfRange(expr, inner.into(), r).into(),
        })
    }

    pub fn error_bad_key<T>(
        &self,
        inner: &Expr<Ctx>,
        path: &ast::Path<Ctx>,
        key: String,
    ) -> Result<T> {
        let expr: Range = self.into();
        Err(match path {
            ast::Path::Local(_p) => ErrorKind::BadAccessInLocal(expr, inner.into(), key).into(),
            ast::Path::Meta(_p) => ErrorKind::BadAccessInGlobal(expr, inner.into(), key).into(),
            ast::Path::Event(_p) => ErrorKind::BadAccessInEvent(expr, inner.into(), key).into(),
        })
    }
    pub fn error_type_conflict_mult<T>(
        &self,
        inner: &Expr<Ctx>,
        got: ValueType,
        expected: Vec<ValueType>,
    ) -> Result<T> {
        Err(ErrorKind::TypeConflict(self.into(), inner.into(), got, expected).into())
    }

    pub fn error_type_conflict<T>(
        &self,
        inner: &Expr<Ctx>,
        got: ValueType,
        expected: ValueType,
    ) -> Result<T> {
        self.error_type_conflict_mult(inner, got, vec![expected])
    }

    pub fn error_guard_not_bool<T>(&self, inner: &Expr<Ctx>, got: &Value) -> Result<T> {
        self.error_type_conflict(inner, got.kind(), ValueType::Bool)
    }

    pub fn error_invalid_unary<T>(
        &self,
        inner: &Expr<Ctx>,
        op: ast::UnaryOpKind,
        val: &Value,
    ) -> Result<T> {
        Err(ErrorKind::InvalidUnary(self.into(), inner.into(), op, val.kind()).into())
    }
    pub fn error_invalid_binary<T>(
        &self,
        inner: &Expr<Ctx>,
        op: ast::BinOpKind,
        left: &Value,
        right: &Value,
    ) -> Result<T> {
        Err(
            ErrorKind::InvalidBinary(self.into(), inner.into(), op, left.kind(), right.kind())
                .into(),
        )
    }
    pub fn error_no_clause_hit<T>(&self) -> Result<T> {
        Err(ErrorKind::NoClauseHit(self.into()).into())
    }

    pub fn error_oops<T>(&self) -> Result<T> {
        Err(ErrorKind::Oops(self.into()).into())
    }

    pub fn error_patch_insert_key_exists<T>(&self, inner: &Expr<Ctx>, key: String) -> Result<T> {
        Err(ErrorKind::InsertKeyExists(self.into(), inner.into(), key).into())
    }

    pub fn error_patch_update_key_missing<T>(&self, inner: &Expr<Ctx>, key: String) -> Result<T> {
        Err(ErrorKind::UpdateKeyMissing(self.into(), inner.into(), key).into())
    }
    pub fn error_missing_effector<T>(&self, inner: &Expr<Ctx>) -> Result<T> {
        Err(ErrorKind::MissingEffectors(self.into(), inner.into()).into())
    }
    pub fn error_overwriting_local_in_comprehension<T>(
        &self,
        inner: &Expr<Ctx>,
        key: String,
    ) -> Result<T> {
        Err(ErrorKind::OverwritingLocal(self.into(), inner.into(), key).into())
    }

    pub fn error_patch_merge_type_conflict<T>(
        &self,
        inner: &Expr<Ctx>,
        key: String,
        val: &Value,
    ) -> Result<T> {
        Err(ErrorKind::MergeTypeConflict(self.into(), inner.into(), key, val.kind()).into())
    }

    pub fn error_assign_array<T>(&self, inner: &Expr<Ctx>) -> Result<T> {
        Err(ErrorKind::AssignIntoArray(self.into(), inner.into()).into())
    }
    pub fn error_invalid_assign_target<T>(&self) -> Result<T> {
        let inner: Range = self.into();

        Err(ErrorKind::InvalidAssign(inner.expand_lines(2), inner).into())
    }
}