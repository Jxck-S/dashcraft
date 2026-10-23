//! Lexical environments.
//!
//! An [`Env`] is a chain of [`Frame`]s (let bindings, record fields, function parameters,
//! section members, shared queries). `let`, record and section frames hold lazy slots that may
//! capture the frame itself, which forms `Arc` cycles; the interpreter registers those frames
//! and [`Frame::clear`]s them when the evaluation ends so nothing leaks.

use std::sync::{Arc, PoisonError, RwLock};

use crate::value::Lazy;
use crate::value::drop::{Garbage, defer};

struct FrameData {
    names: Arc<[Arc<str>]>,
    values: Vec<Lazy>,
    parent: Option<Env>,
}

pub(crate) struct Frame {
    data: RwLock<FrameData>,
}

impl Drop for Frame {
    fn drop(&mut self) {
        let d = self.data.get_mut().unwrap_or_else(PoisonError::into_inner);
        if !d.values.is_empty() {
            defer(Garbage::Slots(std::mem::take(&mut d.values)));
        }
        if let Some(parent) = d.parent.take() {
            defer(Garbage::Env(parent));
        }
    }
}

impl Frame {
    pub fn new(names: Arc<[Arc<str>]>, values: Vec<Lazy>, parent: Option<Env>) -> Arc<Frame> {
        Arc::new(Frame {
            data: RwLock::new(FrameData {
                names,
                values,
                parent,
            }),
        })
    }

    pub fn set_values(&self, values: Vec<Lazy>) {
        let mut d = self.data.write().unwrap_or_else(PoisonError::into_inner);
        d.values = values;
    }

    /// Drops all slots and the parent link (breaks reference cycles).
    pub fn clear(&self) {
        let (values, parent) = {
            let mut d = self.data.write().unwrap_or_else(PoisonError::into_inner);
            d.names = Arc::from(Vec::new());
            (std::mem::take(&mut d.values), d.parent.take())
        };
        // Dropped outside the lock: dropping may cascade into other frames.
        drop(values);
        drop(parent);
    }

    pub fn entries(&self) -> (Arc<[Arc<str>]>, Vec<Lazy>) {
        let d = self.data.read().unwrap_or_else(PoisonError::into_inner);
        (d.names.clone(), d.values.clone())
    }
}

/// An environment: a frame plus, while a binding's own initializer is evaluated, the index of
/// that binding (which is only visible through `@name`).
#[derive(Clone)]
pub(crate) struct Env {
    frame: Arc<Frame>,
    exclude: Option<usize>,
}

pub(crate) enum Lookup {
    Found(Lazy),
    /// Only the binding currently being defined has this name (visible without `@` as a
    /// fallback when no outer scope or library defines it, for recursive functions).
    SelfOnly(Lazy),
    Missing,
}

impl Env {
    pub fn new(frame: Arc<Frame>) -> Env {
        Env {
            frame,
            exclude: None,
        }
    }

    pub fn member(frame: &Arc<Frame>, index: usize) -> Env {
        Env {
            frame: frame.clone(),
            exclude: Some(index),
        }
    }

    pub fn frame(&self) -> &Arc<Frame> {
        &self.frame
    }

    /// A child environment with the given bindings (function parameters, `catch` variable).
    pub fn child(&self, names: Arc<[Arc<str>]>, values: Vec<Lazy>) -> Env {
        Env::new(Frame::new(names, values, Some(self.clone())))
    }

    pub fn lookup(&self, name: &str, inclusive: bool) -> Lookup {
        let mut fallback = None;
        let mut cur = self.clone();
        loop {
            let parent = {
                let d = cur
                    .frame
                    .data
                    .read()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Some(i) = d.names.iter().position(|n| &**n == name)
                    && let Some(v) = d.values.get(i)
                {
                    if inclusive || cur.exclude != Some(i) {
                        return Lookup::Found(v.clone());
                    }
                    if fallback.is_none() {
                        fallback = Some(v.clone());
                    }
                }
                d.parent.clone()
            };
            match parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        match fallback {
            Some(v) => Lookup::SelfOnly(v),
            None => Lookup::Missing,
        }
    }
}
