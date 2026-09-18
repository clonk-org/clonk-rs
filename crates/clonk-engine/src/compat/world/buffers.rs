use super::*;

const MAX_BUFFERS: usize = 8;
const MAX_CAPACITY: usize = 4_096;

#[derive(Default)]
struct ObjectBuffers {
    objects: FxHashMap<ObjectId, Rc<HostWorldObject>>,
    order: Vec<ObjectId>,
    indices: FxHashMap<ObjectId, usize>,
    removed: FxHashSet<ObjectId>,
}

thread_local! {
    static OBJECT_BUFFERS: RefCell<Vec<ObjectBuffers>> = const { RefCell::new(Vec::new()) };
}

impl HostWorldObjectStore {
    pub(super) fn reusable(complete: bool) -> Self {
        let buffers = OBJECT_BUFFERS
            .with(|pool| pool.borrow_mut().pop())
            .unwrap_or_default();
        Self {
            objects: buffers.objects,
            order: buffers.order,
            indices: buffers.indices,
            removed: buffers.removed,
            order_dirty: false,
            complete,
        }
    }
}

impl Drop for HostWorldObjectStore {
    fn drop(&mut self) {
        if [
            self.objects.capacity(),
            self.order.capacity(),
            self.indices.capacity(),
            self.removed.capacity(),
        ]
        .into_iter()
        .any(|capacity| capacity > MAX_CAPACITY)
        {
            return;
        }
        let mut buffers = ObjectBuffers {
            objects: std::mem::take(&mut self.objects),
            order: std::mem::take(&mut self.order),
            indices: std::mem::take(&mut self.indices),
            removed: std::mem::take(&mut self.removed),
        };
        // Drop all object/value ownership before touching the pool. Only the
        // final Rc owner recycles a store; cloned callback views stay live.
        buffers.objects.clear();
        buffers.order.clear();
        buffers.indices.clear();
        buffers.removed.clear();
        let _ = OBJECT_BUFFERS.try_with(|pool| {
            if let Ok(mut pool) = pool.try_borrow_mut() {
                if pool.len() < MAX_BUFFERS {
                    pool.push(buffers);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycled_object_buffers_release_values_only_after_the_last_view() {
        let mut engine = crate::Engine::new();
        engine
            .register_script_definition("TEST", "Test", "func Probe() { return 0; }")
            .unwrap();
        let id = engine
            .spawn_object(crate::SpawnConfig::new("TEST"))
            .unwrap();
        let world = engine.host_world_context();
        let weak = Rc::downgrade(&world.get_shared(id).unwrap());
        let retained = world.clone();
        drop(world);
        let fresh = engine.host_world_context();
        assert!(fresh.object_store.borrow().objects.is_empty());
        assert!(weak.upgrade().is_some());
        drop(fresh);
        drop(retained);
        assert!(
            weak.upgrade().is_none(),
            "the pool must not retain object values"
        );
        let reused = engine.host_world_context();
        assert!(reused.object_store.borrow().objects.is_empty());
        assert!(reused.object_store.borrow().removed.is_empty());
        assert!(reused.get_shared(id).is_some());
    }

    #[test]
    fn object_buffer_pool_does_not_retain_large_or_unbounded_storage() {
        OBJECT_BUFFERS.with(|pool| pool.borrow_mut().clear());
        let mut large = HostWorldObjectStore::reusable(false);
        large.objects.reserve(MAX_CAPACITY + 1);
        drop(large);
        assert_eq!(OBJECT_BUFFERS.with(|pool| pool.borrow().len()), 0);
        let stores = (0..MAX_BUFFERS + 4)
            .map(|_| HostWorldObjectStore::reusable(false))
            .collect::<Vec<_>>();
        drop(stores);
        assert_eq!(OBJECT_BUFFERS.with(|pool| pool.borrow().len()), MAX_BUFFERS);
    }
}
