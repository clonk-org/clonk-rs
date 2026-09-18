use super::*;

const MAX_SCRATCH: usize = 8;
// Scopes are much larger than object IDs; keep this pool deliberately small.
const MAX_CAPACITY: usize = 256;

#[derive(Default)]
pub(super) struct CallbackScratch {
    pub(super) dormant_scopes: Vec<Option<ObjectScopeContext>>,
    pub(super) nested_objects: HashMap<ObjectId, NestedScopeState>,
    pub(super) nested_order: Vec<ObjectId>,
    pub(super) session_local_cells: HashMap<ObjectId, clonk_script::LocalCells>,
    pub(super) foreign_local_cells: HashMap<(ObjectId, String), clonk_script::ValueCell>,
}

thread_local! {
    static CALLBACK_SCRATCH: RefCell<Vec<CallbackScratch>> = const { RefCell::new(Vec::new()) };
}

impl CallbackScratch {
    pub(super) fn take() -> Self {
        CALLBACK_SCRATCH
            .with(|pool| pool.borrow_mut().pop())
            .unwrap_or_default()
    }

    pub(super) fn recycle(mut self) {
        if [
            self.dormant_scopes.capacity(),
            self.nested_objects.capacity(),
            self.nested_order.capacity(),
            self.session_local_cells.capacity(),
            self.foreign_local_cells.capacity(),
        ]
        .into_iter()
        .any(|capacity| capacity > MAX_CAPACITY)
        {
            return;
        }
        // Do not retain live cells, scopes, object IDs or script values
        // between callbacks, even when a returned reference still owns them.
        self.dormant_scopes.clear();
        self.nested_objects.clear();
        self.nested_order.clear();
        self.session_local_cells.clear();
        self.foreign_local_cells.clear();
        let _ = CALLBACK_SCRATCH.try_with(|pool| {
            if let Ok(mut pool) = pool.try_borrow_mut() {
                if pool.len() < MAX_SCRATCH {
                    pool.push(self);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycling_scratch_releases_cells_without_changing_escaped_values() {
        let mut scratch = CallbackScratch::take();
        let escaped = clonk_script::value_cell(Value::Array(vec![Value::Int(41)]));
        let released = clonk_script::value_cell(Value::Int(7));
        let weak = Rc::downgrade(&released);
        scratch
            .foreign_local_cells
            .insert((ObjectId::new(1), "escaped".into()), escaped.clone());
        scratch
            .foreign_local_cells
            .insert((ObjectId::new(2), "released".into()), released);
        scratch.recycle();
        assert!(weak.upgrade().is_none());
        assert_eq!(*escaped.borrow(), Value::Array(vec![Value::Int(41)]));
        let reused = CallbackScratch::take();
        assert!(reused.foreign_local_cells.is_empty());
        assert!(reused.session_local_cells.is_empty());
    }
}
