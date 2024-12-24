use serde::Deserialize;

#[derive(Deserialize)]
struct Station {
    ssid: String,
    auth_method: String,
    password: String
}

#[derive(Deserialize)]
struct WifiConfig {
    station: Station
}

fn main() {
    let wifi_config: WifiConfig = toml::from_str(include_str!("credentials.toml")).
    expect("credentials.toml had unexpected format");

    let sta_creds = wifi_config.station;
    assert!(sta_creds.ssid.len() <= 32);
    assert!(sta_creds.password.len() <= 64);
    
    println!("cargo::rustc-env=station_ssid={}", sta_creds.ssid);
    println!("cargo::rustc-env=station_auth_method={}", sta_creds.auth_method.to_lowercase());
    println!("cargo::rustc-env=station_password={}", sta_creds.password);

    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
}
