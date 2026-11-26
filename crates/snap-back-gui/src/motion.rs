//! Restrained operation feedback. The hero keeps fixed layout bounds while its
//! activity envelope eases in/out. Cached SVGs draw a progress ring for backups
//! and a rotating arc for other operations.
use super::*;
use iced::widget::{stack, svg};
use iced_anim::{Animated, spring::Motion, transition::Easing};
use std::{f32::consts::TAU, time::Instant};

pub(super) struct ActivityMotion {
    envelope: Animated<f32>,
    active: bool,
    backup_fraction: Option<Animated<f32>>,
    angle: f32,
    breath: f32,
    last_frame: Instant,
    arc_frames: [svg::Handle; 360],
    progress_frames: [svg::Handle; 101],
}
impl Default for ActivityMotion {
    fn default() -> Self {
        Self {
            envelope: Animated::spring(
                0.0,
                Motion::SMOOTH.with_duration(Duration::from_millis(420)),
            ),
            active: false,
            backup_fraction: None,
            angle: -TAU / 4.0,
            breath: 0.0,
            last_frame: Instant::now(),
            // Embed rotation inside the image. External SVG rotation in the
            // software renderer can move pixels beyond its damage rectangle.
            // These small, reusable frames keep drawing inside fixed bounds.
            arc_frames: std::array::from_fn(|degrees| {
                svg::Handle::from_memory(format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="112" height="112" viewBox="0 0 112 112"><g transform="rotate({degrees} 56 56)"><circle cx="56" cy="56" r="54" fill="none" stroke="#52deb8" stroke-width="2" stroke-linecap="round" stroke-dasharray="50 289.292"/></g></svg>"##).into_bytes())
            }),
            progress_frames: std::array::from_fn(|percent| {
                let track =
                    r##"<circle cx="56" cy="56" r="54" stroke="#52deb8" stroke-opacity="0.24"/>"##;
                let arc = match percent {
                    0 => String::new(),
                    100 => r##"<circle cx="56" cy="56" r="54" stroke="#52deb8"/>"##.into(),
                    _ => {
                        let filled = 54.0 * TAU * percent as f32 / 100.0;
                        format!(
                            r##"<circle cx="56" cy="56" r="54" stroke="#52deb8" stroke-linecap="round" stroke-dasharray="{filled} 340" transform="rotate(-90 56 56)"/>"##
                        )
                    }
                };
                svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="112" height="112" viewBox="0 0 112 112" fill="none" stroke-width="2">{track}{arc}</svg>"#).into_bytes())
            }),
        }
    }
}
impl ActivityMotion {
    pub fn set_activity(&mut self, active: bool, backup_fraction: Option<f32>) {
        if active {
            match (backup_fraction, &mut self.backup_fraction) {
                (Some(fraction), Some(progress)) if self.active => progress.set_target(fraction),
                (Some(fraction), _) => {
                    self.backup_fraction = Some(Animated::transition(
                        fraction,
                        Easing::EASE_OUT.with_duration(Duration::from_millis(220)),
                    ));
                }
                (None, _) => self.backup_fraction = None,
            }
        }
        if active == self.active {
            return;
        }
        self.active = active;
        self.envelope.set_target(if active { 1.0 } else { 0.0 });
        self.last_frame = Instant::now();
    }
    pub fn tick(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.last_frame)
            .min(Duration::from_millis(48))
            .as_secs_f32();
        self.last_frame = now;
        self.envelope.tick(now);
        if let Some(progress) = &mut self.backup_fraction {
            progress.tick(now);
        } else {
            self.angle = (self.angle + elapsed * 0.95 * self.amount()).rem_euclid(TAU);
        }
        self.breath = (self.breath + elapsed * TAU / 2.8).rem_euclid(TAU);
    }
    pub fn needs_frames(&self) -> bool {
        self.active || self.envelope.is_animating()
    }
    pub fn amount(&self) -> f32 {
        self.envelope.value().clamp(0.0, 1.0)
    }
    pub fn pulse(&self) -> f32 {
        (0.5 - 0.5 * self.breath.cos()) * self.amount()
    }
    pub fn hero(&self) -> Element<'_, Message> {
        let phase = self.breath / TAU;
        let band_diameter = 92.0 + 16.0 * phase;
        let band_opacity = 0.32 * (std::f32::consts::PI * phase).sin() * self.amount();
        let inner = container(text("↶").size(48).color(TEAL))
            .center_x(88)
            .center_y(88)
            .style(|_| container::Style {
                background: Some(Color::from_rgb(0.04, 0.15, 0.13).into()),
                border: Border {
                    color: TEAL.scale_alpha(0.70),
                    width: 2.0,
                    radius: 100.0.into(),
                },
                ..Default::default()
            });
        let disk = container(inner)
            .center_x(112)
            .center_y(112)
            .style(|_| container::Style {
                border: Border {
                    color: TEAL.scale_alpha(0.12),
                    width: 1.0,
                    radius: 100.0.into(),
                },
                ..Default::default()
            });
        // Only this transparent band expands. The logo, its fill, and both
        // boundary circles stay still while the band fades toward the arc.
        let band = container(
            container(Space::new())
                .width(band_diameter)
                .height(band_diameter)
                .style(move |_| container::Style {
                    border: Border {
                        color: TEAL.scale_alpha(band_opacity),
                        width: 2.0,
                        radius: 100.0.into(),
                    },
                    ..Default::default()
                }),
        )
        .center_x(112)
        .center_y(112);
        let arc_frame = (self.angle.rem_euclid(TAU) / TAU * 360.0) as usize % 360;
        let ring = match &self.backup_fraction {
            Some(progress) => {
                let percent = (progress.value().clamp(0.0, 1.0) * 100.0) as usize;
                &self.progress_frames[percent]
            }
            None => &self.arc_frames[arc_frame],
        };
        // All handles are created once; animation just selects a cached frame.
        stack![
            disk,
            band,
            svg(ring.clone())
                .width(112)
                .height(112)
                .opacity(0.68 * self.amount())
        ]
        .clip(true)
        .width(112)
        .height(112)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_changes_over_frames_and_settles_without_idle_frames() {
        let mut motion = ActivityMotion::default();
        assert!(!motion.needs_frames());
        motion.set_activity(true, None);
        assert!(motion.needs_frames());
        let start = Instant::now();
        let start_angle = motion.angle;
        for i in 1..=80 {
            motion.tick(start + Duration::from_millis(i * 16));
        }
        assert!(motion.amount() > 0.9 && motion.pulse() > 0.0 && motion.angle != start_angle);
        motion.set_activity(false, None);
        let stop = Instant::now();
        for i in 1..=600 {
            motion.tick(stop + Duration::from_millis(i * 16));
        }
        assert!(!motion.needs_frames());
        assert!(motion.amount() < 0.001);
    }
    #[test]
    fn repeated_busy_status_does_not_restart_the_envelope() {
        let mut motion = ActivityMotion::default();
        motion.set_activity(true, None);
        let start = Instant::now();
        for i in 1..=30 {
            motion.tick(start + Duration::from_millis(i * 16));
        }
        let amount = motion.amount();
        let last = motion.last_frame;
        motion.set_activity(true, None);
        assert_eq!(motion.amount(), amount);
        assert_eq!(motion.last_frame, last);
    }
}
