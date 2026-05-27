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
}
