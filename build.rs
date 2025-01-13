use serde::Deserialize;

#[derive(Deserialize)]
struct Station {
    ssid: String,
    auth_method: String,
    password: String,
}

#[derive(Deserialize)]
struct AccessPoint {
    ssid: String,
    auth_method: String,
    password: String,
}

#[derive(Deserialize)]
struct Voltages {
    adc_reference_voltage: f64,
    probe_disconnected: f64,
    max_voltage_absolute: f64,
}

#[derive(Deserialize)]
struct Precision {
    tolerance_factor: f64,
    min_voltage_difference: f64,
    initial_frequency: f64,
}

#[derive(Deserialize)]
struct Config {
    station: Station,
    access_point: AccessPoint,
    voltages: Voltages,
    precision: Precision,
}

fn add_env_var(name: &str, value: &str) {
    println!("cargo:rustc-env={}={}", name, value);
}

fn main() {
    let config: Config =
        toml::from_str(include_str!("config.toml")).expect("config.toml had unexpected format");

    // Station
    let sta_creds = config.station;
    assert!(sta_creds.ssid.len() <= 32);
    assert!(sta_creds.password.len() <= 64);

    add_env_var("station_ssid", &sta_creds.ssid);
    add_env_var("station_auth_method", &sta_creds.auth_method.to_lowercase());
    add_env_var("station_password", &sta_creds.password);

    // Access point
    let ap_creds = config.access_point;
    assert!(ap_creds.ssid.len() <= 32);
    assert!(ap_creds.password.len() <= 64);

    add_env_var("access_point_ssid", &ap_creds.ssid);
    add_env_var(
        "access_point_auth_method",
        &ap_creds.auth_method.to_lowercase(),
    );
    add_env_var("access_point_password", &ap_creds.password);

    // Voltages
    let voltages = config.voltages;
    add_env_var(
        "adc_reference_voltage",
        &voltages.adc_reference_voltage.to_string(),
    );
    add_env_var(
        "probe_disconnected",
        &voltages.probe_disconnected.to_string(),
    );
    add_env_var(
        "max_voltage_absolute",
        &voltages.max_voltage_absolute.to_string(),
    );

    // Precision
    let precision = config.precision;
    assert!(precision.tolerance_factor >= 0.0 && precision.tolerance_factor <= 1.0);
    add_env_var("tolerance_factor", &precision.tolerance_factor.to_string());
    add_env_var(
        "min_voltage_difference",
        &precision.min_voltage_difference.to_string(),
    );
    add_env_var(
        "initial_frequency",
        &precision.initial_frequency.to_string(),
    );

    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
}
