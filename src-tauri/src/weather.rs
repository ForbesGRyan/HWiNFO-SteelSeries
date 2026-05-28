//! Weather sensor reader. Fetches wttr.in JSON in a background thread,
//! parses it into a flat WeatherInfo, and serves field lookups via Arc<RwLock>.

/// Unit system selection. Controls which wttr.in field (temp_C vs temp_F, etc.) is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    Metric,
    #[default]
    Imperial,
}

impl Units {
    /// Map a `conf.ini` `units=` string to a `Units`. Unknown values fall back to Imperial.
    pub fn from_config_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "metric" => Units::Metric,
            "imperial" => Units::Imperial,
            _ => Units::default(),
        }
    }

    /// The `conf.ini` `units=` string for this variant (inverse of `from_config_str`).
    pub fn as_str(self) -> &'static str {
        match self {
            Units::Metric => "metric",
            Units::Imperial => "imperial",
        }
    }
}

/// Identifier for one weather field that can appear in `conf.ini` as a sensor name.
///
/// Current-conditions fields are unit-variants. Forecast fields carry a day index
/// 1..=3 where 1 = tomorrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherField {
    Temp,
    Feels,
    Hi,
    Lo,
    Condition,
    ConditionShort,
    Humidity,
    WindSpeed,
    WindDir,
    WindGust,
    PrecipChance,
    PrecipAmount,
    Uv,
    Pressure,
    Clouds,
    Visibility,
    Sunrise,
    Sunset,
    HiD(u8),
    LoD(u8),
    ConditionD(u8),
    ConditionShortD(u8),
    PrecipChanceD(u8),
}

impl WeatherField {
    /// Parse a sensor name like "WEATHER_TEMP" or "WEATHER_HI_D2" into a field.
    /// Returns None for anything that doesn't match (including out-of-range days).
    pub fn from_sensor_name(name: &str) -> Option<Self> {
        match name {
            "WEATHER_TEMP" => Some(Self::Temp),
            "WEATHER_FEELS" => Some(Self::Feels),
            "WEATHER_HI" => Some(Self::Hi),
            "WEATHER_LO" => Some(Self::Lo),
            "WEATHER_CONDITION" => Some(Self::Condition),
            "WEATHER_CONDITION_SHORT" => Some(Self::ConditionShort),
            "WEATHER_HUMIDITY" => Some(Self::Humidity),
            "WEATHER_WIND_SPEED" => Some(Self::WindSpeed),
            "WEATHER_WIND_DIR" => Some(Self::WindDir),
            "WEATHER_WIND_GUST" => Some(Self::WindGust),
            "WEATHER_PRECIP_CHANCE" => Some(Self::PrecipChance),
            "WEATHER_PRECIP_AMOUNT" => Some(Self::PrecipAmount),
            "WEATHER_UV" => Some(Self::Uv),
            "WEATHER_PRESSURE" => Some(Self::Pressure),
            "WEATHER_CLOUDS" => Some(Self::Clouds),
            "WEATHER_VISIBILITY" => Some(Self::Visibility),
            "WEATHER_SUNRISE" => Some(Self::Sunrise),
            "WEATHER_SUNSET" => Some(Self::Sunset),
            _ => parse_day_suffix(name),
        }
    }
}

fn parse_day_suffix(name: &str) -> Option<WeatherField> {
    // Expected forms: WEATHER_HI_D1 / WEATHER_LO_D1 / WEATHER_CONDITION_D1
    //                 WEATHER_CONDITION_SHORT_D1 / WEATHER_PRECIP_CHANCE_D1
    let (prefix, day_str) = name.rsplit_once("_D")?;
    let day: u8 = day_str.parse().ok()?;
    if !(1..=3).contains(&day) {
        return None;
    }
    match prefix {
        "WEATHER_HI" => Some(WeatherField::HiD(day)),
        "WEATHER_LO" => Some(WeatherField::LoD(day)),
        "WEATHER_CONDITION" => Some(WeatherField::ConditionD(day)),
        "WEATHER_CONDITION_SHORT" => Some(WeatherField::ConditionShortD(day)),
        "WEATHER_PRECIP_CHANCE" => Some(WeatherField::PrecipChanceD(day)),
        _ => None,
    }
}

use log::{debug, info, warn};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct WttrResponse {
    current_condition: Vec<WttrCurrent>,
    weather: Vec<WttrDay>,
}

#[derive(Debug, Deserialize)]
struct WttrCurrent {
    #[serde(rename = "FeelsLikeC")]
    feels_c: String,
    #[serde(rename = "FeelsLikeF")]
    feels_f: String,
    cloudcover: String,
    humidity: String,
    #[serde(rename = "precipMM")]
    precip_mm: String,
    pressure: String,
    #[serde(rename = "pressureInches")]
    pressure_inches: String,
    #[serde(rename = "temp_C")]
    temp_c: String,
    #[serde(rename = "temp_F")]
    temp_f: String,
    #[serde(rename = "uvIndex")]
    uv_index: String,
    visibility: String,
    #[serde(rename = "visibilityMiles")]
    visibility_miles: String,
    #[serde(rename = "weatherDesc")]
    weather_desc: Vec<WttrDesc>,
    #[serde(rename = "winddir16Point")]
    wind_dir_16: String,
    #[serde(rename = "windspeedKmph")]
    windspeed_kmph: String,
    #[serde(rename = "windspeedMiles")]
    windspeed_miles: String,
}

#[derive(Debug, Deserialize)]
struct WttrDay {
    astronomy: Vec<WttrAstronomy>,
    #[serde(rename = "maxtempC")]
    maxtemp_c: String,
    #[serde(rename = "maxtempF")]
    maxtemp_f: String,
    #[serde(rename = "mintempC")]
    mintemp_c: String,
    #[serde(rename = "mintempF")]
    mintemp_f: String,
    hourly: Vec<WttrHourly>,
}

#[derive(Debug, Deserialize)]
struct WttrAstronomy {
    sunrise: String,
    sunset: String,
}

#[derive(Debug, Deserialize)]
struct WttrHourly {
    #[serde(default)]
    chanceofrain: String,
    #[serde(default)]
    chanceofsnow: String,
    #[serde(rename = "WindGustKmph", default)]
    wind_gust_kmph: String,
    #[serde(rename = "WindGustMiles", default)]
    wind_gust_miles: String,
    #[serde(rename = "weatherDesc", default)]
    weather_desc: Vec<WttrDesc>,
}

impl WttrHourly {
    /// Combined precipitation chance: max(chanceofrain, chanceofsnow).
    /// Either field may be empty/missing; missing values count as 0.
    fn precip_chance(&self) -> Option<f64> {
        let rain = self.chanceofrain.parse::<f64>().ok();
        let snow = self.chanceofsnow.parse::<f64>().ok();
        match (rain, snow) {
            (Some(r), Some(s)) => Some(r.max(s)),
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WttrDesc {
    value: String,
}

/// Parse a wttr.in `format=j1` response into a `WeatherInfo`, applying unit selection.
/// Forecast days are populated by Task 7; this task only covers current-conditions fields.
pub fn parse(json: &str, units: Units) -> Result<WeatherInfo, anyhow::Error> {
    let raw: WttrResponse = serde_json::from_str(json)
        .map_err(|e| anyhow::anyhow!("failed to parse wttr.in response: {}", e))?;

    let current = raw
        .current_condition
        .first()
        .ok_or_else(|| anyhow::anyhow!("wttr.in response missing current_condition"))?;
    let today = raw
        .weather
        .first()
        .ok_or_else(|| anyhow::anyhow!("wttr.in response missing weather[0]"))?;
    let astronomy = today.astronomy.first();

    let mut info = WeatherInfo::default();

    info.temp = Some(pick(units, &current.temp_c, &current.temp_f).clone());
    info.feels = Some(pick(units, &current.feels_c, &current.feels_f).clone());
    info.hi = Some(pick(units, &today.maxtemp_c, &today.maxtemp_f).clone());
    info.lo = Some(pick(units, &today.mintemp_c, &today.mintemp_f).clone());

    let condition = current.weather_desc.first().map(|d| d.value.clone());
    info.condition_short = condition.as_deref().map(abbreviate_condition);
    info.condition = condition;

    info.humidity = Some(current.humidity.clone());
    info.wind_speed = Some(pick(units, &current.windspeed_kmph, &current.windspeed_miles).clone());
    info.wind_dir = Some(current.wind_dir_16.clone());
    info.wind_gust = today
        .hourly
        .iter()
        .map(|h| pick(units, &h.wind_gust_kmph, &h.wind_gust_miles))
        .filter_map(|s| s.parse::<f64>().ok())
        .fold(None::<f64>, |acc, v| Some(acc.map_or(v, |a| a.max(v))))
        .map(|v| format!("{:.0}", v));
    info.precip_chance = today
        .hourly
        .iter()
        .filter_map(|h| h.precip_chance())
        .fold(None::<f64>, |acc, v| Some(acc.map_or(v, |a| a.max(v))))
        .map(|v| format!("{:.0}", v));
    info.precip_amount = current.precip_mm.parse::<f64>().ok().map(|mm| match units {
        Units::Metric => format!("{:.1}", mm),
        Units::Imperial => format!("{:.1}", mm / 25.4),
    });
    info.uv = Some(current.uv_index.clone());
    info.pressure = Some(pick(units, &current.pressure, &current.pressure_inches).clone());
    info.clouds = Some(current.cloudcover.clone());
    info.visibility = Some(pick(units, &current.visibility, &current.visibility_miles).clone());
    info.sunrise = astronomy.map(|a| a.sunrise.clone());
    info.sunset = astronomy.map(|a| a.sunset.clone());

    // Forecast days: D1 = weather[1], D2 = weather[2], D3 = weather[3]
    // (weather[0] is today, already used for info.hi/info.lo etc).
    for (slot, day_index) in (1..=3usize).enumerate() {
        if let Some(day) = raw.weather.get(day_index) {
            let condition = day
                .hourly
                .get(4)
                .and_then(|h| h.weather_desc.first())
                .map(|d| d.value.clone());
            let precip_chance = day
                .hourly
                .iter()
                .filter_map(|h| h.precip_chance())
                .fold(None::<f64>, |acc, v| Some(acc.map_or(v, |a| a.max(v))))
                .map(|v| format!("{:.0}", v));

            info.days[slot] = Some(DayForecast {
                hi: Some(pick(units, &day.maxtemp_c, &day.maxtemp_f).clone()),
                lo: Some(pick(units, &day.mintemp_c, &day.mintemp_f).clone()),
                condition_short: condition.as_deref().map(abbreviate_condition),
                condition,
                precip_chance,
            });
        }
    }

    Ok(info)
}

fn pick<'a>(units: Units, metric: &'a String, imperial: &'a String) -> &'a String {
    match units {
        Units::Metric => metric,
        Units::Imperial => imperial,
    }
}

/// One day's forecast slice. All fields are `Option<String>` so missing data
/// hides the sensor rather than rendering a placeholder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayForecast {
    pub hi: Option<String>,
    pub lo: Option<String>,
    pub condition: Option<String>,
    pub condition_short: Option<String>,
    pub precip_chance: Option<String>,
}

/// Parsed snapshot of the current weather plus a 3-day forecast.
/// Stored values are already unit-converted strings ready for the OLED.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeatherInfo {
    pub temp: Option<String>,
    pub feels: Option<String>,
    pub hi: Option<String>,
    pub lo: Option<String>,
    pub condition: Option<String>,
    pub condition_short: Option<String>,
    pub humidity: Option<String>,
    pub wind_speed: Option<String>,
    pub wind_dir: Option<String>,
    pub wind_gust: Option<String>,
    pub precip_chance: Option<String>,
    pub precip_amount: Option<String>,
    pub uv: Option<String>,
    pub pressure: Option<String>,
    pub clouds: Option<String>,
    pub visibility: Option<String>,
    pub sunrise: Option<String>,
    pub sunset: Option<String>,
    /// Indexed 0..3 representing D1, D2, D3 (tomorrow, day-after, day-after-that).
    pub days: [Option<DayForecast>; 3],
}

impl WeatherInfo {
    /// Look up a single field for the OLED. Returns `None` if the field is unset
    /// or the requested forecast day is missing.
    pub fn get(&self, field: WeatherField) -> Option<String> {
        match field {
            WeatherField::Temp => self.temp.clone(),
            WeatherField::Feels => self.feels.clone(),
            WeatherField::Hi => self.hi.clone(),
            WeatherField::Lo => self.lo.clone(),
            WeatherField::Condition => self.condition.clone(),
            WeatherField::ConditionShort => self.condition_short.clone(),
            WeatherField::Humidity => self.humidity.clone(),
            WeatherField::WindSpeed => self.wind_speed.clone(),
            WeatherField::WindDir => self.wind_dir.clone(),
            WeatherField::WindGust => self.wind_gust.clone(),
            WeatherField::PrecipChance => self.precip_chance.clone(),
            WeatherField::PrecipAmount => self.precip_amount.clone(),
            WeatherField::Uv => self.uv.clone(),
            WeatherField::Pressure => self.pressure.clone(),
            WeatherField::Clouds => self.clouds.clone(),
            WeatherField::Visibility => self.visibility.clone(),
            WeatherField::Sunrise => self.sunrise.clone(),
            WeatherField::Sunset => self.sunset.clone(),
            WeatherField::HiD(d) => self.day(d).and_then(|f| f.hi.clone()),
            WeatherField::LoD(d) => self.day(d).and_then(|f| f.lo.clone()),
            WeatherField::ConditionD(d) => self.day(d).and_then(|f| f.condition.clone()),
            WeatherField::ConditionShortD(d) => self.day(d).and_then(|f| f.condition_short.clone()),
            WeatherField::PrecipChanceD(d) => self.day(d).and_then(|f| f.precip_chance.clone()),
        }
    }

    fn day(&self, d: u8) -> Option<&DayForecast> {
        if !(1..=3).contains(&d) {
            return None;
        }
        self.days[(d - 1) as usize].as_ref()
    }
}

/// Map a wttr.in condition string to an ≤8-char abbreviation for the OLED.
/// Matching is case-insensitive and uses substring contains for "rain"/"snow"/"thunder"
/// families so wttr.in's variants ("Patchy rain nearby", "Thundery outbreaks possible") map cleanly.
pub fn abbreviate_condition(condition: &str) -> String {
    let lower = condition.to_lowercase();
    let mapped: &str = if lower == "sunny" {
        "Sunny"
    } else if lower == "clear" {
        "Clear"
    } else if lower == "partly cloudy" {
        "P.Cloudy"
    } else if lower == "cloudy" {
        "Cloudy"
    } else if lower == "overcast" {
        "Overcast"
    } else if lower == "mist" {
        "Mist"
    } else if lower == "fog" {
        "Fog"
    } else if lower.contains("thunder") {
        "T.Storm"
    } else if lower.contains("heavy") && lower.contains("rain") {
        "H.Rain"
    } else if lower.contains("moderate") && lower.contains("rain") {
        "M.Rain"
    } else if lower.contains("rain") {
        "L.Rain"
    } else if lower.contains("heavy") && lower.contains("snow") {
        "H.Snow"
    } else if lower.contains("moderate") && lower.contains("snow") {
        "M.Snow"
    } else if lower.contains("snow") {
        "L.Snow"
    } else {
        // Unknown — truncate to 8 chars at a char boundary.
        return condition.chars().take(8).collect();
    };
    mapped.to_string()
}

/// Reads weather data for `run_sensors`. Backed by a shared cache that the
/// refresh thread writes; this reader only reads.
pub struct WeatherReader {
    shared: Arc<RwLock<Option<WeatherInfo>>>,
    stop: Arc<AtomicBool>,
}

impl WeatherReader {
    /// Construct a reader with no data and no refresh thread. All field lookups return `None`.
    /// Used when `[Weather]` is not configured.
    pub fn disabled() -> Self {
        Self {
            shared: Arc::new(RwLock::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Test-only constructor with a pre-populated cache.
    pub fn with_cached_info(info: WeatherInfo) -> Self {
        Self {
            shared: Arc::new(RwLock::new(Some(info))),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Construct a reader for the given config. If `cfg.enabled` is false, returns a disabled
    /// reader. Otherwise spawns a background thread that refreshes `WeatherInfo` on `cfg.refresh_minutes`.
    pub fn spawn(cfg: &crate::settings::WeatherConfig) -> Self {
        if !cfg.enabled {
            info!("Weather disabled (no [Weather].location in config)");
            return Self::disabled();
        }
        let shared = Arc::new(RwLock::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let url = build_request_url(&cfg.location);
        let units = cfg.units;
        let refresh = Duration::from_secs(cfg.refresh_minutes.saturating_mul(60));
        let shared_clone = Arc::clone(&shared);
        let stop_clone = Arc::clone(&stop);

        thread::Builder::new()
            .name("weather-refresh".to_string())
            .spawn(move || refresh_loop(shared_clone, stop_clone, url, units, refresh))
            .expect("spawn weather refresh thread");

        Self { shared, stop }
    }

    /// Signal the refresh thread (if any) to exit. Used before replacing the reader
    /// on config reload so the old thread does not keep polling wttr.in.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Look up a field from the cached `WeatherInfo`. Returns `None` if no data
    /// has been fetched yet, or the field is unset.
    pub fn get_field(&self, field: WeatherField) -> Option<String> {
        self.shared.read().ok()?.as_ref()?.get(field)
    }
}

/// Build the wttr.in request URL. Spaces in the location are percent-encoded.
pub(crate) fn build_request_url(location: &str) -> String {
    let encoded = location.replace(' ', "%20");
    format!("https://wttr.in/{}?format=j1", encoded)
}

fn refresh_loop(
    shared: Arc<RwLock<Option<WeatherInfo>>>,
    stop: Arc<AtomicBool>,
    url: String,
    units: Units,
    refresh: Duration,
) {
    while !stop.load(Ordering::Relaxed) {
        match fetch_and_parse(&url, units) {
            Ok(info) => {
                if let Ok(mut guard) = shared.write() {
                    *guard = Some(info);
                    debug!("Weather refreshed from {}", url);
                }
            }
            Err(e) => {
                let snippet = format!("{}", e);
                let trunc: String = snippet.chars().take(200).collect();
                warn!("Weather refresh failed: {} (keeping prior data)", trunc);
            }
        }
        // Sleep in short ticks so a stop signal is honored promptly instead of
        // blocking for the full refresh interval.
        let mut remaining = refresh;
        let tick = Duration::from_secs(1);
        while remaining > Duration::ZERO && !stop.load(Ordering::Relaxed) {
            let step = remaining.min(tick);
            thread::sleep(step);
            remaining -= step;
        }
    }
}

fn fetch_and_parse(url: &str, units: Units) -> Result<WeatherInfo, anyhow::Error> {
    let body = ureq::get(url)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| anyhow::anyhow!("HTTP error: {}", e))?
        .into_string()
        .map_err(|e| anyhow::anyhow!("read body: {}", e))?;
    parse(&body, units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_sensor_name_parses_each_current_field() {
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_TEMP"),
            Some(WeatherField::Temp)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_FEELS"),
            Some(WeatherField::Feels)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_HI"),
            Some(WeatherField::Hi)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_LO"),
            Some(WeatherField::Lo)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_CONDITION"),
            Some(WeatherField::Condition)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_CONDITION_SHORT"),
            Some(WeatherField::ConditionShort)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_HUMIDITY"),
            Some(WeatherField::Humidity)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_WIND_SPEED"),
            Some(WeatherField::WindSpeed)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_WIND_DIR"),
            Some(WeatherField::WindDir)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_WIND_GUST"),
            Some(WeatherField::WindGust)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_PRECIP_CHANCE"),
            Some(WeatherField::PrecipChance)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_PRECIP_AMOUNT"),
            Some(WeatherField::PrecipAmount)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_UV"),
            Some(WeatherField::Uv)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_PRESSURE"),
            Some(WeatherField::Pressure)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_CLOUDS"),
            Some(WeatherField::Clouds)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_VISIBILITY"),
            Some(WeatherField::Visibility)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_SUNRISE"),
            Some(WeatherField::Sunrise)
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_SUNSET"),
            Some(WeatherField::Sunset)
        );
    }

    #[test]
    fn from_sensor_name_parses_day_variants() {
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_HI_D1"),
            Some(WeatherField::HiD(1))
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_LO_D2"),
            Some(WeatherField::LoD(2))
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_CONDITION_D3"),
            Some(WeatherField::ConditionD(3))
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_CONDITION_SHORT_D1"),
            Some(WeatherField::ConditionShortD(1))
        );
        assert_eq!(
            WeatherField::from_sensor_name("WEATHER_PRECIP_CHANCE_D2"),
            Some(WeatherField::PrecipChanceD(2))
        );
    }

    #[test]
    fn from_sensor_name_rejects_invalid() {
        assert_eq!(WeatherField::from_sensor_name("CLOCK"), None);
        assert_eq!(WeatherField::from_sensor_name("WEATHER"), None);
        assert_eq!(WeatherField::from_sensor_name("WEATHER_FOO"), None);
        // Day out of range:
        assert_eq!(WeatherField::from_sensor_name("WEATHER_HI_D0"), None);
        assert_eq!(WeatherField::from_sensor_name("WEATHER_HI_D9"), None);
        // Non-numeric day:
        assert_eq!(WeatherField::from_sensor_name("WEATHER_HI_DX"), None);
        // Wrong prefix:
        assert_eq!(WeatherField::from_sensor_name("WEATHER_TEMP_D1"), None);
    }

    fn sample_info() -> WeatherInfo {
        let mut info = WeatherInfo::default();
        info.temp = Some("72".into());
        info.feels = Some("74".into());
        info.hi = Some("78".into());
        info.lo = Some("61".into());
        info.condition = Some("Partly cloudy".into());
        info.condition_short = Some("P.Cloudy".into());
        info.humidity = Some("54".into());
        info.wind_speed = Some("8".into());
        info.wind_dir = Some("NW".into());
        info.wind_gust = Some("15".into());
        info.precip_chance = Some("30".into());
        info.precip_amount = Some("0.2".into());
        info.uv = Some("6".into());
        info.pressure = Some("1013".into());
        info.clouds = Some("25".into());
        info.visibility = Some("10".into());
        info.sunrise = Some("06:42 AM".into());
        info.sunset = Some("08:14 PM".into());
        info.days[0] = Some(DayForecast {
            hi: Some("75".into()),
            lo: Some("60".into()),
            condition: Some("Sunny".into()),
            condition_short: Some("Sunny".into()),
            precip_chance: Some("10".into()),
        });
        info
    }

    #[test]
    fn get_returns_current_field_when_present() {
        let info = sample_info();
        assert_eq!(info.get(WeatherField::Temp), Some("72".into()));
        assert_eq!(
            info.get(WeatherField::Condition),
            Some("Partly cloudy".into())
        );
        assert_eq!(info.get(WeatherField::Sunrise), Some("06:42 AM".into()));
    }

    #[test]
    fn get_returns_none_for_unset_field() {
        let info = WeatherInfo::default();
        assert_eq!(info.get(WeatherField::Temp), None);
        assert_eq!(info.get(WeatherField::HiD(1)), None);
    }

    #[test]
    fn get_returns_forecast_field_when_day_present() {
        let info = sample_info();
        assert_eq!(info.get(WeatherField::HiD(1)), Some("75".into()));
        assert_eq!(info.get(WeatherField::ConditionD(1)), Some("Sunny".into()));
        assert_eq!(info.get(WeatherField::PrecipChanceD(1)), Some("10".into()));
    }

    #[test]
    fn get_returns_none_for_missing_day() {
        let info = sample_info(); // only day 0 populated
        assert_eq!(info.get(WeatherField::HiD(2)), None);
        assert_eq!(info.get(WeatherField::HiD(3)), None);
    }

    #[test]
    fn get_returns_none_for_out_of_range_day_index() {
        let info = sample_info();
        assert_eq!(info.get(WeatherField::HiD(0)), None);
        assert_eq!(info.get(WeatherField::HiD(4)), None);
    }

    #[test]
    fn abbreviate_condition_known_terms() {
        assert_eq!(abbreviate_condition("Sunny"), "Sunny");
        assert_eq!(abbreviate_condition("Clear"), "Clear");
        assert_eq!(abbreviate_condition("Partly cloudy"), "P.Cloudy");
        assert_eq!(abbreviate_condition("Cloudy"), "Cloudy");
        assert_eq!(abbreviate_condition("Overcast"), "Overcast");
        assert_eq!(abbreviate_condition("Mist"), "Mist");
        assert_eq!(abbreviate_condition("Fog"), "Fog");
        assert_eq!(abbreviate_condition("Light rain"), "L.Rain");
        assert_eq!(abbreviate_condition("Patchy rain nearby"), "L.Rain");
        assert_eq!(abbreviate_condition("Moderate rain"), "M.Rain");
        assert_eq!(abbreviate_condition("Heavy rain"), "H.Rain");
        assert_eq!(abbreviate_condition("Light snow"), "L.Snow");
        assert_eq!(abbreviate_condition("Moderate snow"), "M.Snow");
        assert_eq!(abbreviate_condition("Heavy snow"), "H.Snow");
        assert_eq!(abbreviate_condition("Thunderstorm"), "T.Storm");
        assert_eq!(
            abbreviate_condition("Thundery outbreaks possible"),
            "T.Storm"
        );
    }

    #[test]
    fn abbreviate_condition_unknown_truncates_to_8() {
        assert_eq!(abbreviate_condition("Hail"), "Hail");
        assert_eq!(abbreviate_condition("Freezing drizzle"), "Freezing");
        // Exactly 8 chars passes through:
        assert_eq!(abbreviate_condition("AbcdEfgh"), "AbcdEfgh");
    }

    #[test]
    fn abbreviate_condition_empty_returns_empty() {
        assert_eq!(abbreviate_condition(""), "");
    }

    #[test]
    fn units_enum_from_str_imperial_default() {
        assert_eq!(Units::from_config_str("metric"), Units::Metric);
        assert_eq!(Units::from_config_str("imperial"), Units::Imperial);
        assert_eq!(Units::from_config_str("METRIC"), Units::Metric);
        assert_eq!(Units::from_config_str(""), Units::Imperial); // default
        assert_eq!(Units::from_config_str("bogus"), Units::Imperial); // default
    }

    fn load_fixture() -> String {
        std::fs::read_to_string("tests/fixtures/wttr_sample.json")
            .expect("fixture file missing — run cargo test from src-tauri/ directory")
    }

    #[test]
    fn parse_imperial_picks_fahrenheit_and_miles() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        assert_eq!(info.temp, Some("68".into()));
        assert_eq!(info.feels, Some("72".into()));
        assert_eq!(info.hi, Some("78".into()));
        assert_eq!(info.lo, Some("61".into()));
        assert_eq!(info.wind_speed, Some("8".into()));
        assert_eq!(info.wind_gust, Some("15".into())); // max of day 0 hourly[*].WindGustMiles
        assert_eq!(info.visibility, Some("10".into()));
        assert_eq!(info.pressure, Some("30".into()));
    }

    #[test]
    fn parse_metric_picks_celsius_and_kmph() {
        let json = load_fixture();
        let info = parse(&json, Units::Metric).unwrap();
        assert_eq!(info.temp, Some("20".into()));
        assert_eq!(info.feels, Some("22".into()));
        assert_eq!(info.hi, Some("26".into()));
        assert_eq!(info.lo, Some("16".into()));
        assert_eq!(info.wind_speed, Some("13".into()));
        assert_eq!(info.wind_gust, Some("24".into()));
        assert_eq!(info.visibility, Some("16".into()));
        assert_eq!(info.pressure, Some("1013".into()));
    }

    #[test]
    fn parse_extracts_unit_agnostic_fields() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        assert_eq!(info.humidity, Some("54".into()));
        assert_eq!(info.wind_dir, Some("NW".into()));
        assert_eq!(info.uv, Some("6".into()));
        assert_eq!(info.clouds, Some("25".into()));
        assert_eq!(info.condition, Some("Partly cloudy".into()));
        assert_eq!(info.condition_short, Some("P.Cloudy".into()));
        assert_eq!(info.sunrise, Some("06:42 AM".into()));
        assert_eq!(info.sunset, Some("08:14 PM".into()));
        // Max chanceofrain across day 0 hourly slots:
        assert_eq!(info.precip_chance, Some("30".into()));
    }

    #[test]
    fn precip_chance_takes_max_of_rain_and_snow() {
        // Inject a snow chance into one hourly slot; precip_chance should pick it up.
        let mut value: serde_json::Value = serde_json::from_str(&load_fixture()).unwrap();
        value["weather"][0]["hourly"][3]["chanceofsnow"] = serde_json::Value::String("75".into());
        let json = serde_json::to_string(&value).unwrap();
        let info = parse(&json, Units::Imperial).unwrap();
        // Snow 75 in slot 3 beats max rain 30 → result is 75.
        assert_eq!(info.precip_chance, Some("75".into()));
    }

    #[test]
    fn precip_chance_falls_back_to_rain_when_snow_missing() {
        // No snow field anywhere → behaves as before, rain max only.
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        assert_eq!(info.precip_chance, Some("30".into()));
    }

    #[test]
    fn parse_precip_amount_converts_to_inches_when_imperial() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        // 0.5 mm / 25.4 = 0.0196... → rounded to 1 decimal = "0.0"
        assert_eq!(info.precip_amount, Some("0.0".into()));
    }

    #[test]
    fn parse_precip_amount_kept_as_mm_when_metric() {
        let json = load_fixture();
        let info = parse(&json, Units::Metric).unwrap();
        assert_eq!(info.precip_amount, Some("0.5".into()));
    }

    #[test]
    fn parse_returns_error_for_garbage_json() {
        let err = parse("not json at all", Units::Imperial).unwrap_err();
        assert!(format!("{}", err).contains("parse"));
    }

    #[test]
    fn parse_populates_three_forecast_days_imperial() {
        // D1 = weather[1] (tomorrow), D2 = weather[2], D3 = weather[3].
        // weather[0] is today and feeds info.hi/info.lo, not days[].
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        let d1 = info.days[0].as_ref().expect("D1 missing");
        let d2 = info.days[1].as_ref().expect("D2 missing");
        let d3 = info.days[2].as_ref().expect("D3 missing");
        assert_eq!(d1.hi, Some("75".into())); // weather[1].maxtempF
        assert_eq!(d2.hi, Some("73".into())); // weather[2].maxtempF
        assert_eq!(d3.hi, Some("70".into())); // weather[3].maxtempF
        assert_eq!(d1.lo, Some("60".into()));
        assert_eq!(d3.lo, Some("55".into()));
    }

    #[test]
    fn parse_populates_forecast_condition_and_abbrev() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        let d1 = info.days[0].as_ref().unwrap();
        let d2 = info.days[1].as_ref().unwrap();
        let d3 = info.days[2].as_ref().unwrap();
        // Pulled from weather[1..=3].hourly[4].weatherDesc[0].value
        assert_eq!(d1.condition, Some("Sunny".into()));
        assert_eq!(d1.condition_short, Some("Sunny".into()));
        assert_eq!(d2.condition, Some("Heavy rain".into()));
        assert_eq!(d2.condition_short, Some("H.Rain".into()));
        assert_eq!(d3.condition, Some("Cloudy".into()));
        assert_eq!(d3.condition_short, Some("Cloudy".into()));
    }

    #[test]
    fn parse_populates_forecast_precip_chance_max() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        let d1 = info.days[0].as_ref().unwrap();
        let d2 = info.days[1].as_ref().unwrap();
        let d3 = info.days[2].as_ref().unwrap();
        assert_eq!(d1.precip_chance, Some("10".into())); // weather[1] hourly max
        assert_eq!(d2.precip_chance, Some("80".into())); // weather[2] hourly max
        assert_eq!(d3.precip_chance, Some("25".into())); // weather[3] hourly max
    }

    #[test]
    fn parse_handles_missing_forecast_days() {
        // Truncate to weather[0] only (a "today-only" response). No D1/D2/D3 data available.
        let mut value: serde_json::Value = serde_json::from_str(&load_fixture()).unwrap();
        let arr = value["weather"].as_array_mut().unwrap();
        arr.truncate(1);
        let trimmed = serde_json::to_string(&value).unwrap();

        let info = parse(&trimmed, Units::Imperial).unwrap();
        assert!(info.days[0].is_none());
        assert!(info.days[1].is_none());
        assert!(info.days[2].is_none());
    }

    #[test]
    fn reader_with_cached_info_returns_field() {
        let info = sample_info();
        let reader = WeatherReader::with_cached_info(info);
        assert_eq!(reader.get_field(WeatherField::Temp), Some("72".into()));
        assert_eq!(
            reader.get_field(WeatherField::ConditionShort),
            Some("P.Cloudy".into())
        );
    }

    #[test]
    fn reader_new_disabled_returns_none() {
        let reader = WeatherReader::disabled();
        assert_eq!(reader.get_field(WeatherField::Temp), None);
        assert_eq!(reader.get_field(WeatherField::HiD(1)), None);
    }

    #[test]
    fn reader_spawn_disabled_when_location_empty() {
        let cfg = crate::settings::WeatherConfig::default();
        let reader = WeatherReader::spawn(&cfg);
        // No location → no thread started → no data ever appears.
        assert_eq!(reader.get_field(WeatherField::Temp), None);
    }

    #[test]
    fn build_url_includes_location_and_format() {
        let url = build_request_url("Seattle,US");
        assert!(url.starts_with("https://wttr.in/"));
        assert!(url.contains("Seattle,US"));
        assert!(url.ends_with("?format=j1"));
    }

    #[test]
    fn build_url_percent_encodes_spaces() {
        let url = build_request_url("New York,US");
        assert!(url.contains("New%20York,US") || url.contains("New+York,US"));
    }
}
