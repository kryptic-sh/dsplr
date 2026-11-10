//! A layout's JSON form (`Layout::to_json`) with fields of dsplr's own
//! beside the layout's fields, as the revert point keeps its token and a
//! profile its save time. `Layout::from_json` passes over a field it does not
//! know, so the file still reads as a layout, for `dsplr apply` too.

use dsplr_model::Layout;
use serde_json::{Map, Value};

/// `layout` as pretty-printed JSON with each of `extra`'s keys set to its
/// value, ending in a newline.
pub fn write(layout: &Layout, extra: &[(&str, Value)]) -> Result<String, String> {
    let text = layout.to_json().map_err(|e| e.to_string())?;
    let mut fields: Map<String, Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for (key, value) in extra {
        fields.insert((*key).to_owned(), value.clone());
    }
    serde_json::to_string_pretty(&fields)
        .map(|text| text + "\n")
        .map_err(|e| e.to_string())
}

/// The layout in `text`, and the value of each of `keys` beside it, `None`
/// for one it has not.
pub fn read<const N: usize>(
    text: &str,
    keys: [&str; N],
) -> Result<(Layout, [Option<Value>; N]), String> {
    let mut fields: Map<String, Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let values = keys.map(|key| fields.remove(key));
    let layout = Layout::from_json(text).map_err(|e| e.to_string())?;
    Ok((layout, values))
}
