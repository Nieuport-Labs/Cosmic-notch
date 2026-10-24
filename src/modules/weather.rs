// SPDX-License-Identifier: GPL-3.0-only

//! The weather from [Open-Meteo](https://open-meteo.com), which needs no key.
//! The island sends it the coordinates of the place the user picked, and only
//! when the weather is switched on.

use serde::{Deserialize, Serialize};

/// A place to show the weather of, as the geocoder found it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// The weather now and today's range, in °C.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weather {
    pub temperature: f32,
    pub high: f32,
    pub low: f32,
    /// WMO weather interpretation code.
    pub code: u8,
    pub day: bool,
}

/// Finds a place by name, in the user's language.
pub async fn find_place(query: String, language: String) -> Result<Option<Place>, String> {
    #[derive(Deserialize)]
    struct Found {
        name: String,
        #[serde(default)]
        country: String,
        latitude: f64,
        longitude: f64,
    }
    #[derive(Deserialize)]
    struct Results {
        #[serde(default)]
        results: Vec<Found>,
    }
    let response = super::google::CLIENT
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[("name", query.as_str()), ("count", "1"), ("language", language.as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let results: Results = response.json().await.map_err(|e| e.to_string())?;
    Ok(results.results.into_iter().next().map(|f| Place {
        name: f.name,
        country: f.country,
        latitude: f.latitude,
        longitude: f.longitude,
    }))
}

#[derive(Deserialize)]
struct Forecast {
    current: Current,
    daily: Daily,
}

#[derive(Deserialize)]
struct Current {
    temperature_2m: f32,
    weather_code: u8,
    is_day: u8,
}

#[derive(Deserialize)]
struct Daily {
    temperature_2m_max: Vec<f32>,
    temperature_2m_min: Vec<f32>,
}

fn parse(forecast: Forecast) -> Weather {
    Weather {
        temperature: forecast.current.temperature_2m,
        high: forecast.daily.temperature_2m_max.first().copied().unwrap_or(f32::NAN),
        low: forecast.daily.temperature_2m_min.first().copied().unwrap_or(f32::NAN),
        code: forecast.current.weather_code,
        day: forecast.current.is_day != 0,
    }
}

/// The weather at `place` now.
pub async fn current(place: Place) -> Result<Weather, String> {
    let latitude = place.latitude.to_string();
    let longitude = place.longitude.to_string();
    let response = super::google::CLIENT
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", latitude.as_str()),
            ("longitude", longitude.as_str()),
            ("current", "temperature_2m,weather_code,is_day"),
            ("daily", "temperature_2m_max,temperature_2m_min"),
            ("forecast_days", "1"),
            ("timezone", "auto"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    Ok(parse(response.json().await.map_err(|e| e.to_string())?))
}

/// Icon of the weather, from the icon theme.
pub fn icon(code: u8, day: bool) -> &'static str {
    match code {
        0 if day => "weather-clear-symbolic",
        0 => "weather-clear-night-symbolic",
        1 | 2 if day => "weather-few-clouds-symbolic",
        1 | 2 => "weather-few-clouds-night-symbolic",
        3 => "weather-overcast-symbolic",
        45 | 48 => "weather-fog-symbolic",
        51..=57 | 80..=82 => "weather-showers-scattered-symbolic",
        61..=67 => "weather-showers-symbolic",
        71..=77 | 85 | 86 => "weather-snow-symbolic",
        95..=99 => "weather-storm-symbolic",
        _ => "weather-overcast-symbolic",
    }
}

/// What kind of weather it is, as a key of the translations.
pub fn kind(code: u8) -> &'static str {
    match code {
        0 => "weather-clear",
        1 | 2 => "weather-partly-cloudy",
        3 => "weather-overcast",
        45 | 48 => "weather-fog",
        51..=57 => "weather-drizzle",
        61..=67 | 80..=82 => "weather-rain",
        71..=77 | 85 | 86 => "weather-snow",
        95..=99 => "weather-storm",
        _ => "weather-overcast",
    }
}

/// A temperature as people write it: "−3 °C".
pub fn format_temperature(celsius: f32) -> String {
    let rounded = celsius.round();
    // Rounding can give −0.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    if rounded < 0.0 {
        format!("−{} °C", -rounded)
    } else {
        format!("{rounded} °C")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_forecast() {
        let json = r#"{"current":{"time":"2026-10-24T22:00","interval":900,"temperature_2m":7.4,"weather_code":3,"is_day":0},
            "daily":{"time":["2026-10-24"],"temperature_2m_max":[12.1],"temperature_2m_min":[4.0]}}"#;
        let weather = parse(serde_json::from_str(json).unwrap());
        assert_eq!(weather.code, 3);
        assert!(!weather.day);
        assert_eq!(weather.high, 12.1);
        assert_eq!(icon(weather.code, weather.day), "weather-overcast-symbolic");
    }

    #[test]
    fn writes_temperatures() {
        assert_eq!(format_temperature(7.4), "7 °C");
        assert_eq!(format_temperature(-0.3), "0 °C");
        assert_eq!(format_temperature(-3.6), "−4 °C");
    }

    #[test]
    fn night_icons() {
        assert_eq!(icon(0, false), "weather-clear-night-symbolic");
        assert_eq!(icon(1, true), "weather-few-clouds-symbolic");
        assert_eq!(icon(63, true), "weather-showers-symbolic");
    }
}
