//! IP-based geolocation lookup using the MaxMind GeoLite2 database,
//! and locale generation based on country/territory information.

use crate::geoip_downloader::GeoIPDownloader;
use chrono::Offset;
use maxminddb::{geoip2, Reader};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::str::FromStr;

const TERRITORY_INFO_XML: &str = include_str!("territory_info.xml");

pub use crate::ip_utils::IpError;

#[derive(Debug, thiserror::Error)]
pub enum GeolocationError {
  #[error("GeoIP database not found. Please download it first.")]
  DatabaseNotFound,

  #[error("Failed to open GeoIP database: {0}")]
  DatabaseOpen(String),

  #[error("Invalid IP address: {0}")]
  InvalidIP(String),

  #[error("IP location not found: {0}")]
  LocationNotFound(String),

  #[error("Unknown territory: {0}")]
  UnknownTerritory(String),

  #[error("No language data for territory: {0}")]
  NoLanguageData(String),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("IP error: {0}")]
  Ip(#[from] IpError),
}

#[derive(Debug, Clone)]
pub struct Locale {
  pub language: String,
  pub region: Option<String>,
}

impl Locale {
  pub fn as_string(&self) -> String {
    if let Some(region) = &self.region {
      format!("{}-{}", self.language, region)
    } else {
      self.language.clone()
    }
  }
}

#[derive(Debug, Clone)]
pub struct Geolocation {
  pub locale: Locale,
  pub longitude: f64,
  pub latitude: f64,
  pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpLocationDetails {
  pub ip: String,
  pub latitude: f64,
  pub longitude: f64,
  pub timezone: String,
  pub timezone_offset_minutes: i32,
  pub accuracy_meters: f64,
  pub city: Option<String>,
  pub region: Option<String>,
  pub country_code: Option<String>,
  pub locale: Option<String>,
  pub source: String,
}

#[derive(Debug, Deserialize)]
struct IpWhoIsResponse {
  success: bool,
  message: Option<String>,
  ip: Option<String>,
  latitude: Option<f64>,
  longitude: Option<f64>,
  city: Option<String>,
  region: Option<String>,
  country_code: Option<String>,
  timezone: Option<IpWhoIsTimezone>,
}

#[derive(Debug, Deserialize)]
struct IpWhoIsTimezone {
  id: Option<String>,
}

struct LanguagePopulation {
  language: String,
  population_percent: f64,
}

fn jitter_coordinates(ip: &str, latitude: f64, longitude: f64) -> (f64, f64) {
  let mut hasher = DefaultHasher::new();
  ip.hash(&mut hasher);
  let seed = hasher.finish();

  let lat_bucket = (seed & 0xffff) as f64 / 65535.0;
  let lon_bucket = ((seed >> 16) & 0xffff) as f64 / 65535.0;
  let lat_sign = if ((seed >> 32) & 1) == 0 { -1.0 } else { 1.0 };
  let lon_sign = if ((seed >> 33) & 1) == 0 { -1.0 } else { 1.0 };

  let lat_delta = lat_sign * (0.003 + lat_bucket * 0.014);
  let lon_delta = lon_sign * (0.003 + lon_bucket * 0.014);

  (latitude + lat_delta, longitude + lon_delta)
}

pub struct LocaleSelector {
  territories: HashMap<String, Vec<LanguagePopulation>>,
}

impl LocaleSelector {
  pub fn new() -> Result<Self, GeolocationError> {
    let mut territories: HashMap<String, Vec<LanguagePopulation>> = HashMap::new();

    let mut reader = XmlReader::from_str(TERRITORY_INFO_XML);
    reader.config_mut().trim_text(true);

    let mut current_territory: Option<String> = None;
    let mut current_languages: Vec<LanguagePopulation> = Vec::new();

    let mut buf = Vec::new();

    loop {
      match reader.read_event_into(&mut buf) {
        Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
          let name = e.name();
          let name_str = std::str::from_utf8(name.as_ref()).unwrap_or("");

          if name_str == "territory" {
            if let Some(code) = current_territory.take() {
              if !current_languages.is_empty() {
                territories.insert(code, std::mem::take(&mut current_languages));
              }
            }

            for attr in e.attributes().flatten() {
              if attr.key.as_ref() == b"type" {
                current_territory = Some(String::from_utf8_lossy(&attr.value).to_uppercase());
              }
            }
          } else if name_str == "languagePopulation" && current_territory.is_some() {
            let mut lang_type = None;
            let mut pop_percent = 0.0;

            for attr in e.attributes().flatten() {
              match attr.key.as_ref() {
                b"type" => {
                  lang_type = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"populationPercent" => {
                  pop_percent = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0.0);
                }
                _ => {}
              }
            }

            if let Some(lang) = lang_type {
              current_languages.push(LanguagePopulation {
                language: lang.replace('_', "-"),
                population_percent: pop_percent,
              });
            }
          }
        }
        Ok(Event::End(ref e)) => {
          let name_ref = e.name();
          let name = std::str::from_utf8(name_ref.as_ref()).unwrap_or("");
          if name == "territory" {
            if let Some(code) = current_territory.take() {
              if !current_languages.is_empty() {
                territories.insert(code, std::mem::take(&mut current_languages));
              }
            }
          }
        }
        Ok(Event::Eof) => break,
        Err(e) => {
          log::warn!("Error parsing territory XML: {}", e);
          break;
        }
        _ => {}
      }
      buf.clear();
    }

    Ok(Self { territories })
  }

  #[allow(clippy::wrong_self_convention)]
  pub fn from_region(&self, region: &str) -> Result<Locale, GeolocationError> {
    let region_upper = region.to_uppercase();

    let languages = self
      .territories
      .get(&region_upper)
      .ok_or_else(|| GeolocationError::UnknownTerritory(region.to_string()))?;

    if languages.is_empty() {
      return Err(GeolocationError::NoLanguageData(region.to_string()));
    }

    let total: f64 = languages.iter().map(|l| l.population_percent).sum();
    let mut rng = rand::rng();
    let target = rng.random::<f64>() * total;
    let mut cumulative = 0.0;

    for lang in languages {
      cumulative += lang.population_percent;
      if cumulative >= target {
        return Ok(normalize_locale(&format!(
          "{}-{}",
          lang.language, region_upper
        )));
      }
    }

    let first_lang = &languages[0].language;
    Ok(normalize_locale(&format!(
      "{}-{}",
      first_lang, region_upper
    )))
  }
}

impl Default for LocaleSelector {
  fn default() -> Self {
    Self::new().unwrap_or(Self {
      territories: HashMap::new(),
    })
  }
}

fn normalize_locale(locale: &str) -> Locale {
  let parts: Vec<&str> = locale.split('-').collect();

  let language = parts
    .first()
    .map(|s| s.to_lowercase())
    .unwrap_or_else(|| "en".to_string());

  let mut region = None;

  for part in parts.iter().skip(1) {
    if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
      // Script subtag (e.g. Hans/Hant) — ignored; Wayfern fingerprint uses language+region only.
      continue;
    }
    region = Some(part.to_uppercase());
  }

  Locale { language, region }
}

pub fn get_geolocation(ip: &str) -> Result<Geolocation, GeolocationError> {
  let mmdb_path =
    GeoIPDownloader::get_mmdb_file_path().map_err(|_| GeolocationError::DatabaseNotFound)?;

  if !mmdb_path.exists() {
    return Err(GeolocationError::DatabaseNotFound);
  }

  let reader =
    Reader::open_readfile(&mmdb_path).map_err(|e| GeolocationError::DatabaseOpen(e.to_string()))?;

  let ip_addr: IpAddr =
    IpAddr::from_str(ip).map_err(|_| GeolocationError::InvalidIP(ip.to_string()))?;

  let lookup_result = reader
    .lookup(ip_addr)
    .map_err(|e| GeolocationError::LocationNotFound(e.to_string()))?;
  let city: geoip2::City = lookup_result
    .decode()
    .map_err(|e| GeolocationError::LocationNotFound(e.to_string()))?
    .ok_or_else(|| GeolocationError::LocationNotFound(ip.to_string()))?;

  let location = &city.location;

  let longitude = location
    .longitude
    .ok_or_else(|| GeolocationError::LocationNotFound("No longitude".to_string()))?;
  let latitude = location
    .latitude
    .ok_or_else(|| GeolocationError::LocationNotFound("No latitude".to_string()))?;
  let timezone = location
    .time_zone
    .ok_or_else(|| GeolocationError::LocationNotFound("No timezone".to_string()))?
    .to_string();

  let country = &city.country;
  let iso_code = country
    .iso_code
    .ok_or_else(|| GeolocationError::LocationNotFound("No country code".to_string()))?
    .to_uppercase();

  let selector = LocaleSelector::new()?;
  let locale = selector.from_region(&iso_code)?;
  let (latitude, longitude) = jitter_coordinates(ip, latitude, longitude);

  Ok(Geolocation {
    locale,
    longitude,
    latitude,
    timezone,
  })
}

pub async fn lookup_ip_location_details(
  ip: &str,
  accuracy_meters: Option<f64>,
) -> Result<IpLocationDetails, GeolocationError> {
  if !crate::ip_utils::validate_ip(ip) {
    return Err(GeolocationError::InvalidIP(ip.to_string()));
  }

  let url = format!(
    "https://ipwho.is/{ip}?fields=success,message,ip,latitude,longitude,city,region,country_code,timezone"
  );
  let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()
    .map_err(|e| GeolocationError::Ip(IpError::Network(e.to_string())))?;
  let response = client
    .get(url)
    .send()
    .await
    .map_err(|e| GeolocationError::Ip(IpError::Network(e.to_string())))?;

  if !response.status().is_success() {
    return Err(GeolocationError::LocationNotFound(format!(
      "ipwho.is returned HTTP {}",
      response.status()
    )));
  }

  let resolved = response
    .json::<IpWhoIsResponse>()
    .await
    .map_err(|e| GeolocationError::LocationNotFound(e.to_string()))?;

  if !resolved.success {
    return Err(GeolocationError::LocationNotFound(
      resolved.message.unwrap_or_else(|| ip.to_string()),
    ));
  }

  let latitude = resolved
    .latitude
    .ok_or_else(|| GeolocationError::LocationNotFound("No latitude".to_string()))?;
  let longitude = resolved
    .longitude
    .ok_or_else(|| GeolocationError::LocationNotFound("No longitude".to_string()))?;
  let timezone = resolved
    .timezone
    .and_then(|tz| tz.id)
    .filter(|tz| !tz.trim().is_empty())
    .ok_or_else(|| GeolocationError::LocationNotFound("No timezone".to_string()))?;
  let timezone_offset_minutes = timezone_offset_minutes(&timezone)
    .ok_or_else(|| GeolocationError::LocationNotFound("Invalid timezone".to_string()))?;
  let locale = resolved.country_code.as_deref().and_then(|country_code| {
    LocaleSelector::new()
      .ok()
      .and_then(|selector| selector.from_region(country_code).ok())
      .map(|locale| locale.as_string())
  });
  let (latitude, longitude) = jitter_coordinates(
    &resolved.ip.clone().unwrap_or_else(|| ip.to_string()),
    latitude,
    longitude,
  );

  Ok(IpLocationDetails {
    ip: resolved.ip.unwrap_or_else(|| ip.to_string()),
    latitude,
    longitude,
    timezone,
    timezone_offset_minutes,
    accuracy_meters: accuracy_meters.unwrap_or(20_000.0),
    city: resolved.city.filter(|value| !value.trim().is_empty()),
    region: resolved.region.filter(|value| !value.trim().is_empty()),
    country_code: resolved
      .country_code
      .filter(|value| !value.trim().is_empty()),
    locale,
    source: "ipwho.is".to_string(),
  })
}

pub fn timezone_offset_minutes(timezone: &str) -> Option<i32> {
  let tz = timezone.parse::<chrono_tz::Tz>().ok()?;
  let now = chrono::Utc::now().with_timezone(&tz);
  let offset_seconds = now.offset().fix().local_minus_utc();
  Some(-(offset_seconds / 60))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_locale_selector_creation() {
    let selector = LocaleSelector::new();
    assert!(selector.is_ok());
  }

  #[test]
  fn test_locale_from_region() {
    let selector = LocaleSelector::new().unwrap();

    let us_locale = selector.from_region("US");
    assert!(us_locale.is_ok());
    let us = us_locale.unwrap();
    assert_eq!(us.region, Some("US".to_string()));

    let de_locale = selector.from_region("DE");
    assert!(de_locale.is_ok());
    let de = de_locale.unwrap();
    assert_eq!(de.region, Some("DE".to_string()));
  }

  #[test]
  fn test_locale_as_string() {
    let locale = Locale {
      language: "en".to_string(),
      region: Some("US".to_string()),
    };
    assert_eq!(locale.as_string(), "en-US");

    let locale_no_region = Locale {
      language: "en".to_string(),
      region: None,
    };
    assert_eq!(locale_no_region.as_string(), "en");
  }

  #[test]
  fn test_normalize_locale() {
    let locale = normalize_locale("en-US");
    assert_eq!(locale.language, "en");
    assert_eq!(locale.region, Some("US".to_string()));

    let zh_tw = normalize_locale("zh-TW");
    assert_eq!(zh_tw.language, "zh");
    assert_eq!(zh_tw.region, Some("TW".to_string()));

    let zh_hant_us = normalize_locale("zh-Hant-US");
    assert_eq!(zh_hant_us.language, "zh");
    assert_eq!(zh_hant_us.region, Some("US".to_string()));
  }
}
