//! Memoized lazy evaluation with cycle detection.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, ThreadId};

use crate::eval::env::Env;
use crate::eval::interp::Interp;
use crate::eval::ir::NodeRef;
use crate::merror::MError;
use crate::value::Value;
use crate::value::drop::{Garbage, defer};

pub(crate) enum Pending {
    /// An expression in an environment.
    Expr(Env, NodeRef),
    /// A shared query / parameter, parsed on first use.
    Shared { name: Arc<str>, source: Arc<str> },
}

enum State {
    Pending(Pending),
    Running(ThreadId),
    Done(Result<Value, MError>),
}

/// A lazily evaluated, memoized slot (let binding, record field, list item, shared query).
pub struct Thunk {
    state: Mutex<State>,
}

impl fmt::Debug for Thunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.peek() {
            Some(Ok(v)) => write!(f, "Thunk({v:?})"),
            Some(Err(e)) => write!(f, "Thunk(error {e})"),
            None => f.write_str("Thunk(<unevaluated>)"),
        }
    }
}

impl Drop for Thunk {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        match std::mem::replace(state, State::Done(Ok(Value::Null))) {
            State::Pending(p) => defer(Garbage::Pending(p)),
            State::Done(Ok(Value::Null)) | State::Running(_) => {}
            State::Done(r) => defer(Garbage::Result(r)),
        }
    }
}

pub(crate) fn cycle_error() -> MError {
    MError::expression("A cyclic reference was encountered during evaluation.")
}

impl Thunk {
    pub(crate) fn new(pending: Pending) -> Arc<Thunk> {
        Arc::new(Thunk {
            state: Mutex::new(State::Pending(pending)),
        })
    }

    /// The memoized result, if evaluated.
    pub fn peek(&self) -> Option<Result<Value, MError>> {
        match &*self.state.lock().unwrap_or_else(PoisonError::into_inner) {
            State::Done(r) => Some(r.clone()),
            _ => None,
        }
    }

    pub(crate) fn force(&self, it: &Interp) -> Result<Value, MError> {
        let pending = {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match &*st {
                State::Done(r) => return r.clone(),
                State::Running(id) => {
                    return Err(if *id == thread::current().id() {
                        cycle_error()
                    } else {
                        MError::Internal("value is being evaluated on another thread".into())
                    });
                }
                State::Pending(_) => {}
            }
            match std::mem::replace(&mut *st, State::Running(thread::current().id())) {
                State::Pending(p) => p,
                _ => return Err(MError::Internal("thunk state changed unexpectedly".into())),
            }
        };
        let result = it.run_pending(pending);
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = State::Done(result.clone());
        result
    }
}
