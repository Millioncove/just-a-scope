use crate::websocket_logistics::{
    is_middle_point_removable_complicated, CyclicWriter, OscilliscopePoint,
};
use esp_hal::{analog::adc::Adc, delay::Delay, time::now};

pub fn measuring_task<const L: usize, ADCI>(
    _adc: Adc<'_, ADCI>,
    point_buffer_writer: &mut CyclicWriter<'_, L, OscilliscopePoint>,
) -> ! {
    let delay = Delay::new();

    let mut before_last: OscilliscopePoint = OscilliscopePoint {
        voltage: 0f64,
        second: 0f64,
    };
    let mut last: OscilliscopePoint = OscilliscopePoint {
        voltage: 0.01f64,
        second: 0.01f64,
    };

    loop {
        let current_second: f64 = now().ticks() as f64 / 1_000_000f64;
        let voltage: f64 = (current_second * current_second / 120f64) % 16f64;
        let voltage: f64 = if voltage > 1f64 {
            voltage * 2.0
        } else {
            voltage
        };

        // Dummy sawtooth waveform.
        let new_point = OscilliscopePoint {
            voltage,
            second: current_second,
        };

        if !is_middle_point_removable_complicated(&before_last, &last, &new_point, 0.001f64, 0.0f64)
        {
            match point_buffer_writer.append(last.clone()) {
                Ok(_) => (),
                Err(_) => (),
            }

            before_last = last;
        }
        last = new_point;

        delay.delay_nanos(100u32);
    }
}
