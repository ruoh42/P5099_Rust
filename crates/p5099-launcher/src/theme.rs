use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2, epaint::Shadow};

pub const BACKGROUND: Color32 = Color32::from_gray(0x10);
pub const HEADER: Color32 = Color32::from_gray(0x15);
pub const SURFACE: Color32 = Color32::from_gray(0x19);
pub const INPUT: Color32 = Color32::from_gray(0x0C);
pub const BORDER: Color32 = Color32::from_gray(0x2A);
pub const TEXT: Color32 = Color32::from_gray(0xE8);
pub const WEAK: Color32 = Color32::from_gray(0x8E);
pub const ACCENT: Color32 = Color32::from_gray(0xEC);
const ON_ACCENT: Color32 = Color32::from_gray(0x12);
pub const SUCCESS: Color32 = Color32::from_rgb(0x8C, 0xC8, 0xA4);
pub const WARNING: Color32 = Color32::from_rgb(0xD6, 0xB8, 0x7C);
pub const ERROR: Color32 = Color32::from_rgb(0xE0, 0x86, 0x86);

const RADIUS: u8 = 8;

pub fn apply(context: &egui::Context) {
    context.set_theme(egui::Theme::Dark);
    context.style_mut_of(egui::Theme::Dark, |style| {
        let spacing = &mut style.spacing;
        spacing.item_spacing = Vec2::new(8.0, 6.0);
        spacing.button_padding = Vec2::new(12.0, 6.0);
        spacing.interact_size.y = 30.0;
        spacing.combo_height = 300.0;

        let visuals = &mut style.visuals;
        visuals.panel_fill = BACKGROUND;
        visuals.window_fill = SURFACE;
        visuals.window_stroke = Stroke::new(1.0, BORDER);
        visuals.window_corner_radius = CornerRadius::same(RADIUS);
        visuals.menu_corner_radius = CornerRadius::same(RADIUS);
        visuals.popup_shadow = Shadow::NONE;
        visuals.extreme_bg_color = INPUT;
        visuals.text_edit_bg_color = Some(INPUT);
        visuals.faint_bg_color = SURFACE;
        visuals.override_text_color = Some(TEXT);
        visuals.weak_text_color = Some(WEAK);
        visuals.hyperlink_color = ACCENT;
        visuals.warn_fg_color = WARNING;
        visuals.error_fg_color = ERROR;
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, ACCENT);

        let widgets = &mut visuals.widgets;
        for (state, fill, stroke) in [
            (&mut widgets.noninteractive, SURFACE, BORDER),
            (
                &mut widgets.inactive,
                Color32::from_gray(0x23),
                Color32::from_gray(0x3A),
            ),
            (
                &mut widgets.hovered,
                Color32::from_gray(0x2C),
                Color32::from_gray(0x40),
            ),
            (
                &mut widgets.active,
                Color32::from_gray(0x33),
                Color32::from_gray(0x70),
            ),
            (&mut widgets.open, Color32::from_gray(0x23), BORDER),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, stroke);
            state.corner_radius = CornerRadius::same(6);
            state.expansion = 0.0;
        }
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    });
}

pub fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS + 2))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

pub fn banner<R>(
    ui: &mut egui::Ui,
    color: Color32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

pub fn message(ui: &mut egui::Ui, color: Color32, text: impl Into<String>) {
    banner(ui, color, |ui| {
        ui.label(RichText::new(text).color(color));
    });
}

pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let enabled = ui.is_enabled();
    let button = egui::Button::new(RichText::new(label).size(16.0).color(if enabled {
        ON_ACCENT
    } else {
        WEAK
    }))
    .fill(if enabled { ACCENT } else { BORDER })
    .stroke(Stroke::NONE)
    .corner_radius(CornerRadius::same(RADIUS))
    .min_size(Vec2::new(ui.available_width(), 40.0));
    ui.vertical_centered(|ui| ui.add(button))
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(6)),
    )
}

pub fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(INPUT)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.x = 4.0;
            #[allow(clippy::cast_precision_loss, reason = "a handful of tabs")]
            let width =
                (ui.available_width() - 4.0 * (labels.len() - 1) as f32) / labels.len() as f32;
            ui.horizontal(|ui| {
                for (index, label) in labels.iter().enumerate() {
                    let active = index == selected;
                    let text =
                        RichText::new(*label)
                            .size(14.0)
                            .color(if active { TEXT } else { WEAK });
                    let button = egui::Button::new(text)
                        .fill(if active {
                            SURFACE
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(if active {
                            Stroke::new(1.0, BORDER)
                        } else {
                            Stroke::NONE
                        })
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(width, 30.0));
                    if ui.add(button).clicked() && !active {
                        clicked = Some(index);
                    }
                }
            });
        });
    clicked
}

pub fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(WEAK));
}

pub fn text_input<'a>(value: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(value)
        .hint_text(RichText::new(hint).color(WEAK.gamma_multiply(0.7)))
        .margin(Margin::symmetric(10, 6))
        .desired_width(f32::INFINITY)
}

pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}
