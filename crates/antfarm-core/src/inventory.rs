use std::collections::HashMap;

pub(crate) fn default_inventory() -> HashMap<String, u16> {
    HashMap::from([
        ("dirt".to_string(), 8),
        ("ore".to_string(), 0),
        ("stone".to_string(), 0),
        ("food".to_string(), 0),
        ("queen".to_string(), 1),
    ])
}

pub(crate) fn inventory_count(inventory: &HashMap<String, u16>, key: &str) -> u16 {
    inventory.get(key).copied().unwrap_or(0)
}

pub(crate) fn add_inventory(inventory: &mut HashMap<String, u16>, key: &str, amount: u16) {
    let entry = inventory.entry(key.to_string()).or_insert(0);
    *entry = entry.saturating_add(amount);
}

pub(crate) fn remove_inventory(inventory: &mut HashMap<String, u16>, key: &str, amount: u16) -> bool {
    let Some(entry) = inventory.get_mut(key) else {
        return false;
    };
    if *entry < amount {
        return false;
    }
    *entry -= amount;
    true
}
