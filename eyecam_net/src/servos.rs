use std::time::Duration;

use rppal::pwm::{Channel, Polarity, Pwm};

use std::f32::consts::PI;

// Dead code allowed because right now I'm not switching these
// at runtime.
#[allow(dead_code)]
enum ServoType {
    SF006C,
    SG90,
}

struct ServoDef {
    servo_type: ServoType,
    period_ms: u64,
    pulse_min_us: u64,
    pulse_neutral_us: u64,
    pulse_max_us: u64,
}

impl ServoDef {
    pub fn new(servo_type: ServoType) -> Self {
        match servo_type {
            ServoType::SF006C => Self {
                servo_type,
                period_ms: 10,
                pulse_min_us: 500,
                pulse_neutral_us: 1500,
                pulse_max_us: 2500,
            },
            ServoType::SG90 => Self {
                servo_type,
                period_ms: 20,
                pulse_min_us: 500,
                pulse_neutral_us: 1500,
                pulse_max_us: 2500,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Movement {
    RotationX(f32),
    RotationY(f32),
}

pub struct Servos {
    y_rotation: Pwm,
    x_rotation: Pwm,
    servo: ServoDef,
}

impl Servos {
    pub fn new() -> Self {
        // Manually change this line when changing physical servos
        let servo = ServoDef::new(ServoType::SG90);

        Self {
            y_rotation: Pwm::with_period(
                Channel::Pwm0,
                Duration::from_millis(servo.period_ms),
                Duration::from_micros(servo.pulse_neutral_us),
                Polarity::Normal,
                true,
            )
            .expect("Failed to init PWM0"),
            x_rotation: Pwm::with_period(
                Channel::Pwm1,
                Duration::from_millis(servo.period_ms),
                Duration::from_micros(servo.pulse_neutral_us),
                Polarity::Normal,
                true,
            )
            .expect("Failed to init PWM1"),
            servo,
        }
    }

    fn rotate(servo: &ServoDef, pwm: &Pwm, radians: f32) {
        let pulse = Self::radians_to_pulse(servo, radians);

        if pulse < servo.pulse_min_us {
            panic!("pulse lower than expected {pulse}");
        }
        if pulse > servo.pulse_max_us {
            panic!("pulse higher than expected {pulse}");
        }

        pwm.set_pulse_width(Duration::from_micros(pulse))
            .unwrap_or_else(|e| {
                eprintln!("{e:?} during rotate..");
            });
    }

    fn radians_to_pulse(servo: &ServoDef, radians: f32) -> u64 {
        let normalized_pulse_max = (servo.pulse_max_us - servo.pulse_min_us) as f32;
        let normalized_radians_max = PI;
        let to_pulse = normalized_pulse_max / normalized_radians_max;

        // 0 is center, range from -pi/2 to pi/2
        let normalized_radians = radians + PI / 2.0;
        let normalized_pulse = (normalized_radians * to_pulse) as u64;
        // 1500 is center, range from 500 to 2500 (see constants)
        let pulse = normalized_pulse + servo.pulse_min_us;

        return pulse;
    }

    pub fn do_movement(self: &Self, movement: Movement) {
        match movement {
            Movement::RotationX(x) => self.set_rotation_x(x),
            Movement::RotationY(y) => self.set_rotation_y(y),
        }
    }

    pub fn set_rotation_x(self: &Self, radians: f32) {
        let radians = match self.servo.servo_type {
            ServoType::SF006C => -radians.clamp(-PI / 4.0, PI / 4.0),
            ServoType::SG90 => -radians.clamp(-PI / 2.0, PI / 2.0),
        };

        Self::rotate(&self.servo, &self.x_rotation, radians);
    }

    pub fn set_rotation_y(self: &Self, radians: f32) {
        let radians = match self.servo.servo_type {
            _ => radians.clamp(-PI / 2.0, PI / 2.0),
        };

        Self::rotate(&self.servo, &self.y_rotation, radians);
    }
}
