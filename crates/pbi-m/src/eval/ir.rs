//! Evaluator IR: the AST lowered into `Arc`-shared nodes that closures and lazy slots can
//! reference after the source tree is gone.

use std::sync::Arc;

use crate::ast::{
    self, BinaryOp, Expr, ExprKind, Intrinsic, ListItem, Literal, NullablePrimitiveType,
    PrimitiveType, TableRowType, TryHandler, TypeExpr, TypeKind, UnaryOp,
};
use crate::span::{Span, line_col};
use crate::value::{Keys, Value};

/// The source a node was compiled from (for error locations).
#[derive(Debug)]
pub(crate) struct SourceText {
    /// `None` for the main query.
    pub name: Option<Arc<str>>,
    pub text: Arc<str>,
}

impl SourceText {
    pub fn location(self: &Arc<Self>, span: Span) -> crate::merror::Location {
        let (line, column) = line_col(&self.text, span.start);
        crate::merror::Location {
            query: self.name.clone(),
            span,
            line,
            column,
        }
    }
}

pub(crate) type NodeRef = Arc<Node>;

#[derive(Debug)]
pub(crate) struct Node {
    pub kind: NodeKind,
    pub span: Span,
    pub src: Arc<SourceText>,
}

impl Node {
    pub fn location(&self) -> crate::merror::Location {
        self.src.location(self.span)
    }
}

/// Names and initializers of a `let` or record expression.
#[derive(Debug)]
pub(crate) struct Bindings {
    pub names: Arc<[Arc<str>]>,
    pub values: Vec<NodeRef>,
    /// Record keys, or the duplicated name.
    pub keys: Result<Arc<Keys>, Arc<str>>,
}

#[derive(Debug)]
pub(crate) struct ParamDef {
    pub name: Arc<str>,
    pub optional: bool,
    pub ty: Option<NullablePrimitiveType>,
}

#[derive(Debug)]
pub(crate) struct FnDef {
    pub params: Vec<ParamDef>,
    pub names: Arc<[Arc<str>]>,
    pub ret: Option<NullablePrimitiveType>,
    pub body: NodeRef,
}

#[derive(Debug)]
pub(crate) enum ListItemIr {
    Single(NodeRef),
    Range(NodeRef, NodeRef),
}

#[derive(Debug)]
pub(crate) enum TryIr {
    Otherwise(NodeRef),
    Catch(Option<Arc<str>>, NodeRef),
}

#[derive(Debug)]
pub(crate) struct FieldSpecIr {
    pub name: Arc<str>,
    pub optional: bool,
    pub ty: Option<TypeNode>,
}

#[derive(Debug)]
pub(crate) enum TypeNode {
    Primitive(PrimitiveType),
    Nullable(Box<TypeNode>),
    Record(Vec<FieldSpecIr>, bool),
    List(Box<TypeNode>),
    Function(Vec<FieldSpecIr>, Box<TypeNode>),
    Table(Vec<FieldSpecIr>),
    TableExpr(NodeRef),
    Expr(NodeRef),
}

#[derive(Debug)]
pub(crate) enum NodeKind {
    Const(Value),
    Verbatim,
    Ident {
        name: Arc<str>,
        inclusive: bool,
    },
    Intrinsic(Intrinsic),
    SectionAccess {
        section: Arc<str>,
        member: Arc<str>,
    },
    Let(Arc<Bindings>, NodeRef),
    If(NodeRef, NodeRef, NodeRef),
    Function(Arc<FnDef>),
    Invoke(NodeRef, Vec<NodeRef>),
    Record(Arc<Bindings>),
    List(Vec<ListItemIr>),
    Field {
        target: Option<NodeRef>,
        field: Arc<str>,
        optional: bool,
    },
    Project {
        target: Option<NodeRef>,
        fields: Vec<Arc<str>>,
        optional: bool,
    },
    Item {
        target: NodeRef,
        selector: NodeRef,
        optional: bool,
    },
    Unary(UnaryOp, NodeRef),
    Binary(BinaryOp, NodeRef, NodeRef),
    As(NodeRef, NullablePrimitiveType),
    Is(NodeRef, NullablePrimitiveType),
    Type(Arc<TypeNode>),
    Try(NodeRef, Option<TryIr>),
    Error(NodeRef),
    NotImplemented,
}

pub(crate) struct Compiler {
    src: Arc<SourceText>,
}

fn name(n: &ast::Name) -> Arc<str> {
    Arc::from(n.text.as_str())
}

impl Compiler {
    pub fn new(src: Arc<SourceText>) -> Self {
        Self { src }
    }

    fn node(&self, kind: NodeKind, span: Span) -> NodeRef {
        Arc::new(Node {
            kind,
            span,
            src: self.src.clone(),
        })
    }

    fn bindings(&self, items: Vec<(Arc<str>, NodeRef)>) -> Arc<Bindings> {
        let (names, values): (Vec<Arc<str>>, Vec<NodeRef>) = items.into_iter().unzip();
        let keys = Keys::new(names.clone()).map(Arc::new);
        Arc::new(Bindings {
            names: names.into(),
            values,
            keys,
        })
    }

    pub fn function(
        &self,
        params: Vec<ParamDef>,
        ret: Option<NullablePrimitiveType>,
        body: NodeRef,
    ) -> Arc<FnDef> {
        let names: Arc<[Arc<str>]> = params.iter().map(|p| p.name.clone()).collect();
        Arc::new(FnDef {
            params,
            names,
            ret,
            body,
        })
    }

    pub fn expr(&self, e: &Expr) -> NodeRef {
        let e = e.unparenthesized();
        let kind = match &e.kind {
            ExprKind::Literal(lit) => match lit {
                Literal::Null => NodeKind::Const(Value::Null),
                Literal::Logical(b) => NodeKind::Const(Value::Logical(*b)),
                Literal::Number(n) => NodeKind::Const(Value::Number(n.value)),
                Literal::Text(t) => NodeKind::Const(Value::Text(t.as_str().into())),
                Literal::Verbatim(_) => NodeKind::Verbatim,
            },
            ExprKind::Identifier { name: n, inclusive } => NodeKind::Ident {
                name: name(n),
                inclusive: *inclusive,
            },
            ExprKind::Intrinsic(i) => NodeKind::Intrinsic(*i),
            ExprKind::SectionAccess { section, member } => NodeKind::SectionAccess {
                section: name(section),
                member: name(member),
            },
            ExprKind::Parenthesized(inner) => return self.expr(inner),
            ExprKind::Let { bindings, body } => {
                let b = self.bindings(
                    bindings
                        .iter()
                        .map(|b| (name(&b.name), self.expr(&b.value)))
                        .collect(),
                );
                NodeKind::Let(b, self.expr(body))
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => NodeKind::If(
                self.expr(condition),
                self.expr(then_branch),
                self.expr(else_branch),
            ),
            ExprKind::Each(body) => NodeKind::Function(self.function(
                vec![ParamDef {
                    name: "_".into(),
                    optional: false,
                    ty: None,
                }],
                None,
                self.expr(body),
            )),
            ExprKind::Function {
                params,
                return_type,
                body,
            } => NodeKind::Function(
                self.function(
                    params
                        .iter()
                        .map(|p| ParamDef {
                            name: name(&p.name),
                            optional: p.optional,
                            ty: p.ty,
                        })
                        .collect(),
                    *return_type,
                    self.expr(body),
                ),
            ),
            ExprKind::Invoke { function, args } => NodeKind::Invoke(
                self.expr(function),
                args.iter().map(|a| self.expr(a)).collect(),
            ),
            ExprKind::Record(fields) => NodeKind::Record(
                self.bindings(
                    fields
                        .iter()
                        .map(|f| (name(&f.name), self.expr(&f.value)))
                        .collect(),
                ),
            ),
            ExprKind::List(items) => NodeKind::List(
                items
                    .iter()
                    .map(|i| match i {
                        ListItem::Single(e) => ListItemIr::Single(self.expr(e)),
                        ListItem::Range { start, end } => {
                            ListItemIr::Range(self.expr(start), self.expr(end))
                        }
                    })
                    .collect(),
            ),
            ExprKind::FieldAccess {
                target,
                field,
                optional,
            } => NodeKind::Field {
                target: target.as_ref().map(|t| self.expr(t)),
                field: name(field),
                optional: *optional,
            },
            ExprKind::FieldProjection {
                target,
                fields,
                optional,
            } => NodeKind::Project {
                target: target.as_ref().map(|t| self.expr(t)),
                fields: fields.iter().map(name).collect(),
                optional: *optional,
            },
            ExprKind::ItemAccess {
                target,
                selector,
                optional,
            } => NodeKind::Item {
                target: self.expr(target),
                selector: self.expr(selector),
                optional: *optional,
            },
            ExprKind::Unary { op, operand } => NodeKind::Unary(*op, self.expr(operand)),
            ExprKind::Binary { op, lhs, rhs } => {
                NodeKind::Binary(*op, self.expr(lhs), self.expr(rhs))
            }
            ExprKind::As { expr, ty } => NodeKind::As(self.expr(expr), *ty),
            ExprKind::Is { expr, ty } => NodeKind::Is(self.expr(expr), *ty),
            ExprKind::Type(t) => NodeKind::Type(Arc::new(self.type_expr(t))),
            ExprKind::Try { expr, handler } => NodeKind::Try(
                self.expr(expr),
                handler.as_ref().map(|h| match h {
                    TryHandler::Otherwise(d) => TryIr::Otherwise(self.expr(d)),
                    TryHandler::Catch { param, body } => {
                        TryIr::Catch(param.as_ref().map(name), self.expr(body))
                    }
                }),
            ),
            ExprKind::Error(e) => NodeKind::Error(self.expr(e)),
            ExprKind::NotImplemented => NodeKind::NotImplemented,
        };
        self.node(kind, e.span)
    }

    fn field_specs(&self, fields: &[ast::FieldSpec]) -> Vec<FieldSpecIr> {
        fields
            .iter()
            .map(|f| FieldSpecIr {
                name: name(&f.name),
                optional: f.optional,
                ty: f.ty.as_ref().map(|t| self.type_expr(t)),
            })
            .collect()
    }

    fn type_expr(&self, t: &TypeExpr) -> TypeNode {
        match &t.kind {
            TypeKind::Primitive(p) => TypeNode::Primitive(*p),
            TypeKind::Nullable(inner) => TypeNode::Nullable(Box::new(self.type_expr(inner))),
            TypeKind::Record(r) => TypeNode::Record(self.field_specs(&r.fields), r.open),
            TypeKind::List(item) => TypeNode::List(Box::new(self.type_expr(item))),
            TypeKind::Function(f) => TypeNode::Function(
                f.params
                    .iter()
                    .map(|p| FieldSpecIr {
                        name: name(&p.name),
                        optional: p.optional,
                        ty: p.ty.as_ref().map(|t| self.type_expr(t)),
                    })
                    .collect(),
                Box::new(self.type_expr(&f.return_type)),
            ),
            TypeKind::Table(TableRowType::Fields(r)) => {
                TypeNode::Table(self.field_specs(&r.fields))
            }
            TypeKind::Table(TableRowType::Expr(e)) => TypeNode::TableExpr(self.expr(e)),
            TypeKind::Expr(e) => TypeNode::Expr(self.expr(e)),
        }
    }
}
