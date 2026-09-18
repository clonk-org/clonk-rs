use super::*;
use std::ops::Deref;

/// A callback-entry projection allocated only when a host API reads it.
/// Clones retain an initialized backing; writes detach it just like the
/// callback world's other copy-on-write tables.
#[derive(Clone)]
pub(crate) struct CallbackSnapshot<T> {
    value: OnceCell<Rc<T>>,
    source: Option<(*const (), unsafe fn(*const ()) -> T)>,
}

impl<T> CallbackSnapshot<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            value: OnceCell::from(Rc::new(value)),
            source: None,
        }
    }

    /// # Safety
    /// The source must obey `LazyHostWorldProvider`'s stable, paused-engine
    /// lifetime contract until this snapshot and every clone are dropped.
    /// The projector may borrow only the fields it reads, never the Engine
    /// as a whole: a callback may exclusively borrow a disjoint object field.
    pub(super) unsafe fn deferred(source: *const (), project: unsafe fn(*const ()) -> T) -> Self {
        Self {
            value: OnceCell::new(),
            source: Some((source, project)),
        }
    }
}

impl<T> AsRef<T> for CallbackSnapshot<T> {
    fn as_ref(&self) -> &T {
        self.value.get_or_init(|| {
            let (source, project) = self.source.expect("a deferred snapshot has a source");
            // SAFETY: the constructor's lifetime and field-borrow contract.
            Rc::new(unsafe { project(source) })
        })
    }
}

impl<T> Deref for CallbackSnapshot<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.as_ref()
    }
}

impl<T: Clone> CallbackSnapshot<T> {
    pub(super) fn make_mut(&mut self) -> &mut T {
        let _ = self.as_ref();
        Rc::make_mut(self.value.get_mut().expect("snapshot was initialized"))
    }
}
