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
}
