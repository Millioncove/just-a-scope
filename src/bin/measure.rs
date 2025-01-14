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

fn take_measurement<const PIN: u8>(
    adc: &mut Adc<'_, ADC1>,
    pin: &mut esp_hal::analog::adc::AdcPin<GpioPin<PIN>, ADC1, AdcCalBasic<ADC1>>,
) -> u16
where
    GpioPin<PIN>: AdcChannel + AnalogPin,
{
    let mut sum = 0u32;
    for _ in 0..32 {
        sum += nb::block!(adc.read_oneshot(pin)).unwrap() as u32;
    }
    return (sum >> 5) as u16;
}

pub fn measuring_task<const L: usize, const PIN: u8>(
    adc_peripheral: ADC1,
    pin: GpioPin<PIN>,
    point_buffer_writer: &mut CyclicWriter<'_, L, OscilliscopePoint>,
    reference_voltage: f64,
    probe_disconnected_voltage: f64,
    max_voltage: f64,
    tolerance_factor: f64,
    min_voltage_difference: f64,
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
        let raw_adc_output = take_measurement(&mut adc, &mut pin);
        let raw_adc_voltage = raw_adc_output as f64 * reference_voltage / 4095f64;
        let adjusted_voltage =
            (raw_adc_voltage - probe_disconnected_voltage) * max_voltage / reference_voltage;
        let new_point = OscilliscopePoint {
            voltage: adjusted_voltage,
            second: current_second,
        };

        let time_difference = last.second - before_last.second;

        if time_difference > 1f64
            || !is_middle_point_removable_complicated(
                &before_last,
                &last,
                &new_point,
                tolerance_factor,
                min_voltage_difference,
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
