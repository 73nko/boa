//! Member expression parsing.
//!
//! More information:
//!  - [ECMAScript specification][spec]
//!
//! [spec]: https://tc39.es/ecma262/#prod-MemberExpression

use super::arguments::Arguments;
use crate::syntax::{
    ast::{
        node::{
            field::{get_private_field::GetPrivateField, GetConstField, GetField},
            Call, GetSuperField, New, Node,
        },
        Keyword, Punctuator,
    },
    lexer::TokenKind,
    parser::{
        expression::{
            left_hand_side::template::TaggedTemplateLiteral, primary::PrimaryExpression, Expression,
        },
        AllowAwait, AllowYield, Cursor, ParseError, ParseResult, TokenParser,
    },
};
use boa_interner::{Interner, Sym};
use boa_profiler::Profiler;
use std::io::Read;

/// Parses a member expression.
///
/// More information:
///  - [ECMAScript specification][spec]
///
/// [spec]: https://tc39.es/ecma262/#prod-MemberExpression
#[derive(Debug, Clone, Copy)]
pub(super) struct MemberExpression {
    name: Option<Sym>,
    allow_yield: AllowYield,
    allow_await: AllowAwait,
}

impl MemberExpression {
    /// Creates a new `MemberExpression` parser.
    pub(super) fn new<N, Y, A>(name: N, allow_yield: Y, allow_await: A) -> Self
    where
        N: Into<Option<Sym>>,
        Y: Into<AllowYield>,
        A: Into<AllowAwait>,
    {
        Self {
            name: name.into(),
            allow_yield: allow_yield.into(),
            allow_await: allow_await.into(),
        }
    }
}

impl<R> TokenParser<R> for MemberExpression
where
    R: Read,
{
    type Output = Node;

    fn parse(self, cursor: &mut Cursor<R>, interner: &mut Interner) -> ParseResult {
        let _timer = Profiler::global().start_event("MemberExpression", "Parsing");

        let token = cursor.peek(0, interner)?.ok_or(ParseError::AbruptEnd)?;
        let mut lhs = match token.kind() {
            TokenKind::Keyword((Keyword::New | Keyword::Super, true)) => {
                return Err(ParseError::general(
                    "keyword must not contain escaped characters",
                    token.span().start(),
                ));
            }
            TokenKind::Keyword((Keyword::New, false)) => {
                let _next = cursor.next(interner).expect("new keyword disappeared");
                let lhs = self.parse(cursor, interner)?;
                let args = match cursor.peek(0, interner)? {
                    Some(next) if next.kind() == &TokenKind::Punctuator(Punctuator::OpenParen) => {
                        Arguments::new(self.allow_yield, self.allow_await)
                            .parse(cursor, interner)?
                    }
                    _ => Box::new([]),
                };
                let call_node = Call::new(lhs, args);

                Node::from(New::from(call_node))
            }
            TokenKind::Keyword((Keyword::Super, _)) => {
                cursor.next(interner).expect("token disappeared");
                let token = cursor.next(interner)?.ok_or(ParseError::AbruptEnd)?;
                match token.kind() {
                    TokenKind::Punctuator(Punctuator::Dot) => {
                        let token = cursor.next(interner)?.ok_or(ParseError::AbruptEnd)?;
                        let field = match token.kind() {
                            TokenKind::Identifier(name) => GetSuperField::from(*name),
                            TokenKind::Keyword((kw, _)) => GetSuperField::from(kw.to_sym(interner)),
                            TokenKind::BooleanLiteral(true) => GetSuperField::from(Sym::TRUE),
                            TokenKind::BooleanLiteral(false) => GetSuperField::from(Sym::FALSE),
                            TokenKind::NullLiteral => GetSuperField::from(Sym::NULL),
                            TokenKind::PrivateIdentifier(_) => {
                                return Err(ParseError::general(
                                    "unexpected private identifier",
                                    token.span().start(),
                                ));
                            }
                            _ => {
                                return Err(ParseError::unexpected(
                                    token.to_string(interner),
                                    token.span(),
                                    "expected super property",
                                ))
                            }
                        };
                        field.into()
                    }
                    TokenKind::Punctuator(Punctuator::OpenBracket) => {
                        let expr = Expression::new(None, true, self.allow_yield, self.allow_await)
                            .parse(cursor, interner)?;
                        cursor.expect(Punctuator::CloseBracket, "super property", interner)?;
                        GetSuperField::from(expr).into()
                    }
                    _ => {
                        return Err(ParseError::unexpected(
                            token.to_string(interner),
                            token.span(),
                            "expected super property",
                        ))
                    }
                }
            }
            _ => PrimaryExpression::new(self.name, self.allow_yield, self.allow_await)
                .parse(cursor, interner)?,
        };

        while let Some(tok) = cursor.peek(0, interner)? {
            match tok.kind() {
                TokenKind::Punctuator(Punctuator::Dot) => {
                    cursor
                        .next(interner)?
                        .expect("dot punctuator token disappeared"); // We move the parser forward.

                    let token = cursor.next(interner)?.ok_or(ParseError::AbruptEnd)?;

                    match token.kind() {
                        TokenKind::Identifier(name) => lhs = GetConstField::new(lhs, *name).into(),
                        TokenKind::Keyword((kw, _)) => {
                            lhs = GetConstField::new(lhs, kw.to_sym(interner)).into();
                        }
                        TokenKind::BooleanLiteral(true) => {
                            lhs = GetConstField::new(lhs, Sym::TRUE).into();
                        }
                        TokenKind::BooleanLiteral(false) => {
                            lhs = GetConstField::new(lhs, Sym::FALSE).into();
                        }
                        TokenKind::NullLiteral => {
                            lhs = GetConstField::new(lhs, Sym::NULL).into();
                        }
                        TokenKind::PrivateIdentifier(name) => {
                            cursor.push_used_private_identifier(*name, token.span().start())?;
                            lhs = GetPrivateField::new(lhs, *name).into();
                        }
                        _ => {
                            return Err(ParseError::expected(
                                ["identifier".to_owned()],
                                token.to_string(interner),
                                token.span(),
                                "member expression",
                            ));
                        }
                    }
                }
                TokenKind::Punctuator(Punctuator::OpenBracket) => {
                    cursor
                        .next(interner)?
                        .expect("open bracket punctuator token disappeared"); // We move the parser forward.
                    let idx = Expression::new(None, true, self.allow_yield, self.allow_await)
                        .parse(cursor, interner)?;
                    cursor.expect(Punctuator::CloseBracket, "member expression", interner)?;
                    lhs = GetField::new(lhs, idx).into();
                }
                TokenKind::TemplateNoSubstitution { .. } | TokenKind::TemplateMiddle { .. } => {
                    lhs = TaggedTemplateLiteral::new(
                        self.allow_yield,
                        self.allow_await,
                        tok.span().start(),
                        lhs,
                    )
                    .parse(cursor, interner)?;
                }
                _ => break,
            }
        }

        Ok(lhs)
    }
}
