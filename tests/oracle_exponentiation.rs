// Keep the exponentiation oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/exponentiation/oracle_power_bigints.rs"]
mod oracle_power_bigints;
#[path = "oracle/exponentiation/oracle_power_numbers.rs"]
mod oracle_power_numbers;
