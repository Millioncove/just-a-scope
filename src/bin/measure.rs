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

        // Dummy sawtooth waveform.
        let new_point = OscilliscopePoint {
            voltage: 4f64,
            second: current_second,
        };

        let time_difference = last.second - before_last.second;

        if time_difference > 1f64
            || !is_middle_point_removable_complicated(
                &before_last,
                &last,
                &new_point,
                0.001f64,
                0.05f64,
            )
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
