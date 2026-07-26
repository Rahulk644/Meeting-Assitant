use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, rc::Retained, sel, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua,
    NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor, NSFont, NSImageScaling,
    NSImageView, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::local_server::MeetingUiState;

const NUB_SIZE: f64 = 52.0;
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 420.0;
const EDGE_MARGIN: f64 = 14.0;
const SCREEN_SAVER_LEVEL: isize = 1000;
static CONTENT_PROTECTED: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class has no Rust-owned ivars or Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct MeetingPillActions;

    unsafe impl NSObjectProtocol for MeetingPillActions {}

    impl MeetingPillActions {
        #[unsafe(method(pause:))]
        fn pause(&self, _sender: &AnyObject) {
            dispatch_control("pause");
        }

        #[unsafe(method(resume:))]
        fn resume(&self, _sender: &AnyObject) {
            dispatch_control("resume");
        }

        #[unsafe(method(stop:))]
        fn stop(&self, _sender: &AnyObject) {
            dispatch_control("stop");
        }

        #[unsafe(method(open:))]
        fn open(&self, _sender: &AnyObject) {
            crate::show_main_window();
        }

        #[unsafe(method(toggle:))]
        fn toggle(&self, _sender: &AnyObject) {
            toggle_expanded();
        }

        #[unsafe(method(catchUp:))]
        fn catch_up(&self, _sender: &AnyObject) {
            set_assist_loading();
            std::thread::spawn(|| {
                crate::present_native_catch_up(
                    crate::local_server::meeting_live_assist_action("catch_up"),
                );
            });
        }
    }
);

impl MeetingPillActions {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init signature and method family are defined by Foundation.
        unsafe { msg_send![super(this), init] }
    }
}

fn dispatch_control(action: &'static str) {
    // Resume can warm the speech engine, so target/action must return to Zoom's event loop immediately.
    std::thread::spawn(move || {
        if !crate::local_server::meeting_control_action(action) {
            log::warn!("meeting pill ignored unavailable action={action}");
        }
    });
}

struct MeetingPill {
    panel: Retained<NSPanel>,
    container: Retained<NSView>,
    background: Retained<NSBox>,
    dot: Retained<NSBox>,
    app_icon: Retained<NSImageView>,
    toggle_label: Retained<NSTextField>,
    toggle: Retained<NSButton>,
    title: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    transcript_heading: Retained<NSTextField>,
    transcript_backgrounds: Vec<Retained<NSBox>>,
    transcript_speakers: Vec<Retained<NSTextField>>,
    transcript_lines: Vec<Retained<NSTextField>>,
    transcript_empty: Retained<NSTextField>,
    assist_background: Retained<NSBox>,
    assist_result: Retained<NSTextField>,
    catch_up_background: Retained<NSBox>,
    catch_up_label: Retained<NSTextField>,
    catch_up: Retained<NSButton>,
    primary_background: Retained<NSBox>,
    primary_label: Retained<NSTextField>,
    primary: Retained<NSButton>,
    stop_background: Retained<NSBox>,
    stop_label: Retained<NSTextField>,
    stop: Retained<NSButton>,
    _actions: Retained<MeetingPillActions>,
    expanded: bool,
    primary_available: bool,
    stop_available: bool,
    transcript_count: usize,
    assist_available: bool,
    assist_loading: bool,
}

thread_local! {
    static PILL: RefCell<Option<MeetingPill>> = const { RefCell::new(None) };
}

impl MeetingPill {
    fn new(mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(NUB_SIZE, NUB_SIZE));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        // SAFETY: Retained Rust ownership outlives the NSPanel and closing must not hand ownership to AppKit.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setIgnoresMouseEvents(false);
        panel.setMovableByWindowBackground(true);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setHidesOnDeactivate(false);
        panel.setWorksWhenModal(true);
        panel.setLevel(SCREEN_SAVER_LEVEL);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setSharingType(if CONTENT_PROTECTED.load(Ordering::Relaxed) {
            NSWindowSharingType::None
        } else {
            NSWindowSharingType::ReadOnly
        });
        if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua })
        {
            panel.setAppearance(Some(&appearance));
        }

        let container = NSView::initWithFrame(NSView::alloc(mtm), frame);
        panel.setContentView(Some(&container));

        let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
        background.setBoxType(NSBoxType::Custom);
        background.setBorderWidth(0.75);
        background.setCornerRadius(18.0);
        background.setBorderColor(&color(1.0, 1.0, 1.0, 0.18));
        background.setFillColor(&color(0.075, 0.075, 0.085, 0.97));
        container.addSubview(&background);

        let dot = box_view(
            NSRect::new(NSPoint::new(37.0, 38.0), NSSize::new(7.0, 7.0)),
            3.5,
            color(0.98, 0.66, 0.20, 1.0),
            mtm,
        );
        container.addSubview(&dot);

        let app_icon = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(NSPoint::new(13.0, 14.0), NSSize::new(26.0, 26.0)),
        );
        if let Some(icon) = NSApplication::sharedApplication(mtm).applicationIconImage() {
            app_icon.setImage(Some(&icon));
        }
        app_icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        app_icon.setAccessibilityLabel(Some(&NSString::from_str("My Assistant")));
        container.addSubview(&app_icon);

        let toggle_label = label(
            "⌄",
            NSRect::new(NSPoint::new(0.0, 1.0), NSSize::new(NUB_SIZE, 10.0)),
            8.0,
            mtm,
        );
        toggle_label.setFont(Some(&NSFont::boldSystemFontOfSize(8.0)));
        toggle_label.setTextColor(Some(&color(0.55, 0.55, 0.60, 1.0)));
        toggle_label.setAlignment(NSTextAlignment::Center);
        container.addSubview(&toggle_label);

        let title = label(
            "Meeting",
            NSRect::new(NSPoint::new(48.0, 384.0), NSSize::new(255.0, 20.0)),
            15.0,
            mtm,
        );
        title.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
        title.setTextColor(Some(&color(0.97, 0.97, 0.98, 1.0)));
        container.addSubview(&title);

        let detail = label(
            "Starting local capture",
            NSRect::new(NSPoint::new(48.0, 363.0), NSSize::new(290.0, 16.0)),
            10.5,
            mtm,
        );
        detail.setTextColor(Some(&color(0.68, 0.68, 0.73, 1.0)));
        container.addSubview(&detail);

        let actions = MeetingPillActions::new(mtm);
        let transcript_heading = label(
            "Live transcript",
            NSRect::new(NSPoint::new(16.0, 332.0), NSSize::new(210.0, 17.0)),
            11.5,
            mtm,
        );
        transcript_heading.setFont(Some(&NSFont::boldSystemFontOfSize(11.5)));
        transcript_heading.setTextColor(Some(&color(0.78, 0.78, 0.82, 1.0)));
        container.addSubview(&transcript_heading);

        let mut transcript_backgrounds = Vec::new();
        let mut transcript_speakers = Vec::new();
        let mut transcript_lines = Vec::new();
        for index in 0..5 {
            let y = 282.0 - (index as f64 * 47.0);
            let bubble = button_background(
                NSRect::new(NSPoint::new(16.0, y), NSSize::new(292.0, 42.0)),
                color(0.145, 0.145, 0.165, 0.98),
                color(1.0, 1.0, 1.0, 0.07),
                mtm,
            );
            bubble.setCornerRadius(11.0);
            container.addSubview(&bubble);

            let speaker = label(
                "",
                NSRect::new(NSPoint::new(25.0, y + 24.0), NSSize::new(268.0, 13.0)),
                9.5,
                mtm,
            );
            speaker.setFont(Some(&NSFont::boldSystemFontOfSize(9.5)));
            speaker.setTextColor(Some(&color(0.61, 0.63, 0.69, 1.0)));
            container.addSubview(&speaker);

            let line = label(
                "",
                NSRect::new(NSPoint::new(25.0, y + 4.0), NSSize::new(268.0, 20.0)),
                11.0,
                mtm,
            );
            line.setUsesSingleLineMode(false);
            line.setMaximumNumberOfLines(2);
            line.setTextColor(Some(&color(0.91, 0.91, 0.93, 1.0)));
            container.addSubview(&line);

            transcript_backgrounds.push(bubble);
            transcript_speakers.push(speaker);
            transcript_lines.push(line);
        }

        let transcript_empty = label(
            "Transcript appears here after each natural pause.",
            NSRect::new(NSPoint::new(40.0, 132.0), NSSize::new(280.0, 34.0)),
            11.0,
            mtm,
        );
        transcript_empty.setUsesSingleLineMode(false);
        transcript_empty.setMaximumNumberOfLines(2);
        transcript_empty.setTextColor(Some(&color(0.56, 0.56, 0.61, 1.0)));
        transcript_empty.setAlignment(NSTextAlignment::Center);
        container.addSubview(&transcript_empty);

        let assist_frame = NSRect::new(NSPoint::new(16.0, 51.0), NSSize::new(328.0, 39.0));
        let assist_background = button_background(
            assist_frame,
            color(0.125, 0.115, 0.17, 0.98),
            color(0.50, 0.39, 0.82, 0.30),
            mtm,
        );
        assist_background.setCornerRadius(10.0);
        container.addSubview(&assist_background);

        let assist_result = label(
            "Ask for a private catch-up from the last minute.",
            NSRect::new(NSPoint::new(25.0, 55.0), NSSize::new(310.0, 30.0)),
            10.5,
            mtm,
        );
        assist_result.setUsesSingleLineMode(false);
        assist_result.setMaximumNumberOfLines(3);
        assist_result.setTextColor(Some(&color(0.75, 0.73, 0.82, 1.0)));
        container.addSubview(&assist_result);

        let catch_up_frame = NSRect::new(NSPoint::new(16.0, 10.0), NSSize::new(168.0, 34.0));
        let catch_up_background = button_background(
            catch_up_frame,
            color(0.46, 0.85, 0.66, 1.0),
            color(0.75, 1.0, 0.86, 0.32),
            mtm,
        );
        container.addSubview(&catch_up_background);
        let catch_up_label = label("What did I miss?", catch_up_frame, 11.0, mtm);
        catch_up_label.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
        catch_up_label.setTextColor(Some(&color(0.055, 0.115, 0.085, 1.0)));
        catch_up_label.setAlignment(NSTextAlignment::Center);
        container.addSubview(&catch_up_label);
        let catch_up = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(""),
                Some(&*actions),
                Some(sel!(catchUp:)),
                mtm,
            )
        };
        catch_up.setFrame(catch_up_frame);
        catch_up.setBordered(false);
        catch_up.setAccessibilityLabel(Some(&NSString::from_str(
            "What did I miss in the last minute?",
        )));
        catch_up.setToolTip(Some(&NSString::from_str(
            "Summarize the last minute from the live transcript",
        )));
        container.addSubview(&catch_up);

        let primary_frame = NSRect::new(NSPoint::new(190.0, 10.0), NSSize::new(72.0, 34.0));
        let primary_background = button_background(
            primary_frame,
            color(0.17, 0.17, 0.19, 1.0),
            color(1.0, 1.0, 1.0, 0.12),
            mtm,
        );
        container.addSubview(&primary_background);
        let primary_label = label("Open", primary_frame, 11.0, mtm);
        primary_label.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
        primary_label.setTextColor(Some(&color(0.97, 0.97, 0.98, 1.0)));
        primary_label.setAlignment(NSTextAlignment::Center);
        container.addSubview(&primary_label);
        // SAFETY: The target is retained by this struct and each selector is implemented above.
        let primary = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(""),
                Some(&*actions),
                Some(sel!(open:)),
                mtm,
            )
        };
        primary.setFrame(primary_frame);
        primary.setBordered(false);
        primary.setAccessibilityLabel(Some(&NSString::from_str("Open My Assistant")));
        primary.setToolTip(Some(&NSString::from_str("Open My Assistant")));
        container.addSubview(&primary);

        let stop_frame = NSRect::new(NSPoint::new(268.0, 10.0), NSSize::new(76.0, 34.0));
        let stop_background = button_background(
            stop_frame,
            color(0.26, 0.11, 0.13, 1.0),
            color(0.96, 0.32, 0.36, 0.36),
            mtm,
        );
        container.addSubview(&stop_background);
        let stop_label = label("Stop", stop_frame, 11.0, mtm);
        stop_label.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
        stop_label.setTextColor(Some(&color(1.0, 1.0, 1.0, 1.0)));
        stop_label.setAlignment(NSTextAlignment::Center);
        container.addSubview(&stop_label);
        // SAFETY: Same retained target/action contract as the primary button.
        let stop = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(""),
                Some(&*actions),
                Some(sel!(stop:)),
                mtm,
            )
        };
        stop.setFrame(stop_frame);
        stop.setBordered(false);
        stop.setAccessibilityLabel(Some(&NSString::from_str(
            "Stop recording and finalize this meeting",
        )));
        stop.setToolTip(Some(&NSString::from_str(
            "Stop recording and finalize this meeting",
        )));
        container.addSubview(&stop);

        let toggle = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(""),
                Some(&*actions),
                Some(sel!(toggle:)),
                mtm,
            )
        };
        toggle.setFrame(frame);
        toggle.setBordered(false);
        toggle.setAccessibilityLabel(Some(&NSString::from_str("Expand live meeting transcript")));
        toggle.setToolTip(Some(&NSString::from_str(
            "Expand or collapse the live meeting control",
        )));
        container.addSubview(&toggle);

        title.setHidden(true);
        detail.setHidden(true);
        transcript_heading.setHidden(true);
        for background in &transcript_backgrounds {
            background.setHidden(true);
        }
        for speaker in &transcript_speakers {
            speaker.setHidden(true);
        }
        for line in &transcript_lines {
            line.setHidden(true);
        }
        transcript_empty.setHidden(true);
        assist_background.setHidden(true);
        assist_result.setHidden(true);
        catch_up_background.setHidden(true);
        catch_up_label.setHidden(true);
        catch_up.setHidden(true);
        primary_background.setHidden(true);
        primary_label.setHidden(true);
        primary.setHidden(true);
        stop_background.setHidden(true);
        stop_label.setHidden(true);
        stop.setHidden(true);

        Self {
            panel,
            container,
            background,
            dot,
            app_icon,
            toggle_label,
            toggle,
            title,
            detail,
            transcript_heading,
            transcript_backgrounds,
            transcript_speakers,
            transcript_lines,
            transcript_empty,
            assist_background,
            assist_result,
            catch_up_background,
            catch_up_label,
            catch_up,
            primary_background,
            primary_label,
            primary,
            stop_background,
            stop_label,
            stop,
            _actions: actions,
            expanded: false,
            primary_available: false,
            stop_available: true,
            transcript_count: 0,
            assist_available: false,
            assist_loading: false,
        }
    }

    fn update(&mut self, state: &MeetingUiState) {
        let app = app_label(&state.app);
        let meeting_title = if state.title.trim().is_empty()
            || state.title.trim().eq_ignore_ascii_case("impromptu meeting")
        {
            if app == "this meeting" {
                "Live meeting".to_string()
            } else {
                format!("{app} meeting")
            }
        } else {
            compact_overlay_text(&state.title, 48)
        };
        let (status, dot_color, primary, action) = match state.phase.as_str() {
            "starting" => (
                "Starting local capture",
                color(0.98, 0.66, 0.20, 1.0),
                Some("Open"),
                Some(sel!(open:)),
            ),
            "recording" => (
                "Recording locally",
                color(0.90, 0.28, 0.30, 1.0),
                Some("Pause"),
                Some(sel!(pause:)),
            ),
            "needs_attention" => (
                "Audio needs attention",
                color(1.0, 0.52, 0.18, 1.0),
                Some("Open"),
                Some(sel!(open:)),
            ),
            "pausing" => ("Pausing capture", color(0.68, 0.58, 0.34, 1.0), None, None),
            "paused" => (
                "Capture paused",
                color(0.58, 0.58, 0.62, 1.0),
                Some("Resume"),
                Some(sel!(resume:)),
            ),
            "finalizing" => (
                "Finalizing locally",
                color(0.57, 0.44, 0.95, 1.0),
                None,
                None,
            ),
            _ => return,
        };
        let detail = match state.phase.as_str() {
            "recording" => format!("{app} · {status} · Mic + meeting audio ready"),
            "needs_attention" => {
                format!("{app} · {}", compact_overlay_text(&state.detail, 64))
            }
            "finalizing" => format!("{app} · Saving transcript and preparing notes"),
            _ => format!("{app} · {status}"),
        };
        self.title
            .setStringValue(&NSString::from_str(&meeting_title));
        self.detail.setStringValue(&NSString::from_str(&detail));
        self.dot.setFillColor(&dot_color);
        self.primary_available = primary.is_some();
        let stop_hidden = state.phase == "finalizing";
        self.stop_available = !stop_hidden;
        self.toggle
            .setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{status} for {meeting_title}. {} live meeting transcript",
                if self.expanded { "Collapse" } else { "Expand" }
            ))));
        if let (Some(title), Some(action)) = (primary, action) {
            self.primary_label
                .setStringValue(&NSString::from_str(title));
            self.primary
                .setAccessibilityLabel(Some(&NSString::from_str(match title {
                    "Pause" => "Pause recording",
                    "Resume" => "Resume recording",
                    _ => "Open My Assistant",
                })));
            // SAFETY: Every selected action is implemented by the retained MeetingPillActions target.
            unsafe { self.primary.setAction(Some(action)) };
            self.primary
                .setToolTip(Some(&NSString::from_str(match title {
                    "Pause" => "Pause and release both audio sources",
                    "Resume" => "Resume microphone and meeting audio capture",
                    _ => "Open My Assistant",
                })));
        }
        self.transcript_count = state.transcript.len().min(self.transcript_lines.len());
        self.assist_available = self.transcript_count > 0 && state.phase != "finalizing";
        for index in 0..self.transcript_lines.len() {
            let Some(line) = state.transcript.get(index) else {
                self.transcript_speakers[index].setStringValue(&NSString::from_str(""));
                self.transcript_lines[index].setStringValue(&NSString::from_str(""));
                continue;
            };
            let y = 94.0 + ((self.transcript_count - 1 - index) as f64 * 47.0);
            let x = if line.is_self { 52.0 } else { 16.0 };
            let speaker = if line.is_self {
                "You".to_string()
            } else if line.speaker.trim().is_empty() {
                "Other speaker".to_string()
            } else {
                compact_overlay_text(&line.speaker, 32)
            };
            self.transcript_backgrounds[index]
                .setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(292.0, 42.0)));
            let bubble_fill = if line.is_self {
                color(0.085, 0.205, 0.155, 0.98)
            } else {
                color(0.145, 0.145, 0.165, 0.98)
            };
            let bubble_border = if line.is_self {
                color(0.43, 0.90, 0.67, 0.18)
            } else {
                color(1.0, 1.0, 1.0, 0.07)
            };
            self.transcript_backgrounds[index].setFillColor(&bubble_fill);
            self.transcript_backgrounds[index].setBorderColor(&bubble_border);
            self.transcript_speakers[index].setFrame(NSRect::new(
                NSPoint::new(x + 9.0, y + 24.0),
                NSSize::new(274.0, 13.0),
            ));
            let speaker_color = if line.is_self {
                color(0.43, 0.90, 0.67, 1.0)
            } else {
                color(0.61, 0.63, 0.69, 1.0)
            };
            self.transcript_speakers[index].setTextColor(Some(&speaker_color));
            self.transcript_speakers[index].setStringValue(&NSString::from_str(&speaker));
            self.transcript_lines[index].setFrame(NSRect::new(
                NSPoint::new(x + 9.0, y + 4.0),
                NSSize::new(274.0, 20.0),
            ));
            self.transcript_lines[index]
                .setStringValue(&NSString::from_str(&compact_overlay_text(&line.text, 150)));
        }
        if !self.assist_loading {
            self.catch_up.setEnabled(self.assist_available);
            let catch_up_fill = if self.assist_available {
                color(0.46, 0.85, 0.66, 1.0)
            } else {
                color(0.18, 0.18, 0.20, 1.0)
            };
            let catch_up_text = if self.assist_available {
                color(0.055, 0.115, 0.085, 1.0)
            } else {
                color(0.61, 0.61, 0.66, 1.0)
            };
            self.catch_up_background.setFillColor(&catch_up_fill);
            self.catch_up_label.setTextColor(Some(&catch_up_text));
            self.catch_up_label.setStringValue(&NSString::from_str(
                if state.phase == "finalizing" {
                    "Finalizing…"
                } else if self.transcript_count == 0 {
                    "Waiting for speech"
                } else {
                    "What did I miss?"
                },
            ));
        }
        if self.transcript_count == 0 {
            self.assist_result.setStringValue(&NSString::from_str(
                if state.phase == "finalizing" {
                    "Your transcript is safe. Finishing the local meeting record."
                } else {
                    "Catch-up becomes available after the first transcript line."
                },
            ));
        }
        if self.expanded {
            self.resize_expanded_for_content();
        }
        self.apply_visibility();
    }

    fn position(&self, mtm: MainThreadMarker) {
        let screens = NSScreen::screens(mtm);
        let Some(screen) = NSScreen::mainScreen(mtm).or_else(|| screens.iter().next()) else {
            return;
        };
        let frame = screen.visibleFrame();
        let x = frame.origin.x + (frame.size.width - NUB_SIZE) / 2.0;
        let y = frame.origin.y + frame.size.height - NUB_SIZE - EDGE_MARGIN;
        self.panel.setFrameOrigin(NSPoint::new(x, y));
    }

    fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
        let size = if self.expanded {
            NSSize::new(WIDTH, expanded_height(self.transcript_count))
        } else {
            NSSize::new(NUB_SIZE, NUB_SIZE)
        };
        self.resize_anchored(size);
        self.layout_mode();
        self.toggle_label
            .setStringValue(&NSString::from_str(if self.expanded {
                "⌃"
            } else {
                "⌄"
            }));
        self.toggle
            .setAccessibilityLabel(Some(&NSString::from_str(if self.expanded {
                "Collapse live meeting transcript"
            } else {
                "Expand live meeting transcript"
            })));
        self.apply_visibility();
    }

    fn resize_expanded_for_content(&self) {
        let height = expanded_height(self.transcript_count);
        if (self.panel.frame().size.height - height).abs() < 0.5 {
            return;
        }
        self.resize_anchored(NSSize::new(WIDTH, height));
        self.layout_mode();
    }

    fn resize_anchored(&self, size: NSSize) {
        let old = self.panel.frame();
        // Keep the user's chosen top-right anchor stable while changing modes.
        let frame = NSRect::new(
            NSPoint::new(
                old.origin.x + old.size.width - size.width,
                old.origin.y + old.size.height - size.height,
            ),
            size,
        );
        self.panel.setFrame_display_animate(frame, true, true);
        self.container
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        self.background
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
    }

    fn layout_mode(&self) {
        let height = self.panel.frame().size.height;
        self.background
            .setCornerRadius(if self.expanded { 20.0 } else { 18.0 });
        self.app_icon.setFrame(if self.expanded {
            NSRect::new(NSPoint::new(16.0, height - 40.0), NSSize::new(24.0, 24.0))
        } else {
            NSRect::new(NSPoint::new(13.0, 14.0), NSSize::new(26.0, 26.0))
        });
        self.dot.setFrame(if self.expanded {
            NSRect::new(NSPoint::new(34.0, height - 23.0), NSSize::new(6.0, 6.0))
        } else {
            NSRect::new(NSPoint::new(37.0, 38.0), NSSize::new(7.0, 7.0))
        });
        self.title.setFrame(NSRect::new(
            NSPoint::new(48.0, height - 36.0),
            NSSize::new(255.0, 20.0),
        ));
        self.detail.setFrame(NSRect::new(
            NSPoint::new(48.0, height - 57.0),
            NSSize::new(290.0, 16.0),
        ));
        self.transcript_heading.setFrame(NSRect::new(
            NSPoint::new(16.0, height - 88.0),
            NSSize::new(210.0, 17.0),
        ));
        self.toggle.setFrame(if self.expanded {
            NSRect::new(NSPoint::new(312.0, height - 44.0), NSSize::new(40.0, 36.0))
        } else {
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(NUB_SIZE, NUB_SIZE))
        });
        self.toggle_label.setFrame(if self.expanded {
            NSRect::new(NSPoint::new(316.0, height - 35.0), NSSize::new(32.0, 14.0))
        } else {
            NSRect::new(NSPoint::new(0.0, 1.0), NSSize::new(NUB_SIZE, 10.0))
        });
    }

    fn apply_visibility(&self) {
        self.title.setHidden(!self.expanded);
        self.detail.setHidden(!self.expanded);
        self.transcript_heading.setHidden(!self.expanded);
        for index in 0..self.transcript_lines.len() {
            let hidden = !self.expanded || index >= self.transcript_count;
            self.transcript_backgrounds[index].setHidden(hidden);
            self.transcript_speakers[index].setHidden(hidden);
            self.transcript_lines[index].setHidden(hidden);
        }
        self.transcript_empty
            .setHidden(!self.expanded || self.transcript_count > 0);
        self.assist_background.setHidden(!self.expanded);
        self.assist_result.setHidden(!self.expanded);
        self.catch_up_background.setHidden(!self.expanded);
        self.catch_up_label.setHidden(!self.expanded);
        self.catch_up.setHidden(!self.expanded);
        let primary_hidden = !self.expanded || !self.primary_available;
        self.primary_background.setHidden(primary_hidden);
        self.primary_label.setHidden(primary_hidden);
        self.primary.setHidden(primary_hidden);
        let stop_hidden = !self.expanded || !self.stop_available;
        self.stop_background.setHidden(stop_hidden);
        self.stop_label.setHidden(stop_hidden);
        self.stop.setHidden(stop_hidden);
    }

    #[cfg(debug_assertions)]
    fn write_preview_pdf(&self, path: &str) {
        self.container.layoutSubtreeIfNeeded();
        // Force one complete AppKit draw pass before asking NSView for PDF data. A conditional
        // display can otherwise preserve a partially invalidated button or glyph layer.
        self.panel.display();
        self.container.display();
        let data = self
            .container
            .dataWithPDFInsideRect(self.container.bounds());
        if !data.writeToFile_atomically(&NSString::from_str(path), true) {
            log::error!("meeting: could not write native pill preview snapshot");
        }
    }
}

fn expanded_height(transcript_count: usize) -> f64 {
    (185.0 + transcript_count.min(5) as f64 * 47.0).clamp(280.0, HEIGHT)
}

fn app_label(app: &str) -> String {
    match app.trim().to_ascii_lowercase().as_str() {
        "zoom" => "Zoom".into(),
        "meet" => "Google Meet".into(),
        "teams" => "Microsoft Teams".into(),
        "webex" => "Webex".into(),
        "slack" => "Slack Huddle".into(),
        "facetime" => "FaceTime".into(),
        "" | "call" => "this meeting".into(),
        other => other.chars().take(36).collect(),
    }
}

fn color(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, alpha)
}

fn box_view(
    frame: NSRect,
    corner_radius: f64,
    fill: Retained<NSColor>,
    mtm: MainThreadMarker,
) -> Retained<NSBox> {
    let view = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    view.setBoxType(NSBoxType::Custom);
    view.setBorderWidth(0.0);
    view.setCornerRadius(corner_radius);
    view.setFillColor(&fill);
    view
}

fn button_background(
    frame: NSRect,
    fill: Retained<NSColor>,
    border: Retained<NSColor>,
    mtm: MainThreadMarker,
) -> Retained<NSBox> {
    let view = box_view(frame, 8.0, fill, mtm);
    view.setBorderWidth(0.5);
    view.setBorderColor(&border);
    view
}

fn label(text: &str, frame: NSRect, size: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setUsesSingleLineMode(true);
    label
}

pub fn set(state: &MeetingUiState) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("meeting: native pill update attempted off the AppKit main thread");
        return;
    };
    PILL.with_borrow_mut(|slot| {
        if state.phase == "idle" {
            if let Some(pill) = slot.as_mut() {
                if pill.expanded {
                    pill.toggle_expanded();
                }
                pill.panel.orderOut(None);
            }
            return;
        }
        let is_new = slot.is_none();
        let pill = slot.get_or_insert_with(|| MeetingPill::new(mtm));
        pill.update(state);
        if is_new {
            pill.position(mtm);
        }
        // NonactivatingPanel keeps Zoom focused while this remains visible across Spaces and full-screen.
        pill.panel.orderFrontRegardless();
    });
}

fn toggle_expanded() {
    PILL.with_borrow_mut(|slot| {
        if let Some(pill) = slot.as_mut() {
            pill.toggle_expanded();
        }
    });
}

fn set_assist_loading() {
    PILL.with_borrow_mut(|slot| {
        if let Some(pill) = slot.as_mut() {
            pill.assist_loading = true;
            pill.assist_result
                .setStringValue(&NSString::from_str("Catching you up from the last minute…"));
            pill.catch_up_label
                .setStringValue(&NSString::from_str("Working…"));
            pill.catch_up_background
                .setFillColor(&color(0.18, 0.18, 0.20, 1.0));
            pill.catch_up_label
                .setTextColor(Some(&color(0.78, 0.78, 0.82, 1.0)));
            pill.catch_up.setEnabled(false);
        }
    });
}

pub fn set_assist_result(result: Result<String, String>) {
    let value = match result {
        Ok(answer) => compact_overlay_text(&answer, 420),
        Err(error) => format!("Couldn’t catch up · {}", compact_overlay_text(&error, 260)),
    };
    PILL.with_borrow_mut(|slot| {
        if let Some(pill) = slot.as_mut() {
            pill.assist_loading = false;
            pill.assist_result
                .setStringValue(&NSString::from_str(&value));
            pill.catch_up_label
                .setStringValue(&NSString::from_str("What did I miss?"));
            pill.catch_up.setEnabled(pill.assist_available);
            let fill = if pill.assist_available {
                color(0.46, 0.85, 0.66, 1.0)
            } else {
                color(0.18, 0.18, 0.20, 1.0)
            };
            let text = if pill.assist_available {
                color(0.055, 0.115, 0.085, 1.0)
            } else {
                color(0.61, 0.61, 0.66, 1.0)
            };
            pill.catch_up_background.setFillColor(&fill);
            pill.catch_up_label.setTextColor(Some(&text));
        }
    });
}

fn compact_overlay_text(value: &str, cap: usize) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    value.chars().take(cap).collect()
}

#[cfg(debug_assertions)]
pub fn write_preview_snapshot(path: &str) {
    let Some(_mtm) = MainThreadMarker::new() else {
        log::error!("meeting: native pill snapshot attempted off the AppKit main thread");
        return;
    };
    PILL.with_borrow(|slot| {
        if let Some(pill) = slot.as_ref() {
            pill.write_preview_pdf(path);
        }
    });
}

#[cfg(debug_assertions)]
pub fn expand_for_preview() {
    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    PILL.with_borrow_mut(|slot| {
        if let Some(pill) = slot.as_mut() {
            if !pill.expanded {
                pill.toggle_expanded();
            }
        }
    });
}

pub fn set_content_protected(protected: bool) {
    CONTENT_PROTECTED.store(protected, Ordering::Relaxed);
    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    PILL.with_borrow(|slot| {
        if let Some(pill) = slot.as_ref() {
            pill.panel.setSharingType(if protected {
                NSWindowSharingType::None
            } else {
                NSWindowSharingType::ReadOnly
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_labels_are_user_facing() {
        assert_eq!(app_label("zoom"), "Zoom");
        assert_eq!(app_label("meet"), "Google Meet");
        assert_eq!(app_label(""), "this meeting");
    }

    #[test]
    fn meeting_nub_stays_tiny_and_expansion_remains_bounded() {
        let dimensions = (NUB_SIZE, WIDTH, HEIGHT);
        assert!(dimensions.0 <= 56.0);
        assert!(dimensions.1 <= 380.0);
        assert!(dimensions.2 <= 440.0);
        assert!(dimensions.1 < dimensions.2);
        assert_eq!(expanded_height(0), 280.0);
        assert_eq!(expanded_height(2), 280.0);
        assert_eq!(expanded_height(5), HEIGHT);
        assert_eq!(
            compact_overlay_text("  one\n two   three ", 80),
            "one two three"
        );
    }
}
