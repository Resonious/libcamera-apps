use std::time::Duration;

use rppal::pwm::{Channel, Polarity, Pwm};

use std::f32::consts::PI;

// From the SF006C Clutch Gear Digital Servo datasheet

// SF006C
// const PERIOD_MS: u64 = 10;
// const PULSE_MIN_US: u64 = 500;
// const PULSE_NEUTRAL_US: u64 = 1500;
// const PULSE_MAX_US: u64 = 2500;

// SG90
const PERIOD_MS: u64 = 20;
const PULSE_MIN_US: u64 = 1000;
const PULSE_NEUTRAL_US: u64 = 1500;
const PULSE_MAX_US: u64 = 2000;

#[derive(Clone, Copy, Debug)]
pub enum Movement {
    RotationX(f32),
    RotationY(f32),
}

pub struct Servos {
    y_rotation: Pwm,
    x_rotation: Pwm,
}

impl Servos {
    pub fn new() -> Self {
        Self {
            y_rotation: Pwm::with_period(
                Channel::Pwm0,
                Duration::from_millis(PERIOD_MS),
                Duration::from_micros(PULSE_NEUTRAL_US),
                Polarity::Normal,
                true,
            )
            .expect("Failed to init PWM0"),
            x_rotation: Pwm::with_period(
                Channel::Pwm1,
                Duration::from_millis(PERIOD_MS),
                Duration::from_micros(PULSE_NEUTRAL_US),
                Polarity::Normal,
                true,
            )
            .expect("Failed to init PWM1"),
        }
    }

    fn rotate(pwm: &Pwm, radians: f32) {
        let pulse = Self::radians_to_pulse(radians);

        if pulse < PULSE_MIN_US {
            panic!("pulse lower than expected {pulse}");
        }
        if pulse > PULSE_MAX_US {
            panic!("pulse higher than expected {pulse}");
        }

        pwm.set_pulse_width(Duration::from_micros(pulse))
            .unwrap_or_else(|e| {
                eprintln!("{e:?} during rotate..");
            });
    }

    fn radians_to_pulse(radians: f32) -> u64 {
        let normalized_pulse_max = (PULSE_MAX_US - PULSE_MIN_US) as f32;
        let normalized_radians_max = PI;
        let to_pulse = normalized_pulse_max / normalized_radians_max;

        // 0 is center, range from -pi/2 to pi/2
        let normalized_radians = radians + PI / 2.0;
        let normalized_pulse = (normalized_radians * to_pulse) as u64;
        // 1500 is center, range from 500 to 2500 (see constants)
        let pulse = normalized_pulse + PULSE_MIN_US;

        return pulse;
    }

    pub fn do_movement(self: &Self, movement: Movement) {
        match movement {
            Movement::RotationX(x) => self.set_rotation_x(x),
            Movement::RotationY(y) => self.set_rotation_y(y),
        }
    }

    pub fn set_rotation_x(self: &Self, radians: f32) {
        Self::rotate(&self.x_rotation, -radians.clamp(-PI / 4.0, PI / 4.0));
    }

    pub fn set_rotation_y(self: &Self, radians: f32) {
        Self::rotate(&self.y_rotation, radians.clamp(-PI / 2.0, PI / 2.0));
    }
}
