use crate::config::ConfigStore;
use crate::effects::{evaluate_scene, resample_colors};
use crate::fan::resolve_sensor;
use crate::fan_service::FanService;
use crate::model::{
    now_secs, AppConfig, DeviceSettings, Direction, FanBackendPreference, FanCurve, LightingConfig,
    Page, Pattern, Profile, Rgb, ZoneUpdate,
};
use crate::presets::{placement_id, zone_from_template, COOLING_PRESETS, PALETTES, ZONE_TEMPLATES};
use crate::preview::{self, PreviewState};
use crate::rgb_service::RgbService;
use eframe::egui::{
    self, Align, Align2, Color32, ComboBox, DragValue, FontId, Frame, Layout, RichText, ScrollArea,
    Sense, Slider, Stroke, TextEdit, Ui, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

const BG: Color32 = Color32::from_rgb(9, 12, 17);
const PANEL: Color32 = Color32::from_rgb(16, 21, 28);
const TEXT: Color32 = Color32::from_rgb(230, 236, 245);
const MUTED: Color32 = Color32::from_rgb(132, 143, 160);
const DIM: Color32 = Color32::from_rgb(90, 100, 116);
const ACCENT: Color32 = Color32::from_rgb(34, 211, 238);
const VIOLET: Color32 = Color32::from_rgb(129, 140, 248);
const GOOD: Color32 = Color32::from_rgb(74, 222, 128);
const WARN: Color32 = Color32::from_rgb(251, 191, 36);
const BAD: Color32 = Color32::from_rgb(251, 113, 133);

fn rgb_color(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

fn palette_color(palette: &[Rgb]) -> Color32 {
    palette
        .first()
        .map(|color| rgb_color(*color))
        .unwrap_or(DIM)
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(10.0).strong().color(DIM));
}

pub struct T1App {
    config: AppConfig,
    store: ConfigStore,
    rgb: RgbService,
    fans: FanService,
    frames: HashMap<String, Vec<Rgb>>,
    rgb_status: crate::model::RgbStatus,
    fan_status: crate::model::FansStatus,
    preview: PreviewState,
    selected_zone: Option<String>,
    show_add_zone: bool,
    started: Instant,
    last_frame: Instant,
    last_save: Instant,
    dirty: bool,
    toast: Option<(String, Instant)>,
    new_profile_name: String,
    export_path: String,
    import_path: String,
    screenshot_path: Option<std::path::PathBuf>,
    screenshot_requested: bool,
}

impl T1App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut config: AppConfig,
        store: ConfigStore,
        screenshot_path: Option<std::path::PathBuf>,
    ) -> Self {
        config.validate();
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = Color32::from_rgb(7, 9, 13);
        visuals.faint_bg_color = Color32::from_rgb(21, 27, 35);
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.5);
        visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.4);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(31, 39, 50);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(22, 28, 36);
        visuals.override_text_color = Some(TEXT);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::new(9.0, 7.0);
        style.spacing.button_padding = Vec2::new(10.0, 6.0);
        cc.egui_ctx.set_style(style);

        let active = config.active_profile();
        let fans = FanService::new(config.devices.clone(), active.cooling.clone());
        let rgb = RgbService::new();
        rgb.set_demo(config.devices.demo_mode);
        fans.scan();
        if config.devices.openrgb.auto_connect && !config.devices.demo_mode {
            rgb.connect(&config.devices.openrgb.host, config.devices.openrgb.port);
        }
        if active.cooling.enabled {
            fans.apply();
        }
        let selected_zone = active.lighting.zones.first().map(|zone| zone.id.clone());
        Self {
            config,
            store,
            rgb,
            fans,
            frames: HashMap::new(),
            rgb_status: crate::model::RgbStatus::default(),
            fan_status: crate::model::FansStatus::default(),
            preview: PreviewState::default(),
            selected_zone,
            show_add_zone: false,
            started: Instant::now(),
            last_frame: Instant::now(),
            last_save: Instant::now(),
            dirty: false,
            toast: None,
            new_profile_name: String::new(),
            export_path: "t1-profile.json".into(),
            import_path: "t1-profile.json".into(),
            screenshot_path,
            screenshot_requested: false,
        }
    }

    fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
    }

    fn mark_dirty(&mut self) {
        self.config.touch();
        self.dirty = true;
    }

    fn save_now(&mut self) {
        match self.store.save(&self.config) {
            Ok(()) => {
                self.dirty = false;
                self.last_save = Instant::now();
            }
            Err(error) => self.toast(format!("Could not save config: {error}")),
        }
    }

    fn cpu_temperature(&self) -> Option<f32> {
        resolve_sensor("cpu", &self.fan_status.sensors).and_then(|sensor| sensor.value_c)
    }

    fn push_frames(&mut self) {
        if self.config.devices.demo_mode || !self.config.active_profile().lighting.enabled {
            return;
        }
        let lighting = &self.config.active_profile().lighting;
        let mut updates = Vec::new();
        for zone in &lighting.zones {
            if !zone.enabled {
                continue;
            }
            let Some(mapping) = &zone.mapping else {
                continue;
            };
            let Some(colors) = self.frames.get(&zone.id) else {
                continue;
            };
            updates.push(ZoneUpdate {
                device_index: mapping.device_index,
                zone_index: mapping.zone_index,
                colors: resample_colors(colors, mapping.led_count.max(1) as usize),
            });
        }
        self.rgb.push_frame(updates);
    }

    fn activate_profile(&mut self, id: &str) {
        if !self.config.profiles.iter().any(|profile| profile.id == id) {
            return;
        }
        self.config.active_profile_id = id.to_string();
        let profile = self.config.active_profile().clone();
        self.selected_zone = profile.lighting.zones.first().map(|zone| zone.id.clone());
        self.fans
            .configure(self.config.devices.clone(), profile.cooling);
        self.mark_dirty();
    }

    fn set_page(&mut self, page: Page) {
        self.config.ui.page = page;
        self.mark_dirty();
    }

    fn commit_profile(&mut self, profile: Profile) {
        let old_cooling = self.config.active_profile().cooling.clone();
        if let Some(slot) = self
            .config
            .profiles
            .iter_mut()
            .find(|item| item.id == profile.id)
        {
            *slot = profile;
        }
        let active = self.config.active_profile();
        if self
            .selected_zone
            .as_ref()
            .map(|id| !active.lighting.zones.iter().any(|zone| &zone.id == id))
            .unwrap_or(true)
        {
            self.selected_zone = active.lighting.zones.first().map(|zone| zone.id.clone());
        }
        if old_cooling != self.config.active_profile().cooling {
            self.fans.configure(
                self.config.devices.clone(),
                self.config.active_profile().cooling.clone(),
            );
        }
        self.config.validate();
        self.mark_dirty();
    }

    fn commit_devices(&mut self, devices: DeviceSettings) {
        let old = self.config.devices.clone();
        if old == devices {
            return;
        }
        self.config.devices = devices.clone();
        let backend_changed = old.demo_mode != devices.demo_mode
            || old.fan_backend != devices.fan_backend
            || old.lhm_dll_path != devices.lhm_dll_path;
        if old.demo_mode != devices.demo_mode {
            self.rgb.set_demo(devices.demo_mode);
        }
        if backend_changed {
            self.fans
                .configure(devices, self.config.active_profile().cooling.clone());
            self.fans.scan();
        }
        self.mark_dirty();
    }

    fn commit_config(&mut self, config: AppConfig) {
        let old = self.config.clone();
        self.config = config;
        self.config.validate();
        if old.active_profile().cooling != self.config.active_profile().cooling
            || old.devices != self.config.devices
        {
            self.fans.configure(
                self.config.devices.clone(),
                self.config.active_profile().cooling.clone(),
            );
        }
        if old.devices.demo_mode != self.config.devices.demo_mode
            || old.devices.fan_backend != self.config.devices.fan_backend
            || old.devices.lhm_dll_path != self.config.devices.lhm_dll_path
        {
            self.rgb.set_demo(self.config.devices.demo_mode);
            self.fans.scan();
        }
        self.mark_dirty();
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("T1 CONTROL").size(16.0).strong().color(TEXT));
            ui.label(
                RichText::new("FORM D T1 V2.5 · LUMEN S24")
                    .size(9.0)
                    .color(DIM),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let current = self.config.active_profile().name.clone();
                ComboBox::from_id_salt("profile-top")
                    .selected_text(current)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        let profiles: Vec<(String, String)> = self
                            .config
                            .profiles
                            .iter()
                            .map(|profile| (profile.id.clone(), profile.name.clone()))
                            .collect();
                        for (id, name) in profiles {
                            let active = id == self.config.active_profile_id;
                            if ui.selectable_label(active, name).clicked() && !active {
                                self.activate_profile(&id);
                            }
                        }
                    });
                let (fan_label, fan_color) = if self.fan_status.applied {
                    ("FANS LIVE", GOOD)
                } else if self.fan_status.demo {
                    ("FAN DEMO", VIOLET)
                } else if self.fan_status.controllable {
                    ("FANS READY", WARN)
                } else {
                    ("FANS OFF", DIM)
                };
                ui.label(
                    RichText::new(fan_label)
                        .size(10.0)
                        .strong()
                        .color(fan_color),
                );
                let (rgb_label, rgb_color) =
                    if self.rgb_status.mode == crate::model::RgbMode::Simulated {
                        ("RGB SIM", VIOLET)
                    } else if self.rgb_status.connected {
                        ("OPENRGB", GOOD)
                    } else if self.rgb_status.connecting {
                        ("CONNECTING", WARN)
                    } else {
                        ("RGB OFFLINE", DIM)
                    };
                ui.label(
                    RichText::new(rgb_label)
                        .size(10.0)
                        .strong()
                        .color(rgb_color),
                );
            });
        });
    }

    fn nav(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        for (page, label) in [
            (Page::Build, "Build"),
            (Page::Lighting, "Lighting"),
            (Page::Cooling, "Cooling"),
            (Page::Devices, "Devices"),
            (Page::Profiles, "Profiles"),
        ] {
            if ui
                .selectable_label(self.config.ui.page == page, RichText::new(label).size(14.0))
                .clicked()
            {
                self.set_page(page);
            }
        }
        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.label(RichText::new("9.95 L").size(10.0).color(MUTED));
            ui.label(RichText::new("135 · 220 · 335 mm").size(9.0).color(DIM));
            ui.separator();
        });
    }

    fn build_page(&mut self, ui: &mut Ui) {
        let mut profile = self.config.active_profile().clone();
        let frames = self.frames.clone();
        let mut selected = self.selected_zone.clone();
        let mut preview_state = self.preview.clone();
        let mut show_panels = self.config.ui.show_panels;
        let mut show_add = self.show_add_zone;
        let mut changed = false;
        let mut go_devices = false;

        let full_height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(205.0, full_height),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.label(RichText::new("RGB ZONES").size(10.0).strong().color(DIM));
                    ui.label(format!("{} zones", profile.lighting.zones.len()));
                    ScrollArea::vertical()
                        .max_height(full_height - 86.0)
                        .show(ui, |ui| {
                            let mut remove = None;
                            for zone in &mut profile.lighting.zones {
                                let is_selected = selected.as_deref() == Some(zone.id.as_str());
                                ui.horizontal(|ui| {
                                    let (rect, _) = ui
                                        .allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
                                    let palette = zone
                                        .effect
                                        .palette
                                        .as_deref()
                                        .unwrap_or(&profile.lighting.global.palette);
                                    ui.painter().rect_filled(rect, 3.0, palette_color(palette));
                                    if ui.selectable_label(is_selected, &zone.name).clicked() {
                                        selected = Some(zone.id.clone());
                                    }
                                    if ui.checkbox(&mut zone.enabled, "").changed() {
                                        changed = true;
                                    }
                                    if !zone.builtin && ui.small_button("x").clicked() {
                                        remove = Some(zone.id.clone());
                                    }
                                });
                            }
                            if let Some(id) = remove {
                                profile.lighting.zones.retain(|zone| zone.id != id);
                                if selected.as_deref() == Some(id.as_str()) {
                                    selected =
                                        profile.lighting.zones.first().map(|zone| zone.id.clone());
                                }
                                changed = true;
                            }
                        });
                    if ui.button("+ Add zone").clicked() {
                        show_add = !show_add;
                    }
                    if show_add {
                        ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            for template in ZONE_TEMPLATES {
                                if template.builtin
                                    && profile
                                        .lighting
                                        .zones
                                        .iter()
                                        .any(|zone| zone.placement == template.placement)
                                {
                                    continue;
                                }
                                if ui.button(template.name).clicked() {
                                    let id = format!(
                                        "{}-{}-{}",
                                        placement_id(template.placement),
                                        now_secs(),
                                        profile.lighting.zones.len()
                                    );
                                    let zone = zone_from_template(template, id.clone());
                                    profile.lighting.zones.push(zone);
                                    selected = Some(id);
                                    changed = true;
                                    show_add = false;
                                }
                            }
                        });
                    }
                },
            );
            ui.separator();
            let center_width = (ui.available_width() - 285.0).max(220.0);
            ui.allocate_ui_with_layout(
                Vec2::new(center_width, full_height),
                Layout::top_down(Align::Min),
                |ui| {
                    let preview_height = (ui.available_height() - 32.0).max(160.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), preview_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            preview::show(
                                ui,
                                &mut preview_state,
                                &profile.lighting,
                                &frames,
                                selected.as_deref(),
                                show_panels,
                            );
                        },
                    );
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut show_panels, "Outer panels").changed() {
                            // Persisted below.
                        }
                        ui.label(
                            RichText::new("Drag to orbit · scroll to zoom")
                                .size(9.0)
                                .color(DIM),
                        );
                    });
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(270.0, full_height),
                Layout::top_down(Align::Min),
                |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        inspector(
                            ui,
                            &mut profile,
                            selected.as_deref(),
                            &mut changed,
                            &mut go_devices,
                        );
                    });
                },
            );
        });

        self.preview = preview_state;
        self.show_add_zone = show_add;
        self.selected_zone = selected;
        if show_panels != self.config.ui.show_panels {
            self.config.ui.show_panels = show_panels;
            self.mark_dirty();
        }
        if changed {
            self.commit_profile(profile);
        }
        if go_devices {
            self.set_page(Page::Devices);
        }
    }

    fn lighting_page(&mut self, ui: &mut Ui) {
        let mut profile = self.config.active_profile().clone();
        let frames = self.frames.clone();
        let mut changed = false;
        ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(460.0, ui.available_height()),
                    Layout::top_down(Align::Min),
                    |ui| {
                        let global = &mut profile.lighting.global;
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut profile.lighting.enabled, "Lighting output")
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label(RichText::new(global.pattern.hint()).size(10.0).color(DIM));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Pattern");
                            if pattern_combo(ui, "global-pattern", &mut global.pattern) {
                                changed = true;
                            }
                        });
                        section(ui, "PALETTE");
                        if palette_presets(ui, &mut global.palette) {
                            changed = true;
                        }
                        if palette_editor(ui, &mut global.palette) {
                            changed = true;
                        }
                        if ui
                            .add(Slider::new(&mut global.brightness, 0..=100).text("Brightness"))
                            .changed()
                        {
                            changed = true;
                        }
                        if ui
                            .add(Slider::new(&mut global.speed, 0..=100).text("Speed"))
                            .changed()
                        {
                            changed = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label("Direction");
                            ComboBox::from_id_salt("direction")
                                .selected_text(match global.direction {
                                    Direction::Forward => "Forward",
                                    Direction::Reverse => "Reverse",
                                })
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_value(
                                            &mut global.direction,
                                            Direction::Forward,
                                            "Forward",
                                        )
                                        .clicked()
                                    {
                                        changed = true;
                                    }
                                    if ui
                                        .selectable_value(
                                            &mut global.direction,
                                            Direction::Reverse,
                                            "Reverse",
                                        )
                                        .clicked()
                                    {
                                        changed = true;
                                    }
                                });
                        });

                        section(ui, "PER-ZONE OVERRIDES");
                        for zone in &mut profile.lighting.zones {
                            ui.push_id(&zone.id, |ui| {
                                let header = format!("{} · {} LEDs", zone.name, zone.led_count);
                                egui::CollapsingHeader::new(header)
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        if ui.checkbox(&mut zone.enabled, "Enabled").changed() {
                                            changed = true;
                                        }
                                        ui.horizontal(|ui| {
                                            ui.label("Pattern");
                                            let global_pattern = profile.lighting.global.pattern;
                                            if pattern_override(
                                                ui,
                                                &zone.id,
                                                &mut zone.effect.pattern,
                                                global_pattern,
                                            ) {
                                                changed = true;
                                            }
                                        });
                                        let mut custom_palette = zone.effect.palette.is_some();
                                        if ui
                                            .checkbox(&mut custom_palette, "Custom palette")
                                            .changed()
                                        {
                                            zone.effect.palette = if custom_palette {
                                                Some(profile.lighting.global.palette.clone())
                                            } else {
                                                None
                                            };
                                            changed = true;
                                        }
                                        if let Some(palette) = &mut zone.effect.palette {
                                            if palette_editor(ui, palette) {
                                                changed = true;
                                            }
                                        }
                                        let mut override_brightness =
                                            zone.effect.brightness.is_some();
                                        if ui
                                            .checkbox(
                                                &mut override_brightness,
                                                "Override brightness",
                                            )
                                            .changed()
                                        {
                                            zone.effect.brightness =
                                                override_brightness.then_some(100);
                                            changed = true;
                                        }
                                        if let Some(brightness) = &mut zone.effect.brightness {
                                            if ui
                                                .add(
                                                    Slider::new(brightness, 0..=100)
                                                        .text("Zone brightness"),
                                                )
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                            });
                        }
                    },
                );
                ui.separator();
                ui.vertical(|ui| {
                    ui.heading("Zone output");
                    ui.label(
                        RichText::new(
                            "Same evaluator as the OpenRGB stream. Colours below are a direct readout.",
                        )
                        .size(10.0)
                        .color(DIM),
                    );
                    led_preview(ui, &profile.lighting, &frames);
                });
            });
        });
        if changed {
            self.commit_profile(profile);
        }
    }

    fn cooling_page(&mut self, ui: &mut Ui) {
        let mut profile = self.config.active_profile().clone();
        let fan_status = self.fan_status.clone();
        let mut changed = false;
        let mut action = None;

        ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Fan curves");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Restore firmware").clicked() {
                        action = Some(CoolingAction::Disable);
                    }
                    if ui.button("Apply curves").clicked() {
                        action = Some(CoolingAction::Apply);
                    }
                    if ui.button("Rescan").clicked() {
                        action = Some(CoolingAction::Scan);
                    }
                    let (label, color) = if fan_status.applied {
                        ("LIVE", GOOD)
                    } else if fan_status.demo {
                        ("DEMO", VIOLET)
                    } else if fan_status.controllable {
                        ("READY", WARN)
                    } else {
                        ("UNAVAILABLE", DIM)
                    };
                    ui.label(RichText::new(label).strong().color(color));
                });
            });
            ui.label(
                RichText::new(
                    "Recommended starting curves for the Lumen S24 (Aspect 12, 500–2000 RPM) in the T1 v2.5. Mesh panels restrict airflow, so these ramp earlier than an open bench. Starting points, not measurements.",
                )
                .size(11.0)
                .color(MUTED),
            );

            section(ui, "RECOMMENDED");
            ui.horizontal_wrapped(|ui| {
                for preset in COOLING_PRESETS {
                    if ui.button(preset.name).clicked() {
                        for channel in &mut profile.cooling.channels {
                            channel.max_rpm = preset.max_rpm;
                            channel.curve = preset.curve();
                        }
                        changed = true;
                    }
                }
            });

            if let Some(reason) = &fan_status.reason {
                ui.label(RichText::new(reason).size(10.0).color(WARN));
            }

            for channel in &mut profile.cooling.channels {
                ui.push_id(&channel.id, |ui| {
                    Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&channel.name).strong());
                            ui.checkbox(&mut channel.enabled, "Enabled");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Output");
                            ComboBox::from_id_salt(("output", &channel.id))
                                .selected_text(
                                    channel
                                        .device_label
                                        .clone()
                                        .unwrap_or_else(|| "Not mapped".into()),
                                )
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(channel.device_id.is_none(), "Not mapped")
                                        .clicked()
                                    {
                                        channel.device_id = None;
                                        channel.device_label = None;
                                    }
                                    for device in &fan_status.devices {
                                        let selected = channel.device_id.as_deref()
                                            == Some(device.id.as_str());
                                        if ui.selectable_label(selected, &device.label).clicked() {
                                            channel.device_id = Some(device.id.clone());
                                            channel.device_label = Some(device.label.clone());
                                            channel.max_rpm = device.max_rpm;
                                            channel.curve.max_rpm = device.max_rpm;
                                            changed = true;
                                        }
                                    }
                                });
                            ui.label("Sensor");
                            let options = sensor_options(&channel.curve.sensor, &fan_status.sensors);
                            let selected_label = options
                                .iter()
                                .find(|(id, _)| id == &channel.curve.sensor)
                                .map(|(_, label)| label.clone())
                                .unwrap_or_else(|| channel.curve.sensor.clone());
                            ComboBox::from_id_salt(("sensor", &channel.id))
                                .selected_text(selected_label)
                                .width(210.0)
                                .show_ui(ui, |ui| {
                                    for (id, label) in &options {
                                        if ui
                                            .selectable_value(
                                                &mut channel.curve.sensor,
                                                id.clone(),
                                                label,
                                            )
                                            .clicked()
                                        {
                                            changed = true;
                                        }
                                    }
                                });
                        });

                        let status = fan_status
                            .channels
                            .iter()
                            .find(|status| status.id == channel.id);
                        let temp = resolve_sensor(&channel.curve.sensor, &fan_status.sensors)
                            .and_then(|sensor| sensor.value_c);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "Temp {}   Target {}   Actual {}   Duty {}",
                                    temp.map(|value| format!("{value:.1}°C"))
                                        .unwrap_or_else(|| "—".into()),
                                    status
                                        .and_then(|status| status.target_rpm)
                                        .map(|value| format!("{value} RPM"))
                                        .unwrap_or_else(|| "—".into()),
                                    status
                                        .and_then(|status| status.rpm)
                                        .map(|value| format!("{value} RPM"))
                                        .unwrap_or_else(|| "—".into()),
                                    status
                                        .and_then(|status| status.duty_percent)
                                        .map(|value| format!("{value}%"))
                                        .unwrap_or_else(|| "—".into()),
                                ))
                                .size(10.0)
                                .color(MUTED),
                            );
                        });
                        if let Some(error) = status.and_then(|status| status.error.as_ref()) {
                            ui.label(RichText::new(error).size(10.0).color(BAD));
                        }
                        if curve_editor(ui, &mut channel.curve) {
                            changed = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label("Max RPM");
                            if ui
                                .add(DragValue::new(&mut channel.max_rpm).range(100..=6000))
                                .changed()
                            {
                                channel.curve.max_rpm = channel.max_rpm;
                                changed = true;
                            }
                            ComboBox::from_id_salt(("preset", &channel.id))
                                .selected_text("Load recommended curve…")
                                .show_ui(ui, |ui| {
                                    for preset in COOLING_PRESETS {
                                        if ui.selectable_label(false, preset.name).clicked() {
                                            channel.max_rpm = preset.max_rpm;
                                            channel.curve = preset.curve();
                                            changed = true;
                                        }
                                    }
                                });
                        });
                        if ui
                            .add(
                                Slider::new(&mut channel.curve.hysteresis_c, 0.0..=10.0)
                                    .text("Hysteresis °C"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                });
            }

            section(ui, "SAFETY");
            ui.label(
                RichText::new(
                    "Explicit mapping only. Closing the app restores the firmware fan mode. If a sensor disappears after a successful read, the mapped fan runs at the fail-safe speed.",
                )
                .size(10.0)
                .color(DIM),
            );
        });

        if changed {
            self.commit_profile(profile);
        }
        match action {
            Some(CoolingAction::Apply) => {
                let mut profile = self.config.active_profile().clone();
                profile.cooling.enabled = true;
                self.commit_profile(profile);
                self.fans.apply();
            }
            Some(CoolingAction::Disable) => {
                let mut profile = self.config.active_profile().clone();
                profile.cooling.enabled = false;
                self.commit_profile(profile);
                self.fans.disable();
            }
            Some(CoolingAction::Scan) => self.fans.scan(),
            None => {}
        }
    }

    fn devices_page(&mut self, ui: &mut Ui) {
        let mut devices = self.config.devices.clone();
        let mut lhm_path = devices.lhm_dll_path.clone().unwrap_or_default();
        let mut profile = self.config.active_profile().clone();
        let rgb_status = self.rgb_status.clone();
        let fan_status = self.fan_status.clone();
        let mut changed = false;
        let mut action = None;

        ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Devices & bridges");
            ui.label(
                RichText::new(
                    "T1 Control renders the effect and sends per-zone frames through the OpenRGB SDK. OpenRGB carries the MSI Mystic Light protocol, so MSI Center is not required.",
                )
                .size(10.5)
                .color(MUTED),
            );

            section(ui, "RGB OUTPUT · OPENRGB");
            ui.horizontal(|ui| {
                ui.label("Host");
                ui.add(TextEdit::singleline(&mut devices.openrgb.host).desired_width(150.0));
                ui.label("Port");
                ui.add(DragValue::new(&mut devices.openrgb.port).range(1..=65535));
                ui.checkbox(&mut devices.openrgb.auto_connect, "Auto-connect");
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!devices.demo_mode, egui::Button::new("Connect"))
                    .clicked()
                {
                    action = Some(DeviceAction::Connect);
                }
                if ui.button("Disconnect").clicked() {
                    action = Some(DeviceAction::Disconnect);
                }
                if ui.button("Refresh devices").clicked() {
                    action = Some(DeviceAction::Refresh);
                }
                let (label, color) = if rgb_status.mode == crate::model::RgbMode::Simulated {
                    ("SIMULATED", VIOLET)
                } else if rgb_status.connected {
                    ("CONNECTED", GOOD)
                } else if rgb_status.connecting {
                    ("CONNECTING", WARN)
                } else {
                    ("OFFLINE", DIM)
                };
                ui.label(RichText::new(label).strong().color(color));
            });
            if let Some(error) = &rgb_status.error {
                ui.label(RichText::new(error).size(10.0).color(BAD));
            }
            for device in &rgb_status.devices {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&device.name).strong());
                    ui.label(
                        RichText::new(format!(
                            "{}{}",
                            device.location,
                            if device.location.is_empty() { "" } else { " · " }
                        ))
                        .size(9.0)
                        .color(DIM),
                    );
                });
                for zone in &device.zones {
                    ui.label(
                        RichText::new(format!("   {} · {} LEDs", zone.name, zone.led_count))
                            .size(9.0)
                            .color(DIM),
                    );
                }
            }

            section(ui, "ZONE MAPPING");
            ui.horizontal(|ui| {
                if ui.button("Auto-suggest mappings").clicked() {
                    auto_map(&mut profile, &rgb_status.devices);
                    changed = true;
                }
                ui.label(
                    RichText::new("One scene zone to one OpenRGB zone; LED counts are resampled.")
                        .size(9.0)
                        .color(DIM),
                );
            });
            for zone in &mut profile.lighting.zones {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&zone.name).size(10.0).color(TEXT));
                    let selected_device = zone
                        .mapping
                        .as_ref()
                        .map(|mapping| mapping.device_name.clone())
                        .unwrap_or_else(|| "Not mapped".into());
                    ComboBox::from_id_salt(("map-device", &zone.id))
                        .selected_text(selected_device)
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(zone.mapping.is_none(), "Not mapped").clicked() {
                                zone.mapping = None;
                                changed = true;
                            }
                            for device in &rgb_status.devices {
                                let selected = zone
                                    .mapping
                                    .as_ref()
                                    .map(|mapping| mapping.device_index == device.index)
                                    .unwrap_or(false);
                                if ui.selectable_label(selected, &device.name).clicked() {
                                    if let Some(target) = device.zones.first() {
                                        zone.mapping = Some(crate::model::Mapping {
                                            device_index: device.index,
                                            device_name: device.name.clone(),
                                            zone_index: target.index,
                                            zone_name: target.name.clone(),
                                            led_count: target.led_count,
                                        });
                                        changed = true;
                                    }
                                }
                            }
                        });
                    if let Some(mapping) = &mut zone.mapping {
                        let device = rgb_status
                            .devices
                            .iter()
                            .find(|device| device.index == mapping.device_index);
                        let zones = device
                            .map(|device| device.zones.clone())
                            .unwrap_or_default();
                        ComboBox::from_id_salt(("map-zone", &zone.id))
                            .selected_text(mapping.zone_name.clone())
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                for target in &zones {
                                    let selected = target.index == mapping.zone_index;
                                    if ui.selectable_label(selected, &target.name).clicked() {
                                        mapping.zone_index = target.index;
                                        mapping.zone_name = target.name.clone();
                                        mapping.led_count = target.led_count;
                                        changed = true;
                                    }
                                }
                            });
                        ui.label(
                            RichText::new(format!("{} LEDs", mapping.led_count))
                                .size(9.0)
                                .color(DIM),
                        );
                    }
                });
            }
            section(ui, "FAN OUTPUT");
            ui.horizontal(|ui| {
                ui.label("Backend");
                ComboBox::from_id_salt("fan-backend")
                    .selected_text(match devices.fan_backend {
                        FanBackendPreference::Auto => "Auto detect",
                        FanBackendPreference::Sysfs => "Linux hwmon",
                        FanBackendPreference::Lhm => "LibreHardwareMonitor",
                        FanBackendPreference::Smc => "macOS SMC",
                        FanBackendPreference::None => "Disabled",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut devices.fan_backend,
                            FanBackendPreference::Auto,
                            "Auto detect",
                        );
                        ui.selectable_value(
                            &mut devices.fan_backend,
                            FanBackendPreference::Sysfs,
                            "Linux hwmon",
                        );
                        ui.selectable_value(
                            &mut devices.fan_backend,
                            FanBackendPreference::Lhm,
                            "LibreHardwareMonitor",
                        );
                        ui.selectable_value(
                            &mut devices.fan_backend,
                            FanBackendPreference::Smc,
                            "macOS SMC",
                        );
                        ui.selectable_value(
                            &mut devices.fan_backend,
                            FanBackendPreference::None,
                            "Disabled",
                        );
                    });
                ui.checkbox(&mut devices.demo_mode, "Demo hardware");
                if ui.button("Rescan hardware").clicked() {
                    action = Some(DeviceAction::ScanFans);
                }
            });
            if cfg!(target_os = "windows") {
                ui.horizontal(|ui| {
                    ui.label("LHM DLL");
                    ui.add(
                        TextEdit::singleline(&mut lhm_path)
                            .hint_text("C:\\Tools\\LibreHardwareMonitorLib.dll")
                            .desired_width(440.0),
                    );
                });
            }
            let (fan_label, fan_color) = if fan_status.demo {
                ("DEMO", VIOLET)
            } else if fan_status.controllable {
                ("READY", GOOD)
            } else if fan_status.available {
                ("NO PWM", WARN)
            } else {
                ("UNAVAILABLE", DIM)
            };
            ui.label(RichText::new(fan_label).strong().color(fan_color));
            if let Some(reason) = &fan_status.reason {
                ui.label(RichText::new(reason).size(10.0).color(WARN));
            }
            if !fan_status.sensors.is_empty() {
                ui.label(RichText::new("TEMPERATURE SENSORS").size(9.0).strong().color(DIM));
                for sensor in &fan_status.sensors {
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  {}",
                            sensor.label,
                            sensor
                                .value_c
                                .map(|value| format!("{value:.1} °C"))
                                .unwrap_or_else(|| "—".into())
                        ))
                        .size(10.0)
                        .color(MUTED),
                    );
                }
            }
            if !fan_status.devices.is_empty() {
                ui.label(RichText::new("CONTROLLABLE HEADERS").size(9.0).strong().color(DIM));
                for device in &fan_status.devices {
                    ui.label(
                        RichText::new(format!("{}  ·  {} max RPM", device.label, device.max_rpm))
                            .size(10.0)
                            .color(MUTED),
                    );
                }
            }


        });

        devices.lhm_dll_path = (!lhm_path.trim().is_empty()).then_some(lhm_path);
        self.commit_devices(devices);
        match action {
            Some(DeviceAction::Connect) => self.rgb.connect(
                &self.config.devices.openrgb.host,
                self.config.devices.openrgb.port,
            ),
            Some(DeviceAction::Disconnect) => self.rgb.disconnect(),
            Some(DeviceAction::Refresh) => self.rgb.refresh(),
            Some(DeviceAction::ScanFans) => self.fans.scan(),
            None => {}
        }
        if changed {
            self.commit_profile(profile);
        }
    }

    fn profiles_page(&mut self, ui: &mut Ui) {
        let mut config = self.config.clone();
        let mut action = None;
        let mut active_name = config.active_profile().name.clone();

        ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Profiles");
            ui.label(
                RichText::new("Complete lighting + cooling snapshots. Files are plain JSON.")
                    .size(10.5)
                    .color(MUTED),
            );
            ui.horizontal(|ui| {
                ui.label("Active name");
                if ui
                    .add(TextEdit::singleline(&mut active_name).desired_width(240.0))
                    .changed()
                {
                    action = Some(ProfileAction::Rename(active_name.clone()));
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.new_profile_name)
                        .hint_text("New profile name")
                        .desired_width(240.0),
                );
                if ui.button("Save current as…").clicked() {
                    action = Some(ProfileAction::SaveAs(self.new_profile_name.clone()));
                }
            });

            for profile in &config.profiles {
                ui.push_id(&profile.id, |ui| {
                    Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let active = profile.id == config.active_profile_id;
                            ui.label(RichText::new(&profile.name).strong().color(if active {
                                ACCENT
                            } else {
                                TEXT
                            }));
                            if active {
                                ui.label(RichText::new("ACTIVE").size(9.0).color(ACCENT));
                            }
                            ui.label(
                                RichText::new(format!(
                                    "{} zones · {} colours · {} channels",
                                    profile.lighting.zones.len(),
                                    profile.lighting.global.palette.len(),
                                    profile.cooling.channels.len()
                                ))
                                .size(9.0)
                                .color(DIM),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("Delete").clicked() {
                                    action = Some(ProfileAction::Delete(profile.id.clone()));
                                }
                                if ui.button("Duplicate").clicked() {
                                    action = Some(ProfileAction::Duplicate(profile.id.clone()));
                                }
                                if !active && ui.button("Activate").clicked() {
                                    action = Some(ProfileAction::Activate(profile.id.clone()));
                                }
                            });
                        });
                    });
                });
            }

            section(ui, "IMPORT / EXPORT");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut self.export_path).desired_width(300.0));
                if ui.button("Export active").clicked() {
                    action = Some(ProfileAction::Export(self.export_path.clone()));
                }
            });
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut self.import_path).desired_width(300.0));
                if ui.button("Import profile").clicked() {
                    action = Some(ProfileAction::Import(self.import_path.clone()));
                }
            });
        });

        match action {
            Some(ProfileAction::Rename(name)) => {
                config.active_profile_mut().name = name;
                config.validate();
                self.commit_config(config);
            }
            Some(ProfileAction::Activate(id)) => self.activate_profile(&id),
            Some(ProfileAction::SaveAs(name)) => {
                let mut profile = config.active_profile().clone();
                profile.id = format!("profile-{}", now_secs());
                profile.name = if name.trim().is_empty() {
                    format!("{} copy", config.active_profile().name)
                } else {
                    name
                };
                profile.created_at = now_secs();
                profile.updated_at = now_secs();
                config.profiles.push(profile.clone());
                config.active_profile_id = profile.id.clone();
                self.new_profile_name.clear();
                self.commit_config(config);
                self.selected_zone = self
                    .config
                    .active_profile()
                    .lighting
                    .zones
                    .first()
                    .map(|zone| zone.id.clone());
            }
            Some(ProfileAction::Duplicate(id)) => {
                if let Some(base) = config
                    .profiles
                    .iter()
                    .find(|profile| profile.id == id)
                    .cloned()
                {
                    let mut profile = base.clone();
                    profile.id = format!("profile-{}", now_secs());
                    profile.name = format!("{} copy", base.name);
                    profile.created_at = now_secs();
                    profile.updated_at = now_secs();
                    config.profiles.push(profile);
                    self.commit_config(config);
                }
            }
            Some(ProfileAction::Delete(id)) => {
                if config.profiles.len() > 1 {
                    config.profiles.retain(|profile| profile.id != id);
                    config.validate();
                    self.commit_config(config);
                } else {
                    self.toast("Keep at least one profile");
                }
            }
            Some(ProfileAction::Export(path)) => {
                match self
                    .store
                    .export_profile(Path::new(&path), self.config.active_profile())
                {
                    Ok(()) => self.toast(format!("Exported to {path}")),
                    Err(error) => self.toast(format!("Export failed: {error}")),
                }
            }
            Some(ProfileAction::Import(path)) => {
                match self.store.import_profile(Path::new(&path), &self.config) {
                    Ok(profile) => {
                        self.config.profiles.push(profile);
                        self.mark_dirty();
                        self.toast("Profile imported");
                    }
                    Err(error) => self.toast(format!("Import failed: {error}")),
                }
            }
            None => {}
        }
    }
}

impl eframe::App for T1App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        self.rgb_status = self.rgb.status();
        self.fan_status = self.fans.status();

        if now.duration_since(self.last_frame) >= Duration::from_millis(50) {
            let temperature = self.cpu_temperature();
            let time = self.started.elapsed().as_secs_f32();
            self.frames = evaluate_scene(&self.config.active_profile().lighting, time, temperature);
            self.push_frames();
            self.last_frame = now;
        }
        if self.dirty && now.duration_since(self.last_save) >= Duration::from_millis(500) {
            self.save_now();
        }

        egui::TopBottomPanel::top("top")
            .exact_height(48.0)
            .show(ctx, |ui| self.top_bar(ui));
        egui::SidePanel::left("nav")
            .exact_width(150.0)
            .resizable(false)
            .show(ctx, |ui| self.nav(ui));
        egui::CentralPanel::default().show(ctx, |ui| match self.config.ui.page {
            Page::Build => self.build_page(ui),
            Page::Lighting => self.lighting_page(ui),
            Page::Cooling => self.cooling_page(ui),
            Page::Devices => self.devices_page(ui),
            Page::Profiles => self.profiles_page(ui),
        });

        if let Some(path) = self.screenshot_path.clone() {
            if !self.screenshot_requested && self.started.elapsed() >= Duration::from_millis(1200) {
                self.screenshot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let events = ctx.input(|input| input.events.clone());
            for event in events {
                if let egui::Event::Screenshot { image, .. } = event {
                    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                    for pixel in &image.pixels {
                        rgba.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), pixel.a()]);
                    }
                    match image::save_buffer(
                        &path,
                        &rgba,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => eprintln!("[t1-control] screenshot saved to {}", path.display()),
                        Err(error) => eprintln!("[t1-control] screenshot failed: {error}"),
                    }
                    self.screenshot_path = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    break;
                }
            }
        }

        if let Some((message, when)) = self.toast.clone() {
            if when.elapsed() < Duration::from_secs(6) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -16.0))
                    .show(ctx, |ui| {
                        Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(message);
                        });
                    });
            } else {
                self.toast = None;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}

impl Drop for T1App {
    fn drop(&mut self) {
        let _ = self.store.save(&self.config);
        self.fans.shutdown();
        self.rgb.shutdown();
    }
}

enum CoolingAction {
    Apply,
    Disable,
    Scan,
}

enum DeviceAction {
    Connect,
    Disconnect,
    Refresh,
    ScanFans,
}

enum ProfileAction {
    Rename(String),
    Activate(String),
    SaveAs(String),
    Duplicate(String),
    Delete(String),
    Export(String),
    Import(String),
}

fn pattern_combo(ui: &mut Ui, id: &str, value: &mut Pattern) -> bool {
    let mut changed = false;
    ComboBox::from_id_salt(id)
        .selected_text(value.label())
        .width(190.0)
        .show_ui(ui, |ui| {
            for pattern in Pattern::ALL {
                if ui
                    .selectable_value(value, pattern, pattern.label())
                    .clicked()
                {
                    changed = true;
                }
            }
        });
    changed
}

fn pattern_override(ui: &mut Ui, id: &str, value: &mut Option<Pattern>, global: Pattern) -> bool {
    let mut changed = false;
    ComboBox::from_id_salt(("pattern", id))
        .selected_text(
            value
                .map(|pattern| pattern.label().to_string())
                .unwrap_or_else(|| format!("Inherit · {}", global.label())),
        )
        .show_ui(ui, |ui| {
            if ui.selectable_value(value, None, "Inherit").clicked() {
                changed = true;
            }
            for pattern in Pattern::ALL {
                if ui
                    .selectable_value(value, Some(pattern), pattern.label())
                    .clicked()
                {
                    changed = true;
                }
            }
        });
    changed
}

fn palette_presets(ui: &mut Ui, palette: &mut Vec<Rgb>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for preset in PALETTES {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, 2.0, palette_color(preset.colors));
            if ui.button(preset.name).clicked() {
                *palette = preset.colors.to_vec();
                changed = true;
            }
        }
    });
    changed
}

fn palette_editor(ui: &mut Ui, palette: &mut Vec<Rgb>) -> bool {
    let mut changed = false;
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        let can_remove = palette.len() > 1;
        for (index, color) in palette.iter_mut().enumerate() {
            let mut value = color.to_array();
            if ui.color_edit_button_srgb(&mut value).changed() {
                *color = Rgb::from_array(value);
                changed = true;
            }
            if can_remove && ui.small_button("x").clicked() {
                remove = Some(index);
            }
        }
        if ui.small_button("+").clicked() {
            palette.push(Rgb::WHITE);
            changed = true;
        }
    });
    if let Some(index) = remove {
        palette.remove(index);
        changed = true;
    }
    changed
}

fn sensor_options(setting: &str, sensors: &[crate::model::SensorInfo]) -> Vec<(String, String)> {
    let mut options = vec![
        ("cpu".to_string(), "CPU package · auto".to_string()),
        ("gpu".to_string(), "GPU · auto".to_string()),
        ("coolant".to_string(), "AIO coolant · auto".to_string()),
        ("motherboard".to_string(), "Motherboard · auto".to_string()),
    ];
    for sensor in sensors {
        options.push((
            sensor.id.clone(),
            format!(
                "{} · {}",
                sensor.label,
                sensor
                    .value_c
                    .map(|value| format!("{value:.1} °C"))
                    .unwrap_or_else(|| "n/a".into())
            ),
        ));
    }
    if !options.iter().any(|(id, _)| id == setting) {
        options.push((setting.to_string(), setting.to_string()));
    }
    options
}

fn curve_editor(ui: &mut Ui, curve: &mut FanCurve) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 190.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, Color32::from_rgb(7, 9, 13));

    let padding = 28.0;
    let plot = egui::Rect::from_min_max(
        rect.min + Vec2::new(padding, 10.0),
        rect.max - Vec2::new(10.0, 26.0),
    );
    let max_rpm = curve.max_rpm.max(100) as f32;
    for index in 0..=4 {
        let fraction = index as f32 / 4.0;
        let y = plot.bottom() - fraction * plot.height();
        painter.line_segment(
            [
                egui::Pos2::new(plot.left(), y),
                egui::Pos2::new(plot.right(), y),
            ],
            Stroke::new(0.6, Color32::from_rgba_unmultiplied(120, 135, 155, 35)),
        );
        painter.text(
            egui::Pos2::new(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{}", (max_rpm * fraction).round() as u16),
            FontId::proportional(9.0),
            DIM,
        );
    }
    for temp in (20..=100).step_by(10) {
        let x = plot.left() + (temp as f32 - 20.0) / 80.0 * plot.width();
        painter.line_segment(
            [
                egui::Pos2::new(x, plot.top()),
                egui::Pos2::new(x, plot.bottom()),
            ],
            Stroke::new(0.6, Color32::from_rgba_unmultiplied(120, 135, 155, 28)),
        );
        painter.text(
            egui::Pos2::new(x, plot.bottom() + 12.0),
            Align2::CENTER_TOP,
            format!("{temp}"),
            FontId::proportional(9.0),
            DIM,
        );
    }

    let points = curve.points.clone();
    let to_screen = |point: &crate::model::FanCurvePoint| {
        egui::Pos2::new(
            plot.left() + (point.temp_c.clamp(20.0, 100.0) - 20.0) / 80.0 * plot.width(),
            plot.bottom() - (point.rpm as f32 / max_rpm).clamp(0.0, 1.0) * plot.height(),
        )
    };
    let screen_points: Vec<egui::Pos2> = points.iter().map(to_screen).collect();
    for pair in screen_points.windows(2) {
        painter.line_segment([pair[0], pair[1]], Stroke::new(2.2, ACCENT));
    }
    for screen in &screen_points {
        let hovered = response
            .hover_pos()
            .map(|pos| pos.distance(*screen) < 10.0)
            .unwrap_or(false);
        painter.circle_filled(
            *screen,
            if hovered { 5.5 } else { 4.0 },
            if hovered { Color32::WHITE } else { ACCENT },
        );
    }

    let mut changed = false;
    if response.double_clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            if let Some(index) = screen_points
                .iter()
                .position(|screen| screen.distance(pointer) < 12.0)
            {
                if points.len() > 2 {
                    curve.points.remove(index);
                    changed = true;
                }
            } else if plot.contains(pointer) {
                let temp = 20.0 + ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0) * 80.0;
                let temp = (temp.round() * 10.0).round() / 10.0;
                if !curve
                    .points
                    .iter()
                    .any(|point| (point.temp_c - temp).abs() < 1.0)
                {
                    let rpm =
                        ((plot.bottom() - pointer.y) / plot.height()).clamp(0.0, 1.0) * max_rpm;
                    curve.points.push(crate::model::FanCurvePoint {
                        temp_c: temp,
                        rpm: rpm.round() as u16,
                    });
                    changed = true;
                }
            }
        }
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(pointer) = response.interact_pointer_pos() {
            if let Some(index) = screen_points
                .iter()
                .position(|screen| screen.distance(pointer) < 14.0)
            {
                let low = if index == 0 {
                    20.0
                } else {
                    curve.points[index - 1].temp_c + 1.0
                };
                let high = if index + 1 >= curve.points.len() {
                    100.0
                } else {
                    curve.points[index + 1].temp_c - 1.0
                };
                let temp = 20.0 + ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0) * 80.0;
                let temp = if low <= high {
                    temp.clamp(low, high)
                } else {
                    curve.points[index].temp_c
                };
                let rpm = ((plot.bottom() - pointer.y) / plot.height()).clamp(0.0, 1.0) * max_rpm;
                curve.points[index].temp_c = temp;
                curve.points[index].rpm = rpm.round() as u16;
                changed = true;
            }
        }
    }
    if changed {
        curve.points.sort_by(|a, b| a.temp_c.total_cmp(&b.temp_c));
        curve.normalize();
    }
    painter.text(
        egui::Pos2::new(plot.left(), plot.top() - 2.0),
        Align2::LEFT_BOTTOM,
        "Drag points · double-click chart to add or remove",
        FontId::proportional(8.5),
        DIM,
    );
    changed
}

fn led_preview(ui: &mut Ui, lighting: &LightingConfig, frames: &HashMap<String, Vec<Rgb>>) {
    ScrollArea::vertical().show(ui, |ui| {
        for zone in &lighting.zones {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&zone.name).size(10.0).strong());
                ui.label(
                    RichText::new(format!("{} LEDs", zone.led_count))
                        .size(9.0)
                        .color(DIM),
                );
            });
            let count = zone.led_count.min(64) as usize;
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width().max(120.0), 22.0),
                Sense::hover(),
            );
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 5.0, Color32::BLACK);
            let colors = frames.get(&zone.id);
            let gap = 2.0;
            let cell = ((rect.width() - gap * (count as f32 - 1.0)) / count as f32).max(1.0);
            for index in 0..count {
                let color = colors
                    .and_then(|colors| colors.get(index))
                    .map(|color| rgb_color(*color))
                    .unwrap_or(Color32::BLACK);
                let x = rect.left() + index as f32 * (cell + gap);
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::Pos2::new(x, rect.top() + 3.0),
                        Vec2::new(cell, rect.height() - 6.0),
                    ),
                    2.0,
                    color,
                );
            }
            ui.add_space(5.0);
        }
    });
}

fn inspector(
    ui: &mut Ui,
    profile: &mut Profile,
    selected: Option<&str>,
    changed: &mut bool,
    go_devices: &mut bool,
) {
    let Some(index) =
        selected.and_then(|id| profile.lighting.zones.iter().position(|zone| zone.id == id))
    else {
        ui.label(RichText::new("Select a zone").color(DIM));
        return;
    };
    let global = profile.lighting.global.clone();
    let zone = &mut profile.lighting.zones[index];

    ui.label(RichText::new("INSPECTOR").size(10.0).strong().color(DIM));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if zone.builtin {
                "S24"
            } else {
                zone.kind.label()
            })
            .size(9.0)
            .color(ACCENT),
        );
        ui.label(RichText::new(zone.placement.label()).size(9.0).color(DIM));
    });
    if ui
        .add(TextEdit::singleline(&mut zone.name).desired_width(f32::INFINITY))
        .changed()
    {
        *changed = true;
    }
    ui.horizontal(|ui| {
        ui.label("LEDs");
        if ui
            .add(DragValue::new(&mut zone.led_count).range(1..=512))
            .changed()
        {
            *changed = true;
        }
        if ui.checkbox(&mut zone.enabled, "Enabled").changed() {
            *changed = true;
        }
    });

    section(ui, "EFFECT OVERRIDE");
    ui.horizontal(|ui| {
        ui.label("Pattern");
        if pattern_override(ui, &zone.id, &mut zone.effect.pattern, global.pattern) {
            *changed = true;
        }
    });
    let mut custom_palette = zone.effect.palette.is_some();
    if ui.checkbox(&mut custom_palette, "Custom palette").changed() {
        zone.effect.palette = custom_palette.then(|| global.palette.clone());
        *changed = true;
    }
    if let Some(palette) = &mut zone.effect.palette {
        if palette_editor(ui, palette) {
            *changed = true;
        }
    }
    let mut override_brightness = zone.effect.brightness.is_some();
    if ui
        .checkbox(&mut override_brightness, "Brightness offset")
        .changed()
    {
        zone.effect.brightness = override_brightness.then_some(100);
        *changed = true;
    }
    if let Some(brightness) = &mut zone.effect.brightness {
        if ui
            .add(Slider::new(brightness, 0..=100).text("Zone brightness"))
            .changed()
        {
            *changed = true;
        }
    }
    let mut override_speed = zone.effect.speed.is_some();
    if ui.checkbox(&mut override_speed, "Speed override").changed() {
        zone.effect.speed = override_speed.then_some(global.speed);
        *changed = true;
    }
    if let Some(speed) = &mut zone.effect.speed {
        if ui
            .add(Slider::new(speed, 0..=100).text("Zone speed"))
            .changed()
        {
            *changed = true;
        }
    }

    section(ui, "OPENRGB OUTPUT");
    match &zone.mapping {
        Some(mapping) => {
            ui.label(RichText::new(&mapping.device_name).strong());
            ui.label(
                RichText::new(format!(
                    "{} · {} LEDs",
                    mapping.zone_name, mapping.led_count
                ))
                .size(9.0)
                .color(DIM),
            );
        }
        None => {
            ui.label(RichText::new("Not mapped").size(10.0).color(WARN));
        }
    }
    if ui.button("Map in Devices").clicked() {
        *go_devices = true;
    }
    if !zone.builtin && ui.button("Remove zone").clicked() {
        profile.lighting.zones.remove(index);
        *changed = true;
    }
}

fn auto_map(profile: &mut Profile, devices: &[crate::model::RgbDevice]) {
    let mut used = HashSet::new();
    for zone in &mut profile.lighting.zones {
        let terms: &[&str] = match zone.kind {
            crate::model::ZoneKind::Pump => &["pump", "block", "cpu", "cooler"],
            crate::model::ZoneKind::Fan => &["fan", "ring", "aspect", "radiator"],
            crate::model::ZoneKind::Strip => &["strip", "led", "argb", "diffus", "light"],
            crate::model::ZoneKind::Block => &["gpu", "block", "water", "accent"],
        };
        let candidate = devices
            .iter()
            .flat_map(|device| device.zones.iter().map(move |zone| (device, zone)))
            .find(|(device, target)| {
                !used.contains(&(device.index, target.index))
                    && terms
                        .iter()
                        .any(|term| target.name.to_lowercase().contains(term))
            })
            .or_else(|| {
                devices
                    .iter()
                    .flat_map(|device| device.zones.iter().map(move |zone| (device, zone)))
                    .find(|(device, target)| !used.contains(&(device.index, target.index)))
            });
        if let Some((device, target)) = candidate {
            used.insert((device.index, target.index));
            zone.mapping = Some(crate::model::Mapping {
                device_index: device.index,
                device_name: device.name.clone(),
                zone_index: target.index,
                zone_name: target.name.clone(),
                led_count: target.led_count,
            });
        }
    }
}

pub fn demo_apply(config: &mut AppConfig) {
    config.devices.demo_mode = true;
    config.ui.show_panels = false;
    let profile = config.active_profile_mut();
    profile.cooling.enabled = true;
    if let Some(channel) = profile.cooling.channels.get_mut(0) {
        channel.device_id = Some("demo-aio-1".into());
        channel.device_label = Some("AIO fan 1 · Aspect 12".into());
    }
    if let Some(channel) = profile.cooling.channels.get_mut(1) {
        channel.device_id = Some("demo-aio-2".into());
        channel.device_label = Some("AIO fan 2 · Aspect 12".into());
    }
}
