use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::serde_json::{self, Value};
use near_sdk::store::IterableMap;
use near_sdk::{
    env, ext_contract, near, require, AccountId, BorshStorageKey, Gas, Promise, PromiseError,
};
use urlencoding::encode;

const FETCH_GAS: Gas = Gas::from_tgas(40);
const CALLBACK_GAS: Gas = Gas::from_tgas(20);

#[near(serializers = [json])]
#[derive(Clone)]
pub enum FetchStatus {
    Completed,
    TimedOut,
}

#[near(serializers = [json])]
#[derive(Clone)]
pub struct FetchResult {
    pub request_id: u64,
    pub url: String,
    pub status: FetchStatus,
    pub body: Option<Vec<u8>>,
    pub context: Option<Vec<u8>>,
    pub caller: AccountId,
}

#[ext_contract(http_fetcher)]
trait HttpFetcher {
    fn fetch(&mut self, url: String, context: Option<Vec<u8>>) -> FetchResult;
}

#[derive(BorshSerialize, BorshDeserialize, BorshStorageKey)]
enum StorageKey {
    WeatherByCity,
}

#[near(contract_state)]
pub struct Contract {
    fetcher_account: AccountId,
    weather_by_city: IterableMap<String, String>,
}

#[near]
impl Contract {
    #[init]
    pub fn new(fetcher_account: AccountId) -> Self {
        require!(!env::state_exists(), "Already initialized");
        Self {
            fetcher_account,
            weather_by_city: IterableMap::new(StorageKey::WeatherByCity),
        }
    }

    pub fn request_weather(&mut self, city: String) -> Promise {
        let encoded_city = encode(&city);
        let url = format!(
            "https://api.openweathermap.org/data/2.5/find?q={encoded_city}&appid=5796abbde9106b7da4febfae8c44c232&units=metric"
        );
        http_fetcher::ext(self.fetcher_account.clone())
            .with_static_gas(FETCH_GAS)
            .fetch(url, Some(city.as_bytes().to_vec()))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_weather_response(city),
            )
    }

    #[private]
    pub fn on_weather_response(
        &mut self,
        city: String,
        #[callback_result] result: Result<FetchResult, PromiseError>,
    ) -> bool {
        match result {
            Ok(fetch_result) => match fetch_result.status {
                FetchStatus::Completed => {
                    if let Some(body) = fetch_result.body {
                        if let Some(message) = format_weather_message(&body) {
                            env::log_str(&message);
                            self.weather_by_city.insert(city.clone(), message);
                            true
                        } else {
                            env::log_str("Failed to parse weather payload");
                            false
                        }
                    } else {
                        env::log_str("Fetch completed without body");
                        false
                    }
                }
                FetchStatus::TimedOut => {
                    env::log_str("Fetch timed out");
                    false
                }
            },
            Err(_) => {
                env::log_str("Fetch promise failed");
                false
            }
        }
    }

    pub fn get_cached_weather(&self, city: String) -> Option<String> {
        self.weather_by_city
            .get(&city)
            .cloned()
    }
}

impl Default for Contract {
    fn default() -> Self {
        env::panic_str("Contract must be initialized with new(fetcher_account)");
    }
}

fn format_weather_message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let list = value.get("list")?.as_array()?;
    let first = list.first()?;

    let city = first.get("name")?.as_str()?.to_string();
    let country = first
        .get("sys")
        .and_then(|sys| sys.get("country"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let temperature_c = first
        .get("main")
        .and_then(|main| main.get("temp"))
        .and_then(|t| t.as_f64());
    let description = first
        .get("weather")
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.first())
        .and_then(|entry| entry.get("description"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());

    let mut message = match country {
        Some(country) => format!("Weather in {city}, {country}"),
        None => format!("Weather in {city}"),
    };

    if let Some(temp) = temperature_c {
        message.push_str(&format!(" is {:.1}°C", temp));
    }

    if let Some(desc) = description {
        message.push_str(&format!(" ({desc})"));
    }

    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;
    use std::str::FromStr;

    fn account(name: &str) -> AccountId {
        AccountId::from_str(name).unwrap()
    }

    #[test]
    fn initialization() {
        testing_env!(VMContextBuilder::new()
            .current_account_id(account("weather.testnet"))
            .build());
        let contract = Contract::new(account("fetcher.testnet"));
        assert_eq!(contract.fetcher_account, account("fetcher.testnet"));
    }
}
