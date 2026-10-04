//! The few styles libadwaita does not already have: the tinted chips the
//! sidebars and inspector use, and the strip's rounded frame. Colours are
//! libadwaita's named palette, so they follow light and dark.

const CSS: &str = "
.chip {
    border-radius: 99px;
    padding: 0 7px;
    font-size: 0.85em;
    background: alpha(currentColor, 0.12);
}
.chip-accent { background: alpha(@accent_bg_color, 0.25); }
.chip-blue   { background: alpha(@blue_3, 0.25); }
.chip-orange { background: alpha(@orange_3, 0.25); }
.chip-gray   { background: alpha(@light_5, 0.25); }
.chip-red    { background: alpha(@red_3, 0.25); }
.chip-green  { background: alpha(@green_4, 0.25); }
.chip-purple { background: alpha(@purple_3, 0.25); }
.chip-indigo { background: alpha(@purple_2, 0.2); }
.romlens-strip-frame { border-radius: 4px; }
.romlens-idiom {
    border-radius: 8px;
    padding: 10px;
    background: alpha(@purple_2, 0.1);
}
.diff-changed { background: alpha(@orange_3, 0.2); }
.diff-added   { background: alpha(@green_4, 0.2); }
.diff-removed { background: alpha(@red_3, 0.2); }
.voice-strip {
    border-radius: 8px;
    padding: 10px;
    background: alpha(currentColor, 0.05);
    border: 1px solid alpha(currentColor, 0.2);
}
.voice-selected { border: 2px solid @accent_bg_color; }
.block-selected { background: alpha(@accent_bg_color, 0.3); }
.block-loop { background: alpha(@green_4, 0.2); }
.flag-on { background: alpha(@accent_bg_color, 0.25); }
.flag-off { opacity: 0.45; }
.romlens-flags-title { font-size: 0.8em; opacity: 0.6; }
.question-bubble {
    border-radius: 12px;
    background: alpha(@accent_bg_color, 0.2);
}
.lesson-card { border: 1px solid alpha(@accent_bg_color, 0.4); }
.concept {
    border-radius: 6px;
    padding: 5px 8px;
    background: alpha(currentColor, 0.08);
}
.concept-1 { background: alpha(@accent_bg_color, 0.29); }
.concept-2 { background: alpha(@accent_bg_color, 0.46); }
.concept-3 { background: alpha(@accent_bg_color, 0.63); }
.concept-4 { background: alpha(@accent_bg_color, 0.8); color: white; }
.concept-5 { background: alpha(@accent_bg_color, 0.97); color: white; }
.proven-1 { border: 1.5px solid @accent_bg_color; }
.proven-2 { border: 2px solid @accent_bg_color; }
.proven-3 { border: 2.5px solid @accent_bg_color; }
.proven-4 { border: 3px solid @accent_bg_color; }
.proven-5 { border: 3.5px solid @accent_bg_color; }
.quiz-bit { min-width: 24px; min-height: 24px; padding: 0; }
.quiz-result {
    border-radius: 8px;
    padding: 10px;
    background: alpha(currentColor, 0.08);
}
";

pub fn install() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// A small tinted pill. `class` is one of the `chip-*` names, or empty.
pub fn chip(text: &str, class: &str) -> gtk::Label {
    use gtk::prelude::*;
    let l = gtk::Label::new(Some(text));
    l.add_css_class("chip");
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_valign(gtk::Align::Center);
    l
}
