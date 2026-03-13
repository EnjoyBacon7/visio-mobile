//! Adaptive context engine.
//!
//! Determines the best meeting mode (Office, Pedestrian, Car) based on
//! environmental signals such as network type, motion, and Bluetooth car kit.

/// High-level meeting mode derived from context signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveMode {
    Office,
    Pedestrian,
    Car,
}

/// Type of network the device is currently connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkType {
    Wifi,
    Cellular,
    Unknown,
}

/// A single context signal reported by the platform layer.
#[derive(Debug, Clone)]
pub enum ContextSignal {
    NetworkType(NetworkType),
    MotionDetected(bool),
    BluetoothCarKit(bool),
}

/// Engine that aggregates context signals and derives the current adaptive mode.
#[derive(Debug, Clone)]
pub struct AdaptiveEngine {
    motion: bool,
    bluetooth_car: bool,
    mode_override: Option<AdaptiveMode>,
}

impl Default for AdaptiveEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveEngine {
    /// Creates a new engine with default state (Office mode).
    pub fn new() -> Self {
        Self {
            motion: false,
            bluetooth_car: false,
            mode_override: None,
        }
    }

    /// Returns the current mode, respecting any manual override.
    pub fn current_mode(&self) -> AdaptiveMode {
        self.mode_override.unwrap_or_else(|| self.compute_mode())
    }

    /// Updates internal state from a context signal.
    ///
    /// Returns `Some(new_mode)` if the effective mode changed, `None` otherwise.
    pub fn update_signal(&mut self, signal: ContextSignal) -> Option<AdaptiveMode> {
        let old = self.current_mode();

        match signal {
            ContextSignal::NetworkType(_) => {} // Stored for future use
            ContextSignal::MotionDetected(m) => self.motion = m,
            ContextSignal::BluetoothCarKit(b) => self.bluetooth_car = b,
        }

        let new = self.current_mode();
        if new != old { Some(new) } else { None }
    }

    /// Sets (or clears) a manual mode override.
    pub fn set_override(&mut self, mode: Option<AdaptiveMode>) {
        self.mode_override = mode;
    }

    /// Computes the mode from current signals (ignoring override).
    ///
    /// Priority: Car (bluetooth) > Pedestrian (motion) > Office.
    fn compute_mode(&self) -> AdaptiveMode {
        if self.bluetooth_car {
            AdaptiveMode::Car
        } else if self.motion {
            AdaptiveMode::Pedestrian
        } else {
            AdaptiveMode::Office
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_office() {
        let engine = AdaptiveEngine::new();
        assert_eq!(engine.current_mode(), AdaptiveMode::Office);
    }

    #[test]
    fn motion_becomes_pedestrian() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::MotionDetected(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Pedestrian);
    }

    #[test]
    fn bluetooth_car_kit_becomes_car() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::BluetoothCarKit(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Car);
    }

    #[test]
    fn car_mode_takes_priority_over_pedestrian() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::NetworkType(NetworkType::Cellular));
        engine.update_signal(ContextSignal::MotionDetected(true));
        engine.update_signal(ContextSignal::BluetoothCarKit(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Car);
    }

    #[test]
    fn manual_override_locks_mode() {
        let mut engine = AdaptiveEngine::new();
        engine.set_override(Some(AdaptiveMode::Car));
        assert_eq!(engine.current_mode(), AdaptiveMode::Car);

        // Signals should not change the effective mode while overridden.
        engine.update_signal(ContextSignal::NetworkType(NetworkType::Cellular));
        engine.update_signal(ContextSignal::MotionDetected(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Car);
    }

    #[test]
    fn clear_override_returns_to_auto() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::NetworkType(NetworkType::Cellular));
        engine.update_signal(ContextSignal::MotionDetected(true));
        engine.set_override(Some(AdaptiveMode::Office));
        assert_eq!(engine.current_mode(), AdaptiveMode::Office);

        engine.set_override(None);
        assert_eq!(engine.current_mode(), AdaptiveMode::Pedestrian);
    }

    #[test]
    fn motion_stops_returns_to_office() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::MotionDetected(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Pedestrian);

        engine.update_signal(ContextSignal::MotionDetected(false));
        assert_eq!(engine.current_mode(), AdaptiveMode::Office);
    }

    #[test]
    fn car_to_pedestrian_when_bluetooth_disconnects_while_moving() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::MotionDetected(true));
        engine.update_signal(ContextSignal::BluetoothCarKit(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Car);

        let result = engine.update_signal(ContextSignal::BluetoothCarKit(false));
        assert_eq!(result, Some(AdaptiveMode::Pedestrian));
        assert_eq!(engine.current_mode(), AdaptiveMode::Pedestrian);
    }

    #[test]
    fn car_to_office_when_bluetooth_disconnects_while_stationary() {
        let mut engine = AdaptiveEngine::new();
        engine.update_signal(ContextSignal::BluetoothCarKit(true));
        assert_eq!(engine.current_mode(), AdaptiveMode::Car);

        let result = engine.update_signal(ContextSignal::BluetoothCarKit(false));
        assert_eq!(result, Some(AdaptiveMode::Office));
        assert_eq!(engine.current_mode(), AdaptiveMode::Office);
    }
}
