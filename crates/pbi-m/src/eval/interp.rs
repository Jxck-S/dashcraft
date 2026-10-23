//! The tree-walking interpreter.

use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::{Arc, Weak};

use chrono::{DateTime, FixedOffset, Utc};

use crate::ast::{self, BinaryOp, Intrinsic, NullablePrimitiveType, PrimitiveType};
use crate::eval::EvalOptions;
use crate::eval::env::{Env, Frame, Lookup};
use crate::eval::ir::{
    Bindings, Compiler, FieldSpecIr, ListItemIr, NodeKind, NodeRef, SourceText, TryIr, TypeNode,
};
use crate::eval::thunk::{Pending, Thunk};
use crate::merror::{LimitKind, MError};
use crate::span::line_col;
use crate::stdlib::{self, Library};
use crate::value::format::describe;
use crate::value::ops::{self, Forcer};
use crate::value::types::primitive_conforms;
use crate::value::{
    FieldType, FunctionKind, FunctionType, FunctionValue, Keys, Lazy, ListValue, NativeImpl,
    ParamType, RecordType, RecordValue, TableValue, TypeKind, TypeValue, Value, ValueKind,
};

struct Registry {
    frames: Vec<Weak<Frame>>,
    next_prune: usize,
}

const PRUNE_MIN: usize = 1024;

/// Evaluation state for one engine call. Lives on the evaluator thread only.
pub(crate) struct Interp {
    pub(crate) opts: EvalOptions,
    steps: Cell<u64>,
    depth: Cell<usize>,
    registry: RefCell<Registry>,
    root: RefCell<Option<Env>>,
    sections: RefCell<Vec<(Arc<str>, Env)>>,
    library: &'static Library,
    /// `DateTimeZone.UtcNow()` / `FixedUtcNow()` for this evaluation.
    pub(crate) now: DateTime<Utc>,
    pub(crate) local_offset: FixedOffset,
}

pub(crate) struct DepthGuard<'a>(&'a Interp);

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.0.depth.set(self.0.depth.get().saturating_sub(1));
    }
}

impl Drop for Interp {
    fn drop(&mut self) {
        self.release();
    }
}

impl Forcer for Interp {
    fn force_slot(&self, slot: &Lazy) -> Result<Value, MError> {
        slot.force(self)
    }

    fn binary_limit(&self) -> u64 {
        self.opts.limits.max_binary_bytes
    }
}

fn is_fatal(e: &MError) -> bool {
    matches!(
        e,
        MError::LimitExceeded { .. } | MError::Cancelled | MError::Internal(_)
    )
}

/// `We cannot convert the value X to type T.`
pub(crate) fn convert_error(v: &Value, target: &str) -> MError {
    MError::expression(format!(
        "We cannot convert the value {} to type {target}.",
        describe(v)
    ))
}

fn npt_conforms(t: &NullablePrimitiveType, v: &Value) -> bool {
    if v.kind() == ValueKind::Null {
        return t.nullable || matches!(t.primitive, PrimitiveType::Any | PrimitiveType::Null);
    }
    primitive_conforms(t.primitive, v.kind())
}

fn npt_name(t: &NullablePrimitiveType) -> String {
    let base = TypeValue::primitive(t.primitive).message_name();
    if t.nullable {
        format!("nullable {base}")
    } else {
        base
    }
}

fn integer(v: &Value) -> Result<i64, MError> {
    match v.plain() {
        Value::Number(n) if n.fract() == 0.0 && n.abs() <= 9.007_199_254_740_992e15 => {
            Ok(*n as i64)
        }
        other => Err(MError::expression(format!(
            "We cannot convert the value {} to type Int64.",
            describe(other)
        ))),
    }
}

impl Interp {
    pub(crate) fn new(opts: EvalOptions) -> Interp {
        let now = opts.clock.now_utc();
        let local_offset = opts.clock.local_offset();
        Interp {
            opts,
            steps: Cell::new(0),
            depth: Cell::new(0),
            registry: RefCell::new(Registry {
                frames: Vec::new(),
                next_prune: PRUNE_MIN,
            }),
            root: RefCell::new(None),
            sections: RefCell::new(Vec::new()),
            library: stdlib::library(),
            now,
            local_offset,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Limits
    // -----------------------------------------------------------------------------------------

    /// Counts one evaluation step and checks the step limit and the cancellation flag.
    pub(crate) fn tick(&self) -> Result<(), MError> {
        let steps = self.steps.get().saturating_add(1);
        self.steps.set(steps);
        if steps > self.opts.limits.max_steps {
            return Err(MError::limit(LimitKind::Steps, self.opts.limits.max_steps));
        }
        if self.opts.cancel.load(AtomicOrdering::Relaxed) {
            return Err(MError::Cancelled);
        }
        Ok(())
    }

    pub(crate) fn enter(&self) -> Result<DepthGuard<'_>, MError> {
        self.tick()?;
        let d = self.depth.get() + 1;
        if d > self.opts.limits.max_depth {
            return Err(MError::limit(
                LimitKind::Depth,
                self.opts.limits.max_depth as u64,
            ));
        }
        self.depth.set(d);
        Ok(DepthGuard(self))
    }

    pub(crate) fn check_rows(&self, n: usize) -> Result<(), MError> {
        if n > self.opts.limits.max_rows {
            return Err(MError::limit(
                LimitKind::Rows,
                self.opts.limits.max_rows as u64,
            ));
        }
        Ok(())
    }

    pub(crate) fn check_bytes(&self, n: usize) -> Result<(), MError> {
        if u64::try_from(n).unwrap_or(u64::MAX) > self.opts.limits.max_binary_bytes {
            return Err(MError::limit(
                LimitKind::Bytes,
                self.opts.limits.max_binary_bytes,
            ));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Frames
    // -----------------------------------------------------------------------------------------

    fn register(&self, frame: &Arc<Frame>) {
        let mut r = self.registry.borrow_mut();
        r.frames.push(Arc::downgrade(frame));
        if r.frames.len() >= r.next_prune {
            r.frames.retain(|w| w.strong_count() > 0);
            r.next_prune = (r.frames.len() * 2).max(PRUNE_MIN);
        }
    }

    /// Clears every registered frame, breaking reference cycles. Values reachable from the
    /// result stay valid as long as they were resolved first.
    pub(crate) fn release(&self) {
        let frames = std::mem::take(&mut self.registry.borrow_mut().frames);
        for w in frames {
            if let Some(f) = w.upgrade() {
                f.clear();
            }
        }
        self.root.borrow_mut().take();
        self.sections.borrow_mut().clear();
    }

    fn slot(&self, node: &NodeRef, env: Env) -> Lazy {
        match &node.kind {
            NodeKind::Const(v) => Lazy::Value(v.clone()),
            NodeKind::Function(def) => {
                Lazy::Value(Value::Function(FunctionValue::closure(def.clone(), env)))
            }
            _ => Lazy::Thunk(Thunk::new(Pending::Expr(env, node.clone()))),
        }
    }

    /// Creates a registered frame for `let`/record bindings and returns its slots.
    fn bind(&self, b: &Bindings, parent: &Env) -> (Env, Vec<Lazy>) {
        let frame = Frame::new(b.names.clone(), Vec::new(), Some(parent.clone()));
        let values: Vec<Lazy> = b
            .values
            .iter()
            .enumerate()
            .map(|(i, node)| self.slot(node, Env::member(&frame, i)))
            .collect();
        frame.set_values(values.clone());
        self.register(&frame);
        (Env::new(frame), values)
    }

    /// Installs the global environment: shared queries / parameters, evaluated lazily.
    pub(crate) fn set_shared(&self, shared: &indexmap::IndexMap<String, String>) -> Env {
        let names: Arc<[Arc<str>]> = shared.keys().map(|k| Arc::from(k.as_str())).collect();
        let values = shared
            .iter()
            .map(|(name, source)| {
                Lazy::Thunk(Thunk::new(Pending::Shared {
                    name: Arc::from(name.as_str()),
                    source: Arc::from(source.as_str()),
                }))
            })
            .collect();
        let frame = Frame::new(names, values, None);
        self.register(&frame);
        let env = Env::new(frame);
        *self.root.borrow_mut() = Some(env.clone());
        env
    }

    fn root_env(&self) -> Env {
        if let Some(e) = self.root.borrow().as_ref() {
            return e.clone();
        }
        let frame = Frame::new(Arc::from(Vec::new()), Vec::new(), None);
        let env = Env::new(frame);
        *self.root.borrow_mut() = Some(env.clone());
        env
    }

    pub(crate) fn run_pending(&self, p: Pending) -> Result<Value, MError> {
        match p {
            Pending::Expr(env, node) => self.eval(&node, &env),
            Pending::Shared { name, source } => {
                let expr = crate::parse_expression(&source).map_err(|e| {
                    let (line, column) = line_col(&source, e.span.start);
                    MError::Syntax {
                        query: Some(name.to_string()),
                        message: e.message.clone(),
                        span: e.span,
                        line,
                        column,
                    }
                })?;
                let src = Arc::new(SourceText {
                    name: Some(name),
                    text: source,
                });
                let node = Compiler::new(src).expr(&expr);
                self.eval(&node, &self.root_env())
            }
        }
    }

    /// Evaluates a section document: returns the record of its members.
    pub(crate) fn eval_section(
        &self,
        section: &ast::Section,
        src: Arc<SourceText>,
    ) -> Result<Value, MError> {
        let compiler = Compiler::new(src);
        let names: Arc<[Arc<str>]> = section
            .members
            .iter()
            .map(|m| Arc::from(m.name.text.as_str()))
            .collect();
        let keys = Arc::new(Keys::try_new(names.to_vec())?);
        let frame = Frame::new(names, Vec::new(), Some(self.root_env()));
        let env = Env::new(frame.clone());
        let values: Vec<Lazy> = section
            .members
            .iter()
            .map(|m| self.slot(&compiler.expr(&m.value), env.clone()))
            .collect();
        frame.set_values(values.clone());
        self.register(&frame);
        self.sections
            .borrow_mut()
            .push((Arc::from(section.name.text.as_str()), env));
        Ok(Value::Record(RecordValue::new(keys, values)))
    }

    // -----------------------------------------------------------------------------------------
    // Evaluation
    // -----------------------------------------------------------------------------------------

    pub(crate) fn eval(&self, node: &NodeRef, env: &Env) -> Result<Value, MError> {
        let _guard = self.enter()?;
        self.eval_node(node, env)
            .map_err(|e| e.or_location(|| node.location()))
    }

    fn eval_node(&self, node: &NodeRef, env: &Env) -> Result<Value, MError> {
        match &node.kind {
            NodeKind::Const(v) => Ok(v.clone()),
            NodeKind::Verbatim => Err(MError::expression(
                "The expression contains verbatim (#!\"...\") text and cannot be evaluated.",
            )),
            NodeKind::Ident { name, inclusive } => self.lookup(name, *inclusive, env)?.force(self),
            NodeKind::Intrinsic(i) => self.intrinsic(*i),
            NodeKind::SectionAccess { section, member } => {
                let env = self
                    .sections
                    .borrow()
                    .iter()
                    .find(|(n, _)| n == section)
                    .map(|(_, e)| e.clone());
                let Some(env) = env else {
                    return Err(MError::expression(format!(
                        "The section '{section}' wasn't found."
                    )));
                };
                let (names, values) = env.frame().entries();
                match names.iter().position(|n| n == member) {
                    Some(i) => match values.get(i) {
                        Some(v) => v.force(self),
                        None => Err(MError::Internal("section member missing".into())),
                    },
                    None => Err(MError::expression(format!(
                        "The member '{member}' of section '{section}' wasn't found."
                    ))),
                }
            }
            NodeKind::Let(b, body) => {
                if let Err(dup) = &b.keys {
                    return Err(MError::expression(format!(
                        "The variable '{dup}' is defined more than once."
                    )));
                }
                let (child, _) = self.bind(b, env);
                self.eval(body, &child)
            }
            NodeKind::If(c, t, e) => {
                let cond = self.eval(c, env)?;
                match cond.plain() {
                    Value::Logical(true) => self.eval(t, env),
                    Value::Logical(false) => self.eval(e, env),
                    other => Err(convert_error(other, "Logical")),
                }
            }
            NodeKind::Function(def) => Ok(Value::Function(FunctionValue::closure(
                def.clone(),
                env.clone(),
            ))),
            NodeKind::Invoke(f, args) => {
                let fv = self.eval(f, env)?;
                let Value::Function(func) = fv.plain() else {
                    return Err(MError::expression(format!(
                        "We cannot convert a value of type {} to type Function.",
                        fv.kind().name()
                    )));
                };
                let func = func.clone();
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.eval(a, env)?);
                }
                self.invoke(&func, argv)
            }
            NodeKind::Record(b) => {
                let keys = b.keys.clone().map_err(|d| {
                    MError::expression(format!("The field '{d}' already exists in the record."))
                })?;
                let (_, values) = self.bind(b, env);
                Ok(Value::Record(RecordValue::new(keys, values)))
            }
            NodeKind::List(items) => self.eval_list(items, env),
            NodeKind::Field {
                target,
                field,
                optional,
            } => {
                let t = match target {
                    Some(n) => self.eval(n, env)?,
                    None => self.underscore(env)?,
                };
                self.field_access(&t, field, *optional)
            }
            NodeKind::Project {
                target,
                fields,
                optional,
            } => {
                let t = match target {
                    Some(n) => self.eval(n, env)?,
                    None => self.underscore(env)?,
                };
                self.project(&t, fields, *optional)
            }
            NodeKind::Item {
                target,
                selector,
                optional,
            } => {
                let t = self.eval(target, env)?;
                let s = self.eval(selector, env)?;
                self.item_access(&t, &s, *optional)
            }
            NodeKind::Unary(op, x) => ops::unary(*op, &self.eval(x, env)?),
            NodeKind::Binary(op, l, r) => self.eval_binary(*op, l, r, env),
            NodeKind::As(x, t) => {
                let v = self.eval(x, env)?;
                if npt_conforms(t, &v) {
                    Ok(v)
                } else {
                    Err(convert_error(&v, &npt_name(t)))
                }
            }
            NodeKind::Is(x, t) => Ok(Value::Logical(npt_conforms(t, &self.eval(x, env)?))),
            NodeKind::Type(t) => Ok(Value::Type(self.eval_type(t, env)?)),
            NodeKind::Try(x, handler) => self.eval_try(x, handler.as_ref(), env),
            NodeKind::Error(x) => {
                let v = self.eval(x, env)?;
                Err(self.error_from_value(&v)?)
            }
            NodeKind::NotImplemented => Err(MError::expression("Not Implemented")),
        }
    }

    fn lookup(&self, name: &str, inclusive: bool, env: &Env) -> Result<Lazy, MError> {
        let found = env.lookup(name, inclusive);
        if let Lookup::Found(v) = found {
            return Ok(v);
        }
        if let Some(v) = self.library.get(name) {
            return Ok(Lazy::Value(v.clone()));
        }
        if let Lookup::SelfOnly(v) = found {
            return Ok(v);
        }
        Err(stdlib::unknown_name(name))
    }

    fn underscore(&self, env: &Env) -> Result<Value, MError> {
        match env.lookup("_", true) {
            Lookup::Found(v) | Lookup::SelfOnly(v) => v.force(self),
            Lookup::Missing => Err(MError::expression(
                "The name '_' wasn't recognized: implicit field access `[x]` is only valid inside `each`.",
            )),
        }
    }

    fn intrinsic(&self, i: Intrinsic) -> Result<Value, MError> {
        match i {
            Intrinsic::Infinity => Ok(Value::Number(f64::INFINITY)),
            Intrinsic::Nan => Ok(Value::Number(f64::NAN)),
            Intrinsic::Shared => {
                let (names, values) = self.root_env().frame().entries();
                let mut pairs: Vec<(Arc<str>, Lazy)> = names.iter().cloned().zip(values).collect();
                for (n, v) in self.library.entries() {
                    if !n.starts_with('#') && !pairs.iter().any(|(p, _)| &**p == n) {
                        pairs.push((Arc::from(n), Lazy::Value(v.clone())));
                    }
                }
                Ok(Value::Record(RecordValue::from_lazy_pairs(pairs)?))
            }
            Intrinsic::Sections => {
                let sections = self.sections.borrow().clone();
                let mut pairs = Vec::new();
                for (name, env) in sections {
                    let (names, values) = env.frame().entries();
                    let rec =
                        RecordValue::from_lazy_pairs(names.iter().cloned().zip(values).collect())?;
                    pairs.push((name, Lazy::Value(Value::Record(rec))));
                }
                Ok(Value::Record(RecordValue::from_lazy_pairs(pairs)?))
            }
            other => self.library.get(other.keyword()).cloned().ok_or_else(|| {
                MError::unsupported(other.keyword(), crate::merror::LOAD_MANUALLY_HINT)
            }),
        }
    }

    fn eval_list(&self, items: &[ListItemIr], env: &Env) -> Result<Value, MError> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item {
                ListItemIr::Single(n) => out.push(self.slot(n, env.clone())),
                ListItemIr::Range(s, e) => {
                    let start = integer(&self.eval(s, env)?)?;
                    let end = integer(&self.eval(e, env)?)?;
                    if end >= start {
                        let count = usize::try_from(end.saturating_sub(start))
                            .unwrap_or(usize::MAX)
                            .saturating_add(1);
                        self.check_rows(out.len().saturating_add(count))?;
                        out.reserve(count);
                        for k in start..=end {
                            out.push(Lazy::Value(Value::Number(k as f64)));
                        }
                    }
                }
            }
        }
        self.check_rows(out.len())?;
        Ok(Value::List(ListValue::new(out)))
    }

    fn eval_binary(
        &self,
        op: BinaryOp,
        l: &NodeRef,
        r: &NodeRef,
        env: &Env,
    ) -> Result<Value, MError> {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                let a = self.eval(l, env)?;
                let a = match a.plain() {
                    Value::Logical(b) => Some(*b),
                    Value::Null => None,
                    other => return Err(convert_error(other, "Logical")),
                };
                let short = if op == BinaryOp::And {
                    Some(false)
                } else {
                    Some(true)
                };
                if a == short {
                    return Ok(Value::Logical(a == Some(true)));
                }
                let b = self.eval(r, env)?;
                let b = match b.plain() {
                    Value::Logical(b) => Some(*b),
                    Value::Null => None,
                    other => return Err(convert_error(other, "Logical")),
                };
                if b == short {
                    return Ok(Value::Logical(b == Some(true)));
                }
                // Both operands are now the non-short-circuiting value or null.
                Ok(match (a, b) {
                    (Some(x), Some(_)) => Value::Logical(x),
                    _ => Value::Null,
                })
            }
            BinaryOp::Coalesce => {
                let a = self.eval(l, env)?;
                if a.is_null() {
                    self.eval(r, env)
                } else {
                    Ok(a)
                }
            }
            BinaryOp::Meta => {
                let v = self.eval(l, env)?;
                let m = self.eval(r, env)?;
                match m.plain() {
                    Value::Record(rec) => Ok(v.with_meta(rec.clone())),
                    other => Err(convert_error(other, "Record")),
                }
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                let a = self.eval(l, env)?;
                let b = self.eval(r, env)?;
                let eq = ops::equals(self, &a, &b, 0)?;
                Ok(Value::Logical(if op == BinaryOp::Equal { eq } else { !eq }))
            }
            BinaryOp::LessThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThan
            | BinaryOp::GreaterThanOrEqual => {
                let a = self.eval(l, env)?;
                let b = self.eval(r, env)?;
                ops::relational(op, &a, &b)
            }
            BinaryOp::Concatenate => {
                let a = self.eval(l, env)?;
                let b = self.eval(r, env)?;
                match (a.plain(), b.plain()) {
                    (Value::Text(x), Value::Text(y)) => self.check_bytes(x.len() + y.len())?,
                    (Value::List(x), Value::List(y)) => self.check_rows(x.len() + y.len())?,
                    (Value::Table(x), Value::Table(y)) => {
                        self.check_rows(x.row_count() + y.row_count())?;
                    }
                    _ => {}
                }
                ops::arithmetic(op, &a, &b)
            }
            BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                let a = self.eval(l, env)?;
                let b = self.eval(r, env)?;
                ops::arithmetic(op, &a, &b)
            }
        }
    }

    fn eval_try(&self, x: &NodeRef, handler: Option<&TryIr>, env: &Env) -> Result<Value, MError> {
        match self.eval(x, env) {
            Ok(v) => match handler {
                None => Ok(Value::Record(RecordValue::from_pairs([
                    ("HasError", Value::Logical(false)),
                    ("Value", v),
                ])?)),
                Some(_) => Ok(v),
            },
            Err(e) if e.is_catchable() => {
                let rec = error_record(&e)?;
                match handler {
                    None => Ok(Value::Record(RecordValue::from_pairs([
                        ("HasError", Value::Logical(true)),
                        ("Error", Value::Record(rec)),
                    ])?)),
                    Some(TryIr::Otherwise(d)) => self.eval(d, env),
                    Some(TryIr::Catch(param, body)) => {
                        let child = match param {
                            Some(p) => env.child(
                                Arc::from(vec![p.clone()]),
                                vec![Lazy::Value(Value::Record(rec))],
                            ),
                            None => env.clone(),
                        };
                        self.eval(body, &child)
                    }
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Builds the error raised by `error value`.
    fn error_from_value(&self, v: &Value) -> Result<MError, MError> {
        match v.plain() {
            Value::Text(t) => Ok(MError::expression(t.to_string())),
            Value::Record(r) => {
                let text = |name: &str| -> Result<Option<String>, MError> {
                    match r.get(name) {
                        Some(slot) => Ok(slot.force(self)?.as_text().map(str::to_owned)),
                        None => Ok(None),
                    }
                };
                let reason = text("Reason")?.unwrap_or_else(|| "Expression.Error".to_owned());
                let message = text("Message")?.unwrap_or_default();
                let detail = match r.get("Detail") {
                    Some(slot) => {
                        let d = slot.force(self)?;
                        if d.is_null() { None } else { Some(Box::new(d)) }
                    }
                    None => None,
                };
                Ok(MError::Evaluation {
                    reason,
                    message,
                    detail,
                    location: None,
                })
            }
            other => Err(convert_error(other, "Record")),
        }
    }

    pub(crate) fn field_access(
        &self,
        t: &Value,
        field: &str,
        optional: bool,
    ) -> Result<Value, MError> {
        match t.plain() {
            Value::Record(r) => match r.get(field) {
                Some(slot) => slot.force(self),
                None if optional => Ok(Value::Null),
                None => Err(MError::expression(format!(
                    "The field '{field}' of the record wasn't found."
                ))),
            },
            Value::Table(tb) => match tb.column_by_name(field) {
                Some(cells) => Ok(Value::List(ListValue::new(cells.to_vec()))),
                None if optional => Ok(Value::Null),
                None => Err(stdlib::missing_column(field)),
            },
            other => Err(MError::expression(format!(
                "We cannot apply field access to the type {}.",
                other.kind().name()
            ))),
        }
    }

    fn project(&self, t: &Value, fields: &[Arc<str>], optional: bool) -> Result<Value, MError> {
        match t.plain() {
            Value::Record(r) => {
                let mut values = Vec::with_capacity(fields.len());
                for f in fields {
                    match r.get(f) {
                        Some(v) => values.push(v.clone()),
                        None if optional => values.push(Lazy::Value(Value::Null)),
                        None => {
                            return Err(MError::expression(format!(
                                "The field '{f}' of the record wasn't found."
                            )));
                        }
                    }
                }
                let keys = Keys::try_new(fields.to_vec())?;
                Ok(Value::Record(RecordValue::new(Arc::new(keys), values)))
            }
            Value::Table(tb) => {
                let missing = if optional {
                    stdlib::MissingField::UseNull
                } else {
                    stdlib::MissingField::Error
                };
                Ok(Value::Table(stdlib::table::select_columns(
                    tb, fields, missing,
                )?))
            }
            other => Err(MError::expression(format!(
                "We cannot apply field access to the type {}.",
                other.kind().name()
            ))),
        }
    }

    pub(crate) fn item_access(
        &self,
        t: &Value,
        s: &Value,
        optional: bool,
    ) -> Result<Value, MError> {
        let not_enough = || {
            MError::expression(
                "There weren't enough elements in the enumeration to complete the operation.",
            )
        };
        let index = |s: &Value| -> Result<Option<usize>, MError> {
            match s.plain() {
                Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 => {
                    Ok(usize::try_from(*n as u64).ok())
                }
                Value::Number(n) if n.fract() == 0.0 => Ok(None),
                other => Err(convert_error(other, "Number")),
            }
        };
        match (t.plain(), s.plain()) {
            (Value::List(l), _) => match index(s)?.and_then(|i| l.items().get(i)) {
                Some(slot) => slot.force(self),
                None if optional => Ok(Value::Null),
                None => Err(not_enough()),
            },
            (Value::Table(tb), Value::Record(key)) => self.table_key_lookup(tb, key, optional),
            (Value::Table(tb), _) => match index(s)?.and_then(|i| tb.row(i)) {
                Some(row) => Ok(Value::Record(row)),
                None if optional => Ok(Value::Null),
                None => Err(not_enough()),
            },
            (other, _) => Err(MError::expression(format!(
                "We cannot apply item access to the type {}.",
                other.kind().name()
            ))),
        }
    }

    fn table_key_lookup(
        &self,
        tb: &TableValue,
        key: &RecordValue,
        optional: bool,
    ) -> Result<Value, MError> {
        let mut criteria = Vec::with_capacity(key.len());
        for (name, slot) in key.iter() {
            let ci = tb
                .column_index(name)
                .ok_or_else(|| stdlib::missing_column(name))?;
            criteria.push((ci, slot.force(self)?));
        }
        let mut found = None;
        for row in 0..tb.row_count() {
            self.tick()?;
            let mut all = true;
            for (ci, expected) in &criteria {
                let cell = match tb.cell(row, *ci) {
                    Some(c) => c.force(self)?,
                    None => Value::Null,
                };
                if !ops::equals(self, &cell, expected, 0)? {
                    all = false;
                    break;
                }
            }
            if all {
                if found.is_some() {
                    return Err(MError::expression(
                        "The key matched more than one row in the table.",
                    ));
                }
                found = Some(row);
            }
        }
        match found.and_then(|r| tb.row(r)) {
            Some(row) => Ok(Value::Record(row)),
            None if optional => Ok(Value::Null),
            None => Err(MError::expression(
                "The key didn't match any rows in the table.",
            )),
        }
    }

    fn type_fields(&self, fields: &[FieldSpecIr], env: &Env) -> Result<Vec<FieldType>, MError> {
        fields
            .iter()
            .map(|f| {
                Ok(FieldType {
                    name: f.name.clone(),
                    ty: match &f.ty {
                        Some(t) => self.eval_type(t, env)?,
                        None => TypeValue::any(),
                    },
                    optional: f.optional,
                })
            })
            .collect()
    }

    fn eval_type(&self, t: &TypeNode, env: &Env) -> Result<TypeValue, MError> {
        Ok(match t {
            TypeNode::Primitive(p) => TypeValue::primitive(*p),
            TypeNode::Nullable(inner) => self.eval_type(inner, env)?.nullable(),
            TypeNode::Record(fields, open) => TypeValue::new(TypeKind::Record(RecordType {
                fields: self.type_fields(fields, env)?,
                open: *open,
            })),
            TypeNode::List(item) => TypeValue::new(TypeKind::List(self.eval_type(item, env)?)),
            TypeNode::Function(params, ret) => TypeValue::new(TypeKind::Function(FunctionType {
                params: self
                    .type_fields(params, env)?
                    .into_iter()
                    .map(|f| ParamType {
                        name: f.name,
                        ty: f.ty,
                        optional: f.optional,
                    })
                    .collect(),
                ret: self.eval_type(ret, env)?,
            })),
            TypeNode::Table(fields) => TypeValue::table(
                self.type_fields(fields, env)?
                    .into_iter()
                    .map(|f| (f.name, f.ty))
                    .collect(),
            ),
            TypeNode::TableExpr(e) => {
                let v = self.eval(e, env)?;
                match v.as_type().map(TypeValue::kind) {
                    Some(TypeKind::Record(r)) => TypeValue::table(
                        r.fields
                            .iter()
                            .map(|f| (f.name.clone(), f.ty.clone()))
                            .collect(),
                    ),
                    Some(TypeKind::Table(_)) => v.as_type().cloned().unwrap_or_else(TypeValue::any),
                    _ => return Err(convert_error(&v, "Type")),
                }
            }
            TypeNode::Expr(e) => {
                let v = self.eval(e, env)?;
                match v.as_type() {
                    Some(t) => t.clone(),
                    None => return Err(convert_error(&v, "Type")),
                }
            }
        })
    }

    // -----------------------------------------------------------------------------------------
    // Invocation
    // -----------------------------------------------------------------------------------------

    pub(crate) fn invoke(&self, f: &FunctionValue, args: Vec<Value>) -> Result<Value, MError> {
        let (min, max) = f.arity();
        if args.len() < min || args.len() > max {
            let expects = if min == max {
                format!("{min}")
            } else {
                format!("between {min} and {max}")
            };
            let name = f.name().map(|n| format!(" ({n})")).unwrap_or_default();
            return Err(MError::expression(format!(
                "{} arguments were passed to a function{name} which expects {expects}.",
                args.len()
            )));
        }
        match &*f.0 {
            FunctionKind::Closure { def, env } => {
                let mut values = Vec::with_capacity(def.params.len());
                let mut args = args.into_iter();
                for p in &def.params {
                    let v = args.next().unwrap_or(Value::Null);
                    if let Some(t) = &p.ty
                        && !npt_conforms(t, &v)
                    {
                        return Err(convert_error(&v, &npt_name(t)));
                    }
                    values.push(Lazy::Value(v));
                }
                let child = env.child(def.names.clone(), values);
                let r = self.eval(&def.body, &child)?;
                if let Some(t) = &def.ret
                    && !npt_conforms(t, &r)
                {
                    return Err(convert_error(&r, &npt_name(t)));
                }
                Ok(r)
            }
            FunctionKind::Native(n) => {
                let mut a: Vec<Value> = if n.raw_args {
                    args
                } else {
                    args.into_iter().map(Value::into_plain).collect()
                };
                a.resize(n.params.len(), Value::Null);
                match &n.imp {
                    NativeImpl::Static(func) => func(self, &a),
                    NativeImpl::Dyn(func) => func(self, &a),
                }
            }
        }
    }

    /// Invokes a function value with arguments (for library functions taking callbacks).
    pub(crate) fn call(&self, f: &FunctionValue, args: &[Value]) -> Result<Value, MError> {
        let _guard = self.enter()?;
        self.invoke(f, args.to_vec())
    }

    // -----------------------------------------------------------------------------------------
    // Resolution of results
    // -----------------------------------------------------------------------------------------

    fn resolve_slot(&self, slot: &Lazy) -> Result<Lazy, MError> {
        match slot.force(self) {
            Ok(v) => Ok(Lazy::Value(self.resolve(v)?)),
            Err(e) if is_fatal(&e) => Err(e),
            Err(e) => Ok(Lazy::Error(Arc::new(e))),
        }
    }

    fn needs_resolve(slot: &Lazy) -> bool {
        match slot {
            Lazy::Thunk(_) => true,
            Lazy::Error(_) => false,
            Lazy::Value(v) => matches!(
                v,
                Value::List(_) | Value::Record(_) | Value::Table(_) | Value::Meta(_)
            ),
        }
    }

    fn resolve_slots(&self, slots: &[Lazy]) -> Result<Option<Vec<Lazy>>, MError> {
        if !slots.iter().any(Self::needs_resolve) {
            return Ok(None);
        }
        let mut out = Vec::with_capacity(slots.len());
        for s in slots {
            out.push(if Self::needs_resolve(s) {
                self.resolve_slot(s)?
            } else {
                s.clone()
            });
        }
        Ok(Some(out))
    }

    /// Evaluates every slot reachable from `v` (outside function bodies), so the value can be
    /// inspected without the interpreter. Errors in slots are kept as slot errors.
    pub(crate) fn resolve(&self, v: Value) -> Result<Value, MError> {
        let _guard = self.enter()?;
        Ok(match v {
            Value::List(l) => match self.resolve_slots(l.items())? {
                Some(items) => Value::List(ListValue::new(items)),
                None => Value::List(l),
            },
            Value::Record(r) => match self.resolve_slots(r.values())? {
                Some(values) => Value::Record(RecordValue::new(r.keys().clone(), values)),
                None => Value::Record(r),
            },
            Value::Table(t) => {
                let mut changed = false;
                let mut columns = Vec::with_capacity(t.column_count());
                for c in t.columns() {
                    self.tick()?;
                    match self.resolve_slots(c)? {
                        Some(cells) => {
                            changed = true;
                            columns.push(Arc::new(cells));
                        }
                        None => columns.push(c.clone()),
                    }
                }
                if changed {
                    Value::Table(TableValue::from_parts(
                        t.keys().clone(),
                        t.column_types().to_vec(),
                        columns,
                    )?)
                } else {
                    Value::Table(t)
                }
            }
            Value::Meta(m) => {
                let value = self.resolve(m.value.clone())?;
                let meta = match self.resolve(Value::Record(m.meta.clone()))? {
                    Value::Record(r) => r,
                    _ => m.meta.clone(),
                };
                value.with_meta(meta)
            }
            other => other,
        })
    }
}

/// The M error record for a caught error: `[Reason, Message, Detail, Message.Format,
/// Message.Parameters]`.
pub(crate) fn error_record(e: &MError) -> Result<RecordValue, MError> {
    let (reason, message, detail) = match e {
        MError::Evaluation {
            reason,
            message,
            detail,
            ..
        } => (
            reason.clone(),
            message.clone(),
            detail.as_deref().cloned().unwrap_or(Value::Null),
        ),
        other => (
            "Expression.Error".to_owned(),
            other.to_string(),
            Value::Null,
        ),
    };
    RecordValue::from_pairs([
        ("Reason", Value::text(reason)),
        ("Message", Value::text(message.clone())),
        ("Detail", detail),
        ("Message.Format", Value::text(message)),
        ("Message.Parameters", Value::Null),
    ])
}
