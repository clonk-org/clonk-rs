use std::sync::Arc;

use crate::value::Value;

type CallHook = Arc<dyn Fn(&str, &[Value]) + Send + Sync>;
type ReturnHook = Arc<dyn Fn(&str, &Value) + Send + Sync>;

#[derive(Clone, Default)]
pub struct DebuggerHooks {
    on_call: Option<CallHook>,
    on_return: Option<ReturnHook>,
}

impl DebuggerHooks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_on_call<F>(mut self, callback: F) -> Self
    where
        F: Fn(&str, &[Value]) + Send + Sync + 'static,
    {
        self.set_on_call(callback);
        self
    }

    pub fn with_on_return<F>(mut self, callback: F) -> Self
    where
        F: Fn(&str, &Value) + Send + Sync + 'static,
    {
        self.set_on_return(callback);
        self
    }

    pub fn set_on_call<F>(&mut self, callback: F)
    where
        F: Fn(&str, &[Value]) + Send + Sync + 'static,
    {
        self.on_call = Some(Arc::new(callback));
    }

    pub fn set_on_return<F>(&mut self, callback: F)
    where
        F: Fn(&str, &Value) + Send + Sync + 'static,
    {
        self.on_return = Some(Arc::new(callback));
    }

    pub fn clear_on_call(&mut self) {
        self.on_call = None;
    }

    pub fn clear_on_return(&mut self) {
        self.on_return = None;
    }

    pub fn on_call(&self) -> Option<&CallHook> {
        self.on_call.as_ref()
    }

    pub fn on_return(&self) -> Option<&ReturnHook> {
        self.on_return.as_ref()
    }

    pub fn is_enabled(&self) -> bool {
        self.on_call.is_some() || self.on_return.is_some()
    }
}
