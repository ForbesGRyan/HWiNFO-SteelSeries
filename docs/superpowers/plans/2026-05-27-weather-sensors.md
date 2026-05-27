# Weather Sensors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `WEATHER_*` and `WEATHER_*_D{1,2,3}` special sensors that display live weather data from wttr.in on SteelSeries OLED, alongside HWiNFO sensors, CLOCK, DATE, MEDIA_*, and MOUSE_BATTERY.

**Architecture:** New `weather.rs` module owns a `WeatherReader` whose background refresh thread fetches wttr.in JSON every N minutes, parses it (applying metric/imperial unit selection) into a flat `WeatherInfo`, and stores it behind `Arc<RwLock<Option<WeatherInfo>>>`. `run_sensors` matches the `WEATHER_*` sensor prefix exactly like it matches `MEDIA_*`, calling `WeatherReader::get_field` to look up the cached string. Network never blocks the 1 s main tick; missing fields hide their sensor slot.

**Tech Stack:** Rust 2021, `ureq` for sync HTTP, `serde` + `serde_json` for parsing, `std::thread` + `Arc<RwLock>` for the refresh loop. No new async runtime.

**Spec:** `docs/superpowers/specs/2026-05-27-weather-sensors-design.md` (read before starting).

---

## File Map

| Path                                                | Status   | Responsibility                                                                                  |
|-----------------------------------------------------|----------|-------------------------------------------------------------------------------------------------|
| `src-tauri/Cargo.toml`                              | modify   | Add `ureq` dependency.                                                                          |
| `src-tauri/src/weather.rs`                          | **new**  | `WeatherReader`, `WeatherInfo`, `WeatherField`, parse, abbreviate, refresh thread, unit tests.  |
| `src-tauri/tests/fixtures/wttr_sample.json`         | **new**  | Trimmed real wttr.in `format=j1` response used as a parsing fixture.                            |
| `src-tauri/src/main.rs`                             | modify   | `mod weather;` declaration only.                                                                |
| `src-tauri/src/settings.rs`                         | modify   | Parse `[Weather]` section into `WeatherConfig`; attach to `AppConfig`.                          |
| `src-tauri/src/daemon.rs`                           | modify   | Hold `WeatherReader` on `Daemon`; pass to `run_sensors` via `build_display_value`.              |
| `src-tauri/src/gui.rs`                              | modify   | Pass `WeatherReader` into the direct `run_sensors` call site (around line 305).                 |
| `src-tauri/src/utils.rs`                            | modify   | Add `WEATHER_*` branch in `run_sensors`; add weather-related tests.                             |
| `README.md`                                         | modify   | Document `WEATHER_*` sensors and `[Weather]` config section.                                    |

---

## Task 1: Add `ureq` dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the dependency line**

Add this line under `[dependencies]` (alphabetical order, just before `windows`):

```toml
ureq = { version = "2", features = ["json"] }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: build succeeds with no errors (warnings OK).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "deps: add ureq for weather HTTP client"
```

---

## Task 2: Create `weather.rs` with `WeatherField` enum + `from_sensor_name`

**Files:**
- Create: `src-tauri/src/weather.rs`
- Modify: `src-tauri/src/main.rs` (add `mod weather;`)

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/weather.rs` with:

```rust
//! Weather sensor reader. Fetches wttr.in JSON in a background thread,
//! parses it into a flat WeatherInfo, and serves field lookups via Arc<RwLock>.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_sensor_name_parses_each_current_field() {
        assert_eq!(WeatherField::from_sensor_name("WEATHER_TEMP"), Some(WeatherField::Temp));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_FEELS"), Some(WeatherField::Feels));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_HI"), Some(WeatherField::Hi));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_LO"), Some(WeatherField::Lo));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_CONDITION"), Some(WeatherField::Condition));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_CONDITION_SHORT"), Some(WeatherField::ConditionShort));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_HUMIDITY"), Some(WeatherField::Humidity));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_WIND_SPEED"), Some(WeatherField::WindSpeed));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_WIND_DIR"), Some(WeatherField::WindDir));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_WIND_GUST"), Some(WeatherField::WindGust));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_PRECIP_CHANCE"), Some(WeatherField::PrecipChance));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_PRECIP_AMOUNT"), Some(WeatherField::PrecipAmount));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_UV"), Some(WeatherField::Uv));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_PRESSURE"), Some(WeatherField::Pressure));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_CLOUDS"), Some(WeatherField::Clouds));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_VISIBILITY"), Some(WeatherField::Visibility));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_SUNRISE"), Some(WeatherField::Sunrise));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_SUNSET"), Some(WeatherField::Sunset));
    }

    #[test]
    fn from_sensor_name_parses_day_variants() {
        assert_eq!(WeatherField::from_sensor_name("WEATHER_HI_D1"), Some(WeatherField::HiD(1)));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_LO_D2"), Some(WeatherField::LoD(2)));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_CONDITION_D3"), Some(WeatherField::ConditionD(3)));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_CONDITION_SHORT_D1"), Some(WeatherField::ConditionShortD(1)));
        assert_eq!(WeatherField::from_sensor_name("WEATHER_PRECIP_CHANCE_D2"), Some(WeatherField::PrecipChanceD(2)));
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
}
```

Add to `src-tauri/src/main.rs` near the other `mod` lines (after line 23 `mod media;`):

```rust
mod weather;
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests`
Expected: 3 tests pass (`from_sensor_name_parses_each_current_field`, `from_sensor_name_parses_day_variants`, `from_sensor_name_rejects_invalid`).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/weather.rs src-tauri/src/main.rs
git commit -m "feat(weather): add WeatherField enum and sensor-name parser"
```

---

## Task 3: Add `WeatherInfo` data shape + `WeatherInfo::get` lookup

**Files:**
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Write the failing tests**

Append below the existing tests module body (inside `mod tests`):

```rust
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
        assert_eq!(info.get(WeatherField::Condition), Some("Partly cloudy".into()));
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests`
Expected: compile error — `WeatherInfo`, `DayForecast`, `info.get` undefined.

- [ ] **Step 3: Add the types and the lookup method**

Insert into `src-tauri/src/weather.rs` above the existing `#[cfg(test)] mod tests` block:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests`
Expected: 8 tests pass (3 existing + 5 new).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/weather.rs
git commit -m "feat(weather): add WeatherInfo data shape and field lookup"
```

---

## Task 4: Condition abbreviation table

**Files:**
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Write the failing tests**

Append to `mod tests`:

```rust
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
        assert_eq!(abbreviate_condition("Thundery outbreaks possible"), "T.Storm");
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::abbreviate`
Expected: compile error — `abbreviate_condition` undefined.

- [ ] **Step 3: Add the function**

Append to `src-tauri/src/weather.rs` (above the test module):

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::abbreviate`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/weather.rs
git commit -m "feat(weather): add condition abbreviation table"
```

---

## Task 5: Add wttr.in fixture and `Units` enum

**Files:**
- Create: `src-tauri/tests/fixtures/wttr_sample.json`
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Create the fixture file**

Create `src-tauri/tests/fixtures/wttr_sample.json`:

```json
{
  "current_condition": [
    {
      "FeelsLikeC": "22",
      "FeelsLikeF": "72",
      "cloudcover": "25",
      "humidity": "54",
      "precipMM": "0.5",
      "pressure": "1013",
      "pressureInches": "30",
      "temp_C": "20",
      "temp_F": "68",
      "uvIndex": "6",
      "visibility": "16",
      "visibilityMiles": "10",
      "weatherDesc": [{"value": "Partly cloudy"}],
      "winddir16Point": "NW",
      "winddirDegree": "300",
      "windspeedKmph": "13",
      "windspeedMiles": "8"
    }
  ],
  "weather": [
    {
      "astronomy": [{"sunrise": "06:42 AM", "sunset": "08:14 PM"}],
      "maxtempC": "26",
      "maxtempF": "78",
      "mintempC": "16",
      "mintempF": "61",
      "hourly": [
        {"chanceofrain": "5",  "WindGustKmph": "10", "WindGustMiles": "6"},
        {"chanceofrain": "10", "WindGustKmph": "12", "WindGustMiles": "7"},
        {"chanceofrain": "20", "WindGustKmph": "15", "WindGustMiles": "9"},
        {"chanceofrain": "30", "WindGustKmph": "18", "WindGustMiles": "11"},
        {"chanceofrain": "25", "WindGustKmph": "24", "WindGustMiles": "15", "weatherDesc": [{"value": "Partly cloudy"}]},
        {"chanceofrain": "15", "WindGustKmph": "20", "WindGustMiles": "12"},
        {"chanceofrain": "5",  "WindGustKmph": "16", "WindGustMiles": "10"},
        {"chanceofrain": "0",  "WindGustKmph": "12", "WindGustMiles": "7"}
      ]
    },
    {
      "astronomy": [{"sunrise": "06:43 AM", "sunset": "08:13 PM"}],
      "maxtempC": "24",
      "maxtempF": "75",
      "mintempC": "15",
      "mintempF": "60",
      "hourly": [
        {"chanceofrain": "0"}, {"chanceofrain": "0"}, {"chanceofrain": "0"}, {"chanceofrain": "10"},
        {"chanceofrain": "10", "weatherDesc": [{"value": "Sunny"}]},
        {"chanceofrain": "5"}, {"chanceofrain": "0"}, {"chanceofrain": "0"}
      ]
    },
    {
      "astronomy": [{"sunrise": "06:44 AM", "sunset": "08:12 PM"}],
      "maxtempC": "23",
      "maxtempF": "73",
      "mintempC": "14",
      "mintempF": "57",
      "hourly": [
        {"chanceofrain": "40"}, {"chanceofrain": "50"}, {"chanceofrain": "60"}, {"chanceofrain": "70"},
        {"chanceofrain": "80", "weatherDesc": [{"value": "Heavy rain"}]},
        {"chanceofrain": "70"}, {"chanceofrain": "50"}, {"chanceofrain": "30"}
      ]
    }
  ]
}
```

- [ ] **Step 2: Write the failing test**

Append to the test module in `weather.rs`:

```rust
    #[test]
    fn units_enum_from_str_imperial_default() {
        assert_eq!(Units::from_config_str("metric"), Units::Metric);
        assert_eq!(Units::from_config_str("imperial"), Units::Imperial);
        assert_eq!(Units::from_config_str("METRIC"), Units::Metric);
        assert_eq!(Units::from_config_str(""), Units::Imperial);     // default
        assert_eq!(Units::from_config_str("bogus"), Units::Imperial); // default
    }
```

- [ ] **Step 3: Add the `Units` enum**

Insert near the top of `weather.rs`, just below the file-level doc comment:

```rust
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
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::units`
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/fixtures/wttr_sample.json src-tauri/src/weather.rs
git commit -m "feat(weather): add wttr.in JSON fixture and Units enum"
```

---

## Task 6: Parse wttr.in JSON into `WeatherInfo` (current conditions)

**Files:**
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Write the failing tests**

Append to the test module:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::parse`
Expected: compile error — `parse` and the underlying response types are undefined.

- [ ] **Step 3: Add `serde` response types and the `parse` function**

Add to `src-tauri/src/weather.rs` (above the `WeatherInfo` definitions):

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WttrResponse {
    current_condition: Vec<WttrCurrent>,
    weather: Vec<WttrDay>,
}

#[derive(Debug, Deserialize)]
struct WttrCurrent {
    #[serde(rename = "FeelsLikeC")] feels_c: String,
    #[serde(rename = "FeelsLikeF")] feels_f: String,
    cloudcover: String,
    humidity: String,
    #[serde(rename = "precipMM")] precip_mm: String,
    pressure: String,
    #[serde(rename = "pressureInches")] pressure_inches: String,
    #[serde(rename = "temp_C")] temp_c: String,
    #[serde(rename = "temp_F")] temp_f: String,
    #[serde(rename = "uvIndex")] uv_index: String,
    visibility: String,
    #[serde(rename = "visibilityMiles")] visibility_miles: String,
    #[serde(rename = "weatherDesc")] weather_desc: Vec<WttrDesc>,
    #[serde(rename = "winddir16Point")] wind_dir_16: String,
    #[serde(rename = "windspeedKmph")] windspeed_kmph: String,
    #[serde(rename = "windspeedMiles")] windspeed_miles: String,
}

#[derive(Debug, Deserialize)]
struct WttrDay {
    astronomy: Vec<WttrAstronomy>,
    #[serde(rename = "maxtempC")] maxtemp_c: String,
    #[serde(rename = "maxtempF")] maxtemp_f: String,
    #[serde(rename = "mintempC")] mintemp_c: String,
    #[serde(rename = "mintempF")] mintemp_f: String,
    hourly: Vec<WttrHourly>,
}

#[derive(Debug, Deserialize)]
struct WttrAstronomy {
    sunrise: String,
    sunset: String,
}

#[derive(Debug, Deserialize)]
struct WttrHourly {
    #[serde(default)] chanceofrain: String,
    #[serde(rename = "WindGustKmph", default)] wind_gust_kmph: String,
    #[serde(rename = "WindGustMiles", default)] wind_gust_miles: String,
    #[serde(rename = "weatherDesc", default)] weather_desc: Vec<WttrDesc>,
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

    let current = raw.current_condition.first()
        .ok_or_else(|| anyhow::anyhow!("wttr.in response missing current_condition"))?;
    let today = raw.weather.first()
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
    info.wind_gust = today.hourly.iter()
        .map(|h| pick(units, &h.wind_gust_kmph, &h.wind_gust_miles))
        .filter_map(|s| s.parse::<f64>().ok())
        .fold(None::<f64>, |acc, v| Some(acc.map_or(v, |a| a.max(v))))
        .map(|v| format!("{:.0}", v));
    info.precip_chance = today.hourly.iter()
        .filter_map(|h| h.chanceofrain.parse::<f64>().ok())
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

    Ok(info)
}

fn pick<'a>(units: Units, metric: &'a String, imperial: &'a String) -> &'a String {
    match units {
        Units::Metric => metric,
        Units::Imperial => imperial,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::parse`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/weather.rs
git commit -m "feat(weather): parse current-condition fields from wttr.in JSON"
```

---

## Task 7: Parse 3-day forecast into `WeatherInfo.days`

**Files:**
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Write the failing tests**

Append to the test module:

```rust
    #[test]
    fn parse_populates_three_forecast_days_imperial() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        let d1 = info.days[0].as_ref().expect("D1 missing");
        let d2 = info.days[1].as_ref().expect("D2 missing");
        let d3 = info.days[2].as_ref().expect("D3 missing");
        assert_eq!(d1.hi, Some("78".into()));
        assert_eq!(d2.hi, Some("75".into()));
        assert_eq!(d3.hi, Some("73".into()));
        assert_eq!(d1.lo, Some("61".into()));
        assert_eq!(d3.lo, Some("57".into()));
    }

    #[test]
    fn parse_populates_forecast_condition_and_abbrev() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        let d1 = info.days[0].as_ref().unwrap();
        let d2 = info.days[1].as_ref().unwrap();
        let d3 = info.days[2].as_ref().unwrap();
        // Pulled from hourly[4].weatherDesc[0].value
        assert_eq!(d1.condition, Some("Partly cloudy".into()));
        assert_eq!(d1.condition_short, Some("P.Cloudy".into()));
        assert_eq!(d2.condition, Some("Sunny".into()));
        assert_eq!(d2.condition_short, Some("Sunny".into()));
        assert_eq!(d3.condition, Some("Heavy rain".into()));
        assert_eq!(d3.condition_short, Some("H.Rain".into()));
    }

    #[test]
    fn parse_populates_forecast_precip_chance_max() {
        let json = load_fixture();
        let info = parse(&json, Units::Imperial).unwrap();
        let d1 = info.days[0].as_ref().unwrap();
        let d2 = info.days[1].as_ref().unwrap();
        let d3 = info.days[2].as_ref().unwrap();
        assert_eq!(d1.precip_chance, Some("30".into())); // max of 5,10,20,30,25,15,5,0
        assert_eq!(d2.precip_chance, Some("10".into()));
        assert_eq!(d3.precip_chance, Some("80".into()));
    }

    #[test]
    fn parse_handles_missing_forecast_days() {
        // Strip out everything past weather[0] (one-day response).
        let mut value: serde_json::Value =
            serde_json::from_str(&load_fixture()).unwrap();
        let arr = value["weather"].as_array_mut().unwrap();
        arr.truncate(1);
        let trimmed = serde_json::to_string(&value).unwrap();

        let info = parse(&trimmed, Units::Imperial).unwrap();
        assert!(info.days[0].is_some());
        assert!(info.days[1].is_none());
        assert!(info.days[2].is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::parse_populates`
Expected: 3 tests fail with assertion errors (days slots are `None`). The fourth (`parse_handles_missing_forecast_days`) passes because the array defaults to `None`.

- [ ] **Step 3: Populate forecast days in `parse`**

Replace the end of `parse` in `weather.rs` (just before `Ok(info)`) — add this block after the existing field assignments:

```rust
    // Forecast days: D1 = weather[1], D2 = weather[2], D3 = weather[3].
    for (slot, day_index) in (1..=3usize).enumerate() {
        if let Some(day) = raw.weather.get(day_index) {
            let condition = day.hourly.get(4)
                .and_then(|h| h.weather_desc.first())
                .map(|d| d.value.clone());
            let precip_chance = day.hourly.iter()
                .filter_map(|h| h.chanceofrain.parse::<f64>().ok())
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests`
Expected: All weather tests pass (parsing + lookup + abbreviate + units = ~17 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/weather.rs
git commit -m "feat(weather): parse 3-day forecast into WeatherInfo.days"
```

---

## Task 8: `WeatherReader` shell + `with_cached_info` test constructor

**Files:**
- Modify: `src-tauri/src/weather.rs`

- [ ] **Step 1: Write the failing tests**

Append to the test module:

```rust
    #[test]
    fn reader_with_cached_info_returns_field() {
        let info = sample_info();
        let reader = WeatherReader::with_cached_info(info);
        assert_eq!(reader.get_field(WeatherField::Temp), Some("72".into()));
        assert_eq!(reader.get_field(WeatherField::ConditionShort), Some("P.Cloudy".into()));
    }

    #[test]
    fn reader_new_disabled_returns_none() {
        let reader = WeatherReader::disabled();
        assert_eq!(reader.get_field(WeatherField::Temp), None);
        assert_eq!(reader.get_field(WeatherField::HiD(1)), None);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::reader`
Expected: compile error — `WeatherReader` undefined.

- [ ] **Step 3: Add `WeatherReader` (no thread yet)**

Append to `weather.rs` (above the test module):

```rust
use std::sync::{Arc, RwLock};

/// Reads weather data for `run_sensors`. Backed by a shared cache that the
/// refresh thread writes; this reader only reads.
pub struct WeatherReader {
    shared: Arc<RwLock<Option<WeatherInfo>>>,
}

impl WeatherReader {
    /// Construct a reader with no data and no refresh thread. All field lookups return `None`.
    /// Used when `[Weather]` is not configured.
    pub fn disabled() -> Self {
        Self {
            shared: Arc::new(RwLock::new(None)),
        }
    }

    /// Test-only constructor with a pre-populated cache.
    pub fn with_cached_info(info: WeatherInfo) -> Self {
        Self {
            shared: Arc::new(RwLock::new(Some(info))),
        }
    }

    /// Look up a field from the cached `WeatherInfo`. Returns `None` if no data
    /// has been fetched yet, or the field is unset.
    pub fn get_field(&self, field: WeatherField) -> Option<String> {
        self.shared.read().ok()?.as_ref()?.get(field)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::reader`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/weather.rs
git commit -m "feat(weather): add WeatherReader with cache-backed field lookup"
```

---

## Task 9: `WeatherConfig` in `settings.rs`

**Files:**
- Modify: `src-tauri/src/settings.rs`

- [ ] **Step 1: Write the failing tests**

Append to the test module at the bottom of `src-tauri/src/settings.rs` (find the existing `mod tests` — if there isn't one, append a new module). Use this test file structure:

```rust
#[cfg(test)]
mod weather_config_tests {
    use super::*;
    use ini::Ini;

    fn ini_with_weather(section: &str) -> Ini {
        let raw = format!("[Main]\nstyle=vertical\n\n{}", section);
        Ini::load_from_str(&raw).unwrap()
    }

    #[test]
    fn weather_config_missing_section_disabled() {
        let ini = ini_with_weather("");
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(!cfg.enabled);
        assert_eq!(cfg.location, "");
    }

    #[test]
    fn weather_config_empty_location_disabled() {
        let ini = ini_with_weather("[Weather]\nlocation=\"\"\nunits=\"metric\"\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(!cfg.enabled);
    }

    #[test]
    fn weather_config_populated_enabled() {
        let ini = ini_with_weather(
            "[Weather]\nlocation=\"Seattle,US\"\nunits=\"metric\"\nrefresh_minutes=10\n",
        );
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(cfg.enabled);
        assert_eq!(cfg.location, "Seattle,US");
        assert_eq!(cfg.units, crate::weather::Units::Metric);
        assert_eq!(cfg.refresh_minutes, 10);
    }

    #[test]
    fn weather_config_defaults_when_keys_missing() {
        let ini = ini_with_weather("[Weather]\nlocation=\"Boston\"\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert!(cfg.enabled);
        assert_eq!(cfg.units, crate::weather::Units::Imperial);
        assert_eq!(cfg.refresh_minutes, 15);
    }

    #[test]
    fn weather_config_refresh_minutes_clamped_to_one() {
        let ini = ini_with_weather("[Weather]\nlocation=\"X\"\nrefresh_minutes=0\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert_eq!(cfg.refresh_minutes, 1);
    }

    #[test]
    fn weather_config_strips_surrounding_quotes_from_location() {
        // ini crate already strips quotes on load, but be explicit in case of nesting.
        let ini = ini_with_weather("[Weather]\nlocation=Boise,US\n");
        let cfg = WeatherConfig::from_ini(&ini);
        assert_eq!(cfg.location, "Boise,US");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::weather_config_tests`
Expected: compile error — `WeatherConfig` undefined.

- [ ] **Step 3: Add `WeatherConfig` and attach to `AppConfig`**

In `src-tauri/src/settings.rs`:

Add near the top with the other use statements:

```rust
use crate::weather::Units;
```

Add the struct after the `CustomSensor` definition (around line 16):

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeatherConfig {
    pub enabled: bool,
    pub location: String,
    #[serde(with = "units_serde")]
    pub units: Units,
    pub refresh_minutes: u64,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            location: String::new(),
            units: Units::Imperial,
            refresh_minutes: 15,
        }
    }
}

impl WeatherConfig {
    pub fn from_ini(config: &ini::Ini) -> Self {
        let section = match config.section(Some("Weather")) {
            Some(s) => s,
            None => return Self::default(),
        };
        let location = section.get("location").unwrap_or("").trim().to_string();
        if location.is_empty() {
            return Self::default();
        }
        let units = Units::from_config_str(section.get("units").unwrap_or(""));
        let refresh_minutes = section
            .get("refresh_minutes")
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.max(1))
            .unwrap_or(15);
        Self {
            enabled: true,
            location,
            units,
            refresh_minutes,
        }
    }
}

mod units_serde {
    use super::Units;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(units: &Units, s: S) -> Result<S::Ok, S::Error> {
        match units {
            Units::Metric => "metric",
            Units::Imperial => "imperial",
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Units, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Units::from_config_str(&s))
    }
}
```

Add a `weather: WeatherConfig` field to the `AppConfig` struct (after `custom_sensors`):

```rust
    #[serde(default)]
    pub weather: WeatherConfig,
```

In the `AppConfig::from_ini` `Ok(Self { ... })` block, add the weather field at the bottom (after `custom_sensors: ...`):

```rust
            weather: WeatherConfig::from_ini(config),
```

The `Default for WeatherConfig` is needed because `#[serde(default)]` invokes it when the field is absent in serialized JSON state.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::weather_config_tests`
Expected: 6 tests pass.

- [ ] **Step 5: Run the full test suite to ensure nothing else broke**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(weather): parse [Weather] section into WeatherConfig"
```

---

## Task 10: Background refresh thread

**Files:**
- Modify: `src-tauri/src/weather.rs`

This task wires up the actual HTTP fetch loop. The thread is intentionally **not** unit-tested directly (would require either a mock HTTP layer we don't need, or a flaky network test). The pure pieces (`parse`, `get_field`) are already covered. The thread is a thin wrapper.

- [ ] **Step 1: Write the failing test**

Append to the test module:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests::reader_spawn`
Expected: compile error — `WeatherReader::spawn` and `build_request_url` undefined.

- [ ] **Step 3: Add `spawn` + `build_request_url` + the fetch loop**

Add to `weather.rs` near the top:

```rust
use log::{debug, info, warn};
use std::thread;
use std::time::Duration;
```

Add to the `WeatherReader` impl:

```rust
    /// Construct a reader for the given config. If `cfg.enabled` is false, returns a disabled
    /// reader. Otherwise spawns a background thread that refreshes `WeatherInfo` on `cfg.refresh_minutes`.
    pub fn spawn(cfg: &crate::settings::WeatherConfig) -> Self {
        if !cfg.enabled {
            info!("Weather disabled (no [Weather].location in config)");
            return Self::disabled();
        }
        let shared = Arc::new(RwLock::new(None));
        let url = build_request_url(&cfg.location);
        let units = cfg.units;
        let refresh = Duration::from_secs(cfg.refresh_minutes.saturating_mul(60));
        let shared_clone = Arc::clone(&shared);

        thread::Builder::new()
            .name("weather-refresh".to_string())
            .spawn(move || refresh_loop(shared_clone, url, units, refresh))
            .expect("spawn weather refresh thread");

        Self { shared }
    }
```

Add free functions below the `WeatherReader` impl:

```rust
/// Build the wttr.in request URL. Spaces in the location are percent-encoded.
pub(crate) fn build_request_url(location: &str) -> String {
    let encoded: String = location
        .chars()
        .map(|c| if c == ' ' { "%20".to_string() } else { c.to_string() })
        .collect();
    format!("https://wttr.in/{}?format=j1", encoded)
}

fn refresh_loop(
    shared: Arc<RwLock<Option<WeatherInfo>>>,
    url: String,
    units: Units,
    refresh: Duration,
) {
    loop {
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
        thread::sleep(refresh);
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml weather::tests`
Expected: All weather tests pass. The `reader_spawn_disabled_when_location_empty` test does NOT hit the network (cfg.enabled = false).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/weather.rs
git commit -m "feat(weather): background refresh thread fetches wttr.in periodically"
```

---

## Task 11: Wire `WeatherReader` into `Daemon` + `run_sensors` signature

**Files:**
- Modify: `src-tauri/src/utils.rs`
- Modify: `src-tauri/src/daemon.rs`
- Modify: `src-tauri/src/gui.rs`

This task extends `run_sensors` to accept a `&WeatherReader` parameter, threads it through `build_display_value` and `Daemon`, and updates the `gui.rs` call site. No `WEATHER_*` branch yet — that's Task 12.

- [ ] **Step 1: Update `run_sensors` signature**

In `src-tauri/src/utils.rs`, change the `run_sensors` signature from:

```rust
pub fn run_sensors<'a>(
    pages_sensors: &'a ini::Properties,
    labels: &mut Vec<&'a str>,
    units: &mut Vec<&'a str>,
    values: &mut Vec<String>,
    hwinfo: &Hwinfo,
    decimal: bool,
    mouse_battery_reader: &mut MouseBatteryReader,
    media_reader: &mut MediaReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<(), anyhow::Error> {
```

to:

```rust
pub fn run_sensors<'a>(
    pages_sensors: &'a ini::Properties,
    labels: &mut Vec<&'a str>,
    units: &mut Vec<&'a str>,
    values: &mut Vec<String>,
    hwinfo: &Hwinfo,
    decimal: bool,
    mouse_battery_reader: &mut MouseBatteryReader,
    media_reader: &mut MediaReader,
    weather_reader: &crate::weather::WeatherReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<(), anyhow::Error> {
```

Add this import at the top of `utils.rs` (after existing `use crate::media::...`):

```rust
use crate::weather::WeatherReader;
```

- [ ] **Step 2: Update every existing `run_sensors` test in `utils.rs`**

Every test in `utils.rs::tests` that calls `run_sensors` currently passes 9 args. Add a `&WeatherReader::disabled()` argument before `None` / `hid_api`. There are many call sites — use a find/replace.

Pattern to replace (one variant):

```rust
        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            None,
        )
```

becomes:

```rust
        let weather = WeatherReader::disabled();
        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
```

Apply the same change to the `true`-decimal variant too. Repeat for every `run_sensors(` call in `utils.rs::tests` (there are 15+). Add `use crate::weather::WeatherReader;` to the test module's `use super::*;` if needed (it's `pub` so `super::*` brings it in).

- [ ] **Step 3: Update `build_display_value` and `Daemon` in `daemon.rs`**

In `src-tauri/src/daemon.rs`, update `build_display_value` signature (around line 331):

```rust
fn build_display_value(
    config: &AppConfig,
    hwinfo: &Hwinfo,
    pages_vec: &[ini::Properties],
    page_counter: usize,
    mouse: &mut MouseBatteryReader,
    media: &mut MediaReader,
    weather: &crate::weather::WeatherReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<Value, anyhow::Error> {
```

Update the `run_sensors` call inside it (around line 356) to pass `weather` before `hid_api`:

```rust
        run_sensors(
            pages_sensors,
            &mut labels,
            &mut units,
            &mut values,
            hwinfo,
            config.decimal,
            mouse,
            media,
            weather,
            hid_api,
        )?;
```

Add `weather_reader: WeatherReader` field to the `Daemon` struct (after `media_reader: MediaReader,` around line 394):

```rust
    weather_reader: crate::weather::WeatherReader,
```

Update `Daemon::new` (around line 401) to construct the reader from config:

```rust
            weather_reader: crate::weather::WeatherReader::spawn(&config.weather),
```

Find every call to `build_display_value(` inside `daemon.rs` and add `&self.weather_reader` / `&weather` argument before `hid_api`. There's one in the real run path and several in tests.

For the tests in `daemon.rs::tests` that call `build_display_value` directly (around line 1043), add a local `let weather = crate::weather::WeatherReader::disabled();` and pass `&weather`.

- [ ] **Step 4: Update `gui.rs` call site**

In `src-tauri/src/gui.rs` around line 305, update the `run_sensors` call. Add before the call:

```rust
        let weather = crate::weather::WeatherReader::disabled();
```

Then add `&weather` to the argument list before `None`/`hid_api`:

```rust
        if let Err(e) = run_sensors(
            &props,
            // ...
            &mut mb,
            &mut media,
            &weather,
            None,
        ) { /* ... */ }
```

- [ ] **Step 5: Verify the build compiles**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: build succeeds. If a `run_sensors(` call was missed, the compiler will flag it.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all tests pass. No behavior change yet — `WEATHER_*` sensors still fall through to "Sensor not found" because the dispatch branch isn't added until Task 12.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/utils.rs src-tauri/src/daemon.rs src-tauri/src/gui.rs
git commit -m "feat(weather): thread WeatherReader through Daemon and run_sensors"
```

---

## Task 12: Add `WEATHER_*` dispatch branch in `run_sensors`

**Files:**
- Modify: `src-tauri/src/utils.rs`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/utils.rs::tests`. Add this near the top of the test module (just after `empty_buffers`):

```rust
    fn weather_with_info() -> crate::weather::WeatherReader {
        let mut info = crate::weather::WeatherInfo::default();
        info.temp = Some("72".into());
        info.condition_short = Some("P.Cloudy".into());
        info.days[0] = Some(crate::weather::DayForecast {
            hi: Some("75".into()),
            lo: Some("60".into()),
            condition: Some("Sunny".into()),
            condition_short: Some("Sunny".into()),
            precip_chance: Some("10".into()),
        });
        crate::weather::WeatherReader::with_cached_info(info)
    }
```

Then add the tests:

```rust
    #[test]
    fn test_run_sensors_weather_temp_returns_cached() {
        let props = make_props(&[("sensor_0", "WEATHER_TEMP"), ("label_0", "Out"), ("unit_0", "°F")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props, &mut labels, &mut units, &mut values,
            &hwinfo, false, &mut mouse, &mut media, &weather, None,
        ).unwrap();

        assert_eq!(labels[0], "Out");
        assert_eq!(units[0], "°F");
        assert_eq!(values[0], "72");
    }

    #[test]
    fn test_run_sensors_weather_forecast_day_field() {
        let props = make_props(&[("sensor_0", "WEATHER_HI_D1"), ("label_0", "Tmrw"), ("unit_0", "°")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props, &mut labels, &mut units, &mut values,
            &hwinfo, false, &mut mouse, &mut media, &weather, None,
        ).unwrap();

        assert_eq!(labels[0], "Tmrw");
        assert_eq!(units[0], "°");
        assert_eq!(values[0], "75");
    }

    #[test]
    fn test_run_sensors_weather_hides_when_no_data() {
        let props = make_props(&[("sensor_0", "WEATHER_TEMP"), ("label_0", "Out"), ("unit_0", "°F")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props, &mut labels, &mut units, &mut values,
            &hwinfo, false, &mut mouse, &mut media, &weather, None,
        ).unwrap();

        // No data → sensor hides (empty label/unit/value).
        assert_eq!(labels[0], "");
        assert_eq!(units[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_weather_unset_field_hides() {
        // Reader has *some* info but temp is None → sensor hides.
        let info = crate::weather::WeatherInfo::default(); // every field None
        let weather = crate::weather::WeatherReader::with_cached_info(info);

        let props = make_props(&[("sensor_0", "WEATHER_TEMP"), ("label_0", "T"), ("unit_0", "°F")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props, &mut labels, &mut units, &mut values,
            &hwinfo, false, &mut mouse, &mut media, &weather, None,
        ).unwrap();

        assert_eq!(labels[0], "");
        assert_eq!(values[0], "");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml utils::tests::test_run_sensors_weather`
Expected: first two assert failures (value isn't set because no dispatch branch yet); third+fourth pass coincidentally because dispatch fails through to "Sensor not found" Err — which means the test will error before assertion. Adjust: expect compile-time success; the first two tests will fail at the `unwrap()` after `run_sensors` because the code path returns an Err (`Sensor not found: WEATHER_TEMP / ` because sensor[0]=WEATHER_TEMP, sensor.len() < 2 → malformed → continue → labels/units stay empty). So actually first two will fail their value assertion (empty != "72").

- [ ] **Step 3: Add the dispatch branch**

In `src-tauri/src/utils.rs::run_sensors`, after the existing `MEDIA_*` branch (just before the `if sensor.len() < 2` malformed-check), insert:

```rust
        } else if let Some(weather_field) = crate::weather::WeatherField::from_sensor_name(sensor[0]) {
            match weather_reader.get_field(weather_field) {
                Some(value) => {
                    labels[k] = label;
                    units[k] = unit;
                    values[k] = value;
                }
                None => {
                    // No data yet, or field unset → hide sensor slot (matches MEDIA_* pattern).
                    labels[k] = "";
                    units[k] = "";
                    values[k] = String::new();
                }
            }
            continue;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml utils::tests`
Expected: all `utils::tests` pass, including 4 new weather tests.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/utils.rs
git commit -m "feat(weather): wire WEATHER_* sensors into run_sensors dispatch"
```

---

## Task 13: Document `WEATHER_*` sensors in README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Find the "Special Sensors" section**

Open `README.md` and locate the line `### Special Sensors` (currently around line 186).

- [ ] **Step 2: Add the WEATHER documentation**

Insert this block immediately after the `**DATE**:` bullet block, before `**BLANK**`:

```markdown
- **WEATHER_***: Live weather data from [wttr.in](https://wttr.in) (no API key required)
  - Requires a `[Weather]` section in `conf.ini`:
    ```ini
    [Weather]
    location="Seattle,US"     # or "lat,lon" e.g. "47.6,-122.3"
    units="imperial"          # metric | imperial
    refresh_minutes=15        # default 15, minimum 1
    ```
  - **Current conditions:**
    - `WEATHER_TEMP` — current temperature
    - `WEATHER_FEELS` — feels-like temperature
    - `WEATHER_HI` / `WEATHER_LO` — today's high / low
    - `WEATHER_CONDITION` — text condition (e.g., `Partly cloudy`)
    - `WEATHER_CONDITION_SHORT` — ≤ 8-char abbreviation (e.g., `P.Cloudy`)
    - `WEATHER_HUMIDITY` — humidity %
    - `WEATHER_WIND_SPEED` / `WEATHER_WIND_DIR` / `WEATHER_WIND_GUST` — wind info
    - `WEATHER_PRECIP_CHANCE` — % chance of precipitation today
    - `WEATHER_PRECIP_AMOUNT` — precipitation amount (mm metric, inches imperial)
    - `WEATHER_UV` — UV index
    - `WEATHER_PRESSURE` — barometric pressure (hPa metric, inHg imperial)
    - `WEATHER_CLOUDS` — cloud cover %
    - `WEATHER_VISIBILITY` — visibility (km metric, miles imperial)
    - `WEATHER_SUNRISE` / `WEATHER_SUNSET` — formatted time strings (e.g., `06:42 AM`)
  - **3-day forecast** (suffix `_D1` = tomorrow, `_D2` = day after, `_D3` = day after that):
    - `WEATHER_HI_D{n}` / `WEATHER_LO_D{n}` — high / low for day n
    - `WEATHER_CONDITION_D{n}` / `WEATHER_CONDITION_SHORT_D{n}` — condition text / abbreviation
    - `WEATHER_PRECIP_CHANCE_D{n}` — % chance of precipitation for day n
  - Configuration example:
    ```ini
    sensor_0="WEATHER_TEMP"
    label_0="Out:"
    unit_0="°F"

    sensor_1="WEATHER_CONDITION_SHORT"
    label_1=""
    unit_1=""

    sensor_2="WEATHER_HI_D1"
    label_2="Tmrw"
    unit_2="°"
    ```
  - Data refreshes in a background thread; if the network is unavailable, the last good value is kept on screen. If no `[Weather]` section is configured, all `WEATHER_*` sensor slots hide.

```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document WEATHER_* sensors and [Weather] config section"
```

---

## Self-Review

After all tasks are committed, verify:

1. **Spec coverage:**
   - 18 current fields + 15 forecast fields = 33 total — Task 2 (enum), Task 3 (lookup), Task 6/7 (parse). ✓
   - Abbreviation table — Task 4. ✓
   - Units selection (metric/imperial) — Task 5/6. ✓
   - `[Weather]` config parsing with defaults & clamping — Task 9. ✓
   - Background refresh thread, never blocks main tick — Task 10. ✓
   - Error handling: keeps prior on fetch fail, hides sensors when no data — Task 10/12. ✓
   - Tests for hide-on-None mirroring MEDIA_* — Task 12. ✓
   - README documentation — Task 13. ✓

2. **No placeholders:** all code blocks are complete; every step has a command with expected output or a code body.

3. **Type consistency:** `WeatherReader` named the same in every task; `WeatherField`, `WeatherInfo`, `DayForecast`, `Units`, `WeatherConfig` are introduced once and referenced consistently. `run_sensors` signature change in Task 11 matches the call in Task 12.

4. **Frequent commits:** every task ends with a commit; ~13 commits total, all small.

---

## Out of Scope (deferred)

- `WEATHER_LOCATION` field (rejected during brainstorm).
- Hourly forecast.
- Provider abstraction / OpenWeatherMap fallback.
- Severe-weather alerts.
- First-fetch synchronous prime on startup (the background thread starts immediately; sensors hide for up to a few seconds).
