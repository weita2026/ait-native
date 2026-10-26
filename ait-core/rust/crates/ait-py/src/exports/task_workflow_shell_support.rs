use ait_core::workflow_context_sdk::{
    WorkflowContext, WorkflowContextError, WorkflowContextResult,
};
use pyo3::{
    exceptions::{PyKeyError, PyRuntimeError, PyValueError},
    prelude::*,
};
use std::cell::RefCell;
use std::path::PathBuf;

pub struct PyWorkflowContext<'a, 'py> {
    ctx: &'a Bound<'py, PyAny>,
    error: RefCell<Option<PyErr>>,
}
impl<'a, 'py> PyWorkflowContext<'a, 'py> {
    pub fn new(ctx: &'a Bound<'py, PyAny>) -> Self {
        Self {
            ctx,
            error: RefCell::new(None),
        }
    }
    fn capture<T>(&self, result: PyResult<T>) -> WorkflowContextResult<T> {
        result.map_err(|error| {
            *self.error.borrow_mut() = Some(error);
            WorkflowContextError::Host("Workflow context metadata lookup failed.".into())
        })
    }
    pub fn map_error(&self, error: WorkflowContextError) -> PyErr {
        if let Some(error) = self.error.borrow_mut().take() {
            return error;
        }
        match error {
            WorkflowContextError::Invalid(message) => PyValueError::new_err(message),
            WorkflowContextError::MissingKey(message) => PyKeyError::new_err(message),
            WorkflowContextError::Host(message) => PyRuntimeError::new_err(message),
        }
    }
}
impl WorkflowContext for PyWorkflowContext<'_, '_> {
    fn bool_toggle(&self, attr_name: &str) -> WorkflowContextResult<Option<bool>> {
        self.capture(ctx_bool_toggle(self.ctx, attr_name))
    }
    fn path(&self, attr_name: &str) -> WorkflowContextResult<Option<PathBuf>> {
        self.capture(ctx_path(self.ctx, attr_name))
    }
}

fn ctx_bool_toggle(ctx: &Bound<'_, PyAny>, attr_name: &str) -> PyResult<Option<bool>> {
    let value = match ctx.getattr(attr_name) {
        Ok(value) => value,
        Err(err) if err.is_instance_of::<PyKeyError>(ctx.py()) => return Ok(None),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(ctx.py()) => {
            return Ok(None)
        }
        Err(err) => return Err(err),
    };
    if value.is_none() {
        return Ok(None);
    }
    if let Ok(flag) = value.extract::<bool>() {
        return Ok(Some(flag));
    }
    let text = value.str()?.to_string_lossy().to_string();
    Ok(match text.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    })
}

fn ctx_path(ctx: &Bound<'_, PyAny>, attr_name: &str) -> PyResult<Option<PathBuf>> {
    let value = match ctx.getattr(attr_name) {
        Ok(value) => value,
        Err(err) if err.is_instance_of::<PyKeyError>(ctx.py()) => return Ok(None),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(ctx.py()) => {
            return Ok(None)
        }
        Err(err) => return Err(err),
    };
    if value.is_none() {
        return Ok(None);
    }
    let text = value.str()?.to_string_lossy().trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(text)))
}
