//! Minimal localization: picks UI strings based on the OS UI language.
//!
//! French if the user's preferred language (System Settings → Language) is
//! French, English otherwise. The language is detected once and cached.

use std::sync::OnceLock;

use objc2_foundation::NSLocale;

static IS_FRENCH: OnceLock<bool> = OnceLock::new();

/// Returns `fr` if the OS UI language is French, otherwise `en`.
pub fn tr(fr: &'static str, en: &'static str) -> &'static str {
    if is_french() {
        fr
    } else {
        en
    }
}

/// True if the user's preferred OS language is French (cached after first call).
pub fn is_french() -> bool {
    *IS_FRENCH.get_or_init(|| {
        // `preferredLanguages` reflects the ordered languages set in System
        // Settings, e.g. ["fr-FR", "en"]. We look at the first one.
        let langs = NSLocale::preferredLanguages();
        langs
            .firstObject()
            .map(|l| l.to_string().to_lowercase().starts_with("fr"))
            .unwrap_or(false)
    })
}
