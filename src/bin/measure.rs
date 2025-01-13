use crate::websocket_logistics::{
    is_middle_point_removable_complicated, CyclicWriter, OscilliscopePoint,
};
use esp_hal::{
    analog::adc::{Adc, AdcCalBasic, AdcChannel, AdcConfig, Attenuation},
    delay::Delay,
    gpio::{AnalogPin, GpioPin},
    peripherals::ADC1,
    prelude::nb,
    time::now,
};

const REFERENCE_VOLTAGE: f64 = 3.1f64;
const MAX_VOLTAGE: f64 = 200f64;
const PROBE_DISCONNECTED_VOLTAGE: f64 = 1.1945787545787547f64;

pub fn measuring_task<const L: usize, const PIN: u8>(
    adc_peripheral: ADC1,
    pin: GpioPin<PIN>,
    point_buffer_writer: &mut CyclicWriter<'_, L, OscilliscopePoint>,
) -> !
where
    GpioPin<PIN>: AdcChannel + AnalogPin,
{
    let mut adc_config = AdcConfig::new();

    let mut pin =
        adc_config.enable_pin_with_cal::<_, AdcCalBasic<_>>(pin, Attenuation::Attenuation11dB);
    let mut adc = Adc::new(adc_peripheral, adc_config);

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
        let raw_adc_output = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
        let raw_adc_voltage = raw_adc_output as f64 * REFERENCE_VOLTAGE / 4095f64;
        let adjusted_voltage =
            (raw_adc_voltage - PROBE_DISCONNECTED_VOLTAGE) * MAX_VOLTAGE / REFERENCE_VOLTAGE;
        let new_point = OscilliscopePoint {
            voltage: adjusted_voltage,
            second: current_second,
        };

        let time_difference = last.second - before_last.second;

        if time_difference > 1f64
            || !is_middle_point_removable_complicated(&before_last, &last, &new_point, 0.3f64, 1f64)
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
