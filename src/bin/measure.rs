use esp_hal::{analog::adc::Adc, delay::Delay, time::now};
use crate::websocket_logistics::{OscilliscopePoint, CyclicWriter};

pub fn measuring_task<const L: usize, ADCI>(
    _adc: Adc<'_, ADCI>,
    point_buffer_writer: &mut CyclicWriter<'_, L, OscilliscopePoint>,
) -> ! {
    let delay = Delay::new();
    loop {
        // Dummy sawtooth waveform.
        let point = OscilliscopePoint {
            voltage: (now().ticks() as f64 / 1_000_000f64) % 2f64,
            second: (now().duration_since_epoch().to_micros() as f64) * 0.000001f64,
        };

        match point_buffer_writer.insert_significant(point, 0.1f64) {
            Ok(_) => (),
            Err(_) => (),
        }
        delay.delay_micros(10);
    }
}
