use std::cell::RefCell;

use objc2::{rc::Retained, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBox, NSBoxType, NSColor, NSFont, NSPanel, NSScreen, NSTextField,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::DictFeedback;

const WIDTH: f64 = 252.0;
const HEIGHT: f64 = 50.0;
const EDGE_MARGIN: f64 = 12.0;
const SCREEN_SAVER_LEVEL: isize = 1000;

struct Pill {
    panel: Retained<NSPanel>,
    marker: Retained<NSTextField>,
    title: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    timer: Retained<NSTextField>,
}

thread_local! {
    static PILL: RefCell<Option<Pill>> = const { RefCell::new(None) };
}

impl Pill {
    fn new(mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setIgnoresMouseEvents(true);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(false);
        panel.setLevel(SCREEN_SAVER_LEVEL);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
        background.setBoxType(NSBoxType::Custom);
        background.setBorderWidth(0.6);
        background.setCornerRadius(HEIGHT / 2.0);
        background.setBorderColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.0, 0.0, 0.0, 0.12,
        ));
        background.setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.99, 0.99, 1.0, 0.98,
        ));
        panel.setContentView(Some(&background));

        let marker = label(
            "●",
            NSRect::new(NSPoint::new(15.0, 19.0), NSSize::new(13.0, 18.0)),
            12.0,
            mtm,
        );
        let title = label(
            "Listening",
            NSRect::new(NSPoint::new(39.0, 23.0), NSSize::new(155.0, 19.0)),
            13.5,
            mtm,
        );
        title.setFont(Some(&NSFont::boldSystemFontOfSize(13.5)));
        let detail = label(
            "Right ⌘ again to finish  ·  Esc cancels",
            NSRect::new(NSPoint::new(39.0, 7.0), NSSize::new(199.0, 16.0)),
            9.5,
            mtm,
        );
        let timer = label(
            "0:00",
            NSRect::new(NSPoint::new(198.0, 23.0), NSSize::new(40.0, 18.0)),
            11.0,
            mtm,
        );
        marker.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.90, 0.28, 0.30, 1.0,
        )));
        title.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.08, 0.08, 0.10, 1.0,
        )));
        detail.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.40, 0.40, 0.43, 1.0,
        )));
        timer.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.29, 0.29, 0.32, 1.0,
        )));
        background.addSubview(&marker);
        background.addSubview(&title);
        background.addSubview(&detail);
        background.addSubview(&timer);

        Self {
            panel,
            marker,
            title,
            detail,
            timer,
        }
    }

    fn update(&self, phase: DictFeedback, elapsed_seconds: u64, trigger_label: &str) {
        match phase {
            DictFeedback::Listening => {
                self.marker.setStringValue(&NSString::from_str("●"));
                self.marker
                    .setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                        0.90, 0.28, 0.30, 1.0,
                    )));
                self.title.setStringValue(&NSString::from_str("Listening"));
                let trigger = if trigger_label.trim().is_empty() {
                    "Shortcut"
                } else {
                    trigger_label.trim()
                };
                self.detail.setStringValue(&NSString::from_str(&format!(
                    "{trigger} again to finish  ·  Esc cancels"
                )));
                self.timer.setHidden(false);
                self.timer.setStringValue(&NSString::from_str(&format!(
                    "{}:{:02}",
                    elapsed_seconds / 60,
                    elapsed_seconds % 60
                )));
            }
            DictFeedback::Transcribing => {
                self.marker.setStringValue(&NSString::from_str("◌"));
                self.marker
                    .setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                        0.35, 0.55, 1.0, 1.0,
                    )));
                self.title
                    .setStringValue(&NSString::from_str("Polishing your words…"));
                self.detail
                    .setStringValue(&NSString::from_str("This usually takes a moment"));
                self.timer.setHidden(true);
            }
            DictFeedback::Idle => return,
        }
    }

    fn position(&self, mtm: MainThreadMarker, position: u8) {
        let Some(screen) = NSScreen::mainScreen(mtm) else {
            return;
        };
        let frame = screen.visibleFrame();
        let (x, y) = origin(
            position,
            (
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ),
            WIDTH,
            HEIGHT,
        );
        self.panel.setFrameOrigin(NSPoint::new(x, y));
    }
}

fn label(text: &str, frame: NSRect, size: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label
}

pub fn set(phase: DictFeedback, position: u8, elapsed_seconds: u64, trigger_label: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("dictation: native pill update attempted off the AppKit main thread");
        return;
    };
    PILL.with_borrow_mut(|slot| {
        if phase == DictFeedback::Idle {
            if let Some(pill) = slot.as_ref() {
                pill.panel.orderOut(None);
            }
            return;
        }
        let pill = slot.get_or_insert_with(|| Pill::new(mtm));
        pill.update(phase, elapsed_seconds, trigger_label);
        pill.position(mtm, position);
        pill.panel.orderFrontRegardless();
    });
}

fn origin(position: u8, screen: (f64, f64, f64, f64), width: f64, height: f64) -> (f64, f64) {
    let (sx, sy, sw, sh) = screen;
    let left = sx + EDGE_MARGIN;
    let right = sx + (sw - width - EDGE_MARGIN).max(EDGE_MARGIN);
    let bottom = sy + EDGE_MARGIN;
    let top = sy + (sh - height - EDGE_MARGIN).max(EDGE_MARGIN);
    let center_x = sx + ((sw - width) / 2.0).max(EDGE_MARGIN);
    let center_y = sy + ((sh - height) / 2.0).max(EDGE_MARGIN);
    match position {
        1 => (right, bottom),
        2 => (right, top),
        3 => (center_x, top),
        4 => (left, center_y),
        5 => (right, center_y),
        _ => (center_x, bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_cover_all_six_edges_in_global_screen_coordinates() {
        let screen = (-1920.0, 25.0, 1920.0, 1055.0);
        assert_eq!(origin(0, screen, WIDTH, HEIGHT), (-1086.0, 37.0));
        assert_eq!(origin(1, screen, WIDTH, HEIGHT), (-264.0, 37.0));
        assert_eq!(origin(2, screen, WIDTH, HEIGHT), (-264.0, 1018.0));
        assert_eq!(origin(3, screen, WIDTH, HEIGHT), (-1086.0, 1018.0));
        assert_eq!(origin(4, screen, WIDTH, HEIGHT), (-1908.0, 527.5));
        assert_eq!(origin(5, screen, WIDTH, HEIGHT), (-264.0, 527.5));
    }

    #[test]
    fn tiny_screen_never_places_origin_before_its_visible_frame() {
        let (x, y) = origin(2, (100.0, 200.0, 100.0, 40.0), WIDTH, HEIGHT);
        assert!(x >= 112.0);
        assert!(y >= 212.0);
    }
}
