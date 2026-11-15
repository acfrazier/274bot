use super::errors::{
    DeclarationReason, ExpressionKind, KeyKind, LiteralReason, ScriptDeclarationError,
    UnsupportedWrapper,
};
use deno_ast::swc::ast::{ArrayLit, Expr, Lit, ObjectLit, Prop, PropName, PropOrSpread, UnaryOp};
use deno_ast::swc::common::{Span, Spanned};
use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SpannedValue {
    pub value: LiteralValue,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum LiteralValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Array(Vec<SpannedValue>),
    Object(Vec<SpannedProperty>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SpannedProperty {
    pub key: String,
    pub key_span: Range<usize>,
    pub value: SpannedValue,
    pub span: Range<usize>,
}

pub(super) fn parse_literal(
    expr: &Expr,
    parsed_text: &str,
    source_offset: usize,
    path: &Path,
    original_source: &str,
    field_path: &str,
) -> Result<SpannedValue, ScriptDeclarationError> {
    let (expr, wrapper_span) = unwrap_wrappers(
        expr,
        parsed_text,
        source_offset,
        path,
        original_source,
        field_path,
    )?;
    let span = source_range(expr.span(), parsed_text, source_offset);
    let value = match expr {
        Expr::Lit(Lit::Str(string)) => LiteralValue::String(decode_string(
            string.span,
            parsed_text,
            source_offset,
            path,
            original_source,
            field_path,
        )?),
        Expr::Lit(Lit::Bool(boolean)) => LiteralValue::Boolean(boolean.value),
        Expr::Lit(Lit::Num(number)) => {
            if !number.value.is_finite() {
                return Err(error(
                    path,
                    original_source,
                    span.clone(),
                    field_path,
                    DeclarationReason::Literal(LiteralReason::NonFinite),
                ));
            }
            LiteralValue::Number(number.value)
        }
        Expr::Array(array) => LiteralValue::Array(parse_array(
            array,
            parsed_text,
            source_offset,
            path,
            original_source,
            field_path,
        )?),
        Expr::Object(object) => LiteralValue::Object(parse_object(
            object,
            parsed_text,
            source_offset,
            path,
            original_source,
            field_path,
        )?),
        Expr::Unary(unary) if unary.op == UnaryOp::Minus => {
            let (operand, _) = unwrap_wrappers(
                &unary.arg,
                parsed_text,
                source_offset,
                path,
                original_source,
                field_path,
            )?;
            let Expr::Lit(Lit::Num(number)) = operand else {
                return Err(expression_error(
                    path,
                    original_source,
                    span,
                    field_path,
                    expression_kind(&Expr::Unary(unary.clone())),
                ));
            };
            let value = -number.value;
            if !value.is_finite() {
                return Err(error(
                    path,
                    original_source,
                    span,
                    field_path,
                    DeclarationReason::Literal(LiteralReason::NonFinite),
                ));
            }
            LiteralValue::Number(value)
        }
        other => {
            return Err(expression_error(
                path,
                original_source,
                span,
                field_path,
                expression_kind(other),
            ));
        }
    };
    Ok(SpannedValue {
        value,
        span: wrapper_span.unwrap_or_else(|| source_range(expr.span(), parsed_text, source_offset)),
    })
}

fn parse_array(
    array: &ArrayLit,
    parsed_text: &str,
    source_offset: usize,
    path: &Path,
    original_source: &str,
    field_path: &str,
) -> Result<Vec<SpannedValue>, ScriptDeclarationError> {
    let mut values = Vec::with_capacity(array.elems.len());
    for (index, element) in array.elems.iter().enumerate() {
        let Some(element) = element else {
            return Err(error(
                path,
                original_source,
                source_range(array.span, parsed_text, source_offset),
                field_path,
                DeclarationReason::Literal(LiteralReason::SparseArray),
            ));
        };
        if element.spread.is_some() {
            return Err(expression_error(
                path,
                original_source,
                source_range(element.span(), parsed_text, source_offset),
                &format!("{field_path}[{index}]"),
                ExpressionKind::Spread,
            ));
        }
        values.push(parse_literal(
            &element.expr,
            parsed_text,
            source_offset,
            path,
            original_source,
            &format!("{field_path}[{index}]"),
        )?);
    }
    Ok(values)
}

fn parse_object(
    object: &ObjectLit,
    parsed_text: &str,
    source_offset: usize,
    path: &Path,
    original_source: &str,
    field_path: &str,
) -> Result<Vec<SpannedProperty>, ScriptDeclarationError> {
    let mut properties = Vec::with_capacity(object.props.len());
    let mut keys = HashSet::with_capacity(object.props.len());
    for item in &object.props {
        let (key, key_span, value, property_span) = match item {
            PropOrSpread::Spread(spread) => {
                return Err(expression_error(
                    path,
                    original_source,
                    source_range(spread.span(), parsed_text, source_offset),
                    field_path,
                    ExpressionKind::Spread,
                ));
            }
            PropOrSpread::Prop(property) => match &**property {
                Prop::KeyValue(pair) => {
                    let (key, key_span) = property_key(
                        &pair.key,
                        parsed_text,
                        source_offset,
                        path,
                        original_source,
                        field_path,
                    )?;
                    (key, key_span, pair.value.as_ref(), pair.span())
                }
                Prop::Shorthand(ident) => {
                    return Err(expression_error(
                        path,
                        original_source,
                        source_range(ident.span, parsed_text, source_offset),
                        field_path,
                        ExpressionKind::ShorthandProperty,
                    ));
                }
                Prop::Assign(assignment) => {
                    return Err(expression_error(
                        path,
                        original_source,
                        source_range(assignment.span, parsed_text, source_offset),
                        field_path,
                        ExpressionKind::ShorthandProperty,
                    ));
                }
                Prop::Getter(getter) => {
                    return Err(expression_error(
                        path,
                        original_source,
                        source_range(getter.span, parsed_text, source_offset),
                        field_path,
                        ExpressionKind::Getter,
                    ));
                }
                Prop::Setter(setter) => {
                    return Err(expression_error(
                        path,
                        original_source,
                        source_range(setter.span, parsed_text, source_offset),
                        field_path,
                        ExpressionKind::Setter,
                    ));
                }
                Prop::Method(method) => {
                    return Err(expression_error(
                        path,
                        original_source,
                        source_range(method.span(), parsed_text, source_offset),
                        field_path,
                        ExpressionKind::Method,
                    ));
                }
            },
        };
        if key == "__proto__" {
            return Err(error(
                path,
                original_source,
                key_span,
                &format!("{field_path}.{key}"),
                DeclarationReason::Literal(LiteralReason::PrototypeKey),
            ));
        }
        if !keys.insert(key.clone()) {
            return Err(error(
                path,
                original_source,
                key_span,
                &format!("{field_path}.{key}"),
                DeclarationReason::Literal(LiteralReason::DuplicateKey(key)),
            ));
        }
        let nested_path = format!("{field_path}.{key}");
        let parsed_value = parse_literal(
            value,
            parsed_text,
            source_offset,
            path,
            original_source,
            &nested_path,
        )?;
        properties.push(SpannedProperty {
            key,
            key_span,
            value: parsed_value,
            span: source_range(property_span, parsed_text, source_offset),
        });
    }
    Ok(properties)
}

fn property_key(
    key: &PropName,
    parsed_text: &str,
    source_offset: usize,
    path: &Path,
    original_source: &str,
    field_path: &str,
) -> Result<(String, Range<usize>), ScriptDeclarationError> {
    match key {
        PropName::Ident(ident) => Ok((
            ident.sym.to_string(),
            source_range(ident.span, parsed_text, source_offset),
        )),
        PropName::Str(string) => Ok((
            decode_string(
                string.span,
                parsed_text,
                source_offset,
                path,
                original_source,
                field_path,
            )?,
            source_range(string.span, parsed_text, source_offset),
        )),
        PropName::Num(number) => Err(error(
            path,
            original_source,
            source_range(number.span, parsed_text, source_offset),
            field_path,
            DeclarationReason::Literal(LiteralReason::KeyKind(KeyKind::Numeric)),
        )),
        PropName::BigInt(number) => Err(error(
            path,
            original_source,
            source_range(number.span, parsed_text, source_offset),
            field_path,
            DeclarationReason::Literal(LiteralReason::KeyKind(KeyKind::BigInt)),
        )),
        PropName::Computed(computed) => Err(error(
            path,
            original_source,
            source_range(computed.span, parsed_text, source_offset),
            field_path,
            DeclarationReason::Literal(LiteralReason::ComputedKey),
        )),
    }
}

fn unwrap_wrappers<'a>(
    mut expr: &'a Expr,
    parsed_text: &str,
    source_offset: usize,
    path: &Path,
    original_source: &str,
    field_path: &str,
) -> Result<(&'a Expr, Option<Range<usize>>), ScriptDeclarationError> {
    let mut full_span = None;
    loop {
        let (inner, span) = match expr {
            Expr::Paren(wrapper) => (&*wrapper.expr, wrapper.span),
            Expr::TsAs(wrapper) => (&*wrapper.expr, wrapper.span),
            Expr::TsTypeAssertion(wrapper) => (&*wrapper.expr, wrapper.span),
            Expr::TsConstAssertion(wrapper) => (&*wrapper.expr, wrapper.span),
            Expr::TsSatisfies(wrapper) => (&*wrapper.expr, wrapper.span),
            Expr::TsNonNull(wrapper) => {
                return Err(error(
                    path,
                    original_source,
                    source_range(wrapper.span, parsed_text, source_offset),
                    field_path,
                    DeclarationReason::Literal(LiteralReason::UnsupportedWrapper(
                        UnsupportedWrapper::TsNonNull,
                    )),
                ));
            }
            _ => break,
        };
        full_span.get_or_insert_with(|| source_range(span, parsed_text, source_offset));
        expr = inner;
    }
    Ok((expr, full_span))
}

fn expression_error(
    path: &Path,
    source: &str,
    span: Range<usize>,
    field_path: &str,
    kind: ExpressionKind,
) -> ScriptDeclarationError {
    error(
        path,
        source,
        span,
        field_path,
        DeclarationReason::Literal(LiteralReason::Expression(kind)),
    )
}

fn expression_kind(expr: &Expr) -> ExpressionKind {
    use deno_ast::swc::ast::{Callee, Expr as E, Lit as L};
    match expr {
        E::Ident(ident) => match ident.sym.as_ref() {
            "undefined" => ExpressionKind::Undefined,
            "NaN" => ExpressionKind::NaN,
            "Infinity" => ExpressionKind::Infinity,
            _ => ExpressionKind::Identifier,
        },
        E::Member(_) | E::SuperProp(_) | E::OptChain(_) => ExpressionKind::PropertyAccess,
        E::Call(call) if matches!(call.callee, Callee::Import(_)) => ExpressionKind::Import,
        E::Call(_) => ExpressionKind::Call,
        E::New(_) => ExpressionKind::NewExpression,
        E::Fn(_) => ExpressionKind::Function,
        E::Arrow(_) => ExpressionKind::ArrowFunction,
        E::Class(_) => ExpressionKind::Class,
        E::Tpl(_) | E::TaggedTpl(_) => ExpressionKind::Template,
        E::Lit(L::Null(_)) => ExpressionKind::Null,
        E::Lit(L::BigInt(_)) => ExpressionKind::BigInt,
        E::Lit(L::Regex(_)) => ExpressionKind::RegularExpression,
        E::Unary(unary) => {
            if unary.op == UnaryOp::Void {
                if matches!(&*unary.arg, E::Lit(L::Num(number)) if number.value == 0.0) {
                    ExpressionKind::Undefined
                } else {
                    ExpressionKind::UnaryExpression
                }
            } else {
                ExpressionKind::UnaryExpression
            }
        }
        E::Bin(_) => ExpressionKind::BinaryExpression,
        E::Cond(_) => ExpressionKind::ConditionalExpression,
        E::Assign(_) => ExpressionKind::Assignment,
        E::Seq(_) => ExpressionKind::Sequence,
        E::Await(_) => ExpressionKind::Await,
        E::Yield(_) => ExpressionKind::Yield,
        E::Update(_) => ExpressionKind::Update,
        E::This(_) => ExpressionKind::This,
        E::MetaProp(_) => ExpressionKind::Import,
        E::JSXElement(_) | E::JSXFragment(_) | E::JSXMember(_) | E::JSXNamespacedName(_) => {
            ExpressionKind::Jsx
        }
        E::Lit(_) => ExpressionKind::OtherExpression,
        E::Array(_) | E::Object(_) => ExpressionKind::OtherExpression,
        E::Paren(_)
        | E::TsAs(_)
        | E::TsTypeAssertion(_)
        | E::TsConstAssertion(_)
        | E::TsSatisfies(_) => ExpressionKind::OtherExpression,
        E::TsNonNull(_) => ExpressionKind::OtherExpression,
        E::TsInstantiation(_) | E::PrivateName(_) | E::JSXEmpty(_) | E::Invalid(_) => {
            ExpressionKind::OtherExpression
        }
    }
}

fn source_range(span: Span, parsed_text: &str, source_offset: usize) -> Range<usize> {
    let parser_start = deno_ast::StartSourcePos::START_SOURCE_POS.as_byte_pos().0 as usize;
    let start = (span.lo.0 as usize)
        .saturating_sub(parser_start)
        .saturating_add(source_offset)
        .min(parsed_text.len().saturating_add(source_offset));
    let end = (span.hi.0 as usize)
        .saturating_sub(parser_start)
        .saturating_add(source_offset)
        .min(parsed_text.len().saturating_add(source_offset));
    start..end.max(start)
}

fn decode_string(
    span: Span,
    parsed_text: &str,
    source_offset: usize,
    path: &Path,
    original_source: &str,
    field_path: &str,
) -> Result<String, ScriptDeclarationError> {
    let range = source_range(span, parsed_text, source_offset);
    let parser_start = deno_ast::StartSourcePos::START_SOURCE_POS.as_byte_pos().0 as usize;
    let start = (span.lo.0 as usize).saturating_sub(parser_start);
    let end = (span.hi.0 as usize).saturating_sub(parser_start);
    let token = parsed_text.get(start..end).unwrap_or_default();
    decode_string_token(token).ok_or_else(|| {
        error(
            path,
            original_source,
            range,
            field_path,
            DeclarationReason::Literal(LiteralReason::LoneSurrogate),
        )
    })
}

fn decode_string_token(token: &str) -> Option<String> {
    let quote = token
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))?;
    let content = token
        .strip_prefix(quote)
        .and_then(|rest| rest.strip_suffix(quote))?;
    let mut chars = content.chars().peekable();
    let mut decoded = String::with_capacity(content.len());
    let mut pending_high = None;
    while let Some(character) = chars.next() {
        if character != '\\' {
            append_scalar(&mut decoded, &mut pending_high, character)?;
            continue;
        }
        match chars.next()? {
            'u' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut value = 0_u32;
                    let mut digits = 0;
                    loop {
                        let digit = chars.next()?;
                        if digit == '}' {
                            break;
                        }
                        value = value.checked_mul(16)?.checked_add(digit.to_digit(16)?)?;
                        digits += 1;
                    }
                    if digits == 0 {
                        return None;
                    }
                    if value <= u16::MAX.into() {
                        append_unit(&mut decoded, &mut pending_high, u16::try_from(value).ok()?)?;
                    } else {
                        append_scalar(&mut decoded, &mut pending_high, char::from_u32(value)?)?;
                    }
                } else {
                    append_unit(
                        &mut decoded,
                        &mut pending_high,
                        take_hex(&mut chars, 4)? as u16,
                    )?;
                }
            }
            'x' => append_scalar(
                &mut decoded,
                &mut pending_high,
                char::from_u32(take_hex(&mut chars, 2)?)?,
            )?,
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            'b' => append_unit(&mut decoded, &mut pending_high, 8)?,
            'f' => append_unit(&mut decoded, &mut pending_high, 12)?,
            'n' => append_unit(&mut decoded, &mut pending_high, 10)?,
            'r' => append_unit(&mut decoded, &mut pending_high, 13)?,
            't' => append_unit(&mut decoded, &mut pending_high, 9)?,
            'v' => append_unit(&mut decoded, &mut pending_high, 11)?,
            '0' => append_unit(&mut decoded, &mut pending_high, 0)?,
            other => append_scalar(&mut decoded, &mut pending_high, other)?,
        }
    }
    pending_high.is_none().then_some(decoded)
}

fn append_unit(output: &mut String, pending_high: &mut Option<u16>, unit: u16) -> Option<()> {
    if (0xd800..=0xdbff).contains(&unit) {
        if pending_high.is_some() {
            return None;
        }
        *pending_high = Some(unit);
    } else if (0xdc00..=0xdfff).contains(&unit) {
        let high = u32::from(pending_high.take()?) - 0xd800;
        let low = u32::from(unit) - 0xdc00;
        output.push(char::from_u32(0x1_0000 + (high << 10) + low)?);
    } else {
        if pending_high.is_some() {
            return None;
        }
        output.push(char::from_u32(u32::from(unit))?);
    }
    Some(())
}

fn append_scalar(output: &mut String, pending_high: &mut Option<u16>, value: char) -> Option<()> {
    if pending_high.is_some() {
        return None;
    }
    output.push(value);
    Some(())
}
fn take_hex(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, count: usize) -> Option<u32> {
    let mut value = 0_u32;
    for _ in 0..count {
        let digit = chars.next()?.to_digit(16)?;
        value = value.checked_mul(16)?.checked_add(digit)?;
    }
    Some(value)
}

fn error(
    path: &Path,
    source: &str,
    span: Range<usize>,
    field_path: &str,
    reason: DeclarationReason,
) -> ScriptDeclarationError {
    ScriptDeclarationError::new(path, source, span, field_path, reason)
}
