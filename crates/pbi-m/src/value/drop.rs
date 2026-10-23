//! Iterative destruction of deeply nested values.
//!
//! M programs can build arbitrarily deep structures (`List.Accumulate(l, {}, (s, x) => {s})`,
//! chains of closures and environments). Dropping them recursively would overflow the stack, so
//! the containers' `Drop` implementations hand their children to a per-thread queue that is
//! drained iteratively by the outermost drop.

use std::cell::RefCell;
use std::sync::Arc;

use crate::eval::env::Env;
use crate::eval::thunk::Pending;
use crate::merror::MError;
use crate::value::{Lazy, TypeKind, Value};

// The fields are never read: they are only held so they can be dropped from the queue.
#[allow(dead_code)]
pub(crate) enum Garbage {
    Slots(Vec<Lazy>),
    Columns(Vec<Arc<Vec<Lazy>>>),
    Value(Value),
    Env(Env),
    Type(TypeKind),
    Pending(Pending),
    Result(Result<Value, MError>),
}

thread_local! {
    static QUEUE: RefCell<Option<Vec<Garbage>>> = const { RefCell::new(None) };
}

/// Drops `g` without recursing into nested containers.
pub(crate) fn defer(g: Garbage) {
    // `Some(g)`: this call is the outermost drop and must drain the queue.
    let first = QUEUE.try_with(|q| match q.try_borrow_mut() {
        Ok(mut q) => match q.as_mut() {
            Some(queue) => {
                queue.push(g);
                None
            }
            None => {
                *q = Some(Vec::new());
                Some(g)
            }
        },
        Err(_) => Some(g),
    });
    let first = match first {
        Ok(Some(g)) => g,
        Ok(None) => return,
        // Thread-local storage is being torn down: drop normally.
        Err(_) => return,
    };
    drop(first);
    loop {
        let next = QUEUE
            .try_with(|q| q.try_borrow_mut().ok().and_then(|mut q| q.as_mut()?.pop()))
            .ok()
            .flatten();
        match next {
            Some(g) => drop(g),
            None => break,
        }
    }
    let _ = QUEUE.try_with(|q| {
        if let Ok(mut q) = q.try_borrow_mut() {
            *q = None;
        }
    });
}
