//! A small RuneScript statement/expression parser for the door openers
//! [`super::stage_doors`] evaluates. It reads what a wall opener's control
//! flow needs — `if`/`else`, `while`, `switch_*`, `return`, `@label`
//! jumps, `def_*` bindings, calls and comparisons — and keeps every other
//! statement as [`Stmt::Other`], which the evaluator never certifies.
//! Every call a statement or expression makes is recorded, including those
//! inside arithmetic ([`Expr::Other`]), string interpolations (`"<~proc>"`)
//! and starred commands (`queue*(…)(…)`). Parsing never guesses: an
//! unbalanced or unexpected token makes the whole block unparsed.

/// One lexical token. Words keep their sigil (`~proc`, `$local`, `%varp`,
/// `^const`, `@label`, `.secondary`); coord literals (`0_50_50_10_10`)
/// are words too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tok {
    Word(String),
    Num(i32),
    /// A string literal and the calls its `<…>` interpolations make.
    Str(Vec<String>),
    Punct(&'static str),
}

/// A parsed expression. String literals carry no value (openers only
/// print them), only the calls their interpolations make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Expr {
    Word(String),
    Num(i32),
    Str(Vec<String>),
    Call(String, Vec<Expr>),
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    /// Checked integer arithmetic used by deterministic movement expressions.
    Arithmetic(ArithmeticOp, Box<Expr>, Box<Expr>),
    /// Arithmetic or another form the evaluator does not model, with every
    /// call it makes.
    Other(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArithmeticOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A parsed statement. [`Stmt::Other`] is a statement the parser could not
/// structure (a starred command, an unrecognised form); it keeps the calls
/// it saw but is never proven safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Stmt {
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>),
    Block(Vec<Stmt>),
    /// `return;` or `return(<expr>);`, with every call and `@label` the
    /// returned expression makes (they run before the script ends).
    Return(Vec<String>),
    /// A value-returning proc retains its expression for content tool selectors.
    ReturnValue(Expr),
    Jump(String, Vec<Expr>),
    Def(String, Option<Expr>),
    Assign(Vec<String>, Expr),
    Call(String, Vec<Expr>),
    While(Expr, Vec<Stmt>),
    /// `switch_*(<expr>) { case <keys> : <stmts> … }`.
    Switch(Expr, Vec<(Vec<Expr>, Vec<Stmt>)>),
    Other(Vec<String>),
}

impl Stmt {
    /// Whether the statement (recursively) contains a [`Stmt::Other`].
    pub(super) fn has_unstructured(&self) -> bool {
        match self {
            Stmt::Other(_) => true,
            Stmt::If(arms, other) => {
                arms.iter().flat_map(|(_, b)| b).any(Stmt::has_unstructured)
                    || other.iter().flatten().any(Stmt::has_unstructured)
            }
            Stmt::Block(body) | Stmt::While(_, body) => body.iter().any(Stmt::has_unstructured),
            Stmt::Switch(_, arms) => arms.iter().flat_map(|(_, b)| b).any(Stmt::has_unstructured),
            Stmt::Return(_)
            | Stmt::ReturnValue(_)
            | Stmt::Jump(..)
            | Stmt::Def(..)
            | Stmt::Assign(..)
            | Stmt::Call(..) => false,
        }
    }

    /// Every call name and `@label` jump in the statement, recursively.
    pub(super) fn calls(&self, out: &mut Vec<String>) {
        match self {
            Stmt::If(arms, other) => {
                for (cond, body) in arms {
                    expr_calls(cond, out);
                    body.iter().for_each(|s| s.calls(out));
                }
                other.iter().flatten().for_each(|s| s.calls(out));
            }
            Stmt::Block(body) => body.iter().for_each(|s| s.calls(out)),
            Stmt::Return(calls) => out.extend(calls.iter().cloned()),
            Stmt::ReturnValue(value) => expr_calls(value, out),
            Stmt::Jump(name, args) => {
                out.push(format!("@{name}"));
                args.iter().for_each(|a| expr_calls(a, out));
            }
            Stmt::Def(_, value) => value.iter().for_each(|v| expr_calls(v, out)),
            Stmt::Assign(_, value) => expr_calls(value, out),
            Stmt::Call(name, args) => {
                out.push(name.clone());
                args.iter().for_each(|a| expr_calls(a, out));
            }
            Stmt::While(cond, body) => {
                expr_calls(cond, out);
                body.iter().for_each(|s| s.calls(out));
            }
            Stmt::Switch(on, arms) => {
                expr_calls(on, out);
                for (keys, body) in arms {
                    keys.iter().for_each(|k| expr_calls(k, out));
                    body.iter().for_each(|s| s.calls(out));
                }
            }
            Stmt::Other(calls) => out.extend(calls.iter().cloned()),
        }
    }
}

fn expr_calls(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Call(name, args) => {
            out.push(name.clone());
            args.iter().for_each(|a| expr_calls(a, out));
        }
        Expr::Cmp(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Arithmetic(_, a, b) => {
            expr_calls(a, out);
            expr_calls(b, out);
        }
        // An argument-less proc call (`~proc` without parentheses).
        Expr::Word(w) if w.starts_with('~') => out.push(w.clone()),
        Expr::Str(calls) | Expr::Other(calls) => out.extend(calls.iter().cloned()),
        Expr::Word(_) | Expr::Num(_) => {}
    }
}

/// Tokens of a script body: `//` and `/* */` comments dropped, strings
/// (with `<…>` interpolations that may nest strings) one [`Tok::Str`]
/// carrying the calls of its interpolations. `None` for an unterminated
/// string, comment or interpolation.
pub(super) fn lex(text: &str) -> Option<Vec<Tok>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            let end = text[i + 2..].find("*/")?;
            i += 2 + end + 2;
        } else if c == b'"' {
            let (end, calls) = scan_string(text, i)?;
            i = end;
            out.push(Tok::Str(calls));
        } else if c.is_ascii_alphanumeric()
            || matches!(c, b'_' | b'~' | b'$' | b'%' | b'^' | b'@' | b'.')
        {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            match word.parse::<i32>() {
                Ok(n) => out.push(Tok::Num(n)),
                Err(_) => out.push(Tok::Word(word.to_string())),
            }
        } else {
            let two = if i + 1 < b.len() {
                &b[i..i + 2]
            } else {
                &b[i..i + 1]
            };
            let (p, n): (&'static str, usize) = match two {
                b">=" => (">=", 2),
                b"<=" => ("<=", 2),
                _ => match c {
                    b'(' => ("(", 1),
                    b')' => (")", 1),
                    b'{' => ("{", 1),
                    b'}' => ("}", 1),
                    b',' => (",", 1),
                    b';' => (";", 1),
                    b'=' => ("=", 1),
                    b'!' => ("!", 1),
                    b'<' => ("<", 1),
                    b'>' => (">", 1),
                    b'&' => ("&", 1),
                    b'|' => ("|", 1),
                    b'+' => ("+", 1),
                    b'-' => ("-", 1),
                    b'*' => ("*", 1),
                    b'/' => ("/", 1),
                    b':' => (":", 1),
                    _ => ("?", 1),
                },
            };
            out.push(Tok::Punct(p));
            i += n;
        }
    }
    Some(out)
}

/// Index just past the string opening at `text[start]`, and the calls its
/// `<…>` segments make. RuneScript text writes a literal `<` as `<lt>`, so
/// every `<` opens a tag or interpolation (`<p,neutral>`, `<$name>`,
/// `<~proc>`, `<text_gender("Sir", "Madam")>` with nested strings) that
/// runs to its `>`; its inside is lexed as script and its calls kept. A
/// segment or string that never closes leaves the text unlexed (`None`).
fn scan_string(text: &str, start: usize) -> Option<(usize, Vec<String>)> {
    let b = text.as_bytes();
    let mut calls = Vec::new();
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some((i + 1, calls)),
            b'<' => {
                let mut j = i + 1;
                while *b.get(j)? != b'>' {
                    j = if b[j] == b'"' {
                        scan_string(text, j)?.0
                    } else {
                        j + 1
                    };
                }
                match lex(&text[i + 1..j]) {
                    Some(toks) => calls.extend(calls_in(&toks)),
                    None => calls.push(UNPARSED.to_string()),
                }
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// The call name recorded for script text that could not be lexed; it is
/// never a known command, so its statement is never certified.
pub(super) const UNPARSED: &str = "<unparsed>";

/// The statements of a script body, or `None` when it does not parse.
pub(super) fn parse_body(text: &str) -> Option<Vec<Stmt>> {
    let toks = lex(text)?;
    let mut p = Parser { toks: &toks, i: 0 };
    let stmts = p.block_until_end()?;
    Some(stmts)
}

struct Parser<'t> {
    toks: &'t [Tok],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(q)) if *q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(q)) if q == w)
    }

    fn eat_punct(&mut self, p: &str) -> Option<()> {
        if self.is_punct(p) {
            self.i += 1;
            Some(())
        } else {
            None
        }
    }

    fn block_until_end(&mut self) -> Option<Vec<Stmt>> {
        let mut out = Vec::new();
        while self.peek().is_some() {
            out.push(self.stmt()?);
        }
        Some(out)
    }

    fn braced(&mut self) -> Option<Vec<Stmt>> {
        self.eat_punct("{")?;
        let mut out = Vec::new();
        while !self.is_punct("}") {
            self.peek()?;
            out.push(self.stmt()?);
        }
        self.i += 1;
        Some(out)
    }

    /// A braced block or a single statement (`if (…) p_opnpc(1);`).
    fn body(&mut self) -> Option<Vec<Stmt>> {
        if self.is_punct("{") {
            self.braced()
        } else {
            Some(vec![self.stmt()?])
        }
    }

    fn stmt(&mut self) -> Option<Stmt> {
        if self.is_punct("{") {
            return self.braced().map(Stmt::Block);
        }
        if self.is_punct(";") {
            self.i += 1;
            return Some(Stmt::Block(vec![]));
        }
        let Some(Tok::Word(word)) = self.peek().cloned() else {
            return None;
        };
        match word.as_str() {
            "if" => self.if_stmt(),
            "return" => {
                self.i += 1;
                let stmt = if self.is_punct("(") {
                    let (start, end) = self.balanced_group()?;
                    Stmt::ReturnValue(parse_expr(&self.toks[start..end])?)
                } else {
                    Stmt::Return(vec![])
                };
                self.eat_punct(";")?;
                Some(stmt)
            }
            "while" => {
                self.i += 1;
                let cond = self.paren_expr()?;
                let body = self.body()?;
                Some(Stmt::While(cond, body))
            }
            w if w.starts_with("switch_") => {
                self.i += 1;
                let on = self.paren_expr()?;
                self.eat_punct("{")?;
                let mut arms = Vec::new();
                while !self.is_punct("}") {
                    if !self.is_word("case") {
                        return None;
                    }
                    self.i += 1;
                    let start = self.i;
                    while !self.is_punct(":") {
                        self.peek()?;
                        self.i += 1;
                    }
                    let keys = split_args(&self.toks[start..self.i])?;
                    self.i += 1;
                    let mut body = Vec::new();
                    while !self.is_word("case") && !self.is_punct("}") {
                        self.peek()?;
                        body.push(self.stmt()?);
                    }
                    arms.push((keys, body));
                }
                self.i += 1;
                Some(Stmt::Switch(on, arms))
            }
            w if w.starts_with('@') => {
                self.i += 1;
                let args = if self.is_punct("(") {
                    self.args()?
                } else {
                    vec![]
                };
                self.eat_punct(";")?;
                Some(Stmt::Jump(w[1..].to_string(), args))
            }
            w if w.starts_with("def_") => {
                self.i += 1;
                let Some(Tok::Word(var)) = self.peek().cloned() else {
                    return None;
                };
                self.i += 1;
                if self.eat_punct(";").is_some() {
                    return Some(Stmt::Def(var, None));
                }
                self.eat_punct("=")?;
                let value = self.expr_until_semicolon()?;
                Some(Stmt::Def(var, Some(value)))
            }
            _ => self.simple(),
        }
    }

    fn if_stmt(&mut self) -> Option<Stmt> {
        let mut arms = Vec::new();
        let mut other = None;
        loop {
            // at `if`
            self.i += 1;
            let cond = self.paren_expr()?;
            let body = self.body()?;
            arms.push((cond, body));
            if !self.is_word("else") {
                break;
            }
            self.i += 1;
            if self.is_word("if") {
                continue;
            }
            other = Some(self.body()?);
            break;
        }
        Some(Stmt::If(arms, other))
    }

    /// A statement ending at a depth-0 `;`: an assignment (`lhs = expr`,
    /// `$x, $z = …`), a bare call, or anything else ([`Stmt::Other`]).
    fn simple(&mut self) -> Option<Stmt> {
        let start = self.i;
        let mut depth = 0i32;
        let mut eq_at = None;
        loop {
            match self.peek()? {
                Tok::Punct("(") | Tok::Punct("{") => depth += 1,
                Tok::Punct(")") | Tok::Punct("}") => depth -= 1,
                Tok::Punct("=") if depth == 0 && eq_at.is_none() => eq_at = Some(self.i),
                Tok::Punct(";") if depth == 0 => break,
                _ => {}
            }
            if depth < 0 {
                return None;
            }
            self.i += 1;
        }
        let end = self.i;
        self.i += 1;
        let toks = &self.toks[start..end];
        if let Some(eq) = eq_at {
            let lhs: Vec<String> = self.toks[start..eq]
                .iter()
                .filter_map(|t| match t {
                    Tok::Word(w) => Some(w.clone()),
                    _ => None,
                })
                .collect();
            let value = parse_expr(&self.toks[eq + 1..end])?;
            return Some(Stmt::Assign(lhs, value));
        }
        match parse_expr(toks) {
            Some(Expr::Call(name, args)) => Some(Stmt::Call(name, args)),
            // An argument-less command (`p_arrivedelay;`, `if_close;`).
            Some(Expr::Word(name)) => Some(Stmt::Call(name, vec![])),
            _ => Some(Stmt::Other(calls_in(toks))),
        }
    }

    fn paren_expr(&mut self) -> Option<Expr> {
        let (start, end) = self.balanced_group()?;
        parse_expr(&self.toks[start + 1..end - 1])
    }

    fn args(&mut self) -> Option<Vec<Expr>> {
        let (start, end) = self.balanced_group()?;
        split_args(&self.toks[start + 1..end - 1])
    }

    fn expr_until_semicolon(&mut self) -> Option<Expr> {
        let start = self.i;
        let mut depth = 0i32;
        loop {
            match self.peek()? {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => depth -= 1,
                Tok::Punct(";") if depth == 0 => break,
                _ => {}
            }
            self.i += 1;
        }
        let end = self.i;
        self.i += 1;
        parse_expr(&self.toks[start..end])
    }

    /// Consume one balanced `(…)` group at the cursor → its token range.
    fn balanced_group(&mut self) -> Option<(usize, usize)> {
        let start = self.i;
        self.eat_punct("(")?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek()? {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => depth -= 1,
                _ => {}
            }
            self.i += 1;
        }
        Some((start, self.i))
    }
}

/// Every call in a token run: a word followed by `(` or by `*` (a starred
/// command, `queue*(…)(…)`), every `~proc` and `@label`, and the calls of
/// every string's interpolations.
fn calls_in(toks: &[Tok]) -> Vec<String> {
    let mut out = Vec::new();
    for (k, t) in toks.iter().enumerate() {
        match t {
            Tok::Word(w)
                if w.starts_with(['@', '~'])
                    || matches!(toks.get(k + 1), Some(Tok::Punct("(" | "*"))) =>
            {
                out.push(w.clone())
            }
            Tok::Str(calls) => out.extend(calls.iter().cloned()),
            _ => {}
        }
    }
    out
}

/// Comma-separated arguments at depth 0.
fn split_args(toks: &[Tok]) -> Option<Vec<Expr>> {
    let mut out = Vec::new();
    if toks.is_empty() {
        return Some(out);
    }
    let mut depth = 0i32;
    let mut start = 0;
    for (k, t) in toks.iter().enumerate() {
        match t {
            Tok::Punct("(") => depth += 1,
            Tok::Punct(")") => depth -= 1,
            Tok::Punct(",") if depth == 0 => {
                out.push(parse_expr(&toks[start..k])?);
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(parse_expr(&toks[start..])?);
    Some(out)
}

/// An expression: `|` binds loosest, then `&`, then one comparison.
pub(super) fn parse_expr(toks: &[Tok]) -> Option<Expr> {
    if toks.is_empty() {
        return None;
    }
    if let Some(k) = split_at_depth0(toks, &["|"]) {
        return Some(Expr::Or(
            Box::new(parse_expr(&toks[..k])?),
            Box::new(parse_expr(&toks[k + 1..])?),
        ));
    }
    if let Some(k) = split_at_depth0(toks, &["&"]) {
        return Some(Expr::And(
            Box::new(parse_expr(&toks[..k])?),
            Box::new(parse_expr(&toks[k + 1..])?),
        ));
    }
    if let Some(k) = split_at_depth0(toks, &["=", "!", "<", ">", "<=", ">="]) {
        let op = match toks[k] {
            Tok::Punct("=") => CmpOp::Eq,
            Tok::Punct("!") => CmpOp::Ne,
            Tok::Punct("<") => CmpOp::Lt,
            Tok::Punct(">") => CmpOp::Gt,
            Tok::Punct("<=") => CmpOp::Le,
            _ => CmpOp::Ge,
        };
        return Some(Expr::Cmp(
            op,
            Box::new(parse_expr(&toks[..k])?),
            Box::new(parse_expr(&toks[k + 1..])?),
        ));
    }
    for ops in [&["+", "-"][..], &["*", "/", "%"][..]] {
        if let Some(k) = arithmetic_split(toks, ops) {
            let op = match toks[k] {
                Tok::Punct("+") => ArithmeticOp::Add,
                Tok::Punct("-") => ArithmeticOp::Subtract,
                Tok::Punct("*") => ArithmeticOp::Multiply,
                Tok::Punct("/") => ArithmeticOp::Divide,
                _ => ArithmeticOp::Modulo,
            };
            return Some(Expr::Arithmetic(
                op,
                Box::new(parse_expr(&toks[..k])?),
                Box::new(parse_expr(&toks[k + 1..])?),
            ));
        }
    }
    primary(toks)
}

/// The first depth-0 token in `ops` (left to right).
fn split_at_depth0(toks: &[Tok], ops: &[&str]) -> Option<usize> {
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate() {
        match t {
            Tok::Punct("(") => depth += 1,
            Tok::Punct(")") => depth -= 1,
            Tok::Punct(p) if depth == 0 && ops.contains(p) => return Some(k),
            _ => {}
        }
    }
    None
}

/// Rightmost binary operator keeps arithmetic left-associative. A leading
/// sign or a sign following another operator is unary, not a split.
fn arithmetic_split(toks: &[Tok], ops: &[&str]) -> Option<usize> {
    let mut depth = 0i32;
    for k in (0..toks.len()).rev() {
        match &toks[k] {
            Tok::Punct(")") => depth += 1,
            Tok::Punct("(") => depth -= 1,
            Tok::Punct(op) if depth == 0 && ops.contains(op) && k > 0 => {
                if matches!(&toks[k - 1], Tok::Word(_) | Tok::Num(_) | Tok::Punct(")")) {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

fn primary(toks: &[Tok]) -> Option<Expr> {
    match toks {
        [Tok::Punct("("), inner @ .., Tok::Punct(")")] if balanced(inner) => parse_expr(inner),
        [Tok::Num(n)] => Some(Expr::Num(*n)),
        [Tok::Punct("-"), Tok::Num(n)] => Some(Expr::Num(-n)),
        [Tok::Str(calls)] => Some(Expr::Str(calls.clone())),
        [Tok::Word(w)] => Some(Expr::Word(w.clone())),
        [Tok::Word(w), Tok::Punct("("), inner @ .., Tok::Punct(")")] if balanced(inner) => {
            Some(Expr::Call(w.clone(), split_args(inner)?))
        }
        _ if balanced(toks) => Some(Expr::Other(calls_in(toks))),
        _ => None,
    }
}

fn balanced(toks: &[Tok]) -> bool {
    let mut depth = 0i32;
    for t in toks {
        match t {
            Tok::Punct("(") => depth += 1,
            Tok::Punct(")") => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}
