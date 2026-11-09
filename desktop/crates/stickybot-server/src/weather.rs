//! Weather for the stick's home screen, from Open-Meteo (free, no API key) — a port of
//! `weather.py`. Location: `"auto"` guesses it from this Mac's internet connection (ipinfo.io,
//! falling back to geojs.io), or a city name from the dashboard (Open-Meteo's geocoding).
//! Refreshed every 30 minutes (5 after a failure); only a few numbers go to the stick (`wx`).

use crate::net::Http;
use serde_json::{Map, Value, json};

pub const REFRESH_S: f64 = 30.0 * 60.0;
pub const RETRY_S: f64 = 5.0 * 60.0;

/// WMO weather code -> (icon the stick draws, one word).
pub fn describe(code: i64) -> (&'static str, &'static str) {
    match code {
        0 => ("clear", "clear"),
        1 | 2 => ("partly", "partly cloudy"),
        3 => ("cloudy", "cloudy"),
        45 | 48 => ("fog", "fog"),
        51..=57 => ("rain", "drizzle"),
        61..=67 | 80..=82 => ("rain", "rain"),
        71..=77 | 85 | 86 => ("snow", "snow"),
        c if c >= 95 => ("storm", "storm"),
        _ => ("cloudy", "cloudy"),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Default)]
pub struct Weather {
    /// what goes to the stick (`{"t","i","c","day","hi","lo","at"}`), or `None`
    pub now: Option<Value>,
    pub place: Option<Place>,
    next: f64,
    place_for: Option<String>,
}

/// Python's `round()` for display: half to even is close enough; the stick shows whole degrees.
fn round(v: &Value) -> Option<i64> {
    v.as_f64().map(|f| f.round_ties_even() as i64)
}

fn locate(http: &dyn Http, where_: &str) -> Result<Place, String> {
    if !where_.is_empty() && where_ != "auto" {
        let d = http.get_json(
            "https://geocoding-api.open-meteo.com/v1/search",
            &[("name", where_.to_string()), ("count", "1".into())],
        )?;
        let r = d.get("results").and_then(Value::as_array).and_then(|a| a.first()).ok_or(format!("no place called '{where_}'"))?;
        return Ok(Place {
            name: r["name"].as_str().unwrap_or("").to_string(),
            lat: r["latitude"].as_f64().ok_or("no latitude")?,
            lon: r["longitude"].as_f64().ok_or("no longitude")?,
        });
    }
    let ipinfo = || -> Result<Place, String> {
        let d = http.get_json("https://ipinfo.io/json", &[])?;
        let loc = d["loc"].as_str().ok_or("no loc")?;
        let (lat, lon) = loc.split_once(',').ok_or("bad loc")?;
        Ok(Place {
            name: d["city"].as_str().unwrap_or("").to_string(),
            lat: lat.trim().parse().map_err(|_| "bad loc")?,
            lon: lon.trim().parse().map_err(|_| "bad loc")?,
        })
    };
    ipinfo().or_else(|_| {
        let d = http.get_json("https://get.geojs.io/v1/ip/geo.json", &[])?;
        let num = |k: &str| d[k].as_str().and_then(|s| s.parse().ok()).or_else(|| d[k].as_f64()).ok_or(format!("no {k}"));
        Ok(Place { name: d["city"].as_str().unwrap_or("").to_string(), lat: num("latitude")?, lon: num("longitude")? })
    })
}

impl Weather {
    /// Due for a refresh (or forced)? Blocking (network): call it on a blocking thread.
    pub fn refresh(&mut self, http: &dyn Http, cfg: &Map<String, Value>, force: bool, now: f64) {
        if !cfg.get("weather").and_then(Value::as_bool).unwrap_or(true) {
            self.now = None;
            return;
        }
        if !force && now < self.next {
            return;
        }
        let where_ = cfg.get("weather_location").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("auto");
        match self.fetch(http, where_) {
            Ok(v) => {
                log::info!("weather {}: {}°, {}", v["at"].as_str().unwrap_or(""), v["t"], v["c"].as_str().unwrap_or(""));
                self.now = Some(v);
                self.next = now + REFRESH_S;
            }
            Err(e) => {
                log::warn!("weather: {e}");
                self.next = now + RETRY_S;
            }
        }
    }

    fn fetch(&mut self, http: &dyn Http, where_: &str) -> Result<Value, String> {
        if self.place.is_none() || self.place_for.as_deref() != Some(where_) {
            self.place = Some(locate(http, where_)?);
            self.place_for = Some(where_.to_string());
        }
        let p = self.place.clone().expect("located");
        let d = http.get_json(
            "https://api.open-meteo.com/v1/forecast",
            &[
                ("latitude", p.lat.to_string()),
                ("longitude", p.lon.to_string()),
                ("current", "temperature_2m,weather_code,is_day".into()),
                ("daily", "temperature_2m_max,temperature_2m_min".into()),
                ("forecast_days", "1".into()),
                ("timezone", "auto".into()),
            ],
        )?;
        let (cur, day) = (&d["current"], &d["daily"]);
        let code = cur["weather_code"].as_f64().ok_or("no weather_code")? as i64;
        let (icon, word) = describe(code);
        Ok(json!({
            "t": round(&cur["temperature_2m"]).ok_or("no temperature")?,
            "i": icon, "c": word,
            "day": cur.get("is_day").and_then(Value::as_f64).unwrap_or(1.0) as i64,
            "hi": round(&day["temperature_2m_max"][0]).ok_or("no max")?,
            "lo": round(&day["temperature_2m_min"][0]).ok_or("no min")?,
            "at": p.name,
        }))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::net::FakeHttp;

    pub fn forecast(http: &FakeHttp, temp: f64, code: i64) {
        http.set(
            "https://api.open-meteo.com/v1/forecast",
            json!({"current": {"temperature_2m": temp, "weather_code": code, "is_day": 1},
                   "daily": {"temperature_2m_max": [24.6], "temperature_2m_min": [11.2]}}),
        );
    }

    fn cfg(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn codes_like_python() {
        assert_eq!(describe(0), ("clear", "clear"));
        assert_eq!(describe(2), ("partly", "partly cloudy"));
        assert_eq!(describe(48), ("fog", "fog"));
        assert_eq!(describe(55), ("rain", "drizzle"));
        assert_eq!(describe(81), ("rain", "rain"));
        assert_eq!(describe(86), ("snow", "snow"));
        assert_eq!(describe(99), ("storm", "storm"));
        assert_eq!(describe(4), ("cloudy", "cloudy"));
    }

    #[test]
    fn auto_location_then_forecast_then_waits_30_min() {
        let http = FakeHttp::default();
        http.set("https://ipinfo.io/json", json!({"loc": "48.85,2.35", "city": "Paris"}));
        forecast(&http, 18.5, 3);
        let mut w = Weather::default();
        w.refresh(&http, &cfg(json!({})), false, 1000.0);
        assert_eq!(w.now, Some(json!({"t": 18, "i": "cloudy", "c": "cloudy", "day": 1, "hi": 25, "lo": 11, "at": "Paris"})));
        let asked = http.asked();
        assert!(asked[1].contains("latitude=48.85&longitude=2.35&current=temperature_2m,weather_code,is_day"), "{asked:?}");
        w.refresh(&http, &cfg(json!({})), false, 1000.0 + 60.0);
        assert_eq!(http.asked().len(), 2); // not due
        w.refresh(&http, &cfg(json!({})), false, 1000.0 + REFRESH_S);
        assert_eq!(http.asked().len(), 3); // the place is kept
    }

    #[test]
    fn a_city_name_geojs_fallback_errors_and_off() {
        let http = FakeHttp::default();
        http.set("https://geocoding-api.open-meteo.com/v1/search", json!({"results": [{"name": "Zürich", "latitude": 47.37, "longitude": 8.54}]}));
        forecast(&http, -0.4, 71);
        let mut w = Weather::default();
        w.refresh(&http, &cfg(json!({"weather_location": "Zurich"})), false, 0.0);
        assert_eq!(w.now.as_ref().unwrap()["at"], "Zürich");
        assert_eq!(w.now.as_ref().unwrap()["i"], "snow");

        // ipinfo down -> geojs (strings for the numbers)
        let http = FakeHttp::default();
        http.set("https://get.geojs.io/v1/ip/geo.json", json!({"latitude": "52.5", "longitude": "13.4", "city": "Berlin"}));
        forecast(&http, 9.0, 0);
        let mut w = Weather::default();
        w.refresh(&http, &cfg(json!({})), false, 0.0);
        assert_eq!(w.now.as_ref().unwrap()["at"], "Berlin");

        // offline: nothing, retried after 5 min
        let http = FakeHttp::default();
        let mut w = Weather::default();
        w.refresh(&http, &cfg(json!({})), false, 0.0);
        assert!(w.now.is_none());
        let n = http.asked().len();
        w.refresh(&http, &cfg(json!({})), false, RETRY_S - 1.0);
        assert_eq!(http.asked().len(), n);

        // turned off: no weather at all
        let mut w = Weather { now: Some(json!({})), ..Weather::default() };
        w.refresh(&http, &cfg(json!({"weather": false})), true, 0.0);
        assert!(w.now.is_none());
    }
}
