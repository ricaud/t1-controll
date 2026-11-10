use crate::model::{FanCurve, FanCurvePoint, SensorInfo};

impl FanCurve {
    pub fn normalize(&mut self) {
        let mut points: Vec<FanCurvePoint> = self
            .points
            .iter()
            .copied()
            .filter(|point| point.temp_c.is_finite())
            .map(|point| FanCurvePoint {
                temp_c: point.temp_c.clamp(-20.0, 120.0),
                rpm: point.rpm.min(10000),
            })
            .collect();
        points.sort_by(|a, b| a.temp_c.total_cmp(&b.temp_c));

        let mut deduped: Vec<FanCurvePoint> = Vec::new();
        for point in points {
            match deduped.last_mut() {
                Some(previous) if (previous.temp_c - point.temp_c).abs() < 0.01 => {
                    *previous = point;
                }
                _ => deduped.push(point),
            }
        }
        if deduped.len() < 2 {
            deduped = FanCurve::default().points;
        }
        self.points = deduped;
        self.min_rpm = self.min_rpm.min(self.max_rpm);
    }

    pub fn rpm_at(&self, temp_c: f32) -> f32 {
        let Some(first) = self.points.first() else {
            return self.min_rpm as f32;
        };
        let Some(last) = self.points.last() else {
            return self.min_rpm as f32;
        };
        if temp_c <= first.temp_c {
            return first.rpm.clamp(self.min_rpm, self.max_rpm) as f32;
        }
        if temp_c >= last.temp_c {
            return last.rpm.clamp(self.min_rpm, self.max_rpm) as f32;
        }
        for window in self.points.windows(2) {
            let a = window[0];
            let b = window[1];
            if temp_c <= b.temp_c {
                let span = b.temp_c - a.temp_c;
                let t = if span <= 0.0 {
                    0.0
                } else {
                    (temp_c - a.temp_c) / span
                };
                let rpm = a.rpm as f32 + (b.rpm as f32 - a.rpm as f32) * t;
                return rpm.clamp(self.min_rpm as f32, self.max_rpm as f32);
            }
        }
        last.rpm.clamp(self.min_rpm, self.max_rpm) as f32
    }
}

pub fn duty_for_rpm(rpm: f32, max_rpm: u16, min_duty: u8) -> u8 {
    if rpm <= 0.0 {
        return 0;
    }
    let percent = (rpm / max_rpm.max(1) as f32) * 100.0;
    (percent.round() as i32).clamp(min_duty.min(100) as i32, 100) as u8
}

/// Hysteresis + slew limiting around a fan curve.
#[derive(Clone, Debug)]
pub struct CurveFollower {
    curve: FanCurve,
    rpm: f32,
    held_temp: Option<f32>,
}

impl CurveFollower {
    pub fn new(curve: FanCurve, initial_rpm: Option<f32>) -> Self {
        let rpm = initial_rpm
            .unwrap_or(curve.min_rpm as f32)
            .clamp(curve.min_rpm as f32, curve.max_rpm as f32);
        Self {
            curve,
            rpm,
            held_temp: None,
        }
    }

    pub fn set_curve(&mut self, curve: FanCurve) {
        self.rpm = self.rpm.clamp(curve.min_rpm as f32, curve.max_rpm as f32);
        self.curve = curve;
    }

    pub fn reset(&mut self, rpm: Option<f32>) {
        self.held_temp = None;
        self.rpm = rpm
            .unwrap_or(self.curve.min_rpm as f32)
            .clamp(self.curve.min_rpm as f32, self.curve.max_rpm as f32);
    }

    pub fn current_rpm(&self) -> f32 {
        self.rpm
    }

    pub fn update(&mut self, temp_c: f32, dt_sec: f32) -> f32 {
        if !temp_c.is_finite() {
            return self.rpm;
        }
        let dt = dt_sec.clamp(0.0, 10.0);
        let hysteresis = self.curve.hysteresis_c.clamp(0.0, 20.0);
        match self.held_temp {
            None => self.held_temp = Some(temp_c),
            Some(held) if temp_c > held + hysteresis => self.held_temp = Some(temp_c),
            Some(held) if temp_c < held - hysteresis => self.held_temp = Some(temp_c),
            _ => {}
        }
        let target = self.curve.rpm_at(self.held_temp.unwrap_or(temp_c));
        if target > self.rpm {
            self.rpm = target.min(self.rpm + self.curve.max_step_up_rpm_per_sec.max(1) as f32 * dt);
        } else if target < self.rpm {
            self.rpm =
                target.max(self.rpm - self.curve.max_step_down_rpm_per_sec.max(1) as f32 * dt);
        }
        self.rpm = self
            .rpm
            .clamp(self.curve.min_rpm as f32, self.curve.max_rpm as f32);
        self.rpm
    }
}

fn alias_matches(alias: &str, sensor: &SensorInfo) -> bool {
    let label = sensor.label.to_lowercase();
    let source = sensor.source.to_lowercase();
    match alias {
        "cpu" => ["cpu", "package", "tctl", "tdie", "core", "processor"]
            .iter()
            .any(|term| label.contains(term) || source.contains(term)),
        "gpu" => ["gpu", "edge", "hotspot", "junction"]
            .iter()
            .any(|term| label.contains(term) || source.contains(term)),
        "coolant" => ["coolant", "water", "liquid", "aio", "pump"]
            .iter()
            .any(|term| label.contains(term) || source.contains(term)),
        "motherboard" => ["motherboard", "system", "chipset", "vrm", "board"]
            .iter()
            .any(|term| label.contains(term) || source.contains(term)),
        _ => false,
    }
}

pub fn resolve_sensor(setting: &str, sensors: &[SensorInfo]) -> Option<SensorInfo> {
    if let Some(exact) = sensors.iter().find(|sensor| sensor.id == setting) {
        return Some(exact.clone());
    }
    let matches: Vec<&SensorInfo> = sensors
        .iter()
        .filter(|sensor| alias_matches(setting, sensor))
        .collect();
    matches
        .iter()
        .find(|sensor| sensor.value_c.is_some())
        .copied()
        .or_else(|| matches.first().copied())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SensorInfo;

    fn curve() -> FanCurve {
        FanCurve {
            sensor: "cpu".into(),
            points: vec![
                FanCurvePoint {
                    temp_c: 30.0,
                    rpm: 600,
                },
                FanCurvePoint {
                    temp_c: 60.0,
                    rpm: 1200,
                },
                FanCurvePoint {
                    temp_c: 90.0,
                    rpm: 2000,
                },
            ],
            hysteresis_c: 3.0,
            min_rpm: 500,
            max_rpm: 2000,
            max_step_up_rpm_per_sec: 400,
            max_step_down_rpm_per_sec: 100,
        }
    }

    fn sensor(label: &str, value: f32) -> SensorInfo {
        SensorInfo {
            id: label.into(),
            label: label.into(),
            source: "test".into(),
            value_c: Some(value),
        }
    }

    #[test]
    fn interpolates_and_clamps() {
        let curve = curve();
        assert_eq!(curve.rpm_at(45.0), 900.0);
        assert_eq!(curve.rpm_at(75.0), 1600.0);
        assert_eq!(curve.rpm_at(0.0), 600.0);
        assert_eq!(curve.rpm_at(120.0), 2000.0);
    }

    #[test]
    fn follower_slews_up_and_down() {
        let mut follower = CurveFollower::new(curve(), Some(500.0));
        assert_eq!(follower.update(90.0, 1.0), 900.0);
        assert_eq!(follower.update(90.0, 1.0), 1300.0);
        assert_eq!(follower.update(90.0, 1.0), 1700.0);
        assert_eq!(follower.update(90.0, 1.0), 2000.0);
        assert_eq!(follower.update(30.0, 1.0), 1900.0);
        assert_eq!(follower.update(30.0, 20.0), 900.0);
        assert_eq!(follower.update(30.0, 20.0), 600.0);
    }

    #[test]
    fn follower_holds_inside_hysteresis() {
        let mut follower = CurveFollower::new(
            FanCurve {
                max_step_up_rpm_per_sec: 5000,
                ..curve()
            },
            Some(600.0),
        );
        follower.update(60.0, 1.0);
        assert_eq!(follower.current_rpm(), 1200.0);
        follower.update(62.0, 1.0);
        assert_eq!(follower.current_rpm(), 1200.0);
        follower.update(64.0, 1.0);
        assert!((follower.current_rpm() - 1306.67).abs() < 0.1);
    }

    #[test]
    fn duty_has_a_floor_and_ceiling() {
        assert_eq!(duty_for_rpm(0.0, 2000, 20), 0);
        assert_eq!(duty_for_rpm(1000.0, 2000, 20), 50);
        assert_eq!(duty_for_rpm(10.0, 2000, 20), 20);
        assert_eq!(duty_for_rpm(4000.0, 2000, 20), 100);
    }

    #[test]
    fn resolves_sensor_aliases() {
        let sensors = vec![sensor("Package id 0", 51.0), sensor("GPU Edge", 62.0)];
        assert_eq!(resolve_sensor("cpu", &sensors).unwrap().value_c, Some(51.0));
        assert_eq!(resolve_sensor("gpu", &sensors).unwrap().value_c, Some(62.0));
        assert!(resolve_sensor("coolant", &sensors).is_none());
    }
}
