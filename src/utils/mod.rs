use std::net::Ipv4Addr;
use std::str::FromStr;

const IP_VALIDATION_ERROR: &str = "IP Validation failed";

pub(crate) fn validate_ip(s: &str) -> Result<(), &'static str> {
    Ipv4Addr::from_str(s)
        .map(|_| ())
        .map_err(|_| IP_VALIDATION_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ip_correctly() -> Result<(), &'static str> {
        validate_ip("180.52.4.240")
    }

    #[test]
    fn validate_ip_fails_on_ipv6() {
        let result = validate_ip("2001:4860:4860::8888");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IP_VALIDATION_ERROR);
    }

    #[test]
    fn validate_ip_fails_on_random_string() {
        let result = validate_ip("this is not a valid ip address");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IP_VALIDATION_ERROR);
    }
}
