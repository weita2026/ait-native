use pyo3::prelude::*;

mod exports;
mod json_support;

#[pymodule(name = "ait_py")]
fn ait_py(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    exports::register(py, module)
}
