use serde_json::Value;

pub(super) fn pick_str(v: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = v.get(*key).and_then(Value::as_str) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

pub(super) fn model_field(v: &Value) -> Option<String> {
    let model = v.get("model")?;
    if let Some(value) = model.as_str() {
        return Some(value.to_string());
    }
    if let Some(obj) = model.as_object() {
        if let Some(value) = obj.get("model").and_then(Value::as_str) {
            return Some(value.to_string());
        }
        return Some(model.to_string());
    }
    None
}
