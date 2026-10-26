mod codec;
mod fields;
mod file_store;

pub use codec::{JsonCodec, JsonCodecError, JsonEncodeOptions, JsonEncodeStyle};
pub use fields::{
    optional_array_field, optional_bool_field, optional_integer_field, optional_object_field,
    optional_path_field, optional_text_field, required_array_field, required_array_value,
    required_bool_field, required_integer_field, required_object_field, required_object_value,
    required_path_field, required_text_field, JsonFieldError,
};
pub use file_store::{
    expand_home_path_with_file_io_store, read_json_object_or_empty_with_file_io_store,
    read_json_or_null_with_file_io_store,
    write_pretty_json_atomically_with_newline_with_file_io_store,
    write_pretty_json_with_file_io_store, JsonFileStore,
};
pub use serde_json::{json, Map as JsonMap, Number as JsonNumber, Value as JsonValue};

#[cfg(test)]
mod tests;
