//! The system sound scheme, as chosen in the classic Sound settings.
//!
//! Each event keeps one sound per scheme and the one in use in `.Current`.
//! Choosing a scheme copies its sounds into `.Current`, and an event the
//! scheme has no sound for goes quiet, as Windows does.

use serde_json::Value;

use super::{Choice, Values, text};
use crate::win;

const SCHEMES: &str = r"AppEvents\Schemes";
const NAMES: &str = r"AppEvents\Schemes\Names";
const APPS: &str = r"AppEvents\Schemes\Apps";
const CURRENT: &str = ".Current";

pub fn schemes() -> Vec<Choice> {
    win::user_subkeys(NAMES)
        .into_iter()
        .map(|id| {
            let name = win::read_user_string(&format!(r"{NAMES}\{id}"), "").unwrap_or_default();
            // System schemes name themselves with a reference such as @mmres.dll,-800.
            let label = if name.starts_with('@') {
                win::load_indirect_string(&name).unwrap_or_default()
            } else {
                name
            };
            let label = if label.is_empty() { id.clone() } else { label };
            Choice { id, label }
        })
        .collect()
}

pub fn read(values: &mut Values) {
    let current = win::read_user_string(SCHEMES, "").filter(|id| !id.is_empty());
    values.insert("soundScheme".into(), current.into());
}

pub fn set(name: &str, value: &Value) -> Option<Result<(), String>> {
    (name == "soundScheme").then(|| text(value).and_then(set_scheme))
}

fn set_scheme(id: &str) -> Result<(), String> {
    if !win::user_subkeys(NAMES).iter().any(|scheme| scheme == id) {
        return Err(format!("O esquema de sons {id} não existe."));
    }
    for app in win::user_subkeys(APPS) {
        let app_key = format!(r"{APPS}\{app}");
        for event in win::user_subkeys(&app_key) {
            let event_key = format!(r"{app_key}\{event}");
            let sound = win::read_user_string(&format!(r"{event_key}\{id}"), "").unwrap_or_default();
            win::set_user_expand_string(&format!(r"{event_key}\{CURRENT}"), "", &sound)?;
        }
    }
    win::set_user_string(SCHEMES, "", id)
}
