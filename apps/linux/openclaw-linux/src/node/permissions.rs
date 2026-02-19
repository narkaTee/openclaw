use std::collections::HashMap;

pub fn current_permissions() -> HashMap<String, bool> {
    // TODO: integrate Linux permission checks (portal / desktop specific).
    HashMap::new()
}
