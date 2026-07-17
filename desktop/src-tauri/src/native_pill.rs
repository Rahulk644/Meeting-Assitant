use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::{rc::Retained, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBox,
    NSBoxType, NSColor, NSEvent, NSFont, NSPanel, NSProgressIndicator, NSProgressIndicatorStyle,
    NSScreen, NSTextField, NSView, NSWindowCollectionBehavior, NSWindowSharingType,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::DictFeedback;

// This is the compact July 10 pill, rebuilt as a native AppKit surface: a 32pt dark capsule that hugs its
// contents. Native ownership keeps it independent from the main WebView while the dimensions, colors, pulse,
// waveform, timer, spinner and copy match the original design.
const HEIGHT: f64 = 32.0;
// Original CSS geometry: 13 + 8 + 9 + 22.5 + 9 + 28 + 13 = 102.5pt.
const LISTENING_WIDTH: f64 = 103.0;
const STATUS_TEXT_X: f64 = 36.0;
const STATUS_TEXT_Y: f64 = 7.0;
const STATUS_TEXT_HEIGHT: f64 = 18.0;
const STATUS_RIGHT_MARGIN: f64 = 13.0;
const STATUS_TEXT_SAFETY: f64 = 2.0;
const CHANGE_MIN_WIDTH: f64 = 132.0;
const CHANGE_MARGIN: f64 = 13.0;
const CHANGE_GAP: f64 = 7.0;
const CHANGE_ARROW_WIDTH: f64 = 12.0;
const CHANGE_TEXT_MAX_WIDTH: f64 = 92.0;
const EDGE_MARGIN: f64 = 12.0;
const SCREEN_SAVER_LEVEL: isize = 1000;
static CONTENT_PROTECTED: AtomicBool = AtomicBool::new(false);

// These are visual weights, not an animation. Every bar is driven by the current microphone RMS level;
// the center bar carries the full envelope while the outer bars keep the compact waveform silhouette.
const VOICE_PROFILE: [f32; 5] = [0.58, 0.88, 1.0, 0.82, 0.55];
const PULSE_ALPHA: [f64; 8] = [1.0, 0.86, 0.66, 0.48, 0.40, 0.48, 0.66, 0.86];
const PULSE_SIZE: [f64; 8] = [8.0, 7.7, 7.2, 6.7, 6.4, 6.7, 7.2, 7.7];

struct Pill {
    panel: Retained<NSPanel>,
    container: Retained<NSView>,
    background: Retained<NSBox>,
    live_dot: Retained<NSBox>,
    wave_bars: Vec<Retained<NSBox>>,
    spinner: Retained<NSProgressIndicator>,
    icon: Retained<NSTextField>,
    title: Retained<NSTextField>,
    change_from: Retained<NSTextField>,
    change_strike: Retained<NSBox>,
    change_arrow: Retained<NSTextField>,
    change_to: Retained<NSTextField>,
    change_more: Retained<NSTextField>,
    timer: Retained<NSTextField>,
    width: Cell<f64>,
    height: Cell<f64>,
    bar_levels: Cell<[f32; 5]>,
}

thread_local! {
    static PILL: RefCell<Option<Pill>> = const { RefCell::new(None) };
}

impl Pill {
    fn new(mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTENING_WIDTH, HEIGHT));
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

        // NSBox applies an implicit content inset to children. That shifted every foreground element up and
        // right even though their frames were mathematically centered. Keep the box as a background-only
        // sibling and put all content directly in a plain NSView coordinate space.
        let container = NSView::initWithFrame(NSView::alloc(mtm), frame);
        panel.setContentView(Some(&container));

        let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
        background.setBoxType(NSBoxType::Custom);
        background.setBorderWidth(0.5);
        background.setCornerRadius(HEIGHT / 2.0);
        background.setBorderColor(&color(1.0, 1.0, 1.0, 0.12));
        background.setFillColor(&color(0.11, 0.11, 0.118, 1.0));
        container.addSubview(&background);

        let live_dot = box_view(
            NSRect::new(NSPoint::new(13.0, 12.0), NSSize::new(8.0, 8.0)),
            4.0,
            color(0.898, 0.282, 0.302, 1.0),
            mtm,
        );
        container.addSubview(&live_dot);

        let wave_colors = [
            (0.561, 0.690, 1.0),
            (0.470, 0.608, 1.0),
            (0.380, 0.526, 1.0),
            (0.286, 0.447, 1.0),
            (0.184, 0.420, 1.0),
        ];
        let mut wave_bars = Vec::with_capacity(5);
        for (index, &(red, green, blue)) in wave_colors.iter().enumerate() {
            let height = 3.5;
            let bar = box_view(
                NSRect::new(
                    NSPoint::new(30.0 + index as f64 * 5.0, (HEIGHT - height) / 2.0),
                    NSSize::new(2.75, height),
                ),
                1.25,
                color(red, green, blue, 1.0),
                mtm,
            );
            container.addSubview(&bar);
            wave_bars.push(bar);
        }

        let spinner = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            NSRect::new(NSPoint::new(13.0, 9.0), NSSize::new(14.0, 14.0)),
        );
        spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        spinner.setIndeterminate(true);
        spinner.setDisplayedWhenStopped(false);
        spinner.setHidden(true);
        container.addSubview(&spinner);

        let icon = label(
            "✓",
            NSRect::new(NSPoint::new(13.0, 7.0), NSSize::new(14.0, 18.0)),
            12.0,
            mtm,
        );
        icon.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
        icon.setHidden(true);
        container.addSubview(&icon);

        let title = label(
            "Transcribing…",
            NSRect::new(
                NSPoint::new(STATUS_TEXT_X, STATUS_TEXT_Y),
                NSSize::new(84.0, STATUS_TEXT_HEIGHT),
            ),
            12.0,
            mtm,
        );
        title.setTextColor(Some(&color(0.957, 0.957, 0.961, 1.0)));
        title.setHidden(true);
        container.addSubview(&title);

        // A short, color-independent proof of the most meaningful edit. The old text is muted and crossed
        // out; the replacement is both arrow-labelled and accent coloured. Full changes remain available in
        // Recent Dictations, so this temporary surface stays bounded and never steals focus.
        let change_from = label(
            "",
            NSRect::new(NSPoint::new(0.0, STATUS_TEXT_Y), NSSize::new(1.0, 18.0)),
            11.0,
            mtm,
        );
        change_from.setTextColor(Some(&NSColor::secondaryLabelColor()));
        change_from.setHidden(true);
        container.addSubview(&change_from);

        let change_strike = box_view(
            NSRect::new(NSPoint::new(0.0, 16.0), NSSize::new(1.0, 1.0)),
            0.5,
            color(0.90, 0.90, 0.92, 0.68),
            mtm,
        );
        change_strike.setHidden(true);
        container.addSubview(&change_strike);

        let change_arrow = label(
            "→",
            NSRect::new(
                NSPoint::new(0.0, STATUS_TEXT_Y),
                NSSize::new(CHANGE_ARROW_WIDTH, 18.0),
            ),
            11.0,
            mtm,
        );
        change_arrow.setTextColor(Some(&NSColor::secondaryLabelColor()));
        change_arrow.setHidden(true);
        container.addSubview(&change_arrow);

        let change_to = label(
            "",
            NSRect::new(NSPoint::new(0.0, STATUS_TEXT_Y), NSSize::new(1.0, 18.0)),
            11.0,
            mtm,
        );
        change_to.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
        change_to.setTextColor(Some(&NSColor::controlAccentColor()));
        change_to.setHidden(true);
        container.addSubview(&change_to);

        let change_more = label(
            "",
            NSRect::new(NSPoint::new(0.0, STATUS_TEXT_Y), NSSize::new(1.0, 18.0)),
            10.0,
            mtm,
        );
        change_more.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        change_more.setHidden(true);
        container.addSubview(&change_more);

        let timer = label(
            "0:00",
            NSRect::new(NSPoint::new(61.5, 7.0), NSSize::new(28.0, 18.0)),
            12.0,
            mtm,
        );
        timer.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
        timer.setTextColor(Some(&color(0.957, 0.957, 0.961, 1.0)));
        container.addSubview(&timer);

        Self {
            panel,
            container,
            background,
            live_dot,
            wave_bars,
            spinner,
            icon,
            title,
            change_from,
            change_strike,
            change_arrow,
            change_to,
            change_more,
            timer,
            width: Cell::new(LISTENING_WIDTH),
            height: Cell::new(HEIGHT),
            bar_levels: Cell::new([0.0; 5]),
        }
    }

    fn resize(&self, width: f64) {
        self.resize_to(width, HEIGHT);
    }

    fn resize_to(&self, width: f64, height: f64) {
        if (self.width.get() - width).abs() < f64::EPSILON
            && (self.height.get() - height).abs() < f64::EPSILON
        {
            return;
        }
        self.width.set(width);
        self.height.set(height);
        self.panel.setContentSize(NSSize::new(width, height));
        self.container.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        self.background.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        self.background.setCornerRadius(height / 2.0);
        self.title.setFrame(NSRect::new(
            NSPoint::new(STATUS_TEXT_X, STATUS_TEXT_Y),
            NSSize::new(
                (width - STATUS_TEXT_X - STATUS_RIGHT_MARGIN).max(20.0),
                STATUS_TEXT_HEIGHT,
            ),
        ));
    }

    fn reset_views(&self) {
        self.live_dot.setHidden(true);
        for bar in &self.wave_bars {
            bar.setHidden(true);
        }
        self.timer.setHidden(true);
        self.spinner.setHidden(true);
        self.icon.setHidden(true);
        self.title.setHidden(true);
        self.change_from.setHidden(true);
        self.change_strike.setHidden(true);
        self.change_arrow.setHidden(true);
        self.change_to.setHidden(true);
        self.change_more.setHidden(true);
        unsafe { self.spinner.stopAnimation(None) };
    }

    fn update(&self, phase: DictFeedback, elapsed_millis: u64, audio_level: f32) {
        self.background.setBorderColor(&color(1.0, 1.0, 1.0, 0.12));
        self.background.setFillColor(&color(0.11, 0.11, 0.118, 1.0));
        self.title
            .setTextColor(Some(&color(0.957, 0.957, 0.961, 1.0)));
        self.reset_views();
        self.resize(width_for(phase));
        self.spinner.setFrame(NSRect::new(
            NSPoint::new(13.0, 9.0),
            NSSize::new(14.0, 14.0),
        ));
        self.icon.setFrame(NSRect::new(
            NSPoint::new(13.0, 7.0),
            NSSize::new(14.0, 18.0),
        ));
        self.timer.setFrame(NSRect::new(
            NSPoint::new(61.5, 7.0),
            NSSize::new(28.0, 18.0),
        ));

        match phase {
            DictFeedback::Listening => {
                let animation_frame = ((elapsed_millis / 120) as usize) % PULSE_ALPHA.len();
                let dot_size = PULSE_SIZE[animation_frame];
                self.live_dot.setFrame(NSRect::new(
                    NSPoint::new(17.0 - dot_size / 2.0, 16.0 - dot_size / 2.0),
                    NSSize::new(dot_size, dot_size),
                ));
                self.live_dot.setCornerRadius(dot_size / 2.0);
                self.live_dot.setFillColor(&color(
                    0.898,
                    0.282,
                    0.302,
                    PULSE_ALPHA[animation_frame],
                ));
                self.live_dot.setHidden(false);
                let bar_levels = voice_bar_levels(audio_level, self.bar_levels.get());
                self.bar_levels.set(bar_levels);
                for (index, (bar, level)) in
                    self.wave_bars.iter().zip(bar_levels.iter()).enumerate()
                {
                    let height = 3.5 + 16.5 * f64::from(*level);
                    bar.setFrame(NSRect::new(
                        NSPoint::new(30.0 + index as f64 * 5.0, (HEIGHT - height) / 2.0),
                        NSSize::new(2.75, height),
                    ));
                    bar.setHidden(false);
                }
                let elapsed_seconds = elapsed_millis / 1_000;
                self.timer.setStringValue(&NSString::from_str(&format!(
                    "{}:{:02}",
                    elapsed_seconds / 60,
                    elapsed_seconds % 60
                )));
                self.timer.setHidden(false);
            }
            DictFeedback::Transcribing => {
                self.spinner.setHidden(false);
                unsafe { self.spinner.startAnimation(None) };
                self.title
                    .setStringValue(&NSString::from_str("Transcribing…"));
                self.layout_status_title(width_for(phase));
                self.title.setHidden(false);
            }
            DictFeedback::PostProcessing => {
                self.spinner.setHidden(false);
                unsafe { self.spinner.startAnimation(None) };
                self.title
                    .setStringValue(&NSString::from_str("Post-processing…"));
                self.layout_status_title(width_for(phase));
                self.title.setHidden(false);
            }
            DictFeedback::ShowingChanges => {
                self.show_status(phase, "✦", "Processed", &NSColor::systemPurpleColor())
            }
            DictFeedback::Inserted | DictFeedback::Fallback => {
                self.show_status(phase, "✓", "Inserted", &NSColor::systemGreenColor())
            }
            DictFeedback::Copied => {
                self.show_status(phase, "⌘", "Copied", &NSColor::systemOrangeColor())
            }
            DictFeedback::NoSpeech => {
                self.show_status(phase, "—", "Nothing heard", &NSColor::secondaryLabelColor())
            }
            DictFeedback::Error => {
                self.show_status(phase, "!", "Couldn’t finish", &NSColor::systemRedColor())
            }
            DictFeedback::Idle => (),
        }
        if phase != DictFeedback::Listening {
            self.bar_levels.set([0.0; 5]);
        }
    }

    fn layout_status_title(&self, minimum_width: f64) {
        // Measure using AppKit's real font metrics instead of assuming a string width. The previous fixed
        // widths clipped “Nothing heard” and were vulnerable to font/rendering differences between Macs.
        self.title.sizeToFit();
        let measured_width = self.title.frame().size.width;
        let width = fitted_status_width(minimum_width, measured_width);
        self.resize(width);
        self.title.setFrame(NSRect::new(
            NSPoint::new(STATUS_TEXT_X, STATUS_TEXT_Y),
            NSSize::new(
                width - STATUS_TEXT_X - STATUS_RIGHT_MARGIN,
                STATUS_TEXT_HEIGHT,
            ),
        ));
    }

    fn show_status(&self, phase: DictFeedback, icon: &str, title: &str, color: &NSColor) {
        self.icon.setStringValue(&NSString::from_str(icon));
        self.icon.setTextColor(Some(color));
        self.icon.setHidden(false);
        self.title.setStringValue(&NSString::from_str(title));
        self.layout_status_title(width_for(phase));
        self.title.setHidden(false);
    }

    fn update_change(&self, original: &str, replacement: &str, additional_changes: usize) {
        self.background.setBorderColor(&color(1.0, 1.0, 1.0, 0.14));
        self.background.setFillColor(&color(0.11, 0.11, 0.118, 1.0));
        self.reset_views();
        self.bar_levels.set([0.0; 5]);

        let original = compact_change_fragment(original);
        let replacement = compact_change_fragment(replacement);
        if original.is_empty() && replacement.is_empty() {
            self.resize(width_for(DictFeedback::ShowingChanges));
            self.show_status(
                DictFeedback::ShowingChanges,
                "✦",
                "Processed",
                &NSColor::systemPurpleColor(),
            );
            return;
        }

        let addition_only = original.is_empty();
        let removal_only = replacement.is_empty();
        let from_width = if addition_only {
            0.0
        } else {
            fit_preview_label(&self.change_from, &original, CHANGE_TEXT_MAX_WIDTH)
        };
        let replacement_label = if removal_only {
            "removed"
        } else {
            &replacement
        };
        let to_width = fit_preview_label(&self.change_to, replacement_label, CHANGE_TEXT_MAX_WIDTH);
        self.change_arrow
            .setStringValue(&NSString::from_str(if addition_only { "+" } else { "→" }));

        let more_width = if additional_changes > 0 {
            let text = format!("+{additional_changes}");
            fit_preview_label(&self.change_more, &text, 34.0)
        } else {
            0.0
        };
        let mut group_width = CHANGE_ARROW_WIDTH + CHANGE_GAP + to_width;
        if !addition_only {
            group_width += from_width + CHANGE_GAP;
        }
        if additional_changes > 0 {
            group_width += CHANGE_GAP + more_width;
        }
        let width = (group_width + CHANGE_MARGIN * 2.0).max(CHANGE_MIN_WIDTH);
        self.resize(width);
        let mut x = (width - group_width) / 2.0;

        if !addition_only {
            self.change_from.setFrame(NSRect::new(
                NSPoint::new(x, STATUS_TEXT_Y),
                NSSize::new(from_width, STATUS_TEXT_HEIGHT),
            ));
            self.change_strike.setFrame(NSRect::new(
                NSPoint::new(x, 15.5),
                NSSize::new(from_width, 1.0),
            ));
            self.change_from.setHidden(false);
            self.change_strike.setHidden(false);
            x += from_width + CHANGE_GAP;
        }

        self.change_arrow.setFrame(NSRect::new(
            NSPoint::new(x, STATUS_TEXT_Y),
            NSSize::new(CHANGE_ARROW_WIDTH, STATUS_TEXT_HEIGHT),
        ));
        let arrow_color = if addition_only {
            NSColor::controlAccentColor()
        } else {
            NSColor::secondaryLabelColor()
        };
        self.change_arrow.setTextColor(Some(&arrow_color));
        self.change_arrow.setHidden(false);
        x += CHANGE_ARROW_WIDTH + CHANGE_GAP;

        self.change_to.setFrame(NSRect::new(
            NSPoint::new(x, STATUS_TEXT_Y),
            NSSize::new(to_width, STATUS_TEXT_HEIGHT),
        ));
        let replacement_color = if removal_only {
            NSColor::secondaryLabelColor()
        } else {
            NSColor::controlAccentColor()
        };
        self.change_to.setTextColor(Some(&replacement_color));
        self.change_to.setHidden(false);
        x += to_width;

        if additional_changes > 0 {
            x += CHANGE_GAP;
            self.change_more.setFrame(NSRect::new(
                NSPoint::new(x, STATUS_TEXT_Y),
                NSSize::new(more_width, STATUS_TEXT_HEIGHT),
            ));
            self.change_more.setHidden(false);
        }
    }

    fn position(&self, mtm: MainThreadMarker, position: u8) {
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        // mainScreen follows the screen containing keyboard focus, which is the app the user is dictating
        // into. Mouse location is only a fallback; using it first could place a "centered" pill on the wrong
        // display while the pointer happened to be elsewhere.
        let screen = NSScreen::mainScreen(mtm).or_else(|| {
            screens.iter().find(|screen| {
                let frame = screen.frame();
                mouse.x >= frame.origin.x
                    && mouse.x <= frame.origin.x + frame.size.width
                    && mouse.y >= frame.origin.y
                    && mouse.y <= frame.origin.y + frame.size.height
            })
        });
        let Some(screen) = screen else {
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
            self.width.get(),
            self.height.get(),
        );
        self.panel.setFrameOrigin(NSPoint::new(x, y));
    }
}

fn voice_bar_levels(audio_level: f32, previous: [f32; 5]) -> [f32; 5] {
    // Expand the middle of the meter range: laptop-mic speech is usually well below digital full scale, but
    // should still make a confident visual gesture. Silence remains exactly zero after the capture gate.
    let audio_level = audio_level.clamp(0.0, 1.0).powf(0.55);
    std::array::from_fn(|index| {
        let target = audio_level * VOICE_PROFILE[index];
        let response = if target > previous[index] { 0.90 } else { 0.55 };
        (previous[index] + (target - previous[index]) * response).clamp(0.0, 1.0)
    })
}

fn fitted_status_width(minimum_width: f64, measured_text_width: f64) -> f64 {
    let measured_text_width = if measured_text_width.is_finite() {
        measured_text_width.max(0.0).ceil()
    } else {
        0.0
    };
    (STATUS_TEXT_X + measured_text_width + STATUS_RIGHT_MARGIN + STATUS_TEXT_SAFETY)
        .max(minimum_width)
}

fn compact_change_fragment(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = compact.chars().count();
    if count <= 96 {
        compact
    } else {
        compact
            .chars()
            .take(95)
            .chain(std::iter::once('…'))
            .collect()
    }
}

fn ellipsized(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    text.chars()
        .take(limit.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

fn fit_preview_label(label: &NSTextField, text: &str, maximum_width: f64) -> f64 {
    // Deliberately ellipsize a bounded preview rather than letting AppKit clip an unknown final glyph. The
    // exact unabridged before/after remains in Recent Dictations.
    let mut limit = text.chars().count().clamp(2, 48);
    loop {
        label.setStringValue(&NSString::from_str(&ellipsized(text, limit)));
        label.sizeToFit();
        let measured = label.frame().size.width.ceil() + 1.0;
        if measured <= maximum_width || limit <= 2 {
            return measured.min(maximum_width);
        }
        limit -= 1;
    }
}

fn width_for(phase: DictFeedback) -> f64 {
    match phase {
        DictFeedback::Listening | DictFeedback::Idle => LISTENING_WIDTH,
        DictFeedback::Transcribing => 132.0,
        DictFeedback::PostProcessing => 158.0,
        DictFeedback::ShowingChanges => 100.0,
        DictFeedback::Inserted | DictFeedback::Fallback => 94.0,
        DictFeedback::Copied => 90.0,
        DictFeedback::NoSpeech => 120.0,
        DictFeedback::Error => 126.0,
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

fn label(text: &str, frame: NSRect, size: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setUsesSingleLineMode(true);
    label
}

pub fn set(phase: DictFeedback, position: u8, elapsed_millis: u64, audio_level: f32) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("dictation: native pill update attempted off the AppKit main thread");
        return;
    };
    PILL.with_borrow_mut(|slot| {
        if phase == DictFeedback::Idle {
            if let Some(pill) = slot.as_ref() {
                pill.bar_levels.set([0.0; 5]);
                pill.panel.orderOut(None);
            }
            return;
        }
        let pill = slot.get_or_insert_with(|| Pill::new(mtm));
        pill.update(phase, elapsed_millis, audio_level);
        pill.position(mtm, position);
        // Unlike orderFront, this remains visible while Chrome, Notes, or any other app is active.
        pill.panel.orderFrontRegardless();
    });
}

pub fn set_change(position: u8, original: &str, replacement: &str, additional_changes: usize) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("dictation: native change preview attempted off the AppKit main thread");
        return;
    };
    PILL.with_borrow_mut(|slot| {
        let pill = slot.get_or_insert_with(|| Pill::new(mtm));
        pill.update_change(original, replacement, additional_changes);
        pill.position(mtm, position);
        pill.panel.orderFrontRegardless();
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
    fn recovered_pill_stays_compact_in_every_phase() {
        assert_eq!(width_for(DictFeedback::Listening), 103.0);
        assert_eq!(width_for(DictFeedback::Transcribing), 132.0);
        assert_eq!(width_for(DictFeedback::PostProcessing), 158.0);
        assert!(width_for(DictFeedback::Error) < 140.0);
        assert_eq!(HEIGHT, 32.0);
    }

    #[test]
    fn status_width_reserves_actual_text_metrics_instead_of_clipping() {
        let expanded = fitted_status_width(120.0, 80.4);
        assert_eq!(expanded, 132.0);
        assert!(expanded - STATUS_TEXT_X - STATUS_RIGHT_MARGIN >= 82.0);
        assert_eq!(fitted_status_width(132.0, 60.0), 132.0);
    }

    #[test]
    fn change_preview_is_single_line_and_explicitly_ellipsized() {
        assert_eq!(
            compact_change_fragment("  um\n twenty   five  "),
            "um twenty five"
        );
        assert_eq!(ellipsized("replacement", 7), "replac…");
        assert_eq!(ellipsized("short", 7), "short");
    }

    #[test]
    fn waveform_is_quiet_at_silence_and_grows_with_microphone_level() {
        let silent = voice_bar_levels(0.0, [0.0; 5]);
        let speaking = voice_bar_levels(0.7, silent);
        let louder = voice_bar_levels(1.0, speaking);
        assert_eq!(silent, [0.0; 5]);
        assert!(speaking[2] > speaking[0]);
        assert!(louder
            .iter()
            .zip(speaking)
            .all(|(next, prior)| *next > prior));
    }

    #[test]
    fn positions_cover_all_six_edges_in_global_screen_coordinates() {
        let screen = (-1920.0, 25.0, 1920.0, 1055.0);
        assert_eq!(origin(0, screen, LISTENING_WIDTH, HEIGHT), (-1011.5, 37.0));
        assert_eq!(origin(1, screen, LISTENING_WIDTH, HEIGHT), (-115.0, 37.0));
        assert_eq!(origin(2, screen, LISTENING_WIDTH, HEIGHT), (-115.0, 1036.0));
        assert_eq!(
            origin(3, screen, LISTENING_WIDTH, HEIGHT),
            (-1011.5, 1036.0)
        );
        assert_eq!(origin(4, screen, LISTENING_WIDTH, HEIGHT), (-1908.0, 536.5));
        assert_eq!(origin(5, screen, LISTENING_WIDTH, HEIGHT), (-115.0, 536.5));
    }

    #[test]
    fn tiny_screen_never_places_origin_before_its_visible_frame() {
        let (x, y) = origin(2, (100.0, 200.0, 100.0, 40.0), LISTENING_WIDTH, HEIGHT);
        assert!(x >= 112.0);
        assert!(y >= 212.0);
    }
}
