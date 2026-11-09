//! A layout's JSON form (`Layout::to_json`) with one field of dsplr's own
//! beside the layout's fields, as the revert point keeps its token and a
//! profile its save time. `Layout::from_json` passes over a field it does not
//! know, so the file still reads as a layout, for `dsplr apply` too.

use dsplr_model::Layout;
use serde_json::{Map, Value};

/// `layout` as pretty-printed JSON with `key` set to `value`, ending in a
/// newline.
pub fn write(layout: &Layout, key: &str, value: Value) -> Result<String, String> {
    let text = layout.to_json().map_err(|e| e.to_string())?;
    let mut fields: Map<String, Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    fields.insert(key.to_owned(), value);
    serde_json::to_string_pretty(&fields)
        .map(|text| text + "\n")
        .map_err(|e| e.to_string())
}

/// The layout in `text`, and the value of `key` beside it, `None` when it
/// has none.
pub fn read(text: &str, key: &str) -> Result<(Layout, Option<Value>), String> {
    let mut fields: Map<String, Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let value = fields.remove(key);
    let layout = Layout::from_json(text).map_err(|e| e.to_string())?;
    Ok((layout, value))
}
