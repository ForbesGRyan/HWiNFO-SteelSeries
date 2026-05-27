# Weather Sensors — Design Spec

**Date:** 2026-05-27
**Status:** Approved (pending user spec review)
**Scope:** Add weather as a new special data source to HWiNFO-SteelSeries, exposing current conditions and a 3-day forecast as `WEATHER_*` sensors usable in `conf.ini` custom sensor slots.

## Goals

- Display weather data on SteelSeries OLED alongside HWiNFO sensors, CLOCK, DATE, MEDIA_*, etc.
- "Full dashboard" coverage: replace need to check a weather app for current conditions and short-term outlook.
- Zero-friction setup: no API keys; user provides only location and units.
- Never block the 1 s main tick on network I/O.
- Match existing special-sensor conventions so users learn the pattern once.

## Non-Goals

- Hourly forecast slots.
- Multi-location support (one location per app instance).
- Severe-weather alerts / push notifications.
- Historical data.
- Provider abstraction (single provider; swap is a future change).

## Data Source

**wttr.in** (`https://wttr.in/<location>?format=j1`).

- Free, no API key, no signup.
- Single GET returns current conditions, 3-day forecast, astronomy (sunrise/sunset), and location echo.
- Trade-off: community-run service, can have outages or rate limits. Stale-but-shown data is the failure mode (acceptable for a desktop ornament).

Future swap to OpenWeatherMap or similar is possible behind the same `WeatherReader` interface; not in scope here.

## Architecture

### New module: `src-tauri/src/weather.rs`

```text
WeatherReader
├─ shared: Arc<RwLock<Option<WeatherInfo>>>
├─ enabled: bool                      # false if location missing
└─ spawn(config) -> Self              # starts background refresh thread

WeatherInfo (cached parsed response)
├─ current: CurrentWeather
└─ forecast: [DayForecast; 3]         # D1, D2, D3

WeatherField (enum)
├─ Temp, Feels, Hi, Lo, Condition, ConditionShort,
├─ Humidity, WindSpeed, WindDir, WindGust,
├─ PrecipChance, PrecipAmount, Uv, Pressure, Clouds, Visibility,
├─ Sunrise, Sunset,
├─ HiD(u8), LoD(u8), ConditionD(u8), ConditionShortD(u8), PrecipChanceD(u8)
│
└─ from_sensor_name(&str) -> Option<WeatherField>
       # parses "WEATHER_TEMP", "WEATHER_HI_D2", etc.
```

### Integration points

1. **`consts.rs`** — no changes required (existing `CUSTOM_SENSORS` / `DISPLAY_LINES` apply).
2. **`settings.rs`** — parse `[Weather]` section into a `WeatherConfig` struct; expose on `AppConfig`.
3. **`main.rs` / `daemon.rs`** — instantiate `WeatherReader` at startup; pass into `run_sensors`.
4. **`utils.rs::run_sensors`** — add `WEATHER_*` branch after `MEDIA_*` branch.
5. **`Cargo.toml`** — add `ureq = { version = "2", features = ["json"] }`.

## Configuration

New `conf.ini` section:

```ini
[Weather]
location="Seattle,US"     # or "lat,lon" e.g. "47.6,-122.3"; passed to wttr.in path
units="imperial"          # metric | imperial
refresh_minutes=15        # default 15; clamped to >= 1
```

Behavior:

- Missing or empty `location` → weather disabled, all `WEATHER_*` sensors hide, log info on startup.
- Invalid `units` → fall back to `imperial`, log warn.
- `refresh_minutes` < 1 → clamped to 1.

Sensors continue to use the existing `sensor_X` / `label_X` / `unit_X` keys. Numeric weather fields return bare numbers; the user supplies the display unit:

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

## Field Catalog

All fields return `Option<String>`. `None` → sensor hides on the OLED (slot blanks out).

### Current conditions (18 fields)

| Sensor                     | wttr.in source                                | Notes                                          |
|----------------------------|-----------------------------------------------|------------------------------------------------|
| `WEATHER_TEMP`             | `current_condition[0].temp_C/F`               | Bare number                                    |
| `WEATHER_FEELS`            | `current_condition[0].FeelsLikeC/F`           | Bare number                                    |
| `WEATHER_HI`               | `weather[0].maxtempC/F`                       | Today's high                                   |
| `WEATHER_LO`               | `weather[0].mintempC/F`                       | Today's low                                    |
| `WEATHER_CONDITION`        | `current_condition[0].weatherDesc[0].value`   | Full text, may be long                         |
| `WEATHER_CONDITION_SHORT`  | abbreviation table (see below)                | ≤ 8 chars                                      |
| `WEATHER_HUMIDITY`         | `current_condition[0].humidity`               | %                                              |
| `WEATHER_WIND_SPEED`       | `current_condition[0].windspeedKmph/Miles`    | Bare number                                    |
| `WEATHER_WIND_DIR`         | `current_condition[0].winddir16Point`         | e.g., `NW`                                     |
| `WEATHER_WIND_GUST`        | `weather[0].hourly[*].WindGustKmph/Miles` max | Today's max gust                               |
| `WEATHER_PRECIP_CHANCE`    | `weather[0].hourly[*].chanceofrain` max       | % today                                        |
| `WEATHER_PRECIP_AMOUNT`    | `current_condition[0].precipMM`               | mm; divided by 25.4 when `units=imperial`      |
| `WEATHER_UV`               | `current_condition[0].uvIndex`                | Integer                                        |
| `WEATHER_PRESSURE`         | `current_condition[0].pressure/pressureInches`| hPa or inHg                                    |
| `WEATHER_CLOUDS`           | `current_condition[0].cloudcover`             | %                                              |
| `WEATHER_VISIBILITY`       | `current_condition[0].visibility/visibilityMiles` | km or mi                                   |
| `WEATHER_SUNRISE`          | `weather[0].astronomy[0].sunrise`             | Pass-through string ("06:42 AM")               |
| `WEATHER_SUNSET`           | `weather[0].astronomy[0].sunset`              | Pass-through string ("08:14 PM")               |

### Forecast — D1, D2, D3 (5 fields × 3 days = 15)

Where `n ∈ {1, 2, 3}` and the wttr.in array index is `n`:

| Sensor                            | Source                                             |
|-----------------------------------|----------------------------------------------------|
| `WEATHER_HI_D{n}`                 | `weather[n].maxtempC/F`                            |
| `WEATHER_LO_D{n}`                 | `weather[n].mintempC/F`                            |
| `WEATHER_CONDITION_D{n}`          | `weather[n].hourly[4].weatherDesc[0].value` (~midday slot) |
| `WEATHER_CONDITION_SHORT_D{n}`    | abbreviation table applied to the above            |
| `WEATHER_PRECIP_CHANCE_D{n}`      | `weather[n].hourly[*].chanceofrain` max            |

**Total: 33 fields.**

### Condition abbreviation table

Used by `WEATHER_CONDITION_SHORT` and `WEATHER_CONDITION_SHORT_D{n}`. Output is ≤ 8 chars.

| Full text                    | Abbrev     |
|------------------------------|------------|
| `Sunny`                      | `Sunny`    |
| `Clear`                      | `Clear`    |
| `Partly cloudy`              | `P.Cloudy` |
| `Cloudy`                     | `Cloudy`   |
| `Overcast`                   | `Overcast` |
| `Mist`                       | `Mist`     |
| `Fog`                        | `Fog`      |
| `Light rain` / `Patchy rain` | `L.Rain`   |
| `Moderate rain`              | `M.Rain`   |
| `Heavy rain`                 | `H.Rain`   |
| `Light snow`                 | `L.Snow`   |
| `Moderate snow`              | `M.Snow`   |
| `Heavy snow`                 | `H.Snow`   |
| `Thunderstorm` / `Thundery*` | `T.Storm`  |
| _other_                      | First 8 chars of full text |

## Data Flow

```text
Startup
└─ AppConfig parsed
   └─ if [Weather].location present:
      WeatherReader::spawn(config) → starts std::thread::spawn refresh loop
                                     returns reader holding Arc<RwLock<Option<WeatherInfo>>>

Refresh thread (loop)
├─ fetch wttr.in (ureq, 10 s timeout)
├─ parse JSON → WeatherInfo
├─ acquire write lock, store Some(WeatherInfo)
└─ sleep refresh_minutes; on error log + keep prior value

Main tick (every 1 s, in daemon.rs / run_sensors)
└─ for each configured sensor:
   └─ WeatherField::from_sensor_name(name) hits?
      └─ weather_reader.get_field(field):
         ├─ Some(val) → labels[k]=label, units[k]=unit, values[k]=val
         └─ None      → labels[k]="", units[k]="", values[k]="" (sensor hidden)
```

## Error Handling

| Failure                              | Behavior                                                     |
|--------------------------------------|--------------------------------------------------------------|
| Network error / non-2xx HTTP         | Log warn; keep prior `WeatherInfo`; sensor keeps last value. |
| JSON parse error                     | Log warn with body snippet (truncated); keep prior.          |
| First fetch fails on startup         | `Arc<RwLock>` stays `None`; all `WEATHER_*` sensors hidden until first success. App does not crash. |
| Specific field missing from response | That `WeatherField` returns `None`; only that sensor hides.  |
| Forecast day absent (e.g., D3 not in `weather[]`) | `WEATHER_*_D3` returns `None`; sensor hides.            |
| Missing `[Weather].location`         | Thread not started; log info; all `WEATHER_*` sensors hidden.|
| Invalid `units` value                | Fall back to `imperial`; log warn at startup.                |

## Testing

Tests live in `weather.rs` and extend `utils.rs` test module.

### `weather.rs` tests

- Parse a committed fixture JSON (snippet of a real wttr.in `format=j1` response, stored under `src-tauri/tests/fixtures/wttr_seattle.json` or inline as a `const` string).
- For each `WeatherField`, assert the extracted value matches the fixture.
- `metric` vs `imperial` selects the correct source field (e.g., `temp_C` vs `temp_F`).
- `PRECIP_AMOUNT` divides mm by 25.4 when `imperial`.
- `CONDITION_SHORT` mapping table: each documented full-text input maps to the documented abbrev; unknown input truncates to 8 chars.
- `from_sensor_name` parses every documented sensor name; rejects malformed (`WEATHER_HI_D9`, `WEATHER_FOO`).
- `WeatherInfo` with day absent returns `None` for that day's fields.

### `utils.rs::run_sensors` tests (additions)

- `WEATHER_TEMP` with a `WeatherReader::with_cached_info(WeatherInfo)` returns the cached value.
- `WEATHER_TEMP` with no cached info (None) hides the sensor (empty label/unit/value), mirroring `test_run_sensors_media_hides_when_nothing_playing`.
- `WEATHER_HI_D2` resolves via the day index.

### Out of scope

- No live network test against wttr.in (flaky, network-dependent).
- No end-to-end OLED rendering test (handled by existing format tests).

## Dependencies

- **New:** `ureq = { version = "2", features = ["json"] }` — sync HTTP, small, fits the project's sync architecture. Brings `serde_json` (already present) into play for parsing.
- **Reused:** `serde` (already a dep) for `#[derive(Deserialize)]` on the wttr.in response struct.

## File-Level Plan

| File                              | Change                                                       |
|-----------------------------------|--------------------------------------------------------------|
| `src-tauri/src/weather.rs`        | **New.** `WeatherReader`, `WeatherInfo`, `WeatherField`, abbrev table, refresh thread, tests. |
| `src-tauri/src/utils.rs`          | Add `WEATHER_*` branch in `run_sensors`; add unit tests.     |
| `src-tauri/src/settings.rs`       | Parse `[Weather]` section into `WeatherConfig`; attach to `AppConfig`. |
| `src-tauri/src/main.rs`           | Add `mod weather;` alongside existing `mod media;` / `mod mouse_battery;`; construct `WeatherReader`; thread into run loop. |
| `src-tauri/src/daemon.rs`         | Pass `WeatherReader` into `run_sensors`.                     |
| `src-tauri/Cargo.toml`            | Add `ureq` dependency.                                       |
| `README.md`                       | Document `WEATHER_*` sensors and `[Weather]` config section. |

## Open Questions (resolved during brainstorm)

- **Provider?** wttr.in (no API key).
- **Field shape?** Raw numeric values + text strings; user supplies `unit_X` for display unit.
- **Forecast depth?** Today + 3-day (D1, D2, D3).
- **Condition handling?** Two fields — full text and ≤8-char abbreviation.
- **Refresh model?** Background thread, default 15 min, never blocks main tick.
- **Location?** Single location per app instance (no roaming, no multi-region).
