use crate::model::{Direction, Effect, LightingConfig, Pattern, Rgb, SceneZone, ZoneEffect};
use std::collections::HashMap;

pub fn speed_multiplier(speed: u8) -> f32 {
    0.15 + (speed.min(100) as f32 / 100.0) * 1.85
}

pub fn sample_palette(palette: &[Rgb], position: f32) -> Rgb {
    if palette.is_empty() {
        return Rgb::BLACK;
    }
    if palette.len() == 1 {
        return palette[0];
    }
    let wrapped = position.rem_euclid(1.0);
    let scaled = wrapped * palette.len() as f32;
    let index = (scaled.floor() as usize) % palette.len();
    let next = (index + 1) % palette.len();
    Rgb::mix(palette[index], palette[next], scaled.fract())
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let hue = h.rem_euclid(1.0);
    let sat = s.clamp(0.0, 1.0);
    let val = v.clamp(0.0, 1.0);
    let i = (hue * 6.0).floor();
    let f = hue * 6.0 - i;
    let p = val * (1.0 - sat);
    let q = val * (1.0 - f * sat);
    let t = val * (1.0 - (1.0 - f) * sat);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (val, t, p),
        1 => (q, val, p),
        2 => (p, val, t),
        3 => (p, q, val),
        4 => (t, p, val),
        _ => (val, p, q),
    };
    Rgb::new(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

fn effective_effect(zone: &SceneZone, global: &Effect) -> Effect {
    let ZoneEffect {
        pattern,
        palette,
        brightness,
        speed,
        direction,
    } = &zone.effect;
    Effect {
        pattern: pattern.unwrap_or(global.pattern),
        palette: palette
            .as_ref()
            .filter(|palette| !palette.is_empty())
            .cloned()
            .unwrap_or_else(|| global.palette.clone()),
        brightness: (global.brightness as f32 * (*brightness).unwrap_or(100) as f32 / 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
        speed: speed.unwrap_or(global.speed),
        direction: direction.unwrap_or(global.direction),
    }
}

fn index_for(i: usize, count: usize, direction: Direction) -> usize {
    if direction == Direction::Reverse {
        count.saturating_sub(1).saturating_sub(i)
    } else {
        i
    }
}

fn temperature_color(temp_c: Option<f32>) -> Option<Rgb> {
    let temp = temp_c?;
    if !temp.is_finite() {
        return None;
    }
    let cool = Rgb::new(34, 211, 238);
    let warm = Rgb::new(74, 222, 128);
    let hot = Rgb::new(251, 191, 36);
    let critical = Rgb::new(239, 68, 68);
    if temp <= 40.0 {
        Some(cool)
    } else if temp <= 60.0 {
        Some(Rgb::mix(cool, warm, (temp - 40.0) / 20.0))
    } else if temp <= 75.0 {
        Some(Rgb::mix(warm, hot, (temp - 60.0) / 15.0))
    } else {
        Some(Rgb::mix(
            hot,
            critical,
            ((temp - 75.0) / 15.0).clamp(0.0, 1.0),
        ))
    }
}

pub fn evaluate_zone(
    zone: &SceneZone,
    global: &Effect,
    time_sec: f32,
    temperature_c: Option<f32>,
) -> Vec<Rgb> {
    let led_count = zone.led_count.clamp(1, 512) as usize;
    if !zone.enabled {
        return vec![Rgb::BLACK; led_count];
    }

    let effect = effective_effect(zone, global);
    let palette = if effect.palette.is_empty() {
        vec![Rgb::WHITE]
    } else {
        effect.palette.clone()
    };
    let brightness = effect.brightness.min(100) as f32 / 100.0;
    let speed = speed_multiplier(effect.speed);
    let t = time_sec.max(0.0);
    let count = led_count;

    match effect.pattern {
        Pattern::Off => vec![Rgb::BLACK; count],
        Pattern::Static => {
            let color = sample_palette(&palette, 0.0);
            vec![color.scale(brightness); count]
        }
        Pattern::Gradient => (0..count)
            .map(|i| {
                let position = if count == 1 {
                    0.0
                } else {
                    index_for(i, count, effect.direction) as f32 / (count - 1) as f32
                };
                sample_palette(&palette, position).scale(brightness)
            })
            .collect(),
        Pattern::Breathing => {
            let period = 4.0 / speed;
            let level = 0.12 + 0.88 * (0.5 + 0.5 * (t / period * std::f32::consts::TAU).sin());
            let color = sample_palette(&palette, 0.0);
            vec![color.scale(brightness * level); count]
        }
        Pattern::Cycle => {
            let offset = t * speed * 0.12;
            (0..count)
                .map(|i| {
                    let dir = if effect.direction == Direction::Reverse {
                        -1.0
                    } else {
                        1.0
                    };
                    let position = offset + (i as f32 / count as f32) * dir;
                    sample_palette(&palette, position).scale(brightness)
                })
                .collect()
        }
        Pattern::Rainbow => {
            let offset = t * speed * (0.07 + (effect.speed as f32 / 100.0) * 0.05);
            (0..count)
                .map(|i| {
                    let position = index_for(i, count, effect.direction) as f32 / count as f32;
                    hsv_to_rgb(position + offset, 1.0, 1.0).scale(brightness)
                })
                .collect()
        }
        Pattern::Wave => {
            let phase = t * speed * 1.6;
            (0..count)
                .map(|i| {
                    let position = index_for(i, count, effect.direction) as f32 / count as f32;
                    let wave = 0.5 + 0.5 * (position * std::f32::consts::TAU - phase).sin();
                    let color = sample_palette(&palette, position * 0.55 + wave * 0.45);
                    color.scale(brightness * (0.35 + 0.65 * wave))
                })
                .collect()
        }
        Pattern::Meteor => {
            let tail = ((count as f32 * 0.55).round() as usize).max(3);
            let span = count + tail;
            let head = ((t * speed * (span as f32 / 3.2)) as usize) % span;
            let color = sample_palette(&palette, (t * speed * 0.1) % 1.0);
            (0..count)
                .map(|i| {
                    let ordered = index_for(i, count, effect.direction);
                    let distance = (head + span - (ordered % span)) % span;
                    if distance >= tail {
                        Rgb::BLACK
                    } else {
                        let fade = (1.0 - distance as f32 / tail as f32).powi(2);
                        color.scale(brightness * fade)
                    }
                })
                .collect()
        }
        Pattern::Temperature => {
            if let Some(color) = temperature_color(temperature_c) {
                vec![color.scale(brightness); count]
            } else {
                vec![sample_palette(&palette, 0.0).scale(brightness * 0.4); count]
            }
        }
    }
}

pub fn evaluate_scene(
    lighting: &LightingConfig,
    time_sec: f32,
    temperature_c: Option<f32>,
) -> HashMap<String, Vec<Rgb>> {
    let mut frames = HashMap::new();
    for zone in &lighting.zones {
        let colors = if lighting.enabled {
            evaluate_zone(zone, &lighting.global, time_sec, temperature_c)
        } else {
            vec![Rgb::BLACK; zone.led_count.clamp(1, 512) as usize]
        };
        frames.insert(zone.id.clone(), colors);
    }
    frames
}

pub fn resample_colors(colors: &[Rgb], count: usize) -> Vec<Rgb> {
    let size = count.max(1);
    if colors.is_empty() {
        return vec![Rgb::BLACK; size];
    }
    if colors.len() == size {
        return colors.to_vec();
    }
    (0..size)
        .map(|i| {
            let position = if size == 1 {
                0.0
            } else {
                i as f32 / (size - 1) as f32 * (colors.len() - 1) as f32
            };
            let lo = position.floor() as usize;
            let hi = (lo + 1).min(colors.len() - 1);
            Rgb::mix(colors[lo], colors[hi], position - lo as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SceneZone;

    fn global() -> Effect {
        Effect {
            pattern: Pattern::Wave,
            palette: vec![Rgb::new(34, 211, 238), Rgb::new(129, 140, 248)],
            brightness: 100,
            speed: 50,
            direction: Direction::Forward,
        }
    }

    fn zone() -> SceneZone {
        SceneZone {
            id: "test".into(),
            led_count: 10,
            ..SceneZone::default()
        }
    }

    #[test]
    fn static_respects_brightness() {
        let colors = evaluate_zone(
            &zone(),
            &Effect {
                pattern: Pattern::Static,
                ..global()
            },
            0.0,
            None,
        );
        assert_eq!(colors.len(), 10);
        assert_eq!(colors[0], Rgb::new(34, 211, 238));
        let dim = evaluate_zone(
            &zone(),
            &Effect {
                pattern: Pattern::Static,
                brightness: 0,
                ..global()
            },
            0.0,
            None,
        );
        assert!(dim.iter().all(|color| *color == Rgb::BLACK));
    }

    #[test]
    fn all_patterns_stay_in_range_and_are_deterministic() {
        for pattern in Pattern::ALL {
            for time in [0.0, 1.7, 9.3, 61.2] {
                let a = evaluate_zone(
                    &zone(),
                    &Effect {
                        pattern,
                        ..global()
                    },
                    time,
                    Some(84.0),
                );
                let b = evaluate_zone(
                    &zone(),
                    &Effect {
                        pattern,
                        ..global()
                    },
                    time,
                    Some(84.0),
                );
                assert_eq!(a, b);
                assert_eq!(a.len(), 10);
            }
        }
    }

    #[test]
    fn temperature_moves_from_cyan_to_red() {
        let effect = Effect {
            pattern: Pattern::Temperature,
            ..global()
        };
        let cool = evaluate_zone(&zone(), &effect, 0.0, Some(30.0));
        let hot = evaluate_zone(&zone(), &effect, 0.0, Some(90.0));
        assert!(cool[0].b > cool[0].r);
        assert!(hot[0].r > hot[0].b);
    }

    #[test]
    fn zone_overrides_work() {
        let mut zone = zone();
        zone.effect = ZoneEffect {
            pattern: Some(Pattern::Static),
            palette: Some(vec![Rgb::WHITE]),
            ..ZoneEffect::default()
        };
        let colors = evaluate_zone(&zone, &global(), 3.0, None);
        assert_eq!(colors[0], Rgb::WHITE);
    }

    #[test]
    fn resampling_preserves_endpoints() {
        let colors = [Rgb::BLACK, Rgb::new(255, 0, 0)];
        let resampled = resample_colors(&colors, 5);
        assert_eq!(resampled.len(), 5);
        assert_eq!(resampled[0], Rgb::BLACK);
        assert_eq!(resampled[4], Rgb::new(255, 0, 0));
        assert_eq!(resampled[2], Rgb::new(128, 0, 0));
    }

    #[test]
    fn disabled_scene_is_black() {
        let mut lighting = LightingConfig {
            enabled: false,
            global: global(),
            zones: vec![zone()],
        };
        lighting.zones[0].id = "a".into();
        let frames = evaluate_scene(&lighting, 1.0, None);
        assert!(frames["a"].iter().all(|color| *color == Rgb::BLACK));
    }
}
