//! atspi focus resolution + live session ops — TODO(on-device) adapter.
//!
//! REAL: the AT-SPI bus connection (the trust gate). TODO(on-device): reading the focused element's
//! role/label/selection/preceding-text and driving live edits — these need a live focused control.
//! Until wired, `resolve_context` reports `trusted=true, focus=None` when the bus is up (⇒ the
//! classifier returns `unknown` and no session starts) and `trusted=false` when it is down (⇒
//! `secure`, fail-closed). Both are contract-valid.

use crate::classify::ClassifyContext;
use crate::session::SessionOps;

/// Resolve the classification context from AT-SPI.
pub async fn resolve_context() -> ClassifyContext {
    match connect().await {
        Err(e) => {
            eprintln!(">> AT-SPI bus unavailable ({e}); classifying as secure (fail-closed)");
            ClassifyContext { trusted: false, focus: None }
        }
        Ok(()) => {
            // TODO(on-device): read the focused element via the AT-SPI focus cache / the app whose
            //   window has State::Active, then its State::Focused descendant. Fill a FocusInput:
            //     role_ax  = platform::roles::atspi_role_to_ax(&node.get_role().name())
            //     subrole  = platform::roles::atspi_secure_subrole(raw_role)
            //     label    = compose(title, description, placeholder)
            //     selection= node.Text.get_selection(0)  (start,end) → (loc, len) char offsets
            //     preceding= node.Text.get_text(max(0,loc-8), loc)
            //     settable = node implements EditableText && selection settable
            //     caret_query_ok = get_text(loc,loc) succeeded
            //     app_id/app_name = application accessible name / toolkit / get_application()
            //   Then classification proceeds and, if naturalLanguage+liveEditable, run the session.
            eprintln!(">> dictation-target: focus resolution not wired on this build; classifying as unknown");
            ClassifyContext { trusted: true, focus: None }
        }
    }
}

async fn connect() -> Result<(), Box<dyn std::error::Error>> {
    // TODO(on-device): confirm the constructor against pinned atspi 0.28. Single real atspi symbol.
    atspi::AccessibilityConnection::new().await?;
    Ok(())
}

/// The live-editing ops backed by AT-SPI + `platform::inject`. Constructed only when a
/// naturalLanguage + liveEditable target exists (currently unreachable until focus is wired).
pub struct AtspiSessionOps {
    injector: platform::inject::Injector,
}

impl AtspiSessionOps {
    pub fn new(server: platform::display::DisplayServer) -> Option<AtspiSessionOps> {
        match platform::inject::Injector::new(server) {
            Ok(injector) => Some(AtspiSessionOps { injector }),
            Err(e) => {
                eprintln!(">> injector unavailable ({e})");
                None
            }
        }
    }
}

impl SessionOps for AtspiSessionOps {
    fn focus_matches(&self) -> bool {
        // TODO(on-device): re-resolve focus + match captured identity (pid/role/subrole/id/label).
        false
    }
    fn string_for_range(&self, _range: (i64, i64)) -> Option<String> {
        // TODO(on-device): AT-SPI Text.get_text(range).
        None
    }
    fn selection_range(&self) -> Option<(i64, i64)> {
        // TODO(on-device): AT-SPI Text.get_selection(0).
        None
    }
    fn replace_range(&mut self, _range: (i64, i64), text: &str) -> bool {
        // TODO(on-device): AT-SPI select `range`, then inject. On X11 this is a real XTEST type;
        // on Wayland it degrades to clipboard-paste (and liveEditable would already be false, so we
        // should not reach here on Wayland).
        if text.is_empty() {
            self.injector.press_backspace()
        } else {
            self.injector.type_text(text)
        }
    }
    fn wait_for_tracked(&self, _range: (i64, i64), _text: &str, _cursor: (i64, i64), _timeout_ms: u64) -> bool {
        // TODO(on-device): poll Text.get_text + get_selection until match or timeout.
        false
    }
    fn set_selection(&mut self, _range: (i64, i64)) -> bool {
        // TODO(on-device): AT-SPI Text.set_selection / set_caret_offset.
        false
    }
}
