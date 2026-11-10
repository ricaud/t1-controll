use crate::effects::resample_colors;
use crate::model::{LightingConfig, Placement, Rgb};
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct PreviewState {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            yaw: 0.62,
            pitch: 0.36,
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

struct Projection {
    yaw: f32,
    pitch: f32,
    scale: f32,
    center: Pos2,
}

impl Projection {
    fn project(&self, point: Vec3) -> (Pos2, f32) {
        let (sin_y, cos_y) = self.yaw.sin_cos();
        let x = point.x * cos_y + point.z * sin_y;
        let z1 = -point.x * sin_y + point.z * cos_y;
        let (sin_p, cos_p) = self.pitch.sin_cos();
        let y = (point.y - 167.5) * cos_p - z1 * sin_p;
        let z = (point.y - 167.5) * sin_p + z1 * cos_p;
        (self.center + Vec2::new(x * self.scale, -y * self.scale), z)
    }
}

struct Face {
    points: Vec<Pos2>,
    depth: f32,
    fill: Color32,
    stroke: Stroke,
}

fn shade(color: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (color.r() as f32 * factor).round().clamp(0.0, 255.0) as u8,
        (color.g() as f32 * factor).round().clamp(0.0, 255.0) as u8,
        (color.b() as f32 * factor).round().clamp(0.0, 255.0) as u8,
    )
}

fn rgba(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[allow(clippy::too_many_arguments)]
fn add_box(
    faces: &mut Vec<Face>,
    projection: &Projection,
    center: Vec3,
    size: Vec3,
    color: Color32,
    alpha: u8,
) {
    let hx = size.x / 2.0;
    let hy = size.y / 2.0;
    let hz = size.z / 2.0;
    let corners = [
        Vec3::new(center.x - hx, center.y - hy, center.z - hz),
        Vec3::new(center.x + hx, center.y - hy, center.z - hz),
        Vec3::new(center.x + hx, center.y + hy, center.z - hz),
        Vec3::new(center.x - hx, center.y + hy, center.z - hz),
        Vec3::new(center.x - hx, center.y - hy, center.z + hz),
        Vec3::new(center.x + hx, center.y - hy, center.z + hz),
        Vec3::new(center.x + hx, center.y + hy, center.z + hz),
        Vec3::new(center.x - hx, center.y + hy, center.z + hz),
    ];
    let indices = [
        ([0usize, 1, 2, 3], 0.72f32),
        ([5, 4, 7, 6], 0.62),
        ([4, 0, 3, 7], 0.55),
        ([1, 5, 6, 2], 0.55),
        ([3, 2, 6, 7], 1.0),
        ([4, 5, 1, 0], 0.45),
    ];
    for (face, factor) in indices {
        let mut points = Vec::with_capacity(4);
        let mut depth = 0.0;
        for index in face {
            let (point, z) = projection.project(corners[index]);
            points.push(point);
            depth += z;
        }
        faces.push(Face {
            points,
            depth: depth / 4.0,
            fill: rgba(shade(color, factor), alpha),
            stroke: Stroke::new(0.7, Color32::from_rgba_unmultiplied(210, 220, 235, 18)),
        });
    }
}

fn frame_line(
    painter: &Painter,
    projection: &Projection,
    a: Vec3,
    b: Vec3,
    width: f32,
    color: Color32,
) {
    let (pa, _) = projection.project(a);
    let (pb, _) = projection.project(b);
    painter.line_segment([pa, pb], Stroke::new(width, color));
}

fn circle_points(center: Vec3, radius: f32, axis: u8, count: usize) -> Vec<Vec3> {
    (0..count)
        .map(|index| {
            let angle = index as f32 / count as f32 * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            match axis {
                0 => Vec3::new(center.x, center.y + radius * cos, center.z + radius * sin),
                1 => Vec3::new(center.x + radius * cos, center.y, center.z + radius * sin),
                _ => Vec3::new(center.x + radius * cos, center.y + radius * sin, center.z),
            }
        })
        .collect()
}

fn draw_rgb_ring(
    painter: &Painter,
    projection: &Projection,
    center: Vec3,
    radius: f32,
    axis: u8,
    colors: &[Rgb],
    selected: bool,
) {
    let points_3d = circle_points(center, radius, axis, 40);
    let points: Vec<Pos2> = points_3d
        .iter()
        .map(|point| projection.project(*point).0)
        .collect();
    let average = colors.iter().fold((0u32, 0u32, 0u32), |sum, color| {
        (
            sum.0 + color.r as u32,
            sum.1 + color.g as u32,
            sum.2 + color.b as u32,
        )
    });
    let divisor = colors.len().max(1) as u32;
    let color = Color32::from_rgb(
        (average.0 / divisor) as u8,
        (average.1 / divisor) as u8,
        (average.2 / divisor) as u8,
    );
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(8.0, rgba(color, 28)),
    ));
    painter.add(Shape::line(points.clone(), Stroke::new(2.4, color)));
    if selected {
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(1.0, Color32::WHITE),
        ));
    }

    let led_count = colors.len().min(24);
    let led_points = circle_points(center, radius, axis, led_count.max(1));
    for (index, point) in led_points.iter().enumerate() {
        if let Some(rgb) = colors.get(index) {
            let (screen, _) = projection.project(*point);
            painter.circle_filled(screen, 2.2, Color32::from_rgb(rgb.r, rgb.g, rgb.b));
        }
    }
}

fn draw_rgb_strip(
    painter: &Painter,
    projection: &Projection,
    start: Vec3,
    end: Vec3,
    colors: &[Rgb],
    selected: bool,
) {
    let (a, _) = projection.project(start);
    let (b, _) = projection.project(end);
    let average = colors.iter().fold((0u32, 0u32, 0u32), |sum, color| {
        (
            sum.0 + color.r as u32,
            sum.1 + color.g as u32,
            sum.2 + color.b as u32,
        )
    });
    let divisor = colors.len().max(1) as u32;
    let color = Color32::from_rgb(
        (average.0 / divisor) as u8,
        (average.1 / divisor) as u8,
        (average.2 / divisor) as u8,
    );
    painter.line_segment([a, b], Stroke::new(8.0, rgba(color, 26)));
    painter.line_segment([a, b], Stroke::new(2.2, color));
    if selected {
        painter.line_segment([a, b], Stroke::new(0.9, Color32::WHITE));
    }
    let count = colors.len().min(32).max(1);
    for (index, rgb) in colors.iter().take(count).enumerate() {
        let t = if count == 1 {
            0.0
        } else {
            index as f32 / (count - 1) as f32
        };
        let point = Pos2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        painter.circle_filled(point, 2.0, Color32::from_rgb(rgb.r, rgb.g, rgb.b));
    }
}

fn zone_colors(
    frames: &HashMap<String, Vec<Rgb>>,
    zones: &[crate::model::SceneZone],
    zone_id: &str,
) -> Vec<Rgb> {
    if let Some(colors) = frames.get(zone_id) {
        return colors.clone();
    }
    zones
        .iter()
        .find(|zone| zone.id == zone_id)
        .map(|zone| resample_colors(&[], zone.led_count.max(1) as usize))
        .unwrap_or_default()
}

pub fn show(
    ui: &mut Ui,
    state: &mut PreviewState,
    lighting: &LightingConfig,
    frames: &HashMap<String, Vec<Rgb>>,
    selected_zone: Option<&str>,
    show_panels: bool,
) -> egui::Response {
    let available = Vec2::new(
        ui.available_width().max(20.0),
        ui.available_height().max(20.0),
    );
    let (rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 14.0, Color32::from_rgb(8, 11, 16));

    if response.dragged_by(egui::PointerButton::Primary) {
        let delta = ui.input(|input| input.pointer.delta());
        state.yaw += delta.x * 0.008;
        state.pitch = (state.pitch + delta.y * 0.006).clamp(-0.15, 1.15);
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.raw_scroll_delta.y);
        if scroll.abs() > 0.0 {
            state.zoom = (state.zoom * (1.0 + scroll * 0.0015)).clamp(0.55, 2.6);
        }
    }

    let scale = rect.height().min(rect.width() * 1.35) * 0.0019 * state.zoom;
    let projection = Projection {
        yaw: state.yaw,
        pitch: state.pitch,
        scale,
        center: rect.center() + Vec2::new(0.0, rect.height() * 0.04),
    };

    // Floor grid.
    for step in -4..=4 {
        let offset = step as f32 * 50.0;
        frame_line(
            &painter,
            &projection,
            Vec3::new(-120.0, 0.0, offset),
            Vec3::new(120.0, 0.0, offset),
            0.8,
            Color32::from_rgba_unmultiplied(70, 82, 100, 32),
        );
        frame_line(
            &painter,
            &projection,
            Vec3::new(offset, 0.0, -120.0),
            Vec3::new(offset, 0.0, 120.0),
            0.8,
            Color32::from_rgba_unmultiplied(70, 82, 100, 32),
        );
    }

    let mut faces: Vec<Face> = Vec::new();
    let panel_alpha = 58;
    let frame = Color32::from_rgb(48, 56, 68);
    let aluminum = Color32::from_rgb(60, 68, 80);
    let dark = Color32::from_rgb(31, 37, 47);
    let pcb = Color32::from_rgb(18, 50, 38);
    let steel = Color32::from_rgb(74, 84, 98);

    // Case frame + panels.
    for x in [-63.0, 63.0] {
        for z in [-105.0, 105.0] {
            add_box(
                &mut faces,
                &projection,
                Vec3::new(x, 167.5, z),
                Vec3::new(9.0, 335.0, 9.0),
                frame,
                255,
            );
        }
    }
    add_box(
        &mut faces,
        &projection,
        Vec3::new(0.0, 332.0, -105.0),
        Vec3::new(135.0, 8.0, 8.0),
        frame,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(0.0, 332.0, 105.0),
        Vec3::new(135.0, 8.0, 8.0),
        frame,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(0.0, 3.0, -105.0),
        Vec3::new(135.0, 8.0, 8.0),
        frame,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(0.0, 3.0, 105.0),
        Vec3::new(135.0, 8.0, 8.0),
        frame,
        255,
    );

    if show_panels {
        // Panels are optional and intentionally translucent so the preview remains useful.
        add_box(
            &mut faces,
            &projection,
            Vec3::new(-67.5, 167.5, 0.0),
            Vec3::new(2.4, 323.0, 208.0),
            aluminum,
            panel_alpha,
        );
        add_box(
            &mut faces,
            &projection,
            Vec3::new(67.5, 167.5, 0.0),
            Vec3::new(2.4, 323.0, 208.0),
            aluminum,
            panel_alpha,
        );
        add_box(
            &mut faces,
            &projection,
            Vec3::new(0.0, 333.8, 0.0),
            Vec3::new(123.0, 2.4, 208.0),
            aluminum,
            panel_alpha,
        );
        add_box(
            &mut faces,
            &projection,
            Vec3::new(0.0, 1.5, 0.0),
            Vec3::new(123.0, 3.0, 208.0),
            aluminum,
            panel_alpha,
        );
        add_box(
            &mut faces,
            &projection,
            Vec3::new(0.0, 167.5, -108.5),
            Vec3::new(121.0, 323.0, 3.0),
            aluminum,
            panel_alpha,
        );
    }

    // Internals.
    add_box(
        &mut faces,
        &projection,
        Vec3::new(0.0, 167.5, 0.0),
        Vec3::new(2.5, 309.0, 196.0),
        dark,
        175,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(4.5, 170.0, 0.0),
        Vec3::new(3.0, 170.0, 170.0),
        pcb,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(26.5, 170.0, -55.0),
        Vec3::new(43.0, 66.0, 66.0),
        dark,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(48.0, 170.0, -55.0),
        Vec3::new(3.0, 68.0, 68.0),
        steel,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(11.0, 205.0, 30.5),
        Vec3::new(5.0, 96.0, 22.0),
        dark,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(13.0, 112.0, 34.0),
        Vec3::new(12.0, 46.0, 56.0),
        steel,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(18.0, 238.0, 66.0),
        Vec3::new(26.0, 46.0, 52.0),
        steel,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(-34.0, 178.0, 12.0),
        Vec3::new(44.0, 244.0, 132.0),
        Color32::from_rgb(34, 41, 52),
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(-56.5, 178.0, 12.0),
        Vec3::new(1.5, 210.0, 112.0),
        steel,
        210,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(32.0, 42.0, -55.0),
        Vec3::new(60.0, 63.0, 100.0),
        dark,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(53.5, 161.0, 40.0),
        Vec3::new(27.0, 272.0, 120.0),
        steel,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(27.5, 95.0, 40.0),
        Vec3::new(24.0, 120.0, 120.0),
        dark,
        255,
    );
    add_box(
        &mut faces,
        &projection,
        Vec3::new(27.5, 227.0, 40.0),
        Vec3::new(24.0, 120.0, 120.0),
        dark,
        255,
    );

    faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    for face in faces {
        painter.add(Shape::convex_polygon(face.points, face.fill, face.stroke));
    }

    // RGB overlays. These are drawn after the chassis for readability.
    let zones = &lighting.zones;
    for zone in zones {
        if !zone.enabled {
            continue;
        }
        let colors = zone_colors(frames, zones, &zone.id);
        let selected = selected_zone == Some(zone.id.as_str());
        match zone.placement {
            Placement::PumpBlock => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(49.0, 170.0, -55.0),
                28.0,
                0,
                &colors,
                selected,
            ),
            Placement::AioFan1 => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(41.0, 95.0, 40.0),
                52.0,
                0,
                &colors,
                selected,
            ),
            Placement::AioFan2 => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(41.0, 227.0, 40.0),
                52.0,
                0,
                &colors,
                selected,
            ),
            Placement::TopFan => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(28.0, 323.0, 55.0),
                52.0,
                1,
                &colors,
                selected,
            ),
            Placement::BottomFan1 => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(28.0, 12.0, -55.0),
                52.0,
                1,
                &colors,
                selected,
            ),
            Placement::BottomFan2 => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(28.0, 12.0, 55.0),
                52.0,
                1,
                &colors,
                selected,
            ),
            Placement::GpuBlock => draw_rgb_ring(
                &painter,
                &projection,
                Vec3::new(-57.0, 180.0, 10.0),
                27.0,
                0,
                &colors,
                selected,
            ),
            Placement::BottomStrip => draw_rgb_strip(
                &painter,
                &projection,
                Vec3::new(-55.0, 7.0, -102.0),
                Vec3::new(55.0, 7.0, -102.0),
                &colors,
                selected,
            ),
            Placement::TopStrip => draw_rgb_strip(
                &painter,
                &projection,
                Vec3::new(-55.0, 330.0, 95.0),
                Vec3::new(55.0, 330.0, 95.0),
                &colors,
                selected,
            ),
            Placement::FrontStrip => draw_rgb_strip(
                &painter,
                &projection,
                Vec3::new(45.0, 30.0, -106.0),
                Vec3::new(45.0, 300.0, -106.0),
                &colors,
                selected,
            ),
            Placement::RearStrip => draw_rgb_strip(
                &painter,
                &projection,
                Vec3::new(-55.0, 300.0, 106.0),
                Vec3::new(55.0, 300.0, 106.0),
                &colors,
                selected,
            ),
            Placement::SideDiffuser => {
                let corners = [
                    Vec3::new(66.0, 9.0, -100.0),
                    Vec3::new(66.0, 9.0, 100.0),
                    Vec3::new(66.0, 327.0, 100.0),
                    Vec3::new(66.0, 327.0, -100.0),
                ];
                for pair in [[0usize, 1], [1, 2], [2, 3], [3, 0]] {
                    draw_rgb_strip(
                        &painter,
                        &projection,
                        corners[pair[0]],
                        corners[pair[1]],
                        &colors,
                        selected,
                    );
                }
            }
        }
    }

    painter.text(
        rect.left_top() + Vec2::new(14.0, 12.0),
        Align2::LEFT_TOP,
        "T1 v2.5 · 9.95 L",
        FontId::proportional(11.0),
        Color32::from_rgb(126, 140, 158),
    );
    if !lighting.enabled {
        painter.text(
            rect.right_top() + Vec2::new(-14.0, 12.0),
            Align2::RIGHT_TOP,
            "LIGHTING OFF",
            FontId::proportional(10.0),
            Color32::from_rgb(150, 96, 104),
        );
    }
    response
}
